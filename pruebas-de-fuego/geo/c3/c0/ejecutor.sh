#!/bin/bash
# ORE 0068 C3·c0 · La entrada de un ejecutor de Spark en Kubernetes, en `puesto-spark` (que trae Spark por
# pip y no el `entrypoint.sh` de la distribución): la rama `executor` de ese guion (Spark 4.1.0), sin más.
# Spark pone las variables (SPARK_DRIVER_URL, SPARK_EXECUTOR_ID, …) y los SPARK_JAVA_OPT_<n> en el pod.
set -e
cd /tmp
CP="${SPARK_CONF_DIR:-/opt/spark/conf}:$SPARK_HOME/jars/*"
[ -n "${SPARK_CLASSPATH:-}" ] && CP="$CP:$SPARK_CLASSPATH"
mapfile -t OPTS < <(env | grep SPARK_JAVA_OPT_ | sort -t_ -k4 -n | sed 's/[^=]*=\(.*\)/\1/g')
exec "$JAVA_HOME/bin/java" "${OPTS[@]}" -Xms"$SPARK_EXECUTOR_MEMORY" -Xmx"$SPARK_EXECUTOR_MEMORY" -cp "$CP" \
  org.apache.spark.scheduler.cluster.k8s.KubernetesExecutorBackend \
  --driver-url "$SPARK_DRIVER_URL" --executor-id "$SPARK_EXECUTOR_ID" --cores "$SPARK_EXECUTOR_CORES" \
  --app-id "$SPARK_APPLICATION_ID" --hostname "$SPARK_EXECUTOR_POD_IP" \
  --resourceProfileId "$SPARK_RESOURCE_PROFILE_ID" --podName "$SPARK_EXECUTOR_POD_NAME"
