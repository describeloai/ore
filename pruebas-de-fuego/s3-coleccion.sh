#!/usr/bin/env bash
# LA COLECCIÓN VIRTUAL, DE PUNTA A PUNTA (0046 E8·1): transacciones de una
# `MediaCollection` sobre el bucket de la medida F1 (con el experimento de E7
# dentro: `a.pdf` sobrescrito, `b.pdf` borrado, `c.jpg` → `c2.jpg`, `d.pdf`
# repetido). SOLO LECTURA sobre el bucket; el manifiesto va a un S3 de mentira.
#
#   1  catálogo + discover foreign: tres colecciones virtuales (los PDF y los
#      JPG de la raíz, los contratos), y el árbol compila
#   2  materialize: la transacción 1 de cada una, con su versión y su huella
#      por ítem; `a.pdf` es su versión nueva, y `Receipt…`, `a.pdf` y `d.pdf`
#      tienen la misma huella
#   3  otra pasada: al día, sin leer el manifiesto ni pedir una huella
#   4  la colección pide sólo `a.pdf` (`from.match`): la transacción 2 retira
#      los otros tres, y quedan `retirado` —su versión sigue en el origen—
#   5  sin el `match`: la transacción 3 los vuelve a meter, y no se pide
#      ninguna huella (ya se conocían)
#   6  servir (E9·3): de dos huellas, dos URLs prefirmadas de su versión en el
#      origen —los bytes, el tipo, a rangos; manipulada, 403; sin el secreto
#      dentro—, y sin la credencial, 69 con lo que necesita
#
# Uso:  ORE_S3_URL='s3://<bucket>/?region=…&access_key_id=…&secret_access_key=…' \
#         bash pruebas-de-fuego/s3-coleccion.sh
set -u

if [ -z "${ORE_S3_URL:-}" ]; then
  echo "se salta · sin ORE_S3_URL no hay bucket"
  exit 0
fi

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python)
TMP="${TMPDIR:-/tmp}/ore-s3-coleccion-$$"
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

echo "── 1 · catálogo y discover foreign"
"$ORE" source catalog s3_demo --out "$TMP/catalogo.json" > "$TMP/cat.txt" 2>&1 \
  || falla "catalog: $(tail -2 "$TMP/cat.txt")"
"$ORE" discover --source s3_demo --type foreign \
  --only default.raiz_pdf --only default.raiz_jpg --only nueva_carpeta.contratos \
  --no-model --owner team:datos --out packages/docs --name docs > "$TMP/disc.txt" 2>&1 \
  || falla "discover: $(tail -3 "$TMP/disc.txt")"
N=$(find packages/docs -path "*collections*" -name "*.yaml" | wc -l | tr -d ' ')
afirma "tres colecciones virtuales" "$([ "$N" = 3 ] && grep -rq "virtual: true" packages/docs && echo 1)" "$N"
if "$ORE" validate . > "$TMP/val.txt" 2>&1; then ok "el árbol compila"
else falla "validate: $(tail -3 "$TMP/val.txt")"; fi
col() { find packages/docs -path "*collections*" -name "$1.yaml" | head -1; }
PDF=$(basename "$(col 'raiz_pdf*')" .yaml)

puntero() { "$PY" -c 'import json,sys,glob
fs=[f for f in glob.glob("datasets/**/*.json",recursive=True) if f.replace("\\","/").endswith("/"+sys.argv[1]+".json")]
d=json.load(open(fs[0],encoding="utf-8")) if len(fs)==1 else {}
v=d
for k in sys.argv[2].split("."): v=v.get(k) if isinstance(v,dict) else None
print(json.dumps(v) if not isinstance(v,str) else v)' "$1" "$2"; }
manifiesto() {  # colección → filas "camino version estado huella", ordenadas
  printf '{"dataset":"%s","metadata_location":"%s"}\n' "$(puntero "$1" dataset)" "$(puntero "$1" metadata_location)" \
    | "$STORE" leer 2>/dev/null | "$PY" -c 'import json,sys
f=[]
for l in sys.stdin:
    try: d=json.loads(l)
    except Exception: continue
    if "camino" in d: f.append(" ".join([d["camino"], d["version"][:6], d["estado"], d["huella"]]))
print("\n".join(sorted(f)))'; }

echo "── 2 · la transacción 1"
"$ORE" materialize . > "$TMP/m1.txt" 2>&1
afirma "materialize termina bien" "$([ $? = 0 ] && echo 1)" "$(tail -4 "$TMP/m1.txt")"
afirma "$PDF: transacción 1, 4 actuales, 4 huellas" \
  "$([ "$(puntero "$PDF" transaccion)" = 1 ] && [ "$(puntero "$PDF" items.actuales)" = 4 ] && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m1.txt" | tail -1)"
M1=$(manifiesto "$PDF"); echo "$M1" > "$TMP/manifiesto1.txt"
afirma "a.pdf es su versión nueva (6GDBQX…)" "$(echo "$M1" | grep -q '^a.pdf 6GDBQX actual' && echo 1)" "$M1"
iguales=$(echo "$M1" | awk '{print $4}' | sort | uniq -c | awk '$1==3' | wc -l | tr -d ' ')
afirma "Receipt…, a.pdf y d.pdf: una misma huella" "$([ "$iguales" = 1 ] && echo 1)" "$M1"
for c in 'raiz_jpg*:2' 'contratos*:4'; do
  n=$(basename "$(col "${c%%:*}")" .yaml)
  afirma "$n: ${c#*:} actuales" "$([ "$(puntero "$n" items.actuales)" = "${c#*:}" ] && echo 1)" "$(puntero "$n" items)"
done

echo "── 3 · otra pasada, sin cambios"
"$ORE" materialize . > "$TMP/m2.txt" 2>&1
afirma "al día: el listado no cambió" "$(grep -A1 "$PDF" "$TMP/m2.txt" | grep -q 'al día' && echo 1)" \
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
afirma "transacción 2: 3 se retiran, 1 actual" \
  "$([ "$(puntero "$PDF" transaccion)" = 2 ] && [ "$(puntero "$PDF" cambios.retiran)" = 3 ] && [ "$(puntero "$PDF" items.actuales)" = 1 ] && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m3.txt" | tail -1)"
M2=$(manifiesto "$PDF")
afirma "los tres quedan \`retirado\`, no perdidos: su versión sigue en el origen" \
  "$([ "$(echo "$M2" | grep -c ' retirado ')" = 3 ] && ! echo "$M2" | grep -q perdido && echo 1)" "$M2"

echo "── 5 · sin el match"
"$PY" - "$F" <<'PYEOF'
import sys
p=sys.argv[1]; t=open(p,encoding="utf-8").read().replace(', match: "a.pdf"', "")
open(p,"w",encoding="utf-8").write(t)
PYEOF
"$ORE" materialize . > "$TMP/m4.txt" 2>&1
afirma "transacción 3: vuelven a entrar los tres, sin pedir una huella" \
  "$([ "$(puntero "$PDF" transaccion)" = 3 ] && [ "$(puntero "$PDF" cambios.entran)" = 3 ] && [ "$(puntero "$PDF" items.actuales)" = 4 ] && grep -A1 "$PDF" "$TMP/m4.txt" | grep -q '0 huellas' && echo 1)" \
  "$(grep -A1 "$PDF" "$TMP/m4.txt" | tail -1)"

echo "── 6 · servir del origen: la versión de cada ítem, con una URL prefirmada (0046 E9·3)"
QN=$("$ORE" collections . --json | "$PY" -c 'import json,sys
print([c["nombre"] for c in json.load(sys.stdin)["colecciones"] if c["nombre"].endswith(".'"$PDF"'")][0])')
M3=$(manifiesto "$PDF")
H1=$(echo "$M3" | awk '$3=="actual"{print $4}' | sort -u | sed -n 1p)
H2=$(echo "$M3" | awk '$3=="actual"{print $4}' | sort -u | sed -n 2p)
"$ORE" collections . --servir "$QN" --huella "$H1" --huella "$H2" --json > "$TMP/v1.json" 2> "$TMP/v1.err"
afirma "dos huellas, dos URLs del origen" \
  "$([ $? = 0 ] && "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["virtual"] is True and len(d["items"])==2 and not d["no_estan"]' "$TMP/v1.json" && echo 1)" \
  "$(cat "$TMP/v1.err")"
"$PY" - "$TMP/v1.json" > "$TMP/v2.txt" 2>&1 <<'PYEOF'
import json, os, sys, urllib.parse, urllib.request, urllib.error
d = json.load(open(sys.argv[1]))
secreto = urllib.parse.parse_qs(urllib.parse.urlparse(os.environ["ORE_S3_URL"]).query).get("secret_access_key", ["?"])[0]
def get(u, h=None):
    try:
        r = urllib.request.urlopen(urllib.request.Request(u, headers=h or {}), timeout=30)
        return r.status, r.read(), r.headers
    except urllib.error.HTTPError as e:
        return e.code, e.read(), e.headers
for i in d["items"]:
    u = i["url"]
    assert secreto not in u and urllib.parse.quote(secreto, safe="") not in u, "el secreto viaja en la URL"
    st, b, h = get(u)
    assert st == 200 and len(b) == i["tamano"], (st, len(b), i["tamano"])
    assert h["x-amz-version-id"] == i["version"], "otra versión"
    assert h["content-type"] == "application/pdf" and h["content-disposition"].startswith('inline; filename="'), (h["content-type"], h["content-disposition"])
    st, b, h = get(u, {"Range": "bytes=0-3"})
    assert st == 206 and b == b"%PDF"[:len(b)], st
    st, _, _ = get(u.replace("inline", "attachment"))
    assert st == 403, ("manipulada", st)
    print("bien", i["camino"])
PYEOF
afirma "del origen, su versión: los bytes, el tipo, en línea y a rangos; manipulada, 403; sin el secreto en la URL" \
  "$([ "$(grep -c '^bien' "$TMP/v2.txt")" = 2 ] && echo 1)" "$(cat "$TMP/v2.txt")"
( unset ORE_S3_URL; "$ORE" collections . --servir "$QN" --huella "$H1" --json > "$TMP/v3.txt" 2>&1; echo "codigo $?" >> "$TMP/v3.txt" )
afirma "sin la credencial: 69, y dice cuál necesita (lo que ore-serve trae del cofre)" \
  "$(grep -q 'codigo 69' "$TMP/v3.txt" && grep -q '{"necesita":{"env":"ORE_S3_URL","fuente":"s3_demo"}}' "$TMP/v3.txt" && echo 1)" "$(cat "$TMP/v3.txt")"

echo
if [ "$fallos" -eq 0 ]; then echo "s3-coleccion · todo en verde"; exit 0; fi
echo "s3-coleccion · $fallos aserciones en rojo"
exit 1
