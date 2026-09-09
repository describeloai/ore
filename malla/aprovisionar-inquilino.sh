#!/usr/bin/env bash
# APROVISIONAR UN INQUILINO — y **escribir, no aplicar**.
#
#   bash malla/aprovisionar-inquilino.sh <nombre> [--seco]
#
# E4 de la `0022`. Su decisión, entera, en una frase:
#
#   ⭐⭐ El aprovisionador NO tiene credenciales de clúster. Escribe el VALOR de
#     cada secreto en el almacén de la plataforma y el MANIFIESTO que lo
#     referencia en el repositorio del inquilino. Dos escrituras.
#
# Porque la alternativa —que cree los `Secret` él— exige permisos de ámbito de
# clúster sobre `Secret`, y entonces **quien puede crear el de un inquilino
# puede leer el de todos**. Es la pieza más peligrosa del sistema, y este script
# existe para que no exista.
#
# ── ⛔ Lo que NO puede hacer, y son dos ─────────────────────────────────────
#
#   1. el `Secret` de la clave de despliegue que Flux necesita para LEER el
#      repositorio del inquilino. Es el único que no puede venir del almacén:
#      `source-controller` lo lee de etcd, y cambiar eso es otro componente.
#      ⇒ Se emite aquí y se deja escrito qué hay que aplicar. Una línea, con
#        nombre, en vez de un permiso general;
#   2. aplicar `malla/13-…`, el enganche que dice QUÉ SE OBEDECE. Vive en
#      nuestro árbol a propósito —si viviera dentro de lo que se obedece, quien
#      escribiera ahí cambiaría a qué apunta el agente— y lo revisamos nosotros.
#
# ── ⭐ Idempotente, y eso no es higiene ────────────────────────────────────
#
# Aprovisionar falla a la mitad: se cae la red, caduca un token, alguien
# interrumpe. Si repetir no fuera seguro, la recuperación sería un humano
# adivinando por dónde iba. Cada paso mira antes de escribir y dice «ya estaba».
set -u

NOMBRE="${1:-}"
SECO=""
for a in "$@"; do [ "$a" = "--seco" ] && SECO="1"; done
[ -n "$NOMBRE" ] && [ "${NOMBRE#--}" = "$NOMBRE" ] || {
  echo "uso: bash malla/aprovisionar-inquilino.sh <nombre> [--seco]" >&2
  exit 64
}

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PROYECTO="project-8853a180-450d-47be-b83"
LUGAR="europe-west1"
LLAVERO="ore"
FORJA_NS="forja"
DUENNO_GIT="describeloai"
NS="t-$NOMBRE"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

falla() { echo "✗ $*" >&2; exit 1; }
paso()  { echo; echo "── $* ──────────────────────────────────────"; }
hecho() { echo "  ✓ $*"; }
ya()    { echo "  · $* — ya estaba"; }
haria() { echo "  ~ $*"; }

# ⛔ En seco NO se toca nada, y se dice lo que se haría. Un aprovisionador que
#   sólo se puede probar aprovisionando es un aprovisionador que nadie prueba.
#
# ⚠️ Y en seco devuelve **1**, que parece raro y es lo correcto: quien llama
#   escribe `correr … && hecho "…"`, así que con un 0 la salida decía «✓ cuenta
#   creada» justo debajo de «~ crearía la cuenta». Una prueba en seco que afirma
#   haber hecho algo es peor que no tenerla. `1` aquí no significa «fallo»:
#   significa **«no lo hice»**, que es exactamente lo que pasó.
#
# ⛔ Y el silencio lo pone AQUÍ, no quien llama. La primera versión tenía
#   `correr … >/dev/null && hecho`, y ese redirect se tragaba también el «~ …»
#   de la prueba en seco: tres pasos desaparecían de la salida sin que nada
#   fallara. La salida estándar de la orden se calla; **su salida de error no**,
#   porque es la que dice por qué.
correr() {
  if [ -n "$SECO" ]; then haria "$(echo "$*" | sed 's|^[^ ]*[/\\]||')"; return 1; fi
  "$@" >/dev/null
}

ruta() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else echo "$1"; fi; }
GCLOUD="$(command -v gcloud.cmd || command -v gcloud)" || falla "hace falta gcloud"
command -v gh >/dev/null || falla "hace falta gh"
PY=$(command -v python3 || command -v python) || falla "hace falta python"

echo "APROVISIONAR \`$NOMBRE\`${SECO:+   (EN SECO: no se escribe nada)}"

# ══════════════════════════════════════════════════════════════════════════
paso "① LA FILA — y es la verdad de la que todo lo demás converge"
# ══════════════════════════════════════════════════════════════════════════
#
# `fundar` declara el nombre del árbol y el de la llave, y NO los crea. La `017`
# lo argumenta: los dos actos no pueden ser uno —una fila es una transacción y
# crear un repositorio es un efecto externo—, así que se elige cuál es la verdad
# y es la fila. Lo demás converge hacia ella.
#
# ⚠️ Este script LEE esa fila; no la escribe. Fundar es un acto de operador con
#   un dueño detrás, y quién es el dueño no lo sabe un script.
ARBOL=$(kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
  "select arbol from iam.organizacion where nombre = '$NOMBRE'" 2>/dev/null | tr -d '\r')
KEK=$(kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
  "select kek from iam.organizacion where nombre = '$NOMBRE'" 2>/dev/null | tr -d '\r')
[ -n "$ARBOL" ] || falla "\`$NOMBRE\` no esta fundada. Antes de aprovisionar hay que fundar:
    ore-iam fundar --organizacion $NOMBRE --emisor <realm> --sub <sub del dueno>"
hecho "arbol declarado: $ARBOL"
hecho "llave declarada: $KEK"
LLAVE="${KEK#*/}"

# ══════════════════════════════════════════════════════════════════════════
paso "② LA LLAVE — el segundo cerrojo, antes de que haya nada que proteger"
# ══════════════════════════════════════════════════════════════════════════
if "$GCLOUD" kms keys describe "$LLAVE" --location="$LUGAR" --keyring="$LLAVERO" \
     --format="value(name)" >/dev/null 2>&1; then
  ya "la clave $KEK"
else
  # ⭐ Con rotación desde el primer día: una clave sin rotación programada es
  #   una clave que nadie va a rotar.
  correr "$GCLOUD" kms keys create "$LLAVE" --location="$LUGAR" --keyring="$LLAVERO" \
    --purpose=encryption --rotation-period=90d --next-rotation-time="+P90D" \
    && hecho "clave $KEK, con rotacion a 90 dias"
fi

# ══════════════════════════════════════════════════════════════════════════
paso "③ LAS TRES CUENTAS DE GOOGLE — una por inquilino, no una compartida"
# ══════════════════════════════════════════════════════════════════════════
#
# ⛔⛔ Y esto es lo que la medida no había visto: `ore-driver@` es UNA cuenta
#   compartida por los `driver` de TODOS los inquilinos. Con ese patrón, dar
#   permiso sobre la llave de uno lo habría dado sobre la de todos — porque
#   suplantan a la misma cuenta.
#
# ⇒ Aquí no se comparte ninguna que tenga alcance sobre algo del inquilino.
cuenta() { # <nombre corto>
  local c="$1" correo="$1@$PROYECTO.iam.gserviceaccount.com"
  if "$GCLOUD" iam service-accounts describe "$correo" --format="value(email)" >/dev/null 2>&1; then
    ya "la cuenta $c"
  else
    correr "$GCLOUD" iam service-accounts create "$c" && hecho "cuenta $c"
  fi
}
enlace() { # <cuenta corta> <ksa>
  correr "$GCLOUD" iam service-accounts add-iam-policy-binding \
    "$1@$PROYECTO.iam.gserviceaccount.com" --role=roles/iam.workloadIdentityUser \
    --member="serviceAccount:$PROYECTO.svc.id.goog[$NS/$2]" \
    && hecho "enlace $1 ← $NS/$2"
}
cuenta "ore-cofre-$NOMBRE"
cuenta "ore-serve-$NOMBRE"
enlace "ore-cofre-$NOMBRE" cofre
enlace "ore-serve-$NOMBRE" ore-serve
# El driver sigue compartiendo cuenta, y por eso NO se le da nada del inquilino:
# sólo el enlace, para que pueda autenticarse contra Google como hasta ahora.
enlace "ore-driver" driver

correr "$GCLOUD" kms keys add-iam-policy-binding "$LLAVE" --location="$LUGAR" \
  --keyring="$LLAVERO" --role=roles/cloudkms.cryptoKeyEncrypterDecrypter \
  --member="serviceAccount:ore-cofre-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  && hecho "el cofre de \`$NOMBRE\` puede usar $KEK — y ninguna otra"

# ══════════════════════════════════════════════════════════════════════════
paso "④ LA FORJA — el repositorio, su usuario, y un testigo que alcanza UNO"
# ══════════════════════════════════════════════════════════════════════════
#
# ⚠️ Estas cuatro llamadas son las que ningún YAML puede hacer. Y el testigo
#   resultante NO se guarda en un fichero ni se enseña: va derecho al almacén.
#
# ── ⛔⛔ Y AQUÍ ESTÁ LA CONCENTRACIÓN, dicha y no escondida ────────────────
#
# Crear un usuario en Forgejo es un acto de ADMINISTRADOR. No hay forma más
# estrecha: un propietario de organización puede crear repositorios, pero
# usuarios no. Así que **el aprovisionador necesita un testigo de la forja que
# pueda crear usuarios**, y eso es tanto como decir administrador de la forja.
#
# ⇒ Y es aceptable por el mismo argumento con el que se aceptó el `cluster-admin`
#   de Flux: el permiso **no desaparece, se CONCENTRA**. En vez de repartirlo por
#   cada persona que dé de alta a un cliente, lo tiene UNA pieza, con una entrada
#   —este guion—, y todo lo que hace queda en un commit.
#
# ⚠️ Lo que NO se toca es la propiedad de la `0022`: esto sigue sin tener ni una
#   credencial de CLÚSTER. Un testigo de forja no crea `Secret` ni namespaces, y
#   quien lo robe no puede leer lo de otros inquilinos en Kubernetes.
#
# ── ⭐ Y de ahí sale lo que este guion tiene que llegar a ser ──────────────
#
# La forja sólo se alcanza desde dentro: `forja.forja.svc.cluster.local`, sin
# Ingress. Así que un aprovisionador que corra fuera necesitaría abrirla al
# mundo — que es peor.
#
# ⇒ Esto no es un script del portátil de alguien: es el cuerpo de un **Job**, con
#   su `Secret` de testigo de forja y su identidad de Google, y **sin ni un
#   permiso de RBAC**. Escribe en la forja, en el almacén y en git; nunca en el
#   servidor de la API. Hoy se corre a mano porque el Job todavía no existe, y
#   eso es lo único que separa esto de la E4 entera.
PROPIETARIO="${ARBOL%%/*}"
REPO="${ARBOL#*/}"

if [ -z "${FORJA_ADMIN:-}" ] && [ -z "$SECO" ]; then
  falla "falta \`FORJA_ADMIN\`, el testigo con el que se crean usuarios y repositorios.
     No se lee de ningun \`Secret\` del cluster a proposito: si este guion supiera
     sacarlo de ahi, necesitaria permisos de cluster — y eso es lo que la \`0022\`
     quito. Se pasa por el entorno, y en su forma final lo inyecta el Job."
fi
forja_api() { # <metodo> <camino> [cuerpo]
  local m="$1" c="$2" d="${3:-}"
  kubectl exec -n "$FORJA_NS" forja-0 -- curl -sS -o /tmp/r -w '%{http_code}' \
    -X "$m" -H "Authorization: token $FORJA_ADMIN" -H 'Content-Type: application/json' \
    ${d:+--data "$d"} "http://localhost:3000/api/v1$c"
}

if [ -n "$SECO" ]; then
  haria "crear la organizacion $PROPIETARIO y el repositorio $ARBOL en la forja"
  haria "crear el usuario serve-$NOMBRE, hacerlo colaborador con escritura, y acunar su testigo"
else
  forja_api POST "/orgs" "{\"username\":\"$PROPIETARIO\"}" >/dev/null
  forja_api POST "/orgs/$PROPIETARIO/repos" "{\"name\":\"$REPO\",\"private\":true}" >/dev/null
  hecho "organizacion y repositorio $ARBOL"
  kubectl exec -n "$FORJA_NS" forja-0 -- su git -c \
    "forgejo admin user create --username serve-$NOMBRE --email serve-$NOMBRE@invalido.paladio.io --random-password --must-change-password=false" \
    >/dev/null 2>&1 || true
  hecho "usuario serve-$NOMBRE"
  forja_api PUT "/repos/$ARBOL/collaborators/serve-$NOMBRE" '{"permission":"write"}' >/dev/null
  hecho "colaborador de SU arbol, y de ninguno mas"
  TESTIGO=$(kubectl exec -n "$FORJA_NS" forja-0 -- su git -c \
    "forgejo admin user generate-access-token --username serve-$NOMBRE --token-name ore-serve --scopes write:repository" \
    2>/dev/null | tr -d '\r' | sed 's/.*: //')
  # ⛔ Y no se imprime. Va derecho al almacen en el paso siguiente.
  [ -n "$TESTIGO" ] && hecho "testigo acunado · $(printf %s "$TESTIGO" | wc -c) bytes, y no se enseña"
fi

# ══════════════════════════════════════════════════════════════════════════
paso "⑤ EL ALMACÉN — el valor va aquí, y NO a un \`Secret\`"
# ══════════════════════════════════════════════════════════════════════════
#
# ⭐ Ésta es la escritura que hace que todo lo demás sea posible sin credenciales
#   de clúster. El manifiesto referencia `$NS-forja-token`; el contenedor de
#   inicio lo trae a un tmpfs; el valor no pasa por etcd en ningún momento.
if "$GCLOUD" secrets describe "$NS-forja-token" --format="value(name)" >/dev/null 2>&1; then
  ya "el secreto $NS-forja-token"
elif [ -n "$SECO" ]; then
  haria "crear el secreto $NS-forja-token con el testigo del paso ④"
else
  correr "el secreto $NS-forja-token" -- true
  "$GCLOUD" secrets create "$NS-forja-token" --replication-policy=user-managed     --locations="$LUGAR" >/dev/null 2>&1 || true
  # ⛔ Por FICHERO y no por `--data-file=-`: el valor no pasa por `argv`, que lo
  #   lee cualquier proceso de la maquina. Y el fichero se borra a continuacion.
  printf %s "$TESTIGO" > "$TMP/t"
  "$GCLOUD" secrets versions add "$NS-forja-token" --data-file="$(ruta "$TMP/t")" >/dev/null     && hecho "testigo guardado en el almacen, y NO en un \`Secret\`"
  rm -f "$TMP/t"
fi
correr "$GCLOUD" secrets add-iam-policy-binding "$NS-forja-token" \
  --member="serviceAccount:ore-serve-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  --role=roles/secretmanager.secretAccessor \
  && hecho "solo \`ore-serve-$NOMBRE\` puede leerlo"

# ══════════════════════════════════════════════════════════════════════════
paso "⑥ EL REPOSITORIO DE INSTANCIA — aquí es donde el alta queda escrita"
# ══════════════════════════════════════════════════════════════════════════
REPO_INST="$DUENNO_GIT/inquilino-$NOMBRE"
if gh repo view "$REPO_INST" >/dev/null 2>&1; then
  ya "el repositorio $REPO_INST"
else
  correr gh repo create "$REPO_INST" --private \
    --description "El compartimento del inquilino $NOMBRE" && hecho "$REPO_INST"
fi

"$PY" "$(ruta "$RAIZ/malla/gen-inquilino.py")" "$NOMBRE" --arbol "$ARBOL" --a "$(ruta "$TMP/rendido")" \
  >/dev/null || falla "no se pudo renderizar"
hecho "renderizado: $(ls "$TMP/rendido" | tr '\n' ' ')"

if [ -n "$SECO" ]; then
  haria "empujar esos manifiestos a $REPO_INST"
else
  ( cd "$TMP/rendido" && git init -q -b main \
    && git add -A \
    && git -c user.name=aprovisionador -c user.email=aprovisionador@invalido \
         commit -q -m "El compartimento del inquilino $NOMBRE" \
    && git remote add origin "https://github.com/$REPO_INST.git" \
    && git push -q -u origin main ) && hecho "empujado a $REPO_INST"
fi

# ══════════════════════════════════════════════════════════════════════════
paso "⑦ LO QUE ESTE SCRIPT NO HACE, Y HAY QUE HACER"
# ══════════════════════════════════════════════════════════════════════════
cat <<FIN

  ⛔ 1 · LA CLAVE DE DESPLIEGUE de Flux. Es el unico secreto que no puede venir
       del almacen: \`source-controller\` lo lee de etcd. Se emite y se aplica a
       mano, que es UNA linea con nombre en vez de un permiso general:

         ssh-keygen -t ed25519 -N "" -f llave
         gh repo deploy-key add llave.pub --repo $REPO_INST --title "flux"
         kubectl create secret generic inquilino-$NOMBRE-llave -n flux-system \\
           --from-file=identity=llave --from-file=identity.pub=llave.pub \\
           --from-file=known_hosts=<(ssh-keyscan github.com)

  ⛔ 2 · EL ENGANCHE. Un \`GitRepository\` y un \`Kustomization\` para
       \`inquilino-$NOMBRE\`, en \`malla/13-…\`. Vive en NUESTRO arbol a
       proposito: es la parte que decide QUE SE OBEDECE, y si viviera dentro de
       lo que se obedece, quien escribiera ahi cambiaria a que apunta el agente.

  ⛔ 3 · EL ARBOL. \`ore init --name $NOMBRE\`, primer commit y push. Es la E5,
       y necesita la imagen de \`serve\` — la unica con \`git\`.

  ⚠️ 4 · Y LA CUOTA. Un \`ore-serve\` y un cofre mas piden ~150m de CPU. El nodo
       estaba al 96% con un solo inquilino: **aprovisionar tiene que contar
       cuota, no solo escribir manifiestos**, y hoy no la cuenta.
FIN
echo
echo "✓ \`$NOMBRE\`${SECO:+ (en seco)}"
