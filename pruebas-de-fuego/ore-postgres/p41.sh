#!/usr/bin/env bash
# P4·1 · el contrato y el esqueleto (ADR 0058). Hecho cuando: una celda crea y lee un proyecto VACÍO;
# otra celda no lo ve.
#
# Con dos celdas DE VERDAD, cada una con su token: un pod de usar y tirar en su namespace con la
# cuenta de su `ore-serve` (`ore.dev/rol: control`), que pide al servidor de metadatos su token de
# Workload Identity con audiencia `ore-postgres` y llama a `ore-postgres`. El token no sale del pod.
#
#   p41.sh [celda-a] [celda-b]       (por defecto demo y victor: dos organizaciones distintas)
#
# Lo que el pod necesita para salir de su celda hacia `ore-pg` (la salida de `ore-serve` hacia el
# producto es P4·6) va en una NetworkPolicy de la prueba, sólo para sus pods, y se borra al acabar.
export ORE_PG_COMPUTO=pod
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}; B=${2:-victor}
P="p41-$(date +%s | tail -c 6)"
IMAGEN=$ORE_PG_REGISTRO/ore-drivers:main
URL=http://ore-postgres.ore-pg.svc.cluster.local.:8100
fallos=0

salida() {   # la NetworkPolicy de la prueba, en el namespace de una celda
  cat <<EOF
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: prueba-p41, namespace: t-$1 }
spec:
  podSelector: { matchLabels: { ore.dev/prueba: p41 } }
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
for c in "$A" "$B"; do salida "$c" | kubectl apply -f - >/dev/null; done
trap 'for c in "$A" "$B"; do kubectl -n t-$c delete networkpolicy prueba-p41 --ignore-not-found >/dev/null; done' EXIT

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
  code=$(curl -s -o /tmp/r -w "%{http_code}" -X "$m" ${h:+-H "$h"} ${d:+-H "Content-Type: application/json" --data "$d"} "$URL$c")
  echo "$code $(tr -d "\n" < /tmp/r)"; }
pide() { pide_con T "$@"; }
TN=""
'

en() {   # en CELDA PASOS… → una línea por paso
  local c=$1; shift
  local pasos; pasos=$(printf '%s\n' "$@")
  kubectl -n "t-$c" run "p41-$c-$RANDOM" --rm -i --restart=Never --quiet --image="$IMAGEN" \
    --overrides="$(python -c 'import json,sys; print(json.dumps({"spec":{
      "serviceAccountName":"ore-serve","nodeSelector":{"ore.dev/pool":"system"},
      "automountServiceAccountToken":False,
      "securityContext":{"runAsNonRoot":True,"runAsUser":1000,"seccompProfile":{"type":"RuntimeDefault"}},
      "containers":[{"name":"p","image":sys.argv[1],"command":["sh","-c",sys.argv[2]],
        "env":[{"name":"URL","value":sys.argv[3]}],
        "resources":{"requests":{"cpu":"10m","memory":"32Mi"},"limits":{"cpu":"200m","memory":"64Mi"}},
        "securityContext":{"allowPrivilegeEscalation":False,"capabilities":{"drop":["ALL"]}}}]}}))' \
      "$IMAGEN" "$DENTRO$pasos" "$URL")" \
    --labels="ore.dev/rol=control,ore.dev/prueba=p41" 2>&1
}

espera() {   # espera N "qué" línea → comprueba el código
  local n=$1 que=$2 l=$3
  if [ "${l%% *}" = "$n" ]; then echo "  ✓ $que · $n"; else echo "  ✗ $que · esperaba $n, llegó: ${l:0:300}"; fallos=$((fallos+1)); fi
}
campo() { python -c 'import json,sys; d=json.loads(sys.argv[1].split(" ",1)[1]); [d:=d[k] for k in sys.argv[2:]]; print(d)' "$1" "${@:2}" 2>/dev/null; }

echo "── $A crea el proyecto $P y lo lee"
mapfile -t R < <(en "$A" \
  "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'" \
  "pide GET /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/proyectos" \
  "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'" \
  "pide POST /v1/postgres/proyectos '{\"id\":\"Mal_Id\"}'")
espera 202 "crear" "${R[0]}"
OP=$(campo "${R[0]}" operacion id); HECHA=$(campo "${R[0]}" operacion hecha); CELDA=$(campo "${R[0]}" proyecto celda)
[ "$HECHA" = True ] && echo "  ✓ la operación $OP nace hecha (vacío: nada que esperar)" || { echo "  ✗ operación: ${R[0]}"; fallos=$((fallos+1)); }
espera 200 "leerlo" "${R[1]}"
case "${R[2]}" in *"\"$P\""*) echo "  ✓ está en su lista";; *) echo "  ✗ no está en su lista: ${R[2]}"; fallos=$((fallos+1));; esac
espera 409 "crearlo otra vez no crea dos" "${R[3]}"
espera 400 "un id que no vale" "${R[4]}"
echo "    (celda $CELDA)"

echo "── $B (otra organización) no lo ve"
mapfile -t R < <(en "$B" \
  "pide GET /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/proyectos" \
  "pide DELETE /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/operaciones/$OP")
espera 404 "leerlo" "${R[0]}"
case "${R[1]}" in 200*) case "${R[1]}" in *"\"$P\""*) echo "  ✗ está en su lista: ${R[1]}"; fallos=$((fallos+1));; *) echo "  ✓ no está en su lista · 200";; esac;;
  *) echo "  ✗ su lista: ${R[1]}"; fallos=$((fallos+1));; esac
espera 404 "borrarlo" "${R[2]}"
espera 404 "ver la operación de $A" "${R[3]}"

echo "── sin un token de celda para ore-postgres, nada"
mapfile -t R < <(en "$A" \
  "pide_con TN GET /v1/postgres/proyectos" \
  "pide_con TB GET /v1/postgres/proyectos" \
  "pide_con TI GET /v1/postgres/proyectos")
espera 401 "sin token" "${R[0]}"
espera 401 "con la firma rota" "${R[1]}"
espera 401 "con el token de la celda para ore-iam (otra audiencia)" "${R[2]}"

echo "── $A lo borra"
mapfile -t R < <(en "$A" \
  "pide DELETE /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/operaciones/$OP")
espera 202 "borrar" "${R[0]}"
espera 404 "ya no está" "${R[1]}"
espera 200 "la operación de crear sigue ahí" "${R[2]}"

echo
[ $fallos = 0 ] && echo "P4·1 ✓ todo" || { echo "P4·1 ✗ $fallos fallos"; exit 1; }
