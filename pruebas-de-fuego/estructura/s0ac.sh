#!/usr/bin/env bash
# S0·a + S0·c (docs/elastic-sessions.md §7) · Kueue ante un pod redimensionado, y bajar en caliente.
#   Un Job en `cola` (la de las sesiones) que nace con 512Mi de requests y 1Gi de límite.
#   · pide-crecer → este guion lo lleva a 3Gi (requests y limits) y mira qué cuenta Kueue
#     (la ClusterQueue y el Workload) y la ResourceQuota; luego pide a Kueue otro Job de 3Gi:
#     ¿lo admite, aunque lo real ya no quepa?
#   · pide-bajar → a 1Gi: ¿baja, o se queda pendiente porque se usa más?
#   bash pruebas-de-fuego/estructura/s0ac.sh
set -uo pipefail
NS=t-victor
aqui="$(cd "$(dirname "$0")" && pwd)"
IMG=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-python:1
N=estructura-s0ac-$(date +%H%M%S)
SAL="$aqui/resultados/$(date +%Y%m%d)-s0ac.jsonl"
mkdir -p "$aqui/resultados"
t() { date -u +%s; }
job() {  # $1 nombre, $2 requests.memory, $3 limits.memory, $4 comando
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: $1
  namespace: $NS
  labels: {kueue.x-k8s.io/queue-name: cola, ore.dev/medida: estructura}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 1800
  ttlSecondsAfterFinished: 600
  template:
    metadata:
      labels: {ore.dev/medida: estructura, ore.dev/rol: puesto}
    spec:
      restartPolicy: Never
      serviceAccountName: puesto
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      volumes: [{name: guion, configMap: {name: $N}}, {name: trabajo, emptyDir: {sizeLimit: 4Gi}}]
      containers:
        - name: medida
          image: $IMG
          imagePullPolicy: Always
          command: $4
          workingDir: /trabajo
          env: [{name: HOME, value: /trabajo}, {name: TRABAJO, value: /trabajo}]
          volumeMounts: [{name: guion, mountPath: /m}, {name: trabajo, mountPath: /trabajo}]
          resizePolicy: [{resourceName: memory, restartPolicy: NotRequired}]
          resources: {requests: {cpu: 100m, memory: $2}, limits: {cpu: "2", memory: $3}}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
}
kueue() {  # $1 paso
  local cq wl rq
  cq=$(kubectl get clusterqueue cq-victor -o jsonpath='{range .status.flavorsUsage[*]}{.name}:{range .resources[*]}{.name}={.total} {end}; {end}')
  wl=$(kubectl -n $NS get workloads -o jsonpath='{range .items[*]}{.metadata.ownerReferences[0].name}={.status.admission.podSetAssignments[0].resourceUsage.memory},admitido:{.status.conditions[?(@.type=="Admitted")].status}; {end}' | tr ';' '\n' | grep estructura-s0 | tr '\n' ' ')
  rq=$(kubectl -n $NS get resourcequota cuota -o jsonpath='{.status.used.requests\.memory}')
  echo "{\"evento\": \"kueue\", \"paso\": \"$1\", \"clusterqueue\": \"$cq\", \"workloads\": \"$wl\", \"resourcequota_requests_memory\": \"$rq\"}" | tee -a "$SAL"
}
redimensionar() {  # $1 memoria
  local t1=$(t)
  kubectl -n $NS patch $POD --subresource resize -p "{\"spec\":{\"containers\":[{\"name\":\"medida\",\"resources\":{\"requests\":{\"memory\":\"$1\"},\"limits\":{\"memory\":\"$1\"}}}]}}" 2>&1 | tail -1
  local real="" c=""
  for i in $(seq 1 60); do
    real=$(kubectl -n $NS get $POD -o jsonpath='{.status.containerStatuses[0].resources.limits.memory}')
    c=$(kubectl -n $NS get $POD -o jsonpath='{range .status.conditions[?(@.type=="PodResizePending")]}{.reason}: {.message}{end}{range .status.conditions[?(@.type=="PodResizeInProgress")]} en curso: {.reason} {.message}{end}')
    [ "$real" = "$1" ] && break
    sleep 1
  done
  echo "{\"evento\": \"redimensionado\", \"a\": \"$1\", \"real\": \"$real\", \"s\": $(( $(t) - t1 )), \"condiciones\": \"$c\"}" | tee -a "$SAL"
}

kubectl -n $NS create configmap $N --from-file="$aqui/s0_bajar.py" >/dev/null || exit 1
kueue "antes"
job $N 512Mi 1Gi '["python3", "/m/s0_bajar.py"]' || exit 1
echo "== $N"
POD=""; CRECIDO=""; BAJADO=""; T0=$(t)
while :; do
  [ -z "$POD" ] && POD=$(kubectl -n $NS get pods -l job-name=$N -o name 2>/dev/null | head -1)
  if [ -n "$POD" ]; then
    LOG=$(kubectl -n $NS logs $POD 2>/dev/null)
    if [ -z "$CRECIDO" ] && echo "$LOG" | grep -q '"pide-crecer"'; then
      CRECIDO=1; kueue "admitido, 512Mi"; redimensionar 3Gi; sleep 3; kueue "tras crecer a 3Gi"
      # ¿Kueue admite otro de 3Gi aunque lo real (3Gi + 3Gi) ya no quepa en lo que cree tener?
      job $N-otro 3Gi 3Gi '["python3", "-c", "import time; time.sleep(60)"]'
      for i in $(seq 1 20); do sleep 2; kubectl -n $NS get workloads -o jsonpath='{range .items[*]}{.metadata.ownerReferences[0].name} {.status.conditions[?(@.type=="Admitted")].status}{"\n"}{end}' | grep -q "^$N-otro True" && break; done
      kueue "con otro Job de 3Gi pedido"
      echo "{\"evento\": \"otro-pod\", \"fase\": \"$(kubectl -n $NS get pods -l job-name=$N-otro -o jsonpath='{.items[0].status.phase} {.items[0].status.conditions[?(@.type=="PodScheduled")].message}' 2>/dev/null)\"}" | tee -a "$SAL"
      kubectl -n $NS delete job $N-otro --wait=false >/dev/null 2>&1
    fi
    if [ -z "$BAJADO" ] && echo "$LOG" | grep -q '"pide-bajar"'; then
      BAJADO=1; redimensionar 1Gi; kueue "tras bajar a 1Gi"
    fi
  fi
  e=$(kubectl -n $NS get job $N -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  [ $(( $(t) - T0 )) -gt 1500 ] && { echo "⛔ pasó de 25 min"; break; }
  sleep 2
done
kubectl -n $NS logs $POD > "$aqui/resultados/$N.log" 2>&1
grep '^{' "$aqui/resultados/$N.log" >> "$SAL"
echo "{\"evento\": \"job\", \"job\": \"$N\", \"estado\": \"$e\", \"terminado\": \"$(kubectl -n $NS get $POD -o jsonpath='{.status.containerStatuses[0].state.terminated.reason}')\"}" | tee -a "$SAL"
kubectl -n $NS delete job $N --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $N >/dev/null 2>&1
grep '^{' "$aqui/resultados/$N.log" | cut -c1-300
