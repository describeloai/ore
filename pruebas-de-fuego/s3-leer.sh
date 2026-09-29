#!/usr/bin/env bash
# S3 DE PUNTA A PUNTA, LAS FILAS (0046 E6): una base standard sobre el bucket de
# la medida F1 con los datasets de Olist copiados al lago, y las filas cuadradas.
# SOLO LECTURA sobre el bucket; el lago es un S3 de mentira.
#
#   1  el catálogo: cada CSV y JSONL trae `_rescued_data` (03 §1.1); el Parquet no
#   2  discover standard de las doce tablas + validate: el árbol compila
#   3  materialize: cada dataset copiado con EXACTAMENTE las filas que F1/E6
#      contaron con pyarrow (1.000.163 en geolocation; 99.224 reseñas con 5.495
#      saltos de línea dentro de comillas), ninguna columna sin estrechar
#   4  los tipos de Iceberg: los congelados en la Table (una fecha de Olist es
#      `timestamp`, `total` de pedidos `decimal(12, 2)`, el código postal texto),
#      y la rescatada en la copia (`rescued_data`: la propiedad no empieza por `_`)
#   5  lo que se relee de las reseñas: el vacío sin comillas es nulo (87.656
#      títulos), el salto de línea dentro de un comentario llega, y nada se
#      rescató
#   6  otra pasada con el bucket igual: nada que leer (el testigo del listado)
#
# Uso:  ORE_S3_URL='s3://<bucket>/?region=…&access_key_id=…&secret_access_key=…' \
#         bash pruebas-de-fuego/s3-leer.sh
# La URL no se imprime nunca. Mejor con binarios de release (`ORE_BIN`): son
# 1,2 M de filas.
set -u

if [ -z "${ORE_S3_URL:-}" ]; then
  echo "se salta · sin ORE_S3_URL no hay bucket contra el que leer"
  exit 0
fi

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python)
TMP="${TMPDIR:-/tmp}/ore-s3-leer-$$"
rm -rf "$TMP"; mkdir -p "$TMP"
S3_PID=""
# `ORE_GUARDAR=1` deja el árbol y lo leído en `$TMP` para mirarlo.
limpiar() { [ -n "$S3_PID" ] && kill "$S3_PID" 2>/dev/null; [ -n "${ORE_GUARDAR:-}" ] || rm -rf "$TMP"; }
trap limpiar EXIT
export PYTHONUTF8=1

fallos=0
ok()    { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '  \xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }
dice()  { printf '  \xc2\xb7 %s\n' "$1"; }
afirma() { if [ "$2" = "1" ]; then ok "$1"; else falla "$1${3:+ · $3}"; fi; }

buscar() {
  local n
  for n in ${ORE_BIN:+"$ORE_BIN/$1" "$ORE_BIN/$1.exe"} \
           "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"             || { echo "no hay binario de \`ore\`"; exit 2; }
DRV="$(buscar ore-read-s3)"     || { echo "no hay binario de \`ore-read-s3\`"; exit 2; }
STORE="$(buscar ore-store-r2)"  || { echo "no hay binario de \`ore-store-r2\`"; exit 2; }
export PATH="$(dirname "$DRV"):$(dirname "$STORE"):$PATH"

# Lo que salga no puede llevar la credencial.
limpio() { "$PY" - "$1" <<'PYEOF'
import os, sys, urllib.parse
q = urllib.parse.parse_qs(os.environ["ORE_S3_URL"].split("?", 1)[1])
t = open(sys.argv[1], encoding="utf-8", errors="replace").read()
sys.exit(1 if any(v and v in t for k in ("access_key_id", "secret_access_key") for v in q.get(k, [])) else 0)
PYEOF
}

# ── el S3 de mentira (el lago) ───────────────────────────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; cat "$TMP/s3.log"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"

reloj() { "$PY" -c 'import time; print(time.time())'; }
lapso() { "$PY" -c "print(f'{$2 - $1:.1f} s')"; }

# ── el árbol ─────────────────────────────────────────────────────────────────
A="$TMP/arbol"; mkdir -p "$A"; cd "$A"
"$ORE" init . --name olist >/dev/null 2>&1 || { echo "ore init falló"; exit 2; }
printf 'datasources:\n  - { name: s3_demo, type: s3, connectionEnv: ORE_S3_URL }\n' >> ontology.config.yaml
cat > conduits.yaml <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: default }
spec:
  owner: team:datos
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y

echo "── 1 · el catálogo"
t0=$(reloj)
"$ORE" source catalog s3_demo --out "$TMP/catalogo.json" > "$TMP/cat.txt" 2>&1 \
  || falla "ore source catalog: $(tail -2 "$TMP/cat.txt")"
limpio "$TMP/cat.txt" && limpio "$TMP/catalogo.json" || falla "1 · el catálogo imprimió la credencial"
dice "ore source catalog en $(lapso "$t0" "$(reloj)")"
rescata=$("$PY" -c 'import json,sys
d=json.load(open(sys.argv[1],encoding="utf-8"))
r=lambda t: any(c["name"]=="_rescued_data" and c.get("type")=="String" for c in t["columns"])
print(sum(1 for t in d["tables"] if t["format"]["type"]!="parquet" and r(t)), sum(1 for t in d["tables"] if t["format"]["type"]=="parquet" and r(t)))' "$TMP/catalogo.json")
afirma "los once CSV/JSONL traen \`_rescued_data\` y el Parquet no" "$([ "$rescata" = "11 0" ] && echo 1)" "$rescata"
TABLAS=$("$PY" -c 'import json,sys; print(" ".join("--only "+t["name"] for t in json.load(open(sys.argv[1],encoding="utf-8"))["tables"]))' "$TMP/catalogo.json")

echo "── 2 · discover standard y validate"
# shellcheck disable=SC2086
"$ORE" discover --source s3_demo --type standard $TABLAS \
  --no-model --owner team:datos --out packages/olist --name olist > "$TMP/disc.txt" 2>&1 \
  || falla "discover: $(tail -3 "$TMP/disc.txt")"
if "$ORE" validate . > "$TMP/val.txt" 2>&1; then ok "el árbol compila"
else falla "validate: $(tail -3 "$TMP/val.txt")"; fi

echo "── 3 · materialize"
t0=$(reloj)
"$ORE" materialize . > "$TMP/mat.txt" 2>&1
rc=$?
dice "ore materialize en $(lapso "$t0" "$(reloj)") (rc=$rc)"
limpio "$TMP/mat.txt" || falla "3 · materialize imprimió la credencial"
afirma "materialize termina bien" "$([ "$rc" = 0 ] && echo 1)" "$(tail -5 "$TMP/mat.txt")"
# El puntero de cada copia, por el final de su nombre (la inducción pone el
# schema y el nombre; aquí basta con que sea único).
puntero() { "$PY" -c 'import json,sys,glob
fs=[f for f in glob.glob("datasets/**/*.json",recursive=True) if f.replace("\\","/").endswith("/"+sys.argv[1]+".json")]
if len(fs)!=1: print("sin-puntero" if not fs else "ambiguo"); sys.exit()
v=json.load(open(fs[0],encoding="utf-8")).get(sys.argv[2])
print(json.dumps(v, sort_keys=True, ensure_ascii=False) if not isinstance(v,str) else v)' "$1" "$2"; }
for v in olist_customers_dataset:99441 olist_geolocation_dataset:1000163 \
         olist_order_items_dataset:112650 olist_order_payments_dataset:103886 \
         olist_order_reviews_dataset:99224 olist_orders_dataset:99441 \
         olist_products_dataset:32951 olist_sellers_dataset:3095 \
         product_category_name_translation:71 logs:600 clientes:80 pedidos:1500; do
  t=${v%%:*}; esperado=${v#*:}
  afirma "$t: $esperado filas" \
    "$([ "$(puntero "$t" estado)" = copiada ] && [ "$(puntero "$t" filas)" = "$esperado" ] && echo 1)" \
    "estado=$(puntero "$t" estado) filas=$(puntero "$t" filas) · $(puntero "$t" motivo)"
  se=$(puntero "$t" columnas_sin_estrechar)
  [ "$se" = "null" ] || [ "$se" = "{}" ] || falla "$t: columnas sin estrechar $se"
done

echo "── 4 · los tipos de Iceberg"
tipo_ice() {  # tabla columna
  local ml; ml=$(puntero "$1" metadata_location)
  curl -s "$ORE_R2_S3_ENDPOINT/copia/${ml#s3://copia/}" | "$PY" -c 'import json,sys
try: m=json.load(sys.stdin)
except Exception: print("sin-metadata"); sys.exit()
s=[s for s in m["schemas"] if s["schema-id"]==m["current-schema-id"]][0]
f=[f for f in s["fields"] if f["name"]==sys.argv[1]]
print(f[0]["type"] if f else "sin-columna")' "$2"
}
for v in olist_orders_dataset:order_purchase_timestamp:timestamp \
         olist_geolocation_dataset:geolocation_lat:double \
         olist_customers_dataset:customer_zip_code_prefix:string \
         olist_order_reviews_dataset:review_score:long \
         pedidos:total:"decimal(12, 2)" pedidos:ts:timestamptz pedidos:fecha:string \
         logs:ts:timestamptz olist_orders_dataset:rescued_data:string; do
  t=${v%%:*}; resto=${v#*:}; c=${resto%%:*}; esperado=${resto#*:}
  real=$(tipo_ice "$t" "$c")
  afirma "$t.$c es $esperado" "$([ "$real" = "$esperado" ] && echo 1)" "es $real"
done

echo "── 5 · lo que se relee de las reseñas"
printf '{"metadata_location":"%s"}\n' "$(puntero olist_order_reviews_dataset metadata_location)" \
  | "$STORE" leer > "$TMP/resenas.jsonl" 2>/dev/null
cuenta=$("$PY" -c 'import json,sys
n=t=m=r=0
for l in open(sys.argv[1],encoding="utf-8"):
    try: d=json.loads(l)
    except Exception: continue
    if "review_id" not in d: continue   # la cabecera
    n+=1
    t+= d.get("review_comment_title") is None
    m+= "\n" in (d.get("review_comment_message") or "")
    r+= d.get("rescued_data") is not None
print(n, t, m, r)' "$TMP/resenas.jsonl")
set -- $cuenta
afirma "se releen 99224 reseñas" "$([ "${1:-}" = 99224 ] && echo 1)" "$cuenta"
afirma "87656 títulos vacíos sin comillas son nulos (03 §1.2)" "$([ "${2:-}" = 87656 ] && echo 1)" "$cuenta"
afirma "los saltos de línea dentro de un comentario llegan" "$([ "${3:-0}" -gt 0 ] && echo 1)" "$cuenta"
afirma "nada rescatado: los tipos congelados encajan en Olist" "$([ "${4:-}" = 0 ] && echo 1)" "$cuenta"

echo "── 6 · otra pasada, con el bucket igual"
t0=$(reloj)
"$ORE" materialize . > "$TMP/mat2.txt" 2>&1
rc=$?
dice "segunda pasada en $(lapso "$t0" "$(reloj)") (rc=$rc)"
afirma "no lee nada: el testigo del listado no cambió" \
  "$([ "$rc" = 0 ] && ! grep -q "leidas" "$TMP/mat2.txt" && echo 1)" "$(grep -m3 "leidas" "$TMP/mat2.txt")"

echo
if [ "$fallos" -eq 0 ]; then echo "s3-leer · todo en verde"; exit 0; fi
echo "s3-leer · $fallos aserciones en rojo"
exit 1
