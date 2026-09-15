#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# EL INFORMADOR DE LA CELDA (0026 E2) — mide B y lo empuja al plano de control
#
# Cada `INTERVALO` segundos: tres GET al API server con el token de la
# ServiceAccount del pod (un Role de solo lectura en SU namespace), un snapshot
# de la 0026-② con `jq`, y un POST a `ore-iam` con el token del AGENTE de la
# celda (client_credentials; el cliente y el secreto los dejo el init en
# `/agente`, nunca en una variable de entorno).
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
: "${INTERVALO:=60}" "${AGENTE:=/agente}"
SA=/var/run/secrets/kubernetes.io/serviceaccount
NS=$(cat "$SA/namespace")
API=https://kubernetes.default.svc
TOKEN=""
VENCE=0

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

# El snapshot de la 0026-②, por stdout.
medir() {
  api "/api/v1/namespaces/$NS/resourcequotas" /tmp/q.json || return 1
  api "/apis/batch/v1/namespaces/$NS/jobs" /tmp/j.json || return 1
  api "/api/v1/namespaces/$NS/pods?labelSelector=ore.dev%2Frol%3Dcontrol" /tmp/p.json || return 1
  jq -n -c --slurpfile q /tmp/q.json --slurpfile j /tmp/j.json --slurpfile p /tmp/p.json --arg t "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '
    ($q[0]) as $q | ($j[0]) as $j | ($p[0]) as $p
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
                            fin: $u.status.completionTime } end) },
        control: (if $c == null then { listo: false, desde: null, reinicios: 0 } else
                   { listo: ((($c.status.containerStatuses // [{}])[0].ready) // false),
                     desde: $c.status.startTime,
                     reinicios: ((($c.status.containerStatuses // [{}])[0].restartCount) // 0) } end) }'
}

echo "informador de \`$CELDA\` · cada ${INTERVALO}s · $NS → $IAM_URL"
while :; do
  if S=$(medir) && [ -n "$S" ]; then
    if token; then
      C=$(curl -sS -m 10 -o /tmp/r -w '%{http_code}' -X POST "$IAM_URL/celdas/$CELDA/estado" \
            -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$S") || C=000
      if [ "$C" = "200" ]; then
        echo "· $(date -u +%H:%M:%S) informado"
      else
        echo "✗ ore-iam contesto $C: $(head -c 200 /tmp/r 2>/dev/null)"
        VENCE=0
      fi
    fi
  else
    echo "✗ no se pudo medir (¿Role? ¿API server?)"
  fi
  sleep "$INTERVALO"
done
