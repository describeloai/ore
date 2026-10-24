#!/usr/bin/env bash
# S1 · ¿Cuánto tarda en abrir una sesión en el session pool? (docs/elastic-sessions.md §7)
#   Un Job con la forma de una sesión (la imagen `puesto-python:1`, la base de C1, `cola-sesiones`)
#   que sólo importa duckdb y dice `listo`: mide lo que el pool cambia —admisión, nodo, imagen,
#   arranque— y no lo que es igual en los dos (Flux, el agente). Como `arranque` de Q0.
#   · frio    → el pool a 0 nodos (el guion lo comprueba antes)
#   · caliente → con el nodo ya encendido
#   bash pruebas-de-fuego/estructura/s1_apertura.sh frio|caliente
set -uo pipefail
NS=t-victor
MODO=${1:?frio|caliente}
aqui="$(cd "$(dirname "$0")" && pwd)"
IMG=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-python:1
N=estructura-s1-apertura-$MODO-$(date +%H%M%S)
SAL="$aqui/resultados/$(date +%Y%m%d)-s1-apertura.jsonl"
mkdir -p "$aqui/resultados"
ms() { python3 -c "import time; print(int(time.time()*1000))"; }
nodos=$(kubectl get nodes -l ore.dev/pool=sesiones --no-headers 2>/dev/null | wc -l)
case $MODO in
  frio) [ "$nodos" -eq 0 ] || { echo "⛔ el pool tiene $nodos nodo(s): en frío tiene que estar a 0"; exit 1; } ;;
  caliente) [ "$nodos" -ge 1 ] || { echo "⛔ el pool está a 0: en caliente tiene que haber nodo"; exit 1; } ;;
esac
T0=$(ms)
cat <<YAML | kubectl create -f - >/dev/null || exit 1
apiVersion: batch/v1
kind: Job
metadata:
  name: $N
  namespace: $NS
  labels: {kueue.x-k8s.io/queue-name: cola-sesiones, ore.dev/medida: estructura}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 900
  ttlSecondsAfterFinished: 300
  template:
    metadata:
      labels: {ore.dev/medida: estructura, ore.dev/rol: puesto}
    spec:
      restartPolicy: Never
      serviceAccountName: puesto
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      volumes: [{name: trabajo, emptyDir: {sizeLimit: 64Gi}}]
      containers:
        - name: medida
          image: $IMG
          imagePullPolicy: Always
          command: ["python3", "-c", "import duckdb; print('listo', flush=True)"]
          workingDir: /trabajo
          env: [{name: HOME, value: /trabajo}]
          volumeMounts: [{name: trabajo, mountPath: /trabajo}]
          resources: {requests: {cpu: 100m, memory: 768Mi}, limits: {cpu: "2", memory: 4Gi}}
          resizePolicy: [{resourceName: memory, restartPolicy: NotRequired}, {resourceName: cpu, restartPolicy: NotRequired}]
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
T_ADM=""; T_POD=""; T_NODO=""; T_CORRE=""; T_LISTO=""; POD=""
while :; do
  ahora=$(ms)
  [ -z "$T_ADM" ] && kubectl -n $NS get workloads -o jsonpath='{range .items[*]}{.metadata.ownerReferences[0].name} {.status.admission.clusterQueue}{"\n"}{end}' 2>/dev/null | grep -q "^$N cq-" && T_ADM=$ahora
  [ -z "$POD" ] && POD=$(kubectl -n $NS get pods -l job-name=$N -o name 2>/dev/null | head -1)
  if [ -n "$POD" ]; then
    [ -z "$T_POD" ] && [ -n "$(kubectl -n $NS get $POD -o jsonpath='{.spec.nodeName}' 2>/dev/null)" ] && T_POD=$ahora
    [ -z "$T_NODO" ] && [ "$MODO" = frio ] && [ "$(kubectl get nodes -l ore.dev/pool=sesiones --no-headers 2>/dev/null | grep -c ' Ready')" -ge 1 ] && T_NODO=$ahora
    [ -z "$T_CORRE" ] && [ -n "$(kubectl -n $NS get $POD -o jsonpath='{.status.containerStatuses[0].state.running.startedAt}{.status.containerStatuses[0].state.terminated.startedAt}' 2>/dev/null)" ] && T_CORRE=$ahora
    [ -z "$T_LISTO" ] && kubectl -n $NS logs $POD 2>/dev/null | grep -q '^listo' && { T_LISTO=$ahora; break; }
  fi
  [ $(( ahora - T0 )) -gt 600000 ] && { echo "⛔ pasó de 10 min"; break; }
  sleep 1
done
s() { [ -n "$1" ] && python3 -c "print(round(($1 - $T0)/1000, 1))" || echo null; }
NODO=$(kubectl -n $NS get $POD -o jsonpath='{.spec.nodeName}' 2>/dev/null)
echo "{\"caso\": \"s1-apertura\", \"modo\": \"$MODO\", \"job\": \"$N\", \"nodo\": \"$NODO\", \"admitido_s\": $(s "$T_ADM"), \"nodo_listo_s\": $(s "$T_NODO"), \"asignado_s\": $(s "$T_POD"), \"contenedor_s\": $(s "$T_CORRE"), \"listo_s\": $(s "$T_LISTO")}" | tee -a "$SAL"
kubectl -n $NS delete job $N --wait=false >/dev/null 2>&1
