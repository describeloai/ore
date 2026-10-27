#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# LA PASADA DEL COPIADOR RESIDENTE (`ore-copiador`)
#
#   copiar-worker.sh <pasada> <dataset,dataset,…>
#
# Es la pasada ACOTADA de `malla/48-la-copia.yaml` (`PROGRAMADA`) sin lo que el Job
# paga por nacer: no hay pod, ni init containers, ni espera de Kueue. Hace lo mismo,
# en el mismo orden y con el mismo `ore materialize`:
#
#   1. el testigo de la forja y el del agente (en un tmpfs): los renueva
#      `ore-copiador` cada 4 min (`copiar-mantener.sh`), y la pasada sólo si faltan;
#   2. un worktree del espejo del árbol que mantiene `ore-copiador`, propio de esta
#      pasada (o un clon, si no hay espejo);
#   3. la credencial de cada fuente que lee un dataset: la pide `ore` al cofre cuando
#      le hace falta (`ORE_CREDENCIAL`, `copiar-credencial.sh`), sólo en su proceso;
#   4. todos los datasets en una sola llamada a `ore materialize`, y sus punteros a
#      la forja (`publicar`, con rebase si la forja avanzó);
#   5. una línea `@@ <dataset> listo|fallo <motivo>` por dataset: es lo que
#      `ore-copiador` cuenta como su estado.
#
# 0073 §9 (fase H): lo que no es trabajo —testigos, clon, compilar el árbol— no se
# paga en la pasada, o se paga una vez y no por dataset.
#
# Lo que NO hace: copias por consulta (necesitan DuckDB, que vive en el puesto), ramas,
# `rehacer` ni refrescos programados. Esas siguen por Job.
# ═══════════════════════════════════════════════════════════════════════════
set -u

PASADA=${1:?falta la pasada}
VISTAS=${2:?faltan los datasets}
PUESTO=${PUESTO:-/puesto}
TRABAJO=${TRABAJO:-/trabajo}
DIR="$TRABAJO/p-$PASADA"

: "${FORJA_URL:?falta FORJA_URL}" "${COFRE_URL:?falta COFRE_URL}" "${SECRETOS:?falta SECRETOS}"
: "${ORGANIZACION:?}" "${EMISOR:?}" "${DIRECCION:?}" "${REALM:?}"

T0=$(date +%s)
paso() { echo "### $(( $(date +%s) - T0 )) s · $*"; }
limpiar() {
  cd /
  # 0073 §9 H2: un worktree del espejo se retira con él; lo que quede, lo poda el mantenimiento.
  [ -n "${DEL_ESPEJO:-}" ] && flock "$ESPEJO.lock" git -C "$ESPEJO" worktree remove --force "$DIR" 2>/dev/null
  rm -rf "$DIR" "$TRABAJO/p-$PASADA.estado"
}

# ── X5 · la ejecución que `ore-serve` abrió para esta pasada (`EJECUCION`) ────
#   Lo que la pasada escribe va también a un registro (un `tee` por un fifo: sh no
#   tiene `>(…)`), y al salir, pase lo que pase, `cerrar-ejecucion.py` la cierra
#   con sus pasos y el final de ese registro (docs/ejecuciones.md §8.12).
mkdir -p "$TRABAJO"
REGISTRO="$TRABAJO/registro-$PASADA.log"
FIFO="$TRABAJO/registro-$PASADA.fifo"
rm -f "$FIFO"; mkfifo "$FIFO"
exec 3>&1 4>&2
tee "$REGISTRO" < "$FIFO" >&3 &
TEE=$!
exec > "$FIFO" 2>&1
SECRETAS=""
terminar() {
  RC=$?
  # 0073 §9 H4: el publicador y el lector de `ore` tienen abierto el registro; si la
  # pasada muere a medias, sin pararlos el `tee` no acaba nunca.
  [ -n "${EST:-}" ] && touch "$EST/fin" 2>/dev/null
  for p in ${PUBLICADOR:-} ${LECTOR:-}; do kill "$p" 2>/dev/null; done
  exec 1>&3 2>&4
  wait "$TEE" 2>/dev/null
  python3 /usr/local/lib/ore/cerrar-ejecucion.py --id "${EJECUCION:-}" --rc "$RC" --inicio "$((T0 * 1000))" \
    --log "$REGISTRO" --vistas "$VISTAS" --arbol "$DIR" --secretas "$SECRETAS" \
    --tiempos "$TRABAJO/p-$PASADA.estado/tiempos" || true
  rm -f "$FIFO" "$REGISTRO"
  limpiar
}
trap terminar EXIT

fallan() { # todos los datasets, con el mismo motivo
  for v in $(echo "$VISTAS" | tr ',' ' '); do echo "@@ $v fallo $1"; done
  exit 1
}

# ── 1 · los testigos, en memoria ────────────────────────────────────────────
#   0073 §9 H1: los renueva `ore-copiador` cada 4 min (`copiar-mantener.sh`), fuera de
#   la pasada. Sólo si no los tiene al día (no arrancó el mantenimiento, o falla) los
#   renueva la pasada, que es lo que hacía siempre.
MANTENER=${MANTENER:-$(dirname "$0")/copiar-mantener.sh}
if ! find "$PUESTO/forja" -mmin -10 2>/dev/null | grep -q .; then
  paso "renovando los testigos (ore-copiador no los tenía al día)"
  sh "$MANTENER" testigos || fallan "no se pudieron leer los testigos de la forja y del agente"
fi

# ── 2 · el clon de esta pasada ──────────────────────────────────────────────
export GIT_TERMINAL_PROMPT=0 GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader \
  GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*'
export GIT_CONFIG_VALUE_0="Authorization: token $(cat "$PUESTO/forja")"
mkdir -p "$TRABAJO"
# 0073 §9 H2: un worktree del espejo que mantiene `ore-copiador` (~0,5 s), al día con
# un fetch bajo su cerrojo; sin espejo, el clon de siempre (3,3 s medidos).
ESPEJO="$TRABAJO/espejo"
en_espejo() { flock "$ESPEJO.lock" git -C "$ESPEJO" "$@"; }
DEL_ESPEJO=""
rm -rf "$DIR"
if [ -d "$ESPEJO/.git" ] && en_espejo fetch -q --prune origin 2>&1 \
  && en_espejo worktree add -q --detach "$DIR" origin/main 2>&1; then
  DEL_ESPEJO=1
  cd "$DIR" || fallan "no hay worktree"
  paso "árbol del espejo"
else
  rm -rf "$DIR"
  git clone --quiet "$FORJA_URL" "$DIR" 2>&1 || fallan "no se pudo clonar el árbol"
  cd "$DIR" || fallan "no hay clon"
  paso "árbol clonado"
fi
mkdir -p "$DIR/calculo"
# Los worktrees comparten las referencias: traerlas, bajo el mismo cerrojo.
traer() {
  if [ -n "$DEL_ESPEJO" ]; then flock "$ESPEJO.lock" git fetch -q origin main; else git fetch -q origin main; fi
}

# ── 3 · las credenciales: las pide `ore` cuando le hacen falta ──────────────
#   0073 §9 H3: antes, `ore view` compilaba el árbol sólo para saber qué fuentes abrir
#   (3,3 s medidos en victor) y la pasada las exportaba. Ahora `ore materialize` ejecuta
#   el gancho (`ORE_CREDENCIAL <fuente>`) cuando una le falta: las mismas credenciales,
#   en su proceso, sin tocar el disco.
export ORE_CREDENCIAL="${ORE_CREDENCIAL:-$(dirname "$0")/copiar-credencial.sh}"

# ── lo que reclaman las demás ramas (para que --recoger no retire lo suyo) ───
rm -rf "$DIR/ramas"; mkdir -p "$DIR/ramas"
for R in $(git for-each-ref refs/remotes/origin --format='%(refname)' | sed 's#^refs/remotes/origin/##'); do
  [ "$R" = HEAD ] || [ "$R" = main ] && continue
  B=$(git merge-base origin/main "origin/$R") || continue
  git diff --name-only --diff-filter=AM "$B" "origin/$R" -- datasets copias resultados | while read -r F; do
    case "$F" in *.json) mkdir -p "$DIR/ramas/$R/$(dirname "$F")"; git show "origin/$R:$F" > "$DIR/ramas/$R/$F" ;; esac
  done
done

# ── publicar: el commit de los punteros y su push, con rebase si la forja avanzó ──
#   Con `sin-rebase` (las publicaciones de mientras `ore` escribe, 0073 §9 H4), si la
#   forja avanzó no se rebasa —`git rebase` no admite un árbol que alguien está
#   escribiendo— y se deja para la siguiente: la última rebasa con el árbol quieto.
publicar() {
  export GIT_AUTHOR_NAME=copiador GIT_AUTHOR_EMAIL=copiador@invalido
  export GIT_COMMITTER_NAME=copiador GIT_COMMITTER_EMAIL=copiador@invalido
  git add -A datasets
  if ! git diff --cached --quiet; then
    git commit -q -m "Copia: $VISTAS" || return 1
  fi
  for _ in 1 2 3 4 5; do
    traer || return 1
    [ -n "$(git rev-list HEAD "^origin/main" | head -1)" ] || return 0
    if [ "${1:-}" = sin-rebase ]; then
      git push -q origin HEAD:main 2>/dev/null
      return
    fi
    git push -q origin HEAD:main && return 0
    git rebase -q origin/main || { git rebase --abort 2>/dev/null; return 1; }
  done
  return 1
}

# ── 4 · los datasets, en una sola llamada; sus punteros, según aparecen ─────
#   0073 §9 H3: una compilación y un barrido por pasada, no por dataset (~3,5 s cada
#   uno medidos en victor). `ore materialize` sigue con los demás cuando uno falla, y
#   cada uno dice lo suyo en su puntero.
#   0073 §9 H4: mientras `ore` copia, un publicador empuja cada segundo los punteros
#   nuevos y dice su `@@` —la consola ve el progreso por ellos—, y en el camino sólo
#   queda la última publicación (~1,7 s medidos por cada una). La hora a la que `ore`
#   empieza cada dataset (la línea con su nombre) da su `duration_ms` en la ejecución.
EST="$TRABAJO/p-$PASADA.estado"
rm -rf "$EST"; mkdir -p "$EST"
: > "$EST/dichos"; : > "$EST/pendientes"; : > "$EST/tiempos"
ms() { python3 -c 'import time;print(int(time.time()*1000))'; }

# El puntero de un dataset, si lo hay.
puntero_de() {
  P="datasets/$(echo "$1" | tr '.' '/').json"
  if [ ! -f "$P" ]; then
    # Un dataset en `default` vive sin su schema en el nombre. Sólo dentro de
    # SU base: buscar en todo `datasets/` tomaba el de otra base con la misma
    # tabla (victor, 2026-10-27: `test1.retail.productos` leyó el error de
    # `cassandra_s_test`).
    P=$(find "datasets/${1%%.*}" -name "$(echo "$1" | awk -F. '{print $NF}').json" 2>/dev/null | head -1)
  fi
  [ -n "$P" ] && [ -f "$P" ] && echo "$P"
}

# `@@ <dataset> listo|fallo <motivo>`, de su puntero; una vez por dataset.
decir() {
  grep -qx "$1" "$EST/dichos" && return 0
  P=$(puntero_de "$1")
  if [ -z "$P" ]; then
    # Sin puntero no hay copia: decir «listo» era mentir. El motivo es lo que
    # `ore` dijo arriba.
    echo "@@ $1 fallo no tiene puntero en datasets/: la copia no llegó a escribirlo"
  elif grep -q '"estado": *"error"' "$P"; then
    MOTIVO=$(sed -n 's/.*"motivo": *"\(.*\)".*/\1/p' "$P" | head -1)
    echo "@@ $1 fallo ${MOTIVO:-la copia terminó en error}"
  else
    paso "\`$1\` publicado"
    echo "@@ $1 listo"
  fi
  echo "$1" >> "$EST/dichos"
}

# Los punteros nuevos, a la forja; y el `@@` de los datasets que traían.
publicar_lo_nuevo() {
  git add -A datasets
  git diff --cached --name-only -- datasets >> "$EST/pendientes"
  [ -s "$EST/pendientes" ] || return 0
  publicar "${1:-}" || return 1
  for v in $(echo "$VISTAS" | tr ',' ' '); do
    P=$(puntero_de "$v")
    [ -n "$P" ] && grep -qx "$P" "$EST/pendientes" && decir "$v"
  done
  : > "$EST/pendientes"
}

( while [ ! -f "$EST/fin" ]; do sleep 1; publicar_lo_nuevo sin-rebase || true; done ) &
PUBLICADOR=$!

# La salida de `ore`, al registro, con la hora a la que empieza cada dataset.
mkfifo "$EST/salida"
( while IFS= read -r l; do
    case ",$VISTAS," in *",$l,"*) echo "$l $(ms)" >> "$EST/tiempos" ;; esac
    printf '%s\n' "$l"
  done < "$EST/salida" ) &
LECTOR=$!

SOLO=""
for v in $(echo "$VISTAS" | tr ',' ' '); do SOLO="$SOLO --vista $v"; done
paso "copiando $(echo "$VISTAS" | tr ',' ' ')"
# Sin comillas a propósito: los nombres los valida `ore-copiador` (sin espacios ni comillas).
ore materialize . $SOLO --recoger --reclaman "$DIR/ramas" --informe datasets --calculado "$DIR/calculo" \
  > "$EST/salida" 2>&1 || echo "⚠️ algún dataset no se copió: su informe en datasets/ lo dice"
wait "$LECTOR"
echo "fin $(ms)" >> "$EST/tiempos"
touch "$EST/fin"
wait "$PUBLICADOR"

# La última: con rebase si la forja avanzó, y el `@@` de los que quedan.
if publicar_lo_nuevo; then
  for v in $(echo "$VISTAS" | tr ',' ' '); do decir "$v"; done
else
  for v in $(echo "$VISTAS" | tr ',' ' '); do
    grep -qx "$v" "$EST/dichos" || echo "@@ $v fallo el puntero no se pudo publicar en la forja"
  done
fi
