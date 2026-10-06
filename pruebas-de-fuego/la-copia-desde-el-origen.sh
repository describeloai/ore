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
# Y una copia de siempre: un Dataset de una Table (la lee `ore materialize`).
cat > "$A/packages/informes/datasets/clientes_copia.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata: { name: clientes_copia, namespace: informes }
spec:
  owner: team:copia
  from: { table: pg.public.clientes }
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

# ── 1b · un paquete roto que no copia nada no deja sin raíces a los demás ──
#   (victor, 2026-10-06: un `.sql` roto en otro paquete hacía fallar `ore view`
#   entero y el Job de la copia no pedía ninguna credencial.)
mkdir -p "$A/packages/roto/views"
cat > "$A/packages/roto/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: roto, version: 0.1.0, status: draft, domain: roto }
spec: { owner: "team:copia" }
YAML
cat > "$A/packages/roto/views/mala.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: mala, namespace: roto }
spec:
  owner: team:copia
  dialect: duckdb
  sql: SELECT x FROM roto.no_existe
  columns:
    x: { type: String }
YAML
"$ORE" validate "$A" >/dev/null 2>&1 && falla "1b · el paquete roto compila (la prueba no prueba nada)"
"$ORE" view "$A" >"$TMP/vista-rota.txt" 2>&1; RC=$?
[ "$RC" = 0 ] && grep -q "raíz      pg" "$TMP/vista-rota.txt" && grep -q "su paquete no compila" "$TMP/vista-rota.txt"   && dice "1b · con un paquete roto al lado, \`ore view\` sigue dando la raíz pg (y dice cuál no compila)"   || falla "1b · ore view con un paquete roto: rc=$RC $(grep -n "raíz\|error" "$TMP/vista-rota.txt" | head -5)"
rm -rf "$A/packages/roto"

# ── 4 · 0053 F8·3 · lo mismo, por la pasarela ────────────────────────────────
#   Con `ORE_PASARELA`, `--preparar` no lanza el conector: pide cada tabla a
#   `ore-federation` con `perfil: "copia"` (en la cola del origen, sin tope).
FED="${FED:-$(dirname "$ORE")/ore-federation}"
if [ -x "$FED" ]; then
  libre() { "$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])'; }
  PF=$(libre)
  "$FED" --escucha "127.0.0.1:$PF" --conectores "$(dirname "$ORE")" --tipos postgres >"$TMP/fed.log" 2>&1 &
  FPID=$!; trap 'kill $FPID 2>/dev/null; rm -rf "$TMP"' EXIT
  for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PF/v1/health" && break; sleep 0.25; done
  D2="$TMP/calc2"; mkdir -p "$D2"
  # Sin conectores en el PATH: si `ore` lanzara uno, fallaría.
  ( cd "$A" && PATH=/usr/bin:/bin ORE_PASARELA="127.0.0.1:$PF" "$ORE" materialize . --preparar "$D2" --vista informes.ventas_es_copia ) >"$TMP/prep2.txt" 2>&1     || falla "4 · preparar por la pasarela: $(tail -8 "$TMP/prep2.txt")"
  grep -q "por la pasarela" "$TMP/prep2.txt" && dice "4 · preparar leyó por la pasarela (sin conector en el PATH)" || falla "4 · no dice pasarela: $(tail -5 "$TMP/prep2.txt")"
  F2=$("$PY" -c 'import pyarrow.ipc as i,sys;print(i.open_stream(sys.argv[1]).read_all().num_rows)' "$D2/informes.ventas_es_copia/entradas/pg.public.pedidos.arrow" 2>&1)
  [ "$F2" = "$FILAS" ] && dice "4 · las mismas $F2 filas que el conector directo" || falla "4 · $F2 filas por la pasarela, $FILAS directas"
  curl -s "http://127.0.0.1:$PF/v1/origins" | grep -q '"copias":2' && dice "4 · la pasarela las cuenta como copias (2)" || falla "4 · origins: $(curl -s "http://127.0.0.1:$PF/v1/origins")"
  # ── 5 · y la copia de siempre (un Dataset de una Table), por la pasarela ──
  #   La lee `ore materialize` y la sella el almacén (aquí, el S3 de mentira).
  STORE="$(dirname "$ORE")/ore-store-r2"
  if [ -x "$STORE" ]; then
    "$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
    trap 'kill $FPID $S3_PID 2>/dev/null; rm -rf "$TMP"' EXIT
    for _ in $(seq 1 40); do [ -s "$TMP/s3.log" ] && break; sleep 0.25; done
    S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
    mkdir -p "$TMP/solo-almacen"; ln -s "$STORE" "$TMP/solo-almacen/ore-store-r2"
    ( cd "$A" && PATH="$TMP/solo-almacen:/usr/bin:/bin" ORE_PASARELA="127.0.0.1:$PF" ORE_STORE=r2         ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira         "$ORE" materialize . --vista informes.clientes_copia --informe "$TMP/informe.json" ) >"$TMP/mat.txt" 2>&1       || falla "5 · materialize por la pasarela: $(tail -8 "$TMP/mat.txt")"
    grep -q "7 filas · 7 leidas" "$TMP/mat.txt" && dice "5 · la copia de siempre, por la pasarela: 7 filas selladas (sin conector en el PATH)"       || falla "5 · materialize: $(tail -5 "$TMP/mat.txt")"
    curl -s "http://127.0.0.1:$PF/v1/origins" | grep -q '"copias":3' && dice "5 · y la pasarela la cuenta (3 copias)" || falla "5 · origins: $(curl -s "http://127.0.0.1:$PF/v1/origins")"
  else
    echo "  · sin \`ore-store-r2\` junto a \`ore\`: la copia de siempre no corre"
  fi
else
  echo "  · sin \`ore-federation\` junto a \`ore\`: la parte de la pasarela no corre"
fi

echo
[ "$MAL" = 0 ] && echo "✓ la copia desde el origen (0053 F7·2 y F8·3)" || { echo "✗ la copia desde el origen"; exit 1; }
