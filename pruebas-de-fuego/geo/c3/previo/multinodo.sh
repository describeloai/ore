#!/usr/bin/env bash
# ORE 0068 C3 · previo · Spark + Sedona en N nodos del pool `pesado`, como podría correr un Build pesado
# repartido: UN Job indexado de N pods (uno por nodo) y un Service sin IP para que se encuentren. El pod 0
# lleva el master *standalone* y el driver (`multinodo.py`); todos llevan un worker. `successPolicy` da el
# Job por terminado cuando acaba el pod 0, y Kubernetes cierra los demás. En el namespace de medida
# `c3-medida` (fuera de la cuota de t-victor y sin Kueue), que se borra al acabar con `BORRAR=1`.
#   bash pruebas-de-fuego/geo/c3/previo/multinodo.sh 2 [PUNTOS=200000000 …]
set -uo pipefail
NS=c3-medida
aqui="$(cd "$(dirname "$0")" && pwd)"
NODOS=${1:?cuántos nodos}; shift
J=mn$NODOS-$(date +%H%M%S)
IMAGEN=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-spark:1
ENV=""
for kv in "$@"; do ENV="$ENV
            - {name: ${kv%%=*}, value: \"${kv#*=}\"}"; done
kubectl get ns $NS >/dev/null 2>&1 || kubectl create ns $NS >/dev/null
kubectl -n $NS create configmap $J --from-file=prog.py="$aqui/multinodo.py" >/dev/null
T0=$(date -u +%s)
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: v1
kind: Service
metadata: {name: $J, namespace: $NS}
spec:
  clusterIP: None
  publishNotReadyAddresses: true
  selector: {c3: $J}
  ports: [{name: master, port: 7077}, {name: driver, port: 7078}, {name: bloques, port: 7079}]
---
apiVersion: batch/v1
kind: Job
metadata: {name: $J, namespace: $NS}
spec:
  completionMode: Indexed
  completions: $NODOS
  parallelism: $NODOS
  successPolicy: {rules: [{succeededIndexes: "0"}]}
  backoffLimit: 0
  activeDeadlineSeconds: 2400
  ttlSecondsAfterFinished: 600
  template:
    metadata: {labels: {c3: $J}}
    spec:
      subdomain: $J
      restartPolicy: Never
      automountServiceAccountToken: false
      securityContext: {runAsNonRoot: true, runAsUser: 65532, seccompProfile: {type: RuntimeDefault}}
      nodeSelector: {ore.dev/pool: pesado}
      tolerations: [{key: ore.dev/pesado, operator: Equal, value: "true", effect: NoSchedule}]
      affinity:
        podAntiAffinity:
          requiredDuringSchedulingIgnoredDuringExecution:
            - {labelSelector: {matchLabels: {c3: $J}}, topologyKey: kubernetes.io/hostname}
      volumes: [{name: guion, configMap: {name: $J}}]
      containers:
        - name: spark
          image: $IMAGEN
          imagePullPolicy: Always
          command: ["bash", "-c"]
          args:
            - |
              set -u
              YO=\$(hostname).$J.$NS.svc.cluster.local
              M=spark://$J-0.$J.$NS.svc.cluster.local:7077
              export SPARK_WORKER_DIR=/tmp/work SPARK_LOCAL_DIRS=/tmp/spark SPARK_NO_DAEMONIZE=1
              mkdir -p /tmp/work /tmp/spark
              if [ "\$JOB_COMPLETION_INDEX" = 0 ]; then
                spark-class org.apache.spark.deploy.master.Master --host \$YO --port 7077 --webui-port 8080 > /tmp/master.log 2>&1 &
                sleep 5
              fi
              # Que el nombre del master resuelva antes de arrancar el worker.
              for i in \$(seq 1 60); do getent hosts $J-0.$J.$NS.svc.cluster.local >/dev/null && break; sleep 2; done
              spark-class org.apache.spark.deploy.worker.Worker \$M --host \$YO --cores 7 --memory 40g > /tmp/worker.log 2>&1 &
              if [ "\$JOB_COMPLETION_INDEX" = 0 ]; then
                MASTER=\$M YO=\$YO NODOS=$NODOS python3 /m/prog.py 2>/tmp/driver.log | grep --line-buffered '^C3|'
                e=\${PIPESTATUS[0]}
                [ \$e = 0 ] || tail -20 /tmp/driver.log | grep -v '^\s*at '
                exit \$e
              fi
              wait
          env:
            - {name: HOME, value: /tmp}$ENV
          volumeMounts: [{name: guion, mountPath: /m}]
          workingDir: /tmp
          resources:
            requests: {cpu: "7", memory: 48Gi}
            limits: {cpu: "8", memory: 48Gi}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
echo "== $J ($NODOS nodos) creado · nodos pesados: $(kubectl get nodes -l ore.dev/pool=pesado --no-headers 2>/dev/null | wc -l)"
while :; do
  c=$(kubectl -n $NS get job $J -o jsonpath='{range .status.conditions[*]}{.type}={.status} {end}' 2>/dev/null)
  case "$c" in *Complete=True*|*Failed=True*|*SuccessCriteriaMet=True*) break;; esac
  [ $(( $(date -u +%s) - T0 )) -gt 2600 ] && { echo "⛔ pasó de 43 min"; break; }
  sleep 10
done
echo "== $c · $(( $(date -u +%s) - T0 )) s desde que se creó"
P0=$(kubectl -n $NS get pods -l c3=$J -o jsonpath='{range .items[?(@.metadata.annotations.batch\.kubernetes\.io/job-completion-index=="0")]}{.metadata.name}{end}')
kubectl -n $NS logs $P0 2>/dev/null | grep -v '^\s*at ' | tail -20
kubectl -n $NS delete job,svc,configmap $J --wait=false >/dev/null 2>&1
[ "${BORRAR:-0}" = 1 ] && kubectl delete ns $NS --wait=false >/dev/null && echo "== $NS borrado"
