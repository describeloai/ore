#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# MEDIDA · el guion SQL (varias sentencias, como Databricks) — antes de tocar
# nada, lo que hace HOY la iteración con el guion del ejemplo de la persona:
#
#   CREATE SCHEMA IF NOT EXISTS ventas.demo_uc;
#   CREATE TABLE IF NOT EXISTS ventas.demo_uc.clientes (id BIGINT, nombre STRING, …);
#   INSERT INTO ventas.demo_uc.clientes (id, …) VALUES (1, …), (2, …);
#   SELECT * FROM ventas.demo_uc.clientes;
#
#   G0  DuckDB solo (sin ORE): el guion entero, sentencia a sentencia
#   G1  hoy, el guion entero como UNA celda `sql` del puesto
#   G2  hoy, cada sentencia en su celda `sql`
#   G3  /v1 createNamespace (lo que Spark y DuckDB llaman para CREATE SCHEMA)
#   G4  /v1 createTable con columnas, desde una celda (lo que sería CREATE TABLE),
#       y un `write(anexar)` de un SELECT sobre VALUES (lo que sería el INSERT)
#   G5  `ore package new` (lo que sería CREATE CATALOG) y cómo lo pinta el índice
#   G6  el resultado de una escritura, hoy
#   G7  (0039 paso 2) cada sentencia del guion, en su celda, sobre su verbo
#   G8  (0039 paso 4) lo que deja cada sentencia de un guion, lo que write() cuenta, y cómo lo pinta la consola
#
# Mide y dice; no falla. Necesita `ore`, `ore-serve`, `ore-store-r2`
# (cargo build --release --workspace) y python con pyarrow, pandas y duckdb.
# ══════════════════════════════════════════════════════════════════════════════
set -u
export PYTHONIOENCODING=utf-8
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8933}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""; AGENTE=""; S3_PID=""
limpiar() {
  [ -n "$AGENTE" ] && kill "$AGENTE" 2>/dev/null
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  [ -n "$S3_PID" ] && kill "$S3_PID" 2>/dev/null
  rm -rf "$TMP"
}
trap limpiar EXIT
para() { echo "⛔ $*" >&2; [ -s "$TMP/serve.txt" ] && tail -15 "$TMP/serve.txt" >&2; [ -s "$TMP/agente.txt" ] && tail -15 "$TMP/agente.txt" >&2; exit 2; }
mide() { printf '  \xc2\xb7 %s\n' "$*"; }
buscar() {
  for c in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" "$RAIZ/target/debug/$1" "$RAIZ/target/debug/$1.exe"; do
    [ -x "$c" ] && { echo "$c"; return 0; }
  done
  return 1
}
PY=$(command -v python3 || command -v python) || para "hace falta python"
SERVE="$(buscar ore-serve)" || para "no hay ore-serve"
ORE="$(buscar ore)" || para "no hay ore"
pide() { # <metodo> <ruta> [cuerpo] → código; cuerpo en r.json
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' ${3:+--data-binary "$3"} "$BASE$2"
}
# Lo que dijo una celda, en una línea.
resumen() {
  "$PY" - "$TMP/r.json" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
s = d.get("salida") or {}
t = s.get("tipo")
if t == "error":
    print("ERROR %s: %s" % (s.get("nombre", ""), " ".join(str(s.get("mensaje", "")).split())[:260]))
elif t == "tabla":
    print("tabla %s filas: %s" % (s.get("total"), json.dumps(s.get("filas"))[:160]))
elif t == "texto":
    print("texto: " + " ".join(s.get("texto", "").split())[:260])
else:
    print("%s %s" % (d.get("estado"), json.dumps(s)[:200]))
EOF
}
celda() { # <lenguaje> <texto> → resumen
  local cuerpo n
  cuerpo=$("$PY" -c 'import json,sys; print(json.dumps({"texto": sys.argv[2], "lenguaje": sys.argv[1]}))' "$1" "$2")
  [ "$(pide POST /puestos/puesto-ana-python/ejecutar "$cuerpo")" = "202" ] || { echo "no se mandó: $(cat "$TMP/r.json")"; return; }
  n=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["celda"])' "$TMP/r.json")
  for _ in $(seq 1 6); do
    pide GET "/puestos/puesto-ana-python/celdas/$n" >/dev/null
    "$PY" -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1])).get("estado")=="hecha" else 1)' "$TMP/r.json" && break
  done
  resumen
}

# ── el árbol: la base `ventas`, con el schema `espana` declarado ─────────────
A="$TMP/arbol"
mkdir -p "$A/packages/ventas/datasets" "$A/packages/ventas/espana/datasets" "$A/datasets"
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
Y
cat > "$A/packages/ventas/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 1.0.0, status: active, domain: sales }
spec: { owner: team:ventas }
Y
cat > "$A/packages/ventas/espana/schema.yaml" <<'Y'
apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: espana, namespace: ventas, description: Lo que se vende en España. }
spec: { owner: team:ventas }
Y
# un Dataset declarado en `espana`, aún sin escribir
cat > "$A/packages/ventas/espana/datasets/clientes.yaml" <<'Y'
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata: { name: clientes, namespace: ventas, schema: espana }
spec:
  owner: team:ventas
  columns: { id: { type: Integer }, pais: { type: String } }
  changes: { mode: append }
Y
( cd "$A" && git init -q -b main && git config core.autocrlf false && git add -A && git -c core.autocrlf=false -c user.name=m -c user.email=m@x commit -qm semilla )

echo "E0 · el árbol con un schema declarado"
mide "ore validate: $(cd "$A" && "$ORE" validate . 2>&1 | tail -4 | tr '
' ' ')"

# ── la cola y el lago ────────────────────────────────────────────────────────
COLA="$TMP/cola.git"; git init -q --bare -b main "$COLA"
mkdir -p "$TMP/semilla" && ( cd "$TMP/semilla" && git init -q -b main && git config core.autocrlf false )
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TMP/rendido" >/dev/null 2>&1 || para "gen-inquilino"
cp "$TMP/rendido/plantilla-puesto.txt" "$TMP/rendido/plantilla-capa.txt" "$TMP/rendido/plantilla-capa-jvm.txt" "$TMP/semilla/"
( cd "$TMP/semilla" && git add -A && git -c user.name=b -c user.email=b@x commit -qm p && git remote add origin "$COLA" && git push -q origin HEAD:main ) || para "cola"
COLA_URL="file://$(cd "$COLA" && pwd)"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) COLA_URL="file:///$(cd "$COLA" && pwd -W)";; esac
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log"); [ -n "$S3_PUERTO" ] || para "S3"
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia" ORE_RETENCION=7d
export ORE_STORE_DIR="$(dirname "$ORE")"
export PATH="$ORE_STORE_DIR:$PATH"

FORJA_TOKEN=no-hace-falta "$SERVE" --repo "$A" --ore "$ORE" --bind "127.0.0.1:$PUERTO" --cola "$COLA_URL" \
  --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/serve.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
[ "$(pide POST /puestos '{}')" = "201" ] || para "abrir el puesto: $(cat "$TMP/r.json")"
TMP_PY="$TMP"; ALM="$TMP/almacen"; mkdir -p "$ALM"; ALM_PY="$ALM"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) TMP_PY="$(cd "$TMP" && pwd -W)"; ALM_PY="$(cd "$ALM" && pwd -W)";; esac
ORE_SERVE="$BASE" PUESTO=puesto-ana-python ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALM_PY" TTL=600 ORE_MEMORIA_MB=1024 \
  TRABAJO_DIR="$TMP_PY" "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/agente.txt" 2>&1 &
AGENTE=$!
for _ in $(seq 1 40); do pide GET /puestos/puesto-ana-python >/dev/null; grep -q '"vivo"' "$TMP/r.json" && break; sleep 0.25; done
grep -q '"vivo"' "$TMP/r.json" || para "el puesto no pasa a vivo"

GUION="$(cat <<'SQL'
CREATE SCHEMA IF NOT EXISTS ventas.demo_uc;

CREATE TABLE IF NOT EXISTS ventas.demo_uc.clientes (
  id BIGINT,
  nombre STRING,
  email STRING,
  created_at TIMESTAMP
);

INSERT INTO ventas.demo_uc.clientes (id, nombre, email, created_at) VALUES
  (1, 'Ana López', 'ana@example.com', current_timestamp()),
  (2, 'Bruno Díaz', 'bruno@example.com', current_timestamp());

SELECT * FROM ventas.demo_uc.clientes;
SQL
)"

echo "G0 · DuckDB solo"
"$PY" - "$GUION" <<'PYG0'
import sys, duckdb
guion = sys.argv[1]
for dialecto, texto in (("tal cual", guion), ("current_timestamp sin ()", guion.replace("current_timestamp()", "current_timestamp"))):
    con = duckdb.connect()
    con.execute("ATTACH ':memory:' AS ventas")
    print("  · %s:" % dialecto)
    for s in [s.strip() for s in texto.split(";") if s.strip()]:
        try:
            r = con.execute(s)
            try:
                filas = r.fetchall(); cols = [d[0] for d in r.description] if r.description else []
            except Exception:
                filas, cols = None, []
            print("      %-40s → %s %s" % (" ".join(s.split())[:40], cols, str(filas)[:120]))
        except Exception as e:
            print("      %-40s → ERROR %s" % (" ".join(s.split())[:40], " ".join(str(e).split())[:150]))
PYG0

echo "G1 · el guion entero, una celda sql"
mide "$(celda sql "$GUION")"

echo "G2 · cada sentencia en su celda"
"$PY" -c 'import sys; open(sys.argv[2], "w", encoding="utf-8").write("\n".join(" ".join(s.split()) for s in sys.argv[1].split(";") if s.strip()))' "$GUION" "$TMP/sent.txt"
while IFS= read -r s; do mide "$(echo "$s" | cut -c1-50)… → $(celda sql "$s")"; done < "$TMP/sent.txt"

echo "G3 · /v1 createNamespace"
c=$(pide POST /v1/ventas/namespaces '{"namespace":["demo_uc"],"properties":{}}'); mide "POST /v1/ventas/namespaces → $c $(head -c 200 "$TMP/r.json")"
c=$(pide POST /v1/namespaces '{"namespace":["ventas","demo_uc"],"properties":{}}'); mide "POST /v1/namespaces → $c $(head -c 200 "$TMP/r.json")"
c=$(pide GET /v1/ventas/namespaces); mide "GET /v1/ventas/namespaces → $c $(head -c 200 "$TMP/r.json")"

echo "G4 · createTable con columnas, desde una celda; y el INSERT como SELECT sobre VALUES"
mide "createTable ventas.espana.clientes2 → $(celda python 'import json
c, r = ore.puesto.pedir("POST", "/v1/ventas/namespaces/espana/tables", {"name": "clientes2", "schema": {"type": "struct", "schema-id": 0, "fields": [
  {"id": 1, "name": "id", "type": "long", "required": False}, {"id": 2, "name": "nombre", "type": "string", "required": False},
  {"id": 3, "name": "email", "type": "string", "required": False}, {"id": 4, "name": "created_at", "type": "timestamptz", "required": False}]}})
print(c, json.dumps(r)[:300])')"
mide "createTable en default (ventas.clientes3) → $(celda python 'c, r = ore.puesto.pedir("POST", "/v1/ventas/namespaces/default/tables", {"name": "clientes3", "schema": {"type": "struct", "schema-id": 0, "fields": [{"id": 1, "name": "id", "type": "long", "required": False}]}}); print(c, str(r)[:200])')"
mide "createTable otra vez (ya existe) → $(celda python 'c, r = ore.puesto.pedir("POST", "/v1/ventas/namespaces/default/tables", {"name": "clientes3", "schema": {"type": "struct", "schema-id": 0, "fields": [{"id": 1, "name": "id", "type": "long", "required": False}]}}); print(c, str(r)[:200])')"
mide "lo que queda: $(cd "$A" && git status --short 2>/dev/null | tr '\n' ' ')"
mide "GET clientes2 (esquema) → $(celda python 'c, r = ore.puesto.pedir("GET", "/v1/ventas/namespaces/espana/tables/clientes2"); md = r.get("metadata", {}) if c == 200 else {}; print(c, [(f["name"], f["type"]) for s in md.get("schemas", []) for f in s["fields"]] if c == 200 else str(r)[:200], md.get("current-snapshot-id"))')"
mide "over() de la tabla vacía → $(celda python 'over("ventas.espana.clientes2")')"
mide "sql() de la tabla vacía → $(celda sql 'select count(*) as n from ventas.espana.clientes2')"
mide "INSERT … VALUES como SELECT, anexar → $(celda python "write(\"ventas.espana.clientes2\", sql(\"SELECT * FROM (VALUES (1, 'Ana López', 'ana@example.com', current_timestamp), (2, 'Bruno Díaz', 'bruno@example.com', current_timestamp)) AS v(id, nombre, email, created_at)\", como=\"arrow\"), modo=\"anexar\")")"
mide "leído → $(celda sql 'select * from ventas.espana.clientes2')"
mide "tipos → $(celda python 'c, r = ore.puesto.pedir("GET", "/v1/ventas/namespaces/espana/tables/clientes2"); md = r["metadata"]; print([(f["name"], f["type"]) for s in md["schemas"] if s["schema-id"] == md["current-schema-id"] for f in s["fields"]])')"
echo "  ── el servidor ──"; grep -iE -B2 -A6 "ore-store" "$TMP/serve.txt" | tail -30
mide "con el mensaje entero → $(celda python 'import json; ore._mensaje = lambda r: json.dumps(r, ensure_ascii=False)[:1500]; write("ventas.espana.clientes2", sql("SELECT 1::BIGINT AS id, NULL::VARCHAR AS nombre, NULL::VARCHAR AS email, now() AS created_at", como="arrow"), modo="anexar")')"
mide "INSERT con columnas de menos (sin email) → $(celda python "write(\"ventas.espana.clientes2\", sql(\"SELECT * FROM (VALUES (3, 'Carla', current_timestamp)) AS v(id, nombre, created_at)\", como=\"arrow\"), modo=\"anexar\")")"
mide "leído → $(celda sql 'select * from ventas.espana.clientes2 order by id')"
mide "lo que queda: $(cd "$A" && git status --short 2>/dev/null | tr '\n' ' '; git log --oneline | head -5 | tr '\n' ' ')"

echo "G5 · ore package new (CREATE CATALOG)"
mide "$(cd "$A" && "$ORE" package new otra_base --owner team:ventas 2>&1 | head -2 | tr '\n' ' ')"
mide "ore validate: $(cd "$A" && "$ORE" validate . 2>&1 | tail -4 | tr '
' ' ')"
"$ORE" assets "$A" --json > "$TMP/idx.json" 2>/dev/null
mide "índice: $("$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); ps=d.get("paquetes") or d.get("packages") or {}; ps = ps.values() if isinstance(ps, dict) else ps; print([(p.get("name") or p.get("nombre"), p.get("type") or p.get("clase"), p.get("source") or p.get("fuente")) for p in ps])' "$TMP/idx.json" 2>&1 | cut -c1-300)"
mide "claves del índice: $("$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); print(list(d.keys()))' "$TMP/idx.json")"

echo "G6 · el resultado de una escritura, hoy"
mide "$(celda sql 'create or replace table ventas.resumen as select 1 as n')"

echo "G7 · (0039 paso 2) cada sentencia, en su celda de la sesión, sobre su verbo"
mide "create schema ventas.demo_uc                 → $(celda sql 'CREATE SCHEMA IF NOT EXISTS ventas.demo_uc')"
mide "otra vez, if not exists                       → $(celda sql 'CREATE SCHEMA IF NOT EXISTS ventas.demo_uc')"
mide "otra vez, sin if not exists                   → $(celda sql 'CREATE SCHEMA ventas.demo_uc')"
mide "create dataset (cols)                         → $(celda sql 'CREATE DATASET IF NOT EXISTS ventas.demo_uc.clientes (id BIGINT, nombre STRING, email STRING, created_at TIMESTAMP)')"
mide "insert … values (current_timestamp)           → $(celda sql "INSERT INTO ventas.demo_uc.clientes (id, nombre, email, created_at) VALUES (1, 'Ana López', 'ana@example.com', current_timestamp), (2, 'Bruno Díaz', 'bruno@example.com', current_timestamp)")"
mide "insert … values (current_timestamp())         → $(celda sql "INSERT INTO ventas.demo_uc.clientes (id, nombre, email, created_at) VALUES (3, 'Carla', 'c@example.com', current_timestamp())")"
mide "insert con columnas de menos                  → $(celda sql "INSERT INTO ventas.demo_uc.clientes (id, nombre) VALUES (4, 'Dora')")"
mide "insert sin lista (por posición)               → $(celda sql "INSERT INTO ventas.demo_uc.clientes VALUES (5, 'Eva', 'e@example.com', TIMESTAMP '2026-01-01 10:00:00')")"
mide "select                                        → $(celda sql 'SELECT id, nombre, email, created_at IS NOT NULL AS con_fecha FROM ventas.demo_uc.clientes ORDER BY id')"
mide "tipos → $(celda python 'c, r = ore.puesto.pedir("GET", "/v1/ventas/namespaces/demo_uc/tables/clientes"); md = r["metadata"]; print([(f["name"], f["type"]) for s in md["schemas"] if s["schema-id"] == md["current-schema-id"] for f in s["fields"]])')"
mide "create table (se niega)                       → $(celda sql 'CREATE TABLE ventas.demo_uc.t (a INT)')"
mide "create standard database mi_base             → $(celda sql 'CREATE STANDARD DATABASE mi_base')"
mide "create database if not exists mi_base        → $(celda sql 'CREATE DATABASE IF NOT EXISTS mi_base')"
mide "create schema mi_base.s                       → $(celda sql 'CREATE SCHEMA mi_base.s')"
mide "create or replace dataset mi_base.s.r as …    → $(celda sql 'CREATE OR REPLACE DATASET mi_base.s.r AS SELECT id, nombre FROM ventas.demo_uc.clientes')"
mide "select de mi_base                              → $(celda sql 'SELECT count(*) AS n FROM mi_base.s.r')"
mide "create foreign database sin origen            → $(celda sql 'CREATE FOREIGN DATABASE espejo')"
mide "lo que queda en el árbol: $(cd "$A" && find packages/mi_base packages/ventas/demo_uc -type f | sort | tr '\n' ' ')"
"$ORE" assets "$A" --json > "$TMP/idx.json" 2>/dev/null
mide "índice: $("$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); ps=d.get("paquetes") or {}; ps = ps.values() if isinstance(ps, dict) else ps; print([(p.get("name"), p.get("type"), p.get("schemas")) for p in ps])' "$TMP/idx.json" 2>&1 | cut -c1-300)"
mide "ore validate: $(cd "$A" && "$ORE" validate . 2>&1 | tail -3 | tr '\n' ' ')"

echo "G8 · (0039 paso 4) lo que deja HOY cada sentencia de un guion"
cat > "$TMP/g8.sql" <<'SQL'
CREATE SCHEMA IF NOT EXISTS ventas.g8;
CREATE DATASET IF NOT EXISTS ventas.g8.clientes (id BIGINT, nombre STRING, created_at TIMESTAMP);
INSERT INTO ventas.g8.clientes (id, nombre, created_at) VALUES (1, 'Ana', current_timestamp), (2, 'Bruno', current_timestamp);
INSERT INTO ventas.g8.clientes (id, nombre) VALUES (3, 'Carla');
CREATE OR REPLACE DATASET ventas.g8.resumen AS SELECT id, nombre FROM ventas.g8.clientes;
INSERT OR REPLACE INTO ventas.g8.resumen SELECT * FROM (VALUES (1, 'Ana bis'), (9, 'Nueva')) AS v(id, nombre);
SELECT * FROM ventas.g8.resumen ORDER BY id
SQL
cuerpo=$("$PY" -c 'import json,sys; print(json.dumps({"texto": open(sys.argv[1], encoding="utf-8").read(), "lenguaje": "sql", "fichero": "g8.sql"}))' "$TMP/g8.sql")
pide POST /puestos/puesto-ana-python/ejecutar "$cuerpo" >/dev/null; cp "$TMP/r.json" "$TMP/g8.json"
for n in $("$PY" -c 'import json,sys; print(" ".join(str(c) for c in json.load(open(sys.argv[1]))["celdas"]))' "$TMP/g8.json"); do
  for _ in $(seq 1 6); do pide GET "/puestos/puesto-ana-python/celdas/$n" >/dev/null; "$PY" -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1])).get("estado")=="hecha" else 1)' "$TMP/r.json" && break; done
  "$PY" - "$TMP/g8.json" "$TMP/r.json" "$n" <<'PYG8'
import json, sys
g = json.load(open(sys.argv[1], encoding="utf-8")); c = json.load(open(sys.argv[2], encoding="utf-8")); n = int(sys.argv[3])
s = next(x for x in g["sentencias"] if x["celda"] == n)
sal = dict(c.get("salida") or {}); sal.pop("traza", None)
for k in ("filas",):
    if k in sal: sal[k] = sal[k][:3]
print("  · %-26s → %s" % (s["que"], json.dumps(sal, ensure_ascii=False)[:230]))
PYG8
done
echo "  ── lo que write() devuelve (anexar 1 fila a un dataset de 3; upsert de 2 con 1 que ya estaba) ──"
mide "$(celda python 'import pyarrow as pa; write("ventas.g8.clientes", pa.table({"id": pa.array([4], pa.int64()), "nombre": ["Dora"]}), modo="anexar")')"
mide "$(celda python 'import pyarrow as pa; write("ventas.g8.resumen", pa.table({"id": pa.array([2, 10], pa.int64()), "nombre": ["Bruno bis", "Diez"]}), modo="upsert")')"
mide "snapshot (summary) → $(celda python 'c, r = ore.puesto.pedir("GET", "/v1/ventas/namespaces/g8/tables/resumen"); md = r["metadata"]; s = [x for x in md["snapshots"] if x["snapshot-id"] == md["current-snapshot-id"]][0]; print({k: v for k, v in s["summary"].items() if "record" in k or k == "operation"})')"
echo "  ── la consola: cómo pinta cada tipo de salida (desdeElPuesto + ResultsPanel) ──"
C=/c/rubix-platform
grep -n "s.tipo === 'vacia'\|s.tipo === 'texto'\|tipo: 'tabla'" "$C/components/code-workspace/CodeWorkspaceClient.tsx" | sed 's/^/    /'
grep -n "case 'vacia'\|'vacia'\|No output\|no output\|tipo === 'texto'" "$C/components/results/ResultsPanel.tsx" | head -8 | sed 's/^/    /'
