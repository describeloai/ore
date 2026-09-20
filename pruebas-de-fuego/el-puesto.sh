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
#  10  write() (W3.6c, 0031 §11)      la celda escribe `hr.lago` como `hr.salida`: la tabla va por
#                                      IPC a `ore-store-r2` (el S3 de mentira, con la credencial que
#                                      el catálogo prestó), el commit por `/v1/…`, y `over("hr.salida")`
#                                      devuelve EL MISMO JSON que hr.lago (los tipos sobreviven la
#                                      vuelta) · la misma escritura otra vez: `repetida`, un snapshot ·
#                                      `anexar` un DataFrame → 5 · `upsert` por clave → 6 y la Table
#                                      declara `mode: upsert, key: [n]` · a una View → error · uint64 → error
#                                      con la columna; y en 8 y 9, Node y Java escriben lo suyo y leen
#                                      lo de los demás
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
SRV=""; SRV2=""; AGENTE=""

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
mkdir -p "$A/packages/hr/tables" "$A/packages/hr/views" "$A/copias"
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
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: empleados, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: hr.empleados_t
  fields:
    id: { from: id, type: String }
    pais: { from: pais, type: String }
Y
cat > "$A/packages/hr/views/espanoles.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: espanoles, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: hr.empleados
  where: { pais: ES }
  fields:
    id: { from: id, type: String }
Y
cat > "$A/packages/hr/views/lago.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: lago, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: hr.empleados
  fields:
    id: { from: id, type: String }
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
"$PY" -c 'import json,sys; json.dump({"estado":"copiada","clave":sys.argv[1],"plan":"x","filas":"3"}, open(sys.argv[2],"w"))' "$CLAVE" "$A/copias/hr_espanoles.json"

# ── el dataset: una tabla Iceberg (0031 §10), y su puntero en el árbol ──────
# Escrita con PyIceberg (el catálogo es el árbol: `medida-w3-iceberg.py`); el
# puntero es `copias/hr_lago.json` con `metadata_location`. Los tres SDK la leen
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
    "$PY" -c 'import json,sys; json.dump({"estado":"copiada","metadata_location":sys.argv[1],"snapshot":"1","plan":"x","filas":"3"}, open(sys.argv[2],"w"))' "$META" "$A/copias/hr_lago.json"
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
cp "$TMP/rendido/plantilla-puesto.txt" "$TMP/rendido/plantilla-capa.txt" "$TMP/cola-semilla/"
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
ORE_SERVE="$BASE" PUESTO=puesto-ana-python ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
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
  [ -f "$A/packages/hr/tables/salida.yaml" ] && grep -q "datasource: lago" "$A/packages/hr/tables/salida.yaml" && grep -q "cuando: { type: DateTimeTz }" "$A/packages/hr/tables/salida.yaml" || falla "10 · la Table del lago no nació tipada en el árbol: $(cat "$A/packages/hr/tables/salida.yaml" 2>/dev/null)"
  [ -f "$A/datasets/hr_salida.json" ] || falla "10 · el puntero no está en el árbol"
  grep -q "name: lago" "$A/ontology.config.yaml" || falla "10 · el datasource lago no se declaró"
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
  grep -q "changes: { mode: upsert, key: \[n\], witness: snapshot }" "$A/packages/hr/tables/salida.yaml" || falla "10 · la Table no declara el upsert: $(grep changes "$A/packages/hr/tables/salida.yaml")"
  celda 'write(\"hr.salida\", over(\"hr.lago\", como=\"arrow\"), clave=[\"n\"])' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ValueError'" || falla "10 · clave sin upsert tenía que ser ValueError: $(cuerpo)"
  # lo que no se escribe: una View, y una columna que 0032 no tiene
  celda 'write(\"hr.espanoles\", over(\"hr.lago\", como=\"arrow\"))' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "10 · escribir una View: $(cuerpo)"
  celda 'import pyarrow as pa; write(\"hr.mala\", pa.table({\"grande\": pa.array([1], pa.uint64())}))' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ValueError' and 'grande' in d['salida']['mensaje']" || falla "10 · uint64: $(cuerpo)"
  [ ! -f "$A/datasets/hr_mala.json" ] || falla "10 · lo negado dejó puntero"
  dice "10 · write(hr.salida) desde la celda: la tabla por IPC a ore-store-r2 con la credencial prestada, el commit por /v1, la Table del lago nace tipada y over() devuelve el mismo JSON que hr.lago · repetida sin snapshot · anexar un DataFrame → 5 y sql lo suma · upsert por clave → 6 (54.75) y la Table declara la clave · una View → error · uint64 → ValueError con la columna"
  ESCRITO_OK=si
else
  ESCRITO_OK=no
  dice "10 · (sin el lago o sin ore-store-r2: write() no se prueba aquí)"
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
  pide GET /puestos "$ANA" >/dev/null; tiene "sorted(p['id'] for p in d['puestos'])==['puesto-ana-node','puesto-ana-python']" || falla "8 · GET /puestos no lista los dos: $(cuerpo)"
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
    celda 'const e = await write(\"hr.salida_node\", await over(\"hr.lago\")); [e.filas, e.repetida]' && tiene "d['salida']['texto']=='[ 3, false ]'" || falla "8 · write(hr.salida_node) desde filas: $(cuerpo)"
    celda 'await over(\"hr.salida_node\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "8 · over(hr.salida_node) no es el mismo JSON que hr.lago: $(cuerpo)"
    celda 'const e2 = await write(\"hr.salida_node\", await over(\"hr.lago\", { como: \"columnas\" })); e2.repetida' && tiene "d['salida']['texto']=='true'" || falla "8 · la misma escritura (por columnas) tenía que ser repetida: $(cuerpo)"
    celda 'const e3 = await write(\"hr.salida_node\", [{ n: 4, letra: \"d\", cuando: new Date(\"2024-06-02T00:00:00Z\"), importe: 4 }], { modo: \"anexar\" }); e3.filas' && tiene "d['salida']['texto']=='4'" || falla "8 · anexar objetos JS: $(cuerpo)"
    celda 'await sql(\"select count(*) as n, sum(importe) as s from hr.salida_node\")' && tiene "d['salida']['filas']==[[4,'7.75']]" || falla "8 · sql sobre lo escrito desde Node: $(cuerpo)"
    celda 'const e4 = await write(\"hr.salida_node\", [{ n: 4, letra: \"D\", cuando: new Date(\"2024-06-03T00:00:00Z\"), importe: 40 }, { n: 6, letra: \"f\", cuando: null, importe: 6 }], { modo: \"upsert\", clave: [\"n\"] }); e4.filas' && tiene "d['salida']['texto']=='5'" || falla "8 · upsert desde Node: $(cuerpo)"
    celda 'await sql(\"select count(*) as n, sum(importe) as s from hr.salida_node\")' && tiene "d['salida']['filas']==[[5,'49.75']]" || falla "8 · sql tras el upsert desde Node: $(cuerpo)"
    celda 'await write(\"hr.espanoles\", [{ a: 1 }])' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "8 · escribir una View desde Node: $(cuerpo)"
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
  "$JAVA" $ABRE -cp "$CLASES_CP$SEP$JAR_CP" ore.Agente --comprobar >"$TMP/comprobar.txt" 2>&1 || falla "9 · --comprobar: $(tail -5 "$TMP/comprobar.txt")"
  [ "$(pide POST /puestos "$ANA" '{"lenguaje":"java"}')" = "201" ] || falla "9 · abrir jvm: $(cuerpo)"
  tiene "d['id']=='puesto-ana-jvm' and d['entorno']=='jvm'" || falla "9 · la ficha jvm: $(cuerpo)"
  en_cola 51-el-puesto-ana-jvm.yaml | grep -q 'image: .*/puesto-jvm:1' || falla "9 · el Job no lleva puesto-jvm:1"
  ORE_SERVE="$BASE" PUESTO=puesto-ana-jvm ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
    "$JAVA" $ABRE -cp "$CLASES_CP$SEP$JAR_CP" ore.Agente >"$TMP/agente.txt" 2>&1 &
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
    celda 'var e = write(\"hr.salida_jvm\", over(\"hr.lago\")); e.get(\"filas\") + \" \" + e.get(\"repetida\")' && tiene "d['salida']['texto']=='\"3 false\"'" || falla "9 · write(hr.salida_jvm) desde Filas: $(cuerpo)"
    celda 'over(\"hr.salida_jvm\")' && tiene "d['salida']['tipo']=='tabla' and $LAGO_COLS and $LAGO_FILAS" || falla "9 · over(hr.salida_jvm) no es el mismo JSON que hr.lago: $(cuerpo)"
    celda 'var e2 = write(\"hr.salida_jvm\", arrow(\"hr.lago\")); e2.get(\"repetida\")' && tiene "d['salida']['texto']=='true'" || falla "9 · la misma escritura (por Arrow) tenía que ser repetida: $(cuerpo)"
    celda 'var e3 = write(\"hr.salida_jvm\", List.of(Map.of(\"n\", 4L, \"letra\", \"d\", \"cuando\", java.time.Instant.parse(\"2024-06-02T00:00:00Z\"), \"importe\", new java.math.BigDecimal(\"4.00\"))), \"anexar\"); e3.get(\"filas\")' && tiene "d['salida']['texto']=='4'" || falla "9 · anexar un List<Map>: $(cuerpo)"
    celda 'sql(\"select count(*) as n, sum(importe) as s from hr.salida_jvm\")' && tiene "d['salida']['filas']==[[4,'7.75']]" || falla "9 · sql sobre lo escrito desde Java: $(cuerpo)"
    celda 'var e4 = write(\"hr.salida_jvm\", List.of(Map.of(\"n\", 4L, \"letra\", \"D\", \"cuando\", java.time.Instant.parse(\"2024-06-03T00:00:00Z\"), \"importe\", new java.math.BigDecimal(\"40.00\")), Map.of(\"n\", 6L, \"letra\", \"f\", \"cuando\", java.time.Instant.parse(\"2024-06-03T00:00:00Z\"), \"importe\", new java.math.BigDecimal(\"6.00\"))), \"upsert\", List.of(\"n\")); e4.get(\"filas\")' && tiene "d['salida']['texto']=='5'" || falla "9 · upsert desde Java: $(cuerpo)"
    celda 'sql(\"select count(*) as n, sum(importe) as s from hr.salida_jvm\")' && tiene "d['salida']['filas']==[[5,'49.75']]" || falla "9 · sql tras el upsert desde Java: $(cuerpo)"
    celda 'write(\"hr.espanoles\", List.of(Map.of(\"a\", 1L)))' && tiene "d['salida']['tipo']=='error' and 'View' in d['salida']['mensaje']" || falla "9 · escribir una View desde Java: $(cuerpo)"
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
echo "✓ el puesto (0031 W3.1–W3.6c): 1–10 · la sesión viva en python, node y jvm, el agente de verdad, over() y sql() sobre las copias, write() al lago desde los tres (y cada uno lee lo de los otros), persona(), la capa declarada en el árbol"
