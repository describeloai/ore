#!/usr/bin/env bash
# Query Q0 · Una medida en victor, como Job de Kueue en t-victor, con la seguridad de un puesto:
#   sesion   → `puesto-python:1` en `cola` con los recursos de una sesión (2 CPU / 4 GiB, DuckDB 2 GB)
#   grande   → `puesto-spark:1` en `cola-pesada` con los de un pod pesado (7–8 CPU / 48 GiB, DuckDB 38 GB, SSD)
#   dialecto → `puesto-spark:1` en `cola`: el corpus de SQL en DuckDB y, traducido con sqlglot, en Spark local
#   q0b      → `puesto-spark:1` en `cola-pesada`: un trabajador compartido (q0b.py) y Spark con identidad por sesión (q0b_spark.py)
#   arranque → lo mismo, vacío: cuánto tarda un trabajador en servir (nodo caliente o frío)
#   q1       → la sesión nueva de Q1 (`/trabajo` con tope, `ORE_DISCO_MB`) con el SDK de la rama montado encima
#   connect  → `puesto-spark:1` como el driver de un Build elástico (`driver-pesado`, `cola-driver`)
#              con un servidor Spark Connect dentro y sus ejecutores en `cola-pesada` (techo 2)
# Dice cuándo pasa cada cosa (admitido, nodo, imagen, arranque, primera línea, fin), guarda las líneas
# JSON en resultados/ y borra el Job y su ConfigMap al acabar.
#   bash pruebas-de-fuego/query/medir.sh sesion|grande|connect [VAR=valor ...]
set -uo pipefail
NS=t-victor
aqui="$(cd "$(dirname "$0")" && pwd)"
ESC=${1:?sesion|grande|connect}; shift
R=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore
PY=$R/puesto-python@sha256:d425862b4b5bb59710004f6ac07dec3c167b4f75163d92c5a72c1bb4756edcc4
SP=$R/puesto-spark@sha256:b9b961f163bf5a76148e0184e6446a7a18683b1eefc80f42fe5cf56f315bf830
N=query-q0-$ESC-$(date +%H%M%S)
SAL="$aqui/resultados/$(date +%Y%m%d)-$ESC.jsonl"
mkdir -p "$aqui/resultados"
t() { date -u +%s; }

extra=""
for kv in "$@"; do extra="$extra            - {name: ${kv%%=*}, value: \"${kv#*=}\"}
"; done

case $ESC in
  sesion)  COLA=cola;        IMG=$PY; SA=puesto;        ETQ="ore.dev/rol: puesto"
           REC='requests: {cpu: "100m", memory: 768Mi}
            limits: {cpu: "2", memory: 4Gi}'
           ENV='- {name: ESCALON, value: sesion}
            - {name: MEMORIA_MB, value: "2048"}'
           CMD='["python3", "/m/consultas.py"]'; POOL=jobs ;;
  grande)  COLA=cola-pesada; IMG=$SP; SA=puesto;        ETQ="ore.dev/rol: puesto"
           REC='requests: {cpu: "7", memory: 48Gi, ephemeral-storage: 250Gi}
            limits: {cpu: "8", memory: 48Gi, ephemeral-storage: 250Gi}'
           ENV='- {name: ESCALON, value: grande}
            - {name: MEMORIA_MB, value: "38912"}'
           CMD='["python3", "/m/consultas.py"]'; POOL=pesado ;;
  connect) COLA=cola-driver; IMG=$SP; SA=driver-pesado; ETQ='ore.dev/rol: puesto
        ore.dev/spark: driver'
           REC='requests: {cpu: "1500m", memory: 12Gi}
            limits: {cpu: "1500m", memory: 12Gi}'
           ENV="- {name: ORE_MEMORIA_MB, value: \"12288\"}
            - {name: ORE_SPARK_ELASTICO, value: \"2\"}
            - {name: ORE_SPARK_IMAGEN, value: \"$SP\"}
            - {name: ORE_POD_NOMBRE, valueFrom: {fieldRef: {fieldPath: metadata.name}}}
            - {name: ORE_POD_IP, valueFrom: {fieldRef: {fieldPath: status.podIP}}}"
           CMD='["python3", "/m/connect.py"]'; POOL=pesado-driver ;;
  dialecto) COLA=cola;       IMG=$SP; SA=puesto;        ETQ="ore.dev/rol: puesto"
           REC='requests: {cpu: "100m", memory: 768Mi}
            limits: {cpu: "2", memory: 4Gi}'
           ENV='- {name: BAJAR, value: "1"}'
           CMD='["python3", "/m/dialecto.py", "/trabajo/dialecto", "--ejecutar"]'; POOL=jobs ;;
  q0b)     COLA=cola-pesada; IMG=$SP; SA=puesto;        ETQ="ore.dev/rol: puesto"
           REC='requests: {cpu: "7", memory: 48Gi, ephemeral-storage: 250Gi}
            limits: {cpu: "8", memory: 48Gi, ephemeral-storage: 250Gi}'
           ENV='- {name: TRABAJO, value: /trabajo}'
           CMD='["sh", "-c", "python3 /m/q0b.py; python3 /m/q0b_spark.py"]'; POOL=pesado ;;
  arranque) COLA=cola-pesada; IMG=$SP; SA=puesto;       ETQ="ore.dev/rol: puesto"
           REC='requests: {cpu: "7", memory: 48Gi, ephemeral-storage: 250Gi}
            limits: {cpu: "8", memory: 48Gi, ephemeral-storage: 250Gi}'
           ENV='- {name: TRABAJO, value: /trabajo}'
           CMD='["python3", "-c", "import duckdb, json; c = duckdb.connect(); print(json.dumps({\"evento\": \"listo\", \"duckdb\": duckdb.__version__}))"]'; POOL=pesado ;;
  q1)      COLA=cola;        IMG=$PY; SA=puesto;        ETQ="ore.dev/rol: puesto"
           REC='requests: {cpu: "100m", memory: 768Mi}
            limits: {cpu: "2", memory: 4Gi}'
           ENV='- {name: ORE_MEMORIA_MB, value: "4096"}
            - {name: ORE_DISCO_MB, value: "4096"}'
           VOL='{sizeLimit: 4Gi}'
           MONTA_SDK=1
           CMD='["python3", "/m/q1.py"]'; POOL=jobs ;;
  *) echo "¿$ESC?"; exit 2 ;;
esac

MONTAJE=""; [ -n "${MONTA_SDK:-}" ] && MONTAJE=", {name: guion, mountPath: /opt/ore/ore/__init__.py, subPath: ore_init.py}"
SDK_INIT=${SDK_INIT:-/c/ORE-q1/puesto/python/ore/__init__.py}
kubectl -n $NS create configmap $N --from-file=ore_init.py="$SDK_INIT" --from-file=q1.py="$aqui/q1.py" --from-file=consultas.py="$aqui/consultas.py" --from-file=connect.py="$aqui/connect.py" --from-file=dialecto.py="$aqui/dialecto.py" --from-file=idiomas.sql="$aqui/idiomas.sql" --from-file=q0b.py="$aqui/q0b.py" --from-file=q0b_spark.py="$aqui/q0b_spark.py" >/dev/null
NODOS0=$(kubectl get nodes -l ore.dev/pool=$POOL --no-headers 2>/dev/null | wc -l)
T0=$(t)
cat <<YAML | kubectl create -f - >/dev/null || exit 1
apiVersion: batch/v1
kind: Job
metadata:
  name: $N
  namespace: $NS
  labels: {kueue.x-k8s.io/queue-name: $COLA, ore.dev/medida: query-q0}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 7200
  ttlSecondsAfterFinished: 900
  template:
    metadata:
      labels:
        ore.dev/medida: query-q0
        $ETQ
    spec:
      restartPolicy: Never
      serviceAccountName: $SA
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      volumes:
        - {name: guion, configMap: {name: $N}}
        - {name: trabajo, emptyDir: ${VOL:-{\}}}
      containers:
        - name: medida
          image: $IMG
          imagePullPolicy: IfNotPresent
          command: $CMD
          workingDir: /trabajo
          env:
            - {name: HOME, value: /trabajo}
            - {name: DERRAME, value: /trabajo/derrame}
            $ENV
$extra          volumeMounts: [{name: guion, mountPath: /m}, {name: trabajo, mountPath: /trabajo}$MONTAJE]
          resources:
            $REC
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
echo "== $N · cola $COLA · nodos $POOL al empezar: $NODOS0"
T_ADM=""; T_NODO=""; T_CORRE=""; T_LINEA=""; POD=""
while :; do
  [ -z "$T_ADM" ] && kubectl -n $NS get workloads -o jsonpath='{range .items[*]}{.metadata.ownerReferences[0].name} {.status.admission.clusterQueue}{"\n"}{end}' 2>/dev/null | grep -q "^$N cq-" && T_ADM=$(t)
  [ -z "$T_NODO" ] && [ "$(kubectl get nodes -l ore.dev/pool=$POOL --no-headers 2>/dev/null | grep -c ' Ready')" -gt 0 ] && T_NODO=$(t)
  [ -z "$POD" ] && POD=$(kubectl -n $NS get pods -l job-name=$N -o name 2>/dev/null | head -1)
  if [ -n "$POD" ] && [ -z "$T_CORRE" ]; then
    [ "$(kubectl -n $NS get $POD -o jsonpath='{.status.phase}' 2>/dev/null)" = Running ] && T_CORRE=$(t)
  fi
  if [ -n "$T_CORRE" ] && [ -z "$T_LINEA" ]; then
    kubectl -n $NS logs $POD 2>/dev/null | grep -q '^{' && T_LINEA=$(t)
  fi
  e=$(kubectl -n $NS get job $N -o jsonpath='{.status.succeeded}/{.status.failed}' 2>/dev/null)
  case "$e" in 1/*|*/1) break;; esac
  [ $(( $(t) - T0 )) -gt 7300 ] && { echo "⛔ pasó de 2 h"; break; }
  sleep 3
done
T_FIN=$(t)
kubectl -n $NS logs $POD > "$aqui/resultados/$N.log" 2>&1
grep '^{' "$aqui/resultados/$N.log" >> "$SAL"
NODO=$(kubectl -n $NS get $POD -o jsonpath='{.spec.nodeName}' 2>/dev/null)
echo "{\"escalon\": \"$ESC\", \"evento\": \"arranque\", \"job\": \"$N\", \"nodos_al_empezar\": $NODOS0, \"nodo\": \"$NODO\", \"admitido_s\": $(( ${T_ADM:-$T0} - T0 )), \"nodo_listo_s\": $(( ${T_NODO:-$T0} - T0 )), \"corriendo_s\": $(( ${T_CORRE:-$T0} - T0 )), \"primera_linea_s\": $(( ${T_LINEA:-$T0} - T0 )), \"fin_s\": $(( T_FIN - T0 )), \"estado\": \"$e\"}" | tee -a "$SAL"
kubectl -n $NS get events --field-selector involvedObject.name=${POD#pod/} -o jsonpath='{range .items[*]}{.reason}|{.message}{"\n"}{end}' 2>/dev/null | grep -E "Pulled|Pulling|Scheduled|TriggeredScaleUp|FailedScheduling" | cut -c1-200
kubectl -n $NS delete job $N --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $N >/dev/null 2>&1
tail -3 "$aqui/resultados/$N.log"
