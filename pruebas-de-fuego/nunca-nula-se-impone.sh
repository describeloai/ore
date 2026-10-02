#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# NUNCA NULA SE IMPONE — ORE 0051 P6, de punta a punta y sin red de verdad: un
# origen de ficheros (`ore-read-jsonl`), el S3 de mentira y `ore materialize`
# con el almacén `ore-store-r2`, como el Job de la copia.
#
# Una `Table` (v1alpha22) cuyo origen garantiza `order_id` (`required: true`),
# y dos copias: `pedidos` (lleva `id ← order_id`) y `paises` (sólo `pais`, que
# nada garantiza). Lo que afirma:
#
#   1  APAGADO (sin `.arbol/nulos.yaml`): la copia es la de antes de P6: en el
#      `metadata.json` de Iceberg todas las columnas son opcionales, y la
#      cabecera no lleva `obligatorias`
#   2  ENCENDIDO (`imponer: true`): `pedidos` se rehace UNA vez —su cabecera
#      gana `obligatorias: [id]`— y `id` queda `required` en Iceberg con el
#      MISMO id de columna; `paises` no tiene nada garantizado y dice «ya está»
#      (su cabecera, byte a byte la de antes); y PyIceberg —si está— lee la
#      tabla y ve `id` como `required`. La pasada siguiente: «ya está»
#   3  UN NULO: el origen trae una fila sin `order_id` —el árbol sigue diciendo
#      que nunca es nula—: la copia FALLA con la columna, la fila y qué hacer,
#      el puntero dice `error` con ese motivo y sigue nombrando la copia de
#      antes, y en el bucket no hay ningún `metadata.json` nuevo (ni el esquema
#      se confirmó)
#   4  APAGAR ES AFLOJAR: sin el fichero, la misma copia se rehace con la fila
#      del nulo dentro, e `id` vuelve a opcional con el mismo id de columna
#
# Necesita `ore`, `ore-store-r2` y `ore-read-jsonl` (en `ORE_BIN`, en
# target/{release,debug} o en el PATH) y python3; pyiceberg, opcional.
#
# Uso:  bash pruebas-de-fuego/nunca-nula-se-impone.sh
# ══════════════════════════════════════════════════════════════════════════════
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python) || { echo "hace falta python"; exit 2; }

buscar() {
  local n
  for n in ${ORE_BIN:+"$ORE_BIN/$1" "$ORE_BIN/$1.exe"} \
           "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  command -v "$1"
}
ORE="$(buscar ore)" || { echo "no hay binario de \`ore\`"; exit 2; }
for b in ore-store-r2 ore-read-jsonl; do
  B="$(buscar $b)" || { echo "no hay binario de \`$b\` — cargo build -p ore-store -p ore-read-jsonl"; exit 2; }
  export PATH="$(dirname "$B"):$PATH"
done

fallos=0
ok()   { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '\xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }
dice() { printf '  \xc2\xb7 %s\n' "$1"; }

TMP="${TMPDIR:-/tmp}/ore-nunca-nula-$$"
rm -rf "$TMP"; mkdir -p "$TMP/datos" "$TMP/arbol"
S3_PID=""
limpiar() { kill "$S3_PID" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT

# ── el S3 de mentira ─────────────────────────────────────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for i in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; cat "$TMP/s3.log"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira

# ── el árbol: una tabla con una garantía, y dos copias ───────────────────────
A="$TMP/arbol"
mkdir -p "$A/packages/ventas/tables" "$A/packages/ventas/datasets"
cat > "$TMP/datos/pedidos.jsonl" <<'J'
{"order_id":"p1","pais":"ES"}
{"order_id":"p2"}
{"order_id":"p3","pais":"PT"}
J
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: nulos, version: 0.1.0 }
datasources:
  - { name: erp, type: jsonl, connectionEnv: FICHEROS_DIR }
Y
cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: nulos }
spec:
  owner: team:data
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y
cat > "$A/packages/ventas/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$A/packages/ventas/tables/pedidos_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: pedidos_t, namespace: ventas }
spec:
  datasource: erp
  object: "pedidos.jsonl"
  columns:
    order_id: { type: String, required: true }
    pais: { type: String }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
cat > "$A/packages/ventas/datasets/pedidos.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: pedidos, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { id: order_id, pais: pais }
Y
cat > "$A/packages/ventas/datasets/paises.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: paises, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { pais: pais }
Y
export FICHEROS_DIR="$TMP/datos"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || { "$ORE" validate "$A"; falla "el árbol no compila"; exit 1; }

P="$A/datasets/ventas/default"
de() { "$PY" -c 'import json,sys
d=json.load(open(sys.argv[1],encoding="utf-8"))
for k in sys.argv[2].split("."): d=d.get(k) if isinstance(d,dict) else None
print("" if d is None else d)' "$P/$1.json" "$2"; }
# Lo que Iceberg dice de una columna: «required id», del metadata.json que el
# puntero nombra, leído del S3 de mentira tal cual (sin pasar por `ore`).
iceberg() { "$PY" - "$S3_PUERTO" "$1" "$2" <<'EOF'
import json, sys, urllib.request
puerto, ml, col = sys.argv[1:]
clave = ml.split("s3://copia/", 1)[1]
m = json.load(urllib.request.urlopen(f"http://127.0.0.1:{puerto}/copia/{clave}"))
s = next(s for s in m["schemas"] if s["schema-id"] == m["current-schema-id"])
f = next(f for f in s["fields"] if f["name"] == col)
print("required" if f["required"] else "optional", f["id"])
EOF
}
metadatas() { "$PY" - "$S3_PUERTO" <<'EOF'
import sys, urllib.request, re
x = urllib.request.urlopen(f"http://127.0.0.1:{sys.argv[1]}/copia?list-type=2").read().decode()
print(len([k for k in re.findall(r"<Key>([^<]+)</Key>", x) if k.endswith(".metadata.json")]))
EOF
}
cabecera_snapshot() { printf '{"metadata_location":"%s","dataset":"x"}\n' "$1" | ore-store-r2 leer | head -1; }

# ── 1 · apagado: la copia de antes ───────────────────────────────────────────
"$ORE" materialize "$A" --informe "$A/datasets" > "$TMP/m1.txt" 2>&1 || { cat "$TMP/m1.txt"; falla "1 · materialize"; exit 1; }
ML1=$(de pedidos metadata_location); H1=$(de pedidos cabecera); HP1=$(de paises cabecera)
[ "$(de pedidos estado)" = "copiada" ] || falla "1 · pedidos no se copió: $(cat "$TMP/m1.txt")"
read -r R1 ID1 <<< "$(iceberg "$ML1" id)"
[ "$R1" = "optional" ] || falla "1 · apagado, \`id\` tenía que ser opcional y es $R1"
cabecera_snapshot "$ML1" | grep -q obligatorias && falla "1 · apagado, la cabecera lleva \`obligatorias\`"
grep -q "impone nunca nula" "$TMP/m1.txt" && falla "1 · apagado, dice que impone: $(cat "$TMP/m1.txt")"
ok "1 · apagado: la copia de antes de P6 (\`id\` opcional en Iceberg, id de columna $ID1; la cabecera sin \`obligatorias\`)"

# ── 2 · encendido ────────────────────────────────────────────────────────────
mkdir -p "$A/.arbol"; printf 'imponer: true\n' > "$A/.arbol/nulos.yaml"
"$ORE" materialize "$A" --informe "$A/datasets" > "$TMP/m2.txt" 2>&1 || { cat "$TMP/m2.txt"; falla "2 · materialize"; exit 1; }
ML2=$(de pedidos metadata_location)
grep -q "impone nunca nula · id" "$TMP/m2.txt" || falla "2 · no dice que impone \`id\`: $(cat "$TMP/m2.txt")"
[ "$(de pedidos estado)" = "copiada" ] && [ "$(de pedidos cabecera)" != "$H1" ] || falla "2 · pedidos tenía que rehacerse con otra cabecera: $(cat "$TMP/m2.txt")"
read -r R2 ID2 <<< "$(iceberg "$ML2" id)"
[ "$R2" = "required" ] && [ "$ID2" = "$ID1" ] || falla "2 · \`id\` tenía que ser required con el id $ID1, y es $R2 $ID2"
read -r RP _ <<< "$(iceberg "$ML2" pais)"
[ "$RP" = "optional" ] || falla "2 · \`pais\` no tiene garantía y salió $RP"
cabecera_snapshot "$ML2" | grep -q '"obligatorias":\["id"\]' || falla "2 · la cabecera del snapshot no lleva \`obligatorias\`: $(cabecera_snapshot "$ML2")"
[ "$(de paises estado)" = "al-dia" ] && [ "$(de paises cabecera)" = "$HP1" ] || falla "2 · paises no tiene nada garantizado y tenía que decir «ya está» con la misma cabecera: $(cat "$TMP/m2.txt")"
if "$PY" -c 'import pyiceberg' 2>/dev/null; then
  "$PY" - "$S3_PUERTO" "$ML2" <<'EOF' && ok "2 · PyIceberg lee la copia: \`id\` required, 3 filas" || falla "2 · PyIceberg no ve \`id\` required"
import sys
from pyiceberg.table import StaticTable
puerto, ml = sys.argv[1:]
t = StaticTable.from_metadata(ml, properties={
    "s3.endpoint": f"http://127.0.0.1:{puerto}", "s3.access-key-id": "de",
    "s3.secret-access-key": "mentira", "s3.region": "us-east-1"})
f = t.schema().find_field("id")
assert f.required, f
a = t.scan().to_arrow()
assert a.num_rows == 3 and a.column("id").null_count == 0, a
EOF
else
  dice "2 · sin pyiceberg: la marca se comprueba en el metadata.json, que es lo que lee cualquier motor"
fi
"$ORE" materialize "$A" --informe "$A/datasets" > "$TMP/m2b.txt" 2>&1 || falla "2 · la segunda pasada"
[ "$(de pedidos estado)" = "al-dia" ] || falla "2 · la segunda pasada tenía que decir «ya está»: $(cat "$TMP/m2b.txt")"
ok "2 · encendido: pedidos se rehace una vez con \`id\` required (el mismo id de columna) y \`obligatorias\` en la cabecera; paises, sin nada garantizado, «ya está»; la pasada siguiente, «ya está»"

# ── 3 · un nulo en lo que nunca es nulo ──────────────────────────────────────
echo '{"pais":"FR"}' >> "$TMP/datos/pedidos.jsonl"
N3=$(metadatas)
"$ORE" materialize "$A" --informe "$A/datasets" --vista ventas.pedidos > "$TMP/m3.txt" 2>&1 && falla "3 · con un nulo en \`id\`, materialize tenía que fallar: $(cat "$TMP/m3.txt")"
grep -q "la columna \`id\` de \`" "$TMP/m3.txt" && grep -q "nunca es nula" "$TMP/m3.txt" && grep -q "vuelve a" "$TMP/m3.txt" || falla "3 · el error no dice la columna y qué hacer: $(cat "$TMP/m3.txt")"
[ "$(de pedidos estado)" = "error" ] || falla "3 · el puntero tenía que decir error: $(cat "$P/pedidos.json")"
de pedidos motivo | grep -q "nunca es nula" || falla "3 · el motivo del puntero: $(de pedidos motivo)"
[ "$(de pedidos metadata_location)" = "$ML2" ] || falla "3 · el puntero tenía que seguir en la copia de antes"
[ "$(metadatas)" = "$N3" ] || falla "3 · quedó un metadata.json nuevo en el bucket ($N3 → $(metadatas)): algo se confirmó"
read -r R3 _ <<< "$(iceberg "$ML2" id)"
[ "$R3" = "required" ] || falla "3 · la copia de antes cambió"
ok "3 · un nulo en \`id\`: falla con la columna, la fila y qué hacer ($(grep -o 'la fila [0-9]*' "$TMP/m3.txt" | head -1)); el puntero dice error y sigue en la copia de antes; nada nuevo en el bucket"

# ── 4 · apagar es aflojar ────────────────────────────────────────────────────
rm -f "$A/.arbol/nulos.yaml"
"$ORE" materialize "$A" --informe "$A/datasets" > "$TMP/m4.txt" 2>&1 || { cat "$TMP/m4.txt"; falla "4 · materialize apagado"; }
ML4=$(de pedidos metadata_location)
[ "$(de pedidos estado)" = "copiada" ] && [ "$(de pedidos filas)" = "4" ] || falla "4 · pedidos tenía que copiarse con las 4 filas: $(cat "$TMP/m4.txt")"
read -r R4 ID4 <<< "$(iceberg "$ML4" id)"
[ "$R4" = "optional" ] && [ "$ID4" = "$ID1" ] || falla "4 · \`id\` tenía que volver a opcional con el id $ID1, y es $R4 $ID4"
[ "$(de pedidos cabecera)" = "$H1" ] || dice "4 · la cabecera no es la del paso 1 (el testigo cambió con la fila nueva): es lo esperado"
ok "4 · apagar es aflojar: la copia se rehace con el nulo dentro, \`id\` opcional y la misma columna ($ID4)"

echo
if [ "$fallos" -eq 0 ]; then echo "✓ nunca nula se impone: 4 de 4"; else echo "✗ $fallos fallos"; exit 1; fi
