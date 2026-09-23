#!/usr/bin/env bash
# EL PUESTO (0031 W3.1): la sesión viva, de punta a punta sin clúster.
#
# `ore-serve` con el árbol en un directorio y la cola en un repositorio pelado
# con la plantilla del puesto; los AGENTES DE VERDAD (`puesto/python/agente.py`,
# `puesto/node/agente.mjs`, `puesto/jvm/ore/Agente.java`) corriendo aquí al
# lado con la identidad de cabecera `agente:…` y el almacén en un directorio
# (`ORE_ALMACEN=dir:`), donde hay una copia de verdad —el sobre `ORECOPY1` con
# Parquet dentro— de la vista `hr.espanoles`. Los puestos node y jvm (8, 9) se
# prueban si hay `node` ≥ 22.13 y `javac` ≥ 21 (en CI, los hay).
#
#   1  POST /puestos (ana)             201 puesto-ana-python encolado · el fichero en la cola con
#                                      el id y el Job `puesto-ana-python-<8 hex>` · repetido, 200 el
#                                      mismo · bea no lo ve (403) · el agente no abre (403) ·
#                                      sin cola, 503
#   2  el agente reclama el puesto     GET /puestos/puesto-ana-python pasa a vivo · otro agente, 403
#   3  las celdas                      `1+1` → texto 2 · `print` → texto · `x = 3` → vacia ·
#                                      `x * 2` → 6 (el espacio dura) · `1/0` → error con traza ·
#                                      bea no manda celdas a lo de ana (403)
#   4  over("hr.espanoles")            tabla: columnas y filas de la copia, total · `hr.nada` →
#                                      error LookupError (404 de datos) · `hr.empleados` sin
#                                      copia → RuntimeError (409)
#   5  DELETE /puestos/puesto-ana-python      200 · el fichero fuera de la cola · el agente se cierra
#                                      (410) · una celda más → 410
#   7  SQL sobre el bucket (W3.3)     una celda `sql`: `select count(*) from hr.espanoles` → tabla
#                                      3 · un join de dos vistas · una que no existe → error ·
#                                      `sql()` desde una celda Python · `java` → 422
#   8  TS en node (W3.4)               POST /puestos {typescript} → puesto-ana-node con puesto-node:1 ·
#                                      el agente de Node (puesto/node/agente.mjs) · celdas TS (tipos
#                                      fuera, contexto que dura, await arriba) · un modulo con export
#                                      · saludo(persona()) → hola persona:ana · over() · sql · python 422
#   9  Java en jvm (W3.4)              POST /puestos {java} → puesto-ana-jvm con puesto-jvm:1 · el
#                                      agente de la JVM (JShell en proceso) · varios snippets por celda
#                                      · una clase con main · saludo(persona()) · over() · sql
#  9c  la capa de la JVM (0037 iii.c) una biblioteca de /capa se ve desde la celda, y lo que
#                                      TAMBIEN trae la imagen lo sigue poniendo la imagen
#  10  write() (W3.6c, 0031 §11)      la celda escribe `hr.lago` como `hr.salida`: la tabla va por
#                                      IPC a `ore-store-r2` (el S3 de mentira, con la credencial que
#                                      el catálogo prestó), el commit por `/v1/…`, y `over("hr.salida")`
#                                      devuelve EL MISMO JSON que hr.lago (los tipos sobreviven la
#                                      vuelta) · la misma escritura otra vez: `repetida`, un snapshot ·
#                                      `anexar` un DataFrame → 5 · `upsert` por clave → 6 y la Table
#                                      declara `mode: upsert, key: [n]` · a una View → error · uint64 → error
#                                      con la columna; y en 8 y 9, Node y Java escriben lo suyo y leen
#                                      lo de los demás
#  6b  la capa de la JVM (0037 iii.c)  GET /entorno/jvm sin-dependencias · un pom.xml ->
#                                      pendiente con SU digest (no el de python) · POST ->
#                                      202 y 55-la-capa-jvm-<corto>.yaml en la cola, con
#                                      capa-jvm:1 y sin el testigo · informe -> lista
#   6  la capa (W3.2)                  GET /entorno sin-dependencias · un pyproject → pendiente con
#                                      digest capa-<12 hex> · POST /puestos (bea) → 409 y el Job de
#                                      la capa en la cola con el digest · POST /entorno → 202 la
#                                      misma · el informe lista → GET /entorno lista, POST /entorno
#                                      200, POST /puestos 201 con la capa en el Job · otra
#                                      declaracion → pendiente otra vez
#
# Uso:  bash pruebas-de-fuego/el-puesto.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8921}"
PUERTO_SIN="${PUERTO_SIN:-8922}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""; SRV2=""; AGENTE=""; AGENTE2=""

falla() {
  echo "✗ $*" >&2
  [ -s "$TMP/arranque.txt" ] && { echo "── lo que dijo el servidor ──" >&2; tail -20 "$TMP/arranque.txt" >&2; }
  [ -s "$TMP/agente.txt" ] && { echo "── lo que dijo el agente ──" >&2; tail -20 "$TMP/agente.txt" >&2; }
  limpiar; exit 1
}
S3_PID=""
limpiar() {
  [ -n "$S3_PID" ] && kill "$S3_PID" 2>/dev/null
  [ -n "$AGENTE" ] && kill "$AGENTE" 2>/dev/null
  [ -n "${AGENTE2:-}" ] && kill "$AGENTE2" 2>/dev/null
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  [ -n "$SRV2" ] && kill "$SRV2" 2>/dev/null
  rm -rf "$TMP"
}
dice() { echo "  · $*"; }
cuerpo() { cat "$TMP/r.json" 2>/dev/null; }
tiene() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if eval(sys.argv[2]) else 1)' "$TMP/r.json" "$1"; }
buscar() {
  for c in "$RAIZ/target/release/$1" "$RAIZ/target/debug/$1" "$RAIZ/target/release/$1.exe" "$RAIZ/target/debug/$1.exe"; do
    [ -x "$c" ] && { echo "$c"; return 0; }
  done
  return 1
}
PY=$(command -v python3 || command -v python) || falla "hace falta python"
"$PY" -c 'import pyarrow, pandas' 2>/dev/null || falla "hacen falta pyarrow y pandas para la copia de prueba (pip install pyarrow pandas)"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\` — cargo build -p ore-serve"
ORE="$(buscar ore)" || falla "no hay binario de \`ore\` — cargo build -p ore-cli"
pide() { # <metodo> <ruta> <quien> [cuerpo]
  local m=$1 r=$2 q=$3 c=${4:-}
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$m" -H "$q" -H 'content-type: application/json' ${c:+--data-binary "$c"} "$BASE$r"
}
ANA='x-ore-sujeto: persona:ana'
BEA='x-ore-sujeto: persona:bea'
AG='x-ore-sujeto: agente:local'
OTRO='x-ore-sujeto: agente:otro'

# ── el árbol: un paquete con una tabla y dos vistas; una con copia ─────────
A="$TMP/arbol"
mkdir -p "$A/packages/hr/tables" "$A/packages/hr/views" "$A/datasets"
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: erp, type: jsonl, connectionEnv: ERP_URL }
Y
cat > "$A/packages/hr/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: hr, version: 1.0.0, status: active, domain: people }
spec: { owner: team:hr }
Y
# un dataset mantenido copia datos, y copiar instancia `materialization.payload`
cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:security
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y
cat > "$A/packages/hr/tables/empleados_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: empleados_t, namespace: hr }
spec:
  datasource: erp
  object: "empleados.jsonl"
  columns:
    id: {}
    pais: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
cat > "$A/packages/hr/views/empleados.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: empleados, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: { table: hr.empleados_t }
  fields: { id: id, pais: pais }
Y
# lo que se tiene son DATASETS (0033): `espanoles` y `lago` son copias con su
# plan sobre la pregunta `empleados`; sus punteros, en `datasets/`
mkdir -p "$A/packages/hr/datasets" "$A/datasets"
cat > "$A/packages/hr/datasets/espanoles.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: espanoles, namespace: hr }
spec:
  owner: team:data
  from: { view: hr.empleados }
  where: { pais: ES }
  fields: { id: id }
Y
cat > "$A/packages/hr/datasets/lago.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: lago, namespace: hr }
spec:
  owner: team:data
  from: { view: hr.empleados }
  fields: { id: id }
Y
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol de partida no compila: $(cd "$A" && "$ORE" validate . 2>&1 | head -3)"

# ── la copia: el sobre ORECOPY1 con Parquet, en un almacén de directorio ────
ALMACEN="$TMP/almacen"
mkdir -p "$ALMACEN/ore/v1"
# El agente corre con el python de la maquina: en Windows la ruta va a la Windows.
ALMACEN_PY="$ALMACEN"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) ALMACEN_PY="$(cd "$ALMACEN" && pwd -W)";; esac
CLAVE=$("$PY" - "$ALMACEN" <<'EOF'
import io, json, hashlib, sys, os
import pyarrow as pa, pyarrow.parquet as pq
t = pa.table({"id": ["e1", "e2", "e3"], "pais": ["ES", "ES", "ES"]})
b = io.BytesIO(); pq.write_table(t, b); carga = b.getvalue()
cab = json.dumps({"clave": "id", "conducto": "hr.espanoles", "esquema": "x", "plan": "x", "testigo": "x"}).encode()
art = b"ORECOPY1" + len(cab).to_bytes(4, "little") + cab + carga
clave = "ore/v1/" + hashlib.sha256(art).hexdigest()
open(os.path.join(sys.argv[1], clave), "wb").write(art)
print(clave)
EOF
)
[ -n "$CLAVE" ] || falla "no se pudo escribir la copia de prueba"
"$PY" -c 'import json,sys; json.dump({"estado":"copiada","clave":sys.argv[1],"plan":"x","filas":"3"}, open(sys.argv[2],"w"))' "$CLAVE" "$A/datasets/hr_espanoles.json"

# ── el dataset: una tabla Iceberg (0031 §10), y su puntero en el árbol ──────
# Escrita con PyIceberg (el catálogo es el árbol: `medida-w3-iceberg.py`); el
# puntero es `datasets/hr_lago.json` con `metadata_location`. Los tres SDK la leen
# EN SITIO con DuckDB por la raíz y la versión. Sin pyiceberg (o sin git) el caso
# se salta y se dice; el camino https contra GCS está medido en `medida-w3-lago.py`.
LAGO_OK=no
if "$PY" -c "import pyiceberg" 2>/dev/null; then
  META=$("$PY" - "$ALMACEN_PY" "$RAIZ" <<'EOF'
import datetime as dt, decimal, importlib.util, os, sys, tempfile
import pyarrow as pa
sp = importlib.util.spec_from_file_location("ice", os.path.join(sys.argv[2], "pruebas-de-fuego", "medida-w3-iceberg.py"))
ice = importlib.util.module_from_spec(sp); sp.loader.exec_module(ice)
Arbol = ice.catalogo_arbol()
bodega = os.path.join(sys.argv[1], "lago").replace("\\", "/")
cat = Arbol("prueba", tempfile.mkdtemp(prefix="ore-lago-arbol-").replace("\\", "/"), warehouse=bodega)
cat.create_namespace("hr")
utc = dt.timezone.utc
t = pa.table({
    "n": pa.array([1, 2, 3], pa.int64()),
    "letra": pa.array(["a", "b", None], pa.string()),
    "cuando": pa.array([dt.datetime(2024, 6, 1, 12, 0, tzinfo=utc), None, dt.datetime(2024, 6, 1, 12, 0, 0, 500000, tzinfo=utc)], pa.timestamp("us", tz="UTC")),
    "importe": pa.array([decimal.Decimal("1.50"), decimal.Decimal("2.25"), None], pa.decimal128(10, 2)),
})
tb = cat.create_table("hr.lago", t.schema)
tb.append(t)
print(tb.metadata_location)
EOF
  )
  if [ -n "$META" ]; then
    "$PY" -c 'import json,sys; json.dump({"estado":"copiada","metadata_location":sys.argv[1],"snapshot":"1","plan":"x","filas":"3"}, open(sys.argv[2],"w"))' "$META" "$A/datasets/hr_lago.json"
    LAGO_OK=si
  else
    echo "  (pyiceberg no pudo escribir la tabla: el dataset Iceberg no se prueba aqui)"
  fi
else
  echo "  (sin pyiceberg: el dataset Iceberg no se prueba aqui — python -m pip install 'pyiceberg[pyarrow]')"
fi
# Lo que los tres agentes tienen que enseñar de hr.lago: el MISMO JSON (0032 T3).
LAGO_COLS="[c['name'] for c in d['salida']['columnas']]==['n','letra','cuando','importe'] and [c['type'] for c in d['salida']['columnas']]==['int64','string','timestamp[us, tz=UTC]','decimal128(10, 2)']"
LAGO_FILAS="d['salida']['filas']==[[1,'a','2024-06-01T12:00:00Z','1.50'],[2,'b',None,'2.25'],[3,None,'2024-06-01T12:00:00.5Z',None]] and d['salida']['total']==3"

# ── la cola: un repositorio pelado con la plantilla del puesto ─────────────
COLA="$TMP/cola.git"
git init -q --bare -b main "$COLA"
mkdir -p "$TMP/cola-semilla" && ( cd "$TMP/cola-semilla" && git init -q -b main && git config core.autocrlf false )
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TMP/rendido" >/dev/null 2>&1 || falla "no se pudo rendir la plantilla del puesto"
[ -f "$TMP/rendido/plantilla-puesto.txt" ] || falla "gen-inquilino no rinde plantilla-puesto.txt"
cp "$TMP/rendido/plantilla-puesto.txt" "$TMP/rendido/plantilla-capa.txt" \
   "$TMP/rendido/plantilla-capa-jvm.txt" "$TMP/cola-semilla/"
( cd "$TMP/cola-semilla" && git add -A && git -c user.name=banco -c user.email=banco@invalido commit -q -m "la plantilla" \
  && git remote add origin "$COLA" && git push -q origin HEAD:main ) || falla "no se pudo sembrar la cola"
en_cola() { git --git-dir="$COLA" show "main:$1" 2>/dev/null; }
COLA_URL="file://$(cd "$COLA" && pwd | sed 's#^/\([a-zA-Z]\)/#\1:/#')"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) COLA_URL="file:///$(cd "$COLA" && pwd -W)";; esac

# ── el lago para escribir (W3.6c): el S3 de mentira, y ore-serve como catálogo ──
# `write()` deja los ficheros ahí con la credencial que `ore-serve` presta (la de
# siempre: en local no hay acotado) y el commit va por `/v1/…`; el SDK encuentra
# `ore-store-r2` por ORE_STORE_DIR.
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || falla "el S3 de mentira no arrancó"
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia" ORE_RETENCION=7d
export ORE_STORE_DIR="$(dirname "$ORE")"
export PATH="$ORE_STORE_DIR:$PATH"   # `ore datasets` corre `ore-store-r2` por el PATH

# ── el servidor (y otro sin cola) ──────────────────────────────────────────
FORJA_TOKEN=no-hace-falta-en-file "$SERVE" --repo "$A" --ore "$ORE" --bind "127.0.0.1:$PUERTO" --cola "$COLA_URL" \
  --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/arranque.txt" 2>&1 &
SRV=$!
"$SERVE" --repo "$A" --ore "$ORE" --bind "127.0.0.1:$PUERTO_SIN" \
  --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/arranque2.txt" 2>&1 &
SRV2=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
curl -s -o /dev/null "$BASE/salud" || falla "el servidor no arranca"

# ── 1 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /puestos "$ANA" '{}')" = "201" ] || falla "1 · abrir: $(cuerpo)"
tiene "d['id']=='puesto-ana-python' and d['estado']=='encolado' and d['entorno']=='python' and d['fichero']=='51-el-puesto-ana-python.yaml' and d['job'].startswith('puesto-ana-python-') and 'commit' in d['cola']" || falla "1 · la ficha: $(cuerpo)"
en_cola 51-el-puesto-ana-python.yaml | grep -q 'name: PUESTO, value: "puesto-ana-python"' || falla "1 · el fichero de la cola no lleva el id: $(en_cola 51-el-puesto-ana-python.yaml | grep -n PUESTO)"
en_cola 51-el-puesto-ana-python.yaml | grep -qE 'name: puesto-ana-python-[0-9a-f]{8}$' || falla "1 · el Job no se llama puesto-ana-python-<8 hex>"
en_cola 51-el-puesto-ana-python.yaml | grep -q 'ore.dev/rol: puesto' || falla "1 · el Job no lleva el rol puesto"
[ "$(pide POST /puestos "$ANA" '{}')" = "200" ] && tiene "d['id']=='puesto-ana-python'" || falla "1 · repetir no dio 200 con el mismo: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana-python "$BEA")" = "403" ] || falla "1 · bea ve el puesto de ana: $(cuerpo)"
[ "$(pide POST /puestos "$AG" '{}')" = "403" ] || falla "1 · un agente abrio un puesto: $(cuerpo)"
[ "$(pide POST /puestos "$BEA" '{"lenguaje":"rust"}')" = "422" ] || falla "1 · rust no dio 422: $(cuerpo)"
en_cola 51-el-puesto-ana-python.yaml | grep -q 'image: .*/puesto-python:1' || falla "1 · el Job no lleva la imagen del entorno python"
en_cola 51-el-puesto-ana-python.yaml | grep -q 'puesto-entorno-modelo' && falla "1 · el hueco del entorno se quedo sin rendir"
[ "$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$BEA" -H 'content-type: application/json' --data-binary '{}' "http://127.0.0.1:$PUERTO_SIN/puestos")" = "503" ] || falla "1 · sin cola no dio 503: $(cuerpo)"
dice "1 · POST /puestos: 201 puesto-ana-python encolado, el fichero en la cola con id, Job puesto-ana-python-<8 hex>, rol puesto e imagen puesto-python:1 · repetido 200 · bea 403 · un agente 403 · rust 422 · sin cola 503"

# ── 2 · el agente reclama el puesto ────────────────────────────────────────
# ⭐ `ORE_LSP` (0037 ③a): el servidor de lenguaje que el agente arrancará
#   cuando llegue el primer mensaje. Aquí, uno DE MENTIRA —cuarenta líneas que
#   hablan LSP— porque lo que esta prueba ejercita es LA CORREA, no pyright:
#   pyright lo prueba la construcción de la imagen, que lo corre contra el SDK.
#
# ⛔ Sin rutas absolutas en `ORE_LSP`: en Git Bash, un valor con `C:\` dentro lo
#   convierte MSYS y el agente acaba arrancando `C;C:\Program Files\Git\...`
#   (medido). El servidor se llama por su nombre y se resuelve desde el
#   directorio de trabajo del agente.
TMP_PY="$TMP"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) TMP_PY="$(cd "$TMP" && pwd -W)";; esac
cat > "$TMP/lsp-de-mentira.py" <<'FIN_LSP'
import json
import sys

e, sal = sys.stdin.buffer, sys.stdout.buffer


def manda(m):
    b = json.dumps(m).encode("utf-8")
    sal.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
    sal.flush()


while True:
    largo = None
    while True:
        linea = e.readline()
        if not linea:
            sys.exit(0)
        linea = linea.strip()
        if not linea:
            break
        if linea.lower().startswith(b"content-length:"):
            largo = int(linea.split(b":")[1])
    if largo is None:
        continue
    m = json.loads(e.read(largo).decode("utf-8"))
    if "id" in m:
        manda({"jsonrpc": "2.0", "id": m["id"], "result": {"eco": m.get("method"), "nulo": None}})
    if m.get("method") == "textDocument/didOpen":
        manda({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
               "params": {"uri": m["params"]["textDocument"]["uri"],
                          "diagnostics": [{"message": "asi no", "severity": 1,
                                           "range": {"start": {"line": 0, "character": 0},
                                                     "end": {"line": 0, "character": 3}}}]}})
FIN_LSP
ORE_SERVE="$BASE" PUESTO=puesto-ana-python ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
  ORE_LSP="$(basename "$PY") lsp-de-mentira.py" TRABAJO_DIR="$TMP_PY" \
  "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/agente.txt" 2>&1 &
AGENTE=$!
for _ in $(seq 1 40); do pide GET /puestos/puesto-ana-python "$ANA" >/dev/null; tiene "d['estado']=='vivo'" && break; sleep 0.25; done
tiene "d['estado']=='vivo'" || falla "2 · el puesto no pasa a vivo: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana-python/pendiente "$OTRO")" = "403" ] || falla "2 · otro agente reclamo el puesto: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana-python/pendiente "$ANA")" = "403" ] || falla "2 · una persona pidio trabajo: $(cuerpo)"
dice "2 · el agente reclama el puesto: vivo · otro agente 403 · una persona 403"

# ── 3 · las celdas ─────────────────────────────────────────────────────────
P=puesto-ana-python; LEN=python   # el puesto y el lenguaje de `celda` (8 y 9 los cambian)
celda() { # <texto json-escapado> → deja la salida en r.json; imprime el codigo de la espera
  local n
  [ "$(pide POST /puestos/$P/ejecutar "$ANA" "{\"texto\":\"$1\",\"lenguaje\":\"$LEN\"}")" = "202" ] || falla "ejecutar ($LEN en $P) no dio 202: $(cuerpo)"
  n=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["celda"])' "$TMP/r.json")
  for _ in $(seq 1 3); do
    pide GET "/puestos/$P/celdas/$n" "$ANA" >/dev/null
    tiene "d['estado']=='hecha'" && return 0
  done
  return 1
}
celda '1+1' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='2'" || falla "3 · 1+1: $(cuerpo)"
celda 'print(\"hola\")' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='hola\n'" || falla "3 · print: $(cuerpo)"
celda 'x = 3' && tiene "d['salida']['tipo']=='vacia'" || falla "3 · x = 3: $(cuerpo)"
celda 'x * 2' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='6'" || falla "3 · x * 2 (el espacio dura): $(cuerpo)"
celda '1/0' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ZeroDivisionError' and 'Traceback' in d['salida']['traza']" || falla "3 · 1/0: $(cuerpo)"
[ "$(pide POST /puestos/puesto-ana-python/ejecutar "$BEA" '{"texto":"1"}')" = "403" ] || falla "3 · bea mando una celda a lo de ana: $(cuerpo)"
[ "$(pide POST /puestos/puesto-ana-python/ejecutar "$ANA" '{}')" = "422" ] || falla "3 · sin texto no dio 422: $(cuerpo)"
pide GET /puestos/puesto-ana-python "$ANA" >/dev/null; tiene "d['celdas']==5 and d['pendientes']==0" || falla "3 · la ficha no cuenta las celdas: $(cuerpo)"
dice "3 · las celdas: 1+1 → 2 · print → texto · x = 3 → vacia · x * 2 → 6 (el espacio dura) · 1/0 → error con traza · bea 403 · sin texto 422"

# ── 3b · el flujo: una respuesta que no termina (0037 ②) ───────────────────
# La consola no pregunta una vez por celda: abre UNA respuesta y lo que le pasa
# al puesto le llega SEGUN PASA. Aqui se abre con curl, se manda una celda
# mientras esta abierta, y se comprueba que el evento sale solo.
FLU="$TMP/flujo.txt"
curl -sN -D "$TMP/flujo.cab" --max-time 6 -H "$ANA" "$BASE/puestos/$P/flujo" >"$FLU" 2>/dev/null &
CURL=$!
sleep 1
[ "$(pide POST /puestos/$P/ejecutar "$ANA" '{"texto":"40+2","lenguaje":"python"}')" = "202" ] || falla "3b · ejecutar no dio 202: $(cuerpo)"
N=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["celda"])' "$TMP/r.json")
wait $CURL 2>/dev/null || true
grep -qi "^transfer-encoding: chunked" "$TMP/flujo.cab" || falla "3b · el flujo no vino troceado: $(cat "$TMP/flujo.cab")"
grep -qi "^content-type: text/event-stream" "$TMP/flujo.cab" || falla "3b · el flujo no dice que es de eventos: $(cat "$TMP/flujo.cab")"
grep -qi "^content-length" "$TMP/flujo.cab" && falla "3b · un flujo con content-length: $(cat "$TMP/flujo.cab")"
grep -q "^event: puesto" "$FLU" || falla "3b · el flujo no abrio diciendo como esta el puesto: $(cat "$FLU")"
grep -q "^id: $N" "$FLU" || falla "3b · la celda $N no llego por el flujo: $(cat "$FLU")"
grep -qE '"texto": *"42"' "$FLU" || falla "3b · la salida no viajo con la celda: $(cat "$FLU")"
# Abierto desde cero, el flujo cuenta primero lo que ya habia: quien acaba de
# abrir el editor quiere el estado, no solo lo que pase de ahora en adelante.
[ "$(grep -c "^event: celda" "$FLU")" = "$N" ] || falla "3b · el flujo no puso al dia las $N celdas: $(cat "$FLU")"

# Y se retoma: con `last-event-id` NO se repite lo ya visto, y sí llega lo nuevo.
curl -sN --max-time 6 -H "$ANA" -H "last-event-id: $N" "$BASE/puestos/$P/flujo" >"$TMP/flujo2.txt" 2>/dev/null &
CURL=$!
sleep 1
[ "$(pide POST /puestos/$P/ejecutar "$ANA" '{"texto":"6*7","lenguaje":"python"}')" = "202" ] || falla "3b · la segunda celda no dio 202: $(cuerpo)"
M=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["celda"])' "$TMP/r.json")
wait $CURL 2>/dev/null || true
grep -q "^id: $M" "$TMP/flujo2.txt" || falla "3b · al retomar no llego la celda $M: $(cat "$TMP/flujo2.txt")"
grep -q "^id: $N" "$TMP/flujo2.txt" && falla "3b · al retomar repitio la celda $N: $(cat "$TMP/flujo2.txt")"
[ "$(pide GET /puestos/$P/flujo "$BEA")" = "403" ] || falla "3b · bea abrio el flujo de ana: $(cuerpo)"
[ "$(pide GET /puestos/no-existe/flujo "$ANA")" = "404" ] || falla "3b · un puesto que no existe no dio 404: $(cuerpo)"
dice "3b · el flujo: text/event-stream troceado y sin content-length · abre con el estado del puesto y las $N celdas que ya habia · la ultima llega SOLA con su salida · se retoma con last-event-id (ni repite ni pierde) · bea 403 · uno que no existe 404"

# ── 3c · el servidor de lenguaje, de punta a punta (0037 ③a) ───────────────
# El editor manda un mensaje de LSP, `ore-serve` lo pasa SIN ABRIRLO, el agente
# se lo da al servidor de lenguaje de su puesto, y lo que conteste vuelve por el
# flujo hasta el editor. Aquí el servidor es de mentira; la correa es de verdad.
curl -sN --max-time 8 -H "$ANA" "$BASE/puestos/$P/lsp/consola" >"$TMP/lsp.txt" 2>/dev/null &
CURL=$!
sleep 1
MSG='{"jsonrpc":"2.0","id":7,"method":"initialize","params":{"raiz":null}}'
[ "$(pide POST /puestos/$P/lsp "$ANA" "{\"mensajes\":[$("$PY" -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$MSG")]}")" = "202" ] || falla "3c · mandar un mensaje de LSP no dio 202: $(cuerpo)"
ABRIR='{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///transforms/ejemplo.py","languageId":"python","version":1,"text":"x = 1\n"}}}'
[ "$(pide POST /puestos/$P/lsp "$ANA" "{\"mensajes\":[$("$PY" -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$ABRIR")]}")" = "202" ] || falla "3c · el didOpen no dio 202: $(cuerpo)"
wait $CURL 2>/dev/null || true
grep -q "^event: lsp" "$TMP/lsp.txt" || falla "3c · no volvió ni un mensaje del servidor de lenguaje: $(cat "$TMP/lsp.txt")"
grep -qE '"id": *7' "$TMP/lsp.txt" || falla "3c · la respuesta no trae el id que se mandó: $(cat "$TMP/lsp.txt")"
grep -qE '"eco": *"initialize"' "$TMP/lsp.txt" || falla "3c · el servidor no vio el método: $(cat "$TMP/lsp.txt")"
# ⛔ Y el `null` sigue siendo `null`: los mensajes viajan como cadenas porque el
#   `Json` de este árbol no modela `null` ni los dobles — pasarlos por él los
#   volvería las cadenas "null" y "1.5" (está medido y escrito en `json.rs`).
grep -qE '"nulo": *null' "$TMP/lsp.txt" || falla "3c · el null llegó degradado: $(cat "$TMP/lsp.txt")"
grep -q "publishDiagnostics" "$TMP/lsp.txt" || falla "3c · el aviso del servidor (sin id) no llegó: $(cat "$TMP/lsp.txt")"
grep -q "asi no" "$TMP/lsp.txt" || falla "3c · el diagnóstico no llegó entero: $(cat "$TMP/lsp.txt")"
# Las dos puertas: el puesto es de una persona, y el flujo del agente es del agente.
[ "$(pide POST /puestos/$P/lsp "$BEA" '{"mensajes":["{}"]}')" = "403" ] || falla "3c · bea le habló al servidor de lenguaje de ana: $(cuerpo)"
[ "$(pide GET /puestos/$P/lsp/consola "$BEA")" = "403" ] || falla "3c · bea escuchó el flujo de ana: $(cuerpo)"
[ "$(pide GET /puestos/$P/lsp/agente "$ANA")" = "403" ] || falla "3c · una persona se puso en el sitio del agente: $(cuerpo)"
[ "$(pide POST /puestos/$P/lsp "$ANA" '{"mensajes":[{"jsonrpc":"2.0"}]}')" = "422" ] || falla "3c · un mensaje que no es cadena no dio 422: $(cuerpo)"
[ "$(pide POST /puestos/$P/lsp "$ANA" '{}')" = "422" ] || falla "3c · sin mensajes no dio 422: $(cuerpo)"
dice "3c · el servidor de lenguaje: el editor manda y vuelve por el flujo (id, metodo y el null INTACTO), el aviso sin id tambien · bea 403 en los dos sentidos · una persona no es el agente 403 · un mensaje que no es cadena 422"

# ── 4 · over() ─────────────────────────────────────────────────────────────
celda 'df = over(\"hr.espanoles\"); df' && tiene "d['salida']['tipo']=='tabla' and [c['name'] for c in d['salida']['columnas']]==['id','pais'] and d['salida']['filas']==[['e1','ES'],['e2','ES'],['e3','ES']] and d['salida']['total']==3" || falla "4 · over(hr.espanoles): $(cuerpo)"
celda 'len(df)' && tiene "d['salida']['texto']=='3'" || falla "4 · len(df): $(cuerpo)"
celda 'over(\"hr.nada\")' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='LookupError'" || falla "4 · hr.nada: $(cuerpo)"
celda 'over(\"hr.empleados\")' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='RuntimeError' and 'no est' in d['salida']['mensaje']" || falla "4 · hr.empleados sin copia: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana-python/datos/hr.espanoles "$AG")" = "200" ] && tiene "d['clave']=='$CLAVE' and d['estado']=='copiada'" || falla "4 · datos: $(cuerpo)"
if [ "$LAGO_OK" = "si" ]; then
  [ "$(pide GET /puestos/puesto-ana-python/datos/hr.lago "$AG")" = "200" ] && tiene "d['metadata_location'].endswith('.metadata.json') and d['clave']=='' and d['estado']=='copiada'" || falla "4 · datos de un dataset Iceberg: $(cuerpo)"
  celda 'over(\"hr.lago\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "4 · over(hr.lago), el dataset Iceberg: $(cuerpo)"
  celda 'over(\"hr.lago\", como=\"arrow\").num_rows' && tiene "d['salida']['texto']=='3'" || falla "4 · over(hr.lago, arrow): $(cuerpo)"
fi
[ "$(pide GET /puestos/puesto-ana-python/datos/hr.espanoles "$ANA")" = "403" ] || falla "4 · una persona pidio datos por la ruta del agente: $(cuerpo)"
dice "4 · over(\"hr.espanoles\") → tabla 3 × 2 desde la copia (ORECOPY1 + Parquet) · over(\"hr.lago\") → el dataset Iceberg leido en sitio por la raiz y la version del puntero (tipos del contrato) · hr.nada → LookupError (404) · sin copia → RuntimeError (409) · datos solo para el agente"

# ── 7 · SQL sobre el bucket (W3.3): la consulta entera, sobre las copias ──
celda_sql() { local l=$LEN; LEN=sql; celda "$1"; local r=$?; LEN=$l; return $r; }
celda_sql 'select count(*) as n from hr.espanoles' && tiene "d['salida']['tipo']=='tabla' and d['salida']['columnas'][0]['name']=='n' and d['salida']['filas']==[[3]]" || falla "7 · count sobre la copia: $(cuerpo)"
celda_sql 'select a.id, b.pais from hr.espanoles a join hr.espanoles b on a.id = b.id order by 1' && tiene "d['salida']['tipo']=='tabla' and d['salida']['total']==3 and d['salida']['filas'][0]==['e1','ES']" || falla "7 · el join: $(cuerpo)"
celda_sql 'select * from hr.nada' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='LookupError'" || falla "7 · una vista que no existe: $(cuerpo)"
if [ "$LAGO_OK" = "si" ]; then
  celda_sql 'select count(*) as n, sum(importe) as s from hr.lago' && tiene "d['salida']['tipo']=='tabla' and d['salida']['filas']==[[3,'3.75']]" || falla "7 · sql sobre el dataset Iceberg: $(cuerpo)"
fi
celda_sql 'select * from hr.empleados' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='RuntimeError'" || falla "7 · una vista sin copia: $(cuerpo)"
celda_sql 'selec nada' && tiene "d['salida']['tipo']=='error'" || falla "7 · sql roto: $(cuerpo)"
celda 'sql(\"select sum(1) as s from hr.espanoles\")' && tiene "d['salida']['tipo']=='tabla' and d['salida']['filas']==[[3]]" || falla "7 · sql() desde python: $(cuerpo)"
[ "$(pide POST /puestos/puesto-ana-python/ejecutar "$ANA" '{"texto":"x","lenguaje":"java"}')" = "422" ] && grep -q 'abre uno `jvm`' "$TMP/r.json" || falla "7 · java en un puesto python no dio 422: $(cuerpo)"
[ "$(pide POST /puestos/puesto-ana-python/ejecutar "$ANA" '{"texto":"x","lenguaje":"rust"}')" = "422" ] || falla "7 · rust no dio 422: $(cuerpo)"
dice "7 · SQL sobre el bucket: count sobre la copia → tabla · join de dos vistas · vista inexistente → LookupError · sin copia → RuntimeError · sql roto → error · sql() desde python · java en un puesto python 422 (abre uno jvm) · rust 422"

# ── 10 · write() (W3.6c, 0031 §11): la celda escribe un dataset, y los tres lo leen ──
if [ "$LAGO_OK" = "si" ] && [ -x "$ORE_STORE_DIR/ore-store-r2" -o -x "$ORE_STORE_DIR/ore-store-r2.exe" ]; then
  celda 'e = write(\"hr.salida\", over(\"hr.lago\", como=\"arrow\")); (e[\"filas\"], e[\"repetida\"], e[\"snapshot\"] != \"\")' && tiene "d['salida']['texto']=='(3, False, True)'" || falla "10 · write(hr.salida): $(cuerpo)"
  [ -f "$A/packages/hr/datasets/salida.yaml" ] && grep -q "kind: Dataset" "$A/packages/hr/datasets/salida.yaml" && grep -q "cuando: { type: DateTimeTz }" "$A/packages/hr/datasets/salida.yaml" || falla "10 · el Dataset escrito no nació tipado en el árbol: $(cat "$A/packages/hr/datasets/salida.yaml" 2>/dev/null)"
  [ -f "$A/datasets/hr_salida.json" ] || falla "10 · el puntero no está en el árbol"
  grep -q "type: lago" "$A/ontology.config.yaml" && falla "10 · write() declaró un datasource lago, y ya no hay tal cosa (0033)"
  # lo escrito, leído: EL MISMO JSON que hr.lago (los cuatro tipos sobreviven la vuelta)
  celda 'over(\"hr.salida\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "10 · over(hr.salida) no es el mismo JSON que hr.lago: $(cuerpo)"
  # la misma escritura otra vez: repetida, y un solo snapshot
  celda 'e = write(\"hr.salida\", over(\"hr.lago\", como=\"arrow\")); e[\"repetida\"]' && tiene "d['salida']['texto']=='True'" || falla "10 · la misma escritura tenía que ser repetida: $(cuerpo)"
  [ "$(pide GET /datasets/hr/salida "$ANA")" = "200" ] && tiene "len(d['snapshots'])==1 and d['escrito_por']=='persona:ana' and d['snapshots'][0]['idempotencia']!=''" || falla "10 · la ficha: $(cuerpo)"
  # anexar un DataFrame de pandas: 5 filas, y el esquema se respeta
  celda 'import pandas as pd, datetime as dt, decimal; e = write(\"hr.salida\", pd.DataFrame({\"n\": [4, 5], \"letra\": [\"d\", \"e\"], \"cuando\": [dt.datetime(2024, 6, 2, tzinfo=dt.timezone.utc)] * 2, \"importe\": [decimal.Decimal(\"4.00\"), decimal.Decimal(\"5.00\")]}), modo=\"anexar\"); e[\"filas\"]' && tiene "d['salida']['texto']=='5'" || falla "10 · anexar: $(cuerpo)"
  celda 'sql(\"select count(*) as n, sum(importe) as s from hr.salida\")' && tiene "d['salida']['filas']==[[5,'12.75']]" || falla "10 · sql sobre lo escrito: $(cuerpo)"
  # upsert por clave (0031 §11 ⑤): el 4 cambia (4.00 → 40.00), el 6 es nuevo → 6 filas, 54.75; la Table declara la clave
  celda 'import pandas as pd, datetime as dt, decimal; e = write(\"hr.salida\", pd.DataFrame({\"n\": [4, 6], \"letra\": [\"D\", \"f\"], \"cuando\": [dt.datetime(2024, 6, 3, tzinfo=dt.timezone.utc)] * 2, \"importe\": [decimal.Decimal(\"40.00\"), decimal.Decimal(\"6.00\")]}), modo=\"upsert\", clave=[\"n\"]); e[\"filas\"]' && tiene "d['salida']['texto']=='6'" || falla "10 · upsert: $(cuerpo)"
  celda 'sql(\"select count(*) as n, sum(importe) as s from hr.salida\")' && tiene "d['salida']['filas']==[[6,'54.75']]" || falla "10 · sql tras el upsert: $(cuerpo)"
  grep -q "changes: { mode: upsert, key: \[n\] }" "$A/packages/hr/datasets/salida.yaml" || falla "10 · el Dataset no declara el upsert: $(grep changes "$A/packages/hr/datasets/salida.yaml")"
  celda 'write(\"hr.salida\", over(\"hr.lago\", como=\"arrow\"), clave=[\"n\"])' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ValueError'" || falla "10 · clave sin upsert tenía que ser ValueError: $(cuerpo)"
  # lo que no se escribe: una View, y una columna que 0032 no tiene
  celda 'write(\"hr.espanoles\", over(\"hr.lago\", como=\"arrow\"))' && tiene "d['salida']['tipo']=='error' and 'mantenido' in d['salida']['mensaje']" || falla "10 · escribir un dataset mantenido: $(cuerpo)"
  celda 'write(\"hr.empleados\", over(\"hr.lago\", como=\"arrow\"))' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "10 · escribir una View: $(cuerpo)"
  celda 'import pyarrow as pa; write(\"hr.mala\", pa.table({\"grande\": pa.array([1], pa.uint64())}))' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ValueError' and 'grande' in d['salida']['mensaje']" || falla "10 · uint64: $(cuerpo)"
  [ ! -f "$A/datasets/hr_mala.json" ] || falla "10 · lo negado dejó puntero"
  # declarar (W3.7 ①): una View sobre lo que la celda escribió, por la puerta de Forge
  celda 'd = declare(\"apiVersion: oos.dev/v1alpha12\\nkind: View\\nmetadata: { name: porLetra, namespace: hr }\\nspec:\\n  owner: team:hr\\n  from: { dataset: hr.salida }\\n  fields: { letra: letra, n: \\\"count()\\\" }\\n  groupBy: [letra]\\n\"); [d[\"kind\"], d[\"nombre\"], d[\"fichero\"], d[\"nueva\"]]' && tiene "d['salida']['texto']==\"['View', 'hr.porLetra', 'packages/hr/views/porLetra.yaml', True]\"" || falla "10 · declare(View): $(cuerpo)"
  grep -q "groupBy: \[letra\]" "$A/packages/hr/views/porLetra.yaml" || falla "10 · la View declarada no está en el árbol"
  celda 'declare({\"kind\": \"View\", \"metadata\": {\"name\": \"porLetra\", \"namespace\": \"hr\"}, \"spec\": {\"owner\": \"team:hr\", \"from\": {\"dataset\": \"hr.salida\"}, \"fields\": {\"letra\": \"letra\", \"n\": \"count()\"}, \"groupBy\": [\"letra\"]}})[\"nueva\"]' && tiene "d['salida']['texto']=='False'" || falla "10 · declare(dict) otra vez: $(cuerpo)"
  celda 'declare(\"apiVersion: oos.dev/v1alpha8\\nkind: View\\nmetadata: { name: rota, namespace: hr }\\nspec:\\n  owner: team:hr\\n  from: { table: hr.nadie }\\n  fields: { a: a }\\n\")' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ValueError' and 'OOS' in d['salida']['mensaje']" || falla "10 · declare de una View rota tenía que ser ValueError con el diagnóstico: $(cuerpo)"
  [ ! -f "$A/packages/hr/views/rota.yaml" ] || falla "10 · lo negado quedó en el árbol"
  celda 'declare(\"kind: Model\\nmetadata: { name: x, namespace: hr }\\nspec: {}\\n\")' && tiene "d['salida']['tipo']=='error' and 'Model' in d['salida']['mensaje'] and '404' in d['salida']['mensaje']" || falla "10 · declare(Model) tenía que decir que no se sirve: $(cuerpo)"
  # el modelo entrenado (v1alpha11, W3.7 ②): un asset del paquete, con su linaje por la View declarada
  celda 'd = declare(\"apiVersion: oos.dev/v1alpha11\\nkind: TrainedModel\\nmetadata: { name: prevision, namespace: hr }\\nspec:\\n  owner: team:hr\\n  framework: sklearn\\n  task: forecast\\n  version: 1\\n  artifacts: models/hr_prevision/v1\\n  digest: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\\n  trainedFrom: [hr.porLetra]\\n\"); [d[\"kind\"], d[\"fichero\"], d[\"nueva\"]]' && tiene "d['salida']['texto']==\"['TrainedModel', 'packages/hr/models/prevision.yaml', True]\"" || falla "10 · declare(TrainedModel): $(cuerpo)"
  grep -q "trainedFrom: \[hr.porLetra\]" "$A/packages/hr/models/prevision.yaml" || falla "10 · el TrainedModel no está en el árbol"
  celda 'declare(\"apiVersion: oos.dev/v1alpha11\\nkind: TrainedModel\\nmetadata: { name: rota, namespace: hr }\\nspec:\\n  owner: team:hr\\n  framework: sklearn\\n  version: 1\\n  artifacts: models/hr_rota/v1\\n  digest: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\\n  trainedFrom: [hr.nadie]\\n\")' && tiene "d['salida']['tipo']=='error' and 'OOS2005' in d['salida']['mensaje']" || falla "10 · un TrainedModel con linaje roto tenía que ser OOS2005: $(cuerpo)"
  celda 'declare(\"apiVersion: oos.dev/v1alpha11\\nkind: TrainedModel\\nmetadata: { name: sin, namespace: hr }\\nspec:\\n  owner: team:hr\\n  framework: sklearn\\n  version: 1\\n  artifacts: models/hr_sin/v1\\n\")' && tiene "d['salida']['tipo']=='error' and 'OOS1004' in d['salida']['mensaje'] and 'digest' in d['salida']['mensaje']" || falla "10 · un TrainedModel sin digest tenía que ser OOS1004: $(cuerpo)"
  [ ! -f "$A/packages/hr/models/rota.yaml" ] && [ ! -f "$A/packages/hr/models/sin.yaml" ] || falla "10 · lo negado quedó en el árbol"
  # la procedencia (W3.7 ③): lo escrito fuera de un transform dice lo que la sesión leyó; dentro, sus inputs
  "$PY" -c 'import json,sys; p=json.load(open(sys.argv[1])); pr=p["procedencia"]; assert pr["puesto"]=="puesto-ana-python" and "hr.lago" in pr["leidas"] and "hr.salida" not in pr["leidas"], pr' "$A/datasets/hr_salida.json" || falla "10 · el puntero no lleva la procedencia (leidas, sin él mismo): $(cat "$A/datasets/hr_salida.json")"
  celda '@transform(inputs=[\"hr.lago\"], output=\"hr.resumen\")\ndef resumir():\n    t = over(\"hr.lago\", como=\"arrow\")\n    return write(\"hr.resumen\", t)\ne = resumir(); e[\"filas\"]' && tiene "d['salida']['texto']=='3'" || falla "10 · un transform: $(cuerpo)"
  "$PY" -c 'import json,sys; p=json.load(open(sys.argv[1])); pr=p["procedencia"]; assert pr=={"inputs":["hr.lago"],"puesto":"puesto-ana-python","transform":"resumir"}, pr' "$A/datasets/hr_resumen.json" || falla "10 · el puntero del transform no lleva inputs y transform: $(cat "$A/datasets/hr_resumen.json")"
  [ "$(pide GET /datasets/hr/resumen "$ANA")" = "200" ] && tiene "d['procedencia']['transform']=='resumir' and d['snapshots'][0]['procedencia']['inputs']==['hr.lago']" || falla "10 · la ficha no enseña la procedencia: $(cuerpo)"
  celda '@transform(inputs=[\"hr.lago\"], output=\"hr.resumen\")\ndef fuera():\n    return over(\"hr.salida\", como=\"arrow\")\nfuera()' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='PermissionError' and 'hr.salida' in d['salida']['mensaje']" || falla "10 · leer fuera de los inputs tenía que ser PermissionError: $(cuerpo)"
  celda '@transform(inputs=[\"hr.lago\"], output=\"hr.resumen\")\ndef otro():\n    return write(\"hr.otro\", over(\"hr.lago\", como=\"arrow\"))\notro()' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='PermissionError' and 'hr.otro' in d['salida']['mensaje']" || falla "10 · escribir fuera del output tenía que ser PermissionError: $(cuerpo)"
  [ ! -f "$A/datasets/hr_otro.json" ] || falla "10 · lo negado dejó puntero"
  celda 'transform(inputs=[\"hr.lago\"], output=\"hr.lago\")' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ValueError'" || falla "10 · input y output iguales: $(cuerpo)"
  [ "$(pide GET /documentos/TrainedModel "$ANA")" = "200" ] && tiene "[x['name'] for x in d['documentos']]==['prevision']" || falla "10 · GET /documentos/TrainedModel: $(cuerpo)"
  dice "10 · write(hr.salida) desde la celda: la tabla por IPC a ore-store-r2 con la credencial prestada, el commit por /v1, el Dataset escrito nace tipado y over() devuelve el mismo JSON que hr.lago · repetida sin snapshot · anexar un DataFrame → 5 y sql lo suma · upsert por clave → 6 (54.75) y el Dataset declara la clave · una View → error · uint64 → ValueError con la columna · declare(View sobre hr.salida) la deja en el árbol, otra vez no es nueva, una rota es ValueError con el OOS y no queda, un Model no se sirve · declare(TrainedModel) con linaje por la View: en el árbol y listado; linaje roto OOS2005; sin digest OOS1004 · la procedencia en el puntero y la ficha (leidas fuera de un transform; inputs + transform dentro); un transform no lee ni escribe fuera de lo declarado (PermissionError)"
  ESCRITO_OK=si
else
  ESCRITO_OK=no
  dice "10 · (sin el lago o sin ore-store-r2: write() no se prueba aquí)"
fi


# ── 11 · el trabajo (W3.7 ④): un fichero del árbol como Job que termina ──────
if [ "$ESCRITO_OK" = si ]; then
  mkdir -p "$A/packages/hr/transforms"
  cat > "$A/packages/hr/transforms/resumir.py" <<'PY'
# Un transform del árbol: lee hr.lago y deja hr.trabajo
@transform(inputs=["hr.lago"], output="hr.trabajo")
def resumir():
    t = over("hr.lago", como="arrow")
    return write("hr.trabajo", t)

e = resumir()
print("filas", e["filas"])
PY
  printf 'raise RuntimeError("se rompe a propósito")\n' > "$A/packages/hr/transforms/roto.py"
  [ "$(pide POST /trabajos "$AG" '{"codigo":"packages/hr/transforms/resumir.py"}')" = "403" ] || falla "11 · un agente lanzó un trabajo: $(cuerpo)"
  [ "$(pide POST /trabajos "$ANA" '{"codigo":"packages/hr/transforms/nadie.py"}')" = "404" ] || falla "11 · un fichero que no está: $(cuerpo)"
  [ "$(pide POST /trabajos "$ANA" '{"codigo":"packages/hr/package.yaml"}')" = "422" ] || falla "11 · un .yaml no es de ningún entorno: $(cuerpo)"
  [ "$(pide POST /trabajos "$ANA" '{"codigo":"../fuera.py"}')" = "422" ] || falla "11 · una ruta fuera del árbol: $(cuerpo)"
  [ "$(pide POST /trabajos "$ANA" '{"codigo":"packages/hr/transforms/resumir.py"}')" = "202" ] || falla "11 · lanzar: $(cuerpo)"
  T=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$TMP/r.json")
  tiene "d['id'].startswith('trabajo-ana-') and d['entorno']=='python' and d['codigo']=='packages/hr/transforms/resumir.py' and d['commit']=='local' and d['trabajo']=='encolado' and d['job'].startswith(d['id']+'-') and d['fichero']=='54-el-trabajo-'+d['id'][8:]+'.yaml'" || falla "11 · la ficha del trabajo: $(cuerpo)"
  F=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["fichero"])' "$TMP/r.json")
  en_cola "$F" | grep -q 'name: TRABAJO, value: "packages/hr/transforms/resumir.py@local"' || falla "11 · el Job no lleva TRABAJO: $(en_cola "$F" | grep -n TRABAJO)"
  en_cola "$F" | grep -q 'image: .*/puesto-python:1' || falla "11 · el Job no lleva la imagen de python"
  [ "$(pide GET /trabajos "$ANA")" = "200" ] && tiene "[t['id'] for t in d['trabajos']]==['$T']" || falla "11 · GET /trabajos: $(cuerpo)"
  [ "$(pide GET /trabajos/$T "$BEA")" = "403" ] || falla "11 · bea ve el trabajo de ana: $(cuerpo)"
  [ "$(pide GET /puestos "$ANA")" = "200" ] && tiene "all(not p['id'].startswith('trabajo-') for p in d['puestos'])" || falla "11 · un trabajo no es un puesto de la lista: $(cuerpo)"
  # el agente, como en el Job: con TRABAJO corre la celda y sale con el resultado
  ORE_SERVE="$BASE" PUESTO="$T" TRABAJO="packages/hr/transforms/resumir.py@local" ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
    "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/trabajo.txt" 2>&1; CODIGO=$?
  [ "$CODIGO" = 0 ] || falla "11 · el agente del trabajo salió con $CODIGO: $(tail -5 "$TMP/trabajo.txt")"
  grep -q "trabajo packages/hr/transforms/resumir.py@local: hecho" "$TMP/trabajo.txt" || falla "11 · el agente no dijo que terminó: $(tail -3 "$TMP/trabajo.txt")"
  [ "$(pide GET /trabajos/$T "$ANA")" = "200" ] && tiene "d['trabajo']=='hecho' and d['estado']=='cerrado' and d['informe']['estado']=='hecho' and d['informe']['fichero']=='trabajos/$T.json' and 'filas 3' in d['informe']['salida']['texto']" || falla "11 · la ficha tras correr: $(cuerpo)"
  "$PY" -c 'import json,sys; i=json.load(open(sys.argv[1])); assert i["codigo"]=="packages/hr/transforms/resumir.py" and i["persona"]=="persona:ana" and i["estado"]=="hecho" and i["ms"]>0, i' "$A/trabajos/$T.json" || falla "11 · el informe en el árbol: $(cat "$A/trabajos/$T.json")"
  "$PY" -c 'import json,sys; pr=json.load(open(sys.argv[1]))["procedencia"]; assert pr=={"codigo":"packages/hr/transforms/resumir.py@local","inputs":["hr.lago"],"puesto":sys.argv[2],"transform":"resumir"}, pr' "$A/datasets/hr_trabajo.json" "$T" || falla "11 · la procedencia de lo que el trabajo escribió: $(cat "$A/datasets/hr_trabajo.json")"
  [ "$(pide GET /datasets/hr/trabajo "$ANA")" = "200" ] && tiene "d['escrito_por']=='persona:ana' and d['procedencia']['codigo']=='packages/hr/transforms/resumir.py@local'" || falla "11 · la ficha del dataset del trabajo: $(cuerpo)"
  en_cola "$F" >/dev/null && falla "11 · el trabajo sigue en la cola tras terminar"
  # uno que se rompe: el agente sale con 1 y el informe dice error
  [ "$(pide POST /trabajos "$ANA" '{"codigo":"packages/hr/transforms/roto.py"}')" = "202" ] || falla "11 · lanzar el roto: $(cuerpo)"
  T2=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$TMP/r.json")
  ORE_SERVE="$BASE" PUESTO="$T2" TRABAJO="packages/hr/transforms/roto.py@local" ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
    "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/trabajo2.txt" 2>&1; CODIGO=$?
  [ "$CODIGO" = 1 ] || falla "11 · el trabajo roto tenía que salir con 1 y salió con $CODIGO"
  [ "$(pide GET /trabajos/$T2 "$ANA")" = "200" ] && tiene "d['trabajo']=='hecho' and d['informe']['estado']=='error' and 'se rompe' in d['informe']['salida']['mensaje']" || falla "11 · el informe del roto: $(cuerpo)"
  [ "$(pide GET /trabajos "$ANA")" = "200" ] && tiene "[t['id'] for t in d['trabajos']]==['$T2','$T']" || falla "11 · los trabajos, del más reciente al más viejo: $(cuerpo)"
  dice "11 · POST /trabajos: 202 trabajo-ana-<hex> con el fichero 54-el-trabajo-… en la cola (TRABAJO=<ruta>@<commit>, la imagen de python); un agente 403, sin fichero 404, un .yaml 422, fuera del árbol 422; el agente con TRABAJO corre la celda y sale 0 → la ficha dice hecho, el informe está en trabajos/<id>.json firmado por ana, el dataset lleva procedencia {codigo, inputs, transform}, y el trabajo sale de la cola; uno roto sale 1 y el informe dice error"
else
  dice "11 · (sin el lago: el trabajo no se prueba aquí)"
fi

# ── 12 · la puerta del puesto (W3.7 gobierno ①): desde un puesto sólo entran los verbos ──
#
# Medido antes: desde una celda, `PUT /arbol/conduits.yaml` era 200 firmado por
# el agente. Lo que decide es el sujeto (agente), no la cabecera: un agente no
# escribe fuera de los verbos —leer, escribir, declarar— ni con `x-ore-puesto`
# ni sin ella. Leer sigue abierto.
P=puesto-ana-python; LEN=python
CRUDO='import json, ore, urllib.request, urllib.error\ndef a_pelo(m, ruta, texto=None, con=True):\n    q = urllib.request.Request(ore.puesto.servidor + ruta, data=(texto or \"\").encode() if texto is not None else None, method=m)\n    [q.add_header(k, v) for k, v in ore.puesto._cabeceras.items()]\n    con and q.add_header(\"x-ore-puesto\", ore.puesto.id)\n    try:\n        return urllib.request.urlopen(q, timeout=30).status\n    except urllib.error.HTTPError as e:\n        return e.code\n'
celda "$CRUDO"'[a_pelo(\"PUT\", \"/arbol/conduits.yaml\", \"x: 1\"), a_pelo(\"PUT\", \"/arbol/conduits.yaml\", \"x: 1\", con=False), a_pelo(\"DELETE\", \"/arbol/packages/hr/tables/empleados_t.yaml\"), a_pelo(\"POST\", \"/ramas\", \"{}\"), a_pelo(\"POST\", \"/propuestas\", \"{}\"), a_pelo(\"POST\", \"/paquetes\", \"{}\"), a_pelo(\"GET\", \"/arbol/conduits.yaml\")]' && tiene "d['salida']['texto']=='[403, 403, 403, 403, 403, 403, 200]'" || falla "12 · la puerta del puesto: $(cuerpo)"
grep -q "^x: 1" "$A/conduits.yaml" 2>/dev/null && falla "12 · el conducto se reescribió desde la celda"
[ -f "$A/packages/hr/tables/empleados_t.yaml" ] || falla "12 · la tabla se retiró desde la celda"
[ "$(pide PUT /arbol/notas/persona.md "$ANA" 'una persona si')" = "201" ] || falla "12 · una persona no escribe en /arbol: $(cuerpo)"
[ "$(pide DELETE /arbol/notas/persona.md "$ANA")" = "200" ] || falla "12 · una persona no retira en /arbol: $(cuerpo)"
if [ "${ESCRITO_OK:-no}" = si ]; then
  # un Dataset se va con su puntero (antes quedaba huérfano)
  [ -f "$A/datasets/hr_otro.json" ] && falla "12 · hr.otro tenía puntero antes de empezar"
  celda 'write(\"hr.huerfano\", over(\"hr.lago\", como=\"arrow\"))[\"filas\"]' && tiene "d['salida']['texto']=='3'" || falla "12 · write(hr.huerfano): $(cuerpo)"
  [ -f "$A/datasets/hr_huerfano.json" ] || falla "12 · hr.huerfano sin puntero"
  celda 'ore.puesto.pedir(\"DELETE\", \"/documentos/Dataset/hr/huerfano\")' && tiene "d['salida']['texto'].startswith('(200,') and \"'puntero': True\" in d['salida']['texto']" || falla "12 · DELETE /documentos/Dataset desde la celda: $(cuerpo)"
  [ ! -f "$A/packages/hr/datasets/huerfano.yaml" ] && [ ! -f "$A/datasets/hr_huerfano.json" ] || falla "12 · el Dataset o su puntero siguen en el árbol"
fi
dice "12 · desde un puesto sólo entran los verbos: PUT/DELETE /arbol, POST /ramas, /propuestas y /paquetes son 403 para el agente (con y sin x-ore-puesto), GET sigue; una persona escribe en /arbol; DELETE /documentos/Dataset retira también el puntero"

# ── 13 · el conducto de la lectura (W3.7 gobierno ②): lo que un dataset lleva, contra contextSurface.workspace ──
#
# Medido antes: un dataset que una Entity clasificaba `high` salía entero por
# `over()` con un conducto de `low`. Ahora `datos_del_puesto` coteja la carga
# del dataset (su raíz y las entidades de su cadena) con
# `contextSurface.workspace` —o con `materialization.payload` si el árbol no
# lo declara— y niega con el OOS.
if [ "${ESCRITO_OK:-no}" = si ]; then
  P=puesto-ana-python; LEN=python
  [ "$(pide PUT /arbol/lattice.yaml "$ANA" 'apiVersion: oos.dev/v1alpha3
kind: Lattice
metadata: { name: sensitivity, namespace: gdpr }
spec:
  levels: [none, low, high]
')" = "201" ] || falla "13 · el reticulo: $(cuerpo)"
  [ "$(pide PUT /arbol/conduits.yaml "$ANA" 'apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:security
  conduits:
    materialization.payload: { oos.maturity: DRAFT, gdpr.sensitivity: low }
')" = "200" ] || falla "13 · el conducto a low: $(cuerpo)"
  celda 'over(\"hr.salida\", como=\"arrow\").num_rows' && tiene "d['salida']['texto'] in ('3','6')" || falla "13 · sin etiqueta, hr.salida se lee: $(cuerpo)"
  celda 'declare(\"apiVersion: oos.dev/v1alpha12\\nkind: View\\nmetadata: { name: salidaV, namespace: hr }\\nspec:\\n  owner: team:hr\\n  from: { dataset: hr.salida }\\n  fields: { n: n, letra: letra, cuando: cuando, importe: importe }\\n\")[\"nueva\"]' && tiene "d['salida']['texto']=='True'" || falla "13 · la View sobre hr.salida: $(cuerpo)"
  celda 'declare(\"apiVersion: oos.dev/v1alpha8\\nkind: Entity\\nmetadata: { name: Salida, namespace: hr }\\nspec:\\n  nature: event\\n  backedBy: hr.salidaV\\n  primaryKey: [n]\\n  timeKey: cuando\\n  properties:\\n    n: { type: Integer }\\n    letra: { type: String }\\n    cuando: { type: DateTimeTz }\\n    importe: { type: Decimal, labels: { gdpr.sensitivity: high } }\\n\")[\"nueva\"]' && tiene "d['salida']['texto']=='True'" || falla "13 · la Entity que clasifica importe high: $(cuerpo)"
  celda 'over(\"hr.salida\", como=\"arrow\").num_rows' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='PermissionError' and 'OOS4002' in d['salida']['mensaje'] and 'materialization.payload' in d['salida']['mensaje'] and 'gdpr.sensitivity:high' in d['salida']['mensaje']" || falla "13 · over(hr.salida) con importe high y el conducto low tenía que ser PermissionError con OOS4002: $(cuerpo)"
  celda 'over(\"hr.salidaV\", como=\"arrow\").num_rows' && tiene "d['salida']['tipo']=='error' and 'OOS4002' in d['salida']['mensaje']" || falla "13 · la View encima tampoco: $(cuerpo)"
  celda 'sql(\"select count(*) n from hr.salida\", como=\"arrow\").num_rows' && tiene "d['salida']['tipo']=='error' and 'OOS4002' in d['salida']['mensaje']" || falla "13 · sql() tampoco: $(cuerpo)"
  [ "$(pide GET /puestos/puesto-ana-python/datos/hr.salida "$AG")" = "403" ] && tiene "d['codigo']=='OOS4002'" || falla "13 · GET datos no dio 403 con el codigo: $(cuerpo)"
  celda 'over(\"hr.lago\", como=\"arrow\").num_rows' && tiene "d['salida']['texto']=='3'" || falla "13 · hr.lago, sin etiqueta, se sigue leyendo: $(cuerpo)"
  # el árbol (una persona, no el puesto) declara por dónde sale hacia el código
  [ "$(pide PUT /arbol/conduits.yaml "$ANA" 'apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:security
  conduits:
    materialization.payload: { oos.maturity: DRAFT, gdpr.sensitivity: low }
    contextSurface.workspace: { oos.maturity: DRAFT, gdpr.sensitivity: high }
')" = "200" ] || falla "13 · contextSurface.workspace a high: $(cuerpo)"
  celda 'over(\"hr.salida\", como=\"arrow\").num_rows' && tiene "d['salida']['texto'] in ('3','6')" || falla "13 · con contextSurface.workspace high, hr.salida se lee: $(cuerpo)"
  [ "$(pide GET /puestos/puesto-ana-python/datos/hr.salida "$AG")" = "200" ] && tiene "d['clasificacion']=={'gdpr.sensitivity':'high'} and d['dataset']=='hr.salida' and 's3.access-key-id' in d.get('credencial', {})" || falla "13 · datos no trae la clasificacion, el dataset y la credencial de lectura (2b): $(cuerpo)"
  dice "13 · el conducto de la lectura: sin etiqueta se lee; con la Entity que clasifica importe high y materialization.payload low, over()/sql() son PermissionError OOS4002 y GET datos 403 con el codigo; contextSurface.workspace high por el arbol lo abre, y datos trae la clasificacion"

  # ── 14 · lo escrito lleva lo que leyó (W3.7 gobierno ③, OOS 01-dataset §5) ──
  #
  # Medido antes: la etiqueta moría en write(). Ahora el Dataset escrito dice
  # `derivedFrom` (de la procedencia: `leidas` sin él mismo, o los `inputs` del
  # transform), cada columna lleva el join de lo leído, y baja por la cadena.
  celda 'e = write(\"hr.derivado\", over(\"hr.salida\", como=\"arrow\")); e[\"filas\"]' && tiene "d['salida']['texto'] in ('3','6')" || falla "14 · write(hr.derivado): $(cuerpo)"
  grep -E "derivedFrom: \[.*hr\.salida.*\]" "$A/packages/hr/datasets/derivado.yaml" | grep -qv "hr.derivado" || falla "14 · el documento no dice derivedFrom con hr.salida (la sesion entera, sin el mismo): $(cat "$A/packages/hr/datasets/derivado.yaml")"
  [ "$(pide GET /puestos/puesto-ana-python/datos/hr.derivado "$AG")" = "200" ] && tiene "d['clasificacion']=={'gdpr.sensitivity':'high'}" || falla "14 · hr.derivado no lleva high: $(cuerpo)"
  # una copia mantenida de lo derivado sale por materialization.payload (low): OOS4002, y no queda
  celda 'declare(\"apiVersion: oos.dev/v1alpha12\\nkind: Dataset\\nmetadata: { name: derivadoCopia, namespace: hr }\\nspec:\\n  owner: team:hr\\n  from: { dataset: hr.derivado }\\n\")' && tiene "d['salida']['tipo']=='error' and 'OOS4002' in d['salida']['mensaje'] and 'gdpr.sensitivity:high' in d['salida']['mensaje']" || falla "14 · la copia de lo derivado tenía que ser OOS4002: $(cuerpo)"
  [ ! -f "$A/packages/hr/datasets/derivadoCopia.yaml" ] || falla "14 · la copia negada quedó en el árbol"
  # fuera de un transform es la sesion entera (sobreaproximar es P4), y nunca el mismo
  celda 'e = write(\"hr.derivado\", over(\"hr.derivado\", como=\"arrow\"), modo=\"anexar\"); e[\"filas\"]' && tiene "d['salida']['texto'] in ('6','12')" || falla "14 · anexar hr.derivado a sí mismo: $(cuerpo)"
  grep -q "hr.derivado" "$A/packages/hr/datasets/derivado.yaml" && grep "derivedFrom" "$A/packages/hr/datasets/derivado.yaml" | grep -q "hr.derivado" && falla "14 · anexar de sí mismo no puede nombrarse: $(grep derivedFrom "$A/packages/hr/datasets/derivado.yaml")"
  # dentro de un transform, derivedFrom son los inputs
  celda '@transform(inputs=[\"hr.salida\"], output=\"hr.derivadoT\")\ndef t():\n    return write(\"hr.derivadoT\", over(\"hr.salida\", como=\"arrow\"))\nt()[\"filas\"]' && tiene "d['salida']['texto'] in ('3','6')" || falla "14 · el transform: $(cuerpo)"
  grep -q "derivedFrom: \[hr.salida\]" "$A/packages/hr/datasets/derivadoT.yaml" || falla "14 · derivedFrom del transform: $(grep derivedFrom "$A/packages/hr/datasets/derivadoT.yaml")"
  dice "14 · lo escrito lleva lo que leyó: hr.derivado dice derivedFrom (la sesion entera, con hr.salida) y lleva high; la copia mantenida encima es OOS4002 por materialization.payload low; anexar de sí mismo no se nombra, y un transform deja exactamente sus inputs"
fi

# ── 15 · lo escrito es de quien lo escribió (W3.7 gobierno ④) ─────────────────
#
# Medido antes: bob anexaba, sobrescribía y retiraba lo de ana con 200. Ahora la
# credencial para escribir (`--prestar`), el commit y retirar el Dataset son de
# la persona del puntero (`escrito_por`); otra recibe 403 con quién. Aquí como
# personas, contra el catálogo: es la misma puerta que el SDK usa.
if [ "${ESCRITO_OK:-no}" = si ]; then
  grep -q '"escrito_por": "persona:ana"' "$A/datasets/hr_derivadoT.json" || grep -q 'escrito_por.*persona:ana' "$A/datasets/hr_derivadoT.json" || falla "15 · hr.derivadoT no dice escrito_por ana: $(cat "$A/datasets/hr_derivadoT.json")"
  [ "$(pide GET /v1/namespaces/hr/tables/derivadoT "$BEA")" = "200" ] || falla "15 · bea no puede ni cargar la tabla sin pedir credencial: $(cuerpo)"
  DELEGAR='x-iceberg-access-delegation: vended-credentials'
  CODIGO=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$BEA" -H "$DELEGAR" "$BASE/v1/namespaces/hr/tables/derivadoT")
  [ "$CODIGO" = "403" ] && grep -q "persona:ana" "$TMP/r.json" && grep -q "ForbiddenException" "$TMP/r.json" || falla "15 · la credencial para escribir lo de ana se le prestó a bea ($CODIGO): $(cuerpo)"
  CODIGO=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$ANA" -H "$DELEGAR" "$BASE/v1/namespaces/hr/tables/derivadoT")
  [ "$CODIGO" = "200" ] || falla "15 · a ana sí se le presta ($CODIGO): $(cuerpo)"
  [ "$(pide POST /v1/namespaces/hr/tables/derivadoT "$BEA" '{"identifier":{"namespace":["hr"],"name":"derivadoT"},"requirements":[],"updates":[]}')" = "403" ] && grep -q "persona:ana" "$TMP/r.json" || falla "15 · el commit de bea sobre lo de ana no dio 403: $(cuerpo)"
  [ "$(pide DELETE /documentos/Dataset/hr/derivadoT "$BEA")" = "403" ] && grep -q "persona:ana" "$TMP/r.json" || falla "15 · bea retiró el dataset de ana: $(cuerpo)"
  [ -f "$A/packages/hr/datasets/derivadoT.yaml" ] || falla "15 · el dataset de ana se fue"
  [ "$(pide DELETE /documentos/Dataset/hr/derivadoT "$ANA")" = "200" ] || falla "15 · ana no pudo retirar lo suyo: $(cuerpo)"
  [ ! -f "$A/packages/hr/datasets/derivadoT.yaml" ] && [ ! -f "$A/datasets/hr_derivadoT.json" ] || falla "15 · lo de ana no se retiró entero"
  dice "15 · lo escrito es de quien lo escribió: a bea no se le presta la credencial de hr.derivadoT (403 con persona:ana), su commit es 403 y no lo retira; ana sí"
fi

# ── 16 · lo declarado, en el servidor (W3.7 gobierno ⑤) ──────────────────────
#
# Medido antes: dentro de `@transform(inputs, output)`, `over()` de lo no
# declarado era PermissionError del SDK, pero `ore.puesto.pedir()` a pelo era
# 200 y `PUT /arbol` desde dentro, 201. Ahora el SDK declara al servidor
# (`POST /puestos/{id}/transform`) y el servidor acota: `datos` sólo resuelve
# `inputs`, el catálogo sólo el `output`.
if [ "${ESCRITO_OK:-no}" = si ]; then
  P=puesto-ana-python; LEN=python
  A_PELO='import json, ore\ndef a_pelo(m, ruta):\n    c, r = ore.puesto.pedir(m, ruta)\n    return c\n'
  celda "$A_PELO"'print(json.dumps([a_pelo(\"GET\", \"/puestos/\" + ore.puesto.id + \"/datos/hr.lago\"), a_pelo(\"GET\", \"/puestos/\" + ore.puesto.id + \"/datos/hr.salida\")]))' && tiene "d['salida']['texto'].strip()=='[200, 200]'" || falla "16 · fuera de un transform se resuelve todo: $(cuerpo)"
  celda "$A_PELO"'@transform(inputs=[\"hr.lago\"], output=\"hr.declarado\")\ndef t():\n    c1, _ = ore.puesto.pedir(\"GET\", \"/puestos/\" + ore.puesto.id)\n    return [a_pelo(\"GET\", \"/puestos/\" + ore.puesto.id + \"/datos/hr.lago\"), a_pelo(\"GET\", \"/puestos/\" + ore.puesto.id + \"/datos/hr.salida\"), a_pelo(\"GET\", \"/v1/namespaces/hr/tables/salida\"), a_pelo(\"GET\", \"/v1/namespaces/hr/tables/declarado\")]\nprint(json.dumps(t()))' && tiene "d['salida']['texto'].strip()=='[200, 403, 403, 404]'" || falla "16 · dentro del transform el servidor no acotó (se esperaba [200, 403, 403, 404]): $(cuerpo)"
  celda 'import json; c, r = ore.puesto.pedir(\"GET\", \"/puestos/\" + ore.puesto.id); print(json.dumps([c, r.get(\"transform\"), r.get(\"inputs\")]))' && tiene "d['salida']['texto'].strip()=='[200, null, null]'" || falla "16 · el transform no se retiró al salir: $(cuerpo)"
  celda "$A_PELO"'print(json.dumps([a_pelo(\"GET\", \"/puestos/\" + ore.puesto.id + \"/datos/hr.salida\")]))' && tiene "d['salida']['texto'].strip()=='[200]'" || falla "16 · fuera del transform se vuelve a resolver: $(cuerpo)"
  celda '@transform(inputs=[\"hr.lago\"], output=\"hr.declarado\")\ndef t2():\n    return write(\"hr.declarado\", over(\"hr.lago\", como=\"arrow\"))\nt2()[\"filas\"]' && tiene "d['salida']['texto']=='3'" || falla "16 · el transform declarado sí escribe lo suyo: $(cuerpo)"
  [ -f "$A/datasets/hr_declarado.json" ] || falla "16 · hr.declarado no quedó"
  dice "16 · lo declarado, en el servidor: dentro de @transform, GET datos de lo no declarado es 403 y el catálogo de otra tabla también (a pelo, rodeando el SDK); GET /puestos lo dice; al salir se retira y todo vuelve a resolverse; lo declarado se escribe"
fi

# ── 17 · el techo de la clase (0036 ⑤) ──────────────────────────────────────
#
# Un repositorio `analytics` NO escribe datos, aunque su código lo declare: el
# techo se aplica donde ya se decide quién escribe —el catálogo—, y no en el
# SDK, que se rodea pidiendo a pelo. Y un `semantics` ni siquiera abre sesión.
if [ "${ESCRITO_OK:-no}" = si ]; then
  [ "$(pide POST /repositorios "$ANA" '{"paquete":"hr","carpeta":"mirar","nombre":"Mirar","plantilla":"analytics"}')" = "201" ] \
    || falla "17 · crear el repositorio analytics: $(cuerpo)"
  [ "$(pide POST /repositorios "$ANA" '{"paquete":"hr","carpeta":"semantica","nombre":"Semantica","plantilla":"semantics"}')" = "201" ] \
    || falla "17 · crear el repositorio semantics: $(cuerpo)"
  # el que no ejecuta no abre sesión
  [ "$(pide POST /puestos "$ANA" '{"lenguaje":"python","repositorio":"packages/hr/semantica"}')" = "422" ] \
    && grep -q "no ejecuta" "$TMP/r.json" || falla "17 · un semantics abrió puesto: $(cuerpo)"
  # el analytics abre, y su ficha lo dice
  [ "$(pide POST /puestos "$ANA" '{"lenguaje":"python","repositorio":"packages/hr/mirar"}')" = "201" ] \
    || falla "17 · abrir el puesto del analytics: $(cuerpo)"
  tiene "d['id']=='puesto-ana-python-mirar' and d['plantilla']=='analytics-python' and d['escribe'] is False and d['repositorio']=='packages/hr/mirar'" \
    || falla "17 · la ficha del puesto no dice su clase: $(cuerpo)"
  ORE_SERVE="$BASE" PUESTO=puesto-ana-python-mirar ORE_SUJETO=agente:mirar ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
    "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/agente-mirar.txt" 2>&1 &
  AGENTE2=$!
  for _ in $(seq 1 40); do pide GET /puestos/puesto-ana-python-mirar "$ANA" >/dev/null; tiene "d['estado']=='vivo'" && break; sleep 0.25; done
  tiene "d['estado']=='vivo'" || falla "17 · el puesto del analytics no pasa a vivo: $(cuerpo)"
  P=puesto-ana-python-mirar; LEN=python
  # leer, sí
  celda 'import json; c, _ = ore.puesto.pedir(\"GET\", \"/puestos/\" + ore.puesto.id + \"/datos/hr.lago\"); print(json.dumps(c))' \
    && tiene "d['salida']['texto'].strip()=='200'" || falla "17 · un analytics no pudo LEER: $(cuerpo)"
  # escribir, no: ni por el SDK ni a pelo, ni aunque lo declare un transform
  celda 'import json; c, _ = ore.puesto.pedir(\"POST\", \"/v1/namespaces/hr/tables\", {\"name\": \"prohibida\"}); print(json.dumps(c))' \
    && tiene "d['salida']['texto'].strip()=='403'" || falla "17 · el catálogo dejó crear una tabla desde un analytics: $(cuerpo)"
  celda '@transform(inputs=[\"hr.lago\"], output=\"hr.prohibida\")\ndef t():\n    return write(\"hr.prohibida\", over(\"hr.lago\", como=\"arrow\"))\ntry:\n    t()\n    print(\"ESCRIBIO\")\nexcept Exception as e:\n    print(type(e).__name__)' \
    && tiene "'ESCRIBIO' not in d['salida'].get('texto','') and d['salida']['tipo'] in ('texto','error')" \
    || falla "17 · un analytics escribió aunque el transform lo declaraba: $(cuerpo)"
  [ ! -f "$A/datasets/hr_prohibida.json" ] || falla "17 · quedó el puntero de lo que no se podía escribir"
  [ "$(pide DELETE /puestos/puesto-ana-python-mirar "$ANA")" = "200" ] || falla "17 · cerrar el puesto del analytics: $(cuerpo)"
  for _ in $(seq 1 100); do kill -0 "$AGENTE2" 2>/dev/null || break; sleep 0.25; done
  AGENTE2=""
  P=puesto-ana-python; LEN=python
  dice "17 · el techo de la clase: un semantics no abre sesión (422); un analytics abre, LEE (200) y NO escribe —el catálogo es 403 a pelo y el transform que lo declara tampoco escribe—, y no queda puntero"
fi

# ── 5 · cerrar ─────────────────────────────────────────────────────────────
[ "$(pide DELETE /puestos/puesto-ana-python "$BEA")" = "403" ] || falla "5 · bea cerro el puesto de ana"
[ "$(pide DELETE /puestos/puesto-ana-python "$ANA")" = "200" ] && tiene "d['estado']=='cerrado' and 'fuera de la cola' in d['cola']" || falla "5 · cerrar: $(cuerpo)"
en_cola 51-el-puesto-ana-python.yaml >/dev/null && falla "5 · el fichero sigue en la cola"
for _ in $(seq 1 100); do kill -0 "$AGENTE" 2>/dev/null || break; sleep 0.25; done
kill -0 "$AGENTE" 2>/dev/null && falla "5 · el agente no se cerro al 410"
AGENTE=""
grep -q "cerrado" "$TMP/agente.txt" || falla "5 · el agente no dijo por que se fue: $(tail -3 "$TMP/agente.txt")"
[ "$(pide POST /puestos/puesto-ana-python/ejecutar "$ANA" '{"texto":"1"}')" = "410" ] || falla "5 · una celda tras cerrar no dio 410: $(cuerpo)"
[ "$(pide POST /puestos "$ANA" '{}')" = "201" ] || falla "5 · abrir de nuevo tras cerrar: $(cuerpo)"
dice "5 · DELETE: 200, el fichero fuera de la cola, el agente se cierra al 410, una celda mas → 410, y se puede abrir otro"

# ── 6 · la capa (W3.2): lo que el árbol declara, resuelto antes del puesto ──
[ "$(pide GET /entorno "$ANA")" = "200" ] && tiene "d['estado']=='sin-dependencias' and d['declarado']==[] and d['digest']==''" || falla "6 · entorno sin dependencias: $(cuerpo)"
[ "$(pide POST /entorno "$ANA")" = "422" ] || falla "6 · resolver sin dependencias no dio 422: $(cuerpo)"
printf '[project]
name = "hr"
dependencies = [
  "polars>=1.40",  # rapido
  "duckdb",
]
' > "$A/packages/hr/pyproject.toml"
printf '[project]
dependencies = ["polars>=1.40"]
' > "$A/pyproject.toml"
[ "$(pide GET /entorno "$ANA")" = "200" ] && tiene "d['estado']=='pendiente' and d['declarado']==['duckdb','polars>=1.40'] and d['digest'].startswith('capa-') and len(d['digest'])==17" || falla "6 · entorno pendiente: $(cuerpo)"
DIGEST=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["digest"])' "$TMP/r.json")
# abrir un puesto con la capa pendiente: 409, y la capa encolada
[ "$(pide POST /puestos "$BEA" '{}')" = "409" ] || falla "6 · abrir con la capa pendiente no dio 409: $(cuerpo)"
tiene "d['capa']=='$DIGEST' and 'Job la-capa-' in d['cola'] and d['entorno']['estado']=='pendiente'" || falla "6 · el 409 no dice la capa ni el Job: $(cuerpo)"
CORTO=${DIGEST#capa-}
en_cola "52-la-capa-$CORTO.yaml" | grep -q "name: CAPA, value: \"$DIGEST\"" || falla "6 · el Job de la capa no esta en la cola con el digest"
en_cola "52-la-capa-$CORTO.yaml" | grep -q 'ore.dev/rol: driver' || falla "6 · el Job de la capa no lleva el rol driver (PyPI)"
[ "$(pide POST /entorno "$ANA")" = "202" ] && tiene "d['job'].startswith('la-capa-$CORTO-') and 'ya encolada' in d['cola']" || falla "6 · POST /entorno pendiente no dio 202 la misma: $(cuerpo)"
en_cola "51-el-puesto-bea-python.yaml" >/dev/null && falla "6 · el puesto de bea se encolo sin capa"
# el informe (lo que 52-la-capa deja en el arbol): la capa esta lista
mkdir -p "$A/entorno"
"$PY" -c 'import json,sys; json.dump({"estado":"lista","digest":sys.argv[1],"declarado":["duckdb","polars>=1.40"],"ruedas":["polars-1.44.2-py3-none-any.whl","duckdb-1.5.5-cp312-abi3-manylinux_2_17_x86_64.whl"],"mb":"55","cuando":"2026-09-19T00:00:00Z"}, open(sys.argv[2],"w"))' "$DIGEST" "$A/entorno/python.json"
[ "$(pide GET /entorno "$ANA")" = "200" ] && tiene "d['estado']=='lista' and len(d['informe']['ruedas'])==2" || falla "6 · entorno lista: $(cuerpo)"
[ "$(pide POST /entorno "$ANA")" = "200" ] || falla "6 · resolver con la capa lista no dio 200: $(cuerpo)"
[ "$(pide POST /puestos "$BEA" '{}')" = "201" ] && tiene "d['id']=='puesto-bea-python'" || falla "6 · abrir con la capa lista: $(cuerpo)"
en_cola 51-el-puesto-bea-python.yaml | grep -q "name: CAPA, value: \"$DIGEST\"" || falla "6 · el puesto de bea no lleva la capa: $(en_cola 51-el-puesto-bea-python.yaml | grep -n CAPA)"
en_cola 51-el-puesto-bea-python.yaml | grep -q 'name: PYTHONPATH, value: /capa' || falla "6 · el puesto no pone /capa en el PYTHONPATH"
# cambia la declaracion: la capa vuelve a estar pendiente
printf '[project]
dependencies = ["polars>=1.40", "scikit-learn"]
' > "$A/pyproject.toml"
[ "$(pide GET /entorno "$ANA")" = "200" ] && tiene "d['estado']=='pendiente' and d['digest']!='$DIGEST'" || falla "6 · otra declaracion no vuelve a pendiente: $(cuerpo)"
pide DELETE /puestos/puesto-bea-python "$BEA" >/dev/null
dice "6 · la capa: sin dependencias · un pyproject → pendiente (capa-<12 hex>) · abrir → 409 y el Job de la capa en la cola (rol driver) · POST /entorno 202 la misma · informe lista → lista, 200, y el puesto nace con la capa y /capa en el PYTHONPATH · otra declaracion → pendiente"


# ── 6b · la capa de la JVM (0037 ③c): el mismo camino, otro fichero ───────
#
# ⭐ Lo que se prueba aquí no es Maven —eso lo prueba la imagen al construirse—
#   sino que el servidor sabe LEER lo que un repositorio de Java declara, que
#   su capa NO es la de Python aunque el árbol tenga las dos, y que el Job que
#   encola es el suyo.
[ "$(pide GET /entorno/jvm "$ANA")" = "200" ] && tiene "d['estado']=='sin-dependencias' and d['entorno']=='jvm' and d['declarado']==[]" || falla "6b · entorno jvm sin dependencias: $(cuerpo)"
[ "$(pide GET /entorno/node "$ANA")" = "404" ] || falla "6b · /entorno/node deberia ser 404 (node nace con su imagen): $(cuerpo)"
# Un pom con de todo: lo que cuenta y lo que NO se honra.
printf '<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <!-- <dependency> en un comentario no es una dependencia -->
  <dependencyManagement><dependencies><dependency>
    <groupId>no.entra</groupId><artifactId>gestionada</artifactId><version>1.0</version>
  </dependency></dependencies></dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.apache.commons</groupId><artifactId>commons-lang3</artifactId><version>3.17.0</version>
    </dependency>
    <dependency>
      <groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId>
      <version>5.11.0</version><scope>test</scope>
    </dependency>
    <dependency><groupId>sin.version</groupId><artifactId>quien-sabe</artifactId></dependency>
  </dependencies>
</project>
' > "$A/packages/hr/pom.xml"
[ "$(pide GET /entorno/jvm "$ANA")" = "200" ] && tiene "d['estado']=='pendiente' and d['declarado']==['org.apache.commons:commons-lang3:3.17.0'] and d['digest'].startswith('capa-') and len(d['digest'])==17" || falla "6b · el pom no se leyo como se escribio: $(cuerpo)"
JVM_DIGEST=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["digest"])' "$TMP/r.json")
# ⭐ Y NO es la capa de Python: el mismo árbol, dos lenguajes, dos capas.
pide GET /entorno "$ANA" >/dev/null
tiene "d['digest']!='$JVM_DIGEST'" || falla "6b · la capa de la JVM y la de Python comparten digest"
JVM_CORTO=${JVM_DIGEST#capa-}
[ "$(pide POST /entorno/jvm "$ANA")" = "202" ] && tiene "d['job'].startswith('la-capa-jvm-$JVM_CORTO-')" || falla "6b · POST /entorno/jvm no dio 202 con su Job: $(cuerpo)"
en_cola "55-la-capa-jvm-$JVM_CORTO.yaml" | grep -q "name: CAPA, value: \"$JVM_DIGEST\"" || falla "6b · el Job de la capa de la JVM no esta en la cola con el digest"
en_cola "55-la-capa-jvm-$JVM_CORTO.yaml" | grep -q 'ore.dev/rol: driver' || falla "6b · el Job no lleva el rol driver (Maven Central)"
en_cola "55-la-capa-jvm-$JVM_CORTO.yaml" | grep -q '/capa-jvm:1' || falla "6b · el Job no resuelve con la imagen capa-jvm:1"
en_cola "55-la-capa-jvm-$JVM_CORTO.yaml" | grep -q 'name: TOPE_MB' || falla "6b · el Job no lleva tope de tamano"
# ⭐ Y abrir un puesto de Java con la capa pendiente ESPERA, igual que el de
#   Python desde W3.2: si naciera sin ella, la primera celda no compilaria y
#   nadie sabria por que.
[ "$(pide POST /puestos "$BEA" '{"lenguaje":"java"}')" = "409" ] || falla "6b · abrir jvm con la capa pendiente no dio 409: $(cuerpo)"
tiene "d['capa']=='$JVM_DIGEST' and 'Job la-capa-jvm-' in d['cola']" || falla "6b · el 409 del puesto jvm no dice la capa ni su Job: $(cuerpo)"
# el informe que 55-la-capa-jvm deja en el arbol: uno POR DIGEST
mkdir -p "$A/entorno"
# ⭐ Con UN AVISO dentro (0037 iii.c · d): la capa esta LISTA y ademas dice
#   que version se quedo fuera. Ese es el viaje que la consola pinta.
AVISO="pediste com.fasterxml.jackson.core:jackson-databind 2.19.0, y esta sesion trae la 2.18.2: gana la de la sesion"
"$PY" -c 'import json,sys; json.dump({"estado":"lista","digest":sys.argv[1],"declarado":["org.apache.commons:commons-lang3:3.17.0"],"jars":["commons-lang3-3.17.0.jar"],"lock":["org.apache.commons:commons-lang3:3.17.0"],"mb":"1","avisos":[sys.argv[3]],"cuando":"2026-09-23T00:00:00Z","entorno":"puesto-jvm:1"}, open(sys.argv[2],"w"))' "$JVM_DIGEST" "$A/entorno/$JVM_DIGEST.json" "$AVISO"
[ "$(pide GET /entorno/jvm "$ANA")" = "200" ] && tiene "d['estado']=='lista' and d['informe']['jars']==['commons-lang3-3.17.0.jar']" || falla "6b · el informe de la JVM no puso la capa lista: $(cuerpo)"
# el aviso llega ENTERO y sin interpretar: la capa esta lista, no es un error
tiene "d['informe']['avisos']==['$AVISO'] and d['estado']=='lista'" || falla "6b · el aviso del choque no llego a quien lo tiene que pintar: $(cuerpo)"
[ "$(pide POST /entorno/jvm "$ANA")" = "200" ] || falla "6b · resolver con la capa de la JVM lista no dio 200: $(cuerpo)"
# y con la capa lista, el puesto nace CON ella y quien la baja sabe que son jars
[ "$(pide POST /puestos "$BEA" '{"lenguaje":"java"}')" = "201" ] && tiene "d['id']=='puesto-bea-jvm'" || falla "6b · abrir jvm con la capa lista: $(cuerpo)"
en_cola 51-el-puesto-bea-jvm.yaml | grep -q "name: CAPA, value: \"$JVM_DIGEST\"" || falla "6b · el puesto jvm de bea no lleva la capa"
en_cola 51-el-puesto-bea-jvm.yaml | grep -q 'name: ENTORNO, value: "jvm"' || falla "6b · traer-la-capa no sabe que la capa es de la JVM (bajaria ruedas)"
pide DELETE /puestos/puesto-bea-jvm "$BEA" >/dev/null
# y la de Python sigue pendiente: el informe de uno no vale para el otro
[ "$(pide GET /entorno "$ANA")" = "200" ] && tiene "d['estado']=='pendiente'" || falla "6b · el informe de la JVM se colo como el de Python: $(cuerpo)"
rm -f "$A/packages/hr/pom.xml"
dice "6b · la capa de la JVM: /entorno/jvm sin dependencias · un pom.xml → pendiente con SU digest (y sin lo que no se honra: dependencyManagement, test, sin version) · POST → 202 y 55-la-capa-jvm-<corto>.yaml con capa-jvm:1 y su tope · abrir jvm con la capa pendiente → 409 · informe → lista CON SU AVISO del choque (pediste X, esta sesion trae Y), el puesto nace con ella (ENTORNO=jvm: jars, no ruedas), y la de Python sigue pendiente"

# ── 8 · TS en el puesto node (W3.4): el agente de Node, celdas TS, un módulo del árbol, persona() ──
NODE=$(command -v node || true)
NODE_OK=$("$NODE" -e 'const [a,b]=process.versions.node.split(".").map(Number); process.stdout.write(a>22||(a==22&&b>=13)?"si":"no")' 2>/dev/null || echo no)
if [ "$NODE_OK" = "si" ]; then
  # El SDK resuelve `@duckdb/node-api` desde donde esta: se copia `puesto/node` y se instala ahi.
  mkdir -p "$TMP/node" && cp -r "$RAIZ/puesto/node/." "$TMP/node/"
  ( cd "$TMP/node" && npm install --no-audit --no-fund --silent @duckdb/node-api >"$TMP/npm.txt" 2>&1 ) || falla "8 · npm install @duckdb/node-api: $(tail -5 "$TMP/npm.txt")"
  [ "$(pide POST /puestos "$ANA" '{"lenguaje":"typescript"}')" = "201" ] || falla "8 · abrir node: $(cuerpo)"
  tiene "d['id']=='puesto-ana-node' and d['entorno']=='node' and d['fichero']=='51-el-puesto-ana-node.yaml'" || falla "8 · la ficha node: $(cuerpo)"
  en_cola 51-el-puesto-ana-node.yaml | grep -q 'image: .*/puesto-node:1' || falla "8 · el Job no lleva puesto-node:1"
  [ "$(pide POST /puestos "$ANA" '{"lenguaje":"javascript"}')" = "200" ] && tiene "d['id']=='puesto-ana-node'" || falla "8 · javascript no es el mismo puesto node: $(cuerpo)"
  pide GET /puestos "$ANA" >/dev/null; tiene "sorted(p['id'] for p in d['puestos'] if p['estado']!='cerrado')==['puesto-ana-node','puesto-ana-python']" || falla "8 · GET /puestos no lista los dos: $(cuerpo)"
  ORE_SERVE="$BASE" PUESTO=puesto-ana-node ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" ORE_CELDAS="$TMP/node-trabajo" TTL=600 \
    "$NODE" --no-warnings "$TMP/node/agente.mjs" >"$TMP/agente.txt" 2>&1 &
  AGENTE=$!
  for _ in $(seq 1 60); do pide GET /puestos/puesto-ana-node "$ANA" >/dev/null; tiene "d['estado']=='vivo'" && break; sleep 0.25; done
  tiene "d['estado']=='vivo'" || falla "8 · el puesto node no pasa a vivo: $(cuerpo)"
  P=puesto-ana-node; LEN=typescript
  celda '1 + 1' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='2'" || falla "8 · 1+1: $(cuerpo)"
  celda 'const x: number = 3' && tiene "d['salida']['tipo']=='vacia'" || falla "8 · const x: number (tipos fuera): $(cuerpo)"
  celda 'x * 2' && tiene "d['salida']['texto']=='6'" || falla "8 · x * 2 (el contexto dura): $(cuerpo)"
  celda 'console.log(\"hola\", x)' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='hola 3\n'" || falla "8 · console.log: $(cuerpo)"
  celda 'noExiste + 1' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ReferenceError'" || falla "8 · ReferenceError: $(cuerpo)"
  celda 'const y = await Promise.resolve(41); y + 1' && tiene "d['salida']['texto']=='42'" || falla "8 · await arriba: $(cuerpo)"
  celda 'interface C { pais: string }\nconst cs: C[] = [{ pais: \"ES\" }, { pais: \"PT\" }]\ncs.filter(c => c.pais === \"ES\").length' && tiene "d['salida']['texto']=='1'" || falla "8 · interface + filter: $(cuerpo)"
  celda 'export function saludo(n: string): string { return \"hola \" + n }' && tiene "d['salida']['tipo']=='texto' and 'saludo' in d['salida']['texto']" || falla "8 · un modulo (export) del arbol: $(cuerpo)"
  celda 'saludo(persona())' && tiene "d['salida']['texto']=='hola persona:ana'" || falla "8 · saludo(persona()) — la funcion del arbol con la identidad de la persona: $(cuerpo)"
  celda 'const df = await over(\"hr.espanoles\"); df' && tiene "d['salida']['tipo']=='tabla' and [c['name'] for c in d['salida']['columnas']]==['id','pais'] and d['salida']['filas']==[['e1','ES'],['e2','ES'],['e3','ES']] and d['salida']['total']==3" || falla "8 · over(hr.espanoles): $(cuerpo)"
  if [ "$LAGO_OK" = "si" ]; then
    celda 'await over(\"hr.lago\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "8 · over(hr.lago), el dataset Iceberg: $(cuerpo)"
    celda 'await sql(\"select count(*) as n, sum(importe) as s from hr.lago\")' && tiene "d['salida']['filas']==[[3,'3.75']]" || falla "8 · sql sobre el dataset Iceberg: $(cuerpo)"
  fi
  if [ "${ESCRITO_OK:-no}" = "si" ]; then
    # write() desde Node: lo que Python escribió (5 filas), leído; y lo suyo, escrito y leído
    celda 'const s = await over(\"hr.salida\"); s.length' && tiene "d['salida']['texto']=='6'" || falla "8 · Node lee lo que Python escribió: $(cuerpo)"
    celda 'const e = await write(\"hr.salida_node\", await over(\"hr.lago\")); [e.filas, e.repetida]' && tiene "d['salida']['texto']=='[ 3, false ]'" && grep -Eq "derivedFrom: \[.*hr\.lago.*\]" "$A/packages/hr/datasets/salida_node.yaml" || falla "8 · write(hr.salida_node) desde filas: $(cuerpo)"
    celda 'await over(\"hr.salida_node\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "8 · over(hr.salida_node) no es el mismo JSON que hr.lago: $(cuerpo)"
    celda 'const e2 = await write(\"hr.salida_node\", await over(\"hr.lago\", { como: \"columnas\" })); e2.repetida' && tiene "d['salida']['texto']=='true'" || falla "8 · la misma escritura (por columnas) tenía que ser repetida: $(cuerpo)"
    celda 'const e3 = await write(\"hr.salida_node\", [{ n: 4, letra: \"d\", cuando: new Date(\"2024-06-02T00:00:00Z\"), importe: 4 }], { modo: \"anexar\" }); e3.filas' && tiene "d['salida']['texto']=='4'" || falla "8 · anexar objetos JS: $(cuerpo)"
    celda 'await sql(\"select count(*) as n, sum(importe) as s from hr.salida_node\")' && tiene "d['salida']['filas']==[[4,'7.75']]" || falla "8 · sql sobre lo escrito desde Node: $(cuerpo)"
    celda 'const e4 = await write(\"hr.salida_node\", [{ n: 4, letra: \"D\", cuando: new Date(\"2024-06-03T00:00:00Z\"), importe: 40 }, { n: 6, letra: \"f\", cuando: null, importe: 6 }], { modo: \"upsert\", clave: [\"n\"] }); e4.filas' && tiene "d['salida']['texto']=='5'" || falla "8 · upsert desde Node: $(cuerpo)"
    celda 'await sql(\"select count(*) as n, sum(importe) as s from hr.salida_node\")' && tiene "d['salida']['filas']==[[5,'49.75']]" || falla "8 · sql tras el upsert desde Node: $(cuerpo)"
    celda 'await write(\"hr.empleados\", [{ a: 1 }])' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "8 · escribir una View desde Node: $(cuerpo)"
    celda 'const dv = await declare(\"apiVersion: oos.dev/v1alpha12\\nkind: View\\nmetadata: { name: porLetraNode, namespace: hr }\\nspec:\\n  owner: team:hr\\n  from: { dataset: hr.salida_node }\\n  fields: { letra: letra, n: \\\"count()\\\" }\\n  groupBy: [letra]\\n\"); [dv.kind, dv.nombre, dv.nueva].join(\" \")' && tiene "d['salida']['texto']=='View hr.porLetraNode true'" || falla "8 · declare(View) desde Node: $(cuerpo)"
    [ -f "$A/packages/hr/views/porLetraNode.yaml" ] || falla "8 · la View declarada desde Node no está en el árbol"
    celda 'const resumirNode = transform({ inputs: [\"hr.lago\"], output: \"hr.resumen_node\" }, async function resumirNode() { return write(\"hr.resumen_node\", await over(\"hr.lago\")); }); (await resumirNode()).filas' && tiene "d['salida']['texto']=='3'" || falla "8 · un transform desde Node: $(cuerpo)"
    "$PY" -c 'import json,sys; pr=json.load(open(sys.argv[1]))["procedencia"]; assert pr=={"inputs":["hr.lago"],"puesto":"puesto-ana-node","transform":"resumirNode"}, pr' "$A/datasets/hr_resumen_node.json" || falla "8 · la procedencia desde Node: $(cat "$A/datasets/hr_resumen_node.json")"
    celda 'await transform({ inputs: [\"hr.lago\"], output: \"hr.resumen_node\" }, async () => over(\"hr.salida\"))()' && tiene "d['salida']['tipo']=='error' and 'hr.salida' in d['salida']['mensaje']" || falla "8 · leer fuera de los inputs desde Node: $(cuerpo)"
    celda 'await declare({ kind: \"View\", metadata: { name: \"rotaNode\", namespace: \"hr\" }, spec: { owner: \"team:hr\", from: { table: \"hr.nadie\" }, fields: { a: \"a\" } } })' && tiene "d['salida']['tipo']=='error' and 'OOS' in d['salida']['mensaje']" || falla "8 · declare de una View rota desde Node: $(cuerpo)"
  fi
  celda 'await over(\"hr.nada\")' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "8 · hr.nada: $(cuerpo)"
  celda_sql 'select count(*) as n from hr.espanoles' && tiene "d['salida']['tipo']=='tabla' and d['salida']['filas']==[[3]]" || falla "8 · sql en node: $(cuerpo)"
  LEN=javascript; celda 'let z = 5; z * 2' && tiene "d['salida']['texto']=='10'" || falla "8 · javascript: $(cuerpo)"; LEN=typescript
  [ "$(pide POST /puestos/puesto-ana-node/ejecutar "$ANA" '{"texto":"1","lenguaje":"python"}')" = "422" ] || falla "8 · python en node no dio 422: $(cuerpo)"
  [ "$(pide DELETE /puestos/puesto-ana-node "$ANA")" = "200" ] || falla "8 · cerrar node: $(cuerpo)"
  for _ in $(seq 1 100); do kill -0 "$AGENTE" 2>/dev/null || break; sleep 0.25; done
  kill -0 "$AGENTE" 2>/dev/null && falla "8 · el agente node no se cerro al 410"
  AGENTE=""
  dice "8 · TS en el puesto node: 1+1 · const x: number → los tipos fuera · el contexto dura · console.log · ReferenceError · await arriba · interface · un modulo con export · saludo(persona()) → hola persona:ana · over() tabla · over(hr.lago) el dataset Iceberg en sitio con el mismo JSON · sql → [[3]] · write(): lee lo de Python, escribe lo suyo (filas y columnas, DuckDB → Parquet → ore-store) con el mismo JSON de vuelta, repetida, anexa objetos JS, una View se niega · javascript · python 422 · cierre"
else
  dice "8 · (sin node ≥ 22.13: el puesto node no se prueba aqui)"
fi

# ── 9 · Java en el puesto jvm (W3.4): el agente de la JVM (JShell en proceso), celdas, main, persona() ──
JAVAC=$(command -v javac || true); JAVA=$(command -v java || true)
JAVA_OK=no
[ -n "$JAVAC" ] && "$JAVAC" -version 2>&1 | grep -qE '^javac (2[1-9]|[3-9][0-9])' && JAVA_OK=si
if [ "$JAVA_OK" = "si" ]; then
  # Los jars: DuckDB JDBC y los de Arrow Java (`puesto/jvm/jars.txt`), en una
  # cache que sobrevive a la prueba (`ORE_JARS`, o el temporal del sistema).
  LIB="${ORE_JARS:-${TMPDIR:-/tmp}/ore-jars}"; mkdir -p "$LIB"
  [ -f "$LIB/duckdb_jdbc.jar" ] || curl -sfL --retry 3 --retry-all-errors -o "$LIB/duckdb_jdbc.jar" "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.5.5.1/duckdb_jdbc-1.5.5.1.jar" || falla "9 · no se pudo bajar duckdb_jdbc"
  grep -v '^#' "$RAIZ/puesto/jvm/jars.txt" | while read -r g v; do n="${g##*/}-$v.jar"; [ -f "$LIB/$n" ] || curl -sfL --retry 3 --retry-all-errors -o "$LIB/$n" "https://repo1.maven.org/maven2/$g/$v/$n" || echo "✗ 9 · no se pudo bajar $n"; done
  LIB_CP="$LIB"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) LIB_CP="$(cd "$LIB" && pwd -W)";; esac
  JAR_CP="$LIB_CP/*"
  CLASES="$TMP/clases"; CLASES_CP="$CLASES"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) mkdir -p "$CLASES"; CLASES_CP="$(cd "$CLASES" && pwd -W)";; esac
  SEP=":"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) SEP=";";; esac
  ABRE="--add-opens=java.base/java.nio=ALL-UNNAMED"
  "$JAVAC" -Xlint:-options --release 21 -cp "$JAR_CP" -d "$CLASES_CP" "$RAIZ"/puesto/jvm/ore/*.java 2>"$TMP/javac.txt" || falla "9 · el agente no compila: $(head -20 "$TMP/javac.txt")"
  # ── la capa de la JVM (0037 ③c), montada como en el puesto ───────────────
  # Un jar en `/capa` con DOS clases: una que sólo está ahí (tiene que verse) y
  # otra que TAMBIÉN trae la imagen (no tiene que verse: gana la imagen). La
  # marca es un METODO y no una constante a propósito — un `static final
  # String` se incrusta al compilar y la prueba saldría verde sin probar nada.
  JAR=$(dirname "$JAVAC")/jar
  CAPA_DIR="$TMP/capa"; mkdir -p "$CAPA_DIR" "$TMP/src-imagen/ore" "$TMP/src-capa/ore" "$TMP/src-capa/capa" "$TMP/c-capa"
  printf 'package ore;\npublic class Marca { public static String quien() { return "IMAGEN"; } }\n' > "$TMP/src-imagen/ore/Marca.java"
  printf 'package ore;\npublic class Marca { public static String quien() { return "CAPA"; } }\n' > "$TMP/src-capa/ore/Marca.java"
  printf 'package capa;\npublic class Saludo { public static String hola() { return "desde la capa"; } }\n' > "$TMP/src-capa/capa/Saludo.java"
  "$JAVAC" -Xlint:-options --release 21 -d "$CLASES_CP" "$TMP/src-imagen/ore/Marca.java" 2>>"$TMP/javac.txt" || falla "9 · la marca de la imagen no compila"
  "$JAVAC" -Xlint:-options --release 21 -d "$TMP/c-capa" "$TMP/src-capa/ore/Marca.java" "$TMP/src-capa/capa/Saludo.java" 2>>"$TMP/javac.txt" || falla "9 · la marca de la capa no compila"
  "$JAR" --create --file "$CAPA_DIR/la-capa.jar" -C "$TMP/c-capa" . || falla "9 · no se pudo empaquetar la capa"
  CAPA_CP="$CAPA_DIR"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) CAPA_CP="$(cd "$CAPA_DIR" && pwd -W)";; esac
  CP_JVM="$CLASES_CP$SEP$JAR_CP$SEP$CAPA_CP/*"
  "$JAVA" $ABRE -cp "$CP_JVM" ore.Agente --comprobar >"$TMP/comprobar.txt" 2>&1 || falla "9 · --comprobar: $(tail -5 "$TMP/comprobar.txt")"
  [ "$(pide POST /puestos "$ANA" '{"lenguaje":"java"}')" = "201" ] || falla "9 · abrir jvm: $(cuerpo)"
  tiene "d['id']=='puesto-ana-jvm' and d['entorno']=='jvm'" || falla "9 · la ficha jvm: $(cuerpo)"
  en_cola 51-el-puesto-ana-jvm.yaml | grep -q 'image: .*/puesto-jvm:1' || falla "9 · el Job no lleva puesto-jvm:1"
  ORE_SERVE="$BASE" PUESTO=puesto-ana-jvm ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
    "$JAVA" $ABRE -cp "$CP_JVM" ore.Agente >"$TMP/agente.txt" 2>&1 &
  AGENTE=$!
  for _ in $(seq 1 120); do pide GET /puestos/puesto-ana-jvm "$ANA" >/dev/null; tiene "d['estado']=='vivo'" && break; sleep 0.25; done
  tiene "d['estado']=='vivo'" || falla "9 · el puesto jvm no pasa a vivo: $(cuerpo)"
  P=puesto-ana-jvm; LEN=java
  celda '1 + 1' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='2'" || falla "9 · 1+1: $(cuerpo)"
  celda 'int x = 3;' && tiene "d['salida']['tipo']=='vacia'" || falla "9 · int x = 3: $(cuerpo)"
  celda 'x * 2' && tiene "d['salida']['texto']=='6'" || falla "9 · x * 2 (la sesion dura): $(cuerpo)"
  celda 'System.out.println(\"hola \" + x);' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='hola 3\n'" || falla "9 · println: $(cuerpo)"
  celda '1 / 0' && tiene "d['salida']['tipo']=='error' and 'ArithmeticException' in d['salida']['nombre']" || falla "9 · 1/0: $(cuerpo)"
  celda 'int y = \"a\";' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='CompilationError'" || falla "9 · no compila: $(cuerpo)"
  celda 'record C(String pais) {}\nvar cs = List.of(new C(\"ES\"), new C(\"PT\"));\ncs.stream().filter(c -> c.pais().equals(\"ES\")).count()' && tiene "d['salida']['texto']=='1'" || falla "9 · record + stream (varios snippets): $(cuerpo)"
  celda 'String saludo(String n) { return \"hola \" + n; }' && tiene "d['salida']['tipo']=='vacia'" || falla "9 · un metodo: $(cuerpo)"

  # ── 9c · la capa de la JVM en el classpath (0037 ③c) ─────────────────────
  #
  # ⭐ Dos cosas, y la segunda es la que importa: que una biblioteca de `/capa`
  #   SE VEA desde una celda (y por tanto que el comodín `/capa/*` llegue a
  #   JShell, que no lo expande solo), y que cuando una clase está en los dos
  #   sitios GANE LA DE LA IMAGEN — el SDK está compilado contra los jars de la
  #   imagen, y una capa que los tapara rompería `over()` de una forma
  #   imposible de explicar.
  celda 'capa.Saludo.hola()' && tiene "d['salida']['texto']=='\"desde la capa\"'" || falla "9c · la capa no se ve desde la celda (¿el comodin no llego a JShell?): $(cuerpo)"
  celda 'ore.Marca.quien()' && tiene "d['salida']['texto']=='\"IMAGEN\"'" || falla "9c · la capa TAPA a la imagen: el orden del classpath esta al reves: $(cuerpo)"
  dice "9c · la capa de la JVM: una biblioteca de /capa se ve desde la celda, y lo que tambien trae la imagen lo sigue poniendo LA IMAGEN (el orden manda)"

  # ── 9b · el servidor de lenguaje de Java, EN LA MISMA JVM (0037 ③b) ──────
  # No hay segundo proceso: el agente contesta con el compilador del JDK
  # (diagnosticos) y con `Trees` (lo que se ve desde esa posicion). Aqui se
  # abre un fichero con un fallo, se espera el subrayado, y se pide una
  # propuesta donde el editor la pediria.
  MAL='public class Ejemplo {\n    public static void main(String[] args) {\n        String saludo = \"hola\";\n        int n = saludo.longitud();\n        wr\n    }\n}\n'
  curl -sN --max-time 10 -H "$ANA" "$BASE/puestos/$P/lsp/consola" >"$TMP/lspj.txt" 2>/dev/null &
  CURL=$!
  sleep 1
  mensaje() { "$PY" -c 'import json,sys; print(json.dumps({"mensajes":[sys.argv[1]]}))' "$1"; }
  ABRIR_J="$("$PY" -c 'import json,sys; print(json.dumps({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///trabajo/transforms/Ejemplo.java","languageId":"java","version":1,"text":sys.argv[1]}}}))' "$(printf "$MAL")")"
  [ "$(pide POST /puestos/$P/lsp "$ANA" "$(mensaje "$ABRIR_J")")" = "202" ] || falla "9b · el didOpen no dio 202: $(cuerpo)"
  sleep 3
  COMP_J='{"jsonrpc":"2.0","id":31,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///trabajo/transforms/Ejemplo.java"},"position":{"line":4,"character":11}}}'
  [ "$(pide POST /puestos/$P/lsp "$ANA" "$(mensaje "$COMP_J")")" = "202" ] || falla "9b · el completion no dio 202: $(cuerpo)"
  HOV_J='{"jsonrpc":"2.0","id":32,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///trabajo/transforms/Ejemplo.java"},"position":{"line":2,"character":16}}}'
  [ "$(pide POST /puestos/$P/lsp "$ANA" "$(mensaje "$HOV_J")")" = "202" ] || falla "9b · el hover no dio 202: $(cuerpo)"
  wait $CURL 2>/dev/null || true
  grep -q "publishDiagnostics" "$TMP/lspj.txt" || falla "9b · no llego ni un diagnostico: $(cat "$TMP/lspj.txt")"
  grep -q "cannot find symbol" "$TMP/lspj.txt" || falla "9b · el diagnostico no es el de javac: $(cat "$TMP/lspj.txt")"
  grep -q '"source": *"javac"' "$TMP/lspj.txt" || falla "9b · el diagnostico no dice quien lo firma: $(cat "$TMP/lspj.txt")"
  grep -qE '"id": *31' "$TMP/lspj.txt" || falla "9b · el completion no volvio: $(cat "$TMP/lspj.txt")"
  grep -q '"label": *"saludo"' "$TMP/lspj.txt" || falla "9b · el completion no ve las variables LOCALES del fichero: $(cat "$TMP/lspj.txt")"
  grep -qE '"id": *32' "$TMP/lspj.txt" || falla "9b · el hover no volvio: $(cat "$TMP/lspj.txt")"
  grep -q "String saludo" "$TMP/lspj.txt" || falla "9b · el hover no dice el tipo: $(cat "$TMP/lspj.txt")"
  dice "9b · el servidor de lenguaje de Java EN LA MISMA JVM: javac firma los diagnosticos (cannot find symbol, con su linea), el autocompletado ve las variables LOCALES del fichero y el hover dice el tipo — sin segundo proceso"
  celda 'saludo(persona())' && tiene "d['salida']['texto']=='\"hola persona:ana\"'" || falla "9 · saludo(persona()) — con la identidad de la persona: $(cuerpo)"
  celda 'public class Programa { public static void main(String[] a) { System.out.println(\"main de \" + persona()); } }' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='main de persona:ana\n'" || falla "9 · una clase con main (un .java del arbol): $(cuerpo)"
  celda 'var df = over(\"hr.espanoles\"); df' && tiene "d['salida']['tipo']=='tabla' and [c['name'] for c in d['salida']['columnas']]==['id','pais'] and d['salida']['filas']==[['e1','ES'],['e2','ES'],['e3','ES']] and d['salida']['total']==3" || falla "9 · over(hr.espanoles): $(cuerpo)"
  if [ "$LAGO_OK" = "si" ]; then
    celda 'over(\"hr.lago\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "9 · over(hr.lago), el dataset Iceberg: $(cuerpo)"
    celda 'sql(\"select count(*) as n, sum(importe) as s from hr.lago\")' && tiene "d['salida']['filas']==[[3,'3.75']]" || falla "9 · sql sobre el dataset Iceberg: $(cuerpo)"
  fi
  if [ "${ESCRITO_OK:-no}" = "si" ]; then
    # write() desde Java: lo que Python (6) y Node (5) escribieron, leído; y lo suyo, escrito y leído
    celda 'over(\"hr.salida\").size() + over(\"hr.salida_node\").size()' && tiene "d['salida']['texto']=='11'" || falla "9 · Java lee lo que Python y Node escribieron: $(cuerpo)"
    celda 'var e = write(\"hr.salida_jvm\", over(\"hr.lago\")); e.get(\"filas\") + \" \" + e.get(\"repetida\")' && grep -Eq "derivedFrom: \[.*hr\.lago.*\]" "$A/packages/hr/datasets/salida_jvm.yaml" && tiene "d['salida']['texto']=='\"3 false\"'" || falla "9 · write(hr.salida_jvm) desde Filas: $(cuerpo)"
    celda 'over(\"hr.salida_jvm\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "9 · over(hr.salida_jvm) no es el mismo JSON que hr.lago: $(cuerpo)"
    celda 'var e2 = write(\"hr.salida_jvm\", arrow(\"hr.lago\")); e2.get(\"repetida\")' && tiene "d['salida']['texto']=='true'" || falla "9 · la misma escritura (por Arrow) tenía que ser repetida: $(cuerpo)"
    celda 'var e3 = write(\"hr.salida_jvm\", List.of(Map.of(\"n\", 4L, \"letra\", \"d\", \"cuando\", java.time.Instant.parse(\"2024-06-02T00:00:00Z\"), \"importe\", new java.math.BigDecimal(\"4.00\"))), \"anexar\"); e3.get(\"filas\")' && tiene "d['salida']['texto']=='4'" || falla "9 · anexar un List<Map>: $(cuerpo)"
    celda 'sql(\"select count(*) as n, sum(importe) as s from hr.salida_jvm\")' && tiene "d['salida']['filas']==[[4,'7.75']]" || falla "9 · sql sobre lo escrito desde Java: $(cuerpo)"
    celda 'var e4 = write(\"hr.salida_jvm\", List.of(Map.of(\"n\", 4L, \"letra\", \"D\", \"cuando\", java.time.Instant.parse(\"2024-06-03T00:00:00Z\"), \"importe\", new java.math.BigDecimal(\"40.00\")), Map.of(\"n\", 6L, \"letra\", \"f\", \"cuando\", java.time.Instant.parse(\"2024-06-03T00:00:00Z\"), \"importe\", new java.math.BigDecimal(\"6.00\"))), \"upsert\", List.of(\"n\")); e4.get(\"filas\")' && tiene "d['salida']['texto']=='5'" || falla "9 · upsert desde Java: $(cuerpo)"
    celda 'sql(\"select count(*) as n, sum(importe) as s from hr.salida_jvm\")' && tiene "d['salida']['filas']==[[5,'49.75']]" || falla "9 · sql tras el upsert desde Java: $(cuerpo)"
    celda 'write(\"hr.empleados\", List.of(Map.of(\"a\", 1L)))' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "9 · escribir una View desde Java: $(cuerpo)"
    celda 'var dv = declare(\"apiVersion: oos.dev/v1alpha12\\nkind: View\\nmetadata: { name: porLetraJvm, namespace: hr }\\nspec:\\n  owner: team:hr\\n  from: { dataset: hr.salida_jvm }\\n  fields: { letra: letra, n: \\\"count()\\\" }\\n  groupBy: [letra]\\n\"); dv.get(\"kind\") + \" \" + dv.get(\"nombre\") + \" \" + dv.get(\"nueva\")' && tiene "d['salida']['texto']=='\"View hr.porLetraJvm true\"'" || falla "9 · declare(View) desde Java: $(cuerpo)"
    [ -f "$A/packages/hr/views/porLetraJvm.yaml" ] || falla "9 · la View declarada desde Java no está en el árbol"
    celda 'var ej = transform(\"resumirJvm\", List.of(\"hr.lago\"), \"hr.resumen_jvm\", () -> write(\"hr.resumen_jvm\", over(\"hr.lago\"))); ej.get(\"filas\")' && tiene "d['salida']['texto']=='3'" || falla "9 · un transform desde Java: $(cuerpo)"
    "$PY" -c 'import json,sys; pr=json.load(open(sys.argv[1]))["procedencia"]; assert pr=={"inputs":["hr.lago"],"puesto":"puesto-ana-jvm","transform":"resumirJvm"}, pr' "$A/datasets/hr_resumen_jvm.json" || falla "9 · la procedencia desde Java: $(cat "$A/datasets/hr_resumen_jvm.json")"
    celda 'transform(\"fuera\", List.of(\"hr.lago\"), \"hr.resumen_jvm\", () -> over(\"hr.salida\"))' && tiene "d['salida']['tipo']=='error' and 'hr.salida' in d['salida']['mensaje']" || falla "9 · leer fuera de los inputs desde Java: $(cuerpo)"
    celda 'declare(Map.of(\"kind\", \"View\", \"metadata\", Map.of(\"name\", \"rotaJvm\", \"namespace\", \"hr\"), \"spec\", Map.of(\"owner\", \"team:hr\", \"from\", Map.of(\"table\", \"hr.nadie\"), \"fields\", Map.of(\"a\", \"a\"))))' && tiene "d['salida']['tipo']=='error' and 'OOS' in d['salida']['mensaje']" || falla "9 · declare de una View rota desde Java: $(cuerpo)"
  fi
  celda 'over(\"hr.nada\")' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "9 · hr.nada: $(cuerpo)"
  celda_sql 'select count(*) as n from hr.espanoles' && tiene "d['salida']['tipo']=='tabla' and d['salida']['filas']==[[3]]" || falla "9 · sql en la jvm: $(cuerpo)"
  [ "$(pide POST /puestos/puesto-ana-jvm/ejecutar "$ANA" '{"texto":"1","lenguaje":"typescript"}')" = "422" ] || falla "9 · typescript en jvm no dio 422: $(cuerpo)"
  [ "$(pide DELETE /puestos/puesto-ana-jvm "$ANA")" = "200" ] || falla "9 · cerrar jvm: $(cuerpo)"
  for _ in $(seq 1 100); do kill -0 "$AGENTE" 2>/dev/null || break; sleep 0.25; done
  kill -0 "$AGENTE" 2>/dev/null && falla "9 · el agente jvm no se cerro al 410"
  AGENTE=""
  dice "9 · Java en el puesto jvm: 1+1 · int x → vacia · la sesion dura · println · ArithmeticException · no compila → CompilationError · record + stream · un metodo · saludo(persona()) · una clase con main · over() tabla · over(hr.lago) el dataset Iceberg en sitio con el mismo JSON · sql → [[3]] · write(): lee lo de Python y Node, escribe lo suyo (Filas, Arrow, List<Map>) con el mismo JSON de vuelta, repetida, anexa, una View se niega · typescript 422 · cierre"
else
  dice "9 · (sin javac ≥ 21: el puesto jvm no se prueba aqui)"
fi

limpiar
echo "✓ el puesto (0031 W3.1–W3.7 y 0036 ⑤): 1–17 · la sesión viva en python, node y jvm, el agente de verdad, over() y sql() sobre las copias, write() al lago desde los tres (y cada uno lee lo de los otros), persona(), la capa declarada en el árbol"
