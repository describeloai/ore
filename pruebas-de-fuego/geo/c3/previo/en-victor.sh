#!/usr/bin/env bash
# ORE 0068 C3 · previo · Un guion del previo en el puesto pesado de victor, sin persona: un Job de medida en
# `cola-pesada` (el pool `pesado`), con `puesto-spark:1` y los recursos de un Build pesado, como `frio.sh` de
# 6c. Sin lago: los datos los genera Spark dentro del pod. Saca las líneas `C3|` y borra el Job.
#   bash pruebas-de-fuego/geo/c3/previo/en-victor.sh difusion.py [VAR=valor …]
set -uo pipefail
NS=t-victor
aqui="$(cd "$(dirname "$0")" && pwd)"
PROG=${1:?el guion}; shift
N=medida-c3-$(basename "$PROG" .py)-$(date +%H%M%S)
IMAGEN=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-spark:1
ENV=""
for kv in "$@"; do ENV="$ENV
            - {name: ${kv%%=*}, value: \"${kv#*=}\"}"; done
# CON="a.jar b.txt": ficheros más junto al guion, en /m (relativos a este directorio).
MAS=""; for f in ${CON:-}; do MAS="$MAS --from-file=$(basename "$f")=$aqui/$f"; done
kubectl -n $NS create configmap $N --from-file=prog.py="$aqui/$PROG" $MAS >/dev/null
T0=$(date -u +%s)
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: $N
  namespace: $NS
  labels: {kueue.x-k8s.io/queue-name: cola-pesada, ore.dev/medida: "c3"}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 3600
  ttlSecondsAfterFinished: 600
  template:
    metadata:
      labels: {ore.dev/medida: "c3"}
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      securityContext: {runAsNonRoot: true, runAsUser: 65532, seccompProfile: {type: RuntimeDefault}}
      volumes: [{name: guion, configMap: {name: $N}}]
      containers:
        - name: medida
          image: $IMAGEN
          imagePullPolicy: Always
          command: ["python3", "/m/prog.py"]
          env:
            - {name: HOME, value: /tmp}
            - {name: HEAP, value: "38g"}$ENV
          volumeMounts: [{name: guion, mountPath: /m}]
          workingDir: /tmp
          resources:
            requests: {cpu: "7", memory: 48Gi}
            limits: {cpu: "8", memory: 48Gi}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
echo "== $N creado"
while :; do
  e=$(kubectl -n $NS get job $N -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  [ $(( $(date -u +%s) - T0 )) -gt 3900 ] && { echo "⛔ pasó de 65 min"; break; }
  sleep 10
done
POD=$(kubectl -n $NS get pods -l job-name=$N -o name | head -1)
echo "== estado: $e · $(( $(date -u +%s) - T0 )) s desde que se creó"
kubectl -n $NS logs $POD 2>/dev/null | grep -E '^C3[A-Z]*[|]'
kubectl -n $NS logs $POD 2>/dev/null | grep -E "Traceback|Error|Exception" | grep -v "^\s*at " | head -5
kubectl -n $NS delete job $N --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $N >/dev/null 2>&1
