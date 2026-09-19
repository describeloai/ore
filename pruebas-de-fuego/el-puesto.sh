#!/usr/bin/env bash
# EL PUESTO (0031 W3.1): la sesión viva, de punta a punta sin clúster.
#
# `ore-serve` con el árbol en un directorio y la cola en un repositorio pelado
# con la plantilla del puesto; el AGENTE DE VERDAD (`puesto/python/agente.py`)
# corriendo aquí al lado con la identidad de cabecera `agente:…` y el almacén en
# un directorio (`ORE_ALMACEN=dir:`), donde hay una copia de verdad —el sobre
# `ORECOPY1` con Parquet dentro— de la vista `hr.espanoles`.
#
#   1  POST /puestos (ana)             201 puesto-ana encolado · el fichero en la cola con
#                                      el id y el Job `puesto-ana-<8 hex>` · repetido, 200 el
#                                      mismo · bea no lo ve (403) · el agente no abre (403) ·
#                                      sin cola, 503
#   2  el agente reclama el puesto     GET /puestos/puesto-ana pasa a vivo · otro agente, 403
#   3  las celdas                      `1+1` → texto 2 · `print` → texto · `x = 3` → vacia ·
#                                      `x * 2` → 6 (el espacio dura) · `1/0` → error con traza ·
#                                      bea no manda celdas a lo de ana (403)
#   4  over("hr.espanoles")            tabla: columnas y filas de la copia, total · `hr.nada` →
#                                      error LookupError (404 de datos) · `hr.empleados` sin
#                                      copia → RuntimeError (409)
#   5  DELETE /puestos/puesto-ana      200 · el fichero fuera de la cola · el agente se cierra
#                                      (410) · una celda más → 410
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
limpiar() {
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

# ── la cola: un repositorio pelado con la plantilla del puesto ─────────────
COLA="$TMP/cola.git"
git init -q --bare -b main "$COLA"
mkdir -p "$TMP/cola-semilla" && ( cd "$TMP/cola-semilla" && git init -q -b main && git config core.autocrlf false )
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TMP/rendido" >/dev/null 2>&1 || falla "no se pudo rendir la plantilla del puesto"
[ -f "$TMP/rendido/plantilla-puesto.txt" ] || falla "gen-inquilino no rinde plantilla-puesto.txt"
cp "$TMP/rendido/plantilla-puesto.txt" "$TMP/cola-semilla/"
( cd "$TMP/cola-semilla" && git add -A && git -c user.name=banco -c user.email=banco@invalido commit -q -m "la plantilla" \
  && git remote add origin "$COLA" && git push -q origin HEAD:main ) || falla "no se pudo sembrar la cola"
en_cola() { git --git-dir="$COLA" show "main:$1" 2>/dev/null; }
COLA_URL="file://$(cd "$COLA" && pwd | sed 's#^/\([a-zA-Z]\)/#\1:/#')"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) COLA_URL="file:///$(cd "$COLA" && pwd -W)";; esac

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
tiene "d['id']=='puesto-ana' and d['estado']=='encolado' and d['lenguaje']=='python' and d['fichero']=='51-el-puesto-ana.yaml' and d['job'].startswith('puesto-ana-') and 'commit' in d['cola']" || falla "1 · la ficha: $(cuerpo)"
en_cola 51-el-puesto-ana.yaml | grep -q 'name: PUESTO, value: "puesto-ana"' || falla "1 · el fichero de la cola no lleva el id: $(en_cola 51-el-puesto-ana.yaml | grep -n PUESTO)"
en_cola 51-el-puesto-ana.yaml | grep -qE 'name: puesto-ana-[0-9a-f]{8}$' || falla "1 · el Job no se llama puesto-ana-<8 hex>"
en_cola 51-el-puesto-ana.yaml | grep -q 'ore.dev/rol: puesto' || falla "1 · el Job no lleva el rol puesto"
[ "$(pide POST /puestos "$ANA" '{}')" = "200" ] && tiene "d['id']=='puesto-ana'" || falla "1 · repetir no dio 200 con el mismo: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana "$BEA")" = "403" ] || falla "1 · bea ve el puesto de ana: $(cuerpo)"
[ "$(pide POST /puestos "$AG" '{}')" = "403" ] || falla "1 · un agente abrio un puesto: $(cuerpo)"
[ "$(pide POST /puestos "$ANA" '{"lenguaje":"java"}')" = "200" ] || true  # ya tiene uno: 200 antes de mirar el lenguaje
[ "$(pide POST /puestos "$BEA" '{"lenguaje":"java"}')" = "422" ] || falla "1 · java no dio 422: $(cuerpo)"
[ "$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$BEA" -H 'content-type: application/json' --data-binary '{}' "http://127.0.0.1:$PUERTO_SIN/puestos")" = "503" ] || falla "1 · sin cola no dio 503: $(cuerpo)"
dice "1 · POST /puestos: 201 puesto-ana encolado, el fichero en la cola con id, Job puesto-ana-<8 hex> y rol puesto · repetido 200 · bea 403 · un agente 403 · java 422 · sin cola 503"

# ── 2 · el agente reclama el puesto ────────────────────────────────────────
ORE_SERVE="$BASE" PUESTO=puesto-ana ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
  "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/agente.txt" 2>&1 &
AGENTE=$!
for _ in $(seq 1 40); do pide GET /puestos/puesto-ana "$ANA" >/dev/null; tiene "d['estado']=='vivo'" && break; sleep 0.25; done
tiene "d['estado']=='vivo'" || falla "2 · el puesto no pasa a vivo: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana/pendiente "$OTRO")" = "403" ] || falla "2 · otro agente reclamo el puesto: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana/pendiente "$ANA")" = "403" ] || falla "2 · una persona pidio trabajo: $(cuerpo)"
dice "2 · el agente reclama el puesto: vivo · otro agente 403 · una persona 403"

# ── 3 · las celdas ─────────────────────────────────────────────────────────
celda() { # <texto json-escapado> → deja la salida en r.json; imprime el codigo de la espera
  local n
  [ "$(pide POST /puestos/puesto-ana/ejecutar "$ANA" "{\"texto\":\"$1\"}")" = "202" ] || falla "3 · ejecutar no dio 202: $(cuerpo)"
  n=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["celda"])' "$TMP/r.json")
  for _ in $(seq 1 3); do
    pide GET "/puestos/puesto-ana/celdas/$n" "$ANA" >/dev/null
    tiene "d['estado']=='hecha'" && return 0
  done
  return 1
}
celda '1+1' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='2'" || falla "3 · 1+1: $(cuerpo)"
celda 'print(\"hola\")' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='hola\n'" || falla "3 · print: $(cuerpo)"
celda 'x = 3' && tiene "d['salida']['tipo']=='vacia'" || falla "3 · x = 3: $(cuerpo)"
celda 'x * 2' && tiene "d['salida']['tipo']=='texto' and d['salida']['texto']=='6'" || falla "3 · x * 2 (el espacio dura): $(cuerpo)"
celda '1/0' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='ZeroDivisionError' and 'Traceback' in d['salida']['traza']" || falla "3 · 1/0: $(cuerpo)"
[ "$(pide POST /puestos/puesto-ana/ejecutar "$BEA" '{"texto":"1"}')" = "403" ] || falla "3 · bea mando una celda a lo de ana: $(cuerpo)"
[ "$(pide POST /puestos/puesto-ana/ejecutar "$ANA" '{}')" = "422" ] || falla "3 · sin texto no dio 422: $(cuerpo)"
pide GET /puestos/puesto-ana "$ANA" >/dev/null; tiene "d['celdas']==5 and d['pendientes']==0" || falla "3 · la ficha no cuenta las celdas: $(cuerpo)"
dice "3 · las celdas: 1+1 → 2 · print → texto · x = 3 → vacia · x * 2 → 6 (el espacio dura) · 1/0 → error con traza · bea 403 · sin texto 422"

# ── 4 · over() ─────────────────────────────────────────────────────────────
celda 'df = over(\"hr.espanoles\"); df' && tiene "d['salida']['tipo']=='tabla' and [c['name'] for c in d['salida']['columnas']]==['id','pais'] and d['salida']['filas']==[['e1','ES'],['e2','ES'],['e3','ES']] and d['salida']['total']==3" || falla "4 · over(hr.espanoles): $(cuerpo)"
celda 'len(df)' && tiene "d['salida']['texto']=='3'" || falla "4 · len(df): $(cuerpo)"
celda 'over(\"hr.nada\")' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='LookupError'" || falla "4 · hr.nada: $(cuerpo)"
celda 'over(\"hr.empleados\")' && tiene "d['salida']['tipo']=='error' and d['salida']['nombre']=='RuntimeError' and 'no est' in d['salida']['mensaje']" || falla "4 · hr.empleados sin copia: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana/datos/hr.espanoles "$AG")" = "200" ] && tiene "d['clave']=='$CLAVE' and d['estado']=='copiada'" || falla "4 · datos: $(cuerpo)"
[ "$(pide GET /puestos/puesto-ana/datos/hr.espanoles "$ANA")" = "403" ] || falla "4 · una persona pidio datos por la ruta del agente: $(cuerpo)"
dice "4 · over(\"hr.espanoles\") → tabla 3 × 2 desde la copia (ORECOPY1 + Parquet) · hr.nada → LookupError (404) · sin copia → RuntimeError (409) · datos solo para el agente"

# ── 5 · cerrar ─────────────────────────────────────────────────────────────
[ "$(pide DELETE /puestos/puesto-ana "$BEA")" = "403" ] || falla "5 · bea cerro el puesto de ana"
[ "$(pide DELETE /puestos/puesto-ana "$ANA")" = "200" ] && tiene "d['estado']=='cerrado' and 'fuera de la cola' in d['cola']" || falla "5 · cerrar: $(cuerpo)"
en_cola 51-el-puesto-ana.yaml >/dev/null && falla "5 · el fichero sigue en la cola"
for _ in $(seq 1 100); do kill -0 "$AGENTE" 2>/dev/null || break; sleep 0.25; done
kill -0 "$AGENTE" 2>/dev/null && falla "5 · el agente no se cerro al 410"
AGENTE=""
grep -q "cerrado" "$TMP/agente.txt" || falla "5 · el agente no dijo por que se fue: $(tail -3 "$TMP/agente.txt")"
[ "$(pide POST /puestos/puesto-ana/ejecutar "$ANA" '{"texto":"1"}')" = "410" ] || falla "5 · una celda tras cerrar no dio 410: $(cuerpo)"
[ "$(pide POST /puestos "$ANA" '{}')" = "201" ] || falla "5 · abrir de nuevo tras cerrar: $(cuerpo)"
dice "5 · DELETE: 200, el fichero fuera de la cola, el agente se cierra al 410, una celda mas → 410, y se puede abrir otro"

limpiar
echo "✓ el puesto (0031 W3.1): 1–5 · la sesión viva, el agente de verdad, over() sobre la copia"
