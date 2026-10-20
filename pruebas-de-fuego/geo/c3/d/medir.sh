#!/usr/bin/env bash
# ORE 0068 C3·d · Las 12 consultas de SpatialBench a escala 100 con el Build elástico: un driver a mano
# (como c0) en el pool `pesado-driver` y de 1 a MAX ejecutores en `pesado`, en el namespace de medida
# `c3d-medida` (el de `generar.sh`, con la cuenta `sb` que lee el bucket). Con la imagen desplegada
# (`puesto-spark:1`). Saca las líneas `C3D|` y la línea de tiempo de los pods.
#   bash pruebas-de-fuego/geo/c3/d/medir.sh [MAX=2] [LIMITE_S=600] [CASOS=q1,q10] [MUESTRA=20]
#   REGLAS=locales bash …/medir.sh CASOS=q2      # con las ReglasDeOre de c3/b/regla.sh, no las de la imagen
#   PREVIO="lentas.py lentas2.py" bash …/medir.sh EXPERIMENTOS=R2,R3,R4   # un previo (sus ficheros, juntos) en vez de las consultas
#   BORRAR=1 bash pruebas-de-fuego/geo/c3/d/medir.sh    # retira el permiso de lectura y el namespace
# Prepara solo lo que falte: el namespace, la cuenta `sb` y un permiso TEMPORAL de LECTURA sobre el
# bucket del banco (los datos de la escala 100 ya están: los subió `generar.sh`).
# Cada 15 s, el disco efímero más alto de los nodos `pesado` (`C3D|disco|…`, del kubelet).
set -uo pipefail
export PYTHONUTF8=1
NS=c3d-medida
aqui="$(cd "$(dirname "$0")" && pwd)"
P=project-8853a180-450d-47be-b83
IMAGEN=europe-west1-docker.pkg.dev/$P/ore/puesto-spark:1
NUM=$(gcloud projects describe $P --format='value(projectNumber)')
QUIEN="principal://iam.googleapis.com/projects/$NUM/locations/global/workloadIdentityPools/$P.svc.id.goog/subject/ns/$NS/sa/sb"
if [ "${BORRAR:-0}" = 1 ]; then
  gcloud storage buckets remove-iam-policy-binding gs://$P-banco-geo --member="$QUIEN" --role=roles/storage.objectViewer >/dev/null && echo "== permiso retirado"
  kubectl delete ns $NS --ignore-not-found --wait=false; exit 0
fi
kubectl get ns $NS >/dev/null 2>&1 || kubectl create ns $NS >/dev/null
kubectl -n $NS create serviceaccount sb --dry-run=client -o yaml | kubectl apply -f - >/dev/null
gcloud storage buckets add-iam-policy-binding gs://$P-banco-geo --member="$QUIEN" --role=roles/storage.objectViewer >/dev/null   && echo "== permiso temporal: $NS/sb lee gs://$P-banco-geo"
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
  --from-file=consultas.json="$aqui/consultas.json" \
  $( [ "${REGLAS:-}" = locales ] && echo --from-file=ore-spark-reglas.jar="$aqui/../b/ore-spark-reglas.jar" ) \
  $( [ -n "${PREVIO:-}" ] && { for f in $PREVIO; do cat "$aqui/$f"; echo; done > /tmp/previo-$J.py; echo --from-file=previo.py=/tmp/previo-$J.py; } ) >/dev/null
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
disco() {  # el pico de ephemeral-storage de cada nodo pesado, cada 15 s
  while :; do
    for n in $(kubectl get nodes -l ore.dev/pool=pesado -o name 2>/dev/null); do
      MSYS_NO_PATHCONV=1 kubectl get --raw "/api/v1/nodes/${n#node/}/proxy/stats/summary" 2>/dev/null | python3 -c "
import json,sys; d=json.load(sys.stdin); fs=d['node'].get('fs',{})
p=max([x.get('ephemeral-storage',{}).get('usedBytes',0) for x in d.get('pods',[]) if x['podRef']['namespace']=='$NS'] or [0])
print('C3D|disco|t=%d nodo=%s usado_nodo_gib=%.1f libre_gib=%.1f pod_max_gib=%.1f' % ($(date -u +%s)-$T0, '${n#node/}'[-4:], fs.get('usedBytes',0)/2**30, fs.get('availableBytes',0)/2**30, p/2**30))" 2>/dev/null
    done
    sleep 15
  done
}
disco & VIGIA=$!
while :; do
  e=$(kubectl -n $NS get job $J -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  [ $(( $(date -u +%s) - T0 )) -gt 11000 ] && { echo "⛔ pasó de 3 h"; break; }
  sleep 15
done
kill $VIGIA 2>/dev/null
POD=$(kubectl -n $NS get pods -l job-name=$J -o name | head -1)
echo "== estado: $e · $(( $(date -u +%s) - T0 )) s"
kubectl -n $NS logs $POD 2>/dev/null | grep -E '^C3D[|]'
kubectl -n $NS logs $POD 2>/dev/null | grep -E "Traceback|Error|Exception" | grep -v "^\s*at \|forbidden" | head -6
kubectl -n $NS delete job $J --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $J >/dev/null 2>&1
