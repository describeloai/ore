#!/usr/bin/env bash
# ORE 0068 C3·c0 · Un driver de Build elástico en victor, a mano, en el namespace de medida `c3c0`
# (laboratorio.yaml): el driver en el pool `pesado-driver` (normal) y los ejecutores que crea en `pesado`
# (spot). Saca las líneas `C3C|`, la línea de tiempo de los pods y nodos, y borra el Job.
#   bash pruebas-de-fuego/geo/c3/c0/c0.sh "vap,pequeno,grande,ocioso" [VAR=valor …]
#   MATAR=1 …      con dos ejecutores vivos 20 s, borra uno (lo que haría spot)
#   BORRAR=1 bash pruebas-de-fuego/geo/c3/c0/c0.sh ""     borra el laboratorio (namespace y política)
set -uo pipefail
export PYTHONUTF8=1
NS=${NS:-c3c0}
aqui="$(cd "$(dirname "$0")" && pwd)"
CASOS=${1-pequeno}; shift || true
if [ "${BORRAR:-0}" = 1 ]; then
  for n in c3c0 c3c0b; do
    kubectl delete validatingadmissionpolicybinding,validatingadmissionpolicy $n-lo-que-crea-el-driver --ignore-not-found
    kubectl delete ns $n --ignore-not-found --wait=false
  done
  kubectl delete clusterqueue cq-c3c0-a cq-c3c0-b --ignore-not-found; exit 0
fi
REG=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-spark
IMAGEN=$REG@$(gcloud artifacts docker images describe $REG:1 --format='value(image_summary.digest)')
API=$(kubectl get endpointslice -n default -l kubernetes.io/service-name=kubernetes -o jsonpath='{.items[0].endpoints[0].addresses[0]}')
python3 - "$aqui/laboratorio.yaml" "$NS" "$IMAGEN" "$API" <<'PY' | kubectl apply -f - >/dev/null || { echo "⛔ el laboratorio no se aplicó"; exit 1; }
import sys; t = open(sys.argv[1], encoding="utf-8").read()
for k, v in zip(("NS", "IMAGEN", "API"), sys.argv[2:]): t = t.replace("${%s}" % k, v)
print(t)
PY
if [ "${KUEUE:-0}" = 1 ]; then
  kubectl apply -f "$aqui/kueue-lab.yaml" >/dev/null
  printf 'apiVersion: kueue.x-k8s.io/v1beta2\nkind: LocalQueue\nmetadata: {name: cola-ejecutores, namespace: %s}\nspec: {clusterQueue: %s}\n' \
    $NS $([ $NS = c3c0 ] && echo cq-c3c0-a || echo cq-c3c0-b) | kubectl apply -f - >/dev/null
  set -- "$@" KUEUE=1
fi
kubectl -n $NS create configmap c3c0-ejecutor --from-file="$aqui/ejecutor.sh" --from-file="$aqui/decom.sh" \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
J=d-$(date +%H%M%S)
kubectl -n $NS create configmap $J --from-file=driver.py="$aqui/driver.py" --from-file=plantilla.yaml="$aqui/plantilla.yaml" >/dev/null
ENV=""; for kv in "$@"; do ENV="$ENV
            - {name: ${kv%%=*}, value: \"${kv#*=}\"}"; done
T0=$(date -u +%s); echo "== $J · $CASOS · $(date -u +%T) · nodos: $(kubectl get nodes -L ore.dev/pool --no-headers | awk '{print $NF}' | sort | uniq -c | tr '\n' ' ')"
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata: {name: $J, namespace: $NS}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 3000
  ttlSecondsAfterFinished: 900
  template:
    metadata: {labels: {c3c0-spark: "si", c3c0-rol: driver, c3c0: $J}}
    spec:
      restartPolicy: Never
      serviceAccountName: driver-pesado
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      nodeSelector: {ore.dev/pool: pesado-driver}
      tolerations: [{key: ore.dev/pesado-driver, operator: Equal, value: "true", effect: NoSchedule}]
      volumes: [{name: m, configMap: {name: $J}}]
      containers:
        - name: driver
          image: $IMAGEN
          command: ["python3", "/m/driver.py"]
          workingDir: /tmp
          env:
            - {name: HOME, value: /tmp}
            - {name: NS, value: $NS}
            - {name: IMAGEN, value: "$IMAGEN"}
            - {name: CASOS, value: "$CASOS"}
            - {name: POD_NAME, valueFrom: {fieldRef: {fieldPath: metadata.name}}}
            - {name: POD_UID, valueFrom: {fieldRef: {fieldPath: metadata.uid}}}
            - {name: POD_IP, valueFrom: {fieldRef: {fieldPath: status.podIP}}}$ENV
          volumeMounts: [{name: m, mountPath: /m}]
          resources:
            requests: {cpu: "1500m", memory: 12Gi}
            limits: {memory: 12Gi}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
kubectl -n $NS get job $J >/dev/null 2>&1 || { echo "⛔ el Job no se creó"; exit 1; }
matado=0
while :; do
  e=$(kubectl -n $NS get job $J -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  if [ "${MATAR:-0}" = 1 ] && [ $matado = 0 ]; then
    viv=$(kubectl -n $NS get pods -l spark-role=executor --field-selector=status.phase=Running -o name 2>/dev/null)
    if [ "$(echo "$viv" | grep -c .)" -ge 2 ]; then
      [ -z "${desde:-}" ] && desde=$(date -u +%s)
      if [ $(( $(date -u +%s) - desde )) -ge 20 ]; then
        v=$(echo "$viv" | tail -1); echo "== $(( $(date -u +%s) - T0 )) s · borro $v (como spot)"
        kubectl -n $NS delete $v --grace-period=15 --wait=false >/dev/null; matado=1
      fi
    fi
  fi
  [ $(( $(date -u +%s) - T0 )) -gt 3100 ] && { echo "⛔ pasó de 52 min"; break; }
  sleep 3
done
POD=$(kubectl -n $NS get pods -l job-name=$J -o name | head -1)
echo "== estado: $e · $(( $(date -u +%s) - T0 )) s desde que se creó"
kubectl -n $NS logs $POD 2>/dev/null | grep -E '^C3C[|]'
kubectl -n $NS logs $POD 2>/dev/null | grep -E "Traceback|Error|Exception|denied" | grep -v "^\s*at " | head -8
echo "== la línea de tiempo (segundos desde la creación del Job)"
kubectl -n $NS get events -o json | python3 -c "
import json,sys,datetime as d
T0=$T0
ev=json.load(sys.stdin)['items']
def t(e):
    s=e.get('eventTime') or e.get('lastTimestamp') or e.get('firstTimestamp')
    return d.datetime.fromisoformat(s.replace('Z','+00:00')).timestamp() if s else 0
for e in sorted(ev,key=t):
    if t(e)<T0-5: continue
    r=e.get('reason'); o=e['involvedObject'].get('name','')
    if r in ('Scheduled','Pulled','Started','TriggeredScaleUp','FailedScheduling','Killing','NotTriggerScaleUp','FailedCreate'):
        print('  %6.0f  %-18s %-40s %s' % (t(e)-T0, r, o[-40:], (e.get('message') or '')[:110]))
"
echo "== nodos ahora: $(kubectl get nodes -L ore.dev/pool --no-headers | awk '{print $NF}' | sort | uniq -c | tr '\n' ' ')"
kubectl -n $NS delete job $J --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $J >/dev/null 2>&1
