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
#   4. todos los datasets en una sola llamada a `ore materialize`, que escribe sus
#      punteros en el catálogo transaccional (0079);
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
# ⭐ 0079 ·5g·3 · LOS PUNTEROS VIVEN EN EL CATÁLOGO TRANSACCIONAL, Y SÓLO AHÍ: `ore` abre su
#   puerta en el esquema del inquilino (`ORE_CATALOGO_ESQUEMA`, lo pone el render), y lo que
#   reclaman las demás ramas lo sabe él. Sin la variable o sin la URL en el tmpfs, la pasada PARA:
#   una que leyera los punteros de git vería un estado viejo, y su `--recoger` retiraría lo vivo
#   (victor, 2026-10-31: 8 datasets, restaurados del borrado reversible del bucket).
[ -n "${ORE_CATALOGO_ESQUEMA:-}" ] || fallan "sin ORE_CATALOGO_ESQUEMA: los punteros viven en el catálogo (0079 ·5g·3)"
CATALOGO_URL="$(cat "$PUESTO/catalogo")" || fallan "sin la URL del catálogo en $PUESTO/catalogo"
export CATALOGO_URL
export ORE_PUNTEROS="ore-catalogo punteros --esquema $ORE_CATALOGO_ESQUEMA"

# ── 3 · las credenciales: las pide `ore` cuando le hacen falta ──────────────
#   0073 §9 H3: antes, `ore view` compilaba el árbol sólo para saber qué fuentes abrir
#   (3,3 s medidos en victor) y la pasada las exportaba. Ahora `ore materialize` ejecuta
#   el gancho (`ORE_CREDENCIAL <fuente>`) cuando una le falta: las mismas credenciales,
#   en su proceso, sin tocar el disco.
export ORE_CREDENCIAL="${ORE_CREDENCIAL:-$(dirname "$0")/copiar-credencial.sh}"

# ── lo que reclaman las demás ramas: lo sabe el catálogo. `--reclaman` va vacío hasta que
#   `ore` deje de pedirlo (0079 ·5g·3 R3).
rm -rf "$DIR/ramas"; mkdir -p "$DIR/ramas"

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

# 0079 ·5d · con el catálogo, el puntero de un dataset lo dice `ore puntero` (no hay
#   fichero), y su estado y motivo se leen del JSON.
texto_de() { ore puntero "$1" --path . 2>/dev/null; }
campo_de() { python3 -c 'import json,sys;print(json.loads(sys.stdin.read()).get(sys.argv[1]) or "")' "$1" 2>/dev/null; }

# `@@ <dataset> listo|fallo <motivo>`, de su puntero en el catálogo (su puntero ya no es el de
#   antes); una vez por dataset.
decir() {
  grep -qx "$1" "$EST/dichos" && return 0
  T=$(texto_de "$1")
  if [ -z "$T" ]; then
    echo "@@ $1 fallo no tiene puntero en el catálogo: la copia no llegó a escribirlo"
  elif [ "$(printf '%s' "$T" | campo_de estado)" = error ]; then
    MOTIVO=$(printf '%s' "$T" | campo_de motivo)
    echo "@@ $1 fallo ${MOTIVO:-la copia terminó en error}"
  else
    paso "\`$1\` publicado"
    echo "@@ $1 listo"
  fi
  echo "$1" >> "$EST/dichos"
}
nuevos_del_catalogo() {
  for v in $(echo "$VISTAS" | tr ',' ' '); do
    grep -qx "$v" "$EST/dichos" && continue
    T=$(texto_de "$v")
    [ -n "$T" ] && [ "$T" != "$(cat "$EST/antes-$v" 2>/dev/null)" ] && decir "$v"
  done
  return 0
}

# El puntero de cada uno antes de copiar: «nuevo» es distinto de esto.
for v in $(echo "$VISTAS" | tr ',' ' '); do texto_de "$v" > "$EST/antes-$v" || true; done
( while [ ! -f "$EST/fin" ]; do sleep 1; nuevos_del_catalogo; done ) &
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

# El `@@` de los que quedan.
nuevos_del_catalogo
for v in $(echo "$VISTAS" | tr ',' ' '); do decir "$v"; done
