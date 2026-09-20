#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# LA INVOCACIÓN SE DECIDE — F4a·I2 (ADR 0029): una `Function` de lectura sobre la
# copia, de punta a punta y sin red de verdad.
#
#   `ore materialize` (jsonl → S3 de mentira)  →  `ore invoke`:
#       `ore-store-r2 leer`   trae las 12 filas de la copia por su puntero
#       `ore-invoke`          las lleva al vLLM de mentira, una llamada por fila
#       `ore-store-r2 sellar` sella el resultado como un dataset en el mismo bucket
#       `--informe`           deja los números y una muestra en el árbol, y el
#                             puntero del dataset de resultados
#
# Lo que afirma:
#   1  la copia se relee por su puntero y vuelve entera (I1)
#   2  cada fila va al modelo con `temperature: 0` y la forma de `output` pedida
#   3  el resultado es un dataset (0031 §10: `resultados/<p>_<f>`) con la copia
#      leída en su cabecera, y la segunda corrida es un snapshot más del mismo
#   4  una fila que el modelo no contesta bien sale como error, y la corrida sigue
#   5  el informe dice filas, ok, errores, tokens, ms y la muestra
#   6  lo que se niega: sin copia hecha, con `effects`, `runtime: wasm`, un
#      `output` que se llama como un campo de la copia, sin puerta
#   7  la puerta pide identidad: sin `MODELO_TOKEN` todas las filas fallan y se dice
#
# Necesita `ore`, `ore-store-r2`, `ore-read-jsonl` y `ore-invoke` en el PATH o en
# target/{release,debug}, y python3.
# ══════════════════════════════════════════════════════════════════════════════
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python) || { echo "hace falta python"; exit 2; }

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  command -v "$1"
}
ORE="$(buscar ore)" || { echo "no hay binario de \`ore\`"; exit 2; }
for b in ore-store-r2 ore-read-jsonl ore-invoke; do
  B="$(buscar $b)" || { echo "no hay binario de \`$b\` — cargo build -p ore-store -p ore-read-jsonl -p ore-invoke"; exit 2; }
  export PATH="$(dirname "$B"):$PATH"
done

fallos=0
ok()   { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '\xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }
dice() { printf '  \xc2\xb7 %s\n' "$1"; }

TMP="${TMPDIR:-/tmp}/ore-invocacion-$$"
rm -rf "$TMP"; mkdir -p "$TMP/datos" "$TMP/arbol"
limpiar() { kill "$S3_PID" "$VLLM_PID" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT

# ── los dos servidores de mentira ────────────────────────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
EXIGE_TOKEN=1 "$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" vllm 0 > "$TMP/vllm.log" 2>&1 & VLLM_PID=$!
for i in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && grep -q listo "$TMP/vllm.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log"); VLLM_PUERTO=$(awk '{print $2}' "$TMP/vllm.log")
[ -n "$S3_PUERTO" ] && [ -n "$VLLM_PUERTO" ] || { echo "los servidores de mentira no arrancaron"; cat "$TMP/s3.log" "$TMP/vllm.log"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira
PUERTA="http://127.0.0.1:$VLLM_PUERTO/v1"

# ── el árbol: una base estándar sin entidades, su copia, y la función ────────
A="$TMP/arbol"
mkdir -p "$A/packages/olist_copia/tables" "$A/packages/olist_copia/views" "$A/packages/olist_copia/functions" "$A/modelos"
cat > "$TMP/datos/categorias.jsonl" <<'J'
{"product_category_name":"beleza_saude","product_category_name_english":"health_beauty"}
{"product_category_name":"informatica_acessorios","product_category_name_english":"computers_accessories"}
{"product_category_name":"automotivo","product_category_name_english":"auto"}
{"product_category_name":"cama_mesa_banho","product_category_name_english":"bed_bath_table"}
{"product_category_name":"moveis_decoracao","product_category_name_english":"furniture_decor"}
{"product_category_name":"esporte_lazer","product_category_name_english":"sports_leisure"}
{"product_category_name":"perfumaria","product_category_name_english":"perfumery"}
{"product_category_name":"utilidades_domesticas","product_category_name_english":"housewares"}
{"product_category_name":"telefonia","product_category_name_english":"telephony"}
{"product_category_name":"relogios_presentes","product_category_name_english":"watches_gifts"}
{"product_category_name":"rompe_todo","product_category_name_english":"breaks"}
{"product_category_name":"bebes","product_category_name_english":"baby"}
J
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: pg, type: jsonl, connectionEnv: FICHEROS_DIR }
  - { name: copia, type: jsonl, connectionEnv: FICHEROS_DIR }
Y
cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:data
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y
cat > "$A/modelos/v2-lite.yaml" <<'Y'
apiVersion: oos.dev/v1alpha9
kind: Model
metadata: { name: v2-lite }
spec:
  profile: g1/deepseek-v2-lite
  tier: shared
  task: chat
Y
cat > "$A/packages/olist_copia/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: olist_copia, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$A/packages/olist_copia/tables/product_category_name_translation.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: olist_product_category_name_translation, namespace: olist_copia }
spec:
  datasource: pg
  object: "categorias.jsonl"
  columns:
    product_category_name: {}
    product_category_name_english: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
cat > "$A/packages/olist_copia/views/productCategoryNameTranslation.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: productCategoryNameTranslation, namespace: olist_copia }
spec:
  owner: team:data
  from: { table: olist_copia.olist_product_category_name_translation }
  fields:
    productCategoryName: product_category_name
    productCategoryNameEnglish: product_category_name_english
  materialized: { datasource: copia, table: "copia.productCategoryNameTranslation" }
Y
cat > "$A/packages/olist_copia/views/sinCopia.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: sinCopia, namespace: olist_copia }
spec:
  owner: team:data
  from: { table: olist_copia.olist_product_category_name_translation }
  fields:
    productCategoryName: product_category_name
Y
funcion() { # nombre over extra
  cat > "$A/packages/olist_copia/functions/$1.yaml" <<Y
apiVersion: oos.dev/v1alpha10
kind: Function
metadata: { name: $1, namespace: olist_copia }
spec:
  runtime: model
  model: modelo/v2-lite
  over: $2
  prompt: "Traduce la categoría al español en una o dos palabras."
  output:
    categoriaEs: { type: String }
$3
Y
}
funcion traducirCategoria olist_copia.productCategoryNameTranslation ""
export FICHEROS_DIR="$TMP/datos"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || { "$ORE" validate "$A"; falla "el árbol no compila"; exit 1; }

# ── 0 · la copia, hecha ──────────────────────────────────────────────────────
salida=$("$ORE" materialize "$A" --informe "$A/copias" 2>&1) || { echo "$salida"; falla "0 · materialize"; exit 1; }
CLAVE=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata_location"])' "$A/copias/olist_copia_productCategoryNameTranslation.json")
[ -n "$CLAVE" ] || { falla "0 · el puntero de la copia no tiene metadata_location"; exit 1; }
# y cuenta por columna (medida W1 §B): en demo una copia «copiada» tenía 2 de 9 columnas
"$PY" - "$A/copias/olist_copia_productCategoryNameTranslation.json" <<'EOF' || { falla "0 · el informe no cuenta las columnas"; exit 1; }
import json, sys
i = json.load(open(sys.argv[1]))
assert i["columnas"] == {"productCategoryName": i["filas"], "productCategoryNameEnglish": i["filas"]}, i.get("columnas")
EOF
dice "0 · copia hecha · $CLAVE"

# ── 1 · I1: la copia vuelve por su nombre ────────────────────────────────────
leida=$(printf '{"metadata_location":"%s"}\n' "$CLAVE" | ore-store-r2 leer) || { falla "1 · leer falló"; exit 1; }
n=$(echo "$leida" | grep -c '^{"productCategoryName"')
[ "$n" = "12" ] || falla "1 · leer devolvió $n filas, no 12"
echo "$leida" | head -1 | grep -q '"plan":"sha256:' || falla "1 · la primera línea no es la cabecera: $(echo "$leida" | head -c 120)"
echo "$leida" | grep -q '"productCategoryName":"perfumaria","productCategoryNameEnglish":"perfumery"' || falla "1 · las filas no llevan los campos de la vista"
printf '{"clave":"ore/v1/nadie"}\n' | ore-store-r2 leer >/dev/null 2>"$TMP/e" && falla "1 · leer lo que no está no falló"
grep -q "no está en el almacén" "$TMP/e" || falla "1 · leer lo que no está no lo dice: $(cat "$TMP/e")"
dice "1 · I1: \`ore-store-r2 leer\` devuelve cabecera + 12 filas por el puntero; lo que no está se dice"

# ── 2·3·4·5 · la invocación ──────────────────────────────────────────────────
export MODELO_TOKEN=de-mentira
salida=$("$ORE" invoke "$A" --funcion olist_copia.traducirCategoria --puerta "$PUERTA" --modelo de-mentira/v2-lite --informe "$A/resultados" 2>&1) || { echo "$salida"; falla "2 · ore invoke falló"; exit 1; }
echo "$salida" | grep -q "12 fila(s) de la copia" || falla "2 · no leyó las 12 filas: $salida"
echo "$salida" | grep -q "11 ok · 1 error(es)" || falla "4 · esperaba 11 ok y 1 error: $salida"
echo "$salida" | grep -q "no contestó un objeto JSON" || falla "4 · el error de la fila no dice por qué: $salida"
echo "$salida" | grep -q "sellado · s3://copia/ore/v2/resultados/olist_copia_traducirCategoria/metadata/00000-.* el dataset nace" || falla "3 · no selló el dataset de resultados: $salida"
RES=$(echo "$salida" | sed -n 's/.*sellado · \(s3:[^ ]*\.metadata\.json\).*/\1/p')
[ -n "$RES" ] && [ "$RES" != "$CLAVE" ] || falla "3 · el resultado se llama como la copia"
llamadas=$(curl -s "http://127.0.0.1:$VLLM_PUERTO/llamadas")
echo "$llamadas" | grep -q '"llamadas": 12' || falla "2 · el modelo no recibió 12 llamadas: $llamadas"
echo "$llamadas" | grep -q '"temperature": 0' || falla "2 · no fue con temperature 0: $llamadas"
echo "$llamadas" | grep -q 'categoriaEs' || falla "2 · el sistema no pidió la forma de output: $llamadas"
dice "2 · 12 llamadas al modelo, temperature 0, la forma de output en el prompt"
dice "4 · la fila \`rompe_todo\` sale como error y la corrida sigue: 11 ok"
# el resultado, releído: lleva los campos de la copia + output, y su cabecera nombra la copia
res=$(printf '{"metadata_location":"%s"}\n' "$RES" | ore-store-r2 leer) || falla "3 · el resultado no se puede releer"
echo "$res" | head -1 | grep -q "\"conducto\":\"function:olist_copia.traducirCategoria\"" || falla "3 · la cabecera no dice la función: $(echo "$res" | head -1)"
echo "$res" | head -1 | grep -q "\"valor\":\"$CLAVE\"" || falla "3 · la cabecera no nombra la copia leída: $(echo "$res" | head -1)"
echo "$res" | grep -q '"categoriaEs":"Perfumería","productCategoryName":"perfumaria"' || falla "3 · el resultado no lleva fila + output: $(echo "$res" | sed -n 2,3p)"
n=$(echo "$res" | grep -c '^{"categoriaEs"'); [ "$n" = "11" ] || falla "3 · el resultado tiene $n filas, no 11"
dice "3 · el resultado es un dataset: fila + output, cabecera con función y copia"
# el informe
INF=$(ls "$A"/resultados/olist_copia_traducirCategoria_*.json | head -1)
[ -n "$INF" ] || falla "5 · no hay informe"
"$PY" - "$INF" "$CLAVE" "$RES" <<'PY' || falla "5 · el informe no dice lo que tiene que decir"
import json, sys
i = json.load(open(sys.argv[1]))
assert i["funcion"] == "olist_copia.traducirCategoria", i
assert i["filas"] == 12 and i["ok"] == 11 and i["errores"] == 1 and i["estado"] == "parcial", i
assert i["copia"]["metadata_location"] == sys.argv[2] and i["resultado"]["metadata_location"] == sys.argv[3], i
assert i["resultado"]["dataset"] == "resultados/olist_copia_traducirCategoria" and i["resultado"]["operacion"] == "creada", i["resultado"]
assert i["tokens"]["entrada"] > 0 and i["tokens"]["salida"] > 0, i["tokens"]
assert i["ms"]["total"] >= 0 and "por_fila" in i["ms"], i["ms"]
assert len(i["muestra"]) == 5 and "output" in i["muestra"][0] and "fila" in i["muestra"][0], i["muestra"]
assert i["modelo"]["nombre"] == "v2-lite" and i["modelo"]["id"] == "de-mentira/v2-lite", i["modelo"]
assert i["errores_muestra"] and "objeto JSON" in i["errores_muestra"][0], i["errores_muestra"]
assert i["cuando"].endswith("Z") and len(i["cuando"]) == 20, i["cuando"]
PY
# y el puntero del dataset de resultados, al lado de las corridas
"$PY" - "$A/resultados/olist_copia_traducirCategoria.json" "$RES" <<'PY' || falla "5 · el puntero del dataset de resultados"
import json, sys
p = json.load(open(sys.argv[1]))
assert p["estado"] == "copiada" and p["metadata_location"] == sys.argv[2] and p["dataset"] == "resultados/olist_copia_traducirCategoria" and p["filas"] == 11, p
PY
dice "5 · informe: 12 filas · 11 ok · 1 error · tokens · ms · muestra de 5 · estado parcial · y el puntero del dataset"
# la segunda corrida: un snapshot más del mismo dataset, y el puntero se mueve
salida2=$("$ORE" invoke "$A" --funcion olist_copia.traducirCategoria --puerta "$PUERTA" --modelo de-mentira/v2-lite --informe "$A/resultados" 2>&1) || { echo "$salida2"; falla "3 · la segunda corrida falló"; }
echo "$salida2" | grep -q "sellado · s3://copia/ore/v2/resultados/olist_copia_traducirCategoria/metadata/00001-.* snapshot nuevo sobre la corrida anterior" || falla "3 · la segunda corrida no fue un snapshot sobre el mismo dataset: $salida2"
RES2=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata_location"])' "$A/resultados/olist_copia_traducirCategoria.json")
[ "$RES2" != "$RES" ] || falla "3 · el puntero de resultados no se movió"
n=$(printf '{"metadata_location":"%s"}\n' "$RES" | ore-store-r2 leer | grep -c '^{"categoriaEs"'); [ "$n" = "11" ] || falla "3 · la primera corrida ya no se relee entera ($n)"
dice "3 · la segunda corrida: un snapshot más del mismo dataset; la primera sigue legible"
# --limite y --seco
salida3=$("$ORE" invoke "$A" --funcion olist_copia.traducirCategoria --puerta "$PUERTA" --modelo de-mentira/v2-lite --limite 3 --seco 2>&1) || falla "2 · --seco falló: $salida3"
echo "$salida3" | grep -q "3 fila(s) de la copia (límite 3)" || falla "2 · --limite no limita: $salida3"
echo "$salida3" | grep -q "seco · no se llama" || falla "2 · --seco llamó: $salida3"
antes=$(curl -s "http://127.0.0.1:$VLLM_PUERTO/llamadas" | sed -n 's/.*"llamadas": \([0-9]*\).*/\1/p')
[ "$antes" = "24" ] || falla "2 · --seco llamó al modelo ($antes llamadas, esperaba 24)"
dice "2 · --limite 3 --seco: 3 filas, ninguna llamada"

# ── 6 · lo que se niega ──────────────────────────────────────────────────────
niega() { # nombre grep
  local s; s=$("$ORE" invoke "$A" --funcion "olist_copia.$1" --puerta "$PUERTA" --modelo x 2>&1) && falla "6 · $1 no se negó: $s"
  echo "$s" | grep -q "$2" || falla "6 · $1 no dice por qué ($2): $s"
}
funcion sobreSinCopia olist_copia.sinCopia ""
niega sobreSinCopia "no declara copia"
sed -i 's/materialized:.*//' "$A/packages/olist_copia/views/productCategoryNameTranslation.yaml"
cp "$A/packages/olist_copia/functions/traducirCategoria.yaml" "$TMP/f.bak"
# la vista sí declara copia pero el informe no está: se restaura la declaración y se esconde el informe
git -C "$A" init -q 2>/dev/null; sed -i 's/^  fields:/  materialized: { datasource: copia, table: "copia.productCategoryNameTranslation" }\n  fields:/' "$A/packages/olist_copia/views/productCategoryNameTranslation.yaml"
mv "$A/copias" "$A/copias.aparte"
niega traducirCategoria "no está hecha"
mv "$A/copias.aparte" "$A/copias"
funcion conEfectos olist_copia.productCategoryNameTranslation "  effects:
    - writes: olist_copia.Nadie.x"
s=$("$ORE" invoke "$A" --funcion olist_copia.conEfectos --puerta "$PUERTA" --modelo x 2>&1) && falla "6 · con effects no se negó"
echo "$s" | grep -qE "declara \`effects\`|no compila" || falla "6 · con effects no dice por qué: $s"
rm "$A/packages/olist_copia/functions/conEfectos.yaml"
funcion colision olist_copia.productCategoryNameTranslation ""
sed -i 's/categoriaEs: { type: String }/productCategoryName: { type: String }/' "$A/packages/olist_copia/functions/colision.yaml"
niega colision "se llama como un campo"
rm "$A/packages/olist_copia/functions/colision.yaml" "$A/packages/olist_copia/functions/sobreSinCopia.yaml"
s=$(env -u MODELO_URL "$ORE" invoke "$A" --funcion olist_copia.traducirCategoria --modelo x 2>&1) && falla "6 · sin puerta no se negó"
echo "$s" | grep -q "no sé dónde está la puerta" || falla "6 · sin puerta no lo dice: $s"
dice "6 · se niega: over sin copia · copia no hecha · effects · output que colisiona · sin puerta"

# ── 7 · la puerta pide identidad ─────────────────────────────────────────────
s=$(env -u MODELO_TOKEN "$ORE" invoke "$A" --funcion olist_copia.traducirCategoria --puerta "$PUERTA" --modelo de-mentira/v2-lite 2>&1) && falla "7 · sin token no falló: $s"
echo "$s" | grep -q "0 ok · 12 error(es)" || falla "7 · sin token no fallan todas: $s"
echo "$s" | grep -q "la puerta contestó 401" || falla "7 · no dice que la puerta pidió identidad: $s"
echo "$s" | grep -q "nada que sellar" || falla "7 · selló sin filas: $s"
dice "7 · sin MODELO_TOKEN: 401 por fila, nada que sellar, y la corrida falla"

if [ "$fallos" = 0 ]; then echo "✓ la invocación se decide: 0–7"; else echo "✗ $fallos fallo(s)"; exit 1; fi
