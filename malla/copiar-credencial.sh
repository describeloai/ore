#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# LA CREDENCIAL DE UNA FUENTE, PARA `ore materialize` (0073 §9, fase H3)
#
#   copiar-credencial.sh <fuente>      → el valor, por la salida, y nada más
#
# Es el gancho `ORE_CREDENCIAL` de la pasada del copiador: `ore` lo ejecuta cuando
# necesita la credencial de una fuente y no la tiene en su entorno. Antes la pasada
# compilaba el árbol con `ore view` sólo para saber cuáles pedir (3,3 s medidos en
# victor); ahora las pide quien las necesita, y las mismas: las de las fuentes que
# leen los datasets pedidos.
#
# El testigo del agente y el valor viven en variables de este proceso: no tocan el
# disco. ⛔ A la salida de errores sólo van motivos, nunca un valor.
# ═══════════════════════════════════════════════════════════════════════════
set -u

F=${1:?falta la fuente}
case "$F" in *[!A-Za-z0-9_-]*) echo "\`$F\` no es un nombre de fuente" >&2; exit 2 ;; esac
PUESTO=${PUESTO:-/puesto}
: "${COFRE_URL:?falta COFRE_URL}" "${ORGANIZACION:?}" "${EMISOR:?}" "${DIRECCION:?}" "${REALM:?}"

DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" \
  | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])') \
  || { echo "no se alcanza el proveedor de identidad" >&2; exit 1; }
[ "$DICE" = "$EMISOR/realms/$REALM" ] \
  || { echo "el proveedor de identidad afirma ser $DICE y se esperaba $EMISOR/realms/$REALM" >&2; exit 1; }
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" \
  -d grant_type=client_credentials -d "client_id=$(cat "$PUESTO/agente-cliente")" \
  -d "client_secret=$(cat "$PUESTO/agente-secreto")" \
  | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])') \
  || { echo "el agente no obtuvo testigo" >&2; exit 1; }

# El cuerpo y, en la última línea, el código: todo en memoria.
R=$(curl -s -w '\n%{http_code}' -H "authorization: Bearer $TOK" \
  "$COFRE_URL/organizaciones/$ORGANIZACION/secretos/fuente-$F")
C=$(printf '%s' "$R" | tail -n 1)
[ "$C" = 200 ] || { echo "el custodio contestó $C para \`fuente-$F\`" >&2; exit 1; }
printf '%s' "$R" | sed '$d' | python3 -c 'import json,sys;print(json.load(sys.stdin)["valor"])' \
  || { echo "lo que el custodio dio para \`fuente-$F\` no tiene \`valor\`" >&2; exit 1; }
