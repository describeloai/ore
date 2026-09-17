#!/usr/bin/env bash
# LA DECISION DE LA COPIA (0027 P1 I2): `POST /paquetes/{n}/vistas/{v}/copia`,
# contra un `ore-serve` de verdad y un arbol como el que `discover` deja.
#
# Lo que fija:
#
#   0  el arbol de partida (Table + View del inductor, sin conducto) compila y
#      no declara ninguna copia: GET /paquetes/olist/copias → []
#   1  una vista que no esta            404
#   2  `key` con un campo que no existe 422 con los campos que hay · NADA escrito
#   3  la decision minima (sin clave)   201 · la vista lleva `materialized` con la
#                                       fuente de su tabla · `conduits.yaml` nace
#                                       con `materialization.payload` · compila
#   4  otra vez                         409: la decision esta tomada
#   5  con clave, en otra vista         201 · la tabla raiz gana `changes: mode:
#                                       upsert, key: [...]` · conduits.yaml no se
#                                       duplica · GET /copias las lista con su clave
#   6  una vista sobre una vista        422: la copia es de la de abajo
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

# ── el arbol: lo que discover deja de olist, en pequeño ─────────────────────
REPO="$TMP/repo"
mkdir -p "$REPO/packages/olist/tables" "$REPO/packages/olist/views" "$REPO/packages/olist/entities"
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
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol de partida no compila: $(cd "$REPO" && "$ORE" validate . 2>&1 | head -3)"

"$SERVE" --repo "$REPO" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
         --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
SUJ='x-ore-sujeto: persona:ana'
post() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" -H 'content-type: application/json' "$BASE/paquetes/olist/vistas/$1/copia" -d "$2"; }
cuerpo() { cat "$TMP/r.json"; }

# ── 0 ───────────────────────────────────────────────────────────────────────
curl -sf -H "$SUJ" "$BASE/paquetes/olist/copias" > "$TMP/c.json" || falla "0 · GET /copias no contesta"
grep -q '"copias":\[\]' "$TMP/c.json" || falla "0 · el arbol de partida declara copias: $(cat "$TMP/c.json")"
dice "0 · el arbol de partida compila y no declara ninguna copia"

# ── 1 ───────────────────────────────────────────────────────────────────────
COD=$(post nadie '{}')
[ "$COD" = "404" ] || falla "1 · una vista que no esta devolvio $COD: $(cuerpo)"
dice "1 · una vista que no esta: 404"

# ── 2 ───────────────────────────────────────────────────────────────────────
COD=$(post customers '{"key":["dni"]}')
[ "$COD" = "422" ] || falla "2 · una clave con un campo inexistente devolvio $COD: $(cuerpo)"
cuerpo | grep -q "customer_id, customer_city" || falla "2 · el 422 no dice los campos que hay: $(cuerpo)"
grep -q materialized "$REPO/packages/olist/views/customers.yaml" && falla "2 · escribio la vista con una clave mala"
[ ! -f "$REPO/conduits.yaml" ] || falla "2 · escribio conduits.yaml con una clave mala"
dice "2 · \`key\` con un campo que no existe: 422 con los campos · nada escrito"

# ── 3 ───────────────────────────────────────────────────────────────────────
COD=$(post customers '')
[ "$COD" = "201" ] || falla "3 · la decision minima devolvio $COD: $(cuerpo)"
grep -q 'materialized: { datasource: pg, table: "copia.customers" }' "$REPO/packages/olist/views/customers.yaml" || falla "3 · la vista no lleva materialized: $(cat "$REPO/packages/olist/views/customers.yaml")"
[ -f "$REPO/conduits.yaml" ] || falla "3 · no nacio conduits.yaml"
grep -q "materialization.payload" "$REPO/conduits.yaml" || falla "3 · conduits.yaml no autoriza el conducto"
grep -q "owner: team:data" "$REPO/conduits.yaml" || falla "3 · el dueño del conducto no es el del paquete"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "3 · el arbol no compila tras la decision: $(cd "$REPO" && "$ORE" validate . 2>&1 | head -3)"
cuerpo | grep -q '"escritos":\["conduits.yaml","packages/olist/views/customers.yaml"\]' || cuerpo | grep -q '"escritos":\["packages/olist/views/customers.yaml","conduits.yaml"\]' || falla "3 · la respuesta no dice que escribio: $(cuerpo)"
dice "3 · la decision minima: 201 · materialized con la fuente de su tabla · conduits.yaml nace · compila"

# ── 4 ───────────────────────────────────────────────────────────────────────
COD=$(post customers '{}')
[ "$COD" = "409" ] || falla "4 · decidir dos veces devolvio $COD: $(cuerpo)"
dice "4 · otra vez: 409, la decision esta tomada"

# ── 5 ───────────────────────────────────────────────────────────────────────
COD=$(post orders '{"key":["order_id"]}')
[ "$COD" = "201" ] || falla "5 · con clave devolvio $COD: $(cuerpo)"
grep -q "mode: upsert" "$REPO/packages/olist/tables/orders.yaml" || falla "5 · la tabla no paso a upsert: $(cat "$REPO/packages/olist/tables/orders.yaml")"
grep -q "key: \[order_id\]" "$REPO/packages/olist/tables/orders.yaml" || falla "5 · la tabla no lleva la clave"
grep -q "witness: log" "$REPO/packages/olist/tables/orders.yaml" || falla "5 · el testigo se perdio"
[ "$(grep -c materialization.payload "$REPO/conduits.yaml")" = "1" ] || falla "5 · el conducto se duplico"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "5 · el arbol no compila con la clave: $(cd "$REPO" && "$ORE" validate . 2>&1 | head -3)"
curl -sf -H "$SUJ" "$BASE/paquetes/olist/copias" > "$TMP/c.json" || falla "5 · GET /copias no contesta"
grep -q '"view":"customers"' "$TMP/c.json" || falla "5 · GET /copias no lista customers: $(cat "$TMP/c.json")"
grep -q '"key":\["order_id"\]' "$TMP/c.json" || falla "5 · GET /copias no da la clave de orders: $(cat "$TMP/c.json")"
dice "5 · con clave: 201 · la tabla raiz en upsert con key · el conducto no se duplica · GET /copias lista las dos, con su clave"

# ── 6 ───────────────────────────────────────────────────────────────────────
COD=$(post pedidos '{}')
[ "$COD" = "422" ] || falla "6 · una vista sobre una vista devolvio $COD: $(cuerpo)"
cuerpo | grep -q "hereda" || falla "6 · el 422 no dice que la copia es de la de abajo: $(cuerpo)"
dice "6 · una vista sobre una vista: 422, la copia es de la de abajo"

echo "✓ la decision de la copia: 0–6"
