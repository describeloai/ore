#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# MEDIDA · LOS DATOS EN UNA RAMA (0044, apéndice D · D0)
#
# Antes de decidir cómo tiene datos una rama, lo que pasa HOY cuando los tiene.
# Una rama ya puede escribir bytes (un puesto o `/v1` con `x-ore-rama`: el
# puntero va a la rama, 0031 §11 ⑦), pero el Job de la copia, el mantenimiento y
# la recogida sólo conocen `main`. Se monta el mismo mundo dos veces:
#
#   main:  `ventas.base` (PyIceberg, 10 filas) y su copia mantenida
#          `ventas.copiaBase` (`from: ventas.base`, `ore materialize`)
#   ── la rama `bea/datos` sale de ahí ──
#   main:  +10 en `base` (20), `ventas.soloMain` nace (4), y la copia se rehace (20)
#   rama:  +5 en `base` (15), `ventas.nueva` nace (3) — por `/v1` con `x-ore-rama`
#
# y se pregunta:
#   M1  `main` recoge (el mantenimiento `ore datasets --recoger --edad 7d` y el Job
#       de la copia `ore materialize --recoger`): ¿sigue la rama leyendo lo suyo?
#   M2  la rama copia (lo que haría el Job de la copia sobre la rama:
#       `ore materialize --recoger` en un clon de la rama): ¿dónde escribe, y
#       sigue `main` leyendo lo suyo?
#   M3  fusionar la rama en `main` con los punteros movidos en los dos lados
#   M4  qué lee la rama de lo que `main` ganó después (el fallback, al día o no)
#   M5  `POST /datasets/{ns}/{n}/confirmar` con `x-ore-rama`: ¿a qué rama va?
#
# No afirma: MIDE. Cada línea `·` es una observación; al final, la tabla.
# Necesita `ore`, `ore-serve`, `ore-store-r2`, git y python3 con pyarrow y
# pyiceberg.
# ══════════════════════════════════════════════════════════════════════════════
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python) || { echo "hace falta python"; exit 2; }
buscar() {
  local n
  for n in "$RAIZ/target/${PERFIL:-debug}/$1" "$RAIZ/target/${PERFIL:-debug}/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  command -v "$1"
}
ORE="$(buscar ore)" || { echo "no hay \`ore\`"; exit 2; }
SERVE="$(buscar ore-serve)" || { echo "no hay \`ore-serve\`"; exit 2; }
STORE="$(buscar ore-store-r2)" || { echo "no hay \`ore-store-r2\`"; exit 2; }
export PATH="$(dirname "$STORE"):$PATH"
"$PY" -c 'import pyarrow, pyiceberg' 2>/dev/null || { echo "hace falta pyarrow y pyiceberg"; exit 2; }
export GIT_AUTHOR_NAME=semilla GIT_AUTHOR_EMAIL=semilla@x
export GIT_COMMITTER_NAME=semilla GIT_COMMITTER_EMAIL=semilla@x

RAMA="bea/datos"
export ORE_RETENCION=7d
TODO="${TMPDIR:-/tmp}/ore-datos-rama-$$"
rm -rf "$TODO"; mkdir -p "$TODO"
PIDS=""
trap 'kill $PIDS 2>/dev/null; [ -n "${GUARDA:-}" ] || rm -rf "$TODO"' EXIT
obs() { printf '  \xc2\xb7 %s\n' "$1"; echo "$1" >> "$TODO/tabla.txt"; }
paso() { printf '\n\xe2\x94\x80\xe2\x94\x80 %s\n' "$1"; }

# El cliente: PyIceberg contra `/v1`, en `main` (`-`) o en una rama.
cat > "$TODO/cli.py" <<'PY'
import sys, json
import pyarrow as pa
from pyiceberg.catalog import load_catalog
base, rama, op, tabla = sys.argv[1:5]
n = int(sys.argv[5]) if len(sys.argv) > 5 else 0
h = {"type": "rest", "uri": base, "header.x-ore-sujeto": "persona:ana"}
if rama != "-":
    h["header.x-ore-rama"] = rama
cat = load_catalog("ore", **h)
def datos(desde, n):
    return pa.table({"id": pa.array(range(desde, desde + n), pa.int64()), "pais": pa.array(["ES"] * n)})
try:
    if op == "crear":
        t = cat.create_table(("ventas", tabla), schema=datos(0, 1).schema)
        t.append(datos(0, n))
        print("ok")
    elif op == "anexar":
        t = cat.load_table(("ventas", tabla))
        t.append(datos(1000 + n, n))
        print("ok")
    elif op == "contar":
        print(cat.load_table(("ventas", tabla)).scan().to_arrow().num_rows)
except Exception as e:
    print("ROTO " + type(e).__name__ + ": " + str(e).replace("\n", " ")[:160])
PY

# ── el mundo: S3 de mentira, forja pelada, ore-serve, y la divergencia ────────
montar() { # $1 = nombre del escenario
  local T="$TODO/$1"; mkdir -p "$T"
  "$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$T/s3.log" 2>&1 & PIDS="$PIDS $!"
  for _ in $(seq 1 50); do grep -q listo "$T/s3.log" 2>/dev/null && break; sleep 0.2; done
  local p; p=$(awk '{print $2}' "$T/s3.log")
  export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$p" ORE_R2_BUCKET=copia \
         ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"
  FORJA="$T/arbol.git"
  git init -q --bare -b main "$FORJA"
  git clone -q "$FORJA" "$T/semilla" 2>/dev/null
  git -C "$T/semilla" config core.autocrlf false
  local A="$T/semilla"
  ( cd "$A" && "$ORE" init . --name lago >/dev/null 2>&1 )
  mkdir -p "$A/packages/ventas/datasets"
  cat > "$A/packages/ventas/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 1.0.0, status: active, domain: sales }
spec: { owner: team:data }
Y
  cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: lago }
spec:
  owner: team:data
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y
  ( cd "$A" && git add -A && git commit -qm semilla && git push -q origin HEAD:main )
  local puerto; puerto=$("$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
  BASE="http://127.0.0.1:$puerto"
  FORJA_TOKEN=no-hace-falta "$SERVE" --forja "file://$FORJA" --ore "$ORE" --bind "127.0.0.1:$puerto" \
    --identidad cabecera --no-es-produccion --organizacion lago >"$T/serve.log" 2>&1 & PIDS="$PIDS $!"
  for _ in $(seq 1 60); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done

  # main: base (10) y su copia mantenida
  [ "$(cli - crear base 10)" = ok ] || { echo "no nació base en main"; tail -5 "$T/serve.log"; exit 1; }
  git clone -q "$FORJA" "$T/m"
  cat > "$T/m/packages/ventas/datasets/copiaBase.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: copiaBase, namespace: ventas }
spec:
  owner: team:data
  from: { dataset: ventas.base }
  fields: { id: id, pais: pais }
Y
  ( cd "$T/m" && "$ORE" materialize . --vista ventas.copiaBase --informe datasets ) > "$T/mat0.txt" 2>&1 || { cat "$T/mat0.txt"; exit 1; }
  ( cd "$T/m" && git add -A && git commit -qm "la copia" && git push -q origin HEAD:main ) || exit 1
  # ── la rama sale de aquí ──
  git --git-dir="$FORJA" branch "$RAMA" main
  # main avanza: +10 en base, soloMain nace, la copia se rehace
  cli - anexar base 10 >/dev/null
  cli - crear soloMain 4 >/dev/null
  ( cd "$T/m" && git pull -q origin main )
  ( cd "$T/m" && "$ORE" materialize . --vista ventas.copiaBase --informe datasets ) > "$T/mat1.txt" 2>&1 || { cat "$T/mat1.txt"; exit 1; }
  ( cd "$T/m" && git add -A && git commit -qm "la copia, otra vez" -q; git push -q origin HEAD:main )
  # la rama escribe: +5 en base, nueva nace
  R_ANEXA=$(cli "$RAMA" anexar base 5)
  R_CREA=$(cli "$RAMA" crear nueva 3)
  T_ACTUAL="$T"
}
cli() { "$PY" "$TODO/cli.py" "$BASE" "$@" 2>>"$TODO/cli.err"; }
puntero() { # rama tabla → metadata_location (o «—»)
  git --git-dir="$FORJA" show "$1:datasets/ventas/default/$2.json" 2>/dev/null \
    | "$PY" -c 'import json,sys;print(json.load(sys.stdin).get("metadata_location","—"))' 2>/dev/null || echo "—"
}
objetos() { curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=ore/v2/" | grep -o '<Key>[^<]*</Key>' | wc -l | tr -d ' '; }
estado() { # etiqueta → las lecturas de los dos lados
  obs "$1 · main: base=$(cli - contar base) soloMain=$(cli - contar soloMain) copiaBase=$(cli - contar copiaBase) · rama: base=$(cli "$RAMA" contar base) nueva=$(cli "$RAMA" contar nueva) copiaBase=$(cli "$RAMA" contar copiaBase) soloMain=$(cli "$RAMA" contar soloMain) · objetos=$(objetos)"
}

# ══ escenario 1 · main recoge ════════════════════════════════════════════════
paso "escenario 1 · la divergencia, y main recoge"
montar e1
T="$T_ACTUAL"
obs "la rama escribe por /v1 con x-ore-rama: anexar=$R_ANEXA crear=$R_CREA"
BM=$(puntero main base); BR=$(puntero "$RAMA" base)
obs "puntero de base · main=${BM##*/ore/v2/} · rama=${BR##*/ore/v2/}"
obs "tabla de nueva (rama) · $(puntero "$RAMA" nueva | sed 's#.*/ore/v2/##')"
estado "antes de recoger"
paso "M1 · el mantenimiento de main (ore datasets --recoger --edad 7d)"
git clone -q "$FORJA" "$T/mant"
"$ORE" datasets "$T/mant" --recoger --edad 7d > "$T/mant.txt" 2>&1
obs "M1a · el mantenimiento dice: $(tr '\n' ' ' < "$T/mant.txt" | cut -c1-300)"
estado "M1a · tras el mantenimiento de main"
paso "M1 · el Job de la copia en main (ore materialize --recoger)"
( cd "$T/mant" && "$ORE" materialize . --recoger --informe datasets ) > "$T/matr.txt" 2>&1
obs "M1b · la copia dice: $(grep -i 'recog\|huérf' "$T/matr.txt" | tr '\n' ' ' | cut -c1-300)"
estado "M1b · tras la copia de main"

# ══ escenario 2 · la rama copia ══════════════════════════════════════════════
paso "escenario 2 · la misma divergencia, y la rama copia"
montar e2
T="$T_ACTUAL"
estado "antes de copiar en la rama"
git clone -q -b "$RAMA" "$FORJA" "$T/r"
CM_ANTES=$(puntero main copiaBase)
( cd "$T/r" && "$ORE" materialize . --recoger --informe datasets ) > "$T/matrama.txt" 2>&1
obs "M2 · la copia en la rama dice: $(grep -i 'copiad\|recog\|huérf\|al d' "$T/matrama.txt" | tr '\n' ' ' | cut -c1-400)"
( cd "$T/r" && git add -A datasets && git commit -qm "Copia en la rama" && git push -q origin HEAD:"$RAMA" )
CR=$(puntero "$RAMA" copiaBase)
obs "M2 · copiaBase · main=${CM_ANTES##*/ore/v2/} · rama=${CR##*/ore/v2/} (misma tabla: $( [ "${CM_ANTES%/metadata/*}" = "${CR%/metadata/*}" ] && echo sí || echo no))"
estado "M2 · tras la copia en la rama"

paso "M4 · qué ve la rama de main"
code=$(curl -s -o "$T/ficha.json" -w '%{http_code}' -H 'x-ore-sujeto: persona:ana' -H "x-ore-rama: $RAMA" "$BASE/datasets/ventas/soloMain")
obs "M4 · GET /datasets/ventas/soloMain en la rama: $code $(head -c 160 "$T/ficha.json")"

paso "M3 · fusionar la rama en main"
git clone -q "$FORJA" "$T/f"
( cd "$T/f" && git fetch -q origin "$RAMA" && git merge --no-edit "origin/$RAMA" > "$T/merge.txt" 2>&1 ); rc=$?
obs "M3 · git merge rc=$rc · $(grep -i 'conflict' "$T/merge.txt" | tr '\n' ' ' | cut -c1-400)"
code=$(curl -s -o "$T/c.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' \
  "$BASE/propuestas" -d "{\"desde\":\"$RAMA\",\"titulo\":\"datos\",\"seco\":true}")
obs "M3 · POST /propuestas seco: $code $(head -c 200 "$T/c.json")"

paso "M5 · confirmar con x-ore-rama"
ML=$(puntero "$RAMA" nueva)
code=$(curl -s -o "$T/conf.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' -H "x-ore-rama: $RAMA" \
  -H 'content-type: application/json' "$BASE/datasets/ventas/nuevaConf/confirmar" -d "{\"metadata_location\":\"$ML\",\"filas\":3,\"columnas\":{\"id\":\"Integer\",\"pais\":\"String\"}}")
en_main=$(git --git-dir="$FORJA" show main:datasets/ventas/default/nuevaConf.json >/dev/null 2>&1 && echo sí || echo no)
en_rama=$(git --git-dir="$FORJA" show "$RAMA":datasets/ventas/default/nuevaConf.json >/dev/null 2>&1 && echo sí || echo no)
obs "M5 · confirmar con x-ore-rama: $code · puntero en main=$en_main · en la rama=$en_rama"

printf '\n══ la tabla ══\n'; cat "$TODO/tabla.txt"
[ -s "$TODO/cli.err" ] && { printf '\n(stderr del cliente, lo último)\n'; tail -3 "$TODO/cli.err"; }
exit 0
