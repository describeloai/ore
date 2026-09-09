#!/usr/bin/env bash
# DESAPROVISIONAR UN INQUILINO — y decir lo que NO se puede deshacer.
#
#   bash malla/desaprovisionar-inquilino.sh <nombre> [--seco]
#
# ── ⭐⭐ Por que esto existe, y no es simetria por gusto ────────────────────
#
# El aprovisionador se probo en seco durante una iteracion entera y parecia
# bueno. La primera corrida de VERDAD destapo tres defectos en diez minutos:
# un `curl` que salia con 23 y una linea verde encima, un `correr` mal escrito,
# y un `git init` que no sobrevivia a la segunda pasada.
#
# ⇒ La leccion no es «probar mas»: es que **un aprovisionador que no se puede
#   deshacer solo se corre de verdad una vez**, y por eso se prueba en seco. Con
#   esto se puede correr, mirar, desmontar y repetir — que es la unica forma de
#   que la E4 sea una propiedad y no una anecdota.
#
# ── ⛔ Lo que NO deshace, y son cosas distintas ─────────────────────────────
#
#   1. LA FILA. `iam.organizacion` es la verdad; borrarla es un acto de
#      operador con un dueno detras, igual que fundar. Aqui no;
#   2. LA CLAVE de KMS. **Google no permite borrar una clave**: solo destruir
#      sus versiones, y con 24h de gracia. Se quita el permiso —que es lo que
#      importa— y la clave queda, vacia de uso. Es una propiedad del KMS, no
#      un descuido nuestro;
#   3. LOS `Secret` y el namespace del cluster. El aprovisionador nunca los
#      creo —esa es la propiedad entera de la `0022`— asi que tampoco los
#      borra. Se borran quitando el enganche de la `13-…` y dejando que Flux
#      pode, que es como se borra cualquier cosa aqui.
set -u

NOMBRE="${1:-}"
SECO=""
for a in "$@"; do [ "$a" = "--seco" ] && SECO="1"; done
[ -n "$NOMBRE" ] && [ "${NOMBRE#--}" = "$NOMBRE" ] || {
  echo "uso: bash malla/desaprovisionar-inquilino.sh <nombre> [--seco]" >&2
  exit 64
}

PROYECTO="project-8853a180-450d-47be-b83"
LUGAR="europe-west1"
LLAVERO="ore"
FORJA_NS="forja"
NS="t-$NOMBRE"

falla() { echo "✗ $*" >&2; exit 1; }
paso()  { echo; echo "── $* ──────────────────────────────────────"; }
hecho() { echo "  ✓ $*"; }
# ⛔ Y en seco, `ya` se calla. Es el mismo tropiezo que el aprovisionador ya
#   documento al reves: alli `correr` devolvia 0 y la salida afirmaba haber
#   hecho lo que solo iba a hacer; aqui devuelve 1 y la rama `|| ya` afirmaba
#   que NO estaba algo que nadie habia mirado. En seco no se comprueba nada,
#   asi que en seco no se dice nada de lo que hay.
ya()    { [ -n "$SECO" ] || echo "  · $* — no estaba"; }
haria() { echo "  ~ $*"; }
correr() {
  if [ -n "$SECO" ]; then haria "$(echo "$*" | sed 's|^[^ ]*[/\\]||')"; return 1; fi
  "$@" >/dev/null 2>&1
}

GCLOUD="$(command -v gcloud.cmd || command -v gcloud)" || falla "hace falta gcloud"

echo "DESAPROVISIONAR \`$NOMBRE\`${SECO:+   (EN SECO: no se borra nada)}"

ARBOL=$(kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
  "select arbol from iam.organizacion where nombre = '$NOMBRE'" 2>/dev/null | tr -d '\r')
[ -n "$ARBOL" ] || falla "\`$NOMBRE\` no esta fundada: no hay de donde leer que borrar"
PROPIETARIO="${ARBOL%%/*}"

# ══════════════════════════════════════════════════════════════════════════
paso "① EL PERMISO SOBRE LA LLAVE — lo primero, y a proposito"
# ══════════════════════════════════════════════════════════════════════════
#
# ⭐ Se quita ANTES que las cuentas. Al reves quedaria, durante unos segundos,
#   una politica que nombra a una cuenta que ya no existe — y esas politicas se
#   quedan escritas con un identificador `deleted:` que reaparece si alguien
#   recrea la cuenta con el mismo nombre. Quitar el permiso primero cierra eso.
correr "$GCLOUD" kms keys remove-iam-policy-binding "$NOMBRE" --location="$LUGAR" \
  --keyring="$LLAVERO" --role=roles/cloudkms.cryptoKeyEncrypterDecrypter \
  --member="serviceAccount:ore-cofre-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  && hecho "nadie puede ya usar $LLAVERO/$NOMBRE" || ya "el permiso sobre la clave"
echo "  ⚠️ la CLAVE se queda: Google no deja borrar claves, solo destruir versiones"

# ══════════════════════════════════════════════════════════════════════════
paso "② LAS CUENTAS DE GOOGLE"
# ══════════════════════════════════════════════════════════════════════════
for c in "ore-cofre-$NOMBRE" "ore-serve-$NOMBRE"; do
  correr "$GCLOUD" iam service-accounts delete "$c@$PROYECTO.iam.gserviceaccount.com" --quiet \
    && hecho "cuenta $c" || ya "la cuenta $c"
done
# El driver comparte cuenta con todos los inquilinos: se le quita SU enlace, no
# la cuenta. Borrarla dejaria sin identidad a los demas.
correr "$GCLOUD" iam service-accounts remove-iam-policy-binding \
  "ore-driver@$PROYECTO.iam.gserviceaccount.com" --role=roles/iam.workloadIdentityUser \
  --member="serviceAccount:$PROYECTO.svc.id.goog[$NS/driver]" \
  && hecho "enlace ore-driver ← $NS/driver" || ya "el enlace del driver"

# ══════════════════════════════════════════════════════════════════════════
paso "③ EL ALMACEN"
# ══════════════════════════════════════════════════════════════════════════
correr "$GCLOUD" secrets delete "$NS-forja-token" --quiet \
  && hecho "secreto $NS-forja-token" || ya "el secreto $NS-forja-token"

# ══════════════════════════════════════════════════════════════════════════
paso "④ LA FORJA — el arbol, el compartimento, la organizacion y el usuario"
# ══════════════════════════════════════════════════════════════════════════
#
# ⚠️ En este orden y no en otro: una organizacion con repositorios dentro no se
#   borra, y un usuario que es dueno de algo tampoco.
if [ -n "$SECO" ]; then
  haria "borrar $ARBOL, $PROPIETARIO/compartimento, la organizacion y serve-$NOMBRE"
elif [ -z "${FORJA_ADMIN:-}" ]; then
  falla "falta \`FORJA_ADMIN\`. Se acuna sin contrasena, desde dentro:
     kubectl exec -n forja forja-0 -- su git -c \\
       'forgejo admin user generate-access-token --username ore-admin \\
        --token-name desaprovisionar --scopes write:admin,write:organization,write:repository'"
else
  api() { kubectl exec -n "$FORJA_NS" forja-0 -- curl -sS -o /dev/null -w '%{http_code}' \
    -X "$1" -H "Authorization: token $FORJA_ADMIN" "http://localhost:3000/api/v1$2" \
    2>/dev/null | tr -d '\r'; }
  hecho "arbol $ARBOL · $(api DELETE "/repos/$ARBOL")"
  # ⭐ El compartimento, y ANTES que la organizacion. Al mudarlo a la forja entro
  #   en el mismo sitio que el arbol, asi que borrarlo dejo de pedir el alcance
  #   `delete_repo` de GitHub — el desmontaje se volvio simetrico.
  #
  # ⚠️ Y el orden no es estetico: una organizacion con repositorios dentro NO se
  #   borra. Con el compartimento despues, el `DELETE /orgs` habria fallado con
  #   un codigo que no dice nada de repositorios.
  hecho "compartimento $PROPIETARIO/compartimento · $(api DELETE "/repos/$PROPIETARIO/compartimento")"
  hecho "organizacion $PROPIETARIO · $(api DELETE "/orgs/$PROPIETARIO")"
  kubectl exec -n "$FORJA_NS" forja-0 -- su git -c \
    "forgejo admin user delete --username serve-$NOMBRE --purge" >/dev/null 2>&1 \
    && hecho "usuario serve-$NOMBRE" || ya "el usuario serve-$NOMBRE"
fi

# ══════════════════════════════════════════════════════════════════════════
echo
echo "✓ \`$NOMBRE\` desmontado${SECO:+ (en seco)} — menos la fila, la clave y lo del cluster"
