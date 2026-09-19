#!/usr/bin/env bash
# W2 · PROPONER (ADR 0030): dos personas, dos ramas, una revision.
#
# `ore-serve` contra un arbol en un repositorio pelado (`file://`) y la API de
# la forja de mentira (`forja-de-mentira.py`: ramas y diffs con git de verdad,
# PRs y reviews en memoria, un solo usuario como la de verdad).
#
#   1  GET /ramas                        main sola, porDefecto, sin propuesta
#   2  POST /ramas {nombre}              201 `ana/vistas-hr` · GET /ramas la lista · 409 repetida ·
#                                       nombre malo 422
#   3  el arbol EN la rama               PUT /arbol/… con X-Ore-Rama → 201 con commit y rama; main
#                                       NO lo tiene (404) y la rama si (200); una rama que no
#                                       existe → 404; el gate «no empeora» sigue en la rama (422)
#   3b POST /arbol/commit               varios ficheros en UN commit con mensaje, o en seco lo que
#                                       seria: A/M/D y +/- de git, el gate «no empeora», 0 cambiados
#                                       al repetir, retirar sale como D
#   4  POST /propuestas                  201 #1 con autor persona:ana · GET /propuestas la lista
#                                       abierta · 422 sin rama · 422 desde main
#   5  GET /propuestas/1                 ficheros (1, added), diff de lineas con el fichero,
#                                       semantico (ore diff: OOS5021 patch), diagnosticos [],
#                                       revisiones []
#   6  la revision es de OTRA persona    ana aprueba lo suyo → 422 · ana fusiona lo suyo → 422 ·
#                                       bea fusiona sin revision → 422 · ana comenta → 201 ·
#                                       bea aprueba → 201 y GET la lista
#   7  POST /propuestas/1/fusionar (bea) 200 · main tiene el fichero · GET /ramas: la rama fuera ·
#                                       GET /propuestas: fusionada · otra vez → 409
#   8  la segunda persona, la segunda rama   bea: rama, fichero, propuesta #2; ana aprueba y fusiona:
#                                       dos personas, dos ramas, una revision · DELETE /ramas con
#                                       propuesta abierta → 409 · DELETE /propuestas cierra y
#                                       entonces la rama se retira · DELETE main → 422
#   9  sin API                           un servidor con --repo (directorio) contesta 422 a /ramas y
#                                       a X-Ore-Rama
#
# Uso:  bash pruebas-de-fuego/la-propuesta.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8917}"
PUERTO_FORJA="${PUERTO_FORJA:-8918}"
PUERTO_DIR="${PUERTO_DIR:-8919}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""; FORJA=""; SRV2=""

falla() {
  echo "✗ $*" >&2
  [ -s "$TMP/arranque.txt" ] && { echo "── lo que dijo el servidor ──" >&2; tail -20 "$TMP/arranque.txt" >&2; }
  limpiar; exit 1
}
dice()  { echo "  · $*"; }
limpiar() { for p in $SRV $FORJA $SRV2; do kill "$p" 2>/dev/null; done; sleep 0.3; rm -rf "$TMP"; }
trap limpiar EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"         || falla "no hay binario de \`ore\` — cargo build -p ore-cli"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\` — cargo build -p ore-serve"
PY=$(command -v python3 || command -v python) || falla "hace falta python"

# ── el arbol: un paquete con una tabla y una vista, en un repositorio pelado ──
A="$TMP/arbol"
mkdir -p "$A/packages/hr/tables" "$A/packages/hr/views"
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: erp, type: jsonl, connectionEnv: FICHEROS_DIR }
Y
cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:data
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y
cat > "$A/packages/hr/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: hr, version: 0.1.0, status: active, domain: people }
spec: { owner: team:data }
Y
cat > "$A/packages/hr/tables/empleados_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: empleados_t, namespace: hr }
spec:
  datasource: erp
  object: "empleados.jsonl"
  columns:
    id: {}
    pais: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
cat > "$A/packages/hr/views/empleados.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: empleados, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: hr.empleados_t
  fields:
    id: { from: id, type: String }
    pais: { from: pais, type: String }
Y
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol de partida no compila: $(cd "$A" && "$ORE" validate . 2>&1 | head -3)"
BARE="$TMP/forja/t-demo/ontologia.git"
mkdir -p "$(dirname "$BARE")" && git init -q --bare -b main "$BARE"
( cd "$A" && git init -q -b main && git config core.autocrlf false && git add -A \
  && git -c user.name=banco -c user.email=banco@invalido commit -q -m "el arbol de partida" \
  && git remote add origin "$BARE" && git push -q origin main ) || falla "no se pudo sembrar la forja"
BARE_URL="file://$(cd "$BARE" && pwd | sed 's#^/\([a-zA-Z]\)/#\1:/#')"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) BARE_URL="file:///$(cd "$BARE" && pwd -W)";; esac

# ── la forja de mentira y el servidor ──────────────────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/forja-de-mentira.py" "$BARE" "$PUERTO_FORJA" >"$TMP/forja.txt" 2>&1 &
FORJA=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PUERTO_FORJA/api/v1/version" && break; sleep 0.25; done
FORJA_TOKEN=de-mentira "$SERVE" --forja "$BARE_URL" --forja-api "127.0.0.1:$PUERTO_FORJA" --ore "$ORE" \
  --bind "127.0.0.1:$PUERTO" --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
grep -q "ramas y propuestas: la API de la forja en 127.0.0.1:$PUERTO_FORJA (t-demo/ontologia)" "$TMP/arranque.txt" || falla "el servidor no dijo la API de la forja: $(cat "$TMP/arranque.txt")"

ANA='x-ore-sujeto: persona:ana'
BEA='x-ore-sujeto: persona:bea'
cuerpo() { cat "$TMP/r.json"; }
# pide <metodo> <camino> <quien> [cuerpo] [rama]
pide() {
  local m=$1 c=$2 q=$3 d=${4:-} r=${5:-}
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$m" -H "$q" -H 'content-type: application/json' \
    ${r:+-H "x-ore-rama: $r"} ${d:+-d "$d"} "$BASE$c"
}
tiene() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); assert eval(sys.argv[2]), d' "$TMP/r.json" "$1" 2>/dev/null; }
NUEVA='apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: espanoles, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: hr.empleados
  where: { pais: ES }
  fields:
    id: { from: id, type: String }
'
put_fichero() { # <ruta> <quien> <texto> [rama]
  local ruta=$1 q=$2 texto=$3 r=${4:-}
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X PUT -H "$q" -H 'content-type: text/plain' \
    ${r:+-H "x-ore-rama: $r"} --data-binary "$texto" "$BASE/arbol/$ruta"
}

# ── 1 ───────────────────────────────────────────────────────────────────────
[ "$(pide GET /ramas "$ANA")" = "200" ] || falla "1 · GET /ramas: $(cuerpo)"
tiene "d['porDefecto']=='main' and [r['nombre'] for r in d['ramas']]==['main'] and d['ramas'][0]['porDefecto'] is True and d['ramas'][0]['propuesta'] is False" || falla "1 · las ramas de partida: $(cuerpo)"
dice "1 · GET /ramas: main sola, por defecto, sin propuesta"

# ── 2 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /ramas "$ANA" '{"nombre":"vistas-hr"}')" = "201" ] || falla "2 · crear la rama: $(cuerpo)"
tiene "d['rama']=='ana/vistas-hr' and d['desde']=='main' and d['de']=='persona:ana'" || falla "2 · la rama no es de ana: $(cuerpo)"
[ "$(pide GET /ramas "$ANA")" = "200" ] && tiene "sorted(r['nombre'] for r in d['ramas'])==['ana/vistas-hr','main']" || falla "2 · GET /ramas no la lista: $(cuerpo)"
[ "$(pide POST /ramas "$ANA" '{"nombre":"vistas-hr"}')" = "409" ] || falla "2 · repetir la rama no dio 409: $(cuerpo)"
[ "$(pide POST /ramas "$ANA" '{"nombre":"a b"}')" = "422" ] || falla "2 · un nombre con espacio no dio 422: $(cuerpo)"
[ "$(pide POST /ramas "$ANA" '{"nombre":"../x"}')" = "422" ] || falla "2 · un nombre con .. no dio 422: $(cuerpo)"
[ "$(pide POST /ramas "$BEA" '{}')" = "201" ] && tiene "d['rama'].startswith('bea/propuesta-')" || falla "2 · sin nombre no salio bea/propuesta-<fecha>: $(cuerpo)"
dice "2 · POST /ramas: 201 ana/vistas-hr · listada · 409 repetida · 422 nombre malo · sin nombre, la fecha"

# ── 3 ───────────────────────────────────────────────────────────────────────
[ "$(put_fichero packages/hr/views/espanoles.yaml "$ANA" "$NUEVA" ana/vistas-hr)" = "201" ] || falla "3 · PUT en la rama: $(cuerpo)"
tiene "d['rama']=='ana/vistas-hr' and len(d['commit'])>=7 and d['nueva'] is True and d['diagnosticos']==[]" || falla "3 · la respuesta no dice la rama y el commit: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/espanoles.yaml "$ANA")" = "404" ] || falla "3 · main tiene el fichero de la rama: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/espanoles.yaml "$ANA" "" ana/vistas-hr)" = "200" ] || falla "3 · la rama no tiene el fichero: $(cuerpo)"
[ "$(pide GET /arbol "$ANA" "" ana/vistas-hr)" = "200" ] && tiene "any('espanoles' in str(x) for x in d.get('ficheros', d.get('arbol', d)))" || falla "3 · GET /arbol en la rama no lista el fichero: $(cuerpo | head -c 300)"
[ "$(pide GET /arbol/diagnosticos "$ANA" "" ana/vistas-hr)" = "200" ] && tiene "d['diagnosticos']==[]" || falla "3 · diagnosticos de la rama: $(cuerpo)"
[ "$(pide GET /arbol "$ANA" "" nadie/rama)" = "404" ] || falla "3 · una rama que no existe no dio 404: $(cuerpo)"
[ "$(pide GET /arbol "$ANA" "" 'a b')" = "422" ] || falla "3 · una rama con nombre malo no dio 422: $(cuerpo)"
# la misma identidad dos veces (OOS2035): el arbol de la rama empeora y no se escribe
ROTA=$(printf '%s' "$NUEVA")
[ "$(put_fichero packages/hr/views/rota.yaml "$ANA" "$ROTA" ana/vistas-hr)" = "422" ] || falla "3 · una vista rota en la rama no dio 422: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/rota.yaml "$ANA" "" ana/vistas-hr)" = "404" ] || falla "3 · la vista rota quedo en la rama"
dice "3 · el arbol EN la rama: PUT con X-Ore-Rama → 201 con commit y rama · main no lo tiene, la rama si · diagnosticos de la rama · 404 rama inexistente · 422 nombre malo · el gate «no empeora» sigue en la rama"

# ── 3b · varios ficheros en UN commit con mensaje (POST /arbol/commit), y en seco lo que seria ──
# Lo que el panel de Commit del workspace enseña sale de git, no de un contador:
# `A`/`M`/`D` de `git status`, +/− de `git diff --cached --numstat`.
"$PY" - "$TMP/commit.json" "$NUEVA" <<'EOF'
import json, sys
nueva = sys.argv[2]
portugueses = nueva.replace('name: espanoles', 'name: portugueses').replace('pais: ES', 'pais: PT')
cambiada = nueva.rstrip('\n') + '\n    pais: { from: pais, type: String }\n'
json.dump({"seco": True, "ficheros": [
    {"ruta": "packages/hr/views/portugueses.yaml", "texto": portugueses},
    {"ruta": "packages/hr/views/espanoles.yaml", "texto": cambiada},
]}, open(sys.argv[1], 'w'))
EOF
commit() { # <quien> <rama> <fichero json>
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$1" -H 'content-type: application/json' -H "x-ore-rama: $2" --data-binary "@$3" "$BASE/arbol/commit"
}
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "200" ] || falla "3b · en seco: $(cuerpo)"
tiene "d['seco'] is True and d['cambiados']==2 and {c['ruta']:(c['estado'],c['mas'],c['menos']) for c in d['cambios']}=={'packages/hr/views/portugueses.yaml':('A',9,0),'packages/hr/views/espanoles.yaml':('M',1,0)} and d['diagnosticos']==[]" || falla "3b · los cambios en seco no son los de git: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/portugueses.yaml "$ANA" "" ana/vistas-hr)" = "404" ] || falla "3b · en seco escribio en la rama"
# sin mensaje y sin seco: 422
"$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); d["seco"]=False; json.dump(d, open(sys.argv[1],"w"))' "$TMP/commit.json"
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "422" ] || falla "3b · sin mensaje no dio 422: $(cuerpo)"
# el commit de verdad, con mensaje
"$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); d["mensaje"]="Los portugueses, y una descripcion para los espanoles"; json.dump(d, open(sys.argv[1],"w"))' "$TMP/commit.json"
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "201" ] || falla "3b · el commit: $(cuerpo)"
tiene "d['seco'] is False and d['cambiados']==2 and d['rama']=='ana/vistas-hr' and len(d['commit'])>=7" || falla "3b · la respuesta del commit: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/portugueses.yaml "$ANA" "" ana/vistas-hr)" = "200" ] || falla "3b · la rama no tiene el fichero nuevo"
git --git-dir="$BARE" log -1 --format='%an · %s' ana/vistas-hr | grep -q "persona:ana · Los portugueses, y una descripcion para los espanoles" || falla "3b · el commit no lleva el mensaje y el autor: $(git --git-dir="$BARE" log -1 --format='%an · %s' ana/vistas-hr)"
[ "$(git --git-dir="$BARE" show --stat --format= ana/vistas-hr | grep -c 'yaml')" = "2" ] || falla "3b · el commit no lleva los dos ficheros"
# el mismo commit otra vez: nada cambia, 200 con 0 cambiados
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "200" ] && tiene "d['cambiados']==0" || falla "3b · repetir el commit no dio 200 con 0 cambiados: $(cuerpo)"
# y el gate: un fichero que duplica una identidad → 422 con los diagnosticos y los cambios, nada escrito
"$PY" - "$TMP/commit.json" "$NUEVA" <<'EOF'
import json, sys
json.dump({"seco": False, "mensaje": "rompo", "ficheros": [{"ruta": "packages/hr/views/rota.yaml", "texto": sys.argv[2]}]}, open(sys.argv[1], 'w'))
EOF
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "422" ] || falla "3b · el gate no dio 422: $(cuerpo)"
tiene "len(d['diagnosticos'])>=1 and d['cambios'][0]['estado']=='A'" || falla "3b · el 422 no trae diagnosticos y cambios: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/rota.yaml "$ANA" "" ana/vistas-hr)" = "404" ] || falla "3b · el gate escribio igual"
# retirar en el mismo commit
"$PY" -c 'import json,sys; json.dump({"seco": True, "retirar": ["packages/hr/views/portugueses.yaml"]}, open(sys.argv[1],"w"))' "$TMP/commit.json"
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "200" ] && tiene "d['cambios']==[{'ruta':'packages/hr/views/portugueses.yaml','estado':'D','mas':0,'menos':9}]" || falla "3b · retirar en seco: $(cuerpo)"
dice "3b · POST /arbol/commit: en seco, A/M con +/- de git y nada escrito · sin mensaje 422 · con mensaje, UN commit de la persona con los dos ficheros en la rama · repetido, 0 cambiados · el gate 422 con diagnosticos y cambios · retirar sale como D"

# ── 4 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /propuestas "$ANA" '{"titulo":"x"}')" = "422" ] || falla "4 · sin rama no dio 422: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"main"}')" = "422" ] || falla "4 · proponer main no dio 422: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/vistas-hr","titulo":"Los empleados de Espana","descripcion":"una vista con where"}')" = "201" ] || falla "4 · proponer: $(cuerpo)"
tiene "d['numero']==1 and d['autor']=='persona:ana' and d['rama']=='ana/vistas-hr' and d['base']=='main' and d['estado']=='abierta' and d['titulo']=='Los empleados de Espana' and d['descripcion']=='una vista con where'" || falla "4 · la propuesta no lleva a ana: $(cuerpo)"
[ "$(pide GET /propuestas "$ANA")" = "200" ] && tiene "len(d['propuestas'])==1 and d['propuestas'][0]['estado']=='abierta'" || falla "4 · GET /propuestas: $(cuerpo)"
[ "$(pide GET /ramas "$ANA")" = "200" ] && tiene "[r for r in d['ramas'] if r['nombre']=='ana/vistas-hr'][0]['propuesta']==1" || falla "4 · GET /ramas no dice la propuesta de la rama: $(cuerpo)"
dice "4 · POST /propuestas: 201 #1 de persona:ana · listada abierta · la rama sabe su propuesta · 422 sin rama · 422 desde main"

# ── 5 ───────────────────────────────────────────────────────────────────────
[ "$(pide GET /propuestas/1 "$BEA")" = "200" ] || falla "5 · GET /propuestas/1: $(cuerpo)"
tiene "sorted((f['ruta'],f['estado'],f['mas'],f['menos']) for f in d['ficheros'])==[('packages/hr/views/espanoles.yaml','added',10,0),('packages/hr/views/portugueses.yaml','added',9,0)]" || falla "5 · los ficheros: $(cuerpo | head -c 400)"
tiene "'+++ b/packages/hr/views/espanoles.yaml' in d['diff'] and '+  where: { pais: ES }' in d['diff']" || falla "5 · el diff de lineas: $(cuerpo | head -c 400)"
tiene "any(c.get('code')=='OOS5021' for c in d['semantico']['changes']) and d['semantico']['requiredBump']=='patch'" || falla "5 · el diff semantico: $(cuerpo | head -c 600)"
tiene "d['diagnosticos']==[] and d['revisiones']==[]" || falla "5 · diagnosticos y revisiones de partida: $(cuerpo | head -c 300)"
[ "$(pide GET /propuestas/9 "$BEA")" = "404" ] || falla "5 · una propuesta que no existe no dio 404"
dice "5 · GET /propuestas/1: ficheros (dos added, +10 y +9) · diff de lineas · diff de significado (OOS5021, patch) · diagnosticos [] · revisiones []"

# ── 6 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /propuestas/1/revisar "$ANA" '{"veredicto":"aprobar"}')" = "422" ] || falla "6 · ana aprobo lo suyo: $(cuerpo)"
[ "$(pide POST /propuestas/1/fusionar "$ANA")" = "422" ] || falla "6 · ana fusiono lo suyo: $(cuerpo)"
cuerpo | grep -q "quien propone no fusiona" || falla "6 · el 422 no dice por que: $(cuerpo)"
[ "$(pide POST /propuestas/1/fusionar "$BEA")" = "422" ] || falla "6 · bea fusiono sin revision: $(cuerpo)"
cuerpo | grep -q "no tiene revisión" || falla "6 · el 422 no dice que falta la revision: $(cuerpo)"
[ "$(pide POST /propuestas/1/revisar "$ANA" '{"veredicto":"comentar","texto":"lo he probado"}')" = "201" ] || falla "6 · ana no pudo comentar: $(cuerpo)"
[ "$(pide POST /propuestas/1/revisar "$BEA" '{"veredicto":"otro"}')" = "422" ] || falla "6 · un veredicto inventado no dio 422"
[ "$(pide POST /propuestas/1/revisar "$BEA" '{"veredicto":"aprobar","texto":"bien visto"}')" = "201" ] || falla "6 · bea no pudo aprobar: $(cuerpo)"
tiene "d['por']=='persona:bea' and d['veredicto']=='aprueba'" || falla "6 · la revision no es de bea: $(cuerpo)"
[ "$(pide GET /propuestas/1 "$ANA")" = "200" ] && tiene "[(r['por'],r['veredicto'],r['texto']) for r in d['revisiones']]==[('persona:ana','comenta','lo he probado'),('persona:bea','aprueba','bien visto')]" || falla "6 · las revisiones: $(cuerpo | head -c 600)"
dice "6 · la revision es de OTRA persona: ana no aprueba ni fusiona lo suyo (422) · sin revision no se fusiona (422) · ana comenta, bea aprueba · GET las lista con quien y veredicto"

# ── 7 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /propuestas/1/fusionar "$BEA")" = "200" ] || falla "7 · fusionar: $(cuerpo)"
tiene "d['fusionada'] is True and d['por']=='persona:bea' and d['revisada_por']==['persona:bea'] and d['rama']=='ana/vistas-hr'" || falla "7 · la fusion no dice quien: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/espanoles.yaml "$ANA")" = "200" ] || falla "7 · main no tiene la vista fusionada: $(cuerpo)"
[ "$(pide GET /ramas "$ANA")" = "200" ] && tiene "all(r['nombre']!='ana/vistas-hr' for r in d['ramas'])" || falla "7 · la rama sigue tras fusionar: $(cuerpo)"
[ "$(pide GET /propuestas "$ANA")" = "200" ] && tiene "d['propuestas'][0]['estado']=='fusionada' and d['propuestas'][0]['fusionada']!=''" || falla "7 · la propuesta no esta fusionada: $(cuerpo)"
[ "$(pide POST /propuestas/1/fusionar "$BEA")" = "409" ] || falla "7 · fusionar dos veces no dio 409: $(cuerpo)"
git --git-dir="$BARE" log -1 --format=%s main | grep -q "Propuesta #1 de persona:ana, revisada por persona:bea y fusionada por persona:bea" || falla "7 · el commit de merge no dice quien: $(git --git-dir="$BARE" log -1 --format=%s main)"
dice "7 · fusionar (bea): 200 · main tiene la vista · la rama fuera · fusionada · 409 la segunda vez · el commit de merge dice quien propuso, quien reviso y quien fusiono"

# ── 8 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /ramas "$BEA" '{"nombre":"franceses"}')" = "201" ] || falla "8 · la rama de bea: $(cuerpo)"
FR=$(printf '%s' "$NUEVA" | sed 's/name: espanoles/name: franceses/; s/pais: ES/pais: FR/')
[ "$(put_fichero packages/hr/views/franceses.yaml "$BEA" "$FR" bea/franceses)" = "201" ] || falla "8 · el fichero de bea en su rama: $(cuerpo)"
[ "$(pide POST /propuestas "$BEA" '{"rama":"bea/franceses"}')" = "201" ] && tiene "d['numero']==2 and d['autor']=='persona:bea' and d['titulo'].startswith('Propuesta de persona:bea')" || falla "8 · la propuesta de bea: $(cuerpo)"
[ "$(pide DELETE /ramas/bea/franceses "$BEA")" = "409" ] || falla "8 · retirar una rama con propuesta abierta no dio 409: $(cuerpo)"
[ "$(pide POST /propuestas/2/fusionar "$BEA")" = "422" ] || falla "8 · bea fusiono lo suyo"
[ "$(pide POST /propuestas/2/revisar "$ANA" '{"veredicto":"aprobar"}')" = "201" ] || falla "8 · ana no pudo aprobar: $(cuerpo)"
[ "$(pide POST /propuestas/2/fusionar "$ANA")" = "200" ] && tiene "d['por']=='persona:ana' and d['revisada_por']==['persona:ana']" || falla "8 · ana no pudo fusionar: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/franceses.yaml "$ANA")" = "200" ] || falla "8 · main no tiene la vista de bea"
[ "$(pide GET /propuestas "$ANA")" = "200" ] && tiene "sorted((p['numero'],p['autor'],p['estado']) for p in d['propuestas'])==[(1,'persona:ana','fusionada'),(2,'persona:bea','fusionada')]" || falla "8 · las dos propuestas: $(cuerpo)"
# cerrar sin fusionar, y entonces la rama se retira
[ "$(pide POST /ramas "$ANA" '{"nombre":"borrador"}')" = "201" ] || falla "8 · la rama borrador"
[ "$(put_fichero packages/hr/views/borrador.yaml "$ANA" "$(printf '%s' "$NUEVA" | sed 's/name: espanoles/name: borrador/')" ana/borrador)" = "201" ] || falla "8 · el fichero del borrador: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/borrador"}')" = "201" ] && tiene "d['numero']==3" || falla "8 · la propuesta del borrador: $(cuerpo)"
[ "$(pide DELETE /propuestas/3 "$ANA")" = "200" ] && tiene "d['estado']=='cerrada'" || falla "8 · cerrar la propuesta: $(cuerpo)"
[ "$(pide DELETE /propuestas/3 "$ANA")" = "409" ] || falla "8 · cerrar dos veces no dio 409"
[ "$(pide GET /arbol/packages/hr/views/borrador.yaml "$ANA")" = "404" ] || falla "8 · main tiene el borrador cerrado"
[ "$(pide DELETE /ramas/ana/borrador "$ANA")" = "200" ] && tiene "d['retirada'] is True" || falla "8 · retirar la rama del borrador: $(cuerpo)"
[ "$(pide DELETE /ramas/main "$ANA")" = "422" ] || falla "8 · retirar main no dio 422: $(cuerpo)"
[ "$(pide DELETE /ramas/nadie "$ANA")" = "404" ] || falla "8 · retirar una rama inexistente no dio 404: $(cuerpo)"
dice "8 · dos personas, dos ramas, una revision: bea propone, ana aprueba y fusiona · 409 retirar una rama con propuesta · cerrar una propuesta (200, 409 despues) y entonces la rama se retira · main no se retira (422)"

# ── 9 ───────────────────────────────────────────────────────────────────────
mkdir -p "$TMP/dir" && cp -r "$A/." "$TMP/dir/"
"$SERVE" --repo "$TMP/dir" --ore "$ORE" --bind "127.0.0.1:$PUERTO_DIR" --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/arranque2.txt" 2>&1 &
SRV2=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PUERTO_DIR/salud" && break; sleep 0.25; done
[ "$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$ANA" "http://127.0.0.1:$PUERTO_DIR/ramas")" = "422" ] || falla "9 · sin API, /ramas no dio 422: $(cuerpo)"
[ "$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$ANA" -H 'x-ore-rama: x/y' "http://127.0.0.1:$PUERTO_DIR/arbol")" = "422" ] || falla "9 · sin forja, X-Ore-Rama no dio 422: $(cuerpo)"
[ "$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$ANA" "http://127.0.0.1:$PUERTO_DIR/arbol")" = "200" ] || falla "9 · sin cabecera el arbol sigue: $(cuerpo | head -c 200)"
dice "9 · sin API de la forja: /ramas 422 · X-Ore-Rama 422 · el arbol sin cabecera, como siempre"

echo "✓ la propuesta (0030 W2): 1–9 · dos personas, dos ramas, una revision"
