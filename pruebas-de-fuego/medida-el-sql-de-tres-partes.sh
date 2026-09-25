#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# MEDIDA · el SQL de tres partes (0038 P3) — antes de tocar nada, lo que hace
# HOY la iteración de punta a punta cuando un nombre lleva su schema, con un
# puesto de verdad (el agente de Python), el S3 de mentira y ore-serve como
# catálogo, sobre un árbol que YA tiene un schema declarado (OOS v1alpha13,
# P1): `ventas.espana`.
#
#   E0  el árbol: `ore validate` con `kind: Schema` y un Dataset en `espana`
#   E1  write() a `p.n` (la de hoy), a `p.default.n` y a `p.espana.n`
#   E2  sql() y over() con dos partes, con `default` y con `espana`
#   E3  una celda `sql` que escribe: a `p.n`, a `p.default.n`, a `p.espana.n`
#   E4  lo que el árbol queda: punteros, Dataset (¿en la carpeta del schema, con
#       `metadata.schema`?), `ore validate`, el índice (`schema` de cada ítem)
#   E5  el catálogo /v1: config, namespaces, tablas de `ventas`, de `espana`
#   E6  el servidor de SQL (LSP): qué ofrece tras FROM y tras `ventas.`, y qué
#       dice de un nombre de tres partes
#
# Mide y dice; no falla. Necesita `ore`, `ore-serve`, `ore-store-r2`
# (cargo build --release --workspace) y python con pyarrow, pandas y duckdb.
# ══════════════════════════════════════════════════════════════════════════════
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8931}"
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
mide "ore validate: $(cd "$A" && "$ORE" validate . 2>&1 | tail -1)"

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

echo "E1 · write()"
celda python 'import pyarrow as pa; t = pa.table({"id": pa.array([1, 2, 3], pa.int64()), "pais": ["ES", "FR", "ES"]}); None' >/dev/null
mide "write(\"ventas.pedidos\", t)          → $(celda python 'write("ventas.pedidos", t)')"
mide "write(\"ventas.default.otros\", t)    → $(celda python 'write("ventas.default.otros", t)')"
mide "write(\"ventas.espana.pedidos\", t)   → $(celda python 'write("ventas.espana.pedidos", t)')"
mide "write(\"ventas.espana.clientes\", t)  → $(celda python 'write("ventas.espana.clientes", t)')  (declarado en espana)"

echo "E2 · sql() y over()"
mide "sql(... from ventas.pedidos)          → $(celda python 'sql("select count(*) as n from ventas.pedidos")')"
mide "sql(... from ventas.default.pedidos)  → $(celda python 'sql("select count(*) as n from ventas.default.pedidos")')"
mide "sql(... from ventas.espana.clientes)  → $(celda python 'sql("select count(*) as n from ventas.espana.clientes")')"
mide "over(\"ventas.default.pedidos\")        → $(celda python 'over("ventas.default.pedidos")')"
mide "over(\"ventas.espana.clientes\")        → $(celda python 'over("ventas.espana.clientes")')"

echo "E3 · una celda sql que escribe"
mide "create … ventas.resumen as … ventas.pedidos                  → $(celda sql 'create or replace table ventas.resumen as select pais, count(*) as n from ventas.pedidos group by pais')"
mide "create … ventas.default.resumen2 as … ventas.default.pedidos → $(celda sql 'create or replace table ventas.default.resumen2 as select * from ventas.default.pedidos')"
mide "create … ventas.espana.resumen as … ventas.pedidos           → $(celda sql 'create or replace table ventas.espana.resumen as select * from ventas.pedidos')"
mide "insert into ventas.espana.clientes select … ventas.pedidos    → $(celda sql 'insert into ventas.espana.clientes select id, pais from ventas.pedidos')"
mide "select … from ventas.default.pedidos (celda sql)              → $(celda sql 'select count(*) as n from ventas.default.pedidos')"

echo "E4 · lo que queda en el árbol"
mide "punteros: $(cd "$A/datasets" && find . -name '*.json' | sed 's#^\./##' | sort | tr '\n' ' ')"
mide "documentos de ventas: $(cd "$A/packages/ventas" && find . -name '*.yaml' | sed 's#^\./##' | sort | tr '\n' ' ')"
for f in "$A"/packages/ventas/datasets/*.yaml; do
  [ -f "$f" ] && mide "  $(basename "$f"): $(grep -E '^apiVersion|^metadata' "$f" | tr '\n' ' ' | cut -c1-150)"
done
mide "ore validate: $(cd "$A" && "$ORE" validate . 2>&1 | tail -3 | tr '\n' ' ' | cut -c1-300)"
"$ORE" assets "$A" --json > "$TMP/idx.json" 2>/dev/null
mide "índice: $("$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); print(sorted((k, v.get("schema"), bool(v.get("puntero"))) for k, v in d["items"].items() if v.get("kind") in ("Dataset",)))' "$TMP/idx.json" 2>&1 | cut -c1-400)"

echo "E5 · el catálogo /v1"
for r in /v1/config /v1/namespaces /v1/namespaces/ventas/tables /v1/namespaces/espana/tables "/v1/namespaces/ventas%1Fespana/tables" /v1/namespaces/ventas/tables/pedidos; do
  c=$(pide GET "$r")
  mide "GET $r → $c $(head -c 220 "$TMP/r.json" | tr '\n' ' ')"
done

echo "E6 · el servidor de SQL (LSP)"
lsp() { pide POST /puestos/puesto-ana-python/lsp "$("$PY" -c 'import json,sys; print(json.dumps({"mensajes": sys.argv[1:]}))' "$@")" >/dev/null; }
curl -sN --max-time 14 -H 'x-ore-sujeto: persona:ana' "$BASE/puestos/puesto-ana-python/lsp/consola" >"$TMP/lsp.txt" 2>/dev/null &
CURL=$!; sleep 1
abre() { printf '{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///t/%s","languageId":"sql","version":1,"text":%s}}}' "$1" "$("$PY" -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$2")"; }
lsp '{"jsonrpc":"2.0","id":"sql:1","method":"initialize","params":{"processId":null,"rootUri":"file:///t","capabilities":{},"initializationOptions":{"lenguaje":"sql"}}}' \
  "$(abre from.sql 'select * from ')" "$(abre base.sql 'select * from ventas.')" \
  "$(abre tres.sql 'select id from ventas.default.pedidos')" "$(abre esp.sql 'select id from ventas.espana.clientes')" \
  "$(abre dos.sql 'select id from ventas.pedidos')"
sleep 3
lsp '{"jsonrpc":"2.0","id":"sql:2","method":"textDocument/completion","params":{"textDocument":{"uri":"file:///t/from.sql"},"position":{"line":0,"character":14}}}' \
    '{"jsonrpc":"2.0","id":"sql:3","method":"textDocument/completion","params":{"textDocument":{"uri":"file:///t/base.sql"},"position":{"line":0,"character":21}}}'
wait $CURL 2>/dev/null
"$PY" - "$TMP/lsp.txt" <<'EOF'
import json, sys
ms = [json.loads(l[6:]) for l in open(sys.argv[1], encoding="utf-8") if l.startswith("data: ")]
por_id = {m["id"]: m for m in ms if "id" in m and "method" not in m}
for i, que in (("sql:2", "tras FROM"), ("sql:3", "tras `ventas.`")):
    items = [x["label"] for x in (por_id.get(i, {}).get("result") or {}).get("items", [])]
    print("  · completion %s: %s" % (que, items[:12]))
for m in ms:
    if m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"].endswith((".sql")):
        u = m["params"]["uri"].rsplit("/", 1)[1]
        if u in ("tres.sql", "esp.sql", "dos.sql"):
            print("  · diagnósticos de %s: %s" % (u, [" ".join(d["message"].split())[:140] for d in m["params"]["diagnostics"]]))
EOF
