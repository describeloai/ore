#!/usr/bin/env bash
# R0 · La estructura de Query (docs/ejecuciones.md §8.10): un Job de Kueue en t-victor con la
# seguridad de un puesto y la imagen de hoy (`puesto-python:1`).
#   r01          → qué operaciones derraman y cuáles fallan: 4 GiB como L, en `pesado` (SSD de sobra)
#   r02          → el puesto crece en marcha: empieza con 4 GiB en `pesado`; al ver `pide-crecer`
#                  este guion lo redimensiona en caliente a 16 GiB, y a 400 GiB al ver `pide-imposible`
#   r03-balanced → un disco efímero propio (pd-balanced, 200 GiB) en la cola de las sesiones
#   r03-ssd      → lo mismo con pd-ssd
#   r03-local    → el emptyDir del nodo de las sesiones (lo de hoy)
# Guarda las líneas JSON en resultados/<fecha>-<caso>.jsonl, con los tiempos del Job, y borra el
# Job y su ConfigMap al acabar.
#   bash pruebas-de-fuego/estructura/medir.sh r01|r02|r03-balanced|r03-ssd|r03-local [VAR=valor ...]
set -uo pipefail
NS=t-victor
aqui="$(cd "$(dirname "$0")" && pwd)"
CASO=${1:?r01|r01b|r01c|s1-mode|n0-node|r02|r03-balanced|r03-ssd|r03-local}; shift
IMG=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-python:1
N=estructura-$CASO-$(date +%H%M%S)
SAL="$aqui/resultados/$(date +%Y%m%d)-$CASO.jsonl"
mkdir -p "$aqui/resultados"
t() { date -u +%s; }

extra=""
for kv in "$@"; do extra="$extra            - {name: ${kv%%=*}, value: \"${kv#*=}\"}
"; done
# Como una sesión de L (malla/51-el-puesto.yaml): pide poco y su techo es 2 CPU / 4 GiB.
L_REC='requests: {cpu: "100m", memory: 768Mi}
            limits: {cpu: "2", memory: 4Gi}'
VOL="- {name: trabajo, emptyDir: {}}"
MONTA="{name: trabajo, mountPath: /trabajo}"
case $CASO in
  r01|r01b) COLA=cola-pesada; POOL=pesado; CMD='["python3", "/m/r01_derrame.py"]'
       REC='requests: {cpu: "2", memory: 4Gi, ephemeral-storage: 120Gi}
            limits: {cpu: "2", memory: 4Gi, ephemeral-storage: 120Gi}' ;;
  r01c) COLA=cola-pesada; POOL=pesado; CMD='["python3", "/m/r01_derrame.py"]'
       REC='requests: {cpu: "2", memory: 4Gi, ephemeral-storage: 20Gi}
            limits: {cpu: "2", memory: 4Gi, ephemeral-storage: 20Gi}' ;;
  # Elastic Sessions S1 · la primera prueba del session pool (malla/75, singleProcessOOMKill):
  # las tres operaciones que en S0·b mataron el contenedor entero, con la forma de una sesión
  # (base de C1, techo de 4 GiB, `/trabajo` de 64 GiB en la SSD). Pasa si cada una sale
  # `killed` (muere su proceso), el banco llega a `fin` y el Job termina bien.
  s1-mode) COLA=cola-sesiones; POOL=sesiones; CMD='["python3", "/m/r01_derrame.py"]'; REC=$L_REC
       VOL="- {name: trabajo, emptyDir: {sizeLimit: 64Gi}}"
       extra="$extra            - {name: CASOS, value: \"agrupa_alta,mode,histogram,approx_quantile\"}
" ;;
  # Elastic Sessions N0 · Node antes de darle elasticidad (n0_node.mjs): los topes por defecto, la
  # segunda conexión con la consulta en marcha mientras el pod crece a 16 GiB, y sin tope en un hijo.
  n0-node) COLA=cola-sesiones; POOL=sesiones; CMD='["node", "/m/n0_node.mjs"]'; REC=$L_REC
       IMG=${IMG%/puesto-python:1}/puesto-node:1
       VOL="- {name: trabajo, emptyDir: {sizeLimit: 64Gi}}" ;;
  r02) COLA=cola-pesada; POOL=pesado; CMD='["python3", "/m/r02_crecer.py"]'
       REC='requests: {cpu: "2", memory: 4Gi, ephemeral-storage: 120Gi}
            limits: {cpu: "2", memory: 4Gi, ephemeral-storage: 120Gi}' ;;
  r03-balanced|r03-ssd)
       COLA=cola; POOL=""; CMD='["python3", "/m/r03_disco.py"]'; REC=$L_REC
       CLASE=standard-rwo; [ $CASO = r03-ssd ] && CLASE=premium-rwo
       VOL="- {name: trabajo, emptyDir: {}}
        - name: derrame
          ephemeral:
            volumeClaimTemplate:
              spec: {accessModes: [ReadWriteOnce], storageClassName: $CLASE, resources: {requests: {storage: 200Gi}}}"
       MONTA="{name: trabajo, mountPath: /trabajo}, {name: derrame, mountPath: /derrame}" ;;
  r03-local)
       COLA=cola; POOL=""; CMD='["python3", "/m/r03_disco.py"]'; REC=$L_REC
       VOL="- {name: trabajo, emptyDir: {}}
        - {name: derrame, emptyDir: {sizeLimit: 8Gi}}"
       MONTA="{name: trabajo, mountPath: /trabajo}, {name: derrame, mountPath: /derrame}"
       extra="$extra            - {name: GB, value: \"2\"}
" ;;
  *) echo "¿$CASO?"; exit 2 ;;
esac

kubectl -n $NS create configmap $N --from-file="$aqui/r01_derrame.py" --from-file="$aqui/r02_crecer.py" --from-file="$aqui/r03_disco.py" --from-file="$aqui/n0_node.mjs" >/dev/null || exit 1
T0=$(t)
cat <<YAML | kubectl create -f - >/dev/null || exit 1
apiVersion: batch/v1
kind: Job
metadata:
  name: $N
  namespace: $NS
  labels: {kueue.x-k8s.io/queue-name: $COLA, ore.dev/medida: estructura}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 7200
  ttlSecondsAfterFinished: 900
  template:
    metadata:
      labels: {ore.dev/medida: estructura, ore.dev/rol: puesto}
    spec:
      restartPolicy: Never
      serviceAccountName: puesto
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, fsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      volumes:
        - {name: guion, configMap: {name: $N}}
        $VOL
      containers:
        - name: medida
          image: $IMG
          imagePullPolicy: Always
          command: $CMD
          workingDir: /trabajo
          env:
            - {name: HOME, value: /trabajo}
            - {name: TRABAJO, value: /trabajo}
            - {name: VOLUMEN, value: $CASO}
            - {name: DERRAME, value: /derrame}
$extra          volumeMounts: [{name: guion, mountPath: /m}, $MONTA]
          resizePolicy: [{resourceName: memory, restartPolicy: NotRequired}, {resourceName: cpu, restartPolicy: NotRequired}]
          resources:
            $REC
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
echo "== $N · cola $COLA"
T_ADM=""; T_CORRE=""; POD=""; CRECIDO=""; IMPOSIBLE=""
redimensionar() {  # $1 = memoria
  local t1=$(t)
  kubectl -n $NS patch $POD --subresource resize \
    -p "{\"spec\":{\"containers\":[{\"name\":\"medida\",\"resources\":{\"requests\":{\"memory\":\"$1\"},\"limits\":{\"memory\":\"$1\"}}}]}}" 2>&1 | tail -1
  local e=""
  for i in $(seq 1 30); do
    e=$(kubectl -n $NS get $POD -o jsonpath='{range .status.conditions[*]}{.type}={.status}:{.reason}:{.message} {end}' 2>/dev/null | tr ' ' '\n' | grep -E "PodResize" | tr '\n' ' ')
    local real=$(kubectl -n $NS get $POD -o jsonpath='{.status.containerStatuses[0].resources.limits.memory}' 2>/dev/null)
    [ "$real" = "$1" ] && { echo "{\"evento\": \"redimensionado\", \"a\": \"$1\", \"s\": $(( $(t) - t1 )), \"condiciones\": \"$e\"}" | tee -a "$SAL"; return; }
    sleep 1
  done
  echo "{\"evento\": \"redimensionado\", \"a\": \"$1\", \"s\": null, \"condiciones\": \"$e\"}" | tee -a "$SAL"
}
while :; do
  [ -z "$T_ADM" ] && kubectl -n $NS get workloads -o jsonpath='{range .items[*]}{.metadata.ownerReferences[0].name} {.status.admission.clusterQueue}{"\n"}{end}' 2>/dev/null | grep -q "^$N cq-" && T_ADM=$(t)
  [ -z "$POD" ] && POD=$(kubectl -n $NS get pods -l job-name=$N -o name 2>/dev/null | head -1)
  if [ -n "$POD" ] && [ -z "$T_CORRE" ]; then
    [ "$(kubectl -n $NS get $POD -o jsonpath='{.status.phase}' 2>/dev/null)" = Running ] && T_CORRE=$(t)
  fi
  if { [ $CASO = r02 ] || [ $CASO = n0-node ]; } && [ -n "$T_CORRE" ]; then
    LOG=$(kubectl -n $NS logs $POD 2>/dev/null)
    [ -z "$CRECIDO" ] && echo "$LOG" | grep -q '"pide-crecer"' && { CRECIDO=1; redimensionar 16Gi; }
    [ -z "$IMPOSIBLE" ] && echo "$LOG" | grep -q '"pide-imposible"' && { IMPOSIBLE=1; redimensionar 400Gi; }
  fi
  e=$(kubectl -n $NS get job $N -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  [ $(( $(t) - T0 )) -gt 7300 ] && { echo "⛔ pasó de 2 h"; break; }
  sleep 2
done
T_FIN=$(t)
kubectl -n $NS logs $POD > "$aqui/resultados/$N.log" 2>&1
grep '^{' "$aqui/resultados/$N.log" >> "$SAL"
NODO=$(kubectl -n $NS get $POD -o jsonpath='{.spec.nodeName}' 2>/dev/null)
MUERTE=$(kubectl -n $NS get $POD -o jsonpath='{.status.containerStatuses[0].state.terminated.reason}' 2>/dev/null)
echo "{\"caso\": \"$CASO\", \"evento\": \"job\", \"job\": \"$N\", \"nodo\": \"$NODO\", \"admitido_s\": $(( ${T_ADM:-$T0} - T0 )), \"corriendo_s\": $(( ${T_CORRE:-$T0} - T0 )), \"fin_s\": $(( T_FIN - T0 )), \"estado\": \"$e\", \"terminado\": \"$MUERTE\"}" | tee -a "$SAL"
kubectl -n $NS delete job $N --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $N >/dev/null 2>&1
tail -4 "$aqui/resultados/$N.log" | cut -c1-300
