#!/usr/bin/env bash
# ORE 0068 C3·d · Las 12 consultas de SpatialBench a escala 100 con el Build elástico: un driver a mano
# (como c0) en el pool `pesado-driver` y de 1 a MAX ejecutores en `pesado`, en el namespace de medida
# `c3d-medida` (el de `generar.sh`, con la cuenta `sb` que lee el bucket). Con la imagen desplegada
# (`puesto-spark:1`). Saca las líneas `C3D|` y la línea de tiempo de los pods.
#   bash pruebas-de-fuego/geo/c3/d/medir.sh [MAX=2] [LIMITE_S=600] [CASOS=q1,q10]
set -uo pipefail
export PYTHONUTF8=1
NS=c3d-medida
aqui="$(cd "$(dirname "$0")" && pwd)"
P=project-8853a180-450d-47be-b83
IMAGEN=europe-west1-docker.pkg.dev/$P/ore/puesto-spark:1
GCS=https://repo1.maven.org/maven2/com/google/cloud/bigdataoss/gcs-connector/3.0.4/gcs-connector-3.0.4-shaded.jar
cat <<YAML | kubectl apply -f - >/dev/null
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata: {name: driver, namespace: $NS}
rules:
  - {apiGroups: [""], resources: [pods, configmaps, services, persistentvolumeclaims], verbs: [create, get, list, watch, delete, deletecollection]}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata: {name: driver, namespace: $NS}
roleRef: {apiGroup: rbac.authorization.k8s.io, kind: Role, name: driver}
subjects: [{kind: ServiceAccount, name: sb, namespace: $NS}]
YAML
J=d-$(date +%H%M%S)
kubectl -n $NS create configmap $J --from-file=medir.py="$aqui/medir.py" --from-file=plantilla.yaml="$aqui/plantilla.yaml" \
  --from-file=consultas.json="$aqui/consultas.json" >/dev/null
ENV=""; for kv in "$@"; do ENV="$ENV
            - {name: ${kv%%=*}, value: \"${kv#*=}\"}"; done
T0=$(date -u +%s); echo "== $J · $* · $(date -u +%T)"
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata: {name: $J, namespace: $NS}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 10800
  ttlSecondsAfterFinished: 3600
  template:
    metadata: {labels: {c3d: "si", c3d-rol: driver}}
    spec:
      restartPolicy: Never
      serviceAccountName: sb
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      nodeSelector: {ore.dev/pool: pesado-driver}
      tolerations: [{key: ore.dev/pesado-driver, operator: Equal, value: "true", effect: NoSchedule}]
      volumes: [{name: m, configMap: {name: $J}}]
      containers:
        - name: driver
          image: $IMAGEN
          imagePullPolicy: Always
          command: ["bash", "-c", "python3 -c \"import urllib.request as u; u.urlretrieve('$GCS', '/tmp/gcs-connector.jar')\" && python3 /m/medir.py"]
          workingDir: /tmp
          env:
            - {name: HOME, value: /tmp}
            - {name: NS, value: $NS}
            - {name: IMAGEN, value: "$IMAGEN"}
            - {name: DATOS, value: "gs://$P-banco-geo/spatialbench/sf100"}
            - {name: POD_NAME, valueFrom: {fieldRef: {fieldPath: metadata.name}}}
            - {name: POD_IP, valueFrom: {fieldRef: {fieldPath: status.podIP}}}$ENV
          volumeMounts: [{name: m, mountPath: /m}]
          resources:
            requests: {cpu: "1500m", memory: 12Gi}
            limits: {memory: 12Gi}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
while :; do
  e=$(kubectl -n $NS get job $J -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  [ $(( $(date -u +%s) - T0 )) -gt 11000 ] && { echo "⛔ pasó de 3 h"; break; }
  sleep 15
done
POD=$(kubectl -n $NS get pods -l job-name=$J -o name | head -1)
echo "== estado: $e · $(( $(date -u +%s) - T0 )) s"
kubectl -n $NS logs $POD 2>/dev/null | grep -E '^C3D[|]'
kubectl -n $NS logs $POD 2>/dev/null | grep -E "Traceback|Error|Exception" | grep -v "^\s*at \|forbidden" | head -6
kubectl -n $NS delete job $J --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $J >/dev/null 2>&1
