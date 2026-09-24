#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# MEDIDA · los punteros (0038 P2) — antes de mover `datasets/<p>_<n>.json` a
# `datasets/<base>/<schema>/<n>.json`, lo que hoy hacen los punteros y el lago
# con un nombre, de punta a punta, sin red: el S3 de mentira y `ore-store-r2`.
#
#   M1  `ore materialize --recoger` (el Job de la copia, malla/48) sobre un
#       árbol con un dataset ESCRITO (`write()` → `--commit`) y ninguna vista
#       mantenida: ¿se queda el dataset en el bucket?
#   M2  el choque: `a_b.c` y `a.b_c` → ¿el mismo fichero de puntero?
#   M3  un nombre anidado (`datasets/ventas/default/anidada`): ¿dónde lo pone
#       el lago, y qué hace `recoger-huerfanas` cuando SÍ se reclama?
#   M4  el borrado de un paquete (`copia.rs`, prefijo `<paquete>_`): lo que se
#       llevaría con `ventas` cuando existe `ventas_eu`
#
# Mide y dice; no falla. Necesita `ore`, `ore-store-r2` y python3 con pyarrow.
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
STORE="$(buscar ore-store-r2)" || { echo "no hay binario de \`ore-store-r2\`"; exit 2; }
export PATH="$(dirname "$STORE"):$PATH"
mide() { printf '  \xc2\xb7 %s\n' "$1"; }

TMP="${TMPDIR:-/tmp}/ore-punteros-$$"
rm -rf "$TMP"; mkdir -p "$TMP"
S3_PID=""
trap 'kill "$S3_PID" 2>/dev/null; rm -rf "$TMP"' EXIT
export GIT_AUTHOR_NAME=m GIT_AUTHOR_EMAIL=m@x GIT_COMMITTER_NAME=m GIT_COMMITTER_EMAIL=m@x

"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"
objetos() { curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=$1" | grep -o '<Key>[^<]*</Key>' | wc -l | tr -d ' '; }

A="$TMP/a"; mkdir -p "$A"
( cd "$A" && git init -q -b main && "$ORE" init . --name m >/dev/null 2>&1 ) || { echo "ore init"; exit 2; }
for p in ventas ventas_eu a_b a; do
  mkdir -p "$A/packages/$p"
  printf 'apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: %s, version: 1.0.0, status: active, domain: sales }\nspec: { owner: team:data }\n' "$p" > "$A/packages/$p/package.yaml"
done

cat > "$TMP/ipc.py" <<'PY'
import sys, pyarrow as pa
t = pa.table({"id": pa.array(range(3), pa.int64())})
w = pa.ipc.new_stream(sys.stdout.buffer, t.schema); w.write_table(t); w.close()
PY
# escribe <dataset> <tabla> <operacion>: `write()` de un puesto → --commit
escribe() {
  { printf '{"dataset":"%s","modo":"sobrescribir","base":"","operacion":"%s"}\n' "$1" "$3"
    "$PY" "$TMP/ipc.py"; } | "$STORE" escribir > "$TMP/e.json" 2>"$TMP/e.err" || { cat "$TMP/e.err"; return 1; }
  "$ORE" datasets "$A" --commit --tabla "$2" --peticion "@$TMP/e.json" --json > "$TMP/c.json" 2>"$TMP/c.err" \
    || { cat "$TMP/c.err" "$TMP/c.json"; return 1; }
}

echo "M1 · el Job de la copia y un dataset escrito"
escribe datasets/ventas_escrita ventas.escrita op-1 || exit 1
ANTES=$(objetos ore/v2/datasets/ventas_escrita/)
mide "tras write(): $ANTES objetos en ore/v2/datasets/ventas_escrita/, puntero $(ls "$A/datasets")"
"$ORE" materialize "$A" --recoger --informe datasets --seco > "$TMP/m1s.txt" 2>&1
mide "materialize --recoger --seco dice: $(grep -i 'hu[eé]rfan\|retir' "$TMP/m1s.txt" | head -2 | tr '\n' ' ')"
"$ORE" materialize "$A" --recoger --informe datasets > "$TMP/m1.txt" 2>&1
DESPUES=$(objetos ore/v2/datasets/ventas_escrita/)
mide "materialize --recoger dice: $(grep -i 'hu[eé]rfan\|retir' "$TMP/m1.txt" | head -2 | tr '\n' ' ')"
mide "M1 → $ANTES objetos antes, $DESPUES después (el puntero sigue: $([ -f "$A/datasets/ventas_escrita.json" ] && echo sí || echo no))"

echo "M2 · el choque de nombres"
escribe datasets/a_b_c a_b.c op-2 >/dev/null || mide "a_b.c no se escribió"
ML_ABC=$("$PY" -c 'import json;print(json.load(open("'"$A"'/datasets/a_b_c.json"))["tabla"])' 2>/dev/null)
escribe datasets/a_b_c a.b_c op-3 >/dev/null 2>&1 || mide "a.b_c no se escribió: $(head -c 200 "$TMP/c.err")"
mide "M2 → ficheros en datasets/: $(ls "$A/datasets" | tr '\n' ' '); el puntero a_b_c.json dice tabla=$("$PY" -c 'import json;print(json.load(open("'"$A"'/datasets/a_b_c.json"))["tabla"])') (antes: $ML_ABC)"

echo "M3 · un nombre anidado en el lago"
{ printf '{"dataset":"datasets/ventas/default/anidada","modo":"sobrescribir","base":"","operacion":"op-4"}\n'; "$PY" "$TMP/ipc.py"; } \
  | "$STORE" escribir > "$TMP/e4.json" 2>"$TMP/e4.err" || mide "escribir anidado: $(head -c 300 "$TMP/e4.err")"
mide "M3 → objetos en ore/v2/datasets/ventas/default/anidada/: $(objetos ore/v2/datasets/ventas/default/anidada/)"
printf '{"datasets":["datasets/ventas/default/anidada","datasets/ventas_escrita","datasets/a_b_c"],"claves":[],"seco":"true"}\n' \
  | "$STORE" recoger-huerfanas > "$TMP/h.txt" 2>&1
mide "M3 → recoger-huerfanas en seco, con el anidado reclamado: $(tr '\n' ' ' < "$TMP/h.txt" | head -c 300)"

echo "M4 · el borrado de un paquete por prefijo"
escribe datasets/ventas_eu_x ventas_eu.x op-5 >/dev/null || true
mide "M4 → punteros: $(ls "$A/datasets" | tr '\n' ' ')"
mide "M4 → lo que empieza por \`ventas_\` (lo que borrar \`ventas\` se llevaría): $(cd "$A/datasets" && ls ventas_* | tr '\n' ' ')"
