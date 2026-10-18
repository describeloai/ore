#!/usr/bin/env bash
# ORE 0068 C0·3 · Spark 4.1 + Sedona 1.9.1 a escala en el pod c0, una consulta por proceso.
# A escala 100 se salta lo que ya agotó el límite a escala 10 (no va a caber en 20 min con 10× datos).
#   kubectl -n c0-medida exec c0 -- bash -c 'nohup bash /trabajo/c0/pool/spark-escala.sh > /trabajo/spark.log 2>&1 &'
set -uo pipefail
T=/trabajo
export SB_QUERIES=$T/sb-queries HOME=$T/home
SAL=$T/resultados/$(date +%Y%m%d)-pool.jsonl
LIM=1200
CP=$(ls $T/ivy/jars/*.jar | tr '\n' ':')
marca() { echo "[$(date +%H:%M:%S)] $*"; }
chispa() {
  /opt/spark/bin/spark-submit --master 'local[16]' --driver-memory 48g \
    --driver-class-path "$CP" --conf spark.executor.extraClassPath="$CP" \
    --conf spark.local.dir=$T/spark-tmp --conf spark.sql.shuffle.partitions=64 \
    --conf spark.sql.extensions=org.apache.sedona.sql.SedonaSqlExtensions \
    --conf spark.serializer=org.apache.spark.serializer.KryoSerializer \
    --conf spark.kryo.registrator=org.apache.sedona.core.serde.SedonaKryoRegistrator \
    $T/c0/correr.py spark "$@" 2>&1 | grep --line-buffered '^{' | tee -a $SAL
}
agoto() { # ¿la consulta $1 agotó el límite a escala 10 en spark?
  grep '"motor": "spark"' $SAL | grep '"datos": "/trabajo/sb/sf10"' | grep -q "\"caso\": \"$1\", \"estado\": \"limite\""
}
for q in b1 b2 b2d b3 b3d b4; do marca "spark banco $q"; chispa banco $T/banco - $LIM $q; done
for q in q1 q2 q2d q3 q4 q5 q6 q7 q8 q9 q10 q11 q12; do marca "spark sb/sf10 $q"; chispa spatialbench $T/sb/sf10 - $LIM $q; done
for q in q1 q2 q2d q3 q4 q5 q6 q7 q8 q9 q10 q11 q12; do
  if agoto $q; then marca "spark sb/sf100 $q: se salta (agotó a escala 10)"; continue; fi
  marca "spark sb/sf100 $q"; chispa spatialbench $T/sb/sf100 - $LIM $q
done
marca "copia al bucket"
TOK=$(curl -s -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token | python3 -c "import json,sys;print(json.load(sys.stdin)['access_token'])")
curl -sf -X POST -H "Authorization: Bearer $TOK" -H "Content-Type: application/x-ndjson" --data-binary @$SAL \
  "https://storage.googleapis.com/upload/storage/v1/b/project-8853a180-450d-47be-b83-banco-geo/o?uploadType=media&name=computo/c0/$(basename $SAL)" >/dev/null && echo copiado
marca fin
