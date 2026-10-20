#!/usr/bin/env bash
# ORE 0068 C4·2 · El previo del generador de DuckDB (medir.py) en victor, en un Job con los límites de un
# puesto Lightweight (2 CPU, 4 GiB, `sistema-spot`, como malla/51-el-puesto.yaml) y la imagen desplegada.
# Datos sintéticos generados en el pod: no lee nada del lago ni del bucket.
#   bash pruebas-de-fuego/geo/c4/previo/medir.sh [CLIENTES=100000,1000000] [CASOS=within,extensive] [LIMITE_S=600]
#   BORRAR=1 bash pruebas-de-fuego/geo/c4/previo/medir.sh     # retira el namespace
set -uo pipefail
export PYTHONUTF8=1
NS=c4-medida
aqui="$(cd "$(dirname "$0")" && pwd)"
P=project-8853a180-450d-47be-b83
IMAGEN=europe-west1-docker.pkg.dev/$P/ore/puesto-python:1
if [ "${BORRAR:-0}" = 1 ]; then kubectl delete ns $NS --ignore-not-found --wait=false; exit 0; fi
kubectl get ns $NS >/dev/null 2>&1 || kubectl create ns $NS >/dev/null
J=p-$(date +%H%M%S)
kubectl -n $NS create configmap $J --from-file=medir.py="$aqui/medir.py" >/dev/null
ENV=""; for kv in "$@"; do ENV="$ENV
            - {name: ${kv%%=*}, value: \"${kv#*=}\"}"; done
T0=$(date -u +%s); echo "== $J · $* · $(date -u +%T)"
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata: {name: $J, namespace: $NS}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 7200
  ttlSecondsAfterFinished: 3600
  template:
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      volumes: [{name: m, configMap: {name: $J}}, {name: d, emptyDir: {sizeLimit: 8Gi}}]
      containers:
        - name: duckdb
          image: $IMAGEN
          command: ["python3", "/m/medir.py"]
          workingDir: /d
          env:
            - {name: HOME, value: /tmp}$ENV
          volumeMounts: [{name: m, mountPath: /m}, {name: d, mountPath: /d}]
          resources:
            requests: {cpu: "100m", memory: 768Mi}
            limits: {cpu: "2", memory: 4Gi}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
while :; do
  e=$(kubectl -n $NS get job $J -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  [ $(( $(date -u +%s) - T0 )) -gt 7300 ] && { echo "⛔ pasó de 2 h"; break; }
  sleep 15
done
POD=$(kubectl -n $NS get pods -l job-name=$J -o name | head -1)
echo "== estado: $e · $(( $(date -u +%s) - T0 )) s"
kubectl -n $NS logs $POD 2>/dev/null | grep -E '^C4P[|]'
kubectl -n $NS logs $POD 2>/dev/null | grep -E "Traceback|Error|OOM" | head -6
kubectl -n $NS get pod ${POD#pod/} -o jsonpath='{.status.containerStatuses[0].state.terminated.reason}' 2>/dev/null; echo
kubectl -n $NS delete job $J --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $J >/dev/null 2>&1
