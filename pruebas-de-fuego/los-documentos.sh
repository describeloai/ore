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
#  20  /proyectos (0035 ②, ⑦.1)    crear es un commit del sujeto y el `id` sale
#                                     del titulo; NACE CON SITIO — su
#                                     `packages/<id>/package.yaml` en el MISMO
#                                     commit, y `contiene` lo nombra el primero
#                                     (0035 ⑦.1: sin suelo, lo primero que se
#                                     guarde en el proyecto tiene que ir a un
#                                     paquete prestado); el paquete de otro NO
#                                     se adopta (409); el nombre repetido 409;
#                                     el indice de assets los trae con sus
#                                     items; PUT reescribe el manifiesto entero
#                                     y NO le quita el sitio; DELETE se lleva la
#                                     LENTE y lo que nombraba —su sitio tambien—
#                                     SIGUE en el arbol; y desde un puesto, 403
#  22  /repositorios (0036 ②)      nacer ENTERO: el manifiesto y la semilla de
#                                     la clase en UN commit, y el proyecto que
#                                     lo nombra en ese mismo; 409 si esa carpeta
#                                     ya es uno; una clase inventada, 422 con
#                                     las que hay; PUT conserva la prosa; y
#                                     borrar su carpeta se lleva el manifiesto
#  23  lo acotado (0036 ④)         `X-Ore-Raiz` en `GET /arbol`: el editor abre
#                                     SU carpeta y no la celda; la cabeza del
#                                     arbol no cambia; un alcance que no es una
#                                     carpeta de paquete, 422, y una que no
#                                     esta, 404
#  21  carpetas (0035 ③b, ⑦)       una carpeta es un fichero dentro (README);
#                                     el indice la nombra POR ESTAR, sin
#                                     esperar a que caiga un documento (0035 ⑦:
#                                     contarlas por items dejaba invisible la
#                                     carpeta recien creada, que es justo donde
#                                     se guarda lo primero); `DELETE
#                                     /arbol/<carpeta>` se la
#                                     lleva ENTERA en UN commit diciendo que
#                                     ficheros; y si al irse el arbol empeora,
#                                     422 y no se pierde nada
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
[ "$(pide GET /documentos/Ruleset)" = "404" ] || falla "11 · un kind que no esta en la tabla no dio 404"
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

# ── 18b · Function y Action por la misma puerta (0034 paso 5) ────────────────
#
# La funcion de solo lectura sobre `hr.empleados` entra (el compilador no exige
# el .wasm en disco: lo que exige es la forma y que `over` resuelva); se lista
# por su kind; y una Action que escribe una propiedad sin integridad es un 422
# con su OOS7005: la puerta compila la Action, no solo la guarda.
[ "$(pide GET /documentos/Function)" = "200" ] && cumple "d['documentos'] == []" "18b · sin funciones al principio"
FUNCION='{"spec":{"runtime":"wasm","entrypoint":"dist/contar.wasm","over":"hr.empleados","output":{"n":{"type":"Integer"}}}}'
[ "$(pide PUT /documentos/Function/hr/contar "$FUNCION")" = "201" ] || falla "18b · PUT Function no dio 201 · $(cat "$TMP/r.json")"
[ "$(asunto)" = 'escribir la función `hr.contar`' ] || falla "18b · el asunto: $(asunto)"
[ "$(pide GET /documentos/Function)" = "200" ] && cumple "[d['name'] for d in d['documentos']] == ['contar'] and d['documentos'][0]['fichero'] == 'packages/hr/functions/contar.yaml'" "18b · la funcion se lista por su kind, en functions/"
[ "$(pide GET /documentos/Function/hr/contar)" = "200" ] && cumple "d['spec']['over'] == 'hr.empleados'" "18b · y se lee entera"
ACCION='{"spec":{"over":"hr.empleados","input":{"nota":{"type":"String","required":true}},"sets":[{"writes":"hr.Employee.fullName","from":"input.nota"}]}}'
[ "$(pide PUT /documentos/Action/hr/anotar "$ACCION")" = "422" ] || falla "18b · una Action sobre una propiedad sin integridad tenia que ser 422 · $(cat "$TMP/r.json")"
grep -q "OOS7005" "$TMP/r.json" || falla "18b · el 422 no dice OOS7005 · $(cat "$TMP/r.json")"
[ "$(pide GET /documentos/Action)" = "200" ] && cumple "d['documentos'] == []" "18b · y la Action que no compila no queda"
[ "$(pide DELETE /documentos/Function/hr/contar)" = "200" ] || falla "18b · retirar la funcion no dio 200 · $(cat "$TMP/r.json")"
dice "18b · Function y Action por /documentos: la funcion entra, se lista en functions/ y se retira; la Action que escribe sin integridad es 422 con OOS7005"
# ── 19 · /arbol: el arbol por ruta, lo que el editor abre (0030 W0) ──────────
pon() { # ruta fichero-con-el-texto [cabecera-extra]
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X PUT -H "$SUJ" -H "${3:-x-nada: 1}" \
    -H 'Content-Type: text/yaml' --data-binary "@$2" "$BASE/arbol/$1"
}
[ "$(pide GET /arbol)" = "200" ] || falla "19 · GET /arbol · $(cat "$TMP/r.json")"
cumple "len(d['ficheros']) > 20 and d['cabeza'] and any(f['ruta']=='packages/hr/entities/Employee.yaml' and f['kind']=='Entity' for f in d['ficheros']) and any(f['ruta']=='ontology.config.yaml' for f in d['ficheros'])" "19 · el indice: cabeza, rutas y kinds"
cumple "all('/' not in f['ruta'][:1] and '..' not in f['ruta'] for f in d['ficheros'])" "19 · rutas relativas"
[ "$(pide GET /arbol/packages/hr/entities/Employee.yaml)" = "200" ] || falla "19 · GET /arbol/<ruta> · $(cat "$TMP/r.json")"
cumple "d['kind']=='Entity' and 'kind: Entity' in d['texto'] and d['commit']['hash'] and d['ruta']=='packages/hr/entities/Employee.yaml'" "19 · el fichero con su kind, su texto y su commit"
CABEZA=$(campo "d['cabeza']")
[ "$(pide GET /arbol/no/existe.yaml)" = "404" ] || falla "19 · un fichero que no esta no dio 404"
[ "$(pide GET /arbol/../etc/passwd)" != "200" ] || falla "19 · una ruta con .. entro"
[ "$(pide GET /arbol/diagnosticos)" = "200" ] || falla "19 · GET /arbol/diagnosticos · $(cat "$TMP/r.json")"
cumple "d['diagnosticos']==[] and d['cabeza']" "19 · acme-retail compila: sin diagnosticos"
# una vista nueva por ruta: 201, sin diagnosticos, commit del sujeto
cat > "$TMP/porRuta.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: porRuta, namespace: hr }
spec:
  owner: team:people-data
  from: { view: empleados }
  fields:
    id: employeeId
Y
[ "$(pon packages/hr/views/porRuta.yaml "$TMP/porRuta.yaml")" = "201" ] || falla "19 · PUT /arbol de una vista nueva · $(cat "$TMP/r.json")"
cumple "d['nueva'] is True and d['kind']=='View' and d['diagnosticos']==[] and d['commit']" "19 · 201 con kind, commit y sin diagnosticos"
[ "$(asunto)" = 'escribir `packages/hr/views/porRuta.yaml`' ] || falla "18b · el asunto: $(asunto)"
git --git-dir="$FORJA" log -1 --format='%an' main | grep -q "ana" || falla "19 · el commit no es del sujeto: $(git --git-dir="$FORJA" log -1 --format='%an' main)"
# el mismo texto otra vez: 200, igual, y ningun commit
ANTES=$(cabeza)
[ "$(pon packages/hr/views/porRuta.yaml "$TMP/porRuta.yaml")" = "200" ] || falla "19 · reescribir lo mismo no dio 200"
cumple "d.get('igual') is True" "19 · lo mismo es igual"
[ "$(cabeza)" = "$ANTES" ] || falla "19 · reescribir lo mismo hizo un commit"
# romper una referencia: 422 con el marcador en su fichero y su linea, y nada escrito
sed 's/from: { view: empleados }/from: { view: no_existe }/' "$TMP/porRuta.yaml" > "$TMP/rota.yaml"
[ "$(pon packages/hr/views/porRuta.yaml "$TMP/rota.yaml")" = "422" ] || falla "19 · romper una referencia no dio 422 · $(cat "$TMP/r.json")"
cumple "d['diagnosticos'][0]['codigo']=='OOS2018' and d['diagnosticos'][0]['fichero']=='packages/hr/views/porRuta.yaml' and d['diagnosticos'][0]['linea']==6 and d['diagnosticos'][0]['columna']>0 and d['diagnosticos'][0]['severidad']=='error'" "19 · el marcador: OOS2018, fichero, linea 6, columna, severidad"
[ "$(pide GET /arbol/packages/hr/views/porRuta.yaml)" = "200" ] && cumple "'view: empleados' in d['texto']" "19 · la que rompia no se escribio"
[ "$(cabeza)" = "$ANTES" ] || falla "19 · un 422 hizo commit"
# If-Match viejo: 409 y nada escrito
sed 's/id: Worker_Reference.ID/id: Worker_Reference.ID\n    nombre: Legal_Name/' "$TMP/porRuta.yaml" > "$TMP/mas.yaml"
[ "$(pon packages/hr/views/porRuta.yaml "$TMP/mas.yaml" "If-Match: $CABEZA")" = "409" ] || falla "19 · con If-Match viejo no dio 409 · $(cat "$TMP/r.json")"
[ "$(pon packages/hr/views/porRuta.yaml "$TMP/mas.yaml" "If-Match: $(cabeza)")" = "200" ] || falla "19 · con If-Match al dia no entro · $(cat "$TMP/r.json")"
# lo gobernado que se induce, y la historia, no se editan
[ "$(pon packages/hr/discover.scope.json "$TMP/porRuta.yaml")" = "422" ] || falla "19 · discover.scope.json entro por /arbol"
[ "$(pon .git/config "$TMP/porRuta.yaml")" = "422" ] || falla "19 · .git entro por /arbol"
# retirar: 200, y otra vez 404
[ "$(pide DELETE /arbol/packages/hr/views/porRuta.yaml)" = "200" ] || falla "19 · DELETE /arbol · $(cat "$TMP/r.json")"
[ "$(asunto)" = 'retirar `packages/hr/views/porRuta.yaml`' ] || falla "19 · el asunto del retiro: $(asunto)"
[ "$(pide GET /arbol/packages/hr/views/porRuta.yaml)" = "404" ] || falla "19 · retirada y sigue"
dice "19 · /arbol: indice con kinds · fichero con commit · PUT compila (201, igual, 422 con marcador fichero:linea:columna, 409 If-Match) · lo inducido y .git no se editan · DELETE"

# ── 20 · /proyectos: la lente, escrita (0035 ②) ─────────────────────────────
ANTES_ITEMS=$(pide GET /assets >/dev/null; campo "len(d['items'])")
[ "$(pide POST /proyectos '{"nombre":"Customer Churn","descripcion":"Abandono sobre la plantilla.","contiene":["hr"]}')" = "201" ] \
  || falla "20 · POST /proyectos · $(cat "$TMP/r.json")"
cumple "d['id']=='customer-churn' and d['nombre']=='Customer Churn' and d['contiene']==['customer-churn','hr'] and d['nueva'] is True and d['commit'] and 'sinResolver' not in d" "20 · 201 con el id del titulo, SU SITIO el primero de lo que nombra, y el commit"
cumple "d['sitio']=='packages/customer-churn'" "20 · nace con sitio propio (0035 vii.1)"
[ "$(asunto)" = 'crear un proyecto' ] || falla "20 · el asunto: $(asunto)"
git --git-dir="$FORJA" log -1 --format='%an' main | grep -q "ana" || falla "20 · el commit no es del sujeto"
# el sitio y el manifiesto, en EL MISMO commit: un proyecto sin suelo es el fallo que vii arregla
[ "$(git --git-dir="$FORJA" show --name-only --format='' main | sort | tr '\n' ' ')" = "packages/customer-churn/package.yaml proyectos/customer-churn/README.md " ] \
  || falla "20 · el sitio no nacio con el proyecto: $(git --git-dir="$FORJA" show --name-only --format='' main | tr '\n' ' ')"
git --git-dir="$FORJA" show main:packages/customer-churn/package.yaml | grep -q 'status: draft' \
  || falla "20 · el paquete del proyecto no nace en draft: $(git --git-dir="$FORJA" show main:packages/customer-churn/package.yaml)"
git --git-dir="$FORJA" show main:packages/customer-churn/package.yaml | grep -q 'owner: "user:ana"' \
  || falla "20 · el owner del sitio no es quien lo creo: $(git --git-dir="$FORJA" show main:packages/customer-churn/package.yaml)"
# y el arbol sigue compilando CON el paquete vacio dentro (si no, el commit no habria entrado)
[ "$(pide GET /arbol/diagnosticos)" = "200" ] && cumple "[x for x in d['diagnosticos'] if x['severidad']=='error']==[]" "20 · con el sitio dentro, el arbol compila igual"
# el paquete de otro NO se adopta: nacer dentro de algo que ya estaba seria fingir que lo creo
ANTES=$(cabeza)
[ "$(pide POST /proyectos '{"nombre":"hr"}')" = "409" ] || falla "20 · adoptar el paquete `hr` no dio 409 · $(cat "$TMP/r.json")"
[ "$(cabeza)" = "$ANTES" ] || falla "20 · el 409 del paquete ajeno hizo commit"
# el mismo nombre otra vez: 409, y nada escrito
ANTES=$(cabeza)
[ "$(pide POST /proyectos '{"nombre":"Customer  Churn"}')" = "409" ] || falla "20 · el nombre repetido no dio 409 · $(cat "$TMP/r.json")"
[ "$(cabeza)" = "$ANTES" ] || falla "20 · un 409 hizo commit"
[ "$(pide POST /proyectos '{"descripcion":"sin nombre"}')" = "422" ] || falla "20 · sin nombre no dio 422"
# un proyecto que nombra lo que aun no existe: entra, y lo dice
[ "$(pide POST /proyectos '{"nombre":"Nomina 2026","contiene":["hr/nomina"]}')" = "201" ] || falla "20 · un proyecto que nombra lo que no existe no entro · $(cat "$TMP/r.json")"
cumple "d['id']=='nomina-2026' and d['contiene']==['nomina-2026','hr/nomina'] and d['sinResolver']==['hr/nomina']" "20 · lo que no resuelve se dice y no impide; SU SITIO si resuelve, aunque este vacio"
# un proyecto de ANTES de vii.1 (escrito a mano, sin sitio): editarlo se lo da
printf -- '---
nombre: "Legado"
---

De antes.
' > "$TMP/legado.md"
[ "$(pon proyectos/legado/README.md "$TMP/legado.md")" = "201" ] || falla "20 · el proyecto legado no entro · $(cat "$TMP/r.json")"
[ "$(pide GET /assets)" = "200" ] && cumple "[p for p in d['proyectos'] if p['nombre']=='legado'][0]['sitio'] is None" "20 · un proyecto de antes de vii.1 no tiene sitio"
[ "$(pide PUT /proyectos/legado '{"nombre":"Legado"}')" = "200" ] || falla "20 · PUT del legado · $(cat "$TMP/r.json")"
cumple "d['sitio']=='packages/legado' and d['contiene']==['legado']" "20 · editar un proyecto sin sitio SE LO DA (0035 vii.1)"
[ "$(pide DELETE /proyectos/legado)" = "200" ] || falla "20 · no se pudo retirar el legado"
# el indice de assets los trae, sin ruta nueva y sin cambiar los items
[ "$(pide GET /assets)" = "200" ] || falla "20 · GET /assets · $(cat "$TMP/r.json")"
cumple "len(d['items']) == $ANTES_ITEMS" "20 · los proyectos no son items: el indice no cambia"
cumple "[p['nombre'] for p in d['proyectos']] == ['customer-churn','nomina-2026']" "20 · /assets los trae, por nombre de carpeta"
cumple "next(p for p in d['proyectos'] if p['nombre']=='customer-churn')['items'] > 0" "20 · el que nombra el paquete hr tiene items"
cumple "next(p for p in d['proyectos'] if p['nombre']=='nomina-2026')['items'] == 0" "20 · el que no resuelve, cero"
cumple "next(p for p in d['proyectos'] if p['nombre']=='customer-churn')['version']['sujeto']=='persona.ana'" "20 · quien lo creo sale de su propio manifiesto"
cumple "any(it['proyectos']==['customer-churn'] for it in d['items'].values())" "20 · y cada item dice en que proyectos esta, en plural"
# PUT: el manifiesto entero
[ "$(pide PUT /proyectos/customer-churn '{"nombre":"Customer Churn","descripcion":"Otra cosa.","contiene":["hr","sales"]}')" = "200" ] \
  || falla "20 · PUT /proyectos/{id} · $(cat "$TMP/r.json")"
cumple "d['descripcion']=='Otra cosa.' and d['contiene']==['customer-churn','hr','sales'] and d['nueva'] is False and d['commit']" "20 · el manifiesto entero, reescrito — y el PUT NO le quita el sitio"
[ "$(pide PUT /proyectos/no-existe '{"nombre":"X"}')" = "404" ] || falla "20 · PUT de uno que no esta no dio 404"
# DELETE: se va la lente, NO lo que nombraba
[ "$(pide DELETE /proyectos/customer-churn)" = "200" ] || falla "20 · DELETE /proyectos/{id} · $(cat "$TMP/r.json")"
cumple "d['retirado'] is True and d['siguenEnElArbol']==['customer-churn','hr','sales'] and d['sitio']=='packages/customer-churn'" "20 · lo que nombraba se dice y sigue — SU SITIO tambien, y se dice aparte"
[ "$(pide GET /arbol/packages/customer-churn/package.yaml)" = "200" ] || falla "20 · borrar la lente se llevo su sitio"
[ "$(pide GET /documentos/Entity/hr/Employee)" = "200" ] || falla "20 · borrar el proyecto se llevo lo que nombraba"
[ "$(pide GET /assets)" = "200" ] && cumple "len(d['items']) == $ANTES_ITEMS and [p['nombre'] for p in d['proyectos']] == ['nomina-2026']" "20 · el arbol entero sigue; la lente se fue"
[ "$(pide DELETE /proyectos/customer-churn)" = "404" ] || falla "20 · retirado y sigue"
# y desde un puesto, no: un proyecto lo crea una persona (W3.7 gobierno ①)
CODIGO=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: agente:puesto-ana-python'   -H 'Content-Type: application/json' -d '{"nombre":"Desde el puesto"}' "$BASE/proyectos")
[ "$CODIGO" = "403" ] || falla "20 · un agente creo un proyecto ($CODIGO) · $(cat "$TMP/r.json")"
dice "20 · /proyectos: crear es un commit del sujeto (id del titulo) y NACE CON SITIO (su package.yaml en el mismo commit, en draft y con dueno; el ajeno no se adopta: 409) · nombre repetido 409 · lo que no resuelve entra y se dice · /assets los trae con sus items y cada item en plural · PUT el manifiesto entero y no le quita el sitio · DELETE se lleva la lente y NO lo que nombraba, su sitio incluido"

# ── 21 · las carpetas de un proyecto (0035 ③b) ──────────────────────────────
printf '# Ingesta\n' > "$TMP/carpeta.md"
[ "$(pon packages/hr/ingesta/README.md "$TMP/carpeta.md")" = "201" ] || falla "21 · el README de la carpeta no entro · $(cat "$TMP/r.json")"
cumple "d['diagnosticos']==[] and d['commit']" "21 · una carpeta se crea con un fichero dentro, y compila"
[ "$(pide GET /assets)" = "200" ] && cumple "'ingesta' in [p for p in d['paquetes'] if p['name']=='hr'][0]['carpetas']" "21 · con solo un README, el indice YA la nombra (0035 vii: una carpeta existe por estar)"
cat > "$TMP/enIngesta.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: enIngesta, namespace: hr }
spec:
  owner: team:people-data
  from: { view: empleados }
  fields:
    id: employeeId
Y
[ "$(pon packages/hr/ingesta/views/enIngesta.yaml "$TMP/enIngesta.yaml")" = "201" ] || falla "21 · el documento dentro de la carpeta no entro · $(cat "$TMP/r.json")"
[ "$(pide GET /assets)" = "200" ] && cumple "'ingesta' in [p for p in d['paquetes'] if p['name']=='hr'][0]['carpetas']" "21 · y con un documento dentro, sigue nombrandola"
# una carpeta que al irse rompe el arbol: 422/409 y no se pierde nada
cat > "$TMP/laQueUsa.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: laQueUsa, namespace: hr }
spec:
  owner: team:people-data
  from: { view: enIngesta }
  fields:
    id: id
Y
[ "$(pon packages/hr/views/laQueUsa.yaml "$TMP/laQueUsa.yaml")" = "201" ] || falla "21 · la vista que usa la de dentro no entro · $(cat "$TMP/r.json")"
ANTES=$(cabeza)
CODIGO=$(pide DELETE /arbol/packages/hr/ingesta)
[ "$CODIGO" = "422" ] || [ "$CODIGO" = "409" ] || falla "21 · borrar una carpeta que rompe el arbol dio $CODIGO · $(cat "$TMP/r.json")"
[ "$(cabeza)" = "$ANTES" ] || falla "21 · un borrado rechazado hizo commit"
[ "$(pide GET /arbol/packages/hr/ingesta/views/enIngesta.yaml)" = "200" ] || falla "21 · el rechazo se llevo lo de dentro"
[ "$(pide DELETE /arbol/packages/hr/views/laQueUsa.yaml)" = "200" ] || falla "21 · no se pudo quitar la que usaba"
# y ahora la carpeta entera, en UN commit, diciendo que ficheros
ANTES=$(cabeza)
[ "$(pide DELETE /arbol/packages/hr/ingesta)" = "200" ] || falla "21 · DELETE de la carpeta · $(cat "$TMP/r.json")"
cumple "d['carpeta'] is True and d['retirado'] is True and sorted(d['ficheros'])==['packages/hr/ingesta/README.md','packages/hr/ingesta/views/enIngesta.yaml']" "21 · se lleva los dos ficheros y los dice"
[ "$(git --git-dir="$FORJA" rev-list --count "$ANTES..$(cabeza)")" = "1" ] || falla "21 · la carpeta se fue en mas de un commit"
[ "$(asunto)" = 'retirar `packages/hr/ingesta`' ] || falla "21 · el asunto: $(asunto)"
[ "$(pide GET /arbol/packages/hr/ingesta/README.md)" = "404" ] || falla "21 · el README sigue"
[ "$(pide DELETE /arbol/packages/hr/ingesta)" = "404" ] || falla "21 · una carpeta que no esta no dio 404"
[ "$(pide GET /assets)" = "200" ] && cumple "'ingesta' not in [p for p in d['paquetes'] if p['name']=='hr'][0]['carpetas']" "21 · el indice ya no la nombra"
dice "21 · carpetas: una carpeta es un fichero dentro · el indice la nombra POR ESTAR, vacia o no (0035 vii) · DELETE de la carpeta se la lleva entera en UN commit diciendo que ficheros · si al irse el arbol empeora, 422 y nada se pierde"

# ── 22 · /repositorios: la unidad de trabajo (0036 ②) ───────────────────────
[ "$(pide POST /proyectos '{"nombre":"Personas","contiene":[]}')" = "201" ] || falla "22 · el proyecto de la prueba no entro · $(cat "$TMP/r.json")"
ANTES=$(cabeza)
[ "$(pide POST /repositorios '{"paquete":"hr","carpeta":"raw","nombre":"New Pipelines Java Transform","plantilla":"transforms","proyecto":"personas"}')" = "201" ] \
  || falla "22 · POST /repositorios · $(cat "$TMP/r.json")"
cumple "d['ruta']=='packages/hr/raw' and d['plantilla']=='transforms-python' and d['plantillaVersion']==2 and d['nombre']=='New Pipelines Java Transform' and d['nueva'] is True and d['proyecto']=='personas' and d['commit']" "22 · 201 con su ruta, su clase (la clave vieja `transforms` resuelve a `transforms-python`), su version y el proyecto"
cumple "d['semilla']==['packages/hr/raw/pyproject.toml','packages/hr/raw/transforms/ejemplo.py']" "22 · la semilla es un ARBOL de ficheros: su entorno y su ejemplo (0036 viii.a)"
[ "$(git --git-dir="$FORJA" rev-list --count "$ANTES..$(cabeza)")" = "1" ] || falla "22 · nacer entero costo mas de un commit"
[ "$(asunto)" = 'crear un repositorio' ] || falla "22 · el asunto: $(asunto)"
# el manifiesto, la semilla y el proyecto, en ESE commit
git --git-dir="$FORJA" show --name-only --format= main > "$TMP/tocados.txt"
grep -q "packages/hr/raw/README.md" "$TMP/tocados.txt" || falla "22 · el manifiesto no esta en el commit"
grep -q "packages/hr/raw/transforms/ejemplo.py" "$TMP/tocados.txt" || falla "22 · la semilla no esta en el commit"
grep -q "proyectos/personas/README.md" "$TMP/tocados.txt" || falla "22 · el proyecto no se actualizo en el mismo commit"
# el indice lo trae, con su clase, y el proyecto lo nombra
[ "$(pide GET /assets)" = "200" ] || falla "22 · GET /assets · $(cat "$TMP/r.json")"
cumple "[r['ruta'] for r in d['repositorios']] == ['packages/hr/raw']" "22 · /assets trae el repositorio"
cumple "d['repositorios'][0]['plantilla']=='transforms-python' and d['repositorios'][0]['version']['sujeto']=='persona.ana'" "22 · con su clase y quien lo creo"
cumple "[p for p in d['proyectos'] if p['nombre']=='personas'][0]['contiene']==['personas','hr/raw']" "22 · el proyecto lo nombra — ESTE repositorio y no el paquete entero (0035 vii.3), detras de su propio sitio"
cumple "[p for p in d['proyectos'] if p['nombre']=='personas'][0]['sitio']=='packages/personas'" "22 · y el indice dice cual es SU raiz, aparte de lo que nombra"
# uno DENTRO de su propio sitio no anade nada: ya lo alcanza
[ "$(pide POST /repositorios '{"paquete":"personas","carpeta":"mio","nombre":"Mio","plantilla":"models","proyecto":"personas"}')" = "201" ]   || falla "22 · un repositorio en el sitio del proyecto no entro · $(cat "$TMP/r.json")"
[ "$(pide GET /assets)" = "200" ] && cumple "[p for p in d['proyectos'] if p['nombre']=='personas'][0]['contiene']==['personas','hr/raw']" "22 · lo que cae en su sitio NO se dice dos veces (0035 vii.3)"
[ "$(pide DELETE /arbol/packages/personas/mio)" = "200" ] || falla "22 · no se pudo retirar el repositorio de dentro"
# viii.a · el ejemplo CORRE (no es un comentario) y el entorno se declara EN la instancia
[ "$(pide GET /arbol/packages/hr/raw/transforms/ejemplo.py)" = "200" ]   && cumple "len([l for l in d['texto'].splitlines() if l.strip() and not l.strip().startswith('#')]) >= 5 and '@transform(' in d['texto']" "22 · el ejemplo es codigo, no un comentario"
[ "$(pide GET /entorno "" "x-ore-raiz: packages/hr/raw")" = "200" ]   && cumple "d['alcance']=='packages/hr/raw' and d['declarado']==[]" "22 · nace sin dependencias: declarar lo que nadie usa seria una capa para nada"
printf '[project]
name = "x"
version = "0.1.0"
dependencies = ["polars"]
' > "$TMP/py.toml"
[ "$(pon packages/hr/raw/pyproject.toml "$TMP/py.toml")" = "200" ] || falla "22 · no se pudo declarar en el pyproject de la instancia · $(cat "$TMP/r.json")"
[ "$(pide GET /entorno "" "x-ore-raiz: packages/hr/raw")" = "200" ]   && cumple "d['declarado']==['polars'] and d['digest']" "22 · y declarar AHI le da capa propia (0036 iii): el fichero sembrado es lo que lo enciende"
# el sitio ya esta cogido
ANTES=$(cabeza)
[ "$(pide POST /repositorios '{"paquete":"hr","carpeta":"raw","nombre":"Otro","plantilla":"models"}')" = "409" ] || falla "22 · la carpeta cogida no dio 409 · $(cat "$TMP/r.json")"
[ "$(cabeza)" = "$ANTES" ] || falla "22 · un 409 hizo commit"
# una clase inventada, y un paquete que no esta
[ "$(pide POST /repositorios '{"paquete":"hr","carpeta":"otro","nombre":"X","plantilla":"lo-que-sea"}')" = "422" ] || falla "22 · una clase inventada no dio 422"
grep -q "transforms-python, transforms-java, transforms-sql, analytics-python, models-python, functions-python, semantics" "$TMP/r.json" || falla "22 · el 422 no dice las clases que hay · $(cat "$TMP/r.json")"
[ "$(pide POST /repositorios '{"paquete":"noexiste","carpeta":"x","nombre":"X","plantilla":"models"}')" = "404" ] || falla "22 · un paquete que no esta no dio 404"
# PUT: el manifiesto entero, conservando la prosa
[ "$(pide PUT /repositorios/packages/hr/raw '{"nombre":"Renombrado","plantilla":"analytics","plantillaVersion":1}')" = "200" ] \
  || falla "22 · PUT /repositorios · $(cat "$TMP/r.json")"
cumple "d['nombre']=='Renombrado' and d['plantilla']=='analytics-python' and d['nueva'] is False and d['commit']" "22 · el manifiesto reescrito"
[ "$(pide GET /arbol/packages/hr/raw/README.md)" = "200" ] && cumple "'Lo que este repositorio hace' in d['texto']" "22 · la prosa se conserva"
[ "$(pide PUT /repositorios/packages/hr/noexiste '{"nombre":"X","plantilla":"models"}')" = "404" ] || falla "22 · PUT de uno que no esta no dio 404"
# desde un puesto, no: un repositorio lo crea una persona
CODIGO=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: agente:puesto-ana-python' \
  -H 'Content-Type: application/json' -d '{"paquete":"hr","carpeta":"z","nombre":"X","plantilla":"models"}' "$BASE/repositorios")
[ "$CODIGO" = "403" ] || falla "22 · un agente creo un repositorio ($CODIGO)"
# y borrar su carpeta se lleva el manifiesto: no hay verbo nuevo
[ "$(pide DELETE /arbol/packages/hr/raw)" = "200" ] || falla "22 · DELETE de la carpeta del repositorio · $(cat "$TMP/r.json")"
cumple "d['carpeta'] is True and 'packages/hr/raw/README.md' in d['ficheros']" "22 · el manifiesto se va con la carpeta"
[ "$(pide GET /assets)" = "200" ] && cumple "d['repositorios']==[]" "22 · y el indice ya no lo trae"
[ "$(pide DELETE /proyectos/personas)" = "200" ] || falla "22 · no se pudo retirar el proyecto de la prueba"
dice "22 · /repositorios: nace ENTERO (manifiesto + semilla + el proyecto que lo nombra, en UN commit) · la carpeta cogida 409 · la clase inventada 422 con las que hay · PUT conserva la prosa · desde un puesto 403 · borrar la carpeta se lleva el manifiesto"

# ── 23 · lo acotado: el editor abre SU carpeta (0036 ④) ─────────────────────
[ "$(pide POST /repositorios '{"paquete":"hr","carpeta":"raw","nombre":"Raw","plantilla":"transforms"}')" = "201" ] \
  || falla "23 · el repositorio de la prueba no entro · $(cat "$TMP/r.json")"
[ "$(pide POST /repositorios '{"paquete":"hr","carpeta":"clean","nombre":"Clean","plantilla":"analytics"}')" = "201" ] \
  || falla "23 · el segundo repositorio no entro · $(cat "$TMP/r.json")"
[ "$(pide GET /arbol)" = "200" ] || falla "23 · GET /arbol · $(cat "$TMP/r.json")"
TODOS=$(campo "len(d['ficheros'])")
CABEZA_ARBOL=$(campo "d['cabeza']")
acotado() { # ruta-raiz
  curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$SUJ" -H "x-ore-raiz: $1" "$BASE/arbol"
}
[ "$(acotado packages/hr/raw)" = "200" ] || falla "23 · GET /arbol acotado · $(cat "$TMP/r.json")"
cumple "d['raiz']=='packages/hr/raw' and d['cabeza']=='$CABEZA_ARBOL'" "23 · dice su raiz, y la cabeza es la del arbol (el arbol es uno)"
cumple "all(f['ruta'].startswith('packages/hr/raw/') for f in d['ficheros'])" "23 · solo lo suyo"
cumple "0 < len(d['ficheros']) < $TODOS" "23 · menos que la celda entera ($TODOS)"
cumple "any(f['ruta']=='packages/hr/raw/README.md' for f in d['ficheros']) and any(f['ruta']=='packages/hr/raw/transforms/ejemplo.py' for f in d['ficheros'])" "23 · el manifiesto y la semilla, dentro"
[ "$(acotado packages/hr/clean)" = "200" ] && cumple "all('clean' in f['ruta'] for f in d['ficheros'])" "23 · el de al lado ve lo suyo, no lo de este"
[ "$(acotado otra/cosa/aqui)" = "422" ] || falla "23 · un alcance que no es una carpeta de paquete no dio 422"
[ "$(acotado packages/hr/noexiste)" = "404" ] || falla "23 · una carpeta que no esta no dio 404"
[ "$(pide GET /arbol)" = "200" ] && cumple "len(d['ficheros'])==$TODOS and 'raiz' not in d" "23 · sin cabecera, la celda entera como siempre"
# y las propuestas, acotadas (aqui el arbol es solo main: la lista esta vacia,
# pero el alcance viaja y se dice)
CODIGO=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$SUJ" -H "x-ore-raiz: packages/hr/raw" "$BASE/propuestas")
# (este arbol es `file://`, sin API de forja: 422 «no sabe de propuestas». Lo que
#  se comprueba aqui es que la CABECERA no rompe la ruta; el filtro de verdad lo
#  mide `medida-el-repositorio.py` §4 contra una forja con API.)
case "$CODIGO" in 200|422|501|502) ;; *) falla "23 · GET /propuestas acotado dio $CODIGO · $(cat "$TMP/r.json")";; esac
CODIGO=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$SUJ" -H "x-ore-raiz: otra/cosa" "$BASE/propuestas")
[ "$CODIGO" = "422" ] || falla "23 · un alcance invalido en /propuestas no dio 422"
[ "$(pide DELETE /arbol/packages/hr/raw)" = "200" ] || falla "23 · no se pudo retirar el repositorio de la prueba"
[ "$(pide DELETE /arbol/packages/hr/clean)" = "200" ] || falla "23 · no se pudo retirar el segundo"
dice "23 · lo acotado: GET /arbol con X-Ore-Raiz trae SOLO su carpeta (la cabeza sigue siendo la del arbol) · el de al lado ve lo suyo · sin cabecera, la celda entera · 422 lo que no es carpeta de paquete, 404 lo que no esta"

echo
echo "ok · /documentos/{kind}: un motor, una tabla de kinds — Entity, View, Table, Concept, Interface, TrainedModel, Dataset, Function, Action — y /conceptos; /arbol por ruta (0030 W0); /proyectos, la lente (0035 ②); las carpetas, enteras y en un commit (0035 ③b); /repositorios, la unidad de trabajo (0036 ②); el arbol acotado a un repositorio (0036 ④); escribir es un commit del sujeto que no empeora el arbol"
