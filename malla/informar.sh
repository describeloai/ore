#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# EL INFORMADOR DE LA CELDA (0026 E2) — mide B y lo empuja al plano de control
#
# Tres GET al API server con el token de la ServiceAccount del pod (un Role de
# solo lectura en SU namespace), un snapshot de la 0026-② con `jq`, y un POST a
# `ore-iam` con el token del AGENTE de la celda (client_credentials; el cliente
# y el secreto los dejo el init en `/agente`, nunca en una variable de entorno).
#
# ⭐⭐ Y EN TIEMPO REAL (2026-09-17, Data › Jobs): la pantalla tiene que decir
#   lo que pasa mientras pasa, y la latencia estaba aqui —un tick por minuto—.
#   Ahora:
#     · un `watch` sobre los Jobs del namespace (una conexion larga: el API
#       server EMPUJA cada cambio en < 1 s) despierta el bucle en el acto;
#     · mientras algun Job corre, se mide cada `INTERVALO_ACTIVO` s (3) y el
#       snapshot lleva las ultimas lineas del log del contenedor que trabaja,
#       que es lo que mueve la tabla de tablas fila a fila;
#     · en reposo, el latido de `INTERVALO` s (60), como siempre.
#   La consola sigue sin tocar el API server: pregunta a `ore-iam`.
#
# ⛔ `set -u` y NO `set -e`: cada paso decide si sigue. Un informador que muere
#   por una respuesta rara deja de informar justo cuando mas falta. Lo que
#   falla se dice en el log con `✗`, y se vuelve a intentar al siguiente tick.
#
# ⭐ La referencia de este snapshot es `pruebas-de-fuego/medida-el-estado-de-la-
#   celda.py --snapshot`: mismas claves, mismos valores, desde fuera con kubectl.
#   Si esto y aquello dejan de coincidir, uno de los dos miente.
# ═══════════════════════════════════════════════════════════════════════════
set -u
: "${CELDA:?falta CELDA}" "${IAM_URL:?falta IAM_URL}" "${IDP_TOKEN_URL:?falta IDP_TOKEN_URL}"
: "${INTERVALO:=60}" "${INTERVALO_ACTIVO:=3}" "${AGENTE:=/agente}"
SA=/var/run/secrets/kubernetes.io/serviceaccount
NS=$(cat "$SA/namespace")
API=https://kubernetes.default.svc
TOKEN=""
VENCE=0
EVENTO=/tmp/evento

# El token del agente: se renueva cuando le queda menos de un minuto (vida 300 s).
token() {
  ahora=$(date +%s)
  [ $((VENCE - ahora)) -ge 60 ] && return 0
  R=$(curl -sS -m 10 -X POST "$IDP_TOKEN_URL" \
        -d grant_type=client_credentials \
        -d "client_id=$(cat "$AGENTE/cliente")" \
        --data-urlencode "client_secret=$(cat "$AGENTE/secreto")") || { echo "✗ el IdP no contesta"; return 1; }
  TOKEN=$(printf '%s' "$R" | jq -r '.access_token // empty')
  [ -n "$TOKEN" ] || { echo "✗ el IdP no dio token: $(printf '%s' "$R" | head -c 200)"; return 1; }
  VENCE=$((ahora + $(printf '%s' "$R" | jq -r '.expires_in // 300')))
}

# Una lectura del API server a un FICHERO, con el token de la ServiceAccount (el
# kubelet lo rota: se relee). ⛔ A fichero y no a una variable: la lista de Jobs
# de una celda con muchas fuentes no cabe en la linea de ordenes de `jq`
# («Argument list too long», medido en `demo` el 2026-09-15).
api() {
  curl -sS -m 10 --cacert "$SA/ca.crt" -H "Authorization: Bearer $(cat "$SA/token")" -o "$2" "$API$1"
}

# ── El watch: cada cambio de un Job del namespace deja una marca ────────────
#
# Una conexion larga al API server; se reabre cada 5 minutos (el token de la
# ServiceAccount rota y `curl -m` la cierra). Solo deja una marca: quien mide es
# el bucle de abajo, que la ve en un segundo. Si el Role no tiene `watch`, el
# API server contesta 403 en una linea y el bucle sigue midiendo a su ritmo.
vigilar() {
  while :; do
    curl -sS -N -m 300 --cacert "$SA/ca.crt" -H "Authorization: Bearer $(cat "$SA/token")" \
      "$API/apis/batch/v1/namespaces/$NS/jobs?watch=true" 2>/dev/null \
      | while IFS= read -r linea; do
          case "$linea" in *'"type":"'*) : > "$EVENTO" ;; esac
        done
    sleep 2
  done
}

# ── Los logs de un Job: las ultimas lineas del contenedor que trabaja ───────
#
# El contenedor es el ULTIMO de la plantilla (`catalogar`, `copiar`: los init
# no cuentan); el pod, el mas reciente con `job-name=<n>`. A fichero, como todo.
log_de() { # <job> <contenedor> <lineas> → /tmp/log.txt
  api "/api/v1/namespaces/$NS/pods?labelSelector=job-name%3D$1" /tmp/pods.json || return 1
  POD=$(jq -r '(.items // []) | sort_by(.metadata.creationTimestamp) | last | .metadata.name // empty' /tmp/pods.json)
  [ -n "$POD" ] || { : > /tmp/log.txt; return 0; }
  api "/api/v1/namespaces/$NS/pods/$POD/log?container=$2&tailLines=$3" /tmp/log.txt || : > /tmp/log.txt
  # un pod que aun no arranco contesta un JSON de error, no un log
  if head -c 1 /tmp/log.txt | grep -q '{'; then : > /tmp/log.txt; fi
}

# El snapshot de la 0026-②, por stdout. Con la LISTA de Jobs (los 20 ultimos) y,
# para los que corren o fallaron hace poco, el log que mueve la pantalla.
medir() {
  api "/api/v1/namespaces/$NS/resourcequotas" /tmp/q.json || return 1
  api "/apis/batch/v1/namespaces/$NS/jobs" /tmp/j.json || return 1
  api "/api/v1/namespaces/$NS/pods?labelSelector=ore.dev%2Frol%3Dcontrol" /tmp/p.json || return 1

  # La lista, y a quien pedirle el log: los que corren (40 lineas) y los
  # fallidos de las ultimas 2 h (12 lineas, el motivo). Nunca mas de 6 logs.
  jq -c --arg ahora "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '
    def estado: if (.status.active // 0) > 0 then "corriendo"
                elif (.status.succeeded // 0) > 0 then "ok"
                elif (.status.failed // 0) > 0 then "fallido" else "pendiente" end;
    def tipo: if (.metadata.name | startswith("catalogo-")) then "catalogo"
              elif (.metadata.name | startswith("copiar-")) then "copia"
              elif (.metadata.name | startswith("invocar-")) then "invocacion" else "otro" end;
    def sujeto: [(.spec.template.spec.containers // [])[].env // [] | .[] | select(.name == "VISTAS" or .name == "FUENTE" or .name == "FUNCION") | .value] | first // "";
    def contenedor: ((.spec.template.spec.containers // []) | last | .name // "");
    def reciente: ((.status.completionTime // ([.status.conditions[]? | select(.type == "Failed") | .lastTransitionTime] | first) // .status.startTime // "") as $t | ($t != "") and (($ahora | sub("Z$"; "") | strptime("%Y-%m-%dT%H:%M:%S") | mktime) - ($t | sub("Z$"; "") | strptime("%Y-%m-%dT%H:%M:%S") | mktime) < 7200));
    (.items // []) | sort_by(.metadata.creationTimestamp) | reverse | .[:20]
    | map({ nombre: .metadata.name, tipo: tipo, sujeto: sujeto, estado: estado,
            inicio: .status.startTime,
            # Un Job fallido no tiene completionTime: su fin es cuando paso a Failed.
            fin: (.status.completionTime // ([.status.conditions[]? | select(.type == "Failed") | .lastTransitionTime] | first)),
            contenedor: contenedor,
            quiere_log: ((estado == "corriendo") or (estado == "fallido" and reciente)) })' /tmp/j.json > /tmp/lista.json || return 1

  # Los logs, uno a uno, a un objeto {nombre: texto}
  printf '{}' > /tmp/logs.json
  n=0
  jq -r '.[] | select(.quiere_log) | "\(.nombre) \(.contenedor) \(.estado)"' /tmp/lista.json | while read -r J C E; do
    [ $n -ge 6 ] && break
    n=$((n + 1))
    [ "$E" = "corriendo" ] && L=40 || L=12
    log_de "$J" "$C" "$L" || continue
    jq -c --arg j "$J" --rawfile t /tmp/log.txt '. + {($j): ($t | .[-3000:])}' /tmp/logs.json > /tmp/logs2.json && mv /tmp/logs2.json /tmp/logs.json
  done

  jq -n -c --slurpfile q /tmp/q.json --slurpfile j /tmp/j.json --slurpfile p /tmp/p.json \
        --slurpfile l /tmp/lista.json --slurpfile g /tmp/logs.json --arg t "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '
    ($q[0]) as $q | ($j[0]) as $j | ($p[0]) as $p | ($l[0]) as $lista | ($g[0]) as $logs
    | (($q.items // [])[0].status // {}) as $s
    | def par(k): [($s.used[k] // "0"), ($s.hard[k] // "0")];
      def ent(k): [(($s.used[k] // "0") | tonumber), (($s.hard[k] // "0") | tonumber)];
      (($j.items // []) | sort_by(.status.startTime // "") | reverse | .[0]) as $u
    | (($p.items // [])[0]) as $c
    | { v: 1,
        medido_en: $t,
        cuota: { cpu: par("requests.cpu"), memoria: par("requests.memory"), jobs: ent("count/jobs.batch") },
        jobs: { activos:  ([($j.items // [])[].status.active    // 0] | add // 0),
                ok:       ([($j.items // [])[].status.succeeded // 0] | add // 0),
                fallidos: ([($j.items // [])[].status.failed    // 0] | add // 0),
                ultimo: (if $u == null then null else
                          { nombre: $u.metadata.name,
                            estado: (if ($u.status.active // 0) > 0 then "activo"
                                     elif ($u.status.failed // 0) > 0 then "fallido" else "ok" end),
                            inicio: $u.status.startTime,
                            fin: $u.status.completionTime } end),
                lista: ($lista | map(del(.quiere_log, .contenedor) + (if $logs[.nombre] then { log: $logs[.nombre] } else {} end))) },
        control: (if $c == null then { listo: false, desde: null, reinicios: 0 } else
                   { listo: ((($c.status.containerStatuses // [{}])[0].ready) // false),
                     desde: $c.status.startTime,
                     reinicios: ((($c.status.containerStatuses // [{}])[0].restartCount) // 0) } end) }'
}

informar() {
  if S=$(medir) && [ -n "$S" ]; then
    if token; then
      C=$(curl -sS -m 10 -o /tmp/r -w '%{http_code}' -X POST "$IAM_URL/celdas/$CELDA/estado" \
            -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$S") || C=000
      if [ "$C" = "200" ]; then
        ACTIVOS=$(printf '%s' "$S" | jq -r '.jobs.activos // 0')
        echo "· $(date -u +%H:%M:%S) informado · $(printf '%s' "$S" | wc -c | tr -d ' ') B · $ACTIVOS activo(s)"
      else
        echo "✗ ore-iam contesto $C: $(head -c 200 /tmp/r 2>/dev/null)"
        VENCE=0
      fi
    fi
  else
    echo "✗ no se pudo medir (¿Role? ¿API server?)"
  fi
}

echo "informador de \`$CELDA\` · latido ${INTERVALO}s · con Jobs corriendo ${INTERVALO_ACTIVO}s · watch · $NS → $IAM_URL"
vigilar &
ACTIVOS=0
while :; do
  rm -f "$EVENTO"
  informar
  # Espera: corta si algo corre; larga si no; y en cualquier caso, un evento
  # del watch la corta en un segundo.
  [ "${ACTIVOS:-0}" -gt 0 ] && ESPERA=$INTERVALO_ACTIVO || ESPERA=$INTERVALO
  i=0
  while [ $i -lt "$ESPERA" ]; do
    [ -e "$EVENTO" ] && break
    sleep 1
    i=$((i + 1))
  done
done
