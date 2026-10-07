#!/usr/bin/env bash
# 0057 B4·0/B4·1 · LA FORANEA SE LEE — la matriz de lo que se puede preguntar a
# una foreign database (OOS v1alpha27), camino a camino. Cada celda es
# un camino × una clase de consulta (T1–T7 sobre tablas y vistas, M1–M4 sobre la
# coleccion virtual, G1–G6 de un `.sql` que crea o escribe), con lo que contesta,
# y falla si no es lo esperado (0057 B4·1: Python, celdas y `.sql`).
# Con `javac` >= 21, la columna Java (0057 B4·2·1): celdas de un puesto JVM con
# su agente de verdad, por el mismo contrato; con `node` >= 22.13, la columna TS
# (B4·2·2), con un puesto Node. Y Build (B4·6): transforms que leen la foreign
# database, construidos por `POST /builds` y el agente del trabajo.
#
# El origen es el S3 de mentira: dos tablas parquet (`datos/clientes`,
# `datos/pedidos`) y una carpeta de PDFs (`docs/contratos`). La base foranea
# `vivo` expone `datos` y `docs` en espejo; `congelada` es la misma sobre una
# fuente con la lectura en vivo apagada. `ore-serve` decide, `ore-federation`
# lee y el SDK de Python pregunta desde un puesto de mentira.
#
#   ORE_BIN=target/debug bash pruebas-de-fuego/la-foranea-se-lee.sh
#
# Necesita python con pyarrow, duckdb y pyyaml (el SDK del puesto).
set -u
BIN="${ORE_BIN:-target/debug}"
abs() { echo "$(cd "$1" && pwd)"; }
BIN=$(abs "$BIN")
PY=$(command -v python3 || command -v python)
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
PIDS=""
trap 'for p in $PIDS; do kill $p 2>/dev/null; done; rm -rf "$TMP"' EXIT
libre() { "$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'; }

# ── el origen: el S3 de mentira, con dos tablas parquet y unos PDFs ─────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & PIDS="$PIDS $!"
for _ in $(seq 1 40); do [ -s "$TMP/s3.log" ] && break; sleep 0.25; done
S3="http://127.0.0.1:$(awk '{print $2}' "$TMP/s3.log")"
curl -s -X PUT "$S3/lago" >/dev/null
"$PY" - "$TMP" <<'PYX'
import sys, datetime, pyarrow as pa, pyarrow.parquet as pq
t = sys.argv[1]
pq.write_table(pa.table({
    "id": pa.array([1, 2, 3, 4, 5], pa.int64()),
    "pais": ["ES", "PT", "ES", "FR", "ES"],
    "alta": pa.array([datetime.date(2026, 1, d) for d in (3, 5, 9, 12, 20)]),
}), f"{t}/clientes.parquet")
pq.write_table(pa.table({
    "id": pa.array(range(1, 9), pa.int64()),
    "cliente": pa.array([1, 1, 2, 3, 3, 3, 4, 5], pa.int64()),
    "importe": pa.array([10.0, 20.0, 5.0, 7.5, 2.5, 30.0, 12.0, 8.0]),
}), f"{t}/pedidos.parquet")
# 0057 B4·6: una tabla que no cabe en el tope de una lectura en vivo (100 000).
pq.write_table(pa.table({"id": pa.array(range(100_001), pa.int64())}), f"{t}/grande.parquet")
PYX
curl -s -X PUT --data-binary @"$TMP/clientes.parquet" "$S3/lago/datos/clientes/parte-0.parquet" >/dev/null
curl -s -X PUT --data-binary @"$TMP/pedidos.parquet" "$S3/lago/datos/pedidos/parte-0.parquet" >/dev/null
curl -s -X PUT --data-binary @"$TMP/grande.parquet" "$S3/lago/datos/grande/parte-0.parquet" >/dev/null
for k in "anio=2026/a.pdf:aaaa" "anio=2026/b.pdf:bbbbbb" "anio=2025/c.pdf:cc"; do
  curl -s -X PUT --data-binary "${k#*:}" "$S3/lago/docs/contratos/${k%%:*}" >/dev/null
done
export FED_S3_URL="s3://lago?region=us-east-1&endpoint=$S3&access_key_id=de&secret_access_key=mentira"
export FED_S3_APAGADA_URL="$FED_S3_URL"

# ── el arbol: la fuente (sus punteros), la foranea y la congelada ───────────
FORJA="$TMP/forja.git"; A="$TMP/arbol"
git init -q --bare -b main "$FORJA"; git init -q -b main "$A"
cat > "$A/ontology.config.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha27
kind: OntologyConfig
metadata: { name: fed, version: 0.1.0 }
datasources:
  - name: s3
    type: s3
    connectionEnv: FED_S3_URL
    federation: true
  - name: s3_apagada
    type: s3
    connectionEnv: FED_S3_APAGADA_URL
YAML
cat > "$A/conduits.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: fed }
spec:
  owner: team:fed
  conduits:
    contextSurface.workspace: { oos.maturity: DRAFT }
    federation.read: { oos.maturity: DRAFT }
    materialization.payload: { oos.maturity: DRAFT }
YAML
fuente() { # nombre-del-paquete datasource
  local P=$1 D=$2
  mkdir -p "$A/packages/$P/datos/tables" "$A/packages/$P/docs/objects"
  cat > "$A/packages/$P/package.yaml" <<YAML
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: $P, version: 0.1.0, status: draft, domain: $P }
spec: { owner: "team:fed", exports: [$P.datos.clientes, $P.datos.pedidos, $P.datos.grande, $P.docs.contratos] }
YAML
  for s in datos docs; do
    printf 'apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: %s, namespace: %s }\nspec: { owner: team:fed }\n' "$s" "$P" > "$A/packages/$P/$s/schema.yaml"
  done
  cat > "$A/packages/$P/datos/tables/clientes.yaml" <<YAML
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: clientes, namespace: $P, schema: datos }
spec:
  datasource: $D
  object: "datos/clientes/"
  format: { type: parquet, match: "*.parquet" }
  columns:
    id: { type: Integer, physicalType: int64, required: true }
    pais: { type: String, physicalType: string }
    alta: { type: Date, physicalType: date32 }
  reads:
    fullScan: cheap
    predicatePushdown: [eq, neq, in, range, isNull]
  changes: { mode: retract, witness: listing }
YAML
  cat > "$A/packages/$P/datos/tables/pedidos.yaml" <<YAML
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: pedidos, namespace: $P, schema: datos }
spec:
  datasource: $D
  object: "datos/pedidos/"
  format: { type: parquet, match: "*.parquet" }
  columns:
    id: { type: Integer, physicalType: int64, required: true }
    cliente: { type: Integer, physicalType: int64 }
    importe: { type: Float, physicalType: double }
  reads:
    fullScan: cheap
    predicatePushdown: [eq, neq, in, range, isNull]
  changes: { mode: retract, witness: listing }
YAML
  cat > "$A/packages/$P/datos/tables/grande.yaml" <<YAML
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: grande, namespace: $P, schema: datos }
spec:
  datasource: $D
  object: "datos/grande/"
  format: { type: parquet, match: "*.parquet" }
  columns:
    id: { type: Integer, physicalType: int64, required: true }
  reads:
    fullScan: cheap
    predicatePushdown: [eq, neq, in, range, isNull]
  changes: { mode: retract, witness: listing }
YAML
  cat > "$A/packages/$P/docs/objects/contratos.yaml" <<YAML
apiVersion: oos.dev/v1alpha16
kind: ObjectTable
metadata: { name: contratos, namespace: $P, schema: docs }
spec:
  datasource: $D
  prefix: "docs/contratos/"
  match: "**/*.pdf"
  partitions: [anio]
  media: document
  reads: { fullScan: cheap }
  changes: { mode: retract, witness: listing }
YAML
}
fuente s3 s3
fuente s3_apagada s3_apagada
foranea() { # nombre datasource
  mkdir -p "$A/packages/$1"
  cat > "$A/packages/$1/package.yaml" <<YAML
apiVersion: oos.dev/v1alpha27
kind: Package
metadata: { name: $1, version: 0.1.0, status: draft, domain: $1 }
spec:
  owner: "team:fed"
  foreign: { datasource: $2, include: [datos, docs] }
YAML
}
foranea vivo s3
# una standard database donde copiar (T8)
mkdir -p "$A/packages/std/copias"
cat > "$A/packages/std/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: std, version: 0.1.0, status: draft, domain: std }
spec: { owner: "team:fed" }
YAML
printf 'apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: copias, namespace: std }
spec: { owner: team:fed }
' > "$A/packages/std/copias/schema.yaml"
foranea congelada s3_apagada
# dos vistas del usuario en la foranea: una que se empuja entera y una con junta
mkdir -p "$A/packages/vivo/informes/views"
printf 'apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: informes, namespace: vivo }\nspec: { owner: team:fed }\n' > "$A/packages/vivo/informes/schema.yaml"
cat > "$A/packages/vivo/informes/views/clientes_es.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: clientes_es, namespace: vivo, schema: informes }
spec:
  owner: team:fed
  dialect: duckdb
  sql: |
    SELECT id, alta FROM vivo.datos.clientes WHERE pais = 'ES'
  columns:
    id: { type: Integer }
    alta: { type: Date }
YAML
cat > "$A/packages/vivo/informes/views/ventas_por_pais.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: ventas_por_pais, namespace: vivo, schema: informes }
spec:
  owner: team:fed
  dialect: duckdb
  sql: |
    SELECT c.pais, count(*) AS pedidos, sum(p.importe) AS total
    FROM vivo.datos.pedidos p JOIN vivo.datos.clientes c ON p.cliente = c.id
    GROUP BY c.pais
  columns:
    pais: { type: String }
    pedidos: { type: Integer }
    total: { type: Float }
YAML
# ── 0057 B4·6 · un repositorio de transforms que lee la foreign database ────
mkdir -p "$A/packages/std/etl/transforms"
printf -- '---\nnombre: ETL\nplantilla: transforms-python\nplantillaVersion: 1\n---\nLo que se copia del origen.\n' > "$A/packages/std/etl/README.md"
cat > "$A/packages/std/etl/transforms/copias.py" <<'PY'
from ore import transform, sql, over, write


@transform(inputs=["vivo.datos.clientes"], output="std.copias.b_clientes")
def b_clientes():
    """sql() sobre una foreign table, por su nombre en la foreign database."""
    return write("std.copias.b_clientes", sql("select id, pais from vivo.datos.clientes"))


@transform(inputs=["s3.datos.clientes"], output="std.copias.b_por_fuente")
def b_por_fuente():
    """Por el nombre de la fuente."""
    return write("std.copias.b_por_fuente", sql("select pais, count(*) n from s3.datos.clientes group by pais"))


@transform(inputs=["vivo.informes.ventas_por_pais"], output="std.copias.b_ventas")
def b_ventas():
    """Una vista viva con junta: la declarada cubre lo que lee."""
    return write("std.copias.b_ventas", over("vivo.informes.ventas_por_pais"))


@transform(inputs=["vivo.datos.pedidos"], output="std.copias.b_over")
def b_over():
    """over() de una tabla expuesta."""
    return write("std.copias.b_over", over("vivo.datos.pedidos"))


@transform(inputs=["vivo.datos.grande"], output="std.copias.b_grande")
def b_grande():
    """Más filas que el tope de una lectura en vivo: un build las copia todas."""
    return write("std.copias.b_grande", sql("select id from vivo.datos.grande"))


@transform(inputs=["congelada.datos.clientes"], output="std.copias.b_congelada")
def b_congelada():
    """Un input de una foreign database congelada: el build falla con OOS2051."""
    return write("std.copias.b_congelada", sql("select id from congelada.datos.clientes"))
PY
cat > "$A/packages/std/etl/transforms/copias.sql" <<'SQL'
CREATE OR REPLACE DATASET std.copias.b_sql AS
SELECT pais, count(*) AS n FROM vivo.datos.clientes GROUP BY pais;
SQL
( cd "$A" && "$BIN/ore" transforms generate . > "$TMP/generate.txt" 2>&1 ) || { echo "  ✗ ore transforms generate:"; cat "$TMP/generate.txt"; }
( cd "$A" && "$BIN/ore" validate . > "$TMP/validate.txt" 2>&1 ) && echo "  · el arbol compila" || { echo "  ✗ el arbol no compila:"; head -20 "$TMP/validate.txt"; }
( cd "$A" && git add -A && git -c user.email=t@t -c user.name=t commit -qm semilla && git remote add origin "$FORJA" && git push -q origin HEAD:main ) \
  || { echo "no se sembró la forja"; exit 1; }

# ── la cola de los puestos, la pasarela y el servidor ───────────────────────
COLA="$TMP/cola.git"; git init -q --bare -b main "$COLA"; mkdir -p "$TMP/cola-semilla"
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TMP/rendido" >/dev/null 2>&1 || { echo "no se rindió la plantilla del puesto"; exit 1; }
cp "$TMP/rendido/plantilla-puesto.txt" "$TMP/rendido/plantilla-capa.txt" \
   "$TMP/rendido/plantilla-capa-jvm.txt" "$TMP/rendido/plantilla-capa-node.txt" "$TMP/cola-semilla/"
( cd "$TMP/cola-semilla" && git init -q -b main && git add -A && git -c user.name=t -c user.email=t@t commit -qm plantilla \
  && git remote add origin "$COLA" && git push -q origin HEAD:main ) || { echo "no se sembró la cola"; exit 1; }
PF=$(libre)
"$BIN/ore-federation" --escucha "127.0.0.1:$PF" --conectores "$BIN" --tipos s3 >"$TMP/fed.log" 2>&1 & PIDS="$PIDS $!"
# 0057 B4·3·2: ore-motor, el motor SQL de la celda (la preview de una vista sin copia).
PM=$(libre)
ORE_MOTOR_ESCUCHA="127.0.0.1:$PM" PYTHONUTF8=1 "$PY" "$RAIZ/puesto/python/ore_motor.py" >"$TMP/motor.log" 2>&1 & PIDS="$PIDS $!"
PS=$(libre); BASE="http://127.0.0.1:$PS"
# El lago (`ore-store`, que pasa el Arrow de la preview a filas), el mismo S3.
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="$S3" ORE_R2_BUCKET=lago ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira
export PATH="$BIN:$PATH"
ORE_MOTOR="127.0.0.1:$PM" ORE_PASARELA="127.0.0.1:$PF" FORJA_TOKEN=no-hace-falta "$BIN/ore-serve" --forja "file://$FORJA" --ore "$BIN/ore" --cola "file://$COLA" \
  --bind "127.0.0.1:$PS" --identidad cabecera --no-es-produccion --organizacion fed >"$TMP/serve.log" 2>&1 & PIDS="$PIDS $!"
for _ in $(seq 1 80); do curl -s -o /dev/null "$BASE/salud" && curl -s -o /dev/null "http://127.0.0.1:$PF/v1/health" && break; sleep 0.25; done
curl -s -o /dev/null -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' "$BASE/puestos" -d '{}'
P=puesto-ana-python
# El agente del puesto, de verdad: corre las celdas del editor (un `.sql`).
ORE_SERVE="$BASE" PUESTO="$P" ORE_SUJETO=agente:local TTL=600 PUESTO_DIR="$TMP" TRABAJO_DIR="$TMP" PYTHONUTF8=1   "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/agente.log" 2>&1 & PIDS="$PIDS $!"
for _ in $(seq 1 20); do curl -s -H 'x-ore-sujeto: persona:ana' "$BASE/puestos/$P" | grep -q '"estado":"vivo"' && break; sleep 0.2; done

# ── 0057 B4·2 · un puesto JVM con su agente, si hay `javac` >= 21 ─────────────
PJ=""
JAVAC=$(command -v javac || true); JAVA=$(command -v java || true)
if [ -n "$JAVAC" ] && "$JAVAC" -version 2>&1 | grep -qE '^javac (2[1-9]|[3-9][0-9])'; then
  LIB="${ORE_JARS:-${TMPDIR:-/tmp}/ore-jars}"; mkdir -p "$LIB"
  [ -f "$LIB/duckdb_jdbc.jar" ] || curl -sfL --retry 3 --retry-all-errors -o "$LIB/duckdb_jdbc.jar" "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.5.5.1/duckdb_jdbc-1.5.5.1.jar"
  grep -v '^#' "$RAIZ/puesto/jvm/jars.txt" | while read -r g v; do n="${g##*/}-$v.jar"; [ -f "$LIB/$n" ] || curl -sfL --retry 3 --retry-all-errors -o "$LIB/$n" "https://repo1.maven.org/maven2/$g/$v/$n"; done
  SEP=":"; LIB_CP="$LIB"; CLASES="$TMP/clases"; mkdir -p "$CLASES"; CLASES_CP="$CLASES"
  case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) SEP=";"; LIB_CP="$(cd "$LIB" && pwd -W)"; CLASES_CP="$(cd "$CLASES" && pwd -W)";; esac
  if "$JAVAC" -Xlint:-options --release 21 -cp "$LIB_CP/*" -d "$CLASES_CP" "$RAIZ"/puesto/jvm/ore/*.java 2>"$TMP/javac.txt"; then
    curl -s -o /dev/null -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' "$BASE/puestos" -d '{"lenguaje":"java"}'
    PJ=puesto-ana-jvm
    ORE_SERVE="$BASE" PUESTO="$PJ" ORE_SUJETO=agente:local ORE_ALMACEN="dir:$TMP" TTL=600 ORE_MEMORIA_MB=1024 \
      "$JAVA" --add-opens=java.base/java.nio=ALL-UNNAMED -cp "$CLASES_CP$SEP$LIB_CP/*" ore.Agente >"$TMP/agente-jvm.log" 2>&1 & PIDS="$PIDS $!"
    for _ in $(seq 1 120); do curl -s -H 'x-ore-sujeto: persona:ana' "$BASE/puestos/$PJ" | grep -q '"estado":"vivo"' && break; sleep 0.25; done
  else
    echo "  ✗ el SDK de Java no compila:"; head -20 "$TMP/javac.txt"; exit 1
  fi
else
  echo "  · sin javac >= 21: la columna Java no se mide"
fi

# ── 0057 B4·2·2 · un puesto Node con su agente, si hay `node` >= 22.13 ────────
PN=""
NODE=$(command -v node || true)
if [ -n "$NODE" ] && [ "$("$NODE" -e 'const [a,b]=process.versions.node.split(".").map(Number); process.stdout.write(a>22||(a==22&&b>=13)?"si":"no")' 2>/dev/null)" = "si" ]; then
  # El SDK resuelve `@duckdb/node-api` desde donde esta: se copia `puesto/node` y se instala ahi.
  mkdir -p "$TMP/node" && cp -r "$RAIZ/puesto/node/." "$TMP/node/"
  ( cd "$TMP/node" && npm install --no-audit --no-fund --silent "$(grep -m1 '^@duckdb/node-api@' "$RAIZ/puesto/node/provisto.txt")" >"$TMP/npm.txt" 2>&1 ) \
    || { echo "  ✗ npm install @duckdb/node-api:"; tail -5 "$TMP/npm.txt"; exit 1; }
  curl -s -o /dev/null -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' "$BASE/puestos" -d '{"lenguaje":"typescript"}'
  PN=puesto-ana-node
  ORE_SERVE="$BASE" PUESTO="$PN" ORE_SUJETO=agente:local ORE_ALMACEN="dir:$TMP" ORE_CELDAS="$TMP/node-trabajo" ORE_COPIAS="$TMP/node-copias" TTL=600 \
    "$NODE" --no-warnings "$TMP/node/agente.mjs" >"$TMP/agente-node.log" 2>&1 & PIDS="$PIDS $!"
  for _ in $(seq 1 60); do curl -s -H 'x-ore-sujeto: persona:ana' "$BASE/puestos/$PN" | grep -q '"estado":"vivo"' && break; sleep 0.25; done
else
  echo "  · sin node >= 22.13: la columna TS no se mide"
fi

# ── la matriz ────────────────────────────────────────────────────────────────
ARBOL="$A" ORE_BIN_DIR="$BIN" TMP_DIR="$TMP" PUESTO_NODE="$PN" PUESTO_JVM="$PJ" ORE_SERVE="$BASE" PUESTO="$P" PYTHONUTF8=1 "$PY" "$RAIZ/pruebas-de-fuego/la-foranea-se-lee.py" "$BASE" "$P"
SALIDA=$?
echo
echo "  (registro del servidor: $(grep -c . "$TMP/serve.log") lineas; de la pasarela: $(grep -c . "$TMP/fed.log"))"
[ -n "${VERBOSO:-}" ] || [ "$SALIDA" != 0 ] && { tail -30 "$TMP/serve.log"; tail -30 "$TMP/fed.log"; tail -30 "$TMP/agente.log"; tail -30 "$TMP/motor.log"; [ -f "$TMP/agente-jvm.log" ] && tail -30 "$TMP/agente-jvm.log"; [ -f "$TMP/agente-node.log" ] && tail -30 "$TMP/agente-node.log"; }
exit "$SALIDA"
