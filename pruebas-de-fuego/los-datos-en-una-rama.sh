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
#   rama:  +5 en `base` —sobre lo que ve, el de `main` de hoy (D3): 25—, y
#          `ventas.nueva` nace (3); copiaBase, sin tocar, es la de `main` (20)
#
# Lo que afirma (D1 · la recogida cuenta con todas las ramas):
#   1  `reclaman`: lo que el Job deja para `--reclaman` —de cada otra rama, sus
#      punteros propios (los que difieren de su punto de salida), y de `main`
#      todos— es base y nueva de la rama, y no lo heredado
#   2  sin `--reclaman`, un directorio que no está: la pasada se niega (66) y no
#      toca el bucket
#   3  `main` recoge (el mantenimiento y el Job de la copia) con `--reclaman`: la
#      rama sigue leyendo base 25 y nueva 3, y `main` lo suyo
#   4  la rama copia y recoge con `--reclaman`: copiaBase 25 en la rama, y `main`
#      sigue leyendo base 20, soloMain 4 y copiaBase 20
#   5  la rama se borra: la pasada siguiente de `main` se lleva lo suyo (nueva
#      entera, y los ficheros de base que sólo la rama nombraba) y `main` sigue
#      entero
#
# Y D2 · `confirmar` en la rama:
#   6  `POST /datasets/{ns}/{n}/confirmar` con `x-ore-rama` mueve el puntero en
#      la rama y no en `main` (D0 M5); sin la cabecera, en `main`, como siempre
#
# Y D3 · lo que la rama no tocó se lee de `main` al día:
#   7  desde la rama, soloMain (nació en `main` después) se lee (4, y no 404) y
#      copiaBase es la de `main` de hoy (20, y no la 10 congelada); base y nueva
#      son suyas (25, 3); `GET /datasets` y la ficha dicen `de`
#   8  escribir en la rama sobre lo heredado (anexar 2 a soloMain por `/v1`)
#      funciona —sin el 409 de cargar el de `main` y confirmar contra el
#      congelado—, la rama lee 6 y `main` sigue 4; sólo soloMain pasa a ser de la
#      rama: copiaBase sigue igual que en el punto de salida
#   9  lo que `main` hace después se ve al momento: `main` anexa 7 a soloMain
#      (11) y la rama sigue en su 6 (ya es suyo); `main` crea otraMain (2) y la
#      rama la lee
#
# Y D4 · construir en la rama (con una cola, la plantilla de `malla/48`):
#  10  rehacer la copia en la rama se encola CON la rama (`48-la-copia-rehacer-*`
#      con `RAMA: bea/datos`), y en `main` sin ella (`RAMA: ""`); dar de alta
#      una fuente en la rama sigue siendo 409 (es de la celda)
#  11  la pasada del Job en la rama —los pasos de `malla/48`: `ore overlay
#      --main`, `materialize` de sus vistas con `--recoger --reclaman` (todo
#      `main` y lo propio de las demás), `ore overlay --undo`, empujar a la
#      rama— construye copiaBase EN la rama (25, de su base) y `main` sigue en
#      20; sólo copiaBase pasa a ser de la rama (soloMain, heredado, no); y la
#      rama sigue leyendo soloMain de `main`
#
# Y D5 · fusionar punteros (con la forja de mentira: propuestas de activos). La
# rama escribió `base` (25, sin receta) y creó `nueva`; construye copiaBase (con
# receta); `main` escribió `base` (20) y rehízo copiaBase después de separarse:
#  12  `GET /ramas/{r}/cambios`: los tres Datasets cambian con `datos`; base y
#      copiaBase, en conflicto con `main` (`enBase: datos`)
#  13  la propuesta en seco dice, por puntero: nueva → promoción (rama), base →
#      conflicto (sin elegir, y qué se pierde), copiaBase → reconstruir (main)
#  14  fusionar sin elegir: 409 con la lista; nada cambia en `main`
#  15  fusionar eligiendo `rama` para base: `main` lee base 25 y nueva 3
#      (promovidas, sin mover un byte), copiaBase sigue en 20 (la suya) y la
#      reconstrucción se encola en `main`; la rama se pone al día sin conflicto
#      y lo fusionado deja de ser suyo
#
# Y D7 · las colecciones en una rama (el S3 de mentira es también el origen):
#  16  dar de alta dos bases EN la rama por la API —`docs` estándar y `docsv`
#      foránea— encola, con la rama, sus colecciones (la mantenida y la
#      virtual); antes encolaba la copia de `main`
#  17  la pasada del Job en la rama las construye en ella y no en `main`, y se
#      sirven desde la rama (`/colecciones/.../items` con `x-ore-rama`)
#  18  el mantenimiento de `main` con `--reclaman` no se lleva sus blobs ni su
#      manifiesto
#  19  `/ramas/{r}/cambios` da la colección con `datos`; se propone como
#      promoción y, fusionada, `main` tiene su puntero tal cual y la sirve
#
# Necesita `ore`, `ore-serve`, `ore-store-r2`, `ore-read-s3` (en `$ORE_TARGET` o
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
  # Con `CON_API`, la forja de mentira (propuestas): el repositorio en `<org>/<repo>.git`.
  [ -n "${CON_API:-}" ] && { FORJA="$T/forja/t-demo/ontologia.git"; mkdir -p "$(dirname "$FORJA")"; }
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
  local api=""
  if [ -n "${CON_API:-}" ]; then
    local pf; pf=$("$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
    "$PY" "$RAIZ/pruebas-de-fuego/forja-de-mentira.py" "$FORJA" "$pf" >"$T/forja.txt" 2>&1 & PIDS="$PIDS $!"
    for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$pf/api/v1/version" && break; sleep 0.25; done
    api="127.0.0.1:$pf"
  fi
  FORJA_TOKEN=no-hace-falta "$SERVE" --forja "file://$FORJA" ${api:+--forja-api "$api"} --ore "$ORE" --bind "127.0.0.1:$puerto" \
    ${COLA_URL:+--cola "$COLA_URL"} \
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
[ "$(lee - base)/$(lee - soloMain)/$(lee - copiaBase)/$(lee "$RAMA" base)/$(lee "$RAMA" nueva)" = "20/4/20/25/3" ] \
  || falla "0 · el mundo no es el esperado: main $(lee - base)/$(lee - soloMain)/$(lee - copiaBase), rama $(lee "$RAMA" base)/$(lee "$RAMA" nueva)"

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
[ "$v" = "20/4/20 · 25/3/20" ] || falla "3 · tras recoger main: $v · $(tr '\n' ' ' < "$T/mant.txt")"
ok "3 · main recoge con --reclaman (el mantenimiento y la copia): main 20/4/20 y la rama 25/3/20 siguen leyéndose ($A0 → $(objetos) objetos)"

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
[ "$v" = "20/4/20 · 25/3/25" ] || falla "4 · tras copiar en la rama: $v · $(tr '\n' ' ' < "$T/matrama.txt" | cut -c1-300)"
ok "4 · la rama copia y recoge con --reclaman: copiaBase 25 en la rama, y main sigue 20/4/20"

# ══ 6 · confirmar en la rama (D2) ════════════════════════════════════════════
esta() { git --git-dir="$FORJA" show "$1:datasets/ventas/default/$2.json" >/dev/null 2>&1 && echo sí || echo no; }
confirma() { # rama|- nombre → código
  local h=(); [ "$1" != - ] && h=(-H "x-ore-rama: $1")
  curl -s -o "$T/conf.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' "${h[@]}" \
    -H 'content-type: application/json' "$BASE/datasets/ventas/$2/confirmar" \
    -d "{\"metadata_location\":\"$(puntero "$RAMA" nueva)\",\"filas\":3,\"columnas\":{\"id\":\"Integer\",\"pais\":\"String\"}}"
}
c=$(confirma "$RAMA" enRama)
[ "$c" = 201 ] && [ "$(esta "$RAMA" enRama)/$(esta main enRama)" = "sí/no" ] \
  || falla "6 · confirmar con x-ore-rama: $c, en la rama $(esta "$RAMA" enRama), en main $(esta main enRama) · $(cat "$T/conf.json")"
c=$(confirma - enMain)
[ "$c" = 201 ] && [ "$(esta main enMain)" = sí ] || falla "6 · confirmar sin cabecera tenía que ir a main: $c · $(cat "$T/conf.json")"
ok "6 · confirmar: con x-ore-rama el puntero va a la rama (y no a main); sin ella, a main"

# ══ 7–9 · lo no tocado, de main al día (D3) ══════════════════════════════════
montar e3
T="$T_ACTUAL"
v="$(lee "$RAMA" soloMain)/$(lee "$RAMA" copiaBase)/$(lee "$RAMA" base)/$(lee "$RAMA" nueva)"
[ "$v" = "4/20/25/3" ] || falla "7 · desde la rama (soloMain/copiaBase/base/nueva): $v, y no 4/20/25/3"
c=$(curl -s -o "$T/lista.json" -w '%{http_code}' -H 'x-ore-sujeto: persona:ana' -H "x-ore-rama: $RAMA" "$BASE/datasets")
de=$("$PY" -c 'import json,sys
d={x["nombre"]:x.get("de","?") for x in json.load(open(sys.argv[1]))["datasets"]}
print(" ".join(k+"="+d.get(k,"-") for k in ["ventas.base","ventas.nueva","ventas.copiaBase","ventas.soloMain"]))' "$T/lista.json" 2>&1)
[ "$c/$de" = "200/ventas.base=rama ventas.nueva=rama ventas.copiaBase=main ventas.soloMain=main" ] \
  || falla "7 · GET /datasets en la rama: $c · $de"
c=$(curl -s -o "$T/ficha.json" -w '%{http_code}' -H 'x-ore-sujeto: persona:ana' -H "x-ore-rama: $RAMA" "$BASE/datasets/ventas/soloMain")
[ "$c" = 200 ] && grep -q '"de":"main"' "$T/ficha.json" || falla "7 · la ficha de soloMain en la rama: $c $(head -c 200 "$T/ficha.json")"
c=$(curl -s -o "$T/assets.json" -w '%{http_code}' -H 'x-ore-sujeto: persona:ana' -H "x-ore-rama: $RAMA" "$BASE/assets")
as=$("$PY" -c 'import json,sys
it=json.load(open(sys.argv[1]))["items"].values()
d={x["ref"].split(":")[-1]:(x.get("puntero") or {}) for x in it if x.get("kind")=="Dataset"}
print(" ".join(k+"="+str(d.get(k,{}).get("de","-"))+"/"+str(d.get(k,{}).get("filas","-")) for k in ["ventas.base","ventas.copiaBase"]))' "$T/assets.json" 2>&1)
[ "$c/$as" = "200/ventas.base=rama/25 ventas.copiaBase=main/20" ] || falla "7 · /assets en la rama: $c · $as"
ok "7 · desde la rama: soloMain 4 (nació en main después) y copiaBase 20 (la de main de hoy); base 25 y nueva 3, suyas; la lista, la ficha y /assets dicen de: main | rama"

r=$(cli "$RAMA" anexar soloMain 2)
[ "$r" = ok ] || falla "8 · anexar en la rama a lo heredado: $r"
v="$(lee "$RAMA" soloMain)/$(lee - soloMain)"
[ "$v" = "6/4" ] || falla "8 · tras anexar en la rama: rama/main soloMain = $v, y no 6/4"
B=$(git --git-dir="$FORJA" merge-base main "$RAMA")
propios=$(git --git-dir="$FORJA" diff --name-only "$B" "$RAMA" -- datasets | tr '\n' ' ')
[ "$propios" = "datasets/ventas/default/base.json datasets/ventas/default/nueva.json datasets/ventas/default/soloMain.json " ] \
  || falla "8 · lo propio de la rama tras escribir: $propios (y no base, nueva y soloMain)"
ok "8 · escribir en la rama sobre lo heredado: anexar a soloMain funciona (rama 6, main 4), y sólo lo escrito pasa a ser suyo ($propios)"

cli - anexar soloMain 7 >/dev/null; cli - crear otraMain 2 >/dev/null
v="$(lee - soloMain)/$(lee "$RAMA" soloMain)/$(lee "$RAMA" otraMain)"
[ "$v" = "11/6/2" ] || falla "9 · main avanza después: main soloMain / rama soloMain / rama otraMain = $v, y no 11/6/2"
ok "9 · lo que main hace después se ve al momento: otraMain (2) se lee desde la rama; soloMain, ya suyo, sigue en 6 (main 11)"

# ══ 10–11 · construir en la rama (D4) ═════════════════════════════════════════
COLA="$TODO/cola.git"
git init -q --bare -b main "$COLA"
mkdir -p "$TODO/cola-semilla" && ( cd "$TODO/cola-semilla" && git init -q -b main && git config core.autocrlf false )
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TODO/rendido" >/dev/null 2>&1
cp "$TODO/rendido/plantilla-copia.txt" "$TODO/cola-semilla/"
( cd "$TODO/cola-semilla" && git add -A && git -c user.name=banco -c user.email=banco@invalido commit -q -m "la plantilla" \
  && git remote add origin "$COLA" && git push -q origin HEAD:main ) || falla "10 · no se pudo sembrar la cola"
COLA_URL="file://$COLA"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) COLA_URL="file:///$(cd "$COLA" && pwd -W)";; esac
montar e4
T="$T_ACTUAL"
rehace() { # rama|- → código; la respuesta en $T/re.json
  local h=(); [ "$1" != - ] && h=(-H "x-ore-rama: $1")
  curl -s -o "$T/re.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' "${h[@]}" "$BASE/paquetes/ventas/copia/rehacer"
}
fichero_de() { "$PY" -c 'import json,sys;print(json.load(open(sys.argv[1])).get("fichero",""))' "$T/re.json"; }
c=$(rehace "$RAMA"); F=$(fichero_de)
[ "$c" = 202 ] && git --git-dir="$COLA" show "main:$F" 2>/dev/null | grep -q "name: RAMA, value: \"$RAMA\"" \
  || falla "10 · rehacer en la rama: $c $(head -c 200 "$T/re.json")"
c=$(rehace -); F=$(fichero_de)
[ "$c" = 202 ] && git --git-dir="$COLA" show "main:$F" 2>/dev/null | grep -q 'name: RAMA, value: ""' \
  || falla "10 · rehacer en main: $c $(head -c 200 "$T/re.json")"
c=$(curl -s -o "$T/alta.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' -H "x-ore-rama: $RAMA" -H 'content-type: application/json' "$BASE/fuentes" -d '{"nombre":"x","tipo":"postgres"}')
[ "$c" = 409 ] && grep -q "de la celda" "$T/alta.json" || falla "10 · alta de una fuente en la rama: $c $(head -c 200 "$T/alta.json")"
ok "10 · rehacer en la rama se encola con RAMA=$RAMA (y en main con RAMA vacío); dar de alta una fuente en la rama sigue siendo 409: es de la celda"

# La pasada del Job en la rama: los pasos de malla/48, tal cual.
git clone -q -b "$RAMA" "$FORJA" "$T/job"; git -C "$T/job" config core.autocrlf false
( cd "$T/job" && "$ORE" overlay . --main origin/main ) > "$T/job.txt" 2>&1 || falla "11 · ore overlay --main: $(cat "$T/job.txt")"
( cd "$T/job" && reclaman "$RAMA" "$T/ramas-job" )
[ -f "$T/ramas-job/main/datasets/ventas/default/soloMain.json" ] || falla "11 · desde la rama, reclaman tenía que traer todo main"
( cd "$T/job" && "$ORE" materialize . --vista ventas.copiaBase --recoger --reclaman "$T/ramas-job" --informe datasets ) >> "$T/job.txt" 2>&1 \
  || falla "11 · materialize en la rama: $(tail -5 "$T/job.txt")"
( cd "$T/job" && "$ORE" overlay . --undo ) >> "$T/job.txt" 2>&1 || falla "11 · ore overlay --undo: $(tail -3 "$T/job.txt")"
( cd "$T/job" && git add -A datasets && git -c user.name=copiador -c user.email=copiador@invalido commit -qm "Copia: ventas.copiaBase en $RAMA" && git push -q origin HEAD:"$RAMA" ) \
  || falla "11 · no se pudo empujar la copia a la rama: $(cd "$T/job" && git status --short | head -5)"
v="$(lee "$RAMA" copiaBase)/$(lee - copiaBase)/$(lee "$RAMA" soloMain)"
[ "$v" = "25/20/4" ] || falla "11 · tras construir en la rama: rama copiaBase / main copiaBase / rama soloMain = $v, y no 25/20/4 · $(tail -4 "$T/job.txt")"
B=$(git --git-dir="$FORJA" merge-base main "$RAMA")
propios=$(git --git-dir="$FORJA" diff --name-only "$B" "$RAMA" -- datasets | tr '\n' ' ')
[ "$propios" = "datasets/ventas/default/base.json datasets/ventas/default/copiaBase.json datasets/ventas/default/nueva.json " ] \
  || falla "11 · lo propio de la rama tras construir: $propios (y no base, copiaBase y nueva)"
[ -e "$T/job/.ore-al-dia.json" ] && falla "11 · quedó .ore-al-dia.json en el clon"
git --git-dir="$FORJA" show "$RAMA:.ore-al-dia.json" >/dev/null 2>&1 && falla "11 · .ore-al-dia.json llegó a la rama"
ok "11 · la pasada del Job en la rama construye copiaBase en ella (25, de su base) y main sigue en 20; sólo lo construido es suyo ($propios) y soloMain se sigue leyendo de main"

# ══ 12–15 · fusionar punteros (D5) ═══════════════════════════════════════════
CON_API=1 montar e5
T="$T_ACTUAL"
# la rama construye copiaBase (la pasada del Job, como en 11)
git clone -q -b "$RAMA" "$FORJA" "$T/job"; git -C "$T/job" config core.autocrlf false
( cd "$T/job" && "$ORE" overlay . --main origin/main && reclaman "$RAMA" "$T/ramas-job" \
  && "$ORE" materialize . --vista ventas.copiaBase --recoger --reclaman "$T/ramas-job" --informe datasets \
  && "$ORE" overlay . --undo && git add -A datasets \
  && git -c user.name=copiador -c user.email=copiador@invalido commit -qm "Copia en la rama" && git push -q origin HEAD:"$RAMA" ) > "$T/job.txt" 2>&1 \
  || falla "12 · la pasada del Job en la rama: $(tail -4 "$T/job.txt")"
pide() { # metodo ruta [cuerpo] → código; en $T/r.json
  if [ -n "${3:-}" ]; then curl -s -o "$T/r.json" -w '%{http_code}' -X "$1" -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' "$BASE$2" -d "$3"
  else curl -s -o "$T/r.json" -w '%{http_code}' -X "$1" -H 'x-ore-sujeto: persona:ana' "$BASE$2"; fi
}
c=$(pide GET /ramas/bea/datos/cambios)
cam=$("$PY" -c 'import json,sys
d={x["id"]:x for x in json.load(open(sys.argv[1]))["cambios"]}
f=lambda i:(d.get(i,{}).get("estado","-"),d.get(i,{}).get("datos",False),d.get(i,{}).get("enBase","-"))
print(f("Dataset:ventas.base"),f("Dataset:ventas.nueva"),f("Dataset:ventas.copiaBase"))' "$T/r.json" 2>&1)
[ "$c · $cam" = "200 · ('modificado', True, 'datos') ('nuevo', True, '-') ('modificado', True, 'datos')" ] \
  || falla "12 · los cambios de la rama: $c · $cam"
ok "12 · los cambios de la rama: base y copiaBase modificados en sus datos y en conflicto con main; nueva, nueva"

ACT='"activos":["Dataset:ventas.base","Dataset:ventas.nueva","Dataset:ventas.copiaBase"]'
c=$(pide POST /propuestas "{\"rama\":\"$RAMA\",$ACT,\"seco\":true}")
datos() { "$PY" -c 'import json,sys
d={x["activo"]:x for x in json.load(open(sys.argv[1])).get("datos",[])}
print(" ".join(k+"="+d.get(k,{}).get("caso","-")+"/"+d.get(k,{}).get("resultado","-") for k in ["ventas.base","ventas.nueva","ventas.copiaBase"]))' "$T/r.json" 2>&1; }
[ "$c · $(datos)" = "200 · ventas.base=conflicto/sin elegir ventas.nueva=promocion/rama ventas.copiaBase=reconstruir/main" ] \
  || falla "13 · la propuesta en seco: $c · $(datos) · $(head -c 300 "$T/r.json")"
grep -q '"se_pierde":"lo de main desde el punto de salida: 20 fila(s), escrito por persona:ana"' "$T/r.json" \
  || falla "13 · el conflicto no dice qué se pierde: $(head -c 400 "$T/r.json")"
ok "13 · en seco, por puntero: nueva se promociona, base choca sin receta (y dice qué se pierde), copiaBase se reconstruye en main"

c=$(pide POST /propuestas "{\"rama\":\"$RAMA\",\"titulo\":\"los datos\",$ACT}")
N=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1])).get("numero",""))' "$T/r.json")
[ "$c" = 201 ] && [ -n "$N" ] || falla "14 · proponer: $c $(head -c 300 "$T/r.json")"
ANTES=$(git --git-dir="$FORJA" rev-parse main)
c=$(pide POST /propuestas/$N/fusionar)
[ "$c" = 409 ] && grep -q '"resultado":"sin elegir"' "$T/r.json" && [ "$(git --git-dir="$FORJA" rev-parse main)" = "$ANTES" ] \
  || falla "14 · fusionar sin elegir: $c · $(head -c 300 "$T/r.json")"
ok "14 · fusionar sin elegir lo que choca sin receta: 409 con la lista, y main no se mueve"

c=$(pide POST /propuestas/$N/fusionar '{"datos":{"ventas.base":"rama"}}')
[ "$c" = 200 ] || falla "15 · fusionar eligiendo la rama: $c · $(head -c 400 "$T/r.json")"
al_dia=$("$PY" -c 'import json,sys;d=json.load(open(sys.argv[1]));print(d.get("ramaAlDia"), "reconstruir" if str(d.get("reconstruir","")).startswith(("encolado","NO encolado")) else d.get("reconstruir"))' "$T/r.json")
[ "$al_dia" = "True reconstruir" ] || falla "15 · tras fusionar (ramaAlDia, reconstruir): $al_dia · $(head -c 400 "$T/r.json")"
v="$(lee - base)/$(lee - nueva)/$(lee - copiaBase)"
[ "$v" = "25/3/20" ] || falla "15 · main tras fusionar (base/nueva/copiaBase): $v, y no 25/3/20"
[ "$(puntero main base)" = "$(puntero "$RAMA" base)" ] || falla "15 · base no se promocionó tal cual (otro metadata.json en main)"
quedan=$(git --git-dir="$FORJA" diff --name-only main "$RAMA" -- datasets/ventas/default/base.json datasets/ventas/default/nueva.json datasets/ventas/default/copiaBase.json | tr '\n' ' ')
[ -z "$quedan" ] || falla "15 · tras ponerse al día, la rama sigue difiriendo de main en lo fusionado: $quedan"
ok "15 · fusionar eligiendo la rama: main lee base 25 y nueva 3 (su metadata.json, sin mover un byte), copiaBase sigue en 20 y se encola su reconstrucción; la rama se pone al día y lo fusionado deja de ser suyo"

# ══ 16–19 · las colecciones en una rama (D7) ═════════════════════════════════
# Un S3 de mentira que es también el origen: dos PDF bajo `pdfs/`, y la fuente
# abarca sólo ese prefijo (el lago vive bajo `ore/`). `main` tiene la conexión (su paquete con el catálogo); la rama
# da de alta dos bases por la API: `docs` estándar (su colección, mantenida) y
# `docsv` foránea (la suya, virtual).
DRV="$(buscar ore-read-s3)" || { echo "no hay \`ore-read-s3\`"; exit 2; }
export PATH="$(dirname "$DRV"):$PATH"
CON_API=1 montar e7
T="$T_ACTUAL"
export ORE_S3_URL="s3://copia/pdfs/?endpoint=$ORE_R2_S3_ENDPOINT&region=auto&access_key_id=de&secret_access_key=mentira"
"$PY" - "$ORE_R2_S3_ENDPOINT" <<'PYEOF'
import sys, urllib.request
for k, b in (("pdfs/a.pdf", b"%PDF-1.4 a"), ("pdfs/b.pdf", b"%PDF-1.4 b, otra cosa")):
    urllib.request.urlopen(urllib.request.Request(sys.argv[1] + "/copia/" + k, data=b, method="PUT",
                                                  headers={"content-type": "application/pdf"}))
PYEOF
( cd "$T/m" && git pull -q origin main \
  && printf 'datasources:\n  - { name: s3_demo, type: s3, connectionEnv: ORE_S3_URL }\n' >> ontology.config.yaml \
  && "$ORE" source catalog s3_demo --out "$T/catalogo.json" \
  && "$ORE" discover --source s3_demo --type foreign --only default.raiz --no-model --owner team:data \
       --out packages/s3_demo --name s3_demo \
  && "$ORE" materialize . --informe datasets \
  && git add -A && git commit -qm "la conexión s3_demo, con su colección" && git push -q origin HEAD:main ) > "$T/con.txt" 2>&1 \
  || falla "16 · la conexión en main: $(tail -4 "$T/con.txt")"
# La rama trae `main` (lo que la consola hace con «traer main»): sin la conexión
# en su árbol no hay de qué inducir.
c=$(curl -s -o "$T/traer.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json'   "$BASE/ramas/$RAMA/fusionar" -d '{"desde":"main"}')
[ "$c" = 200 ] || falla "16 · traer main a la rama: $c $(head -c 300 "$T/traer.json")"
alta() { # nombre tipo → código; la respuesta en $T/alta-$1.json
  curl -s -o "$T/alta-$1.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' -H "x-ore-rama: $RAMA" \
    -H 'content-type: application/json' "$BASE/paquetes" \
    -d "{\"name\":\"$1\",\"source\":\"s3_demo\",\"type\":\"$2\",\"only\":[\"default.raiz\"]}"
}
encolado() { "$PY" -c 'import json,sys;print(json.load(open(sys.argv[1])).get("encolado",""))' "$T/alta-$1.json"; }
ANTES=$(git --git-dir="$COLA" rev-list --count main)
c1=$(alta docs standard); c2=$(alta docsv foreign)
vistas=$(for f in $(git --git-dir="$COLA" diff --name-only "main~$(( $(git --git-dir="$COLA" rev-list --count main) - ANTES ))" main); do
  git --git-dir="$COLA" show "main:$f" | grep -q "name: RAMA, value: \"$RAMA\"" || continue
  git --git-dir="$COLA" show "main:$f" | sed -n 's/.*name: VISTAS, value: "\([^"]*\)".*/\1/p' | head -1
done | tr ',' '\n' | sort -u | tr '\n' ' ')
[ "$c1/$c2" = "200/200" ] && echo "$vistas" | grep -qw "docs\.raiz" && echo "$vistas" | grep -qw "docsv\.raiz" \
  || falla "16 · el alta en la rama encola sus colecciones: $c1/$c2 · VISTAS en la rama: $vistas · $(encolado docs) · $(encolado docsv)"
ok "16 · dar de alta una base en la rama encola, con la rama, sus colecciones —la mantenida y la virtual— ($vistas)"

# La pasada del Job en la rama, con esas VISTAS (los pasos de malla/48).
git clone -q -b "$RAMA" "$FORJA" "$T/job"; git -C "$T/job" config core.autocrlf false
SOLO=""; for v in $vistas; do SOLO="$SOLO --vista $v"; done
# shellcheck disable=SC2086
( cd "$T/job" && "$ORE" overlay . --main origin/main && reclaman "$RAMA" "$T/ramas-job" \
  && "$ORE" materialize . $SOLO --recoger --reclaman "$T/ramas-job" --informe datasets \
  && "$ORE" overlay . --undo && git add -A datasets \
  && git -c user.name=copiador -c user.email=copiador@invalido commit -qm "Copia en la rama" && git push -q origin HEAD:"$RAMA" ) > "$T/job.txt" 2>&1 \
  || falla "17 · la pasada del Job en la rama: $(tail -5 "$T/job.txt")"
PC=datasets/docs/default/raiz.json; PV=datasets/docsv/default/raiz.json
en() { git --git-dir="$FORJA" cat-file -e "$1:$2" 2>/dev/null && echo si || echo no; }
virt() { git --git-dir="$FORJA" show "$RAMA:$1" 2>/dev/null | "$PY" -c 'import json,sys;print(json.load(sys.stdin).get("virtual"))' 2>/dev/null; }
items() { # rama|- base → número de ítems (o el código)
  local h=(); [ "$1" != - ] && h=(-H "x-ore-rama: $1")
  local c; c=$(curl -s -o "$T/it.json" -w '%{http_code}' -H 'x-ore-sujeto: persona:ana' "${h[@]}" "$BASE/colecciones/$2/default/raiz/items")
  [ "$c" = 200 ] && "$PY" -c 'import json,sys;d=json.load(open(sys.argv[1]));print(len(d.get("items",d if isinstance(d,list) else [])))' "$T/it.json" || echo "$c"
}
v="$(en "$RAMA" $PC)/$(en "$RAMA" $PV)/$(en main $PC)/$(en main $PV) · virtual $(virt $PC)/$(virt $PV) · ítems $(items "$RAMA" docs)/$(items "$RAMA" docsv)/$(items - docs)"
[ "$v" = "si/si/no/no · virtual False/True · ítems 2/2/404" ] \
  || falla "17 · construir y servir en la rama (punteros rama/rama/main/main · virtual · ítems rama/rama/main): $v · $(tail -3 "$T/job.txt")"
borrados=$(git --git-dir="$FORJA" diff --name-status --no-renames main..."$RAMA" -- datasets | awk '$1=="D"{print $2}' | tr '\n' ' ')
[ -z "$borrados" ] || falla "17 · la pasada de la rama borró punteros heredados (la colección de s3_demo que trajo de main): $borrados"
ok "17 · el Job construye las dos en la rama y no en main; se sirven desde la rama (2 ítems cada una) y main no las tiene; lo heredado sigue"

# El mantenimiento de main, con lo que reclama la rama: los blobs de la
# mantenida —que sólo nombra la rama— siguen.
git clone -q "$FORJA" "$T/mant7"; git -C "$T/mant7" config core.autocrlf false
( cd "$T/mant7" && reclaman main "$T/ramas7" )
B0=$(objetos blobs/sha256/)
( cd "$T/mant7" && "$ORE" collections . --recoger --gracia 0 --reclaman "$T/ramas7" ) > "$T/mant7.txt" 2>&1 \
  || falla "18 · el mantenimiento de las colecciones: $(cat "$T/mant7.txt")"
( cd "$T/mant7" && "$ORE" datasets . --recoger --edad 0 --reclaman "$T/ramas7" ) >> "$T/mant7.txt" 2>&1 \
  || falla "18 · el mantenimiento de los datasets: $(tail -3 "$T/mant7.txt")"
v="$B0/$(objetos blobs/sha256/)/$(objetos colecciones/docs/default/raiz/metadata/ | grep -v '^0$' >/dev/null && echo manifiesto) · $(items "$RAMA" docs)"
[ "$v" = "2/2/manifiesto · 2" ] || falla "18 · tras el mantenimiento de main (blobs antes/después, manifiesto · ítems en la rama): $v · $(tr '\n' ' ' < "$T/mant7.txt")"
ok "18 · el mantenimiento de main no se lleva nada de la rama: sus 2 blobs y el manifiesto siguen, y la rama sirve sus 2 ítems"

# Proponer y fusionar la base estándar: su colección viaja CON su puntero.
c=$(pide GET /ramas/bea/datos/cambios)
IDS=$("$PY" -c 'import json,sys
print(",".join(json.dumps(x["id"]) for x in json.load(open(sys.argv[1]))["cambios"] if x.get("ruta","").startswith("packages/docs/")))' "$T/r.json")
col=$("$PY" -c 'import json,sys
c=[x for x in json.load(open(sys.argv[1]))["cambios"] if x.get("kind")=="MediaCollection" and x.get("ruta","").startswith("packages/docs/")]
print(c[0].get("estado"), c[0].get("datos", False)) if c else print("-")' "$T/r.json")
[ "$c · $col" = "200 · nuevo True" ] || falla "19 · la colección en los cambios de la rama: $c · $col"
c=$(pide POST /propuestas "{\"rama\":\"$RAMA\",\"titulo\":\"la base docs\",\"activos\":[$IDS]}")
N=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1])).get("numero",""))' "$T/r.json")
caso=$("$PY" -c 'import json,sys;print(" ".join(d["activo"]+"="+d["caso"] for d in json.load(open(sys.argv[1])).get("datos",[])))' "$T/r.json")
[ "$c" = 201 ] && [ -n "$N" ] && [ "$caso" = "docs.raiz=promocion" ] || falla "19 · proponer: $c · $caso · $(head -c 300 "$T/r.json")"
c=$(pide POST /propuestas/$N/fusionar)
blob() { git --git-dir="$FORJA" rev-parse "$1:$PC" 2>/dev/null; }
[ "$c" = 200 ] && [ -n "$(blob main)" ] && [ "$(blob main)" = "$(blob "$RAMA")" ] && [ "$(items - docs)" = 2 ] \
  || falla "19 · fusionar: $c · main $(blob main) · rama $(blob "$RAMA") · ítems en main $(items - docs) · $(head -c 300 "$T/r.json")"
ok "19 · la colección se propone con sus datos (promoción) y, fusionada, main tiene su puntero tal cual y sirve sus 2 ítems"

if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 los datos en una rama: 1\xe2\x80\x9319\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
