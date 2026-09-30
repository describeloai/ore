#!/usr/bin/env bash
# LAS COLECCIONES EN UNA RAMA (0044 C D7). Una base estándar con una colección
# mantenida sobre un S3 de mentira —que hace a la vez de origen y de lago—.
#
# D7a · la recogida de blobs cuenta con las demás ramas:
#   1  la colección con dos PDF: dos blobs en el lago
#   2  su puntero se guarda como el de OTRA rama; aquí la colección pasa a pedir
#      sólo a.pdf (con `retention: 1s`), y b.pdf caduca
#   3  sin `--reclaman`, la recogida (en seco) se llevaría el blob de b.pdf
#   4  con `--reclaman`: no se lleva nada, y dice que otra rama lo nombra
#   5  un `--reclaman` que no es un directorio: 66, y nada se toca
#   6  el manifiesto de otra rama que no se lee: 69, y ningún blob se recoge
#      (la retención de aquí sí corre: es de esta rama)
#   7  de verdad, con `--reclaman`: el blob sigue en el lago
#   8  y de verdad sin él: se va (lo que 7 afirma no era gratis)
#
# Uso:  bash pruebas-de-fuego/las-colecciones-en-una-rama.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python)
TMP="${TMPDIR:-/tmp}/ore-colecciones-rama-$$"
rm -rf "$TMP"; mkdir -p "$TMP"
S3_PID=""
limpiar() { [ -n "$S3_PID" ] && kill "$S3_PID" 2>/dev/null; [ -n "${ORE_GUARDAR:-}" ] || rm -rf "$TMP"; }
trap limpiar EXIT
export PYTHONUTF8=1

fallos=0
ok()    { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '  \xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }
afirma() { if [ "$2" = "1" ]; then ok "$1"; else falla "$1${3:+ · $3}"; fi; }

buscar() {
  local n
  for n in ${ORE_BIN:+"$ORE_BIN/$1" "$ORE_BIN/$1.exe"} \
           "$RAIZ/target/debug/$1" "$RAIZ/target/debug/$1.exe" \
           "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"             || { echo "no hay binario de \`ore\`"; exit 2; }
DRV="$(buscar ore-read-s3)"     || { echo "no hay binario de \`ore-read-s3\`"; exit 2; }
STORE="$(buscar ore-store-r2)"  || { echo "no hay binario de \`ore-store-r2\`"; exit 2; }
export PATH="$(dirname "$DRV"):$(dirname "$STORE"):$PATH"

"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; exit 2; }
S3="http://127.0.0.1:$S3_PUERTO"
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="$S3" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"
# El origen es el mismo S3: dos PDF en la raíz (el lago vive bajo `ore/`).
export ORE_S3_URL="s3://copia/?endpoint=$S3&region=auto&access_key_id=de&secret_access_key=mentira"
"$PY" - "$S3" <<'PYEOF'
import sys, urllib.request
for k, b in (("a.pdf", b"%PDF-1.4 a"), ("b.pdf", b"%PDF-1.4 b, otra cosa")):
    urllib.request.urlopen(urllib.request.Request(sys.argv[1] + "/copia/" + k, data=b, method="PUT",
                                                  headers={"content-type": "application/pdf"}))
PYEOF

blobs_en_el_lago() {
  "$PY" -c 'import urllib.request,re,sys
x=urllib.request.urlopen(sys.argv[1]+"/copia?list-type=2&prefix=ore/v2/blobs/sha256/").read().decode()
print(len(re.findall("<Key>",x)))' "$S3"; }

A="$TMP/arbol"; mkdir -p "$A"; cd "$A"
"$ORE" init . --name ficheros >/dev/null 2>&1 || { echo "ore init falló"; exit 2; }
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

echo "── 1 · la colección, con sus dos blobs"
"$ORE" source catalog s3_demo --out "$TMP/catalogo.json" > "$TMP/cat.txt" 2>&1 \
  || falla "catalog: $(tail -2 "$TMP/cat.txt")"
"$ORE" discover --source s3_demo --type standard --only default.raiz \
  --no-model --owner team:datos --out packages/docs --name docs > "$TMP/disc.txt" 2>&1 \
  || falla "discover: $(tail -3 "$TMP/disc.txt")"
F=$(find packages/docs -path "*collections*" -name "raiz*.yaml" | head -1)
[ -n "$F" ] || { echo "no salió la colección: $(cat "$TMP/disc.txt")"; exit 1; }
PDF=$(basename "$F" .yaml)
"$ORE" materialize . > "$TMP/m1.txt" 2>&1
afirma "materialize: dos blobs en el lago" "$([ $? = 0 ] && [ "$(blobs_en_el_lago)" = 2 ] && echo 1)" \
  "$(blobs_en_el_lago) · $(tail -4 "$TMP/m1.txt")"
P=$(find datasets -name "$PDF.json" | head -1)

echo "── 2 · otra rama se queda con los dos; aquí sólo a.pdf, y b.pdf caduca"
mkdir -p "$TMP/ramas/datasets/docs/default"
cp "$P" "$TMP/ramas/datasets/docs/default/$PDF.json"
"$PY" - "$F" <<'PYEOF'
import re,sys
p=sys.argv[1]; t=open(p,encoding="utf-8").read()
t=re.sub(r"from: \{ objectTable: ([^ }]+) \}", r'from: { objectTable: \1, match: "a.pdf" }', t)
t=t.replace("  formats:", "  retention: 1s\n  formats:", 1)
open(p,"w",encoding="utf-8").write(t)
PYEOF
"$ORE" materialize . > "$TMP/m2.txt" 2>&1
afirma "b.pdf se retira aquí" "$([ $? = 0 ] && grep -q '"retiran": *1' "$P" && echo 1)" "$(tail -3 "$TMP/m2.txt")"
sleep 2

echo "── 3 · sin lo que reclaman las demás ramas"
"$ORE" collections . --recoger --gracia 0 --seco > "$TMP/r1.txt" 2>&1
afirma "en seco se llevaría el blob de b.pdf" \
  "$(grep -q 'en seco · blobs: 2 en el lago, 1 vivos, 0 en su gracia, 1 recogidos' "$TMP/r1.txt" && echo 1)" "$(cat "$TMP/r1.txt")"

echo "── 4 · con --reclaman"
"$ORE" collections . --recoger --gracia 0 --seco --reclaman "$TMP/ramas" > "$TMP/r2.txt" 2>&1
afirma "no se lleva nada, y dice que otra rama nombra lo suyo" \
  "$(grep -q 'de las demás ramas: 1 colección' "$TMP/r2.txt" && grep -q 'en seco · blobs: 2 en el lago, 2 vivos, 0 en su gracia, 0 recogidos' "$TMP/r2.txt" && echo 1)" "$(cat "$TMP/r2.txt")"

echo "── 5 · un --reclaman que no es un directorio"
"$ORE" collections . --recoger --gracia 0 --reclaman "$TMP/no-esta" > "$TMP/r3.txt" 2>&1
codigo=$?
afirma "66, y el lago sigue con dos" "$([ "$codigo" = 66 ] && [ "$(blobs_en_el_lago)" = 2 ] && echo 1)" "$codigo · $(cat "$TMP/r3.txt")"

echo "── 6 · el manifiesto de otra rama no se lee"
mkdir -p "$TMP/ramas-rotas/datasets/docs/default"
"$PY" - "$TMP/ramas/datasets/docs/default/$PDF.json" "$TMP/ramas-rotas/datasets/docs/default/$PDF.json" <<'PYEOF'
import json,sys
d=json.load(open(sys.argv[1],encoding="utf-8"))
d["metadata_location"]=d["metadata_location"].replace("/metadata/","/metadata/no-esta-")
open(sys.argv[2],"w",encoding="utf-8").write(json.dumps(d))
PYEOF
"$ORE" collections . --recoger --gracia 0 --reclaman "$TMP/ramas-rotas" > "$TMP/r4.txt" 2>&1
codigo=$?
afirma "69, no se recoge ningún blob" "$([ "$codigo" = 69 ] && [ "$(blobs_en_el_lago)" = 2 ] && grep -q 'de otra rama' "$TMP/r4.txt" && echo 1)" "$codigo · $(cat "$TMP/r4.txt")"

echo "── 7 · de verdad, con --reclaman"
"$ORE" collections . --recoger --gracia 0 --reclaman "$TMP/ramas" > "$TMP/r5.txt" 2>&1
codigo=$?
afirma "el blob que sólo nombra la otra rama sigue en el lago" \
  "$([ "$codigo" = 0 ] && grep -q '2 en el lago, 2 vivos, 0 en su gracia, 0 recogidos' "$TMP/r5.txt" && [ "$(blobs_en_el_lago)" = 2 ] && echo 1)" "$codigo · $(cat "$TMP/r5.txt")"

echo "── 8 · y sin ella, se va"
"$ORE" collections . --recoger --gracia 0 > "$TMP/r6.txt" 2>&1
afirma "sin --reclaman, el blob de b.pdf se recoge: queda uno" \
  "$([ $? = 0 ] && grep -q '1 recogidos' "$TMP/r6.txt" && [ "$(blobs_en_el_lago)" = 1 ] && echo 1)" "$(cat "$TMP/r6.txt")"

echo
if [ "$fallos" -eq 0 ]; then echo "las colecciones en una rama · todo en verde"; exit 0; fi
echo "las colecciones en una rama · $fallos aserciones en rojo"
exit 1
