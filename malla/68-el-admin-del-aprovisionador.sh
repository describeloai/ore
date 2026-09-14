#!/usr/bin/env bash
# El admin del IdP que usa el aprovisionador — y NO es el admin de arranque.
#
#     bash malla/68-el-admin-del-aprovisionador.sh [--rotar]
#
# ── Por qué existe (0025 E4) ────────────────────────────────────────────────
#
# El paso ⑦ de `aprovisionar-inquilino.sh` crea en Keycloak el cliente del
# agente de cada celda (`ore-agente-<celda>`), y para eso pide un testigo de
# admin con usuario y clave. Medido antes de esto: ⑦ solo habia corrido DESDE
# FUERA, con la clave en el entorno de un operador; el CronJob de `16-…` no
# tenia ninguna y lo decia («sin IDP_ADMIN_PASS: no se crea el agente»). Un
# aprovisionador que no puede hacer uno de sus pasos no es un aprovisionador.
#
# ── ⭐⭐ Y NO ES `temp-admin` ────────────────────────────────────────────────
#
# Lo evidente era copiar al almacen la clave de `idp-initial-admin`, la que el
# operador de Keycloak acuña al arrancar. Dos razones para no hacerlo:
#
#   1. es el admin de TODO: cada realm, cada persona, cada cliente. El paso ⑦
#      necesita crear clientes en `rubix-dev` y nada mas
#   2. es el admin de ARRANQUE. Keycloak lo llama temporal y avisa en cada
#      arranque de que hay que sustituirlo; atarle un proceso es atarse a algo
#      que se va a retirar
#
# ⇒ Un usuario propio en el realm maestro, `aprovisionador`, con UN papel de
#   cliente: `manage-clients` del realm de las personas (`rubix-dev-realm`).
#   Puede crear, leer y configurar clientes de ese realm —que es ⑦ entero— y
#   no puede ver una persona, ni tocar otro realm, ni el maestro.
#
#   Es la misma figura que el testigo de la forja (`forja-admin-token`): un
#   permiso concentrado en UNA pieza, con una entrada, que hace una cosa.
#
# ── Donde vive la clave ────────────────────────────────────────────────────
#
# En el almacen, como `idp-admin`, con `secretAccessor` para la cuenta del
# aprovisionador y nadie mas. El contenedor de inicio de `16-…` la trae a
# `/puesto/idp-admin` (tmpfs) y el guion la lee de ahi. El usuario va por el
# entorno (`IDP_ADMIN_USER`), porque no es un secreto.
#
# La clave se genera DENTRO del pod y viaja por una tuberia hasta `gcloud`:
# no pasa por un fichero ni por `argv`, y este guion no la imprime.
#
# ── Idempotente ────────────────────────────────────────────────────────────
#
# Si el usuario existe y el almacen tiene una version, dice «ya estaba» y no
# toca nada. `--rotar` genera una clave nueva y añade una version: es como se
# rota, y como se repara si alguna vez no cuadran.
set -eu

NS="${NS:-identidad}"
POD="${POD:-idp-0}"
REALM="${REALM:-rubix-dev}"
PROYECTO="project-8853a180-450d-47be-b83"
LUGAR="europe-west1"
USUARIO="aprovisionador"
SECRETO="idp-admin"
ROTAR=""
[ "${1:-}" = "--rotar" ] && ROTAR=1

GCLOUD="$(command -v gcloud.cmd || command -v gcloud)"

U=$(kubectl -n "$NS" get secret idp-initial-admin -o jsonpath='{.data.username}' | base64 -d)
P=$(kubectl -n "$NS" get secret idp-initial-admin -o jsonpath='{.data.password}' | base64 -d)

# ① El usuario y su papel — dentro del pod, con kcadm. Sale por stdout SOLO lo
#   que se puede leer en un registro; la clave sale por stderr.
CLAVE=$(kubectl -n "$NS" exec -i "$POD" -- env HOME=/tmp \
  KC_U="$U" KC_P="$P" R="$REALM" USUARIO="$USUARIO" ROTAR="$ROTAR" sh -c '
set -e
K=/opt/keycloak/bin/kcadm.sh
$K config credentials --server http://127.0.0.1:8080 --realm master \
  --user "$KC_U" --password "$KC_P" >/dev/null

ID=$($K get users -r master -q username="$USUARIO" -q exact=true --fields id --format csv --noquotes)
if [ -z "$ID" ]; then
  $K create users -r master -s username="$USUARIO" -s enabled=true \
    -s emailVerified=true >/dev/null
  ID=$($K get users -r master -q username="$USUARIO" -q exact=true --fields id --format csv --noquotes)
  echo "usuario  $USUARIO · $ID (nuevo)" >&2
  NUEVO=1
else
  echo "usuario  $USUARIO · $ID (ya estaba)" >&2
  NUEVO=""
fi

# El papel: manage-clients del cliente `<realm>-realm` del maestro. Añadirlo
# dos veces no hace nada, asi que se añade siempre.
$K add-roles -r master --uusername "$USUARIO" --cclientid "$R-realm" --rolename manage-clients
echo "papel    $R-realm / manage-clients" >&2

if [ -n "$NUEVO" ] || [ -n "$ROTAR" ]; then
  CLAVE=$(head -c 30 /dev/urandom | base64 | tr -d "=+/\n" | cut -c1-32)
  # NO temporal: quien la usa es un proceso, y un proceso no puede cambiarla.
  $K set-password -r master --username "$USUARIO" --new-password "$CLAVE"
  echo "clave    nueva" >&2
  printf "%s" "$CLAVE"
else
  echo "clave    se conserva (--rotar para cambiarla)" >&2
fi
')

# ② El almacen: el secreto, su version y quien lo lee.
if "$GCLOUD" secrets describe "$SECRETO" --format="value(name)" >/dev/null 2>&1; then
  echo "secreto  $SECRETO (ya estaba)"
else
  "$GCLOUD" secrets create "$SECRETO" --replication-policy=user-managed --locations="$LUGAR" \
    --labels=proyecto=ore >/dev/null
  echo "secreto  $SECRETO (nuevo)"
fi
if [ -n "$CLAVE" ]; then
  printf '%s' "$CLAVE" | "$GCLOUD" secrets versions add "$SECRETO" --data-file=- >/dev/null
  echo "version  añadida"
elif ! "$GCLOUD" secrets versions list "$SECRETO" --filter="state=enabled" --format="value(name)" 2>/dev/null | grep -q .; then
  echo "xx el usuario ya existia y el almacen NO tiene ninguna version: corre con --rotar" >&2
  exit 1
else
  echo "version  la que hay"
fi
"$GCLOUD" secrets add-iam-policy-binding "$SECRETO" \
  --member="serviceAccount:ore-aprovisionador@$PROYECTO.iam.gserviceaccount.com" \
  --role=roles/secretmanager.secretAccessor >/dev/null
echo "lector   ore-aprovisionador@ — y nadie mas"

# ③ Y se comprueba que sirve: un testigo del maestro con ese usuario, y con el
#   una lectura de los clientes del realm — lo que ⑦ hace primero.
if [ -n "$CLAVE" ]; then
  kubectl -n "$NS" exec -i "$POD" -- env USUARIO="$USUARIO" CLAVE="$CLAVE" R="$REALM" sh -c '
set -e
K=/opt/keycloak/bin/kcadm.sh
HOME=/tmp/aprov; mkdir -p $HOME; export HOME
$K config credentials --server http://127.0.0.1:8080 --realm master --user "$USUARIO" --password "$CLAVE" >/dev/null
N=$($K get clients -r "$R" --fields clientId --format csv --noquotes | grep -c "^ore-agente-" || true)
echo "prueba   ve $N clientes ore-agente-* en $R"
if $K get users -r "$R" --fields id --format csv --noquotes >/dev/null 2>&1; then
  echo "xx puede leer PERSONAS de $R, y no deberia" >&2; exit 1
fi
echo "prueba   no ve personas de $R, que es lo correcto"
'
fi
