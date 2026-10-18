#!/usr/bin/env bash
# ORE 0068 C0·0 · Sedona 1.9.1 sobre Spark 4.1 en local, con los jars en el classpath de la aplicación
# (Spark 4.1 trae jts-core-1.20.0 en /opt/spark/jars; por --packages Sedona va a otro cargador y su
# IndexSerde, en el paquete de JTS, no llega a lo protegido: IllegalAccessError en el join).
#   bash pruebas-de-fuego/geo/c0/sedona-en-spark41.sh [fichero.sql]
set -euo pipefail
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
sql="${1:-sedona-en-spark41.sql}"
IMG=apache/spark:4.1.0-scala2.13-java17-python3-ubuntu
PAQ=org.apache.sedona:sedona-spark-shaded-4.1_2.13:1.9.1,org.datasyslab:geotools-wrapper:1.9.1-33.5
MSYS_NO_PATHCONV=1 docker run --rm -u 0 -v "$aqui":/c0 -v c0-ivy:/ivy -e HOME=/tmp "$IMG" bash -c "
  set -e
  # 1 · resolver los jars una vez (caché en el volumen c0-ivy)
  if ! ls /ivy/jars/*sedona-spark-shaded* >/dev/null 2>&1; then
    /opt/spark/bin/spark-sql -S --packages $PAQ --conf spark.jars.ivy=/ivy -e 'select 1' >/dev/null 2>&1
  fi
  CP=\$(ls /ivy/jars/*.jar | tr '\n' ':')
  # 2 · correr con los jars en el classpath de la aplicación
  /opt/spark/bin/spark-sql -S \
    --driver-class-path \"\$CP\" --conf spark.executor.extraClassPath=\"\$CP\" \
    --conf spark.sql.extensions=org.apache.sedona.sql.SedonaSqlExtensions \
    --conf spark.serializer=org.apache.spark.serializer.KryoSerializer \
    --conf spark.kryo.registrator=org.apache.sedona.core.serde.SedonaKryoRegistrator \
    -f /c0/$sql
" 2>&1 | grep -v -E '^\s*$|^[0-9/: ]+WARN '
