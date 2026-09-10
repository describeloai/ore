#!/usr/bin/env bash
# CONVERGER TODOS LOS INQUILINOS — el cuerpo del `CronJob` de `16-…`
#
#   bash malla/converger-inquilinos.sh [<nombre>...] [--seco]
#
#   Sin nombres —la corrida del `CronJob`— converge a TODO el censo. Con
#   nombres, solo a esos, que sirve para mirar uno cuando algo va mal.
#
# ═══════════════════════════════════════════════════════════════════════════
# ⭐⭐ POR QUE ESTO EXISTE, Y QUE ESTABA ROTO SIN EL
#
# `gen-inquilino.py` lo llamaba UN solo sitio: `aprovisionar-inquilino.sh`,
# para UN inquilino, cuando alguien lanzaba el Job a mano. Consecuencia:
#
#   ⇒ cambiar una plantilla NO LLEGABA A NADIE. Ni a `demo`, ni a `prueba`, ni
#     a los que vinieran. El compartimento rendido ayer se quedaba con el
#     manifiesto de ayer para siempre, y nada lo decia.
#
# La pieza dificil ya estaba hecha, y estaba escrita como tal: el paso ⑥ del
# aprovisionador **converge** —clona lo que hay, escribe encima lo rendido, y
# hace commit SOLO si algo cambio—. Su propio comentario lo dice: «no un acto
# que ocurre una vez, sino una funcion que converge».
#
# ⇒ Lo unico que faltaba era quien lo llamara por cada inquilino. Esto.
#
# ── ⛔⛔ Y NI UN PERMISO NUEVO, QUE ERA LA CONDICION ──────────────────────
#
# La alternativa evidente era un lanzador que creara un `Job` por inquilino, y
# eso exige `create job` en `ore-system` sobre una cuenta que puede aprovisionar
# — o sea, un actor nuevo capaz de dar de alta lo que quiera.
#
# Esto corre en el MISMO pod, con la MISMA cuenta y el MISMO `ConfigMap`. La
# frase de `16-…` —«NI UN PERMISO DE RBAC»— sigue siendo verdad palabra por
# palabra.
#
# Y listar el censo tampoco cuesta nada: la `023` concede
# `select (nombre, arbol, kek, entrada) on iam.organizacion` **sin filtro de
# filas**, asi que el papel que ya se usa para leer una fila puede enumerarlas.
#
# ── ⛔⛔ LA GUARDA, Y NO ES OPCIONAL ──────────────────────────────────────
#
# Flux sigue `main` DIRECTAMENTE —`15-…`, `ref: {branch: main}`, cada 5 min— y
# **no espera a CI**. Medido: un commit en rojo llega al cluster igual.
#
# Hasta hoy eso era un agujero acotado: lo peor que pasaba era que la malla
# quedara mal. Con esto, una plantilla mala se rinde en el compartimento de
# TODOS los inquilinos, y sus `Kustomization` llevan `prune: true`.
#
# ⇒ Por eso lo primero que hace esto es `--comprobar-plantillas`, y si falla no
#   toca a nadie. La comprobacion ⑦ —analizar como YAML lo rendido— se escribio
#   el mismo dia y para esto: sin ella, el `44-el-catalogo.yaml` roto que se
#   empujo el 2026-09-09 habria llegado a todos los clientes en una hora.
#
# ⚠️ Lo que la guarda NO cubre: la ⑤ y la ⑥ caminan `malla/` entero y aqui solo
#   hay diez ficheros, asi que quedan fuera. Son las que protegen al
#   REPOSITORIO de un despiste; las que protegen a un INQUILINO de una
#   plantilla mala son las que si corren.
# ═══════════════════════════════════════════════════════════════════════════
set -u

GUION="$(cd "$(dirname "$0")" && pwd)"
PY="${PY:-python3}"
GEN="${GEN:-$GUION/gen-inquilino.py}"
SECO=""
PEDIDOS=""
for a in "$@"; do
  if [ "$a" = "--seco" ]; then SECO="--seco"; else PEDIDOS="$PEDIDOS $a"; fi
done

paso()  { printf '\n== %s\n' "$1"; }
hecho() { printf '   OK %s\n' "$1"; }
falla() { printf '\n xx %s\n' "$1" >&2; exit 1; }

# ══════════════════════════════════════════════════════════════════════════
paso "① LAS PLANTILLAS — antes de tocar a nadie"
# ══════════════════════════════════════════════════════════════════════════
"$PY" "$GEN" --comprobar-plantillas \
  || falla "las plantillas no pasan su propia comprobacion. NO se ha tocado
    ningun inquilino, que es exactamente lo que tiene que pasar: converger con
    una plantilla mala la reparte a todos."

# ══════════════════════════════════════════════════════════════════════════
paso "② EL CENSO — quien hay que converger"
# ══════════════════════════════════════════════════════════════════════════
# ⭐ Las mismas dos formas de leer que `aprovisionar-inquilino.sh`, y por el
#   mismo motivo: DENTRO es el papel de la `023`; FUERA es un operador con el
#   cluster en la mano. El camino bueno es el de dentro, y por eso es el
#   primero.
# ⭐ Con nombres sueltos se convergen SOLO esos, y sirve para mirar uno cuando
#   algo va mal. Sin ellos —la corrida normal del `CronJob`— se convergen todos.
if [ -n "$PEDIDOS" ]; then
  NOMBRES="$PEDIDOS"
elif [ -n "${DENTRO:-}" ]; then
  NOMBRES=$(psql "$(cat /puesto/iam-url)" -tAc \
    "select nombre from iam.organizacion order by nombre" 2>/dev/null | tr -d '\r')
else
  NOMBRES=$(kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
    "select nombre from iam.organizacion order by nombre" 2>/dev/null | tr -d '\r')
fi

# ⛔ Un censo vacio NO es «nada que hacer»: es que la consulta no funciono. Sin
#   esta linea, un `psql` que no arranca —el fallo que ya costo un pod de usar y
#   tirar, cuando `psql: not found` salio como «no esta fundada»— se leeria como
#   una convergencia limpia. Es el mismo cierre que el relevo de CI: si la
#   pregunta no devuelve nada, esto no pasa en verde.
[ -n "$NOMBRES" ] || falla "el censo salio vacio. O no hay ninguna organizacion
    fundada, o la consulta no llego a la base — y las dos se ven igual desde
    aqui, asi que se para."

CUANTOS=$(printf '%s\n' "$NOMBRES" | wc -l | tr -d ' ')
hecho "$CUANTOS inquilinos: $(printf '%s ' $NOMBRES)"

# ⚠️ Y se convergen TODOS los fundados, incluidos los suspendidos — porque el
#   papel de la `023` **no puede leer `estado`**, a proposito: «quien es el
#   dueno y quien esta suspendido no hace falta para crear una clave y un
#   repositorio». Es coherente, y tiene este filo: converger no distingue.
#   El dia que suspender tenga que parar esto, la respuesta no es darle
#   `estado` al aprovisionador — es que fundar retire la fila o que el enganche
#   de `13-…` desaparezca, que es lo que de verdad apaga a un inquilino.

# ══════════════════════════════════════════════════════════════════════════
paso "③ CONVERGER — uno a uno, y uno malo no decide por los demas"
# ══════════════════════════════════════════════════════════════════════════
# ⛔⛔ SIN `set -e` y SIN cortar en el primero que falle. Un inquilino con la
#   forja caida o una fila a medias no puede dejar sin actualizar a los otros
#   nueve — pero tampoco puede pasar en silencio.
#
# ⇒ Se anota, se sigue, y al final se sale en rojo diciendo CUALES. Es la
#   diferencia entre «la convergencia fallo» y «la convergencia fallo para
#   `acme`, y los demas estan al dia».
MALOS=""
for N in $NOMBRES; do
  printf '\n--------------------------------------------------------------\n'
  printf '   %s\n' "$N"
  printf -- '--------------------------------------------------------------\n'
  if bash "$GUION/aprovisionar-inquilino.sh" "$N" $SECO; then
    hecho "$N al dia"
  else
    MALOS="$MALOS $N"
    printf '\n xx %s NO convergio — se sigue con los demas\n' "$N" >&2
  fi
done

# ⚠️ Y esto es una convergencia COMPLETA: cada pasada rehace las comprobaciones
#   de la llave, las cuentas de Google y la forja, aunque lo unico que suela
#   cambiar sea el compartimento. Con dos inquilinos es gratis; con cien son mil
#   llamadas ociosas cada hora.
#
# ⇒ El dia que eso pese, la salida es un `--solo-compartimento` que se salte los
#   pasos ①-⑤ y haga el ⑥. Queda escrito para que se encuentre, y no se hace hoy
#   porque optimizar dos inquilinos es inventarse un problema.
printf '\n==============================================================\n'
if [ -n "$MALOS" ]; then
  falla "no convergieron:$MALOS"
fi
hecho "los $CUANTOS inquilinos estan al dia con las plantillas de este commit"
