#!/usr/bin/env bash
# Lo que comparten las pruebas de P4 que hablan con `ore-postgres` COMO UNA CELDA (ADR 0058): un pod de
# usar y tirar en el namespace de la celda, con la cuenta de su `ore-serve` (`ore.dev/rol: control`),
# que pide su token de Workload Identity con audiencia `ore-postgres` al servidor de metadatos. El
# token no sale del pod. Se carga con `source` después de entorno.sh y con PRUEBA puesta (p41, p422…).
#
#   abrir_celdas demo victor       la NetworkPolicy de salida de la prueba (se borra al salir)
#   en CELDA 'pide MÉTODO CAMINO [CUERPO]' …    una línea `CÓDIGO CUERPO` por paso
#   hasta_hecha CAMINO             (dentro) sondea una operación hasta `hecha` (5 min: una VM puede esperar a un nodo nuevo, ~3,5 min)
#   espera CÓDIGO "qué" LÍNEA      ✓/✗ y cuenta los fallos en $fallos
#   campo LÍNEA clave…             un campo del cuerpo JSON de una línea
: "${PRUEBA:?pon PRUEBA antes de cargar celdas.sh}"
fallos=${fallos:-0}
IMAGEN=$ORE_PG_REGISTRO/ore-drivers:main
URL=http://ore-postgres.ore-pg.svc.cluster.local.:8100

salida() {   # la NetworkPolicy de la prueba, en el namespace de una celda
  cat <<EOF
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: prueba-$PRUEBA, namespace: t-$1 }
spec:
  podSelector: { matchLabels: { ore.dev/prueba: $PRUEBA } }
  policyTypes: [Egress]
  egress:
    - to: [{ namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: kube-system } } }]
      ports: [{ protocol: UDP, port: 53 }, { protocol: TCP, port: 53 }]
    - to:
        - namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: ore-pg } }
          podSelector: { matchLabels: { ore.dev/rol: plano-postgres } }
      ports: [{ protocol: TCP, port: 8100 }]
    - to: [{ ipBlock: { cidr: 169.254.169.254/32 } }, { ipBlock: { cidr: 169.254.169.252/32 } }]
EOF
}
abrir_celdas() {   # abrir_celdas CELDA… → su NetworkPolicy de prueba, que se borra al salir
  CELDAS_ABIERTAS="$*"
  for c in "$@"; do salida "$c" | kubectl apply -f - >/dev/null; done
  trap 'for c in $CELDAS_ABIERTAS; do kubectl -n t-$c delete networkpolicy prueba-$PRUEBA --ignore-not-found >/dev/null; done' EXIT
}

# Lo que corre dentro: `pide MÉTODO CAMINO [CUERPO]` con el token de la celda (audiencia ore-postgres),
# `pide_con VAR …` con otro (TI: audiencia ore-iam; TN: ninguno; TB: basura). Cada línea sale como
# `CÓDIGO CUERPO`.
DENTRO='
md=http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/identity
T=$(curl -sf -H "Metadata-Flavor: Google" "$md?audience=ore-postgres&format=full") || { echo "000 sin token"; exit 0; }
TI=$(curl -sf -H "Metadata-Flavor: Google" "$md?audience=ore-iam&format=full")
TB="$(echo "$T" | cut -d. -f1-2).AAAA"
pide_con() { v=$1; shift; m=$1; c=$2; d=${3:-}; t=$(eval echo \$$v)
  h=""; [ -n "$t" ] && h="Authorization: Bearer $t"
  f=$(mktemp)   # uno por petición: dos a la vez en el mismo pod no se pisan (P4·3·4)
  code=$(curl -s -o "$f" -w "%{http_code}" -X "$m" ${h:+-H "$h"} ${d:+-H "Content-Type: application/json" --data "$d"} "$URL$c")
  echo "$code $(tr -d "\n" < "$f")"; rm -f "$f"; }
pide() { pide_con T "$@"; }
hasta_hecha() { for i in $(seq 1 300); do l=$(pide GET "$1"); case "$l" in *\"hecha\":true*) break;; esac; sleep 1; done; echo "$l"; }
TN=""
'

en() {   # en CELDA PASOS… → una línea por paso
  local c=$1; shift
  local pasos; pasos=$(printf '%s\n' "$@")
  kubectl -n "t-$c" run "$PRUEBA-$c-$RANDOM" --rm -i --restart=Never --quiet --pod-running-timeout=5m --image="$IMAGEN" \
    --overrides="$(python -c 'import json,sys; print(json.dumps({"spec":{
      "serviceAccountName":"ore-serve","nodeSelector":{"ore.dev/pool":"system"},"terminationGracePeriodSeconds":0,
      "automountServiceAccountToken":False,
      "securityContext":{"runAsNonRoot":True,"runAsUser":1000,"seccompProfile":{"type":"RuntimeDefault"}},
      "containers":[{"name":"p","image":sys.argv[1],"command":["sh","-c",sys.argv[2]],
        "env":[{"name":"URL","value":sys.argv[3]}],
        "resources":{"requests":{"cpu":"10m","memory":"32Mi"},"limits":{"cpu":"200m","memory":"64Mi"}},
        "securityContext":{"allowPrivilegeEscalation":False,"capabilities":{"drop":["ALL"]}}}]}}))' \
      "$IMAGEN" "$DENTRO$pasos" "$URL")" \
    --labels="ore.dev/rol=control,ore.dev/prueba=$PRUEBA" 2>&1
}

espera() {   # espera N "qué" línea → comprueba el código
  local n=$1 que=$2 l=$3
  if [ "${l%% *}" = "$n" ]; then echo "  ✓ $que · $n"; else echo "  ✗ $que · esperaba $n, llegó: ${l:0:300}"; fallos=$((fallos+1)); fi
}
campo() { python -c 'import json,sys; d=json.loads(sys.argv[1].split(" ",1)[1]); [d:=d[k] for k in sys.argv[2:]]; print(d)' "$1" "${@:2}" 2>/dev/null; }
