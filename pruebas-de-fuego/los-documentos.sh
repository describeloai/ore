#!/usr/bin/env bash
# LOS DOCUMENTOS DEL ARBOL (Ontology Forge I1): `Entity`, contra un `ore-serve`
# de verdad y con el arbol en una forja.
#
# Forge no lee ficheros: lee `ore-serve`. Y hasta hoy `/esquema` daba nombre,
# clave, respaldo y propiedades — sin `labels` ni `relations`, asi que Entities
# no podia pintar sensibilidad y Links no podia pintar aristas. Esto fija que
# `/documentos/Entity` da el documento ENTERO, y que escribirlo tiene la misma
# figura que `/fuentes` y `/modelos`: clonar, escribir, compilar, empujar con
# el sujeto, y 409 cuando el arbol se movio.
#
# El arbol es `acme-retail`, la ontologia de referencia de OOS, sembrada en un
# repositorio pelado con `file://` (como `servidor-forja.sh`).
#
#   1  GET /documentos/Entity            todas, con `metadata` y `spec` enteros:
#                                        labels, aiContext, relations, uniqueKeys,
#                                        temporal, moved, reserved; y `paquete`
#   2  GET /documentos/Entity/hr/Employee  una, con su YAML y el commit que la trajo
#      …/hr/NoExiste                     404
#   3  PUT valida (con `x-rubix-displayName`)   201 · commit · el autor es el
#                                        sujeto · la lista la ve · el YAML la
#                                        lleva llana y el arbol compila
#   4  PUT sin `backedBy`                422 y la forja NO cambio (lo exige el
#                                        verbo: el compilador aun lo admite)
#   5  PUT con una propiedad que la vista no expone   422 · OOS2022, con
#                                        `donde` y `ayuda`
#   6  PUT con `displayName` sin prefijo  422 · OOS1005
#   7  dos PUT sobre el mismo commit     el primero 200, el segundo 409 y la
#                                        forja se queda en el primero
#   8  DELETE referenciada (Department)  409 con quien la nombra · sigue ahi
#   9  DELETE sin nadie que la nombre    200 · commit · GET 404 despues
#
# Uso:  bash pruebas-de-fuego/los-documentos.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8909}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""
PY=$(command -v python3 || command -v python)

falla() {
  echo "✗ $*" >&2
  [ -s "$TMP/arranque.txt" ] && { echo "── lo que dijo el servidor ──" >&2; tail -20 "$TMP/arranque.txt" >&2; }
  limpiar; exit 1
}
dice()  { echo "  · $*"; }
limpiar() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  rm -rf "$TMP"
}
trap limpiar EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"         || falla "no hay binario de \`ore\`"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\`"
[ -n "$PY" ] || falla "hace falta python"

export GIT_AUTHOR_NAME=semilla GIT_AUTHOR_EMAIL=semilla@x
export GIT_COMMITTER_NAME=semilla GIT_COMMITTER_EMAIL=semilla@x

# ── La forja, sembrada con acme-retail ──────────────────────────────────────
FORJA="$TMP/arbol.git"
git init -q --bare -b main "$FORJA" || falla "no se pudo crear el repositorio pelado"
git clone -q "$FORJA" "$TMP/semilla" 2>/dev/null
cp -r "$RAIZ/vendor/oos/examples/acme-retail/." "$TMP/semilla/"
( cd "$TMP/semilla" && git config core.autocrlf false && "$ORE" validate . >/dev/null 2>&1 \
  && git add -A && git commit -qm "acme-retail" && git push -q origin HEAD:main ) \
  || falla "no se pudo sembrar la forja con acme-retail"
cabeza() { git --git-dir="$FORJA" rev-parse main; }
asunto() { git --git-dir="$FORJA" log -1 --format='%s' main; }
dice "0 · forja sembrada con acme-retail: $(git --git-dir="$FORJA" rev-parse --short main)"

FORJA_TOKEN=no-hace-falta-en-file "$SERVE" \
  --forja "file://$FORJA" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
  --identidad cabecera --no-es-produccion > "$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done
curl -sf "$BASE/version" | grep -q '"arbol":"forja"' || falla "0 · el servidor no arranco en modo forja"

SUJ='x-ore-sujeto: persona:ana'
pide() { # metodo ruta [cuerpo] [cabecera-extra]
  local m="$1" r="$2" c="${3:-}" h="${4:-x-nada: 1}"
  if [ -n "$c" ]; then
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$m" -H "$SUJ" -H "$h" \
      -H 'Content-Type: application/json' -d "$c" "$BASE$r"
  else
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$m" -H "$SUJ" -H "$h" "$BASE$r"
  fi
}
# Una expresion de python sobre la respuesta, o falla con la respuesta entera.
cumple() { "$PY" -c "import json,sys; d=json.load(open(sys.argv[1])); assert ($1), sys.argv[2]" "$TMP/r.json" "$2" \
  || falla "$2 · $(head -c 600 "$TMP/r.json")"; }
campo() { "$PY" -c "import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))" "$TMP/r.json" "$1"; }

# ── 1 · la lista, con el documento entero ───────────────────────────────────
[ "$(pide GET /documentos/Entity)" = "200" ] || falla "1 · GET /documentos/Entity · $(cat "$TMP/r.json")"
cumple "len(d['documentos']) == 7" "1 · acme-retail tiene 7 entidades"
cumple "'ilegibles' not in d" "1 · ninguna ilegible"
cumple "all(x['kind']=='Entity' and x['paquete'] and x['namespace'] and x['fichero'] for x in d['documentos'])" "1 · cada una dice kind, paquete, namespace y fichero"
cumple "[x for x in d['documentos'] if x['namespace']=='hr' and x['name']=='Employee'][0]['metadata']['labels']=={'acme.residency':'eu_only'}" "1 · Employee trae sus labels"
cumple "[x for x in d['documentos'] if x['name']=='Employee'][0]['spec']['relations']['department']['target']=='hr.Department'" "1 · Employee trae sus relations"
cumple "set([x for x in d['documentos'] if x['name']=='Employee'][0]['spec']) >= {'backedBy','uniqueKeys','temporal','moved','reserved','properties','primaryKey','nature'}" "1 · el spec va entero"
cumple "[x for x in d['documentos'] if x['name']=='Employee'][0]['metadata']['aiContext']['synonyms'][0]=='empleado'" "1 · y el metadata tambien (aiContext)"
cumple "[x for x in d['documentos'] if x['name']=='Employee'][0]['spec']['properties']['nationalId']['labels']=={'gdpr.sensitivity':'critical'}" "1 · las labels por propiedad"
CON_LABELS=$(campo "sum(1 for x in d['documentos'] if x['metadata'].get('labels'))")
CON_REL=$(campo "sum(1 for x in d['documentos'] if x['spec'].get('relations'))")
dice "1 · 7 entidades con metadata y spec enteros: $CON_LABELS con labels, $CON_REL con relations"

# ── 2 · una, con su YAML y su commit ────────────────────────────────────────
[ "$(pide GET /documentos/Entity/hr/Employee)" = "200" ] || falla "2 · GET una · $(cat "$TMP/r.json")"
cumple "d['yaml'].startswith('apiVersion: oos.dev/v1alpha8') and 'kind: Entity' in d['yaml']" "2 · trae su YAML tal cual"
cumple "d['commit']['autor']=='semilla' and len(d['commit']['hash'])>=7 and d['commit']['fecha']" "2 · y el commit que la trajo"
cumple "d['fichero']=='packages/hr/entities/Employee.yaml'" "2 · y donde esta"
TRAJO="$(campo "d['commit']['hash']") · $(campo "d['commit']['autor']")"
[ "$(pide GET /documentos/Entity/hr/NoExiste)" = "404" ] || falla "2 · una que no existe no dio 404"
[ "$(pide GET /documentos/Entity/hr/../x)" != "200" ] || falla "2 · un nombre con .. paso"
dice "2 · una entidad trae su YAML, su fichero y el commit ($TRAJO)"

# ── 3 · PUT valida, con la extension de presentacion ────────────────────────
CONTRACTOR='{"metadata":{"description":"Personal externo con contrato mercantil.","x-rubix-displayName":"Contratista","labels":{"acme.residency":"eu_only"}},
  "spec":{"nature":"entity","primaryKey":["employeeId"],"backedBy":"empleados",
    "properties":{"employeeId":{"type":"String"},"fullName":{"type":"String","labels":{"gdpr.sensitivity":"high"}},"departmentId":{"type":"String"}},
    "relations":{"department":{"target":"hr.Department","cardinality":"many_to_one","via":["departmentId"],"required":true}}}}'
ANTES=$(cabeza)
COD=$(pide PUT /documentos/Entity/hr/Contractor "$CONTRACTOR")
[ "$COD" = "201" ] || falla "3 · PUT valida devolvio $COD · $(cat "$TMP/r.json")"
cumple "d['nueva'] is True and d['commit'] and d['fichero']=='packages/hr/entities/Contractor.yaml'" "3 · 201 con commit y fichero"
[ "$(cabeza)" != "$ANTES" ] || falla "3 · la forja no avanzo"
[ "$(git --git-dir="$FORJA" log -1 --format='%an' main)" = "persona:ana" ] || falla "3 · el autor no es el sujeto"
[ "$(git --git-dir="$FORJA" log -1 --format='%cn' main)" = "ore-serve" ]   || falla "3 · el committer no es el servidor"
[ "$(asunto)" = 'escribir la entidad `hr.Contractor`' ] || falla "3 · el asunto del commit: $(asunto)"
[ "$(pide GET /documentos/Entity/hr/Contractor)" = "200" ] || falla "3 · la nueva no se lee"
cumple "d['metadata']['x-rubix-displayName']=='Contratista' and d['spec']['backedBy']=='empleados'" "3 · con su extension y su respaldo"
cumple "'x-rubix-displayName: Contratista' in d['yaml'] and 'target: hr.Department' in d['yaml'] and 'description: Personal externo' in d['yaml']" "3 · el YAML sale llano, no entrecomillado"
cumple "d['commit']['autor']=='persona:ana'" "3 · y su commit lleva al sujeto"
[ "$(pide GET /documentos/Entity)" = "200" ] && cumple "len(d['documentos']) == 8" "3 · la lista la ve"
git clone -q "$FORJA" "$TMP/comprobar" 2>/dev/null && ( cd "$TMP/comprobar" && "$ORE" validate . >/dev/null 2>&1 ) \
  || falla "3 · el arbol de la forja no compila tras el PUT"
rm -rf "$TMP/comprobar"
dice "3 · PUT valida: 201, commit de persona:ana, la lista la ve, el YAML llano, y el arbol compila"

# ── 4 · sin backedBy: lo exige el verbo ─────────────────────────────────────
ANTES=$(cabeza)
SIN=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); del d['spec']['backedBy']; print(json.dumps(d))" "$CONTRACTOR")
[ "$(pide PUT /documentos/Entity/hr/Contratista2 "$SIN")" = "422" ] || falla "4 · sin backedBy no dio 422 · $(cat "$TMP/r.json")"
grep -q 'backedBy' "$TMP/r.json" || falla "4 · el 422 no nombra backedBy"
[ "$(cabeza)" = "$ANTES" ] || falla "4 · un 422 movio la forja"
dice "4 · sin backedBy: 422 con el motivo, y la forja no se movio"

# ── 5 · una propiedad que la vista no expone: OOS2022 ───────────────────────
MAL=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); d['spec']['properties']['inventada']={'type':'String'}; print(json.dumps(d))" "$CONTRACTOR")
[ "$(pide PUT /documentos/Entity/hr/Contractor "$MAL")" = "422" ] || falla "5 · no dio 422 · $(cat "$TMP/r.json")"
cumple "d['diagnosticos'][0]['codigo']=='OOS2022' and 'inventada' in d['diagnosticos'][0]['mensaje'] and d['diagnosticos'][0]['donde'].startswith('packages/hr/entities/Contractor.yaml') and d['diagnosticos'][0]['ayuda']" "5 · OOS2022 con donde y ayuda"
cumple "'OOS2022' in d['error']" "5 · y el error lo resume"
[ "$(cabeza)" = "$ANTES" ] || falla "5 · un 422 movio la forja"
[ "$(pide GET /documentos/Entity/hr/Contractor)" = "200" ] && cumple "'inventada' not in d['spec']['properties']" "5 · la de antes sigue intacta"
dice "5 · una propiedad que la vista no expone: 422 OOS2022, con donde y ayuda"

# ── 6 · displayName sin prefijo: OOS1005 ────────────────────────────────────
MAL=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); d['metadata']['displayName']='Contratista'; print(json.dumps(d))" "$CONTRACTOR")
[ "$(pide PUT /documentos/Entity/hr/Contractor "$MAL")" = "422" ] || falla "6 · no dio 422 · $(cat "$TMP/r.json")"
cumple "d['diagnosticos'][0]['codigo']=='OOS1005' and 'displayName' in d['diagnosticos'][0]['mensaje']" "6 · OOS1005"
dice "6 · displayName sin prefijo: 422 OOS1005 (x-rubix-displayName si pasa: es el 3)"

# ── 7 · dos PUT sobre el mismo commit ───────────────────────────────────────
LEIDO=$(cabeza)
V2=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); d['metadata']['description']='Personal externo. Version 2.'; print(json.dumps(d))" "$CONTRACTOR")
V3=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); d['metadata']['description']='Personal externo. Version 3.'; print(json.dumps(d))" "$CONTRACTOR")
[ "$(pide PUT /documentos/Entity/hr/Contractor "$V2" "If-Match: $LEIDO")" = "200" ] || falla "7 · el primer PUT sobre $LEIDO no dio 200 · $(cat "$TMP/r.json")"
cumple "d['nueva'] is False and d['commit']" "7 · reescribir es 200 con commit"
PRIMERO=$(cabeza)
[ "$PRIMERO" != "$LEIDO" ] || falla "7 · el primer PUT no avanzo la forja"
[ "$(pide PUT /documentos/Entity/hr/Contractor "$V3" "If-Match: $LEIDO")" = "409" ] || falla "7 · el segundo PUT sobre el mismo commit no dio 409 · $(cat "$TMP/r.json")"
grep -q 'se movió' "$TMP/r.json" || falla "7 · el 409 no dice que el arbol se movio"
[ "$(cabeza)" = "$PRIMERO" ] || falla "7 · el segundo PUT movio la forja"
[ "$(pide GET /documentos/Entity/hr/Contractor)" = "200" ] && cumple "d['metadata']['description'].endswith('Version 2.')" "7 · queda la version del primero"
# y con la cabeza al dia, el mismo cambio entra
[ "$(pide PUT /documentos/Entity/hr/Contractor "$V3" "If-Match: $PRIMERO")" = "200" ] || falla "7 · con If-Match al dia no entro · $(cat "$TMP/r.json")"
dice "7 · dos PUT sobre el mismo commit: 200 y 409, la forja se queda en el primero; releer y volver a escribir entra"

# ── 8 · DELETE de una referenciada ──────────────────────────────────────────
ANTES=$(cabeza)
[ "$(pide DELETE /documentos/Entity/hr/Department)" = "409" ] || falla "8 · DELETE referenciada no dio 409 · $(cat "$TMP/r.json")"
grep -q 'hr.Employee' "$TMP/r.json" && grep -q 'hr.Contractor' "$TMP/r.json" && grep -q 'relations.department' "$TMP/r.json" \
  || falla "8 · el 409 no nombra a quien la referencia · $(cat "$TMP/r.json")"
[ "$(cabeza)" = "$ANTES" ] || falla "8 · un 409 movio la forja"
[ "$(pide GET /documentos/Entity/hr/Department)" = "200" ] || falla "8 · Department desaparecio"
[ "$(pide DELETE /documentos/Entity/hr/NoExiste)" = "404" ] || falla "8 · retirar una que no existe no dio 404"
dice "8 · retirar una referenciada: 409 con quien la nombra, y sigue ahi"

# ── 9 · DELETE de una que nadie nombra ──────────────────────────────────────
[ "$(pide DELETE /documentos/Entity/hr/Contractor "" "If-Match: $(cabeza)")" = "200" ] || falla "9 · DELETE no dio 200 · $(cat "$TMP/r.json")"
cumple "d['retirada'] is True and d['commit']" "9 · 200 con commit"
[ "$(asunto)" = 'retirar la entidad `hr.Contractor`' ] || falla "9 · el asunto del commit: $(asunto)"
[ "$(pide GET /documentos/Entity/hr/Contractor)" = "404" ] || falla "9 · retirada y se sigue leyendo"
[ "$(pide GET /documentos/Entity)" = "200" ] && cumple "len(d['documentos']) == 7" "9 · la lista vuelve a 7"
dice "9 · retirar una que nadie nombra: 200, commit, y deja de leerse"

echo
echo "ok · /documentos/Entity: la ficha entera, y escribirla es un commit del sujeto que compila antes de empujar"
