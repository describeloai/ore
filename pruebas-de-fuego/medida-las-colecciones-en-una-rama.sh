#!/usr/bin/env bash
# MEDIDA · LAS COLECCIONES EN UNA RAMA (0044 C D7c). Antes de encolarlas (D7d),
# qué hace hoy cada pieza con una colección que nace en una rama. Una forja de
# mentira (un repositorio desnudo) y un S3 de mentira que es origen y lago.
#
#   M1  la pasada del Job en la rama (la de `malla/48`, con `--vista <la
#       colección>`): ¿corre su transacción y deja su puntero en la rama?
#   M2  la superposición de D3: una colección que sólo tiene `main`, ¿se lee de
#       `main` en la rama y no se ve en `git status`?
#   M3  `ore datasets --recoger` en `main` (que no la tiene): ¿se lleva el
#       manifiesto de la colección de la rama? Con y sin `--reclaman`.
#   M4  `ore collections --recoger` en `main`: ¿sus blobs? (lo que D7a cerró)
#
# Mide y dice; no falla. Uso:  bash pruebas-de-fuego/medida-las-colecciones-en-una-rama.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python)
TMP="${TMPDIR:-/tmp}/ore-medida-colecciones-rama-$$"
rm -rf "$TMP"; mkdir -p "$TMP"
S3_PID=""
limpiar() { [ -n "$S3_PID" ] && kill "$S3_PID" 2>/dev/null; [ -n "${ORE_GUARDAR:-}" ] || rm -rf "$TMP"; }
trap limpiar EXIT
export PYTHONUTF8=1

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
mide() { printf '  %s · %s\n' "$1" "$2"; }

"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3="http://127.0.0.1:$(awk '{print $2}' "$TMP/s3.log")"
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="$S3" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"
export ORE_S3_URL="s3://copia/?endpoint=$S3&region=auto&access_key_id=de&secret_access_key=mentira"
"$PY" - "$S3" <<'PYEOF'
import sys, urllib.request
for k, b in (("a.pdf", b"%PDF-1.4 a"), ("b.pdf", b"%PDF-1.4 b, otra cosa"), ("x/c.pdf", b"%PDF-1.4 c")):
    urllib.request.urlopen(urllib.request.Request(sys.argv[1] + "/copia/" + k, data=b, method="PUT",
                                                  headers={"content-type": "application/pdf"}))
PYEOF
lago() {  # prefijo → cuántos objetos
  "$PY" -c 'import urllib.request,re,sys
x=urllib.request.urlopen(sys.argv[1]+"/copia?list-type=2&prefix="+sys.argv[2]).read().decode()
print(len(re.findall("<Key>",x)))' "$S3" "$1"; }

# ── la forja: `main` con la conexión; la rama `r` da de alta la base ──────────
FORJA="$TMP/forja.git"; git init -q --bare -b main "$FORJA"
A="$TMP/main"; git clone -q -c core.autocrlf=false "$FORJA" "$A" 2>/dev/null; cd "$A"
git config core.autocrlf false; git config user.name ana; git config user.email ana@invalido
"$ORE" init . --name ficheros >/dev/null 2>&1
printf 'datasources:\n  - { name: s3_demo, type: s3, connectionEnv: ORE_S3_URL }\n' >> ontology.config.yaml
printf 'apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: default }\nspec:\n  owner: team:datos\n  conduits:\n    materialization.payload: { oos.maturity: DRAFT }\n' > conduits.yaml
"$ORE" source catalog s3_demo --out "$TMP/catalogo.json" > /dev/null 2>&1
# `main` tiene su propia colección (la de `x/`), con su transacción.
"$ORE" discover --source s3_demo --type standard --only x.x --no-model --owner team:datos \
  --out packages/demain --name demain > "$TMP/d0.txt" 2>&1 || echo "discover main: $(tail -2 "$TMP/d0.txt")"
git add -A; git commit -qm "main"; git push -q origin main
# (Las dos altas antes de materializar nada: el origen y el lago son el mismo
# S3, y el lago escrito entraría en lo que el discover de la rama perfila.)
git checkout -qb r
"$ORE" discover --source s3_demo --type standard --only default.raiz --no-model --owner team:datos \
  --out packages/docs --name docs > "$TMP/d1.txt" 2>&1 || echo "discover rama: $(tail -2 "$TMP/d1.txt")"
git add -A; git commit -qm "alta de una base (en la rama)"; git push -q origin r
git checkout -q main
"$ORE" materialize . > "$TMP/m0.txt" 2>&1 || echo "materialize main: $(tail -3 "$TMP/m0.txt")"
git add -A; git commit -qm "copia en main"; git push -q origin main
git checkout -q r
COL=$(basename "$(find packages/docs -path '*collections*' -name '*.yaml' | head -1)" .yaml)
echo "colección de la rama: docs.$COL · de main: demain.$(basename "$(find packages/demain -path '*collections*' -name '*.yaml' | head -1)" .yaml)"

echo "── M1 · la pasada del Job en la rama"
J="$TMP/job"; git clone -q -c core.autocrlf=false -b r "$FORJA" "$J" 2>/dev/null; cd "$J"; git config core.autocrlf false
"$ORE" overlay . --main origin/main > "$TMP/o1.txt" 2>&1; mide "overlay" "$(tail -1 "$TMP/o1.txt")"
"$ORE" materialize . --vista "docs.$COL" --informe datasets > "$TMP/j1.txt" 2>&1
mide "materialize --vista docs.$COL (salida $?)" "$(tr '\n' ' ' < "$TMP/j1.txt" | cut -c1-300)"
"$ORE" overlay . --undo > /dev/null 2>&1
git add -A datasets
mide "lo que la pasada deja en la rama" "$(git diff --cached --name-only | tr '\n' ' ')"
P=$(git diff --cached --name-only | grep "/$COL.json" | head -1)
[ -n "$P" ] && mide "su puntero" "$("$PY" -c 'import json,sys;d=json.load(open(sys.argv[1]));print(d.get("kind"),"·",d.get("dataset"),"· escrito_por:",d.get("escrito_por"))' "$P")"
mide "blobs en el lago" "$(lago ore/v2/blobs/sha256/)"
git -c user.name=copiador -c user.email=c@invalido commit -qm "copia en r" 2>/dev/null; git push -q origin HEAD:r 2>/dev/null

echo "── M2 · la superposición: la de main, leída en la rama"
cd "$J"; git fetch -q origin
"$ORE" overlay . --main origin/main > "$TMP/o2.txt" 2>&1
PM=$(cd "$A" && git checkout -q main && find datasets -name '*.json' -path '*demain*' | head -1)
mide "el puntero de la colección de main, en la rama" "$([ -f "$J/$PM" ] && echo "está ($PM)" || echo "NO está ($PM)")"
mide "git status tras superponer" "$(git status --porcelain | tr '\n' ' ')"
"$ORE" overlay . --undo > /dev/null 2>&1

echo "── M3 · ore datasets --recoger en main, que no tiene la de la rama"
cd "$A"; git checkout -q main
mkdir -p "$TMP/ramas/r"; (cd "$J" && B=$(git merge-base origin/main origin/r) && git diff --name-only --diff-filter=AM "$B" origin/r -- datasets | while read -r F; do mkdir -p "$TMP/ramas/r/$(dirname "$F")"; git show "origin/r:$F" > "$TMP/ramas/r/$F"; done)
mide "lo que reclama la rama" "$(find "$TMP/ramas" -name '*.json' | sed "s#$TMP/ramas/##" | tr '\n' ' ')"
DS=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1])).get("dataset",""))' "$J/$P" 2>/dev/null)
mide "objetos del manifiesto de la rama ($DS)" "$(lago "ore/v2/$DS/")"
"$ORE" datasets . --recoger --edad 0 --seco > "$TMP/r1.txt" 2>&1
mide "sin --reclaman, en seco" "$(grep -iE "huérfan|$COL|docs" "$TMP/r1.txt" | tr '\n' ' ' | cut -c1-300)"
"$ORE" datasets . --recoger --edad 0 --seco --reclaman "$TMP/ramas" > "$TMP/r2.txt" 2>&1
mide "con --reclaman, en seco" "$(grep -iE "huérfan|$COL|docs" "$TMP/r2.txt" | tr '\n' ' ' | cut -c1-300)"

echo "── M4 · ore collections --recoger en main"
"$ORE" collections . --recoger --gracia 0 --seco > "$TMP/c1.txt" 2>&1
mide "sin --reclaman, en seco" "$(tail -1 "$TMP/c1.txt")"
"$ORE" collections . --recoger --gracia 0 --seco --reclaman "$TMP/ramas" > "$TMP/c2.txt" 2>&1
mide "con --reclaman, en seco" "$(tail -2 "$TMP/c2.txt" | tr '\n' ' ')"
