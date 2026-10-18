#!/usr/bin/env bash
# ORE 0068 C0 · La vara de cómputo en el laboratorio local (Docker), con las mismas imágenes que prod.
#   bash correr.sh <duckdb|spark> <spatialbench|banco> <volumen:subdir> [limite_s] [casos]
#   p. ej. bash correr.sh duckdb spatialbench c0-sb:sf1 600
#          bash correr.sh spark banco c0-banco: 600 b1,b2
# Añade las líneas JSON a resultados/<fecha>-local.jsonl desde aquí fuera (si el motor muere, lo medido queda). SB_REPO apunta al clon de SpatialBench.
set -euo pipefail
motor=$1; conjunto=$2; vol=${3%%:*}; sub=${3#*:}; limite=${4:-600}; casos=${5:-}
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
SB_REPO=${SB_REPO:?el clon de apache/sedona-spatialbench (commit 8e44bdd)}
mkdir -p "$(dirname "$0")/resultados"
sal="$(dirname "$0")/resultados/$(date +%Y%m%d)-local.jsonl"
comun=(--rm -u 0 -v "$aqui":/c0 -v "$vol":/datos:ro -v "$SB_REPO/spatialbench-queries":/sb-queries:ro)
if [ "$motor" = duckdb ]; then
  MSYS_NO_PATHCONV=1 docker run "${comun[@]}" -e C0_MEMORIA="${C0_MEMORIA:-4GB}" --entrypoint python \
    g235-puesto-python:latest /c0/correr.py duckdb "$conjunto" "/datos/$sub" - "$limite" "$casos" | tee -a "$sal"
else
  MSYS_NO_PATHCONV=1 docker run "${comun[@]}" -v c0-ivy:/ivy -e HOME=/tmp \
    apache/spark:4.1.0-scala2.13-java17-python3-ubuntu bash -c "
      CP=\$(ls /ivy/jars/*.jar | tr '\n' ':')
      /opt/spark/bin/spark-submit --master 'local[*]' --driver-memory ${C0_MEMORIA:-4g} \
        --driver-class-path \"\$CP\" --conf spark.executor.extraClassPath=\"\$CP\" \
        --conf spark.sql.extensions=org.apache.sedona.sql.SedonaSqlExtensions \
        --conf spark.serializer=org.apache.spark.serializer.KryoSerializer \
        --conf spark.kryo.registrator=org.apache.sedona.core.serde.SedonaKryoRegistrator \
        /c0/correr.py spark $conjunto /datos/$sub - $limite '$casos'" 2>&1 | grep -v -E 'WARN|SLF4J|incubator' \
    | tee >(grep --line-buffered '^{' >> "$sal")
fi
