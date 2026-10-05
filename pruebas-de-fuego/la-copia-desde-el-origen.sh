#!/usr/bin/env bash
# LA COPIA DESDE EL ORIGEN (ADR 0053 F7·2) — una vista SQL que lee una `Table`
# de un origen, copiada por el Job: `ore materialize --preparar` lee la tabla
# con lo que el reparto empuja (en modo copia: sin tope en vivo), y
# `python -m ore.calcular` la calcula en DuckDB. Postgres de verdad detrás.
#
#   PG_URL=postgres://postgres:x@localhost:5432 ORE=target/debug/ore \
#     bash pruebas-de-fuego/la-copia-desde-el-origen.sh
#
# `ore-read-postgres` se busca junto a `ore`. Python con duckdb y pyarrow.
set -u
ORE="${ORE:-target/debug/ore}"
abs() { echo "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; }
ORE=$(abs "$ORE")
export PATH="$(dirname "$ORE"):$PATH"
PG_URL="${PG_URL:-postgres://postgres:x@localhost:5432}"
PY=$(command -v python3 || command -v python)
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
MAL=0
falla() { echo "  ✗ $*"; MAL=1; }
dice()  { echo "  ✓ $*"; }

psql "$PG_URL/postgres" -qc "drop database if exists copia" -qc "create database copia" >/dev/null || { echo "sin Postgres en $PG_URL"; exit 1; }
psql "$PG_URL/copia" -q -v ON_ERROR_STOP=1 <<'SQL' >/dev/null || { echo "no se sembró el origen"; exit 1; }
create table pedidos (id integer primary key, cliente text, total integer, fecha date);
insert into pedidos select g, 'c' || (g % 7), g * 10, date '2026-01-01' + (g % 300) from generate_series(1, 5000) g;
create table clientes (id text primary key, pais text);
insert into clientes select 'c' || g, case when g % 2 = 0 then 'ES' else 'PT' end from generate_series(0, 6) g;
SQL
export COPIA_PG_URL="$PG_URL/copia"

A="$TMP/arbol"; mkdir -p "$A/packages/pg/public/tables" "$A/packages/informes/views" "$A/packages/informes/datasets"
cat > "$A/ontology.config.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: copia, version: 0.1.0 }
datasources:
  - name: pg
    type: postgres
    connectionEnv: COPIA_PG_URL
YAML
cat > "$A/conduits.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: copia }
spec:
  owner: team:copia
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
    federation.read: { oos.maturity: DRAFT }
YAML
cat > "$A/packages/pg/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: pg, version: 0.1.0, status: draft, domain: pg }
spec: { owner: "team:copia", exports: [pg.public.pedidos, pg.public.clientes] }
YAML
cat > "$A/packages/pg/public/schema.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: public, namespace: pg }
spec: { owner: team:copia }
YAML
tabla() { # nombre columnas…
cat > "$A/packages/pg/public/tables/$1.yaml" <<YAML
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: $1, namespace: pg, schema: public }
spec:
  datasource: pg
  object: "public.$1"
  columns:
$2
  reads:
    fullScan: forbidden
    predicatePushdown: [eq, neq, in, range, isNull]
  changes:
    key: [id]
YAML
}
tabla pedidos "    id: { type: Integer, physicalType: integer, required: true }
    cliente: { type: String, physicalType: text }
    total: { type: Integer, physicalType: integer }
    fecha: { type: Date, physicalType: date }"
tabla clientes "    id: { type: String, physicalType: text, required: true }
    pais: { type: String, physicalType: text }"
cat > "$A/packages/informes/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: informes, version: 0.1.0, status: draft, domain: informes }
spec: { owner: "team:copia", dependencies: [{ name: pg, version: "0.1.0" }] }
YAML
# Una vista materializada que junta, filtra con un RANGO y agrega: lo que el
# `where` de un dataset nunca pudo decir.
cat > "$A/packages/informes/views/ventas_es.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: ventas_es, namespace: informes }
spec:
  owner: team:copia
  dialect: duckdb
  sql: |
    SELECT c.pais, count(*) AS pedidos, sum(p.total) AS total
    FROM pg.public.pedidos p JOIN pg.public.clientes c ON c.id = p.cliente
    WHERE p.fecha >= DATE '2026-06-01' AND c.pais = 'ES'
    GROUP BY c.pais
  columns:
    pais: { type: String }
    pedidos: { type: Integer }
    total: { type: Integer }
YAML
cat > "$A/packages/informes/datasets/ventas_es_copia.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata: { name: ventas_es_copia, namespace: informes }
spec:
  owner: team:copia
  from: { view: informes.ventas_es }
YAML

# ── 1 · el árbol compila, y `ore view` dice de dónde se lee ─────────────────
"$ORE" validate "$A" >"$TMP/v.txt" 2>&1 && dice "1 · el árbol compila (la copia de una vista que lee el origen)" || falla "1 · no compila: $(tail -5 "$TMP/v.txt")"
"$ORE" view "$A" >"$TMP/vista.txt" 2>&1
grep -q "raíz      pg" "$TMP/vista.txt" && dice "1 · \`ore view\` da la raíz pg (el Job pide su credencial)" || falla "1 · ore view: $(grep -n "ra" "$TMP/vista.txt" | head -5)"

# ── 2 · preparar: la tabla leída con lo empujado ────────────────────────────
D="$TMP/calc"; mkdir -p "$D"
( cd "$A" && "$ORE" materialize . --preparar "$D" --vista informes.ventas_es_copia ) >"$TMP/prep.txt" 2>&1 \
  || falla "2 · preparar: $(tail -8 "$TMP/prep.txt")"
C="$D/informes.ventas_es_copia"
[ -f "$C/entradas/pg.public.pedidos.arrow" ] && [ -f "$C/entradas/pg.public.clientes.arrow" ] \
  && dice "2 · preparar leyó las dos tablas del origen a entradas/" || falla "2 · sin entradas: $(ls "$C" "$C/entradas" 2>&1 | tr '\n' ' ') $(tail -5 "$TMP/prep.txt")"
grep -q "leída del origen · \`pg.public.pedidos\` (pg · 3 columnas, 1 filtros empujados)" "$TMP/prep.txt" \
  && dice "2 · pedidos: 3 columnas y el rango de fecha empujado al origen" || falla "2 · lo empujado: $(grep leída "$TMP/prep.txt")"
grep -qF '__ore_dataset\".\"pg.public.pedidos' "$C/peticion.json" && dice "2 · la consulta servida lee la tabla como un dataset más" || falla "2 · consulta: $(grep consulta "$C/peticion.json")"
FILAS=$("$PY" -c 'import pyarrow.ipc as i,sys;print(i.open_stream(sys.argv[1]).read_all().num_rows)' "$C/entradas/pg.public.pedidos.arrow")
[ "$FILAS" -lt 5000 ] && [ "$FILAS" -gt 0 ] && dice "2 · el origen devolvió $FILAS de 5000 filas (filtradas allí, no aquí)" || falla "2 · filas: $FILAS"

# ── 3 · calcular: DuckDB sobre lo leído ─────────────────────────────────────
( cd "$RAIZ/puesto/python" && "$PY" -m ore.calcular "$D" ) >"$TMP/calc.txt" 2>&1 || falla "3 · calcular: $(tail -5 "$TMP/calc.txt")"
ESPERA=$(psql "$PG_URL/copia" -tAc "select count(*)||'|'||sum(p.total) from pedidos p join clientes c on c.id = p.cliente where p.fecha >= date '2026-06-01' and c.pais = 'ES'")
SALE=$("$PY" -c 'import pyarrow.ipc as i,sys;t=i.open_stream(sys.argv[1]).read_all().to_pylist();print("%s|%s"%(t[0]["pedidos"],t[0]["total"]) if t else "nada")' "$C/salida.arrow" 2>&1)
[ "$SALE" = "$ESPERA" ] && dice "3 · la copia calculada casa con el origen: $SALE (pedidos|total)" || falla "3 · sale $SALE, el origen dice $ESPERA"

echo
[ "$MAL" = 0 ] && echo "✓ la copia desde el origen (0053 F7·2)" || { echo "✗ la copia desde el origen"; exit 1; }
