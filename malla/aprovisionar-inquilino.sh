#!/usr/bin/env bash
# APROVISIONAR UN INQUILINO — y **escribir, no aplicar**.
#
#   bash malla/aprovisionar-inquilino.sh <celda> [--seco]
#
# ⭐ El argumento es la CELDA (0025 E4): `t-<celda>` es su namespace, su forja,
#   su cofre y su compartimento; la ORGANIZACION —la cuenta: la llave, el
#   dueño— se lee de la celda, no al reves. Hoy `demo` y `prueba` se llaman
#   como su organizacion y todo sale igual; la segunda celda de `prueba` (E6)
#   es la que pasa por aqui con otro nombre.
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
  echo "uso: bash malla/aprovisionar-inquilino.sh <celda> [--seco]" >&2
  exit 64
}

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PROYECTO="project-8853a180-450d-47be-b83"
# ⛔ El NUMERO del proyecto, escrito, y no `gcloud projects describe`: desde
#   dentro esa llamada fallaba —el papel no lee el proyecto y la API de Resource
#   Manager no esta habilitada— y salia un agente del almacen llamado
#   `service-@…`, con lo que la CMEK no se concedia y la condicion del cofre
#   llevaba un prefijo roto. Un numero de proyecto es tan constante como su id.
NUMERO="339497864493"
LUGAR="europe-west1"
# Donde Bastion publica la lista de perfiles certificados (`bastion profiles
# --publish gs://bastion-perfiles`): lectura publica, sin credencial.
PERFILES_URL="https://storage.googleapis.com/bastion-perfiles/perfiles.json"
LLAVERO="ore"
FORJA_NS="forja"
# Donde esta la forja DESDE DENTRO. Fuera no se alcanza: no tiene puerta al
# mundo, y eso es a proposito.
FORJA_URL="http://forja.forja.svc.cluster.local:3000"
NS="t-$NOMBRE"

# ── ⭐⭐ DOS FORJAS, y este guion habla con las dos ──────────────────────────
#
# Desde el 2026-09-14 (0024 E3-(c)) el arbol y la cola del inquilino viven en SU
# forja, en su celda (`46-la-forja-del-inquilino.yaml`); el compartimento —lo
# que su cluster obedece— sigue en la de la plataforma (0022-①). Cada llamada
# de abajo va a UNA de las dos, y se dice con `en_la_forja`:
#
#   central     forja/forja-0        con `FORJA_ADMIN`      el compartimento
#   inquilino   $NS/forja-0          con `FORJA_ADMIN_INQ`  ontologia, trabajo, serve-<n>
#
# El admin de la forja del inquilino lo acuña ella misma al fundarse y lo deja
# en el almacen como `$NS-forja-admin`. Si todavia no esta —la primera pasada
# funda la forja al empujar la 46 en ⑥— los pasos del inquilino se saltan
# diciendolo, y la pasada siguiente (el CronJob, cada hora) los hace. Es lo
# que hace de esto un reconciliador y no un instalador.
F_NS="$FORJA_NS"; F_URL="$FORJA_URL"; F_ADMIN=""; F_PUERTO=3129
en_la_forja() { # central | inquilino
  case "$1" in
    central)   F_NS="$FORJA_NS"; F_URL="$FORJA_URL"; F_ADMIN="${FORJA_ADMIN:-}"; F_PUERTO=3129 ;;
    inquilino) F_NS="$NS"; F_URL="http://forja.$NS.svc.cluster.local:3000"; F_ADMIN="${FORJA_ADMIN_INQ:-}"; F_PUERTO=3131 ;;
  esac
}
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
# ⭐⭐ PRIMERO LA CELDA, por su nombre (029, vista `celda_de` de la 027): de ella
#   salen el arbol, la entrada, la puerta — y la ORGANIZACION. El guion no
#   recibe la organizacion: la deduce. Es la 0025 entera en dos lineas.
celda() { # <columna de iam.celda_de>
  if [ -n "${DENTRO:-}" ]; then
    psql "$(cat /puesto/iam-url)" -tAc \
      "select $1 from iam.celda_de where celda = '$NOMBRE'" 2>/dev/null | tr -d '\r'
  else
    kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
      "select $1 from iam.celda_de where celda = '$NOMBRE'" 2>/dev/null | tr -d '\r'
  fi
}
ORG=$(celda organizacion)
[ -n "$ORG" ] || falla "no hay ninguna celda \`$NOMBRE\`. Una celda nace al fundar su organizacion:
    ore-iam fundar --organizacion <org> --emisor <realm> --sub <sub del dueno>
  y desde la E6 de la 0025, al pedirla: POST /organizaciones/{org}/celdas"
# Y de la organizacion, lo que es de la CUENTA: la llave. Las columnas que el
# papel de la 023 puede ver son `nombre` y `kek` — `arbol` y `entrada` se
# fueron a la celda con la 031.
consulta() { # <columna de iam.organizacion>
  if [ -n "${DENTRO:-}" ]; then
    psql "$(cat /puesto/iam-url)" -tAc \
      "select $1 from iam.organizacion where nombre = '$ORG'" 2>/dev/null | tr -d '\r'
  else
    kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
      "select $1 from iam.organizacion where nombre = '$ORG'" 2>/dev/null | tr -d '\r'
  fi
}
ARBOL=$(celda arbol)
KEK=$(consulta kek)
[ -n "$ARBOL" ] || falla "la celda \`$NOMBRE\` no tiene arbol: la fila esta a medias"
[ -n "$KEK" ] || falla "la organizacion \`$ORG\` no tiene llave: la fila esta a medias"
hecho "organizacion: $ORG"
# ⭐ Y su ESTADO (033): `retirada` no converge — se DESMONTA, mas abajo.
ESTADO=$(celda estado)
[ -n "$ESTADO" ] || ESTADO=activa
hecho "estado: $ESTADO"
hecho "arbol declarado: $ARBOL"
hecho "llave declarada: $KEK"
LLAVE="${KEK#*/}"
# ⭐ El almacén de la copia (0027 P1): un bucket POR INQUILINO, y el nombre es
#   una función de la celda, como el prefijo de sus secretos. `gen-inquilino.py`
#   y `--cotejar` lo derivan igual: si esto cambiara sin aquello, un Job
#   escribiría en un bucket que la plantilla no nombra.
COPIA="$PROYECTO-$NS-copia"

# ── La forja y la identidad ante ore-iam, ANTES de ②: la retirada (abajo) las
#   necesita y no debe pasar por ② ni ③, que crearian lo que va a borrar. ──────
# ⭐ Un fichero de un repositorio, y SOLO ese (0025 E6): `plataforma/enganches`
#   lleva un fichero por celda, y `empujar` (⑥) borra todo lo que hay antes de
#   copiar — que es lo correcto para un compartimento y lo contrario aqui.
#   Con `<origen>` vacio, se QUITA. Devuelve 0 si empujo, 3 si ya estaba.
empujar_fichero() { # <repositorio> <origen o vacio> <nombre en el repo> <que es>
  local REPO="$1" DE="$2" F="$3" QUE="$4" URL R
  export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraHeader GIT_CONFIG_VALUE_0="Authorization: token $F_ADMIN"
  if [ -n "${DENTRO:-}" ]; then URL="$F_URL/$REPO.git"
  else URL="http://localhost:$F_PUERTO/$REPO.git"; fi
  ( set -e
    rm -rf "$TMP/clon-f"
    cd "$TMP"
    git clone -q "$URL" clon-f 2>/dev/null \
      || { mkdir -p clon-f && cd clon-f && git init -q -b main \
           && git remote add origin "$URL" && cd ..; }
    cd clon-f
    if [ -n "$DE" ]; then cp "$DE" "./$F"; else rm -f "./$F"; fi
    git add -A
    if git diff --cached --quiet; then exit 3; fi
    git -c user.name=aprovisionador -c user.email=aprovisionador@invalido \
      commit -q -m "$QUE de la celda $NOMBRE"
    git push -q -u origin HEAD:main )
  R=$?
  case $R in
    0) hecho "$QUE: $F empujado a $REPO"; return 0 ;;
    3) ya "$F en $REPO"; return 3 ;;
    *) echo "  ⚠ no se pudo empujar $F a $REPO"; return 1 ;;
  esac
}
PROPIETARIO="${ARBOL%%/*}"
REPO="${ARBOL#*/}"

# ⭐ DENTRO sale del almacen, puesto por el contenedor de inicio en un tmpfs.
#   FUERA lo pone quien corre esto. En los dos casos NO viaja por `argv`.
[ -n "${DENTRO:-}" ] && [ -f /puesto/forja-admin ] && FORJA_ADMIN="$(cat /puesto/forja-admin)"
# ⭐ Y la URL del receptor, por lo mismo: su CAMINO es el secreto, asi que no
#   se escribe aqui. Dentro sale del almacen; fuera, del entorno.
[ -n "${DENTRO:-}" ] && [ -f /puesto/receptor-url ] && RECEPTOR="$(cat /puesto/receptor-url)"
: "${RECEPTOR:=}"

# ── ⭐⭐ Y SU PROPIA IDENTIDAD ANTE `ore-iam` (0025 E5) ────────────────────
#
# El cliente `ore-aprovisionador` del realm (68-…): un sujeto de maquina con el
# claim `rubix_tipo=aprovisionador`, que `ore-iam` admite en DOS verbos y en
# ninguno mas — registrar el agente de una celda (⑦) y darla por aprovisionada
# (al final). Con esto el paso 0 del ⑨ («el agente lo registra un Job de
# operador») deja de existir: el reconciliador registra lo que acaba de crear,
# con su huella. Su secreto sale del almacen (dentro) o del entorno (fuera);
# sin el, los dos verbos se saltan y se dice.
[ -n "${DENTRO:-}" ] && [ -f /puesto/aprovisionador-secreto ] && APROV_SECRETO="$(cat /puesto/aprovisionador-secreto)"
: "${APROV_SECRETO:=}"
REALM="${REALM:-rubix}"
if [ -n "${DENTRO:-}" ]; then
  IAM_BASE="http://ore-iam.identidad.svc.cluster.local:8090"
  IDP_TOKEN="http://idp-service.identidad.svc.cluster.local:8080/realms/$REALM/protocol/openid-connect/token"
else
  IAM_BASE="http://localhost:3132"
  IDP_TOKEN="https://login.paladio.io/realms/$REALM/protocol/openid-connect/token"
fi
iam_token() { # → el testigo del aprovisionador, a $TMP/iam (nunca a una variable exportada)
  [ -s "$TMP/iam" ] && return 0
  curl -sSf -X POST "$IDP_TOKEN" -d grant_type=client_credentials -d client_id=ore-aprovisionador \
    --data-urlencode "client_secret=$APROV_SECRETO" 2>/dev/null \
    | "$PY" -c 'import json,sys;print(json.load(sys.stdin)["access_token"])' > "$TMP/iam" 2>/dev/null \
    && [ -s "$TMP/iam" ]
}
iam_verbo() { # <metodo> <camino> [cuerpo] → cuerpo de la respuesta; el codigo, en $TMP/iam-cod
  # ⛔ El codigo va a un FICHERO y no a una variable: quien llama hace
  #   `R=$(iam_verbo …)`, que es una subshell, y una variable puesta ahi no
  #   vuelve. La primera pasada murio con «IAM_COD: unbound variable».
  local m="$1" c="$2" d="${3:-}"
  curl -sS -o "$TMP/iam-r.json" -w '%{http_code}' -X "$m" -H "Authorization: Bearer $(cat "$TMP/iam")" \
    -H 'Content-Type: application/json' ${d:+--data "$d"} "$IAM_BASE$c" 2>/dev/null > "$TMP/iam-cod"
  cat "$TMP/iam-r.json" 2>/dev/null
}
iam_cod() { cat "$TMP/iam-cod" 2>/dev/null; }
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
      "$F_URL/api/v1$c" 2>/dev/null
  else
    kubectl exec -n "$F_NS" forja-0 -- curl -sS -u "$u:$p" \
      -H 'Content-Type: application/json' --data "$d" \
      "http://localhost:3000/api/v1$c" 2>/dev/null
  fi | tr -d '\r' | sed -n 's/.*"sha1":"\([^"]*\)".*/\1/p'
}

forja_api() { # <metodo> <camino> [cuerpo] — imprime el codigo, o corta el guion
  local m="$1" c="$2" d="${3:-}" cod
  # ⭐ DENTRO se habla con la forja por su `Service`; FUERA hay que entrar en su
  #   pod, porque no tiene puerta al mundo. Es la misma llamada por dos caminos,
  #   y el de dentro no necesita ni una credencial de cluster.
  #   Y a CUAL forja lo dice `en_la_forja`.
  if [ -n "${DENTRO:-}" ]; then
    cod=$(curl -sS -o /dev/null -w '%{http_code}' \
      -X "$m" -H "Authorization: token $F_ADMIN" -H 'Content-Type: application/json' \
      ${d:+--data "$d"} "$F_URL/api/v1$c" 2>/dev/null | tr -d '\r')
  else
    cod=$(kubectl exec -n "$F_NS" forja-0 -- curl -sS -o /dev/null -w '%{http_code}' \
      -X "$m" -H "Authorization: token $F_ADMIN" -H 'Content-Type: application/json' \
      ${d:+--data "$d"} "http://localhost:3000/api/v1$c" 2>/dev/null | tr -d '\r')
  fi
  case "$cod" in
    2??|409|422) echo "$cod" ;;
    *) falla "la forja ($F_NS) contesto '$cod' a $m $c" ;;
  esac
}
forja_json() { # <camino> — el cuerpo de un GET, o vacio
  if [ -n "${DENTRO:-}" ]; then
    curl -sS -H "Authorization: token $F_ADMIN" "$F_URL/api/v1$1" 2>/dev/null
  else
    kubectl exec -n "$F_NS" forja-0 -- curl -sS \
      -H "Authorization: token $F_ADMIN" "http://localhost:3000/api/v1$1" 2>/dev/null
  fi | tr -d '\r'
}

# ══════════════════════════════════════════════════════════════════════════
# ⭐⭐ UNA CELDA RETIRADA SE DESMONTA (0025 E6) — y no pasa por lo demás
# ══════════════════════════════════════════════════════════════════════════
#
# La fila dice `retirada` (`POST /celdas/{c}/retirar`, con potestad y huella) y
# este reconciliador hace lo simetrico de todo lo de abajo, en el orden que
# `desaprovisionar-inquilino.sh` argumenta: primero lo que se OBEDECE —el
# enganche fuera, y Flux poda el namespace entero, forja y datos incluidos—,
# luego la puerta, el almacen, los permisos sobre la llave, las cuentas, y el
# compartimento en la forja central. La llave de la ORGANIZACION se queda: es
# de la cuenta, no de la celda. La fila se queda: es historia, y el nombre no
# se reusa (es unico en `iam.celda`).
#
# ⛔ Idempotente y sin cortar: cada paso dice «ya» si no queda nada, asi que
#   una pasada a medias se termina en la siguiente. Y `demo`/`prueba` no pueden
#   llegar aqui: son las celdas de casa, y `retirar` las niega.
if [ "$ESTADO" = "retirada" ]; then
  paso "⓪ RETIRADA — la celda \`$NOMBRE\` de \`$ORG\` se desmonta"
  # el enganche fuera → Flux poda t-$NOMBRE entero
  if [ -n "$SECO" ]; then
    haria "quitar $NOMBRE.yaml de plataforma/enganches, para que Flux pode t-$NOMBRE"
  else
    en_la_forja central
    empujar_fichero "plataforma/enganches" "" "$NOMBRE.yaml" "Retirado el enganche" || true
  fi
  # la puerta
  ENTRADA_RET=$(celda entrada)
  ZONA=""
  while IFS=, read -r z dn; do
    case "$ENTRADA_RET." in *".$dn") ZONA="$z" ;; esac
  done < <("$GCLOUD" dns managed-zones list --format="csv[no-heading](name,dnsName)" 2>/dev/null | tr -d '\r')
  if [ -n "$ZONA" ] && "$GCLOUD" dns record-sets describe "$ENTRADA_RET." --zone="$ZONA" --type=CNAME --format="value(name)" >/dev/null 2>&1; then
    correr "$GCLOUD" dns record-sets delete "$ENTRADA_RET." --zone="$ZONA" --type=CNAME && hecho "registro $ENTRADA_RET fuera de la zona"
  else
    ya "el registro de $ENTRADA_RET"
  fi
  # el almacen: todo lo que lleva la celda delante
  for S in $("$GCLOUD" secrets list --filter="name~^projects/[0-9]+/secrets/$NS-" --format="value(name)" 2>/dev/null | tr -d '\r'); do
    correr "$GCLOUD" secrets delete "$S" --quiet && hecho "secreto $S borrado"
  done
  # la condicion del cofre sobre el proyecto, y los permisos sobre la llave
  correr "$GCLOUD" projects remove-iam-policy-binding "$PROYECTO" \
    --member="serviceAccount:ore-cofre-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
    --role=roles/secretmanager.admin --condition="expression=resource.name.startsWith(\"projects/$NUMERO/secrets/$NS-cofre-\"),title=cofre-$NOMBRE,description=el cofre de $NOMBRE solo bajo su prefijo" \
    --format=none && hecho "\`ore-cofre-$NOMBRE\` ya no administra nada en el almacen" || ya "la condicion del cofre"
  correr "$GCLOUD" kms keys remove-iam-policy-binding "$LLAVE" --location="$LUGAR" --keyring="$LLAVERO" \
    --role=roles/cloudkms.cryptoKeyEncrypterDecrypter \
    --member="serviceAccount:ore-cofre-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
    && hecho "\`ore-cofre-$NOMBRE\` ya no puede usar $KEK" || ya "el permiso del cofre sobre la llave"
  # la copia (0027 P1): el bucket del inquilino, con lo que tenga dentro. Es SU
  # sistema de registro (0018): retirar la celda es retirarlo, y se dice cuanto habia.
  if "$GCLOUD" storage buckets describe "gs://$COPIA" --format="value(name)" >/dev/null 2>&1; then
    N=$("$GCLOUD" storage ls "gs://$COPIA/**" 2>/dev/null | grep -c . || true)
    correr "$GCLOUD" storage rm -r "gs://$COPIA" && hecho "la copia gs://$COPIA borrada ($N objetos)"
  else
    ya "la copia gs://$COPIA"
  fi
  # las cuentas
  for c in "ore-cofre-$NOMBRE" "ore-serve-$NOMBRE" "ore-driver-$NOMBRE" "ore-forja-$NOMBRE" "ore-informador-$NOMBRE"; do
    if "$GCLOUD" iam service-accounts describe "$c@$PROYECTO.iam.gserviceaccount.com" --format="value(email)" >/dev/null 2>&1; then
      correr "$GCLOUD" iam service-accounts delete "$c@$PROYECTO.iam.gserviceaccount.com" --quiet && hecho "cuenta $c borrada"
    else
      ya "la cuenta $c"
    fi
  done
  # el compartimento en la forja central: el repositorio y la organizacion t-<n>
  if [ -n "$SECO" ]; then
    haria "borrar $PROPIETARIO/compartimento y la organizacion $PROPIETARIO de la forja central"
  else
    en_la_forja central
    if [ -n "$(forja_json "/orgs/$PROPIETARIO" | grep -o '"id"' | head -1)" ]; then
      # en subshell: `forja_api` corta el guion con un 404, y aqui un 404 es «ya no esta»
      ( forja_api DELETE "/repos/$PROPIETARIO/compartimento" ) >/dev/null 2>&1 || true
      ( forja_api DELETE "/orgs/$PROPIETARIO" ) >/dev/null 2>&1 || true
      hecho "la organizacion $PROPIETARIO fuera de la forja central"
    else
      ya "la organizacion $PROPIETARIO en la forja central"
    fi
  fi
  # el cliente del agente en el IdP se queda: un cliente sin secreto en el
  # almacen y sin Jobs que lo pidan no hace nada, y borrarlo exige el admin
  # del IdP en cada retirada. Queda dicho.
  echo
  echo "✓ \`$NOMBRE\` retirada${SECO:+ (en seco)}. El namespace t-$NOMBRE lo poda Flux al ver el enganche fuera."
  exit 0
fi

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
paso "③ LAS CUENTAS DE GOOGLE — una por papel y por inquilino, no una compartida"
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
# ⭐ Y la cuarta: la de su FORJA (0024 E3-(c)), que solo sabe hacer una cosa —
#   dejar el testigo de su admin en el almacen al fundarse.
cuenta "ore-forja-$NOMBRE"
# ⭐ La del informador (0026 E2): solo para que su init traiga el agente del
#   almacen. Lee dos secretos y nada mas.
cuenta "ore-informador-$NOMBRE"
enlace "ore-cofre-$NOMBRE" cofre
enlace "ore-serve-$NOMBRE" ore-serve
enlace "ore-driver-$NOMBRE" driver
enlace "ore-forja-$NOMBRE" forja
enlace "ore-informador-$NOMBRE" informador

# ── ⭐⭐ LA COPIA: UN BUCKET POR INQUILINO, Y DOS PAPELES (0027 P1 I2) ─────
#
# Hasta el 2026-09-17 el almacén de copias era UN bucket de R2 con UNA
# credencial de lectura y escritura en un `.env.local`: el hueco que el ADR
# 0015 dejó dicho («quien refresca y quien responde son el mismo») y, con dos
# inquilinos, dos árboles compartiendo espacio de nombres y llave. Aquí:
#
#   · un bucket por celda, en la región de la celda, acceso uniforme (sin ACLs
#     por objeto: solo IAM), y cifrado con la KEK DEL INQUILINO — la misma que
#     cifra su cofre. El agente de servicio de Cloud Storage tiene que poder
#     usarla, igual que el de Secret Manager arriba;
#   · `ore-driver-<n>` ESCRIBE (`objectAdmin`: sellar, y recoger lo superado);
#   · `ore-serve-<n>` LEE (`objectViewer`: la ficha de la copia, F5 mañana).
#     Esa es la separación que 0015 pedía, y sale gratis: son dos cuentas.
#
# ⛔ Sin clave estática de ningún tipo: la política de la organización lo
#   prohíbe (`iam.disableServiceAccountKeyCreation`, y las HMAC de la API S3 de
#   GCS cuentan). Los Jobs hablan con `ore-store-gcs` y el token del metadata
#   server, como ya hacen con Secret Manager.
AGENTE_GCS="service-$NUMERO@gs-project-accounts.iam.gserviceaccount.com"
if "$GCLOUD" storage buckets describe "gs://$COPIA" --format="value(name)" >/dev/null 2>&1; then
  ya "la copia gs://$COPIA"
else
  correr "$GCLOUD" kms keys add-iam-policy-binding "$LLAVE" --location="$LUGAR" \
    --keyring="$LLAVERO" --role=roles/cloudkms.cryptoKeyEncrypterDecrypter \
    --member="serviceAccount:$AGENTE_GCS" \
    && hecho "Cloud Storage puede cifrar con $KEK (CMEK)"
  correr "$GCLOUD" storage buckets create "gs://$COPIA" --location="$LUGAR" \
    --uniform-bucket-level-access --public-access-prevention \
    --default-encryption-key="projects/$PROYECTO/locations/$LUGAR/keyRings/$LLAVERO/cryptoKeys/$LLAVE" \
    && hecho "la copia gs://$COPIA, en $LUGAR, cifrada con $KEK"
fi
correr "$GCLOUD" storage buckets add-iam-policy-binding "gs://$COPIA" \
  --member="serviceAccount:ore-driver-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  --role=roles/storage.objectAdmin && hecho "\`ore-driver-$NOMBRE\` escribe en la copia"
correr "$GCLOUD" storage buckets add-iam-policy-binding "gs://$COPIA" \
  --member="serviceAccount:ore-serve-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  --role=roles/storage.objectViewer && hecho "\`ore-serve-$NOMBRE\` lee la copia, y no escribe"

# ── ⭐⭐ Y EL ALMACÉN PUEDE USARLA COMO CMEK ────────────────────────────────
#
# Desde la 0024-⑤ el material va al Secret Manager de la celda cifrado con ESTA
# llave — el almacén la aplica solo, en vez de cifrar el cofre a mano. Para eso
# el agente de servicio del Secret Manager tiene que poder cerrar y abrir con
# ella. Es una cuenta de Google, una por proyecto; si no existe todavía se crea
# (`services identity create`), y es de plataforma, no del inquilino.
AGENTE_ALMACEN="service-$NUMERO@gcp-sa-secretmanager.iam.gserviceaccount.com"
"$GCLOUD" beta services identity create --service=secretmanager.googleapis.com --project="$PROYECTO" >/dev/null 2>&1 || true
correr "$GCLOUD" kms keys add-iam-policy-binding "$LLAVE" --location="$LUGAR" \
  --keyring="$LLAVERO" --role=roles/cloudkms.cryptoKeyEncrypterDecrypter \
  --member="serviceAccount:$AGENTE_ALMACEN" \
  && hecho "el almacen puede cifrar con $KEK (CMEK)"
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

# ── ¿Esta ya la forja del inquilino? ─────────────────────────────────────────
#
# Su admin vive en el almacen desde que ella misma se funda (46). Sin el, los
# pasos del inquilino se saltan y se dice; con el, todo lo del inquilino va a
# su forja. `INQ` es la bandera.
FORJA_ADMIN_INQ="$("$GCLOUD" secrets versions access latest --secret="$NS-forja-admin" 2>/dev/null | tr -d '\r\n')" || true
INQ=""
if [ -n "$FORJA_ADMIN_INQ" ]; then
  if [ -n "${DENTRO:-}" ]; then
    curl -sf -o /dev/null "http://forja.$NS.svc.cluster.local:3000/api/healthz" 2>/dev/null && INQ=1
  else
    [ "$(kubectl -n "$NS" get statefulset forja -o jsonpath='{.status.readyReplicas}' 2>/dev/null)" = "1" ] && INQ=1
  fi
fi
en_la_forja central
if [ -n "$INQ" ]; then
  hecho "la forja del inquilino esta viva y su admin en el almacen: el arbol y la cola van a ELLA"
else
  echo "  ⚠ la forja del inquilino todavia no esta (o su admin no esta en el almacen)."
  echo "    Esta pasada la funda al empujar la 46; la siguiente pobla sus repositorios."
fi

TESTIGO=""
if [ -n "$SECO" ]; then
  haria "crear la organizacion $PROPIETARIO y el repositorio $ARBOL en la forja del inquilino"
  haria "crear el usuario serve-$NOMBRE, hacerlo colaborador con escritura, y acunar su testigo"
elif [ -z "$INQ" ]; then
  echo "  ~ sin forja del inquilino: la organizacion, el arbol y serve-$NOMBRE, en la pasada siguiente"
else
  en_la_forja inquilino
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
  en_la_forja central
fi

# ══════════════════════════════════════════════════════════════════════════
paso "⑤ EL ALMACÉN — el valor va aquí, y NO a un \`Secret\`"
# ══════════════════════════════════════════════════════════════════════════
#
# ⭐ Ésta es la escritura que hace que todo lo demás sea posible sin credenciales
#   de clúster. El manifiesto referencia `$NS-forja-token`; el contenedor de
#   inicio lo trae a un tmpfs; el valor no pasa por etcd en ningún momento.
# ⭐ Si el paso ④ acuñó un testigo NUEVO —la forja del inquilino recién
#   fundada, por ejemplo—, se guarda aunque el secreto ya exista: el que había
#   era de otra forja. Sin testigo nuevo, se deja como esta.
if [ -n "$SECO" ]; then
  haria "crear el secreto $NS-forja-token con el testigo del paso ④"
elif [ -z "$TESTIGO" ]; then
  if "$GCLOUD" secrets describe "$NS-forja-token" --format="value(name)" >/dev/null 2>&1; then
    ya "el secreto $NS-forja-token"
  else
    echo "  ⚠ no hay testigo que guardar y el secreto $NS-forja-token no existe: la pasada siguiente"
  fi
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

# ── ⭐⭐ Y LA BASE DEL COFRE, que hasta el 2026-09-13 era un `Secret` a mano ──
#
# `cofre-url` es un secreto de PLATAFORMA —la misma base para todos los
# cofres— y por eso no se crea aqui: existe una vez, y aqui solo se le da al
# cofre de ESTE inquilino permiso para leerlo. Medido en
# `medida-el-acoplamiento-del-inquilino.py`: el `Secret` lo habia puesto una
# mano antes de que este guion existiera, y un inquilino nuevo arrancaba con
# el cofre en `CrashLoop` hasta que alguien se acordara.
"$GCLOUD" secrets describe cofre-url --format="value(name)" >/dev/null 2>&1 \
  || falla "no existe el secreto de plataforma \`cofre-url\` en el almacen.
     Es UNO para todos los inquilinos y lo crea el operador una vez, desde la
     base que el cofre ya usa:
       gcloud secrets create cofre-url --replication-policy=user-managed --locations=$LUGAR
       printf 'postgres://cofre_app:...@idp-db.identidad.svc.cluster.local:5432/iam' \
         | gcloud secrets versions add cofre-url --data-file=-"
correr "$GCLOUD" secrets add-iam-policy-binding cofre-url \
  --member="serviceAccount:ore-cofre-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  --role=roles/secretmanager.secretAccessor \
  && hecho "\`ore-cofre-$NOMBRE\` puede leer la base del cofre"

# ── ⭐⭐ EL ADMIN DE SU FORJA, que ella misma acuña ──────────────────────────
#
# El secreto se crea VACIO aqui; la version la añade la forja del inquilino al
# fundarse (46, contenedor `guardar`) con su cuenta, que solo puede añadir a
# ESTE. Lo leen: este guion (para poblarla), y las copias (para copiarla).
if "$GCLOUD" secrets describe "$NS-forja-admin" --format="value(name)" >/dev/null 2>&1; then
  ya "el secreto $NS-forja-admin"
elif [ -n "$SECO" ]; then
  haria "crear el secreto $NS-forja-admin, vacio, para que la forja del inquilino deje ahi su admin"
else
  "$GCLOUD" secrets create "$NS-forja-admin" --replication-policy=user-managed --locations="$LUGAR" \
    --labels=proyecto=ore,inquilino="$NOMBRE" >/dev/null 2>&1 && hecho "secreto $NS-forja-admin, vacio: lo llena la forja al fundarse"
fi
correr "$GCLOUD" secrets add-iam-policy-binding "$NS-forja-admin" \
  --member="serviceAccount:ore-forja-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  --role=roles/secretmanager.secretVersionAdder \
  && hecho "\`ore-forja-$NOMBRE\` puede dejar ahi su admin, y no leer nada"
for QUIEN in ore-aprovisionador ore-copias; do
  correr "$GCLOUD" secrets add-iam-policy-binding "$NS-forja-admin" \
    --member="serviceAccount:$QUIEN@$PROYECTO.iam.gserviceaccount.com" \
    --role=roles/secretmanager.secretAccessor \
    && hecho "\`$QUIEN\` puede leerlo"
done

# ── ⭐⭐ Y SU PREFIJO EN EL ALMACÉN, y NADA MÁS ─────────────────────────────
#
# La 0024-⑤: el material de los secretos de este inquilino vive en el Secret
# Manager como `$NS-cofre-<nombre>`. El cofre los crea y los lee con su cuenta,
# y lo que le impide tocar los de otro es una CONDICIÓN IAM por prefijo:
#
#   roles/secretmanager.admin  si  resource.name.startsWith(".../secrets/$NS-cofre-")
#
# Medido desde dentro del pod en `medida-el-almacen-por-inquilino.py`: crea el
# suyo, NO crea con el prefijo de otro —el `create` también obedece—, NO lee el
# testigo de otro, NO lista el proyecto. Sin la condición, `admin` sería el
# proyecto entero: la concentración que la 0023 desmontó, con otro nombre.
# ⚠️ Esto es un `setIamPolicy` sobre el PROYECTO, y el papel del aprovisionador
#   lo tiene CONDICIONADO a tocar solo concesiones de `secretmanager.admin`
#   (`papel-del-aprovisionador.yaml`, «lo que se concentra»). Desde dentro
#   fallaba con «Policy modification failed» y nadie lo leia.
correr "$GCLOUD" projects add-iam-policy-binding "$PROYECTO" \
  --member="serviceAccount:ore-cofre-$NOMBRE@$PROYECTO.iam.gserviceaccount.com" \
  --role=roles/secretmanager.admin \
  --condition="expression=resource.name.startsWith(\"projects/$NUMERO/secrets/$NS-cofre-\"),title=cofre-$NOMBRE,description=el cofre de $NOMBRE solo bajo su prefijo" \
  --format=none \
  && hecho "\`ore-cofre-$NOMBRE\` administra \`$NS-cofre-*\` en el almacen — y ningun otro prefijo"

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
TRABAJO="$PROPIETARIO/trabajo"
if [ -n "$SECO" ]; then
  haria "crear $COMPARTIMENTO y hacer a \`flux\` colaborador de solo lectura"
else
  # ⛔ Y la ORGANIZACION `t-<celda>` en la forja CENTRAL, antes que el repositorio.
  #   Para `demo` y `prueba` existia de antes (la creo el ④ cuando el arbol
  #   vivia aqui) y nadie la creaba para una celda nueva: la primera celda
  #   pedida (0025 E6) murio con «404 a POST /orgs/t-prueba-dos/repos».
  hecho "organizacion $PROPIETARIO en la forja central · $(forja_api POST "/orgs" "{\"username\":\"$PROPIETARIO\"}")"
  hecho "compartimento $COMPARTIMENTO · $(forja_api POST "/orgs/$PROPIETARIO/repos" \
    "{\"name\":\"compartimento\",\"private\":true}")"
  # ⭐ Y el agente entra como COLABORADOR, uno a uno. Lo que ata a `flux` no es
  #   el ámbito de su token —`read:repository` a secas— sino de qué es
  #   colaborador. Es la misma figura que `serve-<inquilino>`, que dio 404 sobre
  #   el árbol ajeno y no 403.
  hecho "\`flux\` lo lee, y ningun otro · $(forja_api PUT \
    "/repos/$COMPARTIMENTO/collaborators/flux" '{"permission":"read"}')"

  # -- LA COLA DE TRABAJO, Y POR QUE ES OTRO REPOSITORIO -----------------
  #
  # El compartimento dice COMO ES el inquilino: su namespace, su servidor, su
  # cofre, su entrada. La cola dice QUE HAY QUE HACER: un Job de catalogo por
  # fuente pendiente. `gen-inquilino.py` ya emitia las dos categorias —siete
  # ficheros y uno— y vivian en el mismo sitio con el mismo escritor.
  #
  # Medido el 2026-09-10: el compartimento tiene 14 `NetworkPolicy`, 4
  # `ServiceAccount`, 2 `Deployment`, un `Role` y un `RoleBinding`. Darle
  # escritura a `ore-serve` para que encolara trabajo le daria ademas reescribir
  # su propio `Deployment` y a que cuenta corre — el gobernado escribiendo su
  # gobierno.
  #
  # => Dos repositorios, dos escritores: aqui manda el aprovisionador; alli
  #   escribe `serve-<inquilino>`, y solo alli.
  # ⭐ La cola, en la forja del INQUILINO, y PUBLICA dentro de ella: Flux la lee
  #   sin testigo (13-…), porque son Jobs sin secretos y quien llega al puerto
  #   lo dice la NetworkPolicy. Escribirla sigue exigiendo el de serve-<n>.
  if [ -n "$INQ" ]; then
    en_la_forja inquilino
    hecho "cola de trabajo $TRABAJO, en la forja del inquilino · $(forja_api POST "/orgs/$PROPIETARIO/repos" \
      "{\"name\":\"trabajo\",\"private\":false}")"
    hecho "\`serve-$NOMBRE\` escribe en ella, y en ningun otro · $(forja_api PUT \
      "/repos/$TRABAJO/collaborators/serve-$NOMBRE" '{"permission":"write"}')"
    en_la_forja central
  else
    echo "  ~ sin forja del inquilino: la cola, en la pasada siguiente"
  fi
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
# ── ⭐ LOS TUNELES Y LA MUDANZA, ANTES DE LEER EL ARBOL ──────────────────────
#
# El arbol se lee de la forja del inquilino para rendir los Jobs de catalogo
# (`crudo`, abajo). Si la mudanza fuera despues, la primera pasada leeria un
# repositorio VACIO y rendiria la cola sin Jobs — medido el 2026-09-14: paso.
# Asi que primero se muda, y luego se lee.
if [ -z "$SECO" ]; then
  export GIT_CONFIG_COUNT=1
  export GIT_CONFIG_KEY_0=http.extraheader
  # ⭐ DENTRO no hay tunel que abrir: la forja esta a un salto. Estas seis lineas
  #   son exactamente lo que el Job se ahorra, y por eso estan aisladas.
  TUNEL=""; TUNEL_INQ=""
  if [ -n "${DENTRO:-}" ]; then
    :   # la URL la compone `empujar`, que ahora sirve a dos forjas
  else
    kubectl port-forward -n "$FORJA_NS" svc/forja "3129:3000" >/dev/null 2>&1 &
    TUNEL=$!
    if [ -n "$INQ" ]; then
      kubectl port-forward -n "$NS" svc/forja "3131:3000" >/dev/null 2>&1 &
      TUNEL_INQ=$!
    fi
    trap 'kill "$TUNEL" "$TUNEL_INQ" 2>/dev/null; rm -rf "$TMP"' EXIT
    for _ in 1 2 3 4 5 6 7 8; do
      curl -sS -o /dev/null "http://localhost:3129/api/v1/version" 2>/dev/null \
        && { [ -z "$INQ" ] || curl -sS -o /dev/null "http://localhost:3131/api/v1/version" 2>/dev/null; } && break
      sleep 1
    done
  fi
  # ── ④b LA MUDANZA: el arbol y la cola, de la central a la del inquilino ──
  #
  # UNA vez: si el repositorio del inquilino esta vacio y el de la central no,
  # se lleva entero (`--mirror`). Despues, el de la central se queda como copia
  # muerta hasta que alguien lo borre a mano — no se borra nada aqui.
  if [ -n "$INQ" ]; then
    for R in "$REPO" trabajo; do
      # ⚠️ Y LA CARRERA CON LA SEMILLA, medida en `prueba` el 2026-09-14: Flux
      #   aplico el compartimento nuevo, el Job de la semilla (42) vio el arbol
      #   del inquilino VACIO y lo sembro, y la mudanza llego despues a un
      #   repositorio con UN commit que no era el de la central. Por eso «vacio»
      #   aqui incluye «solo tiene la semilla»: un commit en el inquilino y mas
      #   de uno en la central es la semilla ganando la carrera, y se pisa.
      cuenta_commits() { "$PY" -c 'import json,sys
try:
    d = json.load(sys.stdin); print(len(d) if isinstance(d, list) else 0)
except Exception:
    print(0)'; }
      en_la_forja inquilino
      COMMITS_INQ=$(forja_json "/repos/$PROPIETARIO/$R/commits?limit=3" | cuenta_commits)
      en_la_forja central
      COMMITS_CEN=$(forja_json "/repos/$PROPIETARIO/$R/commits?limit=3" | cuenta_commits)
      VACIO=no; [ "$COMMITS_INQ" = "0" ] && VACIO=si
      if [ "$COMMITS_INQ" = "1" ] && [ "$COMMITS_CEN" -gt 1 ]; then
        VACIO=si; echo "  ⚠ $PROPIETARIO/$R en el inquilino solo tiene la semilla y la central tiene historia: se pisa"
      fi
      LLENO=no; [ "$COMMITS_CEN" -gt 0 ] && LLENO=si
      if [ "$VACIO" = "si" ] && [ "$LLENO" = "si" ]; then
        ( set -e; cd "$TMP"; rm -rf espejo.git
          en_la_forja central
          if [ -n "${DENTRO:-}" ]; then DE="$F_URL/$PROPIETARIO/$R.git"; else DE="http://localhost:$F_PUERTO/$PROPIETARIO/$R.git"; fi
          GIT_CONFIG_VALUE_0="Authorization: token $F_ADMIN" git clone -q --mirror "$DE" espejo.git
          en_la_forja inquilino
          if [ -n "${DENTRO:-}" ]; then A="$F_URL/$PROPIETARIO/$R.git"; else A="http://localhost:$F_PUERTO/$PROPIETARIO/$R.git"; fi
          GIT_CONFIG_VALUE_0="Authorization: token $F_ADMIN" git -C espejo.git push -q --mirror --force "$A" ) \
          && hecho "mudado $PROPIETARIO/$R: de la forja de la plataforma a la del inquilino, entero" \
          || falla "no se pudo mudar $PROPIETARIO/$R"
      elif [ "$VACIO" = "no" ]; then
        ya "$PROPIETARIO/$R en la forja del inquilino"
      fi
    done
  fi

fi

FUENTES=""
if [ -z "$SECO" ]; then
  # El arbol se lee de la forja del inquilino si ya esta; si no, de la central,
  # que es donde estuvo hasta la mudanza (④b).
  [ -n "$INQ" ] && en_la_forja inquilino
  crudo() { # <camino dentro del repositorio>
    forja_json "/repos/$ARBOL/$1"
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
print(",".join(sorted(hechos)))
' 2>/dev/null)
  # Dos lineas: las fuentes SIN paquete (se rinde su catalogo) y los paquetes
  # que HAY (un catalogo encolado para una de estas ya termino: fuera).
  CATALOGADAS=$(printf '%s\n' "$FUENTES" | sed -n 2p)
  FUENTES=$(printf '%s\n' "$FUENTES" | sed -n 1p)
fi
[ -n "$FUENTES" ] && hecho "fuentes sin paquete: $FUENTES"

# ⭐ Y las vistas que declaran copia (0027 P1): `paquete.vista` de cada
#   `materialized` del arbol. Con alguna, se rinde el Job de la copia (45).
COPIAS=""
if [ -z "$SECO" ] && [ -n "$INQ" ]; then
  COPIAS=$(for P in $(crudo "contents/packages" | "$PY" -c 'import json,sys
try:
    print(" ".join(e["name"] for e in json.load(sys.stdin) if e.get("type") == "dir"))
except Exception:
    pass'); do
    for V in $(crudo "contents/packages/$P/views" | "$PY" -c 'import json,sys
try:
    print(" ".join(e["name"] for e in json.load(sys.stdin) if e.get("type") == "file" and e["name"].endswith(".yaml")))
except Exception:
    pass'); do
      crudo "raw/packages/$P/views/$V" | "$PY" -c 'import re,sys
t = sys.stdin.read()
if re.search(r"^\s+materialized:", t, re.M):
    m = re.search(r"^\s*name:\s*([A-Za-z0-9_]+)", t.split("metadata", 1)[1] if "metadata" in t else t, re.M)
    if m: print(sys.argv[1] + "." + m.group(1))' "$P"
    done
  done | sort -u | paste -sd, -)
fi
[ -n "$COPIAS" ] && hecho "vistas con copia: $COPIAS"

# ⭐ Se rinde POR CELDA, y la organizacion va aparte: es lo que `ore-serve` le
#   dice al custodio y lo que `ore init --name` graba en el arbol.
# ⛔ El enganche SIN la cola hasta que la forja de la celda viva (`INQ`): la cola
#   —`Role`/`RoleBinding` en `t-<celda>`— no puede aplicarse antes de que el
#   compartimento cree el namespace, y Flux no aplica nada si una pieza no pasa
#   el ensayo. Dos pasadas, como la forja y la cola de trabajo.
"$PY" "$(ruta "${GEN:-$RAIZ/malla/gen-inquilino.py}")" "$NOMBRE" --organizacion "$ORG" --arbol "$ARBOL" \
  ${FUENTES:+--fuentes "$FUENTES"} ${COPIAS:+--copias "$COPIAS"} --a "$(ruta "$TMP/rendido")" --enganche "$(ruta "$TMP/enganche")" \
  ${INQ:+} $([ -n "$INQ" ] || printf -- --sin-cola) \
  >/dev/null || falla "no se pudo renderizar"
hecho "renderizado: $(ls "$TMP/rendido" | tr '\n' ' ')"
# ⭐ Y el ENGANCHE (0025 E6): lo que dice que el compartimento se obedece. Para
#   `demo` y `prueba` esta a mano en `13-…` y el renderizador no lo emite; para
#   cualquier otra celda va a `plataforma/enganches`, que Flux obedece (15).
ENGANCHE=""
[ -f "$TMP/enganche/$NOMBRE.yaml" ] && ENGANCHE="$TMP/enganche/$NOMBRE.yaml"
[ -n "$ENGANCHE" ] && hecho "enganche rendido: $NOMBRE.yaml ($(grep -c '^kind:' "$ENGANCHE") objetos$([ -n "$INQ" ] || printf ', sin la cola hasta que la forja viva'))" || echo "  · el enganche de \`$NOMBRE\` esta a mano en 13-…"

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
  haria "bajar perfiles.json de $PERFILES_URL y empujarlo a la cola con la plantilla"
else
  # ⛔ El testigo por `GIT_CONFIG_*` y no dentro de la URL: un
  #   `http://usuario:token@host/…` deja la credencial en la linea de ordenes,
  #   que lee cualquier proceso de la maquina. Es lo mismo que hace
  #   `ore-serve/git.rs`, y por lo mismo.
  export GIT_CONFIG_COUNT=1
  export GIT_CONFIG_KEY_0=http.extraheader
  # -- ⭐⭐ DOS REPOSITORIOS, Y LA MISMA FUNCION PARA LOS DOS ---------------
  #
  # El compartimento dice COMO ES el inquilino; la cola dice QUE HAY QUE HACER.
  # `gen-inquilino.py` ya emitia las dos categorias —`PLANTILLAS` y `POR_FUENTE`,
  # siete ficheros y uno por fuente— y hasta hoy caian en el mismo sitio.
  #
  # => Se separan por el nombre del fichero, que es donde la categoria ya estaba
  #   escrita: lo que empieza por `44-` es trabajo; el resto, gobierno.
  # ⭐ Y a CUAL forja, lo dice el conmutador: `en_la_forja` antes de cada
  #   `empujar`. El testigo va con ella.
  empujar() {   # <repositorio> <directorio con lo rendido> <que es>
    local REPO="$1" DE="$2" QUE="$3" URL R
    export GIT_CONFIG_VALUE_0="Authorization: token $F_ADMIN"
    if [ -n "${DENTRO:-}" ]; then URL="$F_URL/$REPO.git"
    else URL="http://localhost:$F_PUERTO/$REPO.git"; fi
    ( set -e
      rm -rf "$TMP/clon"
      cd "$TMP"
      git clone -q "$URL" clon 2>/dev/null \
        || { mkdir -p clon && cd clon && git init -q -b main \
             && git remote add origin "$URL" && cd ..; }
      # ⛔ Se borra lo que hubiera y se copia lo rendido: la plantilla es la
      #   verdad. Un fichero que el renderizador ya no emite tiene que
      #   DESAPARECER — si se quedase, Flux seguiria obedeciendolo.
      find clon -maxdepth 1 -name '*.yaml' -delete
      find clon -maxdepth 1 -name '*.txt' -delete
      cp "$DE"/* clon/ 2>/dev/null || true
      # ⭐⭐ Y LA COLA ES ADITIVA con lo que `ore-serve` encola (medido en
      #   `victor` el 2026-09-17, con segundos: el alta encolo el catalogo a
      #   las 16:13:38, empujo el arbol a las :40, y esta pasada, que leyo
      #   el arbol ENTRE los dos, reescribio la cola sin el a las :43 — y lo
      #   borro). Un Job que esta pasada no rinde y la cola ya tiene se
      #   conserva mientras su trabajo siga pendiente: un `44-*` cuya fuente
      #   no tiene paquete todavia, y el `48-la-copia.yaml` si esta pasada no
      #   rindio otro. Lo que si se retira es un catalogo cuya fuente YA tiene
      #   paquete: ese Job termino, y Flux no debe volver a crearlo.
      if [ "$REPO" = "$TRABAJO" ]; then
        for f in $(cd clon && git ls-tree --name-only HEAD 2>/dev/null | grep -E '^(44-.*|48-la-copia)\.yaml$'); do
          [ -e "clon/$f" ] && continue
          case "$f" in
            44-*)
              FTE=$(cd clon && git show "HEAD:$f" 2>/dev/null | sed -n 's/.*name: FUENTE, value: "\([^"]*\)".*/\1/p' | head -1)
              case ",${CATALOGADAS:-}," in *",$FTE,"*) continue ;; esac ;;
          esac
          ( cd clon && git show "HEAD:$f" > "$f" ) && echo "  · conservado $f (encolado por ore-serve, y sigue pendiente)"
        done
      fi
      cd clon
      git add -A
      if git diff --cached --quiet; then cd "$TMP"; exit 3; fi
      git -c user.name=aprovisionador -c user.email=aprovisionador\invalido \
        commit -q -m "$QUE del inquilino $NOMBRE"
      git push -q -u origin HEAD:main )
    R=$?
    case $R in
      0) hecho "empujado a $REPO" ;;
      3) ya "los manifiestos de $REPO" ;;
      *) falla "no se pudo empujar a $REPO" ;;
    esac
  }

  # El corte, por el nombre. ⚠️ Y la cola puede quedar VACIA —un inquilino sin
  #   fuentes pendientes— y eso es legitimo: `empujar` lo dice con «ya estaba».
  mkdir -p "$TMP/gobierno" "$TMP/cola"
  for f in "$TMP"/rendido/*; do
    case "$(basename "$f")" in
      # Los Jobs de las fuentes pendientes, y la PLANTILLA con la que
      # `ore-serve` encola las que vengan. La plantilla es `.txt` a proposito:
      # viaja en la cola y `kustomize` solo aplica los `.yaml` de ahi.
      44-*|48-la-copia.yaml|plantilla-catalogo.txt|plantilla-copia.txt) cp "$f" "$TMP/cola/" ;;
      *)                           cp "$f" "$TMP/gobierno/" ;;
    esac
  done

  en_la_forja central
  empujar "$COMPARTIMENTO" "$TMP/gobierno" "El compartimento"
  # `plataforma/enganches`: la organizacion y el repositorio, una vez, y en
  # CADA pasada (idempotente, tres llamadas): asi el `GitRepository` de la 15
  # tiene rama que leer desde antes de la primera celda pedida. Es de
  # PLATAFORMA —ningun inquilino escribe ahi— y por eso no lleva el aviso ni un
  # usuario propio: lo escribe este guion y nadie mas.
  hecho "organizacion plataforma · $(forja_api POST "/orgs" '{"username":"plataforma"}')"
  hecho "repositorio plataforma/enganches · $(forja_api POST "/orgs/plataforma/repos" '{"name":"enganches","private":true}')"
  hecho "\`flux\` lo lee · $(forja_api PUT "/repos/plataforma/enganches/collaborators/flux" '{"permission":"read"}')"
  printf '%s\n' "# Los enganches de las celdas pedidas (0025 E6)" "" \
    "Un fichero por celda, \`<celda>.yaml\`, rendido por el aprovisionador de \`malla/13-…\`." \
    "Lo obedece la \`Kustomization\` \`enganches\` (\`malla/15-…\`) con \`prune: true\`:" \
    "quitar el fichero es retirar la celda. Nadie escribe aqui a mano." > "$TMP/README.md"
  empujar_fichero "plataforma/enganches" "$TMP/README.md" "README.md" "El README" || true
  if [ -n "$ENGANCHE" ]; then
    empujar_fichero "plataforma/enganches" "$ENGANCHE" "$NOMBRE.yaml" "El enganche" || true
  fi

  # ⭐ LA LISTA DE PERFILES (0027 ⑦, E1): lo que Bastion mide y publica —cada
  #   perfil maquina × modelo con sus numeros—, en la cola al lado de la
  #   plantilla del catalogo. `ore-serve` la lee en `POST /modelos` y rechaza
  #   (422) un perfil que no este: un perfil sin numero no existe. Se BAJA,
  #   no se rinde: es un hecho del sustrato, y quien lo publica es quien lo
  #   mide. Si no se puede bajar, la celda se aprovisiona igual y se dice: sin
  #   lista, `POST /modelos` contesta 503 hasta la pasada siguiente.
  # (por redireccion y no con `-o`: el curl de mingw no sabe escribir en la ruta
  #  de `mktemp` desde dentro de este guion —«(23) client returned ERROR on write»—)
  if curl -sSf -m 20 "$PERFILES_URL" > "$TMP/cola/perfiles.json" 2>"$TMP/perfiles.err"; then
    hecho "perfiles.json · $("$PY" -c 'import json,sys;d=json.load(open(sys.argv[1]));print("%d perfiles · %s" % (len(d["profiles"]), d["generated"]))' "$(ruta "$TMP/cola/perfiles.json")" 2>/dev/null || echo "bajado")"
  else
    echo "  ⚠ sin perfiles.json: no se pudo bajar $PERFILES_URL ($(head -c 200 "$TMP/perfiles.err" | tr '
' ' ')). La celda no podra aceptar un Model hasta la pasada siguiente"
  fi

  if [ -n "$INQ" ]; then
    en_la_forja inquilino
    empujar "$TRABAJO"       "$TMP/cola"     "La cola de trabajo"
    en_la_forja central
  else
    echo "  ~ sin forja del inquilino: la cola se empuja en la pasada siguiente"
  fi

  [ -n "$TUNEL" ] && { kill "$TUNEL" "$TUNEL_INQ" 2>/dev/null; trap 'rm -rf "$TMP"' EXIT; }
  unset GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0
fi

# ══════════════════════════════════════════════════════════════════════════
paso "⑦ EL AGENTE — un cliente de Keycloak por inquilino, no uno copiado a mano"
# ══════════════════════════════════════════════════════════════════════════
# ── ⛔⛔ LO QUE HABIA, MEDIDO ──────────────────────────────────────────────
#
# `ore-agente` era UN cliente de Keycloak para todos los inquilinos, creado a
# mano con `kcadm` el 8 de septiembre, y su secreto vivia en un `Secret`
# `idp-agente` puesto con `kubectl` en `t-demo` y COPIADO a mano a `t-prueba`.
# La 024 lo dejo dicho —«un sujeto para todos, a un grant de todas»— y
# `medida-el-acoplamiento-del-inquilino.py` lo saco como uno de los cuatro
# acoplamientos sin respuesta. Un inquilino nuevo arrancaba con sus Jobs de
# catalogo muriendo con 401 hasta que alguien copiara el `Secret`.
#
# ── LO QUE HACE ESTE PASO ─────────────────────────────────────────────────
#
#   1. un cliente `ore-agente-<n>` en el realm, con LA MISMA receta que el
#      original —cuenta de servicio, sin flujos, testigo de 300 s, audiencia
#      `ore-serve`, claim `rubix_tipo=agente`—, medida por la API de admin
#   2. su secreto al almacen como `t-<n>-agente-secreto` (y el clientId como
#      `t-<n>-agente-cliente`, al lado, para que el init los traiga juntos)
#   3. `ore-driver-<n>` puede leerlos, y nadie mas
#   4. y el `sub` de su cuenta de servicio, que es lo que `ore-iam agente`
#      necesita — se imprime en ⑨, porque registrarlo es un `insert` en `iam`
#      y este papel no escribe ahi (la 023)
#
# ── LA CREDENCIAL DE ADMIN, por el entorno, como `FORJA_ADMIN` ────────────
#
# `IDP_ADMIN_USER` / `IDP_ADMIN_PASS`. Dentro del cluster, de `/puesto/idp-admin`
# si el Job lo trae; y si no hay ninguna, este paso NO se salta en silencio:
# lo dice y sigue, porque un inquilino que ya existe no puede dejar de
# converger por un ajuste que falta en el aprovisionador.
if [ -n "${DENTRO:-}" ] && [ -f /puesto/idp-admin ]; then
  IDP_ADMIN_USER="${IDP_ADMIN_USER:-admin}"
  IDP_ADMIN_PASS="$(cat /puesto/idp-admin)"
fi
AGENTE="ore-agente-$NOMBRE"
AGENTE_SUB=""
REGISTRADO=""
if [ -z "${IDP_ADMIN_PASS:-}" ] && [ -z "$SECO" ]; then
  echo "  ⚠ sin \`IDP_ADMIN_PASS\`: no se crea el agente \`$AGENTE\`. Sin el, los Jobs de"
  echo "    catalogo de \`$NOMBRE\` no tienen con que pedir un testigo. Se pasa por el entorno."
else
  # ⭐ DENTRO se habla con el IdP por su `Service`; FUERA por un tunel, porque
  #   la API de admin no tiene por que estar en la puerta publica. Es la misma
  #   figura que la forja.
  if [ -n "${DENTRO:-}" ]; then
    IDP_BASE="http://idp-service.identidad.svc.cluster.local:8080"
  else
    IDP_BASE="http://localhost:3130"
    if [ -z "$SECO" ]; then
      kubectl port-forward -n identidad svc/idp-service 3130:8080 >/dev/null 2>&1 &
      TUNEL_IDP=$!
      trap 'kill "$TUNEL_IDP" 2>/dev/null; [ -n "${TUNEL:-}" ] && kill "$TUNEL" 2>/dev/null; rm -rf "$TMP"' EXIT
      for _ in 1 2 3 4 5 6 7 8; do
        curl -sS -o /dev/null "$IDP_BASE/realms/master" 2>/dev/null && break
        sleep 1
      done
    fi
  fi
  kc() { # <metodo> <camino> [cuerpo] — contra la API de admin, con el testigo en $TMP/kc
    local m="$1" c="$2" d="${3:-}"
    curl -sS -X "$m" -H "Authorization: Bearer $(cat "$TMP/kc")" -H 'Content-Type: application/json' \
      ${d:+--data "$d"} "$IDP_BASE/admin/realms/$REALM$c" 2>/dev/null
  }
  if [ -n "$SECO" ]; then
    haria "crear el cliente \`$AGENTE\` en \`$REALM\` y guardar su secreto en el almacen"
  else
    # El testigo de admin, a un fichero de $TMP y nunca a una variable que se
    # exporte: `curl` lo lee de ahi en cada llamada.
    curl -sSf -X POST "$IDP_BASE/realms/master/protocol/openid-connect/token" \
      -d grant_type=password -d client_id=admin-cli \
      -d "username=$IDP_ADMIN_USER" --data-urlencode "password=$IDP_ADMIN_PASS" 2>/dev/null \
      | "$PY" -c 'import json,sys;print(json.load(sys.stdin)["access_token"])' > "$TMP/kc" \
      || falla "el IdP no dio testigo de admin: usuario o clave incorrectos, o el tunel no abrio"

    # 1 · el cliente, idempotente por su `clientId`.
    ID=$(kc GET "/clients?clientId=$AGENTE" | "$PY" -c 'import json,sys;l=json.load(sys.stdin);print(l[0]["id"] if l else "")')
    if [ -n "$ID" ]; then
      ya "el cliente \`$AGENTE\`"
    else
      COD=$(curl -sS -o /dev/null -w '%{http_code}' -X POST -H "Authorization: Bearer $(cat "$TMP/kc")" \
        -H 'Content-Type: application/json' "$IDP_BASE/admin/realms/$REALM/clients" --data @- <<JSON
{"clientId":"$AGENTE","name":"agente de $NOMBRE","description":"El sujeto de maquina de la celda $NOMBRE, de la organizacion $ORG: los Jobs de catalogo piden como el. Lo crea el aprovisionador.",
 "enabled":true,"protocol":"openid-connect","publicClient":false,"serviceAccountsEnabled":true,
 "standardFlowEnabled":false,"implicitFlowEnabled":false,"directAccessGrantsEnabled":false,
 "attributes":{"access.token.lifespan":"300"},
 "protocolMappers":[
   {"name":"audiencia-ore-serve","protocol":"openid-connect","protocolMapper":"oidc-audience-mapper",
    "config":{"included.client.audience":"ore-serve","access.token.claim":"true","id.token.claim":"false"}},
   {"name":"rubix-tipo-agente","protocol":"openid-connect","protocolMapper":"oidc-hardcoded-claim-mapper",
    "config":{"claim.name":"rubix_tipo","claim.value":"agente","jsonType.label":"String","access.token.claim":"true","id.token.claim":"false"}}
 ]}
JSON
)
      [ "$COD" = "201" ] || falla "el IdP contesto $COD al crear \`$AGENTE\`"
      ID=$(kc GET "/clients?clientId=$AGENTE" | "$PY" -c 'import json,sys;print(json.load(sys.stdin)[0]["id"])')
      hecho "cliente \`$AGENTE\` creado, con la receta de \`ore-agente\`"
    fi

    # 1b · LOS DOS CLAIMS DEL GATEWAY DE MODELOS (0027 ②, E2), y CONVERGEN: un
    #     cliente creado antes de hoy no los tiene, y añadirlos aqui —y no solo
    #     en la receta de arriba— es lo que hace que `demo`, `prueba` y `victor`
    #     los ganen en la pasada siguiente sin que nadie toque el realm.
    #
    #   `modelos`      la AUDIENCIA del gateway. Como `included.custom.audience`
    #                  y no como cliente: el gateway no es un cliente del realm
    #                  —verifica con el JWKS de fichero, no inicia sesion de
    #                  nadie— y un cliente-audiencia mas seria un secreto mas
    #                  que nadie usa. La cadena `modelos` en `aud` es todo lo
    #                  que el gateway comprueba (`--oidc-audience modelos`).
    #   `rubix_celda`  LA CELDA, dicha por el realm: un cliente por celda, y el
    #                  claim es su nombre. Hasta hoy el gateway la deducia de
    #                  `azp = ore-agente-<celda>`; con el claim, `azp` deja de
    #                  ser un contrato.
    TIENE=$(kc GET "/clients/$ID/protocol-mappers/models" | "$PY" -c 'import json,sys;print(" ".join(m["name"] for m in json.load(sys.stdin)))')
    mapeador() { # <nombre> <json>
      case " $TIENE " in
        *" $1 "*) ya "el mapeador \`$1\` de \`$AGENTE\`" ;;
        *) COD=$(curl -sS -o /dev/null -w '%{http_code}' -X POST -H "Authorization: Bearer $(cat "$TMP/kc")"              -H 'Content-Type: application/json' "$IDP_BASE/admin/realms/$REALM/clients/$ID/protocol-mappers/models" --data "$2")
           [ "$COD" = "201" ] && hecho "mapeador \`$1\` en \`$AGENTE\`" || echo "  ⚠ el IdP contesto $COD al mapeador \`$1\`" ;;
      esac
    }
    mapeador audiencia-modelos '{"name":"audiencia-modelos","protocol":"openid-connect","protocolMapper":"oidc-audience-mapper","config":{"included.custom.audience":"modelos","access.token.claim":"true","id.token.claim":"false"}}'
    mapeador rubix-celda "{\"name\":\"rubix-celda\",\"protocol\":\"openid-connect\",\"protocolMapper\":\"oidc-hardcoded-claim-mapper\",\"config\":{\"claim.name\":\"rubix_celda\",\"claim.value\":\"$NOMBRE\",\"jsonType.label\":\"String\",\"access.token.claim\":\"true\",\"id.token.claim\":\"false\"}}"

    # 4 · el `sub`: la cuenta de servicio del cliente.
    AGENTE_SUB=$(kc GET "/clients/$ID/service-account-user" | "$PY" -c 'import json,sys;print(json.load(sys.stdin)["id"])')
    [ -n "$AGENTE_SUB" ] || falla "el cliente no tiene cuenta de servicio"

    # 2 · el secreto, al almacen. Se compara con lo que hay antes de anadir una
    #     version: anadir una igual en cada convergencia seria deriva con forma
    #     de historial.
    kc GET "/clients/$ID/client-secret" | "$PY" -c 'import json,sys;sys.stdout.write(json.load(sys.stdin)["value"])' > "$TMP/agente-secreto"
    printf '%s' "$AGENTE" > "$TMP/agente-cliente"
    for parte in cliente secreto; do
      S="$NS-agente-$parte"
      if ! "$GCLOUD" secrets describe "$S" --format="value(name)" >/dev/null 2>&1; then
        "$GCLOUD" secrets create "$S" --replication-policy=user-managed --locations="$LUGAR" \
          --labels=proyecto=ore,inquilino="$NOMBRE" >/dev/null 2>&1 || true
      fi
      # ⛔⛔ SIN LEER EL VALOR. Esto comparaba leyendo la version `latest`, y
      #   desde dentro eso NO PUEDE funcionar: el papel del aprovisionador no
      #   tiene `versions.access` a proposito («crea y concede, no usa»). La
      #   lectura fallaba en silencio, «no es igual» salia siempre, y cada
      #   pasada añadia una version identica: 34 del mismo `ore-agente-demo`
      #   antes de que nadie mirara.
      #
      # ⇒ Se compara por HUELLA: el sha256 del valor va en una anotacion del
      #   secreto (metadato, que si puede leer), y solo se añade version cuando
      #   la huella cambia. El valor sigue sin salir del almacen hacia aqui.
      HUELLA=$("$PY" -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$TMP/agente-$parte")
      if [ "$("$GCLOUD" secrets describe "$S" --format='value(annotations.sha256)' 2>/dev/null | tr -d '\r')" = "$HUELLA" ]; then
        ya "el almacen tiene el $parte del agente"
      else
        "$GCLOUD" secrets versions add "$S" --data-file="$(ruta "$TMP/agente-$parte")" >/dev/null \
          && "$GCLOUD" secrets update "$S" --update-annotations="sha256=$HUELLA" >/dev/null 2>&1 \
          && hecho "$parte del agente guardado en el almacen, y NO en un \`Secret\`"
      fi
      # 3 · quien lo lee: el driver de ESTE inquilino.
      # Los Jobs (driver) y el informador (0026 E2) piden como el agente: los dos leen.
      for QUIEN in "ore-driver-$NOMBRE" "ore-informador-$NOMBRE"; do
        correr "$GCLOUD" secrets add-iam-policy-binding "$S" \
          --member="serviceAccount:$QUIEN@$PROYECTO.iam.gserviceaccount.com" \
          --role=roles/secretmanager.secretAccessor \
          && hecho "\`$QUIEN\` puede leer el $parte"
      done
    done
    rm -f "$TMP/kc" "$TMP/agente-secreto" "$TMP/actual"
    hecho "agente \`$AGENTE\` · sub $AGENTE_SUB"

    # 5 · ⭐ Y EN `iam`, POR EL VERBO (0025 E5). Idempotente: la segunda vez dice
    #     «ya» y no deja huella; la primera registra con la huella del
    #     aprovisionador y hereda `usar` sobre los secretos que la organizacion
    #     ya tenga. Es el mismo nucleo que `ore-iam agente`.
    if [ -z "$APROV_SECRETO" ]; then
      echo "  ⚠ sin \`APROV_SECRETO\`: el agente NO se registra en iam. Queda el Job de operador (⑨ 0)."
    elif [ -z "${DENTRO:-}" ] && ! curl -sS -o /dev/null "$IAM_BASE/salud" 2>/dev/null; then
      echo "  ⚠ \`ore-iam\` no se alcanza en $IAM_BASE (fuera hace falta un tunel: kubectl port-forward -n identidad svc/ore-iam 3132:8090)."
    elif ! iam_token; then
      echo "  ⚠ el IdP no dio testigo a \`ore-aprovisionador\`: el agente NO se registra en iam."
    else
      R=$(iam_verbo POST "/organizaciones/$ORG/agentes" "{\"sub\":\"$AGENTE_SUB\",\"nombre\":\"$AGENTE\"}")
      if [ "$(iam_cod)" = "200" ]; then
        REGISTRADO=1
        if printf '%s' "$R" | grep -q '"ya": *true'; then
          ya "el agente en iam"
        else
          hecho "agente registrado en iam: $R"
        fi
      else
        echo "  ⚠ ore-iam contesto $(iam_cod) al registrar el agente: $R"
      fi
    fi
  fi
fi

# ══════════════════════════════════════════════════════════════════════════
paso "⑧ LA PUERTA — la entrada resuelve a la celda, o se dice qué registro falta"
# ══════════════════════════════════════════════════════════════════════════
#
# La 0024-⑥, medida antes (`medida-la-puerta-de-la-celda.py`): cada celda tiene
# su puerta y **el plano de control escribe el DNS**. La fila dice la RELACION
# —`iam.organizacion.entrada` debe resolver a `iam.celda.puerta`— y este paso
# converge el mundo hacia ella:
#
#   · si ya resuelve igual, no hay nada que hacer (hoy: el comodin
#     `*.ore.paladio.io` manda toda entrada a la IP de `ore-mesh`, y con UNA
#     celda eso ES la relacion)
#   · si la zona es de este proyecto (Cloud DNS), se escribe el `CNAME`
#   · si no —hoy `ore.paladio.io` vive en el registrador—, se dice el registro
#     exacto que falta. Es la frase incomoda de la 022: el alta pasa a ser una
#     espera, y quien mueve ficha no es este guion.
#
# ⚠️ Y la trampa del comodin, dicha: la puerta de una celda NUEVA resuelve por
#   el comodin aunque no tenga registro propio, y todo parece converger. Por eso
#   se avisa cuando la celda no es la compartida y su puerta resuelve a la misma
#   IP que cualquier nombre inventado.
#
# ⛔ Se lee por la vista `iam.celda_de` (027): por NOMBRES, sin `id`, que es lo
#   que el papel de la 023 puede ver. La funcion `celda` esta definida en ①.
resuelve() { # <host> → IPs v4 ordenadas, separadas por coma; vacio si no resuelve
  "$PY" -c 'import socket,sys
try: print(",".join(sorted({a[4][0] for a in socket.getaddrinfo(sys.argv[1], 443, socket.AF_INET)})))
except OSError: pass' "$1" 2>/dev/null | tr -d '\r'
}
ENTRADA=$(celda entrada)
PUERTA=$(celda puerta)
TIER=$(celda tier)
if [ -z "$PUERTA" ]; then
  echo "  ⚠ \`$NOMBRE\` no tiene celda, o la base es anterior a la 027: no hay puerta que cotejar"
else
  IP_ENTRADA=$(resuelve "$ENTRADA")
  IP_PUERTA=$(resuelve "$PUERTA")
  IP_COMODIN=$(resuelve "aprovisionador-$$.ore.paladio.io")
  if [ -z "$IP_PUERTA" ]; then
    echo "  ⚠ la puerta \`$PUERTA\` no resuelve. Eso no lo arregla este guion: es el registro A"
    echo "    del balanceador de la celda, y va antes que cualquier CNAME de inquilino"
  else
    # ⭐ Si la zona es de este proyecto, la entrada tiene SU registro, aunque el
    #   comodin ya la resolviera bien: la relacion entrada→puerta queda escrita
    #   donde se lee, y no depende de que `*.ore.paladio.io` siga apuntando a
    #   esta celda. Es lo que la E6 de la 0025 promete: «su puerta en el DNS,
    #   escrita por el aprovisionador».
    ZONA=""
    while IFS=, read -r z dn; do
      case "$ENTRADA." in *".$dn") ZONA="$z" ;; esac
    done < <("$GCLOUD" dns managed-zones list --format="csv[no-heading](name,dnsName)" 2>/dev/null | tr -d '\r')
    if [ -n "$ZONA" ]; then
      if "$GCLOUD" dns record-sets describe "$ENTRADA." --zone="$ZONA" --type=CNAME --format="value(rrdatas)" 2>/dev/null | grep -q .; then
        ya "el registro \`$ENTRADA CNAME $PUERTA\` en la zona \`$ZONA\`"
      else
        correr "$GCLOUD" dns record-sets create "$ENTRADA." --zone="$ZONA" --type=CNAME --ttl=300 --rrdatas="$PUERTA." \
          && hecho "escrito en la zona \`$ZONA\`: $ENTRADA CNAME $PUERTA"
      fi
    elif [ "$IP_ENTRADA" = "$IP_PUERTA" ]; then
      ya "\`$ENTRADA\` → $IP_ENTRADA, que es \`$PUERTA\` (la zona no es nuestra: resuelve por el registrador)"
      [ "$TIER" != "compartido" ] && [ "$IP_PUERTA" = "$IP_COMODIN" ] \
        && echo "  ⚠ pero \`$PUERTA\` resuelve por el COMODIN, no por un registro propio: una celda $TIER necesita su A"
    else
      echo "  ⚠ \`$ENTRADA\` → ${IP_ENTRADA:-nada} y su celda \`$PUERTA\` está en $IP_PUERTA. La zona no es de"
      echo "    este proyecto: el registro lo pone una persona en el registrador, y hasta entonces"
      echo "    la consola dirá que la entrada no lleva a la celda:"
      echo
      echo "      $ENTRADA.   CNAME   $PUERTA."
    fi
  fi
fi

# ══════════════════════════════════════════════════════════════════════════
paso "⑨ LA CELDA, APROVISIONADA — y lo que este script sigue sin hacer"
# ══════════════════════════════════════════════════════════════════════════
#
# ⭐ El `status` del patron de operador (0025 E5): la pasada acabo ENTERA sobre
#   esta celda —su forja viva y poblada, su agente registrado— y el
#   reconciliador lo dice a `iam`, con su identidad. Nulo en la fila significa
#   «ninguna pasada ha acabado todavia», y la consola lo pinta Provisioning.
#   No se dice en seco, ni a medias: una celda «aprovisionada» a la que le
#   falta el agente es una mentira con fecha.
if [ -n "$SECO" ]; then
  haria "dar la celda \`$NOMBRE\` por aprovisionada en iam"
elif [ -z "$INQ" ] || [ -z "$REGISTRADO" ]; then
  echo "  ~ la celda no se da por aprovisionada: $([ -z "$INQ" ] && printf 'su forja no estaba ' ; [ -z "$REGISTRADO" ] && printf 'su agente no se registro')"
elif iam_token; then
  R=$(iam_verbo POST "/celdas/$NOMBRE/aprovisionada")
  [ "$(iam_cod)" = "200" ] && hecho "celda \`$NOMBRE\` aprovisionada: $R" || echo "  ⚠ ore-iam contesto $(iam_cod): $R"
fi
rm -f "$TMP/iam" "$TMP/iam-r.json"

cat <<FIN

  $([ -n "$REGISTRADO" ] && printf '✓' || printf '⛔') 0 · EL AGENTE EN \`iam\`. $([ -n "$REGISTRADO" ] && printf 'Registrado por el verbo, con la huella de\n       este aprovisionador (0025 E5). El Job de operador ya no hace falta.' || printf 'Este guion creo el cliente de Keycloak y guardo su\n       secreto, y esta vez NO pudo registrarlo por el verbo (arriba dice por que).\n       Mientras tanto, el Job de operador:\n\n         kubectl -n identidad create job agente-%s --image=<ore-iam:main> -- \\\n           ore-iam agente --organizacion %s \\\n             --emisor https://login.paladio.io/realms/%s \\\n             --sub %s --nombre %s' "$NOMBRE" "$ORG" "$REALM" "${AGENTE_SUB:-<el sub que imprime el paso ⑦>}" "$AGENTE")

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

  ⛔ 3 · EL ARBOL. \`ore init --name $ORG\`, primer commit y push. Es la E5,
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
