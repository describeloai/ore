#!/usr/bin/env bash
# LOS DOCUMENTOS DEL ARBOL (Ontology Forge): `/documentos/{kind}` — un motor y
# una tabla de kinds (Entity, View, Table, Concept, Interface) — y `/conceptos`,
# contra un `ore-serve` de verdad y con el arbol en una forja.
#
# Forge no lee ficheros: lee `ore-serve`. Y hasta hoy `/esquema` daba nombre,
# clave, respaldo y propiedades — sin `labels` ni `relations`, asi que Entities
# no podia pintar sensibilidad y Links no podia pintar aristas. Esto fija que
# `/documentos/{kind}` da el documento ENTERO, y que escribirlo tiene la misma
# figura que `/fuentes` y `/modelos`: clonar, escribir, compilar, empujar con
# el sujeto, 409 cuando el arbol se movio — y la puerta es «el arbol no
# empeora», no «el arbol compila».
#
# El arbol es `acme-retail`, la ontologia de referencia de OOS, sembrada en un
# repositorio pelado con `file://` (como `servidor-forja.sh`).
#
# Los diez primeros son de Entity, la primera fila de la tabla; el 10 fija la
# puerta; 11-14 son View y Table por el mismo motor:
#
#  10  el arbol roto por otro lado    lo valido entra (201), lo invalido sale con
#                                     SOLO lo suyo, la referenciada sigue en 409
#  11  GET /documentos/View · Table   enteras; un kind fuera de la tabla, 404 con
#                                     la lista de los servidos
#  12  PUT View                       201 y el MISMO `plan sha256` que la misma
#                                     vista por `ore view add`; sin owner 422;
#                                     columna que la tabla no tiene, OOS2018
#  13  PUT con `yaml` tal cual        se guarda con sus comentarios; el nombre lo
#                                     pone la ruta
#  14  DELETE View · Table            409 con quien la nombra (backedBy, from.view,
#                                     from.table); libres, 200
#  15  Concept · Interface · /conceptos  vacios en acme-retail; /conceptos trae
#                                     los importados de vendor/iso.oob
#  16  PUT Concept                    entra sin que nadie lo hable (sinHablar,
#                                     OOS9004 tolerado); sin type 422 (OOS1004);
#                                     con `is`, /conceptos dice quien lo habla
#  17  PUT Interface · DELETE         requires a lo que no esta OOS2001; 409 con
#                                     properties.*.is, requires, implements
#  18  deshacer en orden              dejar de hablar un concepto entra (y lo
#                                     dice); libres, 200; el commit es del sujeto
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
# Un vocabulario importado (`ore pack` → vendor/iso.oob) para que /conceptos tenga
# algo que no es del arbol. `iso` y no el `gdpr` de conformidad: ese trae el lattice
# `gdpr.sensitivity`, que acme-retail ya declara — y desde OOS2035 eso son dos.
mkdir -p "$TMP/iso/concepts"
printf 'apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: iso, version: 1.0.0, status: active, domain: standards }\nspec: { owner: team:standards }\n' > "$TMP/iso/package.yaml"
printf 'apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: iso, version: 0.1.0 }\ndatasources:\n  - { name: ninguna, type: postgres, connectionEnv: NADA }\n' > "$TMP/iso/ontology.config.yaml"
printf 'apiVersion: oos.dev/v1alpha4\nkind: Concept\nmetadata: { name: countryCode, namespace: iso }\nspec:\n  type: String\n  description: ISO 3166-1 alpha-2\n' > "$TMP/iso/concepts/countryCode.yaml"
printf 'apiVersion: oos.dev/v1alpha4\nkind: Concept\nmetadata: { name: currency, namespace: iso }\nspec:\n  type: String\n' > "$TMP/iso/concepts/currency.yaml"
"$ORE" pack "$TMP/iso" -o "$TMP/semilla/vendor/iso.oob" >/dev/null 2>&1 || falla "0 · no se pudo empaquetar el vocabulario iso"
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

# ── 10 · la puerta es «el árbol no empeora», no «el árbol compila» ──────────
#
# Un árbol recién inducido no compila hasta review (el package con owner:
# cambiame → OOS2009, entidades sin primaryKey → OOS2010, la fuente sin
# declarar → OOS2004). Con la puerta «compila entero», ninguna entidad valida
# podía entrar ahí. Se rompe la forja por otro lado —una vista sobre una tabla
# que no existe, OOS2018— y se comprueba que lo válido entra, lo inválido sale
# con SOLO lo suyo, y retirar lo referenciado sigue siendo 409.
git clone -q "$FORJA" "$TMP/romper" 2>/dev/null
cat > "$TMP/romper/packages/hr/views/rota.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: rota, namespace: hr }
spec:
  owner: team:people-data
  from: { table: no_existe }
  fields: { id: "Worker_Reference.ID" }
Y
( cd "$TMP/romper" && git add -A && git commit -qm "una vista sin revisar" && git push -q origin HEAD:main ) || falla "10 · no se pudo romper la forja"
( cd "$TMP/romper" && "$ORE" validate . >/dev/null 2>&1 ) && falla "10 · el arbol roto compila, y tenia que no"
rm -rf "$TMP/romper"
ANTES=$(cabeza)
[ "$(pide PUT /documentos/Entity/hr/Contractor "$CONTRACTOR")" = "201" ] || falla "10 · una entidad valida no entro en un arbol que ya no compilaba · $(cat "$TMP/r.json")"
[ "$(cabeza)" != "$ANTES" ] || falla "10 · no se empujo"
MAL=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); d['spec']['properties']['inventada']={'type':'String'}; print(json.dumps(d))" "$CONTRACTOR")
[ "$(pide PUT /documentos/Entity/hr/Contractor "$MAL")" = "422" ] || falla "10 · la invalida entro · $(cat "$TMP/r.json")"
cumple "[x['codigo'] for x in d['diagnosticos']] == ['OOS2022'] and d['previos'] >= 1 and 'no_existe' not in json.dumps(d['diagnosticos'])" "10 · el 422 trae SOLO lo nuevo (OOS2022) y cuenta los previos"
[ "$(pide DELETE /documentos/Entity/hr/Department)" = "409" ] || falla "10 · retirar una referenciada dejo de ser 409 · $(cat "$TMP/r.json")"
[ "$(pide DELETE /documentos/Entity/hr/Contractor)" = "200" ] || falla "10 · retirar la libre no dio 200 en un arbol roto · $(cat "$TMP/r.json")"
# y la rota se retira por el verbo: nadie la nombra, y quitarla no empeora nada
[ "$(pide DELETE /documentos/View/hr/rota)" = "200" ] || falla "10 · retirar la vista rota no dio 200 · $(cat "$TMP/r.json")"
git clone -q "$FORJA" "$TMP/limpio" 2>/dev/null && ( cd "$TMP/limpio" && "$ORE" validate . >/dev/null 2>&1 ) || falla "10 · el arbol no volvio a compilar tras retirar la rota"
rm -rf "$TMP/limpio"
dice "10 · con el arbol roto por otro lado: lo valido entra, lo invalido sale con solo lo suyo, y la referenciada sigue en 409"

# ── 11 · View y Table: el mismo motor, dos filas más de la tabla de kinds ───
[ "$(pide GET /documentos/View)" = "200" ] || falla "11 · GET /documentos/View · $(cat "$TMP/r.json")"
cumple "{x['name'] for x in d['documentos']} == {'empleados','envios'}" "11 · acme-retail tiene dos vistas"
cumple "[x for x in d['documentos'] if x['name']=='empleados'][0]['spec']['from']=={'table':'workday_worker'} and len([x for x in d['documentos'] if x['name']=='empleados'][0]['spec']['fields'])==12" "11 · la vista trae from y fields enteros"
[ "$(pide GET /documentos/Table)" = "200" ] || falla "11 · GET /documentos/Table · $(cat "$TMP/r.json")"
cumple "len(d['documentos']) == 2 and [x for x in d['documentos'] if x['name']=='workday_worker'][0]['spec']['reads']['fullScan']=='forbidden' and 'physicalType' in json.dumps(d['documentos'])" "11 · la tabla trae sus dos caras y sus columnas"
[ "$(pide GET /documentos/Function)" = "404" ] || falla "11 · un kind que no esta en la tabla no dio 404"
grep -q "Entity · View · Table · Concept · Interface" "$TMP/r.json" || falla "11 · el 404 no dice los kinds servidos · $(cat "$TMP/r.json")"
[ "$(pide GET /documentos/Table/hr/workday_worker)" = "200" ] && cumple "d['yaml'].startswith('apiVersion: oos.dev/v1alpha8') and 'kind: Table' in d['yaml'] and d['commit']['autor']=='semilla'" "11 · una tabla con su YAML y su commit"
dice "11 · View y Table se leen enteras por el mismo motor; un kind fuera de la tabla es 404 con la lista"

# ── 12 · PUT de una View: el mismo plan que `ore view add` ──────────────────
#
# El verbo emite el YAML con el emisor de Entity; `ore view add` con el suyo.
# Que no divergen se mide: la misma vista por los dos caminos da el mismo
# `plan sha256` en `ore view .`.
git clone -q "$FORJA" "$TMP/add" 2>/dev/null
( cd "$TMP/add" && "$ORE" view add --from workday_worker --owner team:people-data --field id=Worker_Reference.ID --path packages/hr solo_ids >/dev/null 2>&1 ) || falla "12 · ore view add fallo"
PLAN_ADD=$(cd "$TMP/add" && "$ORE" view . 2>/dev/null | sed -n "/^hr.solo_ids$/,/^$/p" | grep -o "sha256:[0-9a-f]*" | head -1)
[ -n "$PLAN_ADD" ] || falla "12 · ore view no dio plan para la de add"
rm -rf "$TMP/add"
VISTA='{"metadata":{"labels":{"oos.maturity":"DRAFT"}},"spec":{"owner":"team:people-data","from":{"table":"workday_worker"},"fields":{"id":"Worker_Reference.ID"}}}'
[ "$(pide PUT /documentos/View/hr/solo_ids "$VISTA")" = "201" ] || falla "12 · PUT View no dio 201 · $(cat "$TMP/r.json")"
[ "$(asunto)" = 'escribir la vista `hr.solo_ids`' ] || falla "12 · el asunto: $(asunto)"
git clone -q "$FORJA" "$TMP/put" 2>/dev/null
PLAN_PUT=$(cd "$TMP/put" && "$ORE" view . 2>/dev/null | sed -n "/^hr.solo_ids$/,/^$/p" | grep -o "sha256:[0-9a-f]*" | head -1)
grep -q "^apiVersion" "$TMP/put/packages/hr/views/solo_ids.yaml" || falla "12 · la vista no esta en views/"
rm -rf "$TMP/put"
[ "$PLAN_PUT" = "$PLAN_ADD" ] || falla "12 · el plan por PUT ($PLAN_PUT) no es el plan por view add ($PLAN_ADD)"
# sin owner: lo exige el verbo, no el compilador
SIN=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); del d['spec']['owner']; print(json.dumps(d))" "$VISTA")
[ "$(pide PUT /documentos/View/hr/otra "$SIN")" = "422" ] || falla "12 · una vista sin owner entro · $(cat "$TMP/r.json")"
grep -q "owner" "$TMP/r.json" || falla "12 · el 422 no nombra owner"
# un field que la tabla no tiene: lo dice el compilador, con el nombre
MAL='{"spec":{"owner":"team:people-data","from":{"table":"workday_worker"},"fields":{"id":"No_Existe"}}}'
[ "$(pide PUT /documentos/View/hr/rota2 "$MAL")" = "422" ] || falla "12 · una vista rota entro · $(cat "$TMP/r.json")"
cumple "d['diagnosticos'][0]['codigo']=='OOS2018' and 'No_Existe' in d['diagnosticos'][0]['mensaje']" "12 · OOS2018 con la columna"
dice "12 · PUT View: 201, mismo plan que ore view add ($PLAN_PUT), sin owner 422, columna que no esta OOS2018"

# ── 13 · PUT con `yaml` tal cual: los comentarios sobreviven ────────────────
YAML_DOC=$("$PY" -c 'import json; print(json.dumps({"yaml": "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata:\n  name: solo_ids\n  namespace: hr\nspec:\n  owner: team:people-data\n  # este comentario es de quien escribe, y se queda\n  from: { table: workday_worker }\n  fields:\n    id: \"Worker_Reference.ID\"\n"}))')
[ "$(pide PUT /documentos/View/hr/solo_ids "$YAML_DOC")" = "200" ] || falla "13 · PUT con yaml no dio 200 · $(cat "$TMP/r.json")"
[ "$(pide GET /documentos/View/hr/solo_ids)" = "200" ] && cumple "'este comentario es de quien escribe' in d['yaml'] and 'labels' not in d['metadata']" "13 · el YAML se guardo tal cual, con su comentario"
YAML_MAL=$("$PY" -c 'import json; print(json.dumps({"yaml": "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: otra, namespace: hr }\nspec:\n  owner: team:people-data\n  from: { table: workday_worker }\n  fields: { id: Worker_Reference.ID }\n"}))')
[ "$(pide PUT /documentos/View/hr/solo_ids "$YAML_MAL")" = "422" ] || falla "13 · un yaml con otro nombre entro · $(cat "$TMP/r.json")"
dice "13 · PUT con yaml tal cual: se guarda con sus comentarios, y el nombre lo pone la ruta"

# ── 14 · DELETE: quien nombra a una vista y a una tabla ─────────────────────
[ "$(pide DELETE /documentos/View/hr/empleados)" = "409" ] || falla "14 · retirar la vista de Employee no dio 409 · $(cat "$TMP/r.json")"
grep -q 'hr.Employee` (backedBy)' "$TMP/r.json" || falla "14 · el 409 no dice quien la nombra · $(cat "$TMP/r.json")"
[ "$(pide DELETE /documentos/Table/hr/workday_worker)" = "409" ] || falla "14 · retirar la tabla de empleados no dio 409 · $(cat "$TMP/r.json")"
grep -q 'hr.empleados` (from.view)\|hr.empleados` (from.table)' "$TMP/r.json" || falla "14 · el 409 de la tabla no dice la vista · $(cat "$TMP/r.json")"
grep -q 'hr.solo_ids` (from.table)' "$TMP/r.json" || falla "14 · el 409 de la tabla no dice TODAS las vistas · $(cat "$TMP/r.json")"
# una vista sobre solo_ids, y entonces solo_ids tampoco se retira
[ "$(pide PUT /documentos/View/hr/encima '{"spec":{"owner":"team:people-data","from":{"view":"solo_ids"},"fields":{"id":"id"}}}')" = "201" ] || falla "14 · la vista sobre vista no entro · $(cat "$TMP/r.json")"
[ "$(pide DELETE /documentos/View/hr/solo_ids)" = "409" ] || falla "14 · retirar una vista con otra encima no dio 409"
grep -q 'hr.encima` (from.view)' "$TMP/r.json" || falla "14 · el 409 no dice from.view · $(cat "$TMP/r.json")"
[ "$(pide DELETE /documentos/View/hr/encima)" = "200" ] || falla "14 · retirar la de encima no dio 200 · $(cat "$TMP/r.json")"
[ "$(pide DELETE /documentos/View/hr/solo_ids)" = "200" ] || falla "14 · retirar solo_ids no dio 200 · $(cat "$TMP/r.json")"
[ "$(asunto)" = 'retirar la vista `hr.solo_ids`' ] || falla "14 · el asunto: $(asunto)"
[ "$(pide GET /documentos/View)" = "200" ] && cumple "len(d['documentos']) == 2" "14 · vuelven a ser dos"
dice "14 · DELETE: la vista de un backedBy y la tabla de un from.table son 409 con los nombres; libres, 200"

# ── 15 · Concept e Interface: dos filas mas, y /conceptos con lo importado ──
[ "$(pide GET /documentos/Concept)" = "200" ] && cumple "d['documentos'] == []" "15 · acme-retail no declara ningun Concept"
[ "$(pide GET /documentos/Interface)" = "200" ] && cumple "d['documentos'] == []" "15 · ni ninguna Interface"
[ "$(pide GET /conceptos)" = "200" ] || falla "15 · GET /conceptos · $(cat "$TMP/r.json")"
cumple "sorted(c['name'] for c in d['conceptos']) == ['countryCode','currency'] and all(c['importado'] and c['paquete']=='iso' and c['fichero']=='vendor/iso.oob' and c['hablado']==[] and c['exigido']==[] for c in d['conceptos'])" "15 · los dos conceptos importados, de vendor/iso.oob, sin nadie que los hable"
cumple "[c for c in d['conceptos'] if c['name']=='countryCode'][0]['spec']['description']=='ISO 3166-1 alpha-2'" "15 · el spec del importado viene entero"
dice "15 · Concept e Interface se sirven (vacios en acme-retail); /conceptos trae los importados de vendor/*.oob"

# ── 16 · PUT de un Concept: nadie lo habla y aun asi entra (sinHablar) ──────
CONCEPTO='{"metadata":{"x-rubix-displayName":"Correo personal"},"spec":{"type":"String","labels":{"gdpr.sensitivity":"high"}}}'
[ "$(pide PUT /documentos/Concept/hr/personalEmail "$CONCEPTO")" = "201" ] || falla "16 · el concepto no entro · $(cat "$TMP/r.json")"
cumple "d['sinHablar']==['hr.personalEmail'] and d['fichero']=='packages/hr/concepts/personalEmail.yaml'" "16 · entra con sinHablar: el OOS9004 tolerado, con el nombre"
[ "$(asunto)" = 'escribir el concepto `hr.personalEmail`' ] || falla "16 · el asunto: $(asunto)"
# un segundo concepto que nadie habla tampoco: el OOS9004 del primero ya estaba, y el suyo se tolera
[ "$(pide PUT /documentos/Concept/hr/legalName '{"spec":{"type":"String"}}')" = "201" ] || falla "16 · el segundo concepto no entro · $(cat "$TMP/r.json")"
# pero un Concept sin type es OOS1004 del compilador, y ese NO se tolera
[ "$(pide PUT /documentos/Concept/hr/sinTipo '{"spec":{"description":"nada"}}')" = "422" ] || falla "16 · un concepto sin type entro · $(cat "$TMP/r.json")"
cumple "any(x['codigo']=='OOS1004' for x in d['diagnosticos']) and not any(x['codigo']=='OOS9004' for x in d['diagnosticos'])" "16 · 422 con OOS1004 y sin el OOS9004 de los otros"
# Employee.email pasa a hablar hr.personalEmail: is en vez de type
[ "$(pide GET /documentos/Entity/hr/Employee)" = "200" ] || falla "16 · GET Employee"
HABLA=$("$PY" -c "import json,sys; d=json.load(open(sys.argv[1])); e=d['spec']['properties']['email']; e.pop('type'); e['is']='hr.personalEmail'; print(json.dumps({'metadata':d['metadata'],'spec':d['spec']}))" "$TMP/r.json")
[ "$(pide PUT /documentos/Entity/hr/Employee "$HABLA")" = "200" ] || falla "16 · Employee con is no entro · $(cat "$TMP/r.json")"
[ "$(pide GET /conceptos)" = "200" ] && cumple "[c for c in d['conceptos'] if c['name']=='personalEmail'][0]['hablado']==['hr.Employee.email'] and [c for c in d['conceptos'] if c['name']=='personalEmail'][0]['importado'] is False and [c for c in d['conceptos'] if c['name']=='personalEmail'][0]['metadata']['x-rubix-displayName']=='Correo personal' and len(d['conceptos'])==4" "16 · /conceptos: hr.personalEmail lo habla hr.Employee.email; cuatro en total"
[ "$(pide GET /documentos/Concept)" = "200" ] && cumple "sorted(x['name'] for x in d['documentos'])==['legalName','personalEmail'] and all(x['paquete']=='hr' for x in d['documentos'])" "16 · /documentos/Concept son los dos del arbol, no los importados"
dice "16 · PUT Concept: entra sin que nadie lo hable (sinHablar, OOS9004 tolerado); sin type es 422; con is, /conceptos dice quien lo habla"

# ── 17 · Interface: requires a lo que no esta es OOS2001; DELETE con quien nombra ──
[ "$(pide PUT /documentos/Interface/hr/Party '{"spec":{"requires":["hr.noExiste"]}}')" = "422" ] || falla "17 · una interfaz a un concepto que no esta entro · $(cat "$TMP/r.json")"
cumple "d['diagnosticos'][0]['codigo']=='OOS2001' and 'noExiste' in d['diagnosticos'][0]['mensaje']" "17 · OOS2001 con el nombre"
[ "$(pide PUT /documentos/Interface/hr/Party '{"spec":{"requires":["hr.personalEmail"]}}')" = "201" ] || falla "17 · la interfaz no entro · $(cat "$TMP/r.json")"
cumple "'sinHablar' not in d and d['fichero']=='packages/hr/interfaces/Party.yaml'" "17 · la interfaz no tolera nada"
[ "$(pide GET /conceptos)" = "200" ] && cumple "[c for c in d['conceptos'] if c['name']=='personalEmail'][0]['exigido']==['hr.Party']" "17 · /conceptos: hr.Party exige hr.personalEmail"
[ "$(pide DELETE /documentos/Concept/hr/personalEmail)" = "409" ] || falla "17 · retirar el concepto hablado no dio 409 · $(cat "$TMP/r.json")"
grep -q 'hr.Employee` (properties.email.is)' "$TMP/r.json" || falla "17 · el 409 no dice la propiedad · $(cat "$TMP/r.json")"
grep -q 'hr.Party` (requires)' "$TMP/r.json" || falla "17 · el 409 no dice la interfaz · $(cat "$TMP/r.json")"
# Employee implementa hr.Party (la satisface: email habla personalEmail)
IMPL=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); d['spec']['implements']=['hr.Party']; print(json.dumps(d))" "$HABLA")
[ "$(pide PUT /documentos/Entity/hr/Employee "$IMPL")" = "200" ] || falla "17 · Employee implements no entro · $(cat "$TMP/r.json")"
[ "$(pide DELETE /documentos/Interface/hr/Party)" = "409" ] || falla "17 · retirar la interfaz implementada no dio 409 · $(cat "$TMP/r.json")"
grep -q 'hr.Employee` (implements)' "$TMP/r.json" || falla "17 · el 409 no dice implements · $(cat "$TMP/r.json")"
dice "17 · Interface: requires a lo que no esta es OOS2001; el concepto hablado y la interfaz implementada son 409 con los nombres"

# ── 18 · deshacer en orden: lo que nadie nombra se retira, y el commit es del sujeto ──
[ "$(pide PUT /documentos/Entity/hr/Employee "$HABLA")" = "200" ] || falla "18 · quitar implements no entro"
[ "$(pide DELETE /documentos/Interface/hr/Party)" = "200" ] || falla "18 · retirar la interfaz libre no dio 200 · $(cat "$TMP/r.json")"
[ "$(asunto)" = 'retirar la interfaz `hr.Party`' ] || falla "18 · el asunto: $(asunto)"
# legalName nunca lo hablo nadie: se retira, y el arbol no empeora (su OOS9004 se va)
[ "$(pide DELETE /documentos/Concept/hr/legalName)" = "200" ] || falla "18 · retirar legalName no dio 200 · $(cat "$TMP/r.json")"
[ "$(pide DELETE /documentos/Concept/hr/personalEmail)" = "409" ] || falla "18 · personalEmail sigue hablado y no dio 409"
ORIG=$("$PY" -c "import json,sys; d=json.loads(sys.argv[1]); e=d['spec']['properties']['email']; e.pop('is'); e['type']='String'; print(json.dumps(d))" "$HABLA")
# dejar de hablarlo hace nuevo el OOS9004 — y retirarlo antes es 409: ningun orden
# entraria si la puerta no tolerara OOS9004. Entra, y dice que lo deja sin hablar.
[ "$(pide PUT /documentos/Entity/hr/Employee "$ORIG")" = "200" ] || falla "18 · Employee sin is no entro · $(cat "$TMP/r.json")"
cumple "d['sinHablar']==['hr.personalEmail']" "18 · la entidad dice que deja hr.personalEmail sin hablar"
[ "$(pide DELETE /documentos/Concept/hr/personalEmail)" = "200" ] || falla "18 · retirar personalEmail no dio 200 · $(cat "$TMP/r.json")"
[ "$(asunto)" = 'retirar el concepto `hr.personalEmail`' ] || falla "18 · el asunto: $(asunto)"
[ "$(pide GET /conceptos)" = "200" ] && cumple "len(d['conceptos'])==2 and all(c['importado'] for c in d['conceptos'])" "18 · quedan los dos importados"
dice "18 · deshecho en orden: libres, 200; el arbol vuelve a ser acme-retail mas el vocabulario iso"

echo
echo "ok · /documentos/{kind}: un motor, una tabla de kinds — Entity, View, Table, Concept, Interface — y /conceptos; escribir es un commit del sujeto que no empeora el arbol"
