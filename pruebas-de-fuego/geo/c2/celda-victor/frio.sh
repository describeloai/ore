#!/usr/bin/env bash
# ORE 0068 C2·3 pieza 6 (6c) · El puesto pesado en victor, sin persona: un Job de medida en `cola-pesada`
# (la LocalQueue de t-victor que lleva al pool `pesado`), con `puesto-spark:1`, los recursos de un Build
# pesado y la seguridad de un puesto, corre `join.py`. Dice cuándo pasa cada cosa —Kueue lo admite, el
# nodo nace, la imagen baja, el contenedor arranca, termina— y lo que mide el join. Al acabar lo borra.
#   bash pruebas-de-fuego/geo/c2/celda-victor/frio.sh [etiqueta]     (frío si el pool está a 0)
set -uo pipefail
NS=t-victor
aqui="$(cd "$(dirname "$0")" && pwd)"
N=medida-6c-${1:-frio}-$(date +%H%M%S)
IMAGEN=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-spark:1
t() { date -u +%s; }
kubectl -n $NS create configmap $N --from-file=join.py="$aqui/join.py" >/dev/null
NODOS0=$(kubectl get nodes -l ore.dev/pool=pesado -o name 2>/dev/null | wc -l)
T_CREADO=$(t)
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: $N
  namespace: $NS
  labels: {kueue.x-k8s.io/queue-name: cola-pesada, ore.dev/medida: "6c"}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 2400
  ttlSecondsAfterFinished: 600
  template:
    metadata:
      labels: {ore.dev/medida: "6c"}
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      securityContext: {runAsNonRoot: true, runAsUser: 65532, seccompProfile: {type: RuntimeDefault}}
      volumes: [{name: guion, configMap: {name: $N}}]
      containers:
        - name: medida
          image: $IMAGEN
          imagePullPolicy: Always
          command: ["python3", "/m/join.py"]
          env:
            - {name: HOME, value: /tmp}
            - {name: ORE_MEMORIA_MB, value: "49152"}
            - {name: PUNTOS, value: "${PUNTOS:-20000000}"}
            - {name: ZONAS, value: "${ZONAS:-20000}"}
          volumeMounts: [{name: guion, mountPath: /m}]
          workingDir: /tmp
          resources:
            requests: {cpu: "7", memory: 48Gi}
            limits: {cpu: "8", memory: 48Gi}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
echo "== $N creado · nodos pesados al empezar: $NODOS0"
T_ADMITIDO=""; T_NODO=""; T_FIN=""
while :; do
  [ -z "$T_ADMITIDO" ] && kubectl -n $NS get workloads -o jsonpath='{range .items[*]}{.metadata.ownerReferences[0].name} {.status.admission.clusterQueue}{"\n"}{end}' 2>/dev/null | grep -q "^$N cq-" && T_ADMITIDO=$(t)
  [ -z "$T_NODO" ] && [ "$(kubectl get nodes -l ore.dev/pool=pesado --no-headers 2>/dev/null | grep -c ' Ready')" -gt 0 ] && T_NODO=$(t)
  e=$(kubectl -n $NS get job $N -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) T_FIN=$(t); break;; esac
  [ $(( $(t) - T_CREADO )) -gt 2700 ] && { echo "⛔ pasó de 45 min"; break; }
  sleep 5
done
POD=$(kubectl -n $NS get pods -l job-name=$N -o name | head -1)
echo "== estado: $e"
echo "== los tiempos, desde que se crea el Job (s)"
echo "   Kueue lo admite:        $(( ${T_ADMITIDO:-$T_CREADO} - T_CREADO ))"
echo "   un nodo pesado Ready:   $(( ${T_NODO:-$T_CREADO} - T_CREADO ))$( [ "$NODOS0" -gt 0 ] && echo ' (ya estaba)')"
kubectl -n $NS get events --field-selector involvedObject.name=${POD#pod/} -o jsonpath='{range .items[*]}{.reason}|{.firstTimestamp}{.eventTime}|{.message}{"\n"}{end}' 2>/dev/null \
  | python3 -c "
import sys, datetime as d
t0 = $T_CREADO
for l in sys.stdin:
    r, ts, m = (l.rstrip('\n').split('|', 2) + ['', ''])[:3]
    if r in ('Scheduled', 'Pulling', 'Pulled', 'Created', 'Started', 'TriggeredScaleUp', 'FailedScheduling'):
        try:
            s = d.datetime.fromisoformat(ts[:19].replace('Z', '')).replace(tzinfo=d.timezone.utc).timestamp()
            print('   %-22s %4d  %s' % (r, s - t0, m[:110]))
        except ValueError:
            print('   %-22s  ?   %s' % (r, m[:110]))
"
echo "   el Job termina:         $(( ${T_FIN:-$(t)} - T_CREADO ))"
echo "== lo que mide el join"
kubectl -n $NS logs $POD 2>/dev/null | grep '^6C|' | sed 's/^6C|/   /; s/|/ = /'
kubectl -n $NS logs $POD 2>/dev/null | grep -iE "Traceback|Error|Exception" | head -5
kubectl -n $NS delete job $N --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $N >/dev/null 2>&1
