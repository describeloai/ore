#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# LOS DATOS EN UNA RAMA — 0044 C, de punta a punta y sin red de verdad: la forja
# pelada, el S3 de mentira, `ore-serve` como catálogo y PyIceberg por `/v1` con
# `x-ore-rama` (el camino de un puesto). Es la medida D0
# (`medida-los-datos-en-una-rama.sh`) vuelta prueba: lo que allí se rompía, aquí
# se afirma que no.
#
# El mundo, dos veces igual:
#   main:  `ventas.base` (10) y su copia mantenida `ventas.copiaBase`
#   ── sale la rama `bea/datos` ──
#   main:  +10 en `base` (20), `ventas.soloMain` nace (4), la copia se rehace (20)
#   rama:  +5 en `base` (15), `ventas.nueva` nace (3)
#
# Lo que afirma (D1 · la recogida cuenta con todas las ramas):
#   1  `reclaman`: lo que el Job deja para `--reclaman` —de cada otra rama, sus
#      punteros propios (los que difieren de su punto de salida), y de `main`
#      todos— es base y nueva de la rama, y no lo heredado
#   2  sin `--reclaman`, un directorio que no está: la pasada se niega (66) y no
#      toca el bucket
#   3  `main` recoge (el mantenimiento y el Job de la copia) con `--reclaman`: la
#      rama sigue leyendo base 15 y nueva 3, y `main` lo suyo
#   4  la rama copia y recoge con `--reclaman`: copiaBase 15 en la rama, y `main`
#      sigue leyendo base 20, soloMain 4 y copiaBase 20
#   5  la rama se borra: la pasada siguiente de `main` se lleva lo suyo (nueva
#      entera, y los ficheros de base que sólo la rama nombraba) y `main` sigue
#      entero
#
# Necesita `ore`, `ore-serve`, `ore-store-r2` (en `$ORE_TARGET` o
# `target/debug`), git y python3 con pyarrow y pyiceberg.
# ══════════════════════════════════════════════════════════════════════════════
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python) || { echo "hace falta python"; exit 2; }
buscar() {
  local n
  for n in "${ORE_TARGET:-$RAIZ/target}/debug/$1" "${ORE_TARGET:-$RAIZ/target}/debug/$1.exe"; do
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
fallos=0
ok()    { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '\xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }

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
  git clone -q "$FORJA" "$T/m"; git -C "$T/m" config core.autocrlf false
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
objetos() { curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=ore/v2/${1:-}" | grep -o '<Key>[^<]*</Key>' | wc -l | tr -d ' '; }
lee() { # rama tabla → filas, o ROTO…
  cli "$1" contar "$2"
}

# ── lo que el Job deja para `--reclaman` (0044 C.2 ②) ─────────────────────────
# ⭐ El mismo guion que `malla/48` y `53`: en un clon con todas las ramas, de
#   cada rama que no es la que recoge, sus punteros PROPIOS —los que difieren del
#   punto del que salió—; de `main`, todos. Lo heredado no alarga la vida a los
#   bytes viejos de `main`.
reclaman() { # $1 = la rama que recoge · $2 = el directorio · en el clon (cwd)
  rm -rf "$2"; mkdir -p "$2"
  local r base f
  for r in $(git for-each-ref refs/remotes/origin --format='%(refname)' | sed 's#^refs/remotes/origin/##'); do
    [ "$r" = HEAD ] || [ "$r" = "$1" ] && continue
    if [ "$r" = main ]; then
      git ls-tree -r --name-only "origin/$r" -- datasets copias resultados
    else
      base=$(git merge-base origin/main "origin/$r") || continue
      git diff --name-only --diff-filter=AM "$base" "origin/$r" -- datasets copias resultados
    fi | while read -r f; do
      case "$f" in *.json) mkdir -p "$2/$r/$(dirname "$f")"; git show "origin/$r:$f" > "$2/$r/$f" ;; esac
    done
  done
}

# ══ 1–3 · main recoge ════════════════════════════════════════════════════════
montar e1
T="$T_ACTUAL"
[ "$R_ANEXA" = ok ] && [ "$R_CREA" = ok ] || falla "0 · la rama no escribió por /v1: $R_ANEXA · $R_CREA"
[ "$(lee - base)/$(lee - soloMain)/$(lee - copiaBase)/$(lee "$RAMA" base)/$(lee "$RAMA" nueva)" = "20/4/20/15/3" ] \
  || falla "0 · el mundo no es el de D0: main $(lee - base)/$(lee - soloMain)/$(lee - copiaBase), rama $(lee "$RAMA" base)/$(lee "$RAMA" nueva)"

git clone -q "$FORJA" "$T/mant"
( cd "$T/mant" && reclaman main "$T/ramas" )
got=$(cd "$T/ramas" && find . -name '*.json' | sort | tr '\n' ' ')
[ "$got" = "./bea/datos/datasets/ventas/default/base.json ./bea/datos/datasets/ventas/default/nueva.json " ] \
  || falla "1 · reclaman tenía que dejar base y nueva de la rama, y nada heredado: $got"
ok "1 · reclaman: de la rama, sólo lo suyo (base, nueva); copiaBase, heredada, no"

A0=$(objetos)
"$ORE" datasets "$T/mant" --recoger --edad 7d --reclaman "$T/no-esta" > "$T/sin.txt" 2>&1; rc=$?
[ "$rc" = 66 ] && [ "$(objetos)" = "$A0" ] || falla "2 · un --reclaman que no está: rc=$rc (y no 66), objetos $A0 → $(objetos) · $(cat "$T/sin.txt")"
( cd "$T/mant" && "$ORE" materialize . --recoger --informe datasets --reclaman "$T/no-esta" ) > "$T/sin2.txt" 2>&1; rc=$?
[ "$rc" = 66 ] && [ "$(objetos)" = "$A0" ] || falla "2 · materialize con un --reclaman que no está: rc=$rc, objetos $A0 → $(objetos)"
ok "2 · un --reclaman que no es un directorio: 66 y el bucket intacto ($A0 objetos), en las dos pasadas"

"$ORE" datasets "$T/mant" --recoger --edad 7d --reclaman "$T/ramas" > "$T/mant.txt" 2>&1 || falla "3 · el mantenimiento: $(cat "$T/mant.txt")"
( cd "$T/mant" && "$ORE" materialize . --recoger --informe datasets --reclaman "$T/ramas" ) > "$T/matr.txt" 2>&1 || falla "3 · la copia: $(cat "$T/matr.txt")"
v="$(lee - base)/$(lee - soloMain)/$(lee - copiaBase) · $(lee "$RAMA" base)/$(lee "$RAMA" nueva)/$(lee "$RAMA" copiaBase)"
[ "$v" = "20/4/20 · 15/3/10" ] || falla "3 · tras recoger main: $v · $(tr '\n' ' ' < "$T/mant.txt")"
ok "3 · main recoge con --reclaman (el mantenimiento y la copia): main 20/4/20 y la rama 15/3/10 siguen leyéndose ($A0 → $(objetos) objetos)"

# ── 5 · la rama se borra: lo suyo se va en la pasada siguiente ────────────────
ANTES=$(objetos); NUEVA=$(objetos catalogo/ventas/default/nueva/)
git --git-dir="$FORJA" branch -D "$RAMA" -q
rm -rf "$T/mant2"; git clone -q "$FORJA" "$T/mant2"
( cd "$T/mant2" && reclaman main "$T/ramas2" )
"$ORE" datasets "$T/mant2" --recoger --edad 7d --reclaman "$T/ramas2" > "$T/mant2.txt" 2>&1 || falla "5 · el mantenimiento: $(cat "$T/mant2.txt")"
DESPUES=$(objetos)
[ "$(objetos catalogo/ventas/default/nueva/)" = 0 ] && [ "$DESPUES" -lt "$((ANTES - NUEVA))" ] \
  || falla "5 · borrada la rama, lo suyo tenía que irse: $ANTES → $DESPUES objetos, nueva $NUEVA → $(objetos catalogo/ventas/default/nueva/)"
[ "$(lee - base)/$(lee - soloMain)/$(lee - copiaBase)" = "20/4/20" ] || falla "5 · main tenía que seguir entero: $(lee - base)/$(lee - soloMain)/$(lee - copiaBase)"
ok "5 · borrada la rama, la pasada siguiente se lleva lo suyo ($ANTES → $DESPUES objetos: nueva entera y lo que sólo ella nombraba en base) y main sigue 20/4/20"

# ══ 4 · la rama copia ════════════════════════════════════════════════════════
montar e2
T="$T_ACTUAL"
git clone -q -b "$RAMA" "$FORJA" "$T/r"
( cd "$T/r" && reclaman "$RAMA" "$T/ramas" )
[ -f "$T/ramas/main/datasets/ventas/default/soloMain.json" ] || falla "4 · desde la rama, reclaman tenía que traer todo main: $(cd "$T/ramas" && find . -name '*.json' | tr '\n' ' ')"
( cd "$T/r" && "$ORE" materialize . --recoger --informe datasets --reclaman "$T/ramas" ) > "$T/matrama.txt" 2>&1 || falla "4 · la copia en la rama: $(cat "$T/matrama.txt")"
( cd "$T/r" && git add -A datasets && git commit -qm "Copia en la rama" && git push -q origin HEAD:"$RAMA" ) || falla "4 · no se pudo empujar a la rama"
v="$(lee - base)/$(lee - soloMain)/$(lee - copiaBase) · $(lee "$RAMA" base)/$(lee "$RAMA" nueva)/$(lee "$RAMA" copiaBase)"
[ "$v" = "20/4/20 · 15/3/15" ] || falla "4 · tras copiar en la rama: $v · $(tr '\n' ' ' < "$T/matrama.txt" | cut -c1-300)"
ok "4 · la rama copia y recoge con --reclaman: copiaBase 15 en la rama, y main sigue 20/4/20"

if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 los datos en una rama: 1\xe2\x80\x935\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
