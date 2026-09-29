#!/usr/bin/env bash
# LA COLECCIÓN MANTENIDA, DE PUNTA A PUNTA (0046 E8·2): una base estándar
# sobre el bucket de la medida F1 (con el experimento de E7 dentro), cuyas
# colecciones copian los bytes al lago —por contenido, cotejados— y viven
# aunque el origen no. SOLO LECTURA sobre el bucket; el lago es un S3 de
# mentira que coteja el `ChecksumSHA256` como R2.
#
#   1  catálogo + discover standard: tres colecciones mantenidas, y compila
#   2  materialize: la transacción 1 de cada una con sus blobs; lo repetido se
#      baja una vez (`Receipt…`, `a.pdf` y `d.pdf` son un contenido; `c2.jpg` y
#      `eqfadadg.jpg`, otro), y cada blob del lago es el sha256 de sus bytes
#   3  otra pasada: al día, sin bajar nada
#   4  la colección pide sólo `a.pdf`: los otros tres se retiran y su blob
#      sigue en el lago
#   5  sin el `match`: vuelven a entrar sin bajar un byte ni pedir una huella
#   6  un Job que subió los blobs y se cortó antes de sellar (sin puntero ni
#      manifiesto): la pasada siguiente los encuentra en el lago por su huella
#      y no baja nada
#   7  `ore collections --cotejar`: limpio (con la muestra vuelta a hashear);
#      sin un blob, sale con 1 y nombra los ítems que lo usan
#   8  `ore collections --recoger` (E8·3): con `retention: 1s`, lo retirado
#      caduca y el puntero se mueve; el blob que sólo nombraba eso espera su
#      gracia (lo tocó un Job), en seco no se toca nada, sin gracia se va con
#      su huella del índice, lo de las demás colecciones sigue, y si vuelve se
#      baja otra vez
#
# Uso:  ORE_S3_URL='s3://<bucket>/?region=…&access_key_id=…&secret_access_key=…' \
#         bash pruebas-de-fuego/s3-coleccion-mantenida.sh
set -u

if [ -z "${ORE_S3_URL:-}" ]; then
  echo "se salta · sin ORE_S3_URL no hay bucket"
  exit 0
fi

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python)
TMP="${TMPDIR:-/tmp}/ore-s3-mantenida-$$"
rm -rf "$TMP"; mkdir -p "$TMP"
S3_PID=""
# `ORE_GUARDAR=1` deja el árbol en `$TMP` para mirarlo.
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
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"

A="$TMP/arbol"; mkdir -p "$A"; cd "$A"
"$ORE" init . --name ficheros >/dev/null 2>&1 || { echo "ore init falló"; exit 2; }
printf 'datasources:\n  - { name: s3_demo, type: s3, connectionEnv: ORE_S3_URL }\n' >> ontology.config.yaml
# Copiar es un conducto (spec v1alpha16 `02` §6): la base estándar lo autoriza.
cat > conduits.yaml <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: default }
spec:
  owner: team:datos
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y

echo "── 1 · catálogo y discover standard"
"$ORE" source catalog s3_demo --out "$TMP/catalogo.json" > "$TMP/cat.txt" 2>&1 \
  || falla "catalog: $(tail -2 "$TMP/cat.txt")"
"$ORE" discover --source s3_demo --type standard \
  --only default.raiz_pdf --only default.raiz_jpg --only nueva_carpeta.contratos \
  --no-model --owner team:datos --out packages/docs --name docs > "$TMP/disc.txt" 2>&1 \
  || falla "discover: $(tail -3 "$TMP/disc.txt")"
N=$(find packages/docs -path "*collections*" -name "*.yaml" | wc -l | tr -d ' ')
afirma "tres colecciones mantenidas" "$([ "$N" = 3 ] && ! grep -rq "virtual: true" packages/docs && echo 1)" "$N"
if "$ORE" validate . > "$TMP/val.txt" 2>&1; then ok "el árbol compila (con su conducto)"
else falla "validate: $(tail -3 "$TMP/val.txt")"; fi
col() { find packages/docs -path "*collections*" -name "$1.yaml" | head -1; }
PDF=$(basename "$(col 'raiz_pdf*')" .yaml)
JPG=$(basename "$(col 'raiz_jpg*')" .yaml)
CON=$(basename "$(col 'contratos*')" .yaml)

puntero() { "$PY" -c 'import json,sys,glob
fs=[f for f in glob.glob("datasets/**/*.json",recursive=True) if f.replace("\\","/").endswith("/"+sys.argv[1]+".json")]
d=json.load(open(fs[0],encoding="utf-8")) if len(fs)==1 else {}
v=d
for k in sys.argv[2].split("."): v=v.get(k) if isinstance(v,dict) else None
print(json.dumps(v) if not isinstance(v,str) else v)' "$1" "$2"; }
manifiesto() {  # colección → filas "camino estado huella blob tipo", ordenadas
  printf '{"dataset":"%s","metadata_location":"%s"}\n' "$(puntero "$1" dataset)" "$(puntero "$1" metadata_location)" \
    | "$STORE" leer 2>/dev/null | "$PY" -c 'import json,sys
f=[]
for l in sys.stdin:
    try: d=json.loads(l)
    except Exception: continue
    if "camino" in d: f.append(" ".join([d["camino"].replace(" ","_"), d["estado"], d["huella"], d.get("blob") or "-", d.get("tipo") or "-"]))
print("\n".join(sorted(f)))'; }
# Una ruta que un binario nativo entienda (en Windows, `/c/…` no lo es).
nativa() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else echo "$1"; fi; }
# Cada blob del manifiesto, leído del lago: su sha256 tiene que ser su nombre.
cotejar() {
  local bien=0 mal=0 b r
  for b in $(echo "$1" | awk '{print $4}' | sort -u); do
    r=$(printf '{"blob":"%s","archivo":"%s"}\n' "$b" "$(nativa "$TMP/blob.bin")" | "$STORE" blob-leer 2>&1)
    if echo "$r" | grep -q "\"sha256\":\"$b\""; then bien=$((bien + 1)); else mal=$((mal + 1)); echo "$b · $r" >> "$TMP/cotejo.err"; fi
  done
  echo "$bien $mal"
}
blobs_en_el_lago() {
  "$PY" -c 'import urllib.request,re,sys
x=urllib.request.urlopen(sys.argv[1]+"/copia?list-type=2&prefix=ore/v2/blobs/sha256/").read().decode()
print(len(re.findall("<Key>",x)))' "$ORE_R2_S3_ENDPOINT"; }

echo "── 2 · la transacción 1, con los bytes"
"$ORE" materialize . > "$TMP/m1.txt" 2>&1
afirma "materialize termina bien" "$([ $? = 0 ] && echo 1)" "$(tail -4 "$TMP/m1.txt")"
afirma "$PDF: 4 actuales, 2 bajados (tres claves son un contenido)" \
  "$([ "$(puntero "$PDF" items.actuales)" = 4 ] && [ "$(puntero "$PDF" blobs.bajados)" = 2 ] && [ "$(puntero "$PDF" blobs.distintos)" = 2 ] && [ "$(puntero "$PDF" virtual)" = false ] && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m1.txt" | tail -1)"
afirma "$JPG: 2 actuales, 1 bajado" \
  "$([ "$(puntero "$JPG" items.actuales)" = 2 ] && [ "$(puntero "$JPG" blobs.bajados)" = 1 ] && echo 1)" \
  "$(grep -A1 "$JPG" "$TMP/m1.txt" | tail -1)"
afirma "$CON: 4 actuales, 4 bajados" \
  "$([ "$(puntero "$CON" items.actuales)" = 4 ] && [ "$(puntero "$CON" blobs.bajados)" = 4 ] && echo 1)" \
  "$(grep -A1 "$CON" "$TMP/m1.txt" | tail -1)"
afirma "7 blobs en el lago" "$([ "$(blobs_en_el_lago)" = 7 ] && echo 1)" "$(blobs_en_el_lago)"
M1=$(manifiesto "$PDF"); echo "$M1" > "$TMP/manifiesto1.txt"
afirma "cada fila con su blob y su tipo" "$(echo "$M1" | awk '$4=="-" || $5!="application/pdf"' | grep -q . || echo 1)" "$M1"
read -r bien mal < <(cotejar "$M1")
afirma "cada blob del lago es el sha256 de sus bytes ($bien)" "$([ "$mal" = 0 ] && [ "$bien" = 2 ] && echo 1)" "$bien bien, $mal mal"

echo "── 3 · otra pasada, sin cambios"
"$ORE" materialize . > "$TMP/m2.txt" 2>&1
afirma "al día: nada que bajar" "$(grep -A1 "$PDF" "$TMP/m2.txt" | grep -q 'al día' && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m2.txt" | tail -1)"

echo "── 4 · la colección pide sólo a.pdf"
F="$(col "$PDF")"
"$PY" - "$F" <<'PYEOF'
import re,sys
p=sys.argv[1]; t=open(p,encoding="utf-8").read()
t=re.sub(r"from: \{ objectTable: ([^ }]+) \}", r'from: { objectTable: \1, match: "a.pdf" }', t)
open(p,"w",encoding="utf-8").write(t)
PYEOF
"$ORE" materialize . > "$TMP/m3.txt" 2>&1
M2=$(manifiesto "$PDF")
afirma "transacción 2: 3 se retiran, con su blob, y nada se pierde" \
  "$([ "$(puntero "$PDF" cambios.retiran)" = 3 ] && [ "$(echo "$M2" | awk '$2=="retirado" && $4!="-"' | wc -l | tr -d ' ')" = 3 ] && ! echo "$M2" | grep -q perdido && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m3.txt" | tail -1) · $M2"
read -r bien mal < <(cotejar "$M2")
afirma "lo retirado se sigue leyendo del lago" "$([ "$mal" = 0 ] && [ "$bien" -ge 1 ] && echo 1)" "$bien bien, $mal mal"

echo "── 5 · sin el match"
"$PY" - "$F" <<'PYEOF'
import sys
p=sys.argv[1]; t=open(p,encoding="utf-8").read().replace(', match: "a.pdf"', "")
open(p,"w",encoding="utf-8").write(t)
PYEOF
"$ORE" materialize . > "$TMP/m4.txt" 2>&1
afirma "transacción 3: vuelven los tres sin bajar un byte ni pedir una huella" \
  "$([ "$(puntero "$PDF" cambios.entran)" = 3 ] && [ "$(puntero "$PDF" blobs.bajados)" = 0 ] && grep -A1 "$PDF" "$TMP/m4.txt" | grep -q '0 huellas' && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m4.txt" | tail -1)"

echo "── 6 · un Job que subió y se cortó antes de sellar"
rm -f "$(find datasets -name "$CON.json" | head -1)"
"$ORE" materialize . > "$TMP/m5.txt" 2>&1
afirma "$CON: los 4 ya estaban en el lago por su huella; 0 bajados" \
  "$([ "$(puntero "$CON" items.actuales)" = 4 ] && [ "$(puntero "$CON" blobs.ya_en_el_lago)" = 4 ] && [ "$(puntero "$CON" blobs.bajados)" = 0 ] && echo 1)" \
  "$(grep -A1 "$CON" "$TMP/m5.txt" | tail -1)"
afirma "y el lago sigue con 7 blobs" "$([ "$(blobs_en_el_lago)" = 7 ] && echo 1)" "$(blobs_en_el_lago)"

echo "── 7 · el cotejo: lo que el manifiesto dice, en el lago"
QN=$("$ORE" collections . --json | "$PY" -c 'import json,sys
print([c["nombre"] for c in json.load(sys.stdin)["colecciones"] if c["nombre"].endswith(".'"$PDF"'")][0])')
"$ORE" collections . --cotejar "$QN" --muestra 9 > "$TMP/c1.txt" 2>&1
afirma "limpio: 2 blobs bien, los 2 vueltos a hashear" \
  "$([ $? = 0 ] && grep -q '2 blobs: 2 bien, 2 vueltos a hashear' "$TMP/c1.txt" && echo 1)" "$(cat "$TMP/c1.txt")"
ROTO=$(echo "$M1" | grep '^a.pdf' | awk '{print $4}')
"$PY" -c 'import urllib.request,sys
urllib.request.urlopen(urllib.request.Request(sys.argv[1]+"/copia/ore/v2/blobs/sha256/"+sys.argv[2], method="DELETE"))' "$ORE_R2_S3_ENDPOINT" "$ROTO"
"$ORE" collections . --cotejar "$QN" > "$TMP/c2.txt" 2>&1
codigo=$?
afirma "un blob que falta: sale con 1 y nombra sus tres ítems" \
  "$([ "$codigo" = 1 ] && grep -q "roto · $ROTO · no está en el lago" "$TMP/c2.txt" && grep "roto · $ROTO" "$TMP/c2.txt" | grep -q 'a.pdf' && grep "roto · $ROTO" "$TMP/c2.txt" | grep -q 'd.pdf' && echo 1)" \
  "$codigo · $(cat "$TMP/c2.txt")"

echo "── 8 · la retención y la recogida"
# La colección de los PDF empieza de nuevo (sin puntero: el blob que el paso
# 7 borró se vuelve a subir) con una retención de un segundo, y retira tres.
solo_a() {  # on | off
  "$PY" - "$F" "$1" <<'PYEOF'
import re,sys
p,modo=sys.argv[1],sys.argv[2]; t=open(p,encoding="utf-8").read().replace(', match: "a.pdf"', "")
if modo=="on": t=re.sub(r"from: \{ objectTable: ([^ }]+) \}", r'from: { objectTable: \1, match: "a.pdf" }', t)
if "retention:" not in t: t=t.replace("  formats:", "  retention: 1s\n  formats:", 1)
open(p,"w",encoding="utf-8").write(t)
PYEOF
}
rm -f "$(find datasets -name "$PDF.json" | head -1)"
solo_a on
"$ORE" validate . > "$TMP/val2.txt" 2>&1 || falla "validate con retention: $(tail -3 "$TMP/val2.txt")"
"$ORE" materialize . > "$TMP/m6.txt" 2>&1
solo_a off; "$ORE" materialize . >> "$TMP/m6.txt" 2>&1
solo_a on;  "$ORE" materialize . >> "$TMP/m6.txt" 2>&1
M3=$(manifiesto "$PDF")
afirma "tres retirados (la de a.pdf, resubida: 7 blobs otra vez)" \
  "$([ "$(echo "$M3" | awk '$2=="retirado"' | wc -l | tr -d ' ')" = 3 ] && [ "$(blobs_en_el_lago)" = 7 ] && echo 1)" \
  "$M3 · $(grep -A1 "$PDF" "$TMP/m6.txt" | grep -v "^--")"
sleep 2
"$ORE" collections . --recoger > "$TMP/r1.txt" 2>&1
codigo=$?
afirma "con la gracia de siempre (2 h): caducan 3 filas y el blob que sólo ellas nombraban espera" \
  "$([ "$codigo" = 0 ] && grep -q "$PDF · retención 1s · 4 filas, 3 caducadas" "$TMP/r1.txt" && grep -q '1 en su gracia, 0 recogidos' "$TMP/r1.txt" && echo 1)" "$(cat "$TMP/r1.txt")"
afirma "el manifiesto se queda con a.pdf, y el puntero se movió" \
  "$([ "$(manifiesto "$PDF" | wc -l | tr -d ' ')" = 1 ] && [ "$(puntero "$PDF" retencion.caducados)" = 3 ] && echo 1)" "$(manifiesto "$PDF")"
"$ORE" collections . --recoger --gracia 0 --seco > "$TMP/r2.txt" 2>&1
afirma "en seco, sin gracia: diría 1 recogido y no toca nada" \
  "$(grep -q 'en seco · blobs: 7 en el lago, 6 vivos, 0 en su gracia, 1 recogidos' "$TMP/r2.txt" && [ "$(blobs_en_el_lago)" = 7 ] && echo 1)" "$(cat "$TMP/r2.txt")"
"$ORE" collections . --recoger --gracia 0 > "$TMP/r3.txt" 2>&1
afirma "sin gracia: se va el de Invoice, y su huella; quedan 6" \
  "$(grep -q '1 recogidos' "$TMP/r3.txt" && grep -q '1 huellas del índice recogidas' "$TMP/r3.txt" && [ "$(blobs_en_el_lago)" = 6 ] && echo 1)" "$(cat "$TMP/r3.txt")"
read -r bien mal < <(cotejar "$(manifiesto "$CON")")
afirma "lo de las otras colecciones no se toca: contratos coteja limpio" "$([ "$mal" = 0 ] && [ "$bien" = 4 ] && echo 1)" "$bien bien, $mal mal"
solo_a off
"$ORE" materialize . > "$TMP/m7.txt" 2>&1
afirma "si Invoice vuelve, se baja otra vez: su huella ya no promete nada" \
  "$([ "$(puntero "$PDF" blobs.bajados)" = 1 ] && [ "$(puntero "$PDF" items.actuales)" = 4 ] && [ "$(blobs_en_el_lago)" = 7 ] && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m7.txt" | tail -1)"

echo "── 9 · servir: de la huella, una URL firmada a los bytes (0046 E9·2)"
QC=$("$ORE" collections . --json | "$PY" -c 'import json,sys
print([c["nombre"] for c in json.load(sys.stdin)["colecciones"] if c["nombre"].endswith(".'"$CON"'")][0])')
MC=$(manifiesto "$CON")
H1=$(echo "$MC" | sed -n 1p | awk '{print $3}'); H2=$(echo "$MC" | sed -n 2p | awk '{print $3}')
"$ORE" collections . --servir "$QC" --huella "$H1" --huella "$H2" --huella "crc64nvme:no-esta=" --json > "$TMP/s1.json" 2> "$TMP/s1.err"
afirma "dos huellas, dos URLs; la que no está, dicha" \
  "$([ $? = 0 ] && "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); assert len(d["items"])==2 and d["no_estan"]==["crc64nvme:no-esta="] and d["segundos"]==300' "$TMP/s1.json" && echo 1)" \
  "$(cat "$TMP/s1.json" "$TMP/s1.err")"
"$PY" - "$TMP/s1.json" > "$TMP/s2.txt" 2>&1 <<'PYEOF'
import hashlib, json, sys, urllib.request, urllib.error
d = json.load(open(sys.argv[1]))
def get(u, h=None):
    try:
        r = urllib.request.urlopen(urllib.request.Request(u, headers=h or {}))
        return r.status, r.read(), r.headers
    except urllib.error.HTTPError as e:
        return e.code, e.read(), e.headers
for i in d["items"]:
    st, b, h = get(i["url"])
    assert st == 200 and hashlib.sha256(b).hexdigest() == i["blob"], (st, i["camino"])
    assert h["content-type"] == i["tipo"] == "application/pdf", h["content-type"]
    assert h["content-disposition"].startswith('inline; filename="'), h["content-disposition"]
    st, b, h = get(i["url"], {"Range": "bytes=0-3"})
    assert st == 206 and len(b) == 4, st
    st, _, _ = get(i["url"].replace("inline", "attachment"))
    assert st == 403, ("manipulada", st)
    print("bien", i["camino"])
PYEOF
afirma "cada URL da los bytes de su blob, con su tipo, en línea y a rangos; manipulada, 403" \
  "$([ "$(grep -c '^bien' "$TMP/s2.txt")" = 2 ] && echo 1)" "$(cat "$TMP/s2.txt")"
"$ORE" collections . --servir "$QC" --huella "$H1" --ttl 99999 --json > "$TMP/s3.json" 2>&1
afirma "una URL vive como mucho una hora" \
  "$(grep -q '"segundos":3600' "$TMP/s3.json" && grep -q 'X-Amz-Expires=3600' "$TMP/s3.json" && echo 1)" "$(cat "$TMP/s3.json")"
"$ORE" collections . --servir "$QC" --json > "$TMP/s4.txt" 2>&1
afirma "sin huellas, 64" "$([ $? = 64 ] && echo 1)" "$(cat "$TMP/s4.txt")"

echo
if [ "$fallos" -eq 0 ]; then echo "s3-coleccion-mantenida · todo en verde"; exit 0; fi
echo "s3-coleccion-mantenida · $fallos aserciones en rojo"
exit 1
