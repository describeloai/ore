#!/usr/bin/env bash
# Dar de alta a una persona en un realm — lo que hay que hacer y lo que se olvida.
#
#     bash malla/64-la-primera-persona.sh <realm> <usuario> <correo>
#
# ── Por qué esto es un script y no un manifiesto ────────────────────────────
#
# Un `KeycloakRealmImport` **crea el realm y ya**: se salta uno que exista y se
# declara `Done: True` igualmente —medido—. Así que las personas no viajan en el
# import, y meterlas ahí haría que el segundo despliegue las ignorara en
# silencio. Esto es un acto de gobierno, se hace una vez, y se ejecuta.
#
# ── ⭐ Las TRES cosas que se olvidan, y las tres encierran a la persona ─────
#
#   ① `emailVerified: true`
#      El realm trae `verifyEmail: true` y el mailer está APAGADO. Sin esto la
#      primera entrada exige verificar un correo que no va a llegar nunca, y la
#      persona queda encerrada en su propio primer login. No es un atajo: es que
#      la alternativa —encender el correo— es otra decisión y otro día.
#
#   ② la ORGANIZACIÓN
#      `lib/auth/oidc.ts` pide el ámbito `organization`, y su comentario dice por
#      qué: *«es el ámbito que estampa el claim que `modelo/` LEE. Sin él, el
#      token entra sin organización y el núcleo lo rechaza — correctamente, y con
#      un síntoma que manda a mirar al sitio equivocado»*. Una persona que no es
#      miembro entra bien y **falla después**, que es la peor forma.
#
#      ⛔ Y la API tiene una trampa: el cuerpo de `POST …/members` es el id del
#        usuario **como cadena JSON**, con comillas. Sin ellas devuelve un error
#        que no dice nada del formato.
#
#   ③ la contraseña es TEMPORAL
#      `--temporary` deja `requiredActions: [UPDATE_PASSWORD]`, así que la que se
#      teclea aquí sirve una vez. Quien la escribe no es quien la va a usar, y
#      una contraseña que sobrevive a eso es una contraseña compartida.
#
# ── Y lo que pasa DESPUÉS, que sorprende y es correcto ──────────────────────
#
# El flujo `browser-rubix` exige un SEGUNDO FACTOR —`webauthn` o TOTP, como
# alternativas— porque el realm declara AAL2 contra NIST SP 800-63B-4. La
# primera entrada pedirá cambiar la contraseña **y registrar un segundo factor**.
# No es un fallo de configuración: es la configuración.
set -eu

REALM="${1:-rubix-dev}"
USUARIO="${2:?falta el usuario}"
CORREO="${3:?falta el correo}"
NS="${NS:-identidad}"
POD="${POD:-idp-0}"

U=$(kubectl -n "$NS" get secret idp-initial-admin -o jsonpath='{.data.username}' | base64 -d)
P=$(kubectl -n "$NS" get secret idp-initial-admin -o jsonpath='{.data.password}' | base64 -d)

kubectl -n "$NS" exec "$POD" -- env HOME=/tmp \
  KC_U="$U" KC_P="$P" R="$REALM" USUARIO="$USUARIO" CORREO="$CORREO" sh -c '
set -e
K=/opt/keycloak/bin/kcadm.sh
$K config credentials --server http://127.0.0.1:8080 --realm master \
  --user "$KC_U" --password "$KC_P" >/dev/null

$K create users -r "$R" \
  -s username="$USUARIO" -s email="$CORREO" \
  -s emailVerified=true -s enabled=true >/dev/null 2>&1 || echo "(el usuario ya existia)"

ID=$($K get users -r "$R" -q username="$USUARIO" --fields id --format csv --noquotes)
echo "usuario  $USUARIO  ·  $ID"

# La clave se genera DENTRO del pod. Es temporal y se imprime una vez.
CLAVE=$(head -c 18 /dev/urandom | base64 | tr -d "=+/" | cut -c1-16)
$K set-password -r "$R" --username "$USUARIO" --new-password "$CLAVE" --temporary
echo "temporal $CLAVE"

ORG=$($K get organizations -r "$R" --fields id --format csv --noquotes | head -1)
if [ -n "$ORG" ]; then
  # ⛔ ENTRE COMILLAS. El cuerpo es una cadena JSON, no un objeto.
  printf "\"%s\"" "$ID" > /tmp/miembro.json
  $K create organizations/"$ORG"/members -r "$R" -f /tmp/miembro.json 2>&1 | head -1
  echo "miembros $($K get organizations/$ORG/members -r "$R" --fields username --format csv --noquotes | tr "\n" " ")"
else
  echo "AVISO: el realm no tiene organizaciones. El token saldra SIN el claim que el nucleo lee."
fi

echo "acciones $($K get users -r "$R" -q username="$USUARIO" --fields requiredActions --format csv --noquotes)"
'
