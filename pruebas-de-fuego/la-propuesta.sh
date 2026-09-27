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
#   3c el catalogo EN la rama           GET /paquetes/{p}/esquema y /paquetes con X-Ore-Rama enseñan
#                                       la tabla de la rama y main no · POST /paquetes/{p}/schemas
#                                       escribe en la rama (main sin el schema) · lo que mueve datos
#                                       (ascender a estandar) en una rama: 409 que lo dice
#   3d en que se diferencia la rama     GET /ramas/{r}/cambios: por ACTIVO (nuevo, modificado, borrado)
#                                       frente al punto del que salio —lo que main hizo despues no
#                                       es de la rama—; la columna quitada, OOS5001 atado a su vista,
#                                       a quien alcanza (la vista que la lee); adelante/atras; de
#                                       memoria la segunda vez
#   3b POST /arbol/commit               varios ficheros en UN commit con mensaje, o en seco lo que
#                                       seria: A/M/D y +/- de git, el gate «no empeora» (422
#                                       forzable), forzar: true commitea igual y lo dice, 0 cambiados
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
#   8b el historial de un fichero        GET /arbol/historia/{ruta} (las versiones de git: persona,
#                                       committer, mensaje) · GET /arbol/version/{hash}/{ruta} (el
#                                       texto de entonces, byte a byte) · 422 · 404
#   8c traer otra rama a la mia          POST /ramas/{rama}/fusionar {desde}: git merge de la persona
#                                       con el gate; main 422 (se propone); conflicto 409
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
  # `ORE_BIN`, si se da, primero: los binarios de otro `CARGO_TARGET_DIR`.
  for n in ${ORE_BIN:+"$ORE_BIN/$1" "$ORE_BIN/$1.exe"} \
           "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
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

# ── 3c · el catalogo EN la rama (ramas globales, fase 1) ───────────────────
BAJAS='apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: bajas_t, namespace: hr }
spec:
  datasource: erp
  object: "bajas.jsonl"
  columns:
    id: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
'
# en una rama suya, que se retira al final: la propuesta de 4 es la de ana/vistas-hr sola
[ "$(pide POST /ramas "$ANA" '{"nombre":"catalogo"}')" = "201" ] || falla "3c · la rama del catalogo: $(cuerpo)"
[ "$(put_fichero packages/hr/tables/bajas_t.yaml "$ANA" "$BAJAS" ana/catalogo)" = "201" ] || falla "3c · la tabla en la rama: $(cuerpo)"
[ "$(pide GET /paquetes/hr/esquema "$ANA" "" ana/catalogo)" = "200" ] && cuerpo | grep -q '"name":"bajas_t"' || falla "3c · el esquema de la rama no trae su tabla: $(cuerpo)"
[ "$(pide GET /paquetes/hr/esquema "$ANA")" = "200" ] && ! cuerpo | grep -q 'bajas_t' || falla "3c · el esquema de main trae la tabla de la rama: $(cuerpo)"
[ "$(pide GET /paquetes "$ANA" "" ana/catalogo)" = "200" ] && tiene "[x['tablas'] for x in d['packages'] if x['name']=='hr']==[2]" || falla "3c · GET /paquetes en la rama no cuenta 2 tablas: $(cuerpo)"
[ "$(pide GET /paquetes "$ANA")" = "200" ] && tiene "[x['tablas'] for x in d['packages'] if x['name']=='hr']==[1]" || falla "3c · GET /paquetes en main no cuenta 1 tabla: $(cuerpo)"
[ "$(pide POST /paquetes/hr/schemas "$ANA" '{"name":"borrador"}' ana/catalogo)" = "201" ] || falla "3c · crear el schema en la rama: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/borrador/schema.yaml "$ANA" "" ana/catalogo)" = "200" ] || falla "3c · la rama no tiene el schema: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/borrador/schema.yaml "$ANA")" = "404" ] || falla "3c · main tiene el schema de la rama: $(cuerpo)"
[ "$(pide POST /paquetes/hr/copia "$ANA" '{}' ana/catalogo)" = "409" ] && cuerpo | grep -q 'mueve datos' || falla "3c · ascender en una rama no dio 409: $(cuerpo)"
[ "$(pide DELETE /ramas/ana/catalogo "$ANA")" = "200" ] || falla "3c · retirar la rama del catalogo: $(cuerpo)"
dice "3c · el catalogo EN la rama: esquema y /paquetes con la tabla de la rama (main sin ella) · el schema nuevo en la rama y no en main · ascender en una rama: 409, mueve datos"

# ── 3d · en que se diferencia la rama de main (ramas globales, fase 2) ──────
# Vistas SQL (v1alpha14): su contrato es lo que `ore diff` compara (OOS5001,
# OOS5007); una vista estructurada se compara por el linaje, y otra cosa.
vista14() { # <nombre> <lee> <columnas...>
  local n=$1 f=$2; shift 2
  local cs; cs=$(printf '%s, ' "$@"); cs=${cs%, }
  printf 'apiVersion: oos.dev/v1alpha14\nkind: View\nmetadata: { name: %s, namespace: hr }\nspec:\n  owner: team:data\n  dialect: duckdb\n  sql: |\n    SELECT %s FROM %s\n  columns:\n' "$n" "$cs" "$f"
  for c in "$@"; do printf '    %s: { type: String }\n' "$c"; done
}
[ "$(put_fichero packages/hr/views/publica.yaml "$ANA" "$(vista14 publica hr.empleados_t id pais)")" = "201" ] || falla "3d · la vista publicada de main: $(cuerpo)"
[ "$(put_fichero packages/hr/views/viejos.yaml "$ANA" "$(vista14 viejos hr.empleados_t id)")" = "201" ] || falla "3d · la vista de main que la rama borra: $(cuerpo)"
[ "$(pide POST /ramas "$ANA" '{"nombre":"cambios"}')" = "201" ] || falla "3d · la rama: $(cuerpo)"
[ "$(put_fichero packages/hr/views/publica.yaml "$ANA" "$(vista14 publica hr.empleados_t id)" ana/cambios)" = "200" ] || falla "3d · quitar pais: $(cuerpo)"
[ "$(put_fichero packages/hr/views/ids.yaml "$ANA" "$(vista14 ids hr.publica id)" ana/cambios)" = "201" ] || falla "3d · la vista nueva que la lee: $(cuerpo)"
[ "$(pide DELETE /arbol/packages/hr/views/viejos.yaml "$ANA" "" ana/cambios)" = "200" ] || falla "3d · borrar en la rama: $(cuerpo)"
# main sigue: lo suyo no es de la rama
[ "$(put_fichero packages/hr/views/despues.yaml "$ANA" "$(vista14 despues hr.empleados_t id)")" = "201" ] || falla "3d · main despues del fork: $(cuerpo)"
[ "$(pide GET /ramas/ana/cambios/cambios "$ANA")" = "200" ] || falla "3d · GET cambios: $(cuerpo)"
cp "$TMP/r.json" "$TMP/cambios.json"
tiene "d['base']=='main' and d['adelante']==3 and d['atras']==1 and d['desde_cache'] is False" || falla "3d · la distancia a main: $(cuerpo | head -c 400)"
tiene "d['resumen']=={'nuevos':1,'modificados':1,'borrados':1,'rompen':2}" || falla "3d · el resumen: $(cuerpo | head -c 900)"
tiene "sorted((c['id'],c['estado']) for c in d['cambios'])==[('View:hr.default.ids','nuevo'),('View:hr.default.publica','modificado'),('View:hr.default.viejos','borrado')]" || falla "3d · los activos: $(cuerpo | head -c 900)"
tiene "[c for c in d['cambios'] if c['id']=='View:hr.default.publica'][0]['columnas']=={'anadidas':[],'quitadas':['pais'],'cambiadas':[]}" || falla "3d · la columna quitada: $(cuerpo | head -c 900)"
tiene "'spec.sql' in [c for c in d['cambios'] if c['id']=='View:hr.default.publica'][0]['campos'] and 'pais' in [c for c in d['cambios'] if c['id']=='View:hr.default.publica'][0]['sql']['antes']" || falla "3d · el sql antes y despues: $(cuerpo | head -c 900)"
tiene "[(s['code'],s['subject']) for s in [c for c in d['cambios'] if c['id']=='View:hr.default.publica'][0]['semantico']]==[('OOS5001','hr.publica.pais')]" || falla "3d · OOS5001 no va con su vista: $(cuerpo | head -c 900)"
tiene "[s['code'] for s in [c for c in d['cambios'] if c['id']=='View:hr.default.viejos'][0]['semantico']]==['OOS5007'] and [s['code'] for s in d['semantico']['otros']]==['OOS5021']" || falla "3d · OOS5007 con la borrada, la version suelta: $(cuerpo | head -c 900)"
tiene "[c['ref'] for c in d['cambios'] if c['id']=='View:hr.default.publica']==['view:hr.publica']" || falla "3d · la ref del indice: $(cuerpo | head -c 600)"
tiene "[c for c in d['cambios'] if c['id']=='View:hr.default.publica'][0]['afecta']==['View:hr.default.ids'] and [c for c in d['cambios'] if c['id']=='View:hr.default.ids'][0]['rompe'] is False" || falla "3d · a quien alcanza: $(cuerpo | head -c 900)"
tiene "all('despues' not in c['id'] for c in d['cambios'])" || falla "3d · lo de main despues del fork sale como de la rama"
[ "$(pide GET /ramas/ana/cambios/cambios "$ANA")" = "200" ] && tiene "d['desde_cache'] is True" || falla "3d · la segunda vez no fue de memoria: $(cuerpo | head -c 200)"
[ "$(pide GET /ramas/main/cambios "$ANA")" = "422" ] || falla "3d · main frente a si misma no dio 422: $(cuerpo)"
[ "$(pide GET /ramas/nadie/nada/cambios "$ANA")" = "404" ] || falla "3d · una rama que no existe no dio 404: $(cuerpo)"
# y se deja como estaba: la rama fuera, main sin lo de esta seccion
[ "$(pide DELETE /ramas/ana/cambios "$ANA")" = "200" ] || falla "3d · retirar la rama: $(cuerpo)"
for f in viejos despues publica; do
  [ "$(pide DELETE "/arbol/packages/hr/views/$f.yaml" "$ANA")" = "200" ] || falla "3d · dejar main como estaba ($f): $(cuerpo)"
done
dice "3d · GET /ramas/{r}/cambios: nuevo, modificado y borrado por activo frente al fork (main despues no cuenta) · la columna quitada y el sql antes/despues · OOS5001 con su vista, OOS5007 con la borrada, la version suelta · a quien alcanza · adelante 3, atras 1 · de memoria la segunda vez · 422 main, 404 sin rama"

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
tiene "len(d['diagnosticos'])>=1 and d['cambios'][0]['estado']=='A' and d['forzable'] is True" || falla "3b · el 422 no trae diagnosticos, cambios y forzable: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/rota.yaml "$ANA" "" ana/vistas-hr)" = "404" ] || falla "3b · el gate escribio igual"
# ⭐ el arbol es de quien lo escribe: forzar: true commitea lo roto igual, y lo dice
"$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); d["forzar"]=True; json.dump(d, open(sys.argv[1],"w"))' "$TMP/commit.json"
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "201" ] || falla "3b · forzar no commiteo: $(cuerpo)"
tiene "d['forzado'] is True and d['nuevos']>=1 and d['cambiados']==1 and len(d['diagnosticos'])>=1 and len(d['commit'])>=7" || falla "3b · la respuesta del commit forzado: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/rota.yaml "$ANA" "" ana/vistas-hr)" = "200" ] || falla "3b · el commit forzado no escribio"
git --git-dir="$BARE" log -1 --format='%an · %s' ana/vistas-hr | grep -q "persona:ana · rompo" || falla "3b · el commit forzado no es de ana con su mensaje"
# en seco tras forzar: el arbol ya tiene lo roto, 0 cambiados y ya no empeora
"$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); d["seco"]=True; json.dump(d, open(sys.argv[1],"w"))' "$TMP/commit.json"
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "200" ] && tiene "d['cambiados']==0" || falla "3b · en seco tras forzar: $(cuerpo)"
# y se arregla retirandolo: el arbol mejora, commit normal
"$PY" -c 'import json,sys; json.dump({"seco": False, "mensaje": "fuera lo roto", "retirar": ["packages/hr/views/rota.yaml"]}, open(sys.argv[1],"w"))' "$TMP/commit.json"
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "201" ] && tiene "d['forzado'] is False and d['cambios']==[{'ruta':'packages/hr/views/rota.yaml','estado':'D','mas':0,'menos':9}] and d['diagnosticos']==[]" || falla "3b · retirar lo roto: $(cuerpo)"
# retirar en el mismo commit
"$PY" -c 'import json,sys; json.dump({"seco": True, "retirar": ["packages/hr/views/portugueses.yaml"]}, open(sys.argv[1],"w"))' "$TMP/commit.json"
[ "$(commit "$ANA" ana/vistas-hr "$TMP/commit.json")" = "200" ] && tiene "d['cambios']==[{'ruta':'packages/hr/views/portugueses.yaml','estado':'D','mas':0,'menos':9}]" || falla "3b · retirar en seco: $(cuerpo)"
dice "3b · POST /arbol/commit: en seco, A/M con +/- de git y nada escrito · sin mensaje 422 · con mensaje, UN commit de la persona con los dos ficheros en la rama · repetido, 0 cambiados · el gate 422 forzable con diagnosticos y cambios · forzar: true commitea lo roto y lo dice (forzado, nuevos) · retirar lo roto, commit normal · retirar sale como D"

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

# ── 8b · el historial de un fichero: git lo tiene (Version history) ─────────
# `espanoles.yaml` nacio en la rama de ana (PUT), cambio en el commit de 3b y llego
# a main por la propuesta #1: tres versiones en main, con la persona de autor.
[ "$(pide GET /arbol/historia/packages/hr/views/espanoles.yaml "$ANA")" = "200" ] || falla "8b · historia: $(cuerpo)"
tiene "d['ruta']=='packages/hr/views/espanoles.yaml' and len(d['versiones'])>=2 and all(v['autor']=='persona:ana' and v['committer']=='ore-serve' and len(v['hash'])==40 and v['cuando'] for v in d['versiones']) and d['versiones'][-1]['mensaje']=='escribir \`packages/hr/views/espanoles.yaml\`'" || falla "8b · las versiones no son las de git: $(cuerpo | head -c 600)"
H=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["versiones"][-1]["hash"])' "$TMP/r.json")
[ "$(pide GET "/arbol/version/$H/packages/hr/views/espanoles.yaml" "$ANA")" = "200" ] || falla "8b · una version: $(cuerpo)"
tiene "d['hash']=='$H' and d['texto'].endswith('id: { from: id, type: String }\n') and 'pais: { from: pais' not in d['texto']" || falla "8b · la primera version no es la de entonces, byte a byte: $(cuerpo | head -c 300)"
[ "$(pide GET /arbol/version/zzz/packages/hr/views/espanoles.yaml "$ANA")" = "422" ] || falla "8b · un hash que no lo es no dio 422"
[ "$(pide GET /arbol/version/0000000000000000000000000000000000000000/packages/hr/views/espanoles.yaml "$ANA")" = "404" ] || falla "8b · un commit que no existe no dio 404"
[ "$(pide GET /arbol/historia/packages/hr/views/nadie.yaml "$ANA")" = "404" ] || falla "8b · un fichero que nunca existio no dio 404"
dice "8b · GET /arbol/historia/{ruta}: las versiones de git con persona, committer y mensaje · GET /arbol/version/{hash}/{ruta}: el texto de entonces, byte a byte · 422 hash malo · 404 commit o fichero que no hay"

# ── 8c · traer OTRA rama a la mia (el «Merge» del menu): git merge con el gate ──
[ "$(pide POST /ramas "$ANA" '{"nombre":"al-dia"}')" = "201" ] || falla "8c · la rama al-dia"
# main avanza (bea fusiono franceses despues de que ana... no: al-dia nace de main de ahora; se hace avanzar main con una rama de bea y una propuesta)
[ "$(pide POST /ramas "$BEA" '{"nombre":"italianos"}')" = "201" ] || falla "8c · la rama de bea"
IT=$(printf '%s' "$NUEVA" | sed 's/name: espanoles/name: italianos/; s/pais: ES/pais: IT/')
[ "$(put_fichero packages/hr/views/italianos.yaml "$BEA" "$IT" bea/italianos)" = "201" ] || falla "8c · el fichero de bea"
[ "$(pide POST /ramas/ana/al-dia/fusionar "$ANA" '{"desde":"bea/italianos"}')" = "200" ] && tiene "d['fusionada'] is True and d['rama']=='ana/al-dia' and d['desde']=='bea/italianos' and d['por']=='persona:ana' and len(d['commit'])>=7" || falla "8c · traer la rama de bea: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/italianos.yaml "$ANA" "" ana/al-dia)" = "200" ] || falla "8c · al-dia no tiene la vista de bea"
[ "$(pide GET /arbol/packages/hr/views/italianos.yaml "$ANA")" = "404" ] || falla "8c · main tiene lo que solo se trajo a una rama"
git --git-dir="$BARE" log -1 --format='%an · %s' ana/al-dia | grep -q 'persona:ana · Traer `bea/italianos` a `ana/al-dia`' || falla "8c · el commit de merge: $(git --git-dir="$BARE" log -1 --format='%an · %s' ana/al-dia)"
[ "$(pide POST /ramas/ana/al-dia/fusionar "$ANA" '{"desde":"bea/italianos"}')" = "200" ] && tiene "d['fusionada'] is False" || falla "8c · traer dos veces no dijo que ya estaba: $(cuerpo)"
[ "$(pide POST /ramas/main/fusionar "$ANA" '{"desde":"bea/italianos"}')" = "422" ] || falla "8c · traer a main no dio 422: $(cuerpo)"
cuerpo | grep -q "se propone" || falla "8c · el 422 de main no dice que se propone: $(cuerpo)"
[ "$(pide POST /ramas/ana/al-dia/fusionar "$ANA" '{"desde":"nadie/rama"}')" = "404" ] || falla "8c · traer una rama que no existe no dio 404: $(cuerpo)"
[ "$(pide POST /ramas/ana/al-dia/fusionar "$ANA" '{"desde":"ana/al-dia"}')" = "422" ] || falla "8c · traerse a si misma no dio 422"
[ "$(pide POST /ramas/ana/al-dia/fusionar "$ANA" '{}')" = "422" ] || falla "8c · sin desde no dio 422"
# un conflicto: las dos ramas cambian el mismo fichero de forma distinta
[ "$(put_fichero packages/hr/views/italianos.yaml "$ANA" "$(printf '%s' "$IT" | sed 's/pais: IT/pais: PT/')" ana/al-dia)" = "200" ] || falla "8c · ana cambia italianos en al-dia: $(cuerpo)"
[ "$(put_fichero packages/hr/views/italianos.yaml "$BEA" "$(printf '%s' "$IT" | sed 's/pais: IT/pais: FR/')" bea/italianos)" = "200" ] || falla "8c · bea cambia italianos en su rama: $(cuerpo)"
[ "$(pide POST /ramas/ana/al-dia/fusionar "$ANA" '{"desde":"bea/italianos"}')" = "409" ] || falla "8c · un conflicto no dio 409: $(cuerpo)"
cuerpo | grep -q "packages/hr/views/italianos.yaml" || falla "8c · el 409 no dice que fichero choca: $(cuerpo)"
dice "8c · POST /ramas/{rama}/fusionar {desde}: git merge de la persona en la rama, main sin tocar · repetido, ya estaba · main 422 (se propone) · 404 rama inexistente · 422 a si misma o sin desde · conflicto 409 con el fichero"

# ── 8d · el upgrade de la plantilla: una RAMA y una PROPUESTA (0036 viii.b) ──
#
# Antes de esto, «Upgrade to v2» reescribia el numero del manifiesto y nada mas:
# el repositorio DECIA v2 y ERA v1. Ahora trae los ficheros de la plantilla de
# hoy en una rama y abre una propuesta, porque esos ficheros los ha editado
# alguien y pisarlos sin ensenar el diff seria borrar trabajo.
[ "$(pide POST /repositorios "$ANA" '{"paquete":"hr","carpeta":"pipelines","nombre":"Pipelines","plantilla":"transforms-python"}')" = "201" ] \
  || falla "8d · el repositorio de la prueba: $(cuerpo)"
# y lo dejamos en la version de antes, como uno creado ayer
# ⭐ La version de hoy se PREGUNTA (0037 iii.a): una semilla que cambia sube
#   la version, y una prueba que persiga el numero se pone roja por decir la
#   verdad. Lo que se comprueba es que el upgrade lleva a la de hoy.
pide GET /assets "$ANA" >/dev/null
VER=$("$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); print([c["version"] for c in d["clases"] if c["id"]=="transforms-python"][0])' "$TMP/r.json")
[ "$(pide PUT /repositorios/packages/hr/pipelines "$ANA" '{"nombre":"Pipelines","plantilla":"transforms-python","plantillaVersion":1}')" = "200" ] \
  || falla "8d · no se pudo dejarlo en la v1: $(cuerpo)"
[ "$(pide GET /assets "$ANA")" = "200" ] \
  && tiene "[r for r in d['repositorios'] if r['ruta']=='packages/hr/pipelines'][0]['actualizable'] is True" \
  || falla "8d · el indice no lo da por actualizable: $(cuerpo)"
[ "$(pide POST /repositorios/packages/hr/pipelines/actualizar "$ANA")" = "201" ] \
  && tiene "d['numero']==4 and d['plantillaVersion']==$VER and d['rama'].startswith('ana/plantilla-') and 'packages/hr/pipelines/README.md' in d['ficheros'] and 'packages/hr/pipelines/pyproject.toml' in d['ficheros']" \
  || falla "8d · el upgrade no abrio propuesta: $(cuerpo)"
# main SIGUE en la v1: proponer no es aplicar
[ "$(pide GET /assets "$ANA")" = "200" ] \
  && tiene "[r for r in d['repositorios'] if r['ruta']=='packages/hr/pipelines'][0]['plantillaVersion']==1" \
  || falla "8d · la propuesta ya habia cambiado main: $(cuerpo)"
# y se acepta como se acepta todo aqui: otra persona revisa y fusiona
[ "$(pide POST /propuestas/4/revisar "$BEA" '{"veredicto":"aprobar"}')" = "201" ] || falla "8d · bea no pudo aprobar: $(cuerpo)"
[ "$(pide POST /propuestas/4/fusionar "$BEA")" = "200" ] || falla "8d · bea no pudo fusionar: $(cuerpo)"
[ "$(pide GET /assets "$ANA")" = "200" ] \
  && tiene "[r for r in d['repositorios'] if r['ruta']=='packages/hr/pipelines'][0]['plantillaVersion']==$VER and [r for r in d['repositorios'] if r['ruta']=='packages/hr/pipelines'][0]['actualizable'] is False" \
  || falla "8d · fusionar no lo dejo en la v$VER: $(cuerpo)"
# ya no hay nada que traer
[ "$(pide POST /repositorios/packages/hr/pipelines/actualizar "$ANA")" = "409" ] \
  || falla "8d · actualizar uno que ya esta al dia no dio 409: $(cuerpo)"
[ "$(pide POST /repositorios/packages/hr/nada/actualizar "$ANA")" = "404" ] \
  || falla "8d · actualizar lo que no es un repositorio no dio 404: $(cuerpo)"
dice "8d · el upgrade de la plantilla: trae los ficheros de la version de hoy EN UNA RAMA y abre propuesta (con el manifiesto dentro) · main sigue en la v1 hasta que se fusiona · fusionada, el indice dice la de hoy y deja de ofrecerla · al dia, 409 · lo que no es un repositorio, 404"

# ── 8e · proponer SOLO lo de un repositorio (0044 A.2 · scope proposals) ────
# Una rama global lleva codigo de un repositorio Y una vista del catalogo. La
# propuesta desde el repositorio lleva solo lo suyo: sale de una derivada
# (main + lo que la rama cambia bajo la carpeta), se revisa sobre su huella, se
# valida main + alcance, y al fusionar la rama se pone al dia y conserva lo demas.
[ "$(pide POST /ramas "$ANA" '{"nombre":"mixta"}')" = "201" ] || falla "8e · la rama: $(cuerpo)"
[ "$(put_fichero packages/hr/pipelines/transforms/uno.py "$ANA" 'print(1)' ana/mixta)" = "201" ] || falla "8e · el codigo en la rama: $(cuerpo)"
[ "$(put_fichero packages/hr/views/fuera.yaml "$ANA" "$(vista14 fuera hr.empleados_t id)" ana/mixta)" = "201" ] || falla "8e · la vista en la rama: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/mixta","titulo":"Solo el repo","alcance":"x"}')" = "422" ] || falla "8e · un alcance malo no dio 422: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/mixta","titulo":"Nada","alcance":"packages/hr/otro"}')" = "422" ] && cuerpo | grep -q 'no cambia nada' || falla "8e · un alcance sin cambios no dio 422: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/mixta","titulo":"Solo el repo","descripcion":"el transform","alcance":"packages/hr/pipelines"}')" = "201" ] || falla "8e · proponer con alcance: $(cuerpo)"
tiene "d['rama']=='ana/mixta' and d['alcance']=='packages/hr/pipelines' and d['derivada']=='alcance/ana/mixta/hr/pipelines' and d['ficherosDelAlcance']==['packages/hr/pipelines/transforms/uno.py'] and d['descripcion']=='el transform' and d['autor']=='persona:ana'" || falla "8e · la propuesta con alcance: $(cuerpo)"
N=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["numero"])' "$TMP/r.json")
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/mixta","titulo":"Otra vez","alcance":"packages/hr/pipelines"}')" = "409" ] || falla "8e · el mismo alcance dos veces no dio 409: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/mixta","titulo":"Toda"}')" = "409" ] || falla "8e · la rama entera con un alcance abierto no dio 409: $(cuerpo)"
[ "$(pide GET /propuestas/$N "$BEA")" = "200" ] || falla "8e · GET la propuesta: $(cuerpo)"
tiene "[f['ruta'] for f in d['ficheros']]==['packages/hr/pipelines/transforms/uno.py'] and d['alDia'] is True and d['diagnosticos']==[]" || falla "8e · la PR no es solo el alcance: $(cuerpo | head -c 700)"
[ "$(pide GET /ramas "$ANA")" = "200" ] && tiene "all(not r['nombre'].startswith('alcance/') for r in d['ramas']) and [r for r in d['ramas'] if r['nombre']=='ana/mixta'][0]['propuesta']==$N" || falla "8e · /ramas ensena la derivada o no sabe la propuesta: $(cuerpo)"
# bea aprueba; ana sigue cambiando el repositorio: lo aprobado ya no es lo que hay
[ "$(pide POST /propuestas/$N/revisar "$BEA" '{"veredicto":"aprobar"}')" = "201" ] || falla "8e · bea no pudo aprobar: $(cuerpo)"
[ "$(put_fichero packages/hr/pipelines/transforms/uno.py "$ANA" 'print(2)' ana/mixta)" = "200" ] || falla "8e · cambiar el codigo en la rama: $(cuerpo)"
[ "$(pide GET /propuestas/$N "$BEA")" = "200" ] && tiene "d['alDia'] is False and d['revisiones'][0]['vigente'] is True" || falla "8e · la propuesta no sabe que la rama cambio: $(cuerpo | head -c 700)"
[ "$(pide POST /propuestas/$N/fusionar "$BEA")" = "409" ] && cuerpo | grep -q 'revisarla otra vez' || falla "8e · fusionar lo que nadie ha visto no dio 409: $(cuerpo)"
[ "$(pide GET /propuestas/$N "$BEA")" = "200" ] && tiene "d['alDia'] is True and d['revisiones'][0]['vigente'] is False" || falla "8e · la derivada no se puso al dia o la revision sigue valiendo: $(cuerpo | head -c 700)"
[ "$(pide POST /propuestas/$N/fusionar "$BEA")" = "422" ] && cuerpo | grep -q 'sobre lo que lleva ahora' || falla "8e · una aprobacion vieja dejo fusionar: $(cuerpo)"
[ "$(pide POST /propuestas/$N/revisar "$BEA" '{"veredicto":"aprobar","texto":"ahora si"}')" = "201" ] || falla "8e · bea no pudo aprobar otra vez: $(cuerpo)"
[ "$(pide POST /propuestas/$N/fusionar "$BEA")" = "200" ] || falla "8e · fusionar con alcance: $(cuerpo)"
tiene "d['fusionada'] is True and d['alcance']=='packages/hr/pipelines' and d['rama']=='ana/mixta' and d['ramaAlDia'] is True" || falla "8e · la fusion con alcance: $(cuerpo)"
# main tiene el codigo de hoy y NO la vista; la rama sigue, al dia, con solo la vista
[ "$(pide GET /arbol/packages/hr/pipelines/transforms/uno.py "$ANA")" = "200" ] && cuerpo | grep -q 'print(2)' || falla "8e · main no tiene el codigo: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/fuera.yaml "$ANA")" = "404" ] || falla "8e · la vista de fuera del alcance llego a main: $(cuerpo)"
[ "$(git --git-dir="$BARE" diff --name-only main ana/mixta)" = "packages/hr/views/fuera.yaml" ] || falla "8e · la rama no quedo con solo lo de fuera: $(git --git-dir="$BARE" diff --name-only main ana/mixta)"
git --git-dir="$BARE" show-ref --verify --quiet refs/heads/alcance/ana/mixta/hr/pipelines && falla "8e · la derivada sigue tras fusionar"
git --git-dir="$BARE" log -1 --format=%s main | grep -q "packages/hr/pipelines de ana/mixta), revisada por persona:bea" || falla "8e · el commit de merge no dice el alcance: $(git --git-dir="$BARE" log -1 --format=%s main)"
# un documento del catalogo DENTRO de la carpeta es un activo: se queda en la rama
# (se propone desde el catalogo); lo que no es documento (un .json) va
[ "$(pide POST /ramas "$ANA" '{"nombre":"docs"}')" = "201" ] || falla "8e · la rama docs: $(cuerpo)"
[ "$(put_fichero packages/hr/pipelines/lee.yaml "$ANA" "$(vista14 lee hr.empleados_t id)" ana/docs)" = "201" ] || falla "8e · la vista dentro del repositorio: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/docs","titulo":"Solo un documento","alcance":"packages/hr/pipelines"}')" = "422" ] && cuerpo | grep -q 'no cambia nada' || falla "8e · un alcance con solo un documento del catalogo no dio 422: $(cuerpo)"
[ "$(put_fichero packages/hr/pipelines/transforms/dos.py "$ANA" 'print(3)' ana/docs)" = "201" ] || falla "8e · el codigo de docs: $(cuerpo)"
[ "$(put_fichero packages/hr/pipelines/config.json "$ANA" '{"retries": 3}' ana/docs)" = "201" ] || falla "8e · la configuracion del repositorio: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/docs","titulo":"El codigo, sin la vista","alcance":"packages/hr/pipelines"}')" = "201" ] || falla "8e · proponer docs: $(cuerpo)"
tiene "sorted(d['ficherosDelAlcance'])==['packages/hr/pipelines/config.json','packages/hr/pipelines/transforms/dos.py'] and d['documentosFuera']==['packages/hr/pipelines/lee.yaml']" || falla "8e · el alcance lleva el documento del catalogo: $(cuerpo)"
M=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["numero"])' "$TMP/r.json")
[ "$(pide POST /propuestas/$M/revisar "$BEA" '{"veredicto":"aprobar"}')" = "201" ] || falla "8e · aprobar docs: $(cuerpo)"
[ "$(pide POST /propuestas/$M/fusionar "$BEA")" = "200" ] || falla "8e · fusionar docs: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/pipelines/transforms/dos.py "$ANA")" = "200" ] || falla "8e · main no tiene el codigo de docs"
[ "$(pide GET /arbol/packages/hr/pipelines/lee.yaml "$ANA")" = "404" ] || falla "8e · el documento del catalogo llego a main por la propuesta del repositorio"
[ "$(git --git-dir="$BARE" diff --name-only main ana/docs)" = "packages/hr/pipelines/lee.yaml" ] || falla "8e · la rama docs no quedo con solo el documento: $(git --git-dir="$BARE" diff --name-only main ana/docs)"
# un fichero movido de dentro a fuera del repositorio no se parte: 409
[ "$(pide POST /ramas "$ANA" '{"nombre":"mueve"}')" = "201" ] || falla "8e · la rama mueve: $(cuerpo)"
"$PY" - "$TMP/mueve.json" <<'EOF'
import json, sys
json.dump({"mensaje": "uno.py sale del repositorio",
           "ficheros": [{"ruta": "packages/hr/scripts/uno.py", "texto": "print(2)"}],
           "retirar": ["packages/hr/pipelines/transforms/uno.py"]}, open(sys.argv[1], "w"))
EOF
commit "$ANA" ana/mueve "$TMP/mueve.json" >/dev/null
git --git-dir="$BARE" diff --name-status -M main ana/mueve | grep -q '^R.*packages/hr/pipelines/transforms/uno.py.*packages/hr/scripts/uno.py' || falla "8e · mover en la rama: $(cuerpo) · $(git --git-dir="$BARE" diff --name-status -M main ana/mueve)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/mueve","titulo":"Se lleva uno.py","alcance":"packages/hr/pipelines"}')" = "409" ] && cuerpo | grep -q 'borde del repositorio' || falla "8e · un movimiento que cruza el borde no dio 409: $(cuerpo)"
git --git-dir="$BARE" show-ref --verify --quiet refs/heads/alcance/ana/mueve/hr/pipelines && falla "8e · quedo una derivada del 409"
[ "$(pide GET /arbol/packages/hr/pipelines/transforms/uno.py "$ANA")" = "200" ] || falla "8e · main perdio uno.py"
dice "8e · scope proposals por repositorio: la PR lleva solo el repositorio (una derivada, fuera de /ramas) · 422 alcance malo o sin cambios · 409 el mismo alcance o la rama entera · la rama cambia lo propuesto: 409, la derivada se regenera y la aprobacion vieja no vale · fusionada: main con el codigo de hoy y sin la vista, la rama al dia con solo la vista, la derivada fuera, el merge dice el alcance · un documento del catalogo dentro de la carpeta se queda en la rama (solo el: 422), lo que no es documento va · mover a traves del borde: 409"

# ── 8f · proponer UNOS ACTIVOS desde el catalogo (0044 A.2 · E2) ─────────────
# La misma propuesta que la del repositorio, con otro alcance: unos activos por
# su id. Viajan sus ficheros (y, movido, el de antes); lo que comparte fichero y
# el package.yaml de su base van con ellos; main + alcance se valida y, si le
# falta algo que la rama cambia, se dice cual (faltan); un consumidor de main que
# rompe no se arregla anadiendo: el alcance romperia main.
num() { "$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["numero"])' "$TMP/r.json"; }
[ "$(pide POST /ramas "$ANA" '{"nombre":"activos"}')" = "201" ] || falla "8f · la rama: $(cuerpo)"
[ "$(put_fichero packages/hr/views/a1.yaml "$ANA" "$(vista14 a1 hr.empleados_t id)" ana/activos)" = "201" ] || falla "8f · a1: $(cuerpo)"
[ "$(put_fichero packages/hr/views/a2.yaml "$ANA" "$(vista14 a2 hr.a1 id)" ana/activos)" = "201" ] || falla "8f · a2 (lee a1): $(cuerpo)"
[ "$(put_fichero packages/hr/views/a3.yaml "$ANA" "$(vista14 a3 hr.empleados_t id pais)" ana/activos)" = "201" ] || falla "8f · a3: $(cuerpo)"
[ "$(put_fichero packages/hr/pipelines/transforms/tres.py "$ANA" 'print(4)' ana/activos)" = "201" ] || falla "8f · el codigo: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"x","activos":["View:hr.default.nada"]}')" = "422" ] || falla "8f · un activo que la rama no cambia no dio 422: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"x","activos":["View:hr.default.a3"],"alcance":"packages/hr/pipelines"}')" = "422" ] || falla "8f · carpeta y activos a la vez no dio 422: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"Solo a3","activos":["View:hr.default.a3"]}')" = "201" ] || falla "8f · proponer a3: $(cuerpo)"
tiene "d['activos']==['View:hr.default.a3'] and d['ficherosDelAlcance']==['packages/hr/views/a3.yaml'] and d['faltan']==[] and d['anadidos']==[] and d['derivada'].startswith('alcance/ana/activos/activos-') and d['alcance'] is False" || falla "8f · la propuesta de a3: $(cuerpo)"
N3=$(num)
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"Otra vez","activos":["view:hr.a3"]}')" = "409" ] || falla "8f · el mismo activo (por su ref) dos veces no dio 409: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"Toda"}')" = "409" ] || falla "8f · la rama entera con activos abiertos no dio 409: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"El repo","alcance":"packages/hr/pipelines"}')" = "201" ] || falla "8f · el repositorio y los activos no conviven: $(cuerpo)"
R=$(num); [ "$(pide DELETE /propuestas/$R "$ANA")" = "200" ] || falla "8f · cerrar la del repositorio: $(cuerpo)"
# a2 lee a1, que solo esta en la rama: se propone, pero dice que falta a1
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"Solo a2","activos":["View:hr.default.a2"]}')" = "201" ] || falla "8f · proponer a2: $(cuerpo)"
tiene "d['faltan']==['View:hr.default.a1'] and any(x['codigo']=='OOS2018' for x in d['diagnosticos'])" || falla "8f · a2 sin a1 no dice que falta: $(cuerpo | head -c 700)"
N2=$(num)
[ "$(pide GET /propuestas/$N2 "$BEA")" = "200" ] && tiene "d['faltan']==['View:hr.default.a1']" || falla "8f · el detalle no dice que falta: $(cuerpo | head -c 500)"
[ "$(pide POST /propuestas/$N2/revisar "$BEA" '{"veredicto":"aprobar"}')" = "201" ] || falla "8f · aprobar a2: $(cuerpo)"
[ "$(pide POST /propuestas/$N2/fusionar "$BEA")" = "422" ] && tiene "d['faltan']==['View:hr.default.a1']" || falla "8f · fusionar a2 sin a1: $(cuerpo | head -c 500)"
[ "$(pide DELETE /propuestas/$N2 "$ANA")" = "200" ] || falla "8f · cerrar a2: $(cuerpo)"
# la version del paquete va con sus activos
[ "$(pide GET /arbol/packages/hr/package.yaml "$ANA" "" ana/activos)" = "200" ] || falla "8f · leer package.yaml: $(cuerpo)"
PKG=$("$PY" -c 'import re,sys,json; print(re.sub(r"version: [0-9.]+", "version: 9.9.9", json.load(open(sys.argv[1]))["texto"]), end="")' "$TMP/r.json")
[ "$(put_fichero packages/hr/package.yaml "$ANA" "$PKG" ana/activos)" = "200" ] || falla "8f · subir la version en la rama: $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/activos","titulo":"a1 y a2","activos":["View:hr.default.a1","View:hr.default.a2"]}')" = "201" ] || falla "8f · proponer a1 y a2: $(cuerpo)"
tiene "d['faltan']==[] and d['diagnosticos']==[] and d['anadidos']==['Package:hr'] and 'packages/hr/package.yaml' in d['ficherosDelAlcance'] and sorted(d['activos'])==['Package:hr','View:hr.default.a1','View:hr.default.a2']" || falla "8f · a1+a2 no llevan el paquete o no compilan: $(cuerpo | head -c 700)"
N12=$(num)
# fusionar a3: main tiene a3 y nada mas de la rama
[ "$(pide POST /propuestas/$N3/revisar "$BEA" '{"veredicto":"aprobar"}')" = "201" ] || falla "8f · aprobar a3: $(cuerpo)"
[ "$(pide POST /propuestas/$N3/fusionar "$BEA")" = "200" ] && tiene "d['ramaAlDia'] is True and d['alcance']=='View:hr.default.a3'" || falla "8f · fusionar a3: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/a3.yaml "$ANA")" = "200" ] || falla "8f · main no tiene a3"
[ "$(pide GET /arbol/packages/hr/views/a1.yaml "$ANA")" = "404" ] || falla "8f · a1 llego a main sin proponerse"
[ "$(git --git-dir="$BARE" diff --name-only main ana/activos | sort | tr '\n' ' ')" = "packages/hr/package.yaml packages/hr/pipelines/transforms/tres.py packages/hr/views/a1.yaml packages/hr/views/a2.yaml " ] || falla "8f · la rama no quedo con lo demas: $(git --git-dir="$BARE" diff --name-only main ana/activos)"
[ "$(pide DELETE /propuestas/$N12 "$ANA")" = "200" ] || falla "8f · cerrar a1+a2: $(cuerpo)"
# un consumidor de main que rompe: no hay nada que anadir, el alcance romperia main
[ "$(put_fichero packages/hr/views/b1.yaml "$ANA" "$(vista14 b1 hr.empleados_t id pais)")" = "201" ] || falla "8f · b1 en main: $(cuerpo)"
[ "$(put_fichero packages/hr/views/b2.yaml "$ANA" "$(vista14 b2 hr.b1 pais)")" = "201" ] || falla "8f · b2 en main (lee pais de b1): $(cuerpo)"
[ "$(pide POST /ramas "$ANA" '{"nombre":"rompe"}')" = "201" ] || falla "8f · la rama rompe: $(cuerpo)"
"$PY" - "$TMP/rompe.json" "$(vista14 b1 hr.empleados_t id)" "$(vista14 b2 hr.b1 id)" <<'EOF'
import json, sys
json.dump({"mensaje": "b1 sin pais, y b2 que ya no lo lee", "forzar": True,
           "ficheros": [{"ruta": "packages/hr/views/b1.yaml", "texto": sys.argv[2] + "\n"}]}, open(sys.argv[1], "w"))
EOF
commit "$ANA" ana/rompe "$TMP/rompe.json" >/dev/null
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/rompe","titulo":"b1 sin pais","activos":["View:hr.default.b1"]}')" = "201" ] || falla "8f · proponer b1: $(cuerpo)"
tiene "d['faltan']==[] and any(x['codigo']=='OOS2018' and 'b2' in x['mensaje'] for x in d['diagnosticos'])" || falla "8f · el consumidor roto de main no se dice: $(cuerpo | head -c 700)"
NB=$(num); [ "$(pide DELETE /propuestas/$NB "$ANA")" = "200" ] || falla "8f · cerrar b1: $(cuerpo)"
# un activo movido viaja entero: el fichero de antes sale de main
[ "$(pide POST /ramas "$ANA" '{"nombre":"mueve-a3"}')" = "201" ] || falla "8f · la rama mueve-a3: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/a3.yaml "$ANA")" = "200" ] || falla "8f · leer a3"
"$PY" -c 'import json,sys; open(sys.argv[2],"w").write(json.load(open(sys.argv[1]))["texto"])' "$TMP/r.json" "$TMP/a3.yaml"
"$PY" - "$TMP/mueve-a3.json" "$TMP/a3.yaml" <<'EOF'
import json, sys
json.dump({"mensaje": "a3 cambia de fichero",
           "ficheros": [{"ruta": "packages/hr/views/a3_movida.yaml", "texto": open(sys.argv[2]).read()}],
           "retirar": ["packages/hr/views/a3.yaml"]}, open(sys.argv[1], "w"))
EOF
commit "$ANA" ana/mueve-a3 "$TMP/mueve-a3.json" >/dev/null
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/mueve-a3","titulo":"a3 se muda","activos":["View:hr.default.a3"]}')" = "201" ] || falla "8f · proponer el movido: $(cuerpo)"
NM=$(num)
[ "$(pide POST /propuestas/$NM/revisar "$BEA" '{"veredicto":"aprobar"}')" = "201" ] || falla "8f · aprobar el movido: $(cuerpo)"
[ "$(pide POST /propuestas/$NM/fusionar "$BEA")" = "200" ] || falla "8f · fusionar el movido: $(cuerpo)"
[ "$(pide GET /arbol/packages/hr/views/a3_movida.yaml "$ANA")" = "200" ] && [ "$(pide GET /arbol/packages/hr/views/a3.yaml "$ANA")" = "404" ] || falla "8f · el movimiento no viajo entero"
dice "8f · scope proposals por activos: solo a3 llega a main (la rama sigue con a1, a2, el paquete y el codigo) · 422 activo que la rama no cambia o carpeta y activos a la vez · 409 el mismo activo (por id o ref) o la rama entera; el repositorio convive · a2 sin a1: faltan [a1] al proponer, en el detalle y al fusionar (422) · el package.yaml cambiado va con sus activos · un consumidor de main roto: diagnostico sin faltan · un activo movido viaja entero"

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
