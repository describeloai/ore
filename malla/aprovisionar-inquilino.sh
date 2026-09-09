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
#   1. ✓ YA NO. La clave de despliegue que Flux necesitaba para leer el
#      compartimento era el último permiso de clúster del que este guion no
#      podía librarse — y desapareció al mudar el compartimento a la forja: Flux
#      lee con UN testigo de sólo lectura acuñado una vez, y dar de alta un
#      inquilino es crear un repositorio y añadir un colaborador, por API;
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
# Donde esta la forja DESDE DENTRO. Fuera no se alcanza: no tiene puerta al
# mundo, y eso es a proposito.
FORJA_URL="http://forja.forja.svc.cluster.local:3000"
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
# ── ⭐⭐ DOS FORMAS DE LEER LA MISMA FILA, Y NO SON EQUIVALENTES ───────────
#
#   DENTRO   `psql` como `aprovisionador`, que hereda el papel de la `023`:
#            cuatro columnas de una tabla. Ni escribe, ni ve quien es nadie, ni
#            alcanza `cofre`. Es lo que este guion deberia haber usado siempre.
#
#   FUERA    `kubectl exec` en el pod de la base, que es un `psql` como
#            SUPERUSUARIO. Se usa para leer dos columnas y con el mismo acceso
#            se lee `cofre.material`, `iam.persona` e `iam.concesion` enteras.
#
# ⇒ La cabecera de este fichero dice «el aprovisionador NO tiene credenciales de
#   cluster», y con `kubectl exec` eso era FALSO — y de las gordas. El camino de
#   fuera se conserva porque un operador con el cluster en la mano sigue
#   necesitando correr esto a mano, pero **el camino bueno es el de dentro**, y
#   por eso es el primero.
consulta() { # <columna>
  if [ -n "${DENTRO:-}" ]; then
    psql "$(cat /puesto/iam-url)" -tAc \
      "select $1 from iam.organizacion where nombre = '$NOMBRE'" 2>/dev/null | tr -d '\r'
  else
    kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
      "select $1 from iam.organizacion where nombre = '$NOMBRE'" 2>/dev/null | tr -d '\r'
  fi
}
ARBOL=$(consulta arbol)
KEK=$(consulta kek)
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
# ✓ EL DRIVER, POR FIN CON CUENTA PROPIA.
#
# Aquí ponía: *«el driver sigue compartiendo cuenta, y por eso NO se le da nada
# del inquilino»*. Era una limitación aceptada, y la rompió el catálogo: el Job
# que lee un origen tiene que EMPUJAR el resultado al árbol, así que necesita el
# testigo de la forja de SU inquilino.
#
# ⛔ Y con una cuenta compartida, dárselo a uno se lo daba a TODOS — que es
#   exactamente el patrón que estas mismas líneas rechazan arriba para el cofre.
#   El driver era la excepción que quedaba viva.
cuenta "ore-driver-$NOMBRE"
enlace "ore-cofre-$NOMBRE" cofre
enlace "ore-serve-$NOMBRE" ore-serve
enlace "ore-driver-$NOMBRE" driver

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

# ⭐ DENTRO sale del almacen, puesto por el contenedor de inicio en un tmpfs.
#   FUERA lo pone quien corre esto. En los dos casos NO viaja por `argv`.
[ -n "${DENTRO:-}" ] && [ -f /puesto/forja-admin ] && FORJA_ADMIN="$(cat /puesto/forja-admin)"
# ⭐ Y la URL del receptor, por lo mismo: su CAMINO es el secreto, asi que no
#   se escribe aqui. Dentro sale del almacen; fuera, del entorno.
[ -n "${DENTRO:-}" ] && [ -f /puesto/receptor-url ] && RECEPTOR="$(cat /puesto/receptor-url)"
: "${RECEPTOR:=}"
if [ -z "${FORJA_ADMIN:-}" ] && [ -z "$SECO" ]; then
  falla "falta \`FORJA_ADMIN\`, el testigo con el que se crean usuarios y repositorios.
     No se lee de ningun \`Secret\` del cluster a proposito: si este guion supiera
     sacarlo de ahi, necesitaria permisos de cluster — y eso es lo que la \`0022\`
     quito. Se pasa por el entorno, y en su forma final lo inyecta el Job."
fi
# ── ⚠️⚠️ LO QUE LA PRUEBA EN SECO NO PODIA DESTAPAR ────────────────────────
#
# La primera corrida de verdad lo destapo en esta misma linea. Escribia el
# cuerpo en `-o /tmp/r`, DENTRO del pod de la forja — y ese sistema de ficheros
# es de solo lectura. `curl` salio con **23** en las tres llamadas.
#
# ⇒ Y la salida dijo «✓ organizacion y repositorio» igual, porque el llamante
#   redirigia a `/dev/null` y no miraba nada. La peticion habia funcionado —lo
#   que fallo fue guardar la respuesta— pero eso es suerte: con este codigo, una
#   forja caida habria dado exactamente la misma linea verde.
#
# ⭐ Dos arreglos, y el segundo es el que importa: el cuerpo no nos interesa y va
#   a `/dev/null`, y **el codigo HTTP se comprueba aqui dentro**. Un
#   aprovisionador que afirma haber creado lo que no creo es peor que uno que
#   falla.
#
# ⚠️ `409` y `422` son exito: son «ya existia», que es justo lo que la
#   idempotencia del guion pide en una segunda pasada.
# Una llamada con autenticacion BASICA, que es la unica forma de acuñar el
# testigo de un usuario. Imprime el `sha1` del testigo y nada mas.
#
# ⚠️ El `sed` es fragil a proposito: no hay `jq` en la imagen de drivers y meter
#   un analizador de JSON aqui seria una dependencia por un campo. Si Forgejo
#   cambiara la forma de esa respuesta, `TESTIGO` saldria vacio — y el paso ⑤ lo
#   nota, porque no guarda un secreto vacio.
forja_basica() { # <usuario> <clave> <camino> <cuerpo>
  local u="$1" p="$2" c="$3" d="$4"
  if [ -n "${DENTRO:-}" ]; then
    curl -sS -u "$u:$p" -H 'Content-Type: application/json' --data "$d" \
      "$FORJA_URL/api/v1$c" 2>/dev/null
  else
    kubectl exec -n "$FORJA_NS" forja-0 -- curl -sS -u "$u:$p" \
      -H 'Content-Type: application/json' --data "$d" \
      "http://localhost:3000/api/v1$c" 2>/dev/null
  fi | tr -d '\r' | sed -n 's/.*"sha1":"\([^"]*\)".*/\1/p'
}

forja_api() { # <metodo> <camino> [cuerpo] — imprime el codigo, o corta el guion
  local m="$1" c="$2" d="${3:-}" cod
  # ⭐ DENTRO se habla con la forja por su `Service`; FUERA hay que entrar en su
  #   pod, porque no tiene puerta al mundo. Es la misma llamada por dos caminos,
  #   y el de dentro no necesita ni una credencial de cluster.
  if [ -n "${DENTRO:-}" ]; then
    cod=$(curl -sS -o /dev/null -w '%{http_code}' \
      -X "$m" -H "Authorization: token $FORJA_ADMIN" -H 'Content-Type: application/json' \
      ${d:+--data "$d"} "$FORJA_URL/api/v1$c" 2>/dev/null | tr -d '\r')
  else
    cod=$(kubectl exec -n "$FORJA_NS" forja-0 -- curl -sS -o /dev/null -w '%{http_code}' \
      -X "$m" -H "Authorization: token $FORJA_ADMIN" -H 'Content-Type: application/json' \
      ${d:+--data "$d"} "http://localhost:3000/api/v1$c" 2>/dev/null | tr -d '\r')
  fi
  case "$cod" in
    2??|409|422) echo "$cod" ;;
    *) falla "la forja contesto '$cod' a $m $c" ;;
  esac
}

if [ -n "$SECO" ]; then
  haria "crear la organizacion $PROPIETARIO y el repositorio $ARBOL en la forja"
  haria "crear el usuario serve-$NOMBRE, hacerlo colaborador con escritura, y acunar su testigo"
else
  hecho "organizacion $PROPIETARIO · $(forja_api POST "/orgs" "{\"username\":\"$PROPIETARIO\"}")"
  # ── ⭐⭐ EL AVISO, Y A NIVEL DE ORGANIZACION ─────────────────────────────
  #
  # Va AQUI y no junto a un repositorio, y esa es toda la diferencia entre un
  # sustrato y contabilidad:
  #
  #   por repositorio    hay que acordarse por cada uno. El arbol hoy, el
  #                      compartimento despues, y lo que venga manana
  #   por ORGANIZACION   UNA llamada, y quedan cubiertos todos los
  #                      repositorios del inquilino — incluidos los que
  #                      todavia no existen
  #
  # ⇒ Y por eso esta antes de crear el arbol: asi el primer commit del arbol ya
  #   avisa. Al reves, el unico empujon que nadie oiria seria justo el primero.
  #
  # ⛔ La URL es una CONSTANTE, y eso no es comodidad: es lo que permite que este
  #   guion no tenga permisos de cluster. Con un `Receiver` por inquilino, Flux
  #   publicaria un camino distinto por cada uno en `.status.webhookPath` y
  #   habria que LEERLO del servidor de la API. Con uno solo para todos, se
  #   escribe siempre lo mismo. Ver `17-el-aviso.yaml`.
  #
  # ⚠️ El camino ES el secreto —`generic` no verifica firma— asi que la URL
  #   entera viene del almacen, no de aqui.
  hecho "avisara a Flux en cada empujon · $(forja_api POST "/orgs/$PROPIETARIO/hooks" \
    "{\"type\":\"gitea\",\"active\":true,\"events\":[\"push\"],\
\"config\":{\"url\":\"$RECEPTOR\",\"content_type\":\"json\"}}")"
  hecho "repositorio $ARBOL · $(forja_api POST "/orgs/$PROPIETARIO/repos" "{\"name\":\"$REPO\",\"private\":true}")"
  # ── ⭐⭐ POR API, Y ESTO ES LO QUE PERMITE QUE SEA UN JOB ────────────────
  #
  # Las dos llamadas de abajo eran `kubectl exec … su git -c "forgejo admin …"`,
  # y parecian irreductibles: no hay endpoint de ADMIN para acuñar el testigo de
  # otro usuario.
  #
  # ⇒ Pero este guion CREA a ese usuario, asi que la contraseña la pone el — y
  #   con autenticacion basica puede pedir el testigo en su nombre. El `exec`
  #   era comodidad, no necesidad. Probado contra la forja de verdad con un
  #   usuario de usar y tirar: 201, 201, 204.
  #
  # ⚠️ La contraseña se genera aqui, se usa una vez y no se guarda en ningun
  #   sitio. `serve-<inquilino>` NO es una persona: nadie va a iniciar sesion
  #   con ella. Lo que sale de aqui y sirve es el testigo, y ese va al almacen.
  CLAVE="s-$(head -c 18 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 22)"
  forja_api POST "/admin/users" "{\"username\":\"serve-$NOMBRE\",\
\"email\":\"serve-$NOMBRE@invalido.paladio.io\",\"password\":\"$CLAVE\",\
\"must_change_password\":false}" >/dev/null
  hecho "usuario serve-$NOMBRE"
  hecho "colaborador de SU arbol, y de ninguno mas · $(forja_api PUT \
    "/repos/$ARBOL/collaborators/serve-$NOMBRE" '{"permission":"write"}')"
  # ⛔ Con BASICA y no con el testigo de administrador: Forgejo exige que quien
  #   pide un testigo sea su dueño. Es una regla suya y es la correcta.
  TESTIGO=$(forja_basica "serve-$NOMBRE" "$CLAVE" \
    "/users/serve-$NOMBRE/tokens" '{"name":"ore-serve","scopes":["write:repository"]}')
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
  "$GCLOUD" secrets create "$NS-forja-token" --replication-policy=user-managed     --locations="$LUGAR" >/dev/null 2>&1 || true
  # ⛔ Por FICHERO y no por `--data-file=-`: el valor no pasa por `argv`, que lo
  #   lee cualquier proceso de la maquina. Y el fichero se borra a continuacion.
  printf %s "$TESTIGO" > "$TMP/t"
  "$GCLOUD" secrets versions add "$NS-forja-token" --data-file="$(ruta "$TMP/t")" >/dev/null     && hecho "testigo guardado en el almacen, y NO en un \`Secret\`"
  rm -f "$TMP/t"
fi
# ⚠️ DOS cuentas y no una, y las dos son de ESTE inquilino. El servidor lo lee
#   para clonar el arbol; el driver, para EMPUJAR el catalogo que acaba de leer
#   del origen. Ninguna otra cuenta del proyecto lo alcanza — y eso es justo lo
#   que la cuenta compartida del driver hacia imposible.
for c in "ore-serve-$NOMBRE" "ore-driver-$NOMBRE"; do
  correr "$GCLOUD" secrets add-iam-policy-binding "$NS-forja-token" \
    --member="serviceAccount:$c@$PROYECTO.iam.gserviceaccount.com" \
    --role=roles/secretmanager.secretAccessor \
    && hecho "\`$c\` puede leerlo, y nadie de fuera del inquilino"
done

# ══════════════════════════════════════════════════════════════════════════
paso "⑥ EL REPOSITORIO DE INSTANCIA — aquí es donde el alta queda escrita"
# ══════════════════════════════════════════════════════════════════════════
# ── ⭐⭐ EN LA FORJA, Y ESO QUITÓ LAS DOS CREDENCIALES QUE QUEDABAN ────────
#
# El compartimento vivía en GitHub. Costaba dos cosas que ya no cuesta:
#
#   · una clave de despliegue POR INQUILINO, que `source-controller` lee de
#     etcd ⇒ `create secret` en `flux-system`, o sea una sesión de operador
#     por cada alta;
#   · y un testigo de GitHub **con escritura** dentro del aprovisionador, para
#     poder crear el repositorio. Ése era el que impedía que esto fuera un Job.
#
# ⇒ Aquí las dos desaparecen: crear el repositorio y añadir un colaborador son
#   llamadas a la API que este guion ya sabe hacer, y Flux lee con UN testigo de
#   sólo lectura acuñado una vez.
#
# ⛔ Lo que se pierde está dicho en `13-…`: transferirle el repositorio a un
#   cliente que exija la propiedad deja de significar nada, porque el servidor
#   sigue siendo nuestro.
COMPARTIMENTO="$PROPIETARIO/compartimento"
if [ -n "$SECO" ]; then
  haria "crear $COMPARTIMENTO y hacer a \`flux\` colaborador de solo lectura"
else
  hecho "compartimento $COMPARTIMENTO · $(forja_api POST "/orgs/$PROPIETARIO/repos" \
    "{\"name\":\"compartimento\",\"private\":true}")"
  # ⭐ Y el agente entra como COLABORADOR, uno a uno. Lo que ata a `flux` no es
  #   el ámbito de su token —`read:repository` a secas— sino de qué es
  #   colaborador. Es la misma figura que `serve-<inquilino>`, que dio 404 sobre
  #   el árbol ajeno y no 403.
  hecho "\`flux\` lo lee, y ningun otro · $(forja_api PUT \
    "/repos/$COMPARTIMENTO/collaborators/flux" '{"permission":"read"}')"
fi

# ── ⭐⭐ LO QUE HACE DE ESTO UN RECONCILIADOR Y NO UN INSTALADOR ──────────
#
# Hasta aquí el compartimento salía de la plantilla y nada más: siempre los
# mismos siete manifiestos. Esto lee EL ÁRBOL —la verdad del inquilino— y emite
# además un Job de catálogo por cada fuente que todavía no tiene paquete.
#
# ⇒ Y con eso el ciclo de vida del producto pasa por la plataforma sin que nadie
#   despache nada: alguien da de alta un origen con `ore source add`, `ore-serve`
#   lo empuja al árbol, la forja avisa, esto renderiza, y Flux crea el Job.
#
# ⛔ Se lee por API y no clonando. El testigo de administrador ya alcanza
#   cualquier repositorio, y clonar el árbol entero para mirar dos listas sería
#   traerse la ontología de un cliente a un disco temporal sin necesidad.
#
# ⚠️ Y si algo de esto falla —el árbol no existe todavía, el manifiesto no
#   analiza— se sigue con la lista VACÍA y se dice. Un aprovisionador que se
#   niega a montar el compartimento porque no supo leer una lista deja al
#   inquilino sin nada; uno que monta lo de siempre deja al inquilino en pie y
#   el catálogo llega en la siguiente pasada.
FUENTES=""
if [ -z "$SECO" ]; then
  crudo() { # <camino dentro del repositorio>
    if [ -n "${DENTRO:-}" ]; then
      curl -sS -H "Authorization: token $FORJA_ADMIN" "$FORJA_URL/api/v1/repos/$ARBOL/$1" 2>/dev/null
    else
      kubectl exec -n "$FORJA_NS" forja-0 -- curl -sS \
        -H "Authorization: token $FORJA_ADMIN" "http://localhost:3000/api/v1/repos/$ARBOL/$1" 2>/dev/null
    fi
  }
  FUENTES=$( { crudo "raw/ontology.config.yaml"; printf '\n\036\n'; crudo "contents/packages"; } \
    | "$PY" -c '
import json, re, sys
manifiesto, _, paquetes = sys.stdin.read().partition("\n\x1e\n")
# Las dos formas que `ore source add` puede haber dejado: el bloque de una
# linea que `ore init` documenta, y el de varias que escribe el propio verbo.
dentro, fuentes = False, []
for l in manifiesto.splitlines():
    if re.match(r"^datasources:", l):
        dentro = True; continue
    if dentro and l and not l[0].isspace():
        break
    if not dentro:
        continue
    m = re.search(r"^\s*-\s*(?:\{\s*)?name:\s*([A-Za-z0-9_]+)", l)
    if m:
        fuentes.append(m.group(1))
try:
    # Solo DIRECTORIOS: `packages/.gitkeep` es un fichero, no un paquete.
    hechos = {e["name"] for e in json.loads(paquetes) if e.get("type") == "dir"}
except Exception:
    hechos = set()
print(",".join(f for f in fuentes if f not in hechos))
' 2>/dev/null)
fi
[ -n "$FUENTES" ] && hecho "fuentes sin paquete: $FUENTES"

"$PY" "$(ruta "${GEN:-$RAIZ/malla/gen-inquilino.py}")" "$NOMBRE" --arbol "$ARBOL" \
  ${FUENTES:+--fuentes "$FUENTES"} --a "$(ruta "$TMP/rendido")" \
  >/dev/null || falla "no se pudo renderizar"
hecho "renderizado: $(ls "$TMP/rendido" | tr '\n' ' ')"

# ── ⚠️ Y ESTE PASO NO ERA IDEMPOTENTE, que es lo que destapo la SEGUNDA pasada
#
# Hacia `git init` sobre lo recien rendido y empujaba. La primera vez funciona.
# La segunda, el remoto ya tiene una historia que este arbol nuevo no conoce, y
# `push` sale con «rejected — fetch first».
#
# ⇒ Se CLONA lo que hay, se escribe encima lo rendido, y se hace commit **solo
#   si algo cambio**. Asi la segunda pasada dice «ya estaba» en vez de fallar, y
#   una tercera con la plantilla cambiada empuja exactamente la diferencia.
#
# ⭐ Y esto es lo que un aprovisionador tiene que ser: no un acto que ocurre una
#   vez, sino una funcion que converge. La `017` ya lo dijo de la fila y el
#   arbol; aqui vale igual.
# ── ⚠️ Y AQUI HACE FALTA UN TUNEL, que es la señal de que esto quiere ser un Job
#
# La forja **sólo se alcanza desde dentro del clúster** — no tiene Ingress, y es
# a propósito. Así que un guion que corre fuera necesita un `port-forward` para
# empujar, y eso es una credencial de clúster más.
#
# ⇒ No es un defecto de este paso: es la prueba de que este guion tiene que ser
#   el cuerpo de un **Job**. Dentro del clúster, `forja.forja.svc` se alcanza sin
#   túnel y sin `kubectl`, y estas cinco líneas desaparecen.
#
# ⛔ Lo que NO se hace es evitar el túnel escribiendo los ficheros por la API de
#   contenidos de la forja. Se podría —siete llamadas— y se perdería lo que hace
#   que ⑥ converja: `git` es quien sabe si algo cambió, y un `commit` vacío es
#   ruido en una historia que ES la auditoría.
if [ -n "$SECO" ]; then
  haria "empujar esos manifiestos a $COMPARTIMENTO"
else
  # ⭐ DENTRO no hay tunel que abrir: la forja esta a un salto. Estas seis lineas
  #   son exactamente lo que el Job se ahorra, y por eso estan aisladas.
  TUNEL=""
  if [ -n "${DENTRO:-}" ]; then
    URL_COMP="$FORJA_URL/$COMPARTIMENTO.git"
  else
    PUERTO_FORJA=3129
    kubectl port-forward -n "$FORJA_NS" svc/forja "$PUERTO_FORJA:3000" >/dev/null 2>&1 &
    TUNEL=$!
    trap 'kill "$TUNEL" 2>/dev/null; rm -rf "$TMP"' EXIT
    for _ in 1 2 3 4 5 6 7 8; do
      curl -sS -o /dev/null "http://localhost:$PUERTO_FORJA/api/v1/version" 2>/dev/null && break
      sleep 1
    done
    URL_COMP="http://localhost:$PUERTO_FORJA/$COMPARTIMENTO.git"
  fi
  # ⛔ El testigo por `GIT_CONFIG_*` y no dentro de la URL: un
  #   `http://usuario:token@host/…` deja la credencial en la linea de ordenes,
  #   que lee cualquier proceso de la maquina. Es lo mismo que hace
  #   `ore-serve/git.rs`, y por lo mismo.
  export GIT_CONFIG_COUNT=1
  export GIT_CONFIG_KEY_0=http.extraheader
  export GIT_CONFIG_VALUE_0="Authorization: token $FORJA_ADMIN"
  ( set -e
    cd "$TMP"
    git clone -q "$URL_COMP" clon 2>/dev/null \
      || { mkdir -p clon && cd clon && git init -q -b main \
           && git remote add origin "$URL_COMP" && cd ..; }
    # ⛔ Se borra lo que hubiera y se copia lo rendido: la plantilla es la
    #   verdad. Un fichero que el renderizador ya no emite tiene que
    #   DESAPARECER del compartimento — si se quedase, Flux seguiria
    #   obedeciendolo.
    find clon -maxdepth 1 -name '*.yaml' -delete
    cp "$TMP"/rendido/*.yaml clon/
    cd clon
    git add -A
    if git diff --cached --quiet; then
      cd "$TMP"; exit 3
    fi
    git -c user.name=aprovisionador -c user.email=aprovisionador@invalido \
      commit -q -m "El compartimento del inquilino $NOMBRE"
    git push -q -u origin HEAD:main )
  R=$?
  [ -n "$TUNEL" ] && { kill "$TUNEL" 2>/dev/null; trap 'rm -rf "$TMP"' EXIT; }
  unset GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0
  case $R in
    0) hecho "empujado a $COMPARTIMENTO" ;;
    3) ya "los manifiestos de $COMPARTIMENTO" ;;
    *) falla "no se pudo empujar a $COMPARTIMENTO" ;;
  esac
fi

# ══════════════════════════════════════════════════════════════════════════
paso "⑦ LO QUE ESTE SCRIPT NO HACE, Y HAY QUE HACER"
# ══════════════════════════════════════════════════════════════════════════
cat <<FIN

  ✓ 1 · LA CLAVE DE DESPLIEGUE. **Ya no hay ninguna.** Era el ultimo permiso de
       cluster del que este guion no podia librarse —\`source-controller\` la lee
       de etcd, asi que emitirla exigia \`create secret\` en \`flux-system\` y
       convertia cada alta en una sesion de operador.

       Con el compartimento en la forja, Flux lo lee con UN testigo de solo
       lectura acuñado una vez, y el paso ⑥ hace a \`flux\` colaborador del
       repositorio nuevo por API. Nada que aplicar.

  ⛔ 2 · EL ENGANCHE. Un \`GitRepository\` y un \`Kustomization\` para
       \`inquilino-$NOMBRE\`, en \`malla/13-…\`. Vive en NUESTRO arbol a
       proposito: es la parte que decide QUE SE OBEDECE, y si viviera dentro de
       lo que se obedece, quien escribiera ahi cambiaria a que apunta el agente.

  ⛔ 3 · EL ARBOL. \`ore init --name $NOMBRE\`, primer commit y push. Es la E5,
       y necesita la imagen de \`serve\` — la unica con \`git\`.

  ⚠️ 4 · Y LA CUOTA. Un \`ore-serve\` y un cofre mas piden ~150m de CPU, y este
       guion sigue sin contarla: escribe manifiestos sin mirar si caben.

       ⭐ Pero la cuenta que habia aqui escrita estaba MAL leida. Decia «el nodo
         estaba al 96% con un solo inquilino», que se lee como «cada inquilino
         cuesta un nodo». Medido por pod, en un \`e2-standard-2\`:

           los agentes de GKE      865m   46%   ← kube-dns, anetd, fluentbit…
           la plataforma           850m   45%   ← idp, flux, kueue, forja, iam
           EL INQUILINO            150m    8%   ← ore-serve + cofre

       ⇒ El 96% era un SUELO FIJO, y un suelo no se multiplica: es el mismo con
         uno que con cincuenta. Lo que faltaba no era un nodo por cliente, era
         un nodo mas grande para la plataforma. Con \`e2-standard-4\` el conjunto
         queda al 54% y sobran ~1770m: once inquilinos mas, no uno.
FIN
echo
echo "✓ \`$NOMBRE\`${SECO:+ (en seco)}"
