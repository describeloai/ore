#!/usr/bin/env bash
# LA BASE ESTANDAR (0027 P1 I4b): la copia se induce, no se edita — contra un
# `ore-serve` y un `ore` de verdad, un catalogo como el que el Job deja y una
# cola de trabajo.
#
# Lo que fija:
#
#   0  una base a mano (sin alcance) es foranea y no declara copia:
#      GET /paquetes → type foreign, copias 0/0 · GET /copias → []
#   1  POST /paquetes {type: raro}      422 · nada escrito · y un nombre con guion (no puede ser
#                                       espacio de nombres, OOS2030): 422 antes de escribir nada
#   2  POST /paquetes {type: standard}  200 · la regla en discover.scope.json · EL CATALOGO NO
#                                       MODELA (C1): 0 entidades, 2 tablas, 2 vistas, y las DOS
#                                       con `materialized` — la copia no espera a ninguna clave;
#                                       orders (con clave en el origen) en `upsert`, customers
#                                       como el origen la dijo · solo `dueno` en la cola · el
#                                       conducto espera al dueño · un Job con las dos ·
#                                       GET /paquetes: standard, 2/0 · GET /copias: las dos, pendientes
#   3  contestar `dueno`                200 · `review` re-induce y CONSERVA las dos copias (lo que
#                                       el verbo a mano perdia) · conduits.yaml nace con el dueño ·
#                                       el arbol compila · el Job no cambia (misma lista)
#   3b `ore model tienda olist.customers`  la entidad Customers y su cola (clave, con «la COPIA
#                                       espera») · la copia de customers ESPERA (respalda una
#                                       entidad sin identidad) · orders sigue · contestar `clave`
#                                       la trae de vuelta, en upsert por customer_id · compila
#      (C2) GET /esquema trae `tables` desde tables/ (columnas con physicalType, view, modeled);
#      modelar es POST /paquetes/{n}/tablas/{objeto}/modelar (201 · 409 si ya · 404 si no esta);
#      GET /paquetes dice tablas y modeladas
#   4  ascender                         409 si ya es estandar · 422 si no es una base (sin alcance)
#   5  el informe del Job en el arbol   GET /copias: copiada, filas, copiado_por, cuando · 2/1
#   7  DELETE /paquetes/{n}             409 para la fuente entera (pg) · 200 para una base: el
#                                       paquete fuera, la cola reencolada con las copias que quedan ·
#                                       404 despues · el arbol compila
#   6  una base foranea (sin type)      200 · nada con copia · COPIAR UNA TABLA (POST
#                                       /tablas/{o}/copiar): 201, la base sigue foranea y solo
#                                       esa vista copia (`copies` en el alcance, `copied` en el
#                                       esquema, el Job la lleva) · otra vez 409 · y al ascender
#                                       la base: 201, la regla, las dos con copia, el Job con las
#                                       cuatro · copiar una tabla de una estandar: 409
#
# Uso:  bash pruebas-de-fuego/la-copia-se-decide.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8909}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""

falla() {
  echo "✗ $*" >&2
  [ -s "$TMP/arranque.txt" ] && { echo "── lo que dijo el servidor ──" >&2; tail -20 "$TMP/arranque.txt" >&2; }
  limpiar; exit 1
}
dice()  { echo "  · $*"; }
limpiar() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; sleep 0.3; rm -rf "$TMP"; }
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

# ── el arbol: una base a mano (olist, foranea) y la fuente `pg` con su catalogo ──
REPO="$TMP/repo"
mkdir -p "$REPO/packages/olist/tables" "$REPO/packages/olist/views" "$REPO/packages/olist/entities" "$REPO/packages/pg"
cat > "$REPO/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: pg, type: postgres, connectionEnv: PG_URL }
Y
cat > "$REPO/packages/olist/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: olist, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$REPO/packages/olist/tables/customers.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: olist_customers, namespace: olist }
spec:
  datasource: pg
  object: "olist.customers"
  columns:
    customer_id: {}
    customer_city: {}
  reads:
    fullScan: cheap
  changes:
    mode: append
    witness: log
Y
cat > "$REPO/packages/olist/tables/orders.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: olist_orders, namespace: olist }
spec:
  datasource: pg
  object: "olist.orders"
  columns:
    order_id: {}
    customer_id: {}
  reads:
    fullScan: cheap
  changes:
    mode: append
    witness: log
Y
cat > "$REPO/packages/olist/views/customers.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: customers, namespace: olist }
spec:
  owner: team:data
  from: { table: olist_customers }
  fields: { customer_id: customer_id, customer_city: customer_city }
Y
cat > "$REPO/packages/olist/views/orders.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: orders, namespace: olist }
spec:
  owner: team:data
  from: { table: olist_orders }
  fields: { order_id: order_id, customer_id: customer_id }
Y
cat > "$REPO/packages/olist/views/pedidos.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: pedidos, namespace: olist }
spec:
  owner: team:data
  from: { view: orders }
  fields: { id: order_id }
Y
# Lo que el Job de catalogo deja: el paquete de la fuente con `discover.catalog.json`.
# Dos objetos, uno con clave primaria y otro sin, como los sondearia el driver.
cat > "$REPO/packages/pg/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: pg, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$REPO/packages/pg/discover.catalog.json" <<'J'
{
  "source": "pg",
  "tables": [
    { "name": "olist.customers",
      "columns": [ { "name": "customer_id", "type": "String", "sourceType": "character varying(32)", "required": true }, { "name": "customer_city", "type": "String" } ],
      "reads": { "fullScan": "cheap" },
      "changes": { "mode": "append", "witness": "log" } },
    { "name": "olist.orders",
      "columns": [ { "name": "order_id", "type": "String", "required": true }, { "name": "customer_id", "type": "String" } ],
      "primaryKey": ["order_id"],
      "reads": { "fullScan": "cheap" },
      "changes": { "mode": "upsert", "key": ["order_id"], "witness": "log" } }
  ]
}
J
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol de partida no compila: $(cd "$REPO" && "$ORE" validate . 2>&1 | head -3)"
# con historia, para que el informe (7) lleve quien y cuando
( cd "$REPO" && git init -q && git config core.autocrlf false && git -c user.name=banco -c user.email=banco@invalido add -A \
  && git -c user.name=banco -c user.email=banco@invalido commit -q -m "el arbol de partida" ) || falla "no se pudo dar historia al arbol"

# ── la cola de trabajo: un repositorio pelado con la plantilla rendida ─────
COLA="$TMP/cola.git"
git init -q --bare -b main "$COLA"
mkdir -p "$TMP/cola-semilla" && ( cd "$TMP/cola-semilla" && git init -q && git config core.autocrlf false )
PY=$(command -v python3 || command -v python)
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TMP/rendido" >/dev/null 2>&1 || falla "no se pudo rendir la plantilla de la copia"
cp "$TMP/rendido/plantilla-copia.txt" "$TMP/cola-semilla/"
( cd "$TMP/cola-semilla" && git add -A && git -c user.name=banco -c user.email=banco@invalido commit -q -m "la plantilla" \
  && git remote add origin "$COLA" && git push -q origin HEAD:main ) || falla "no se pudo sembrar la cola"
en_cola() { git --git-dir="$COLA" show "main:$1" 2>/dev/null; }

FORJA_TOKEN=no-hace-falta-en-file "$SERVE" --repo "$REPO" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
         --cola "file://$COLA" --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
SUJ='x-ore-sujeto: persona:ana'
cuerpo() { cat "$TMP/r.json"; }
alta() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" -H 'content-type: application/json' "$BASE/paquetes" -d "$1"; }
asc() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" "$BASE/paquetes/$1/copia"; }
decidir() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" -H 'content-type: application/json' "$BASE/paquetes/$1/decisiones" -d "$2"; }
paquete() { curl -sf -H "$SUJ" "$BASE/paquetes" | "$PY" -c 'import json,sys; print(json.dumps([p for p in json.load(sys.stdin)["packages"] if p["name"]==sys.argv[1]][0], sort_keys=True))' "$1"; }
copias() { curl -sf -H "$SUJ" "$BASE/paquetes/$1/copias"; }
vista()  { cat "$REPO"/packages/$1/views/*__olist_$2.yaml; }
tabla()  { cat "$REPO"/packages/$1/tables/*__olist_$2.yaml; }

# ── 0 ───────────────────────────────────────────────────────────────────────
paquete olist | grep -q '"type": "foreign"' || falla "0 · la base a mano no sale foreign: $(paquete olist)"
paquete olist | grep -q '"copias": {"copiadas": 0, "declaradas": 0}' || falla "0 · la base a mano declara copias: $(paquete olist)"
copias olist | grep -q '"copias":\[\]' || falla "0 · GET /copias de la base a mano no esta vacio: $(copias olist)"
dice "0 · una base a mano es foranea: GET /paquetes foreign, 0/0 · GET /copias []"

# ── 1 ───────────────────────────────────────────────────────────────────────
COD=$(alta '{"name":"tienda","source":"pg","only":["olist.customers","olist.orders"],"type":"raro"}')
[ "$COD" = "422" ] || falla "1 · un type raro devolvio $COD: $(cuerpo)"
[ ! -e "$REPO/packages/tienda" ] || falla "1 · un type raro dejo el paquete escrito"
COD=$(alta '{"name":"test-standard","source":"pg","only":["olist.orders"],"type":"standard"}')
[ "$COD" = "422" ] || falla "1 · un nombre con guion devolvio $COD: $(cuerpo)"
cuerpo | grep -q "no puede ser un espacio de nombres" || falla "1 · no dijo por que el guion no vale: $(cuerpo)"
[ ! -e "$REPO/packages/test-standard" ] || falla "1 · un nombre con guion dejo el paquete escrito"
dice "1 · type raro: 422 y nada escrito · nombre con guion: 422 con la regla, y nada escrito"

# ── 2 ───────────────────────────────────────────────────────────────────────
COD=$(alta '{"name":"tienda","source":"pg","only":["olist.customers","olist.orders"],"type":"standard"}')
[ "$COD" = "200" ] || falla "2 · la base estandar devolvio $COD: $(cuerpo)"
cuerpo | grep -q '"type":"standard"' || falla "2 · la respuesta no dice la clase: $(cuerpo)"
cuerpo | grep -q '"copias":{"copiadas":0,"declaradas":2}' || falla "2 · la respuesta no cuenta las dos copias: $(cuerpo)"
cuerpo | grep -q '0 entidades, 2 tablas y 2 vistas' || falla "2 · el catalogo modelo algo: $(cuerpo)"
[ ! -d "$REPO/packages/tienda/entities" ] || [ -z "$(ls -A "$REPO/packages/tienda/entities" 2>/dev/null)" ] || falla "2 · el catalogo escribio entidades: $(ls "$REPO/packages/tienda/entities")"
grep -q '"entities": \[\]' "$REPO/packages/tienda/discover.scope.json" || falla "2 · el alcance no dice que ninguna esta modelada: $(cat "$REPO/packages/tienda/discover.scope.json")"
cuerpo | grep -q '"encolado":"NO encolado: el conducto espera al dueño' || falla "2 · sin conducto encolo un Job que fallaria: $(cuerpo)"
grep -q '"type": "standard"' "$REPO/packages/tienda/discover.scope.json" || falla "2 · el alcance no lleva la regla: $(cat "$REPO/packages/tienda/discover.scope.json")"
vista tienda orders | grep -q 'materialized: { datasource: pg, table: "copia.orders" }' || falla "2 · orders (con clave) nacio sin copia: $(vista tienda orders)"
tabla tienda orders | grep -q "mode: upsert" || falla "2 · la tabla orders no esta en upsert: $(tabla tienda orders)"
tabla tienda orders | grep -q "key: \[order_id\]" || falla "2 · la tabla orders no lleva la clave: $(tabla tienda orders)"
vista tienda customers | grep -q 'materialized: { datasource: pg, table: "copia.customers" }' || falla "2 · customers (sin clave, sin modelar) no se copia: $(vista tienda customers)"
tabla tienda customers | grep -q "mode: append" || falla "2 · customers sin clave no se queda como el origen la dijo: $(tabla tienda customers)"
grep -q '"clave/' "$REPO/packages/tienda/discover.pending.json" && falla "2 · el catalogo pregunta por la clave: $(grep -o '"id": "[^"]*"' "$REPO/packages/tienda/discover.pending.json")"
[ "$(grep -o '"id": "[^"]*"' "$REPO/packages/tienda/discover.pending.json" | sort -u)" = '"id": "dueno/tienda"' ] || falla "2 · la cola del catalogo no es solo el dueño: $(grep -o '"id": "[^"]*"' "$REPO/packages/tienda/discover.pending.json")"
[ ! -e "$REPO/conduits.yaml" ] || falla "2 · conduits.yaml nacio con el dueño sin decidir (cambiame)"
cuerpo | grep -q '"conducto":{"error":"`conduits.yaml` no nace hasta que `tienda` tenga dueño' || falla "2 · no dijo que el conducto espera al dueño: $(cuerpo)"
en_cola 48-la-copia.yaml >/dev/null && falla "2 · hay un Job en la cola antes de que la copia pueda compilar"
paquete tienda | grep -q '"type": "standard"' || falla "2 · GET /paquetes no dice standard: $(paquete tienda)"
paquete tienda | grep -q '"copias": {"copiadas": 0, "declaradas": 2}' || falla "2 · GET /paquetes no cuenta 2/0: $(paquete tienda)"
paquete tienda | grep -q '"modeladas": 0' && paquete tienda | grep -q '"tablas": 2' || falla "2 · GET /paquetes no dice 2 tablas, 0 modeladas: $(paquete tienda)"
esquema() { curl -sf -H "$SUJ" "$BASE/paquetes/$1/esquema"; }
esquema tienda | grep -q '"entities":\[\]' || falla "2 · el esquema trae entidades que no hay: $(esquema tienda)"
esquema tienda | grep -q '"columns":\[{"name":"customer_id","physicalType":"character varying(32)"},{"name":"customer_city"}\],"copied":true,"datasource":"pg","modeled":false,"name":"olist_customers","object":"olist.customers","view":"customers"' || falla "2 · el esquema no trae las tablas desde tables/: $(esquema tienda)"
copias tienda | grep -q '"copia":{"estado":"pendiente"},"key":\["order_id"\],.*"view":"orders"' || falla "2 · GET /copias no lista orders con su clave, pendiente: $(copias tienda)"
copias tienda | grep -q '"copia":{"estado":"pendiente"},"key":\[\],.*"view":"customers"' || falla "2 · GET /copias no lista customers sin clave, pendiente: $(copias tienda)"
dice "2 · la base estandar: 200 · el catalogo no modela: 0 entidades, 2 tablas, 2 vistas · las DOS con copia (orders en upsert, customers como el origen) · solo dueno en la cola · el conducto espera al dueño y NO se encola todavia · GET /paquetes standard 2/0"

# ── 3 ───────────────────────────────────────────────────────────────────────
COD=$(decidir tienda '{"answers":{"dueno/tienda":"team:data"}}')
[ "$COD" = "200" ] || falla "3 · contestar devolvio $COD: $(cuerpo)"
vista tienda customers | grep -q 'copia.customers' || falla "3 · la re-induccion perdio la copia de customers: $(vista tienda customers)"
vista tienda orders | grep -q 'copia.orders' || falla "3 · la re-induccion perdio la copia de orders: $(vista tienda orders)"
cuerpo | grep -q '"copias":{"copiadas":0,"declaradas":2}' || falla "3 · la respuesta no cuenta las dos: $(cuerpo)"
cuerpo | grep -q '"encolado":"encolado como `48-la-copia.yaml`' || falla "3 · con el conducto, no encolo el Job: $(cuerpo)"
en_cola 48-la-copia.yaml | grep -q 'name: VISTAS, value: "tienda.customers,tienda.orders"' || falla "3 · el Job no lleva las dos: $(en_cola 48-la-copia.yaml | grep -n VISTAS)"
NOMBRE2=$(en_cola 48-la-copia.yaml | sed -n 's/^  name: \(copiar-[0-9a-f]*\)$/\1/p')
[ -n "$NOMBRE2" ] || falla "3 · el Job no se llama copiar-<resumen>"
grep -q "materialization.payload" "$REPO/conduits.yaml" || falla "3 · con el dueño decidido, conduits.yaml no nacio"
grep -q "owner: team:data" "$REPO/conduits.yaml" || falla "3 · conduits.yaml no lleva el dueño del paquete: $(cat "$REPO/conduits.yaml")"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "3 · el arbol no compila con la base estandar: $(cd "$REPO" && "$ORE" validate . 2>&1 | grep -A1 "^error" | head -6)"
dice "3 · dueño contestado: las dos copias se conservan · conduits.yaml nace con el dueño · y AHORA el Job $NOMBRE2 con las dos · el arbol compila"

# ── 3b · modelar una tabla: la entidad, su cola, y la copia que ahora espera ──
modelar() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" "$BASE/paquetes/$1/tablas/$2/modelar"; }
COD=$(modelar tienda olist.nadie)
[ "$COD" = "404" ] || falla "3b · modelar un objeto fuera del alcance devolvio $COD: $(cuerpo)"
COD=$(modelar tienda olist.customers)
[ "$COD" = "201" ] || falla "3b · modelar devolvio $COD: $(cuerpo)"
cuerpo | grep -q '"copias":{"copiadas":0,"declaradas":1}' || falla "3b · modelar no dijo que la copia de customers espera: $(cuerpo)"
grep -q '"entities": \[' "$REPO/packages/tienda/discover.scope.json" && grep -q '"olist.customers"' "$REPO/packages/tienda/discover.scope.json" || falla "3b · el alcance no nombra la modelada: $(cat "$REPO/packages/tienda/discover.scope.json")"
[ -f "$REPO/packages/tienda/entities/Customers.yaml" ] || falla "3b · modelar no trajo la entidad: $(ls "$REPO/packages/tienda/entities" 2>&1)"
[ ! -f "$REPO/packages/tienda/entities/Orders.yaml" ] || falla "3b · modelar customers modelo tambien orders"
grep -q '"clave/olist.customers"' "$REPO/packages/tienda/discover.pending.json" || falla "3b · la cola no pregunta la clave de la modelada"
grep -q "la COPIA de esta tabla espera" "$REPO/packages/tienda/discover.pending.json" || falla "3b · la decision clave no dice que la copia la espera"
vista tienda customers | grep -q "materialized:" && falla "3b · modelada sin clave, la copia no espera: $(vista tienda customers)"
vista tienda orders | grep -q 'copia.orders' || falla "3b · modelar customers toco la copia de orders"
COD=$(modelar tienda olist.customers)
[ "$COD" = "409" ] || falla "3b · modelar dos veces devolvio $COD: $(cuerpo)"
paquete tienda | grep -q '"modeladas": 1' || falla "3b · GET /paquetes no cuenta la modelada: $(paquete tienda)"
esquema tienda | grep -q '"entity":"Customers".*"modeled":true,"name":"olist_customers"' || falla "3b · el esquema no dice que customers esta modelada: $(esquema tienda)"
COD=$(decidir tienda '{"answers":{"clave/olist.customers":["customer_id"]}}')
[ "$COD" = "200" ] || falla "3b · contestar la clave devolvio $COD: $(cuerpo)"
vista tienda customers | grep -q 'copia.customers' || falla "3b · con la clave contestada, la copia no vuelve: $(vista tienda customers)"
tabla tienda customers | grep -q "key: \[customer_id\]" || falla "3b · la tabla no lleva la clave contestada: $(tabla tienda customers)"
tabla tienda customers | grep -q "mode: upsert" || falla "3b · la tabla no paso a upsert"
grep -q "primaryKey: \[customer_id\]" "$REPO/packages/tienda/entities/Customers.yaml" || falla "3b · la entidad no lleva la clave: $(cat "$REPO/packages/tienda/entities/Customers.yaml")"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "3b · el arbol no compila con la modelada: $(cd "$REPO" && "$ORE" validate . 2>&1 | grep -A1 "^error" | head -6)"
dice "3b · POST modelar customers: 201 · su entidad y su cola (la clave, y la copia la espera) · la copia de customers espera, la de orders sigue · contestada la clave, vuelve en upsert y la entidad la lleva · compila"

# ── 4 ───────────────────────────────────────────────────────────────────────
COD=$(asc tienda)
[ "$COD" = "409" ] || falla "4 · ascender una estandar devolvio $COD: $(cuerpo)"
COD=$(asc olist)
[ "$COD" = "422" ] || falla "4 · ascender un paquete sin alcance devolvio $COD: $(cuerpo)"
COD=$(asc nadie)
[ "$COD" = "404" ] || falla "4 · ascender un paquete que no esta devolvio $COD: $(cuerpo)"
dice "4 · ascender: 409 si ya es estandar · 422 sin alcance · 404 si no esta"

# ── 5 · el informe que el Job deja, y la ficha lo trae ──────────────────────
mkdir -p "$REPO/copias"
printf '{\n  "estado": "copiada",\n  "vista": "tienda.orders",\n  "clave": "ore/v1/abc",\n  "digest": "sha256:abc",\n  "plan": "sha256:def",\n  "filas": 99441,\n  "leidas": 99441,\n  "bytes": 1234567,\n  "subido": true,\n  "testigo": { "modo": "log", "valor": "0/1A2B3C" }\n}\n' > "$REPO/copias/tienda_orders.json"
( cd "$REPO" && git add -A && GIT_AUTHOR_NAME=copiador GIT_AUTHOR_EMAIL=copiador@invalido git -c user.name=copiador -c user.email=copiador@invalido commit -q -m "Copia: tienda.orders" ) || falla "5 · no se pudo firmar el informe"
copias tienda | grep -q '"estado":"copiada".*"view":"orders"' || falla "5 · la ficha no trae el estado del informe: $(copias tienda)"
copias tienda | grep -q '"filas":99441' || falla "5 · la ficha no trae las filas: $(copias tienda)"
copias tienda | grep -q '"copiado_por":"copiador"' || falla "5 · la ficha no dice quien copio: $(copias tienda)"
copias tienda | grep -q '"cuando":"20' || falla "5 · la ficha no dice cuando: $(copias tienda)"
copias tienda | grep -q '"copia":{"estado":"pendiente"},.*"view":"customers"' || falla "5 · customers, sin informe, no sale pendiente: $(copias tienda)"
paquete tienda | grep -q '"copias": {"copiadas": 1, "declaradas": 2}' || falla "5 · GET /paquetes no cuenta la copiada: $(paquete tienda)"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "5 · el arbol no compila con copias/ dentro"
dice "5 · el informe del Job: copiada · 99441 filas · copiado_por copiador · cuando · customers pendiente · GET /paquetes 2/1"

# ── 6 · una foranea, y ascenderla ───────────────────────────────────────────
COD=$(alta '{"name":"espejo","source":"pg","only":["olist.customers","olist.orders"]}')
[ "$COD" = "200" ] || falla "6 · la base foranea devolvio $COD: $(cuerpo)"
cuerpo | grep -q '"type":"foreign"' || falla "6 · sin type no es foreign: $(cuerpo)"
grep -q '"type"' "$REPO/packages/espejo/discover.scope.json" && falla "6 · una foranea escribe type en el alcance (no hace falta: es lo que significa no decir nada)"
( vista espejo orders; vista espejo customers ) | grep -q "materialized:" && falla "6 · una foranea nacio con copia"
[ -z "$(ls -A "$REPO/packages/espejo/entities" 2>/dev/null)" ] || falla "6 · una foranea del catalogo modelo algo"
paquete espejo | grep -q '"type": "foreign"' || falla "6 · GET /paquetes no dice foreign: $(paquete espejo)"
# una tabla, una a una: la base sigue foranea
copiar() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" "$BASE/paquetes/$1/tablas/$2/copiar"; }
COD=$(copiar espejo olist.orders)
[ "$COD" = "201" ] || falla "6 · copiar una tabla devolvio $COD: $(cuerpo)"
cuerpo | grep -q '"copias":{"copiadas":0,"declaradas":1}' || falla "6 · copiar una tabla no cuenta 1 declarada: $(cuerpo)"
cuerpo | grep -q '"encolado":"encolado como `48-la-copia.yaml`' || falla "6 · copiar una tabla no encolo el Job: $(cuerpo)"
grep -q '"copies": \[' "$REPO/packages/espejo/discover.scope.json" && grep -q '"olist.orders"' "$REPO/packages/espejo/discover.scope.json" || falla "6 · el alcance no lleva copies: $(cat "$REPO/packages/espejo/discover.scope.json")"
grep -q '"type"' "$REPO/packages/espejo/discover.scope.json" && falla "6 · copiar una tabla cambio la clase"
vista espejo orders | grep -q 'copia.orders' || falla "6 · la tabla copiada no lleva la copia: $(vista espejo orders)"
vista espejo customers | grep -q "materialized:" && falla "6 · copiar orders copio tambien customers"
paquete espejo | grep -q '"type": "foreign"' && paquete espejo | grep -q '"copias": {"copiadas": 0, "declaradas": 1}' || falla "6 · GET /paquetes: sigue foreign con 1 copia: $(paquete espejo)"
esquema espejo | grep -q '"copied":true,.*"name":"olist_orders"' && esquema espejo | grep -q '"copied":false,.*"name":"olist_customers"' || falla "6 · el esquema no dice cual esta copiada: $(esquema espejo)"
en_cola 48-la-copia.yaml | grep -q 'name: VISTAS, value: "espejo.orders,tienda.customers,tienda.orders"' || falla "6 · el Job no lleva espejo.orders: $(en_cola 48-la-copia.yaml | grep -n VISTAS)"
COD=$(copiar espejo olist.orders)
[ "$COD" = "409" ] || falla "6 · copiar dos veces devolvio $COD: $(cuerpo)"
COD=$(copiar espejo olist.nadie)
[ "$COD" = "404" ] || falla "6 · copiar fuera del alcance devolvio $COD: $(cuerpo)"
COD=$(asc espejo)
[ "$COD" = "201" ] || falla "6 · ascender espejo devolvio $COD: $(cuerpo)"
grep -q '"type": "standard"' "$REPO/packages/espejo/discover.scope.json" || falla "6 · ascender no escribio la regla"
vista espejo orders | grep -q 'copia.orders' || falla "6 · ascender no trajo la copia de orders: $(vista espejo orders)"
vista espejo customers | grep -q 'copia.customers' || falla "6 · ascender no copio customers (sin modelar, no espera a nada)"
cuerpo | grep -q '"copias":{"copiadas":0,"declaradas":2}' || falla "6 · la respuesta no cuenta 2/0: $(cuerpo)"
en_cola 48-la-copia.yaml | grep -q 'name: VISTAS, value: "espejo.customers,espejo.orders,tienda.customers,tienda.orders"' || falla "6 · el Job no lleva las cuatro vistas: $(en_cola 48-la-copia.yaml | grep -n VISTAS)"
paquete espejo | grep -q '"type": "standard"' || falla "6 · GET /paquetes no dice standard tras ascender: $(paquete espejo)"
COD=$(copiar espejo olist.customers)
[ "$COD" = "409" ] || falla "6 · copiar una tabla de una estandar devolvio $COD: $(cuerpo)"
dice "6 · una foranea nace sin copia · copiar UNA tabla: 201, sigue foranea, solo esa copia, el Job la lleva, 409 otra vez · al ascender: 201, la regla, las dos con copia, el Job con las cuatro · copiar en una estandar: 409"

# ── 7 · retirar una base ────────────────────────────────────────────────────
borrar() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/paquetes/$1"; }
COD=$(borrar pg)
[ "$COD" = "409" ] || falla "7 · retirar la fuente entera devolvio $COD: $(cuerpo)"
[ -d "$REPO/packages/pg" ] || falla "7 · la fuente entera se fue"
COD=$(borrar espejo)
[ "$COD" = "200" ] || falla "7 · retirar espejo devolvio $COD: $(cuerpo)"
[ ! -e "$REPO/packages/espejo" ] || falla "7 · espejo sigue en el arbol"
[ ! -e "$REPO/.retirando-espejo" ] || falla "7 · quedo el directorio temporal"
cuerpo | grep -q '"retirado":true' || falla "7 · la respuesta no dice retirado: $(cuerpo)"
cuerpo | grep -q '"encolado":"encolado como `48-la-copia.yaml`' || falla "7 · no reencolo la copia con lo que queda: $(cuerpo)"
en_cola 48-la-copia.yaml | grep -q 'name: VISTAS, value: "tienda.customers,tienda.orders"' || falla "7 · el Job no se quedo con las de tienda: $(en_cola 48-la-copia.yaml | grep -n VISTAS)"
COD=$(borrar espejo)
[ "$COD" = "404" ] || falla "7 · retirar dos veces devolvio $COD"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "7 · el arbol no compila sin espejo"
dice "7 · retirar: 409 la fuente entera · 200 la base, fuera del arbol y la copia reencolada con lo que queda · 404 despues · compila"
# 7b · lo que victor destapo (2026-09-17): el validador va por fases y se para en la
# primera. Una base con un nombre que no es espacio de nombres (`OOS2030`, fase de
# pertenencia) TAPA lo que otras bases tengan en fases posteriores (`OOS2009`, enlazado).
# Retirarla los destapa; no los causa: tiene que ser 200, no 422.
mkdir -p "$REPO/packages/mal-nombre/tables"
echo '{"fuente":"pg","objetos":["olist.customers"]}' > "$REPO/packages/mal-nombre/discover.scope.json"
cat > "$REPO/packages/mal-nombre/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: mal-nombre, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$REPO/packages/mal-nombre/tables/customers.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: customers, namespace: mal-nombre }
spec:
  datasource: pg
  object: "olist.customers"
  columns: { customer_id: {} }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: log }
Y
sed -i 's/owner: team:data/owner: cambiame/' "$REPO/packages/olist/package.yaml"
( cd "$REPO" && "$ORE" validate . 2>&1 | grep -q OOS2030 ) || falla "7b · el arbol no se para en OOS2030"
( cd "$REPO" && "$ORE" validate . 2>&1 | grep -q OOS2009 ) && falla "7b · el OOS2009 de olist no queda tapado: la premisa del caso no vale"
COD=$(borrar mal-nombre)
[ "$COD" = "200" ] || falla "7b · ⛔ retirar la base tapadora devolvio $COD (lo destapado no es empeorar): $(cuerpo)"
[ ! -e "$REPO/packages/mal-nombre" ] || falla "7b · mal-nombre sigue en el arbol"
( cd "$REPO" && "$ORE" validate . 2>&1 | grep -q OOS2009 ) || falla "7b · sin la tapadora el OOS2009 de olist no aparece"
sed -i 's/owner: cambiame/owner: team:data/' "$REPO/packages/olist/package.yaml"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "7b · el arbol no compila restaurado"
dice "7b · retirar la base que tapaba (OOS2030) destapa el OOS2009 de otra: 200, no 422"

echo "✓ la base estandar, y el catalogo no modela: 0–7b"
