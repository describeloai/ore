#!/usr/bin/env bash
# BIGQUERY DE PUNTA A PUNTA, CONTRA UN DATASET DE VERDAD: la vara de la Fase A.
#
# Afirma lo que DEBE pasar cuando BigQuery entra por REST (plan de la Fase A,
# 2026-09-26), no lo que pasa hoy. Nacio en ROJO a proposito: el rojo de hoy es
# la linea base medida, y cada paso de la fase apaga las aserciones que llevan su
# etiqueta. Una aserción sin etiqueta ya pasa hoy y no puede dejar de pasar.
#
#   [A2] el transporte REST: `'null'` es texto, los microsegundos llegan, un
#        TIMESTAMP se estrecha a instante (hoy los tres se pierden en `bq`)
#   [A3] el catalogo en el driver: `ore-read-bigquery catalogo` contesta
#   [A4] el contrato de tipos: NUMERIC → decimal(38, 9), REQUIRED → required
#   [A5] (sin aserción aqui todavia: pide una tabla vacia en el dataset)
#
# Lo que se mide, de punta a punta y sin escribir en GCP:
#   1  `ore source check` responde
#   2  el catalogo: las dos tablas, y sus tipos
#   3  `discover --type standard --no-model` + `validate`: el arbol compila
#   4  `ore materialize` contra un S3 de mentira: 8 y 5 filas copiadas, sin
#      columnas que se queden sin estrechar
#   5  la metadata de Iceberg: los fisicos de 0032
#   6  lo que se relee de la copia es EXACTAMENTE lo sembrado
#
# Necesita:
#   · `BQ_URL=bigquery://<proyecto>/<dataset>` con la semilla de
#     `semilla/bigquery-ventas.sql` cargada (filas con id `ore-e2e-*`). Sin
#     `BQ_URL` se salta: CI no alcanza BigQuery, y decirlo es mejor que fingir.
#   · credenciales de `gcloud` (hoy el driver usa `bq`; desde A1, un token).
#   · `ore`, `ore-read-bigquery`, `ore-store-r2` en target/{release,debug}.
#   · Python 3 (el S3 de mentira y las comparaciones).
#
# Uso:  BQ_URL=bigquery://p/ventas bash pruebas-de-fuego/bigquery-real.sh
set -u

if [ -z "${BQ_URL:-}" ]; then
  echo "se salta · sin BQ_URL no hay dataset contra el que medir"
  exit 0
fi

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python)
TMP="${TMPDIR:-/tmp}/ore-bq-real-$$"
rm -rf "$TMP"; mkdir -p "$TMP"
S3_PID=""
limpiar() { [ -n "$S3_PID" ] && kill "$S3_PID" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT
export PYTHONUTF8=1

fallos=0
ok()    { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '  \xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }
dice()  { printf '  \xc2\xb7 %s\n' "$1"; }
# comprueba ETIQUETA DESCRIPCION: lee 0/1 de stdin (1 = se cumple)
afirma() { if [ "$3" = "1" ]; then ok "$1 $2"; else falla "$1 $2${4:+ · $4}"; fi; }

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"                      || { echo "no hay binario de \`ore\`"; exit 2; }
DRV="$(buscar ore-read-bigquery)"        || { echo "no hay binario de \`ore-read-bigquery\`"; exit 2; }
STORE="$(buscar ore-store-r2)"           || { echo "no hay binario de \`ore-store-r2\`"; exit 2; }
export PATH="$(dirname "$DRV"):$(dirname "$STORE"):$PATH"

# ── el S3 de mentira ─────────────────────────────────────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; cat "$TMP/s3.log"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"

reloj() { "$PY" -c 'import time; print(time.time())'; }
lapso() { "$PY" -c "print(f'{$2 - $1:.1f} s')"; }

# ── el arbol ─────────────────────────────────────────────────────────────────
A="$TMP/arbol"; mkdir -p "$A"; cd "$A"
"$ORE" init . --name ventas >/dev/null 2>&1 || { echo "ore init falló"; exit 2; }
printf 'datasources:\n  - { name: bq, type: bigquery, connectionEnv: BQ_URL }\n' >> ontology.config.yaml
# Declarar el conducto es un acto del operador (malla/94): `ore init` no lo hace.
cat > conduits.yaml <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: default }
spec:
  owner: team:datos
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y

echo "── 1 · la fuente responde"
t0=$(reloj)
if "$ORE" source check bq > "$TMP/check.txt" 2>&1; then ok "ore source check bq ($(lapso "$t0" "$(reloj)"))"
else falla "ore source check bq: $(tail -2 "$TMP/check.txt")"; fi

echo "── 2 · el catalogo"
t0=$(reloj)
"$ORE" source catalog bq --out "$TMP/catalogo.json" > "$TMP/cat.txt" 2>&1 \
  || falla "ore source catalog bq: $(tail -2 "$TMP/cat.txt")"
dice "ore source catalog bq en $(lapso "$t0" "$(reloj)")"
cat_col() {  # tabla columna campo
  "$PY" -c 'import json,sys
d=json.load(open(sys.argv[1],encoding="utf-8"))
t=[t for t in d["tables"] if t["name"]==sys.argv[2]]
c=[c for c in (t[0]["columns"] if t else []) if c["name"]==sys.argv[3]]
print(json.dumps(c[0].get(sys.argv[4])) if c else "sin-columna")' "$TMP/catalogo.json" "$1" "$2" "$3" 2>/dev/null
}
n=$("$PY" -c 'import json,sys; print(sorted(t["name"] for t in json.load(open(sys.argv[1],encoding="utf-8"))["tables"]))' "$TMP/catalogo.json" 2>/dev/null)
afirma "" "el catalogo trae ventas.clientes y ventas.pedidos" \
  "$([ "$n" = "['ventas.clientes', 'ventas.pedidos']" ] && echo 1)" "trae $n"
afirma "" "pedidos.ts es DateTimeTz en el catalogo" "$([ "$(cat_col ventas.pedidos ts type)" = '"DateTimeTz"' ] && echo 1)"
afirma "" "pedidos.id es required (REQUIRED en el origen)" "$([ "$(cat_col ventas.pedidos id required)" = 'true' ] && echo 1)"
st=$(cat_col ventas.pedidos total sourceType)
afirma "[A4]" "pedidos.total cita su fisico (NUMERIC(38, 9)) para que la copia sepa su escala" \
  "$(case "$st" in *NUMERIC*38*9*) echo 1;; esac)" "sourceType=$st"
printf '%s' "$BQ_URL" | "$DRV" catalogo bq > "$TMP/drv-cat.json" 2> "$TMP/drv-cat.err"
afirma "[A3]" "el propio driver contesta \`catalogo\` (el catalogo vive en el driver)" \
  "$(grep -q '"ventas.pedidos"' "$TMP/drv-cat.json" && echo 1)" "$(head -c 120 "$TMP/drv-cat.err")"

echo "── 3 · discover standard y validate"
"$ORE" discover --source bq --type standard --only ventas.pedidos --only ventas.clientes \
  --no-model --owner team:datos --out packages/ventas --name ventas > "$TMP/disc.txt" 2>&1 \
  || falla "discover: $(tail -3 "$TMP/disc.txt")"
if "$ORE" validate . > "$TMP/val.txt" 2>&1; then ok "el arbol compila"
else falla "validate: $(tail -3 "$TMP/val.txt")"; fi

echo "── 4 · materialize"
t0=$(reloj)
"$ORE" materialize . > "$TMP/mat.txt" 2>&1
rc=$?
dice "ore materialize en $(lapso "$t0" "$(reloj)") (rc=$rc)"
puntero() { "$PY" -c 'import json,sys
try: d=json.load(open(sys.argv[1],encoding="utf-8"))
except Exception: print("sin-puntero"); sys.exit()
v=d.get(sys.argv[2]); print(json.dumps(v, sort_keys=True, ensure_ascii=False) if not isinstance(v,str) else v)' "datasets/ventas/ventas/$1.json" "$2"; }
# La etiqueta va con la tabla: clientes no tiene TIMESTAMP y ya estrecha hoy.
for v in pedidos:8:[A2] clientes:5:; do
  t=${v%%:*}; resto=${v#*:}; esperado=${resto%%:*}; et=${resto#*:}
  afirma "" "$t copiada con $esperado filas" \
    "$([ "$(puntero "$t" estado)" = copiada ] && [ "$(puntero "$t" filas)" = "$esperado" ] && echo 1)" \
    "estado=$(puntero "$t" estado) filas=$(puntero "$t" filas) · $(puntero "$t" motivo)"
  se=$(puntero "$t" columnas_sin_estrechar)
  afirma "$et" "$t: ninguna columna se queda sin estrechar" "$([ "$se" = "null" ] || [ "$se" = "{}" ] && echo 1)" "$se"
done

echo "── 5 · la metadata de Iceberg"
tipo_ice() {  # tabla columna campo
  local ml; ml=$(puntero "$1" metadata_location)
  curl -s "$ORE_R2_S3_ENDPOINT/copia/${ml#s3://copia/}" | "$PY" -c 'import json,sys
try: m=json.load(sys.stdin)
except Exception: print("sin-metadata"); sys.exit()
s=[s for s in m["schemas"] if s["schema-id"]==m["current-schema-id"]][0]
f=[f for f in s["fields"] if f["name"]==sys.argv[1]]
print(json.dumps(f[0][sys.argv[2]]) if f else "sin-columna")' "$2" "$3"
}
afirma "[A2]" "pedidos.ts es timestamptz" "$([ "$(tipo_ice pedidos ts type)" = '"timestamptz"' ] && echo 1)" "es $(tipo_ice pedidos ts type)"
afirma "[A4]" "pedidos.total es decimal(38, 9)" "$([ "$(tipo_ice pedidos total type)" = '"decimal(38, 9)"' ] && echo 1)" "es $(tipo_ice pedidos total type)"
afirma "" "clientes.alta es date" "$([ "$(tipo_ice clientes alta type)" = '"date"' ] && echo 1)" "es $(tipo_ice clientes alta type)"
afirma "[A4]" "pedidos.id es required en Iceberg" "$([ "$(tipo_ice pedidos id required)" = 'true' ] && echo 1)" "es $(tipo_ice pedidos id required)"

echo "── 6 · lo que se relee es lo sembrado"
cat > "$TMP/esperado-pedidos.jsonl" <<'J'
{"cliente_id":"ore-e2e-c1","id":"ore-e2e-p1","total":"19.99","ts":"2026-09-01 10:00:00+00"}
{"cliente_id":"ore-e2e-c1","id":"ore-e2e-p2","total":"0","ts":"2026-09-02 23:59:59.123456+00"}
{"cliente_id":"ore-e2e-c2","id":"ore-e2e-p3","total":"-5.5","ts":"1970-01-01 00:00:00+00"}
{"cliente_id":"ore-e2e-c2","id":"ore-e2e-p4","total":"12345678901234567890.123456789","ts":"2026-09-03 10:00:00+00"}
{"cliente_id":"ore-e2e-c3","id":"ore-e2e-p5"}
{"id":"ore-e2e-p6","total":"100","ts":"2026-09-04 08:30:00+00"}
{"cliente_id":"ore-e2e-c4","id":"ore-e2e-p7","total":"0.000000001","ts":"2026-09-05 00:00:00+00"}
{"cliente_id":"ore-e2e-c5","id":"ore-e2e-p8","total":"7","ts":"2026-09-26 18:00:00+00"}
J
cat > "$TMP/esperado-clientes.jsonl" <<'J'
{"alta":"2024-01-15","email":"ana@ejemplo.test","id":"ore-e2e-c1","pais":"ES"}
{"alta":"1999-12-31","email":"luis@ejemplo.test","id":"ore-e2e-c2","pais":"FR"}
{"email":"eva@ejemplo.test","id":"ore-e2e-c3"}
{"alta":"2026-09-26","email":"null@ejemplo.test","id":"ore-e2e-c4","pais":"null"}
{"alta":"2000-02-29","email":"ñandú@ejemplo.test","id":"ore-e2e-c5","pais":"PT"}
J
releer() {  # tabla → filas ore-e2e-* ordenadas, en JSON canonico
  printf '{"metadata_location":"%s"}\n' "$(puntero "$1" metadata_location)" | "$STORE" leer 2>/dev/null \
  | "$PY" -c 'import json,sys
filas=[]
for l in sys.stdin:
    try: d=json.loads(l)
    except Exception: continue
    if str(d.get("id","")).startswith("ore-e2e-"): filas.append(d)
for d in sorted(filas, key=lambda d: d["id"]): print(json.dumps(d, sort_keys=True, ensure_ascii=False, separators=(",",":")))'
}
compara() {  # tabla etiqueta id campo descripcion
  local real esperado
  real=$(releer "$1" | "$PY" -c 'import json,sys
for l in sys.stdin:
    d=json.loads(l)
    if d["id"]==sys.argv[1]: print(json.dumps(d.get(sys.argv[2]), ensure_ascii=False))' "$3" "$4")
  esperado=$("$PY" -c 'import json,sys
for l in open(sys.argv[1],encoding="utf-8"):
    d=json.loads(l)
    if d["id"]==sys.argv[2]: print(json.dumps(d.get(sys.argv[3]), ensure_ascii=False))' "$TMP/esperado-$1.jsonl" "$3" "$4")
  afirma "$2" "$5" "$([ "$real" = "$esperado" ] && echo 1)" "esperaba $esperado, salio $real"
}
compara clientes "[A2]" ore-e2e-c4 pais   "el texto 'null' sigue siendo texto"
compara pedidos  "[A2]" ore-e2e-p2 ts     "los microsegundos llegan (23:59:59.123456)"
compara pedidos  "[A2]" ore-e2e-p4 ts     "un TIMESTAMP con +02 llega como instante UTC"
compara pedidos  ""     ore-e2e-p4 total  "un NUMERIC de 29 cifras llega exacto"
compara pedidos  ""     ore-e2e-p5 total  "un NULL se queda ausente"
compara clientes ""     ore-e2e-c5 email  "el UTF-8 llega entero (ñandú)"
for t in pedidos clientes; do
  releer "$t" > "$TMP/real-$t.jsonl"
  # En JSON canonico y sin fijarse en el fin de linea (Python en Windows escribe CRLF).
  dif=$("$PY" -c 'import json,sys
c=lambda f:[json.dumps(json.loads(l),sort_keys=True,ensure_ascii=False,separators=(",",":")) for l in open(f,encoding="utf-8") if l.strip()]
r,e=c(sys.argv[1]),c(sys.argv[2])
print(" | ".join(f"{x} ≠ {y}" for x,y in zip(e,r) if x!=y) + ("" if len(r)==len(e) else f" | {len(r)} filas y no {len(e)}"))' \
    "$TMP/real-$t.jsonl" "$TMP/esperado-$t.jsonl")
  afirma "[A2]" "$t: la copia entera es exactamente lo sembrado" "$([ -z "$dif" ] && echo 1)" "$dif"
done

echo
if [ "$fallos" -eq 0 ]; then echo "bigquery-real · todo en verde"; exit 0; fi
echo "bigquery-real · $fallos aserciones en rojo (la etiqueta dice que paso de la Fase A las apaga)"
exit 1
