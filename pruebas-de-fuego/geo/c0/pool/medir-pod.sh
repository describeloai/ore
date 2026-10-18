#!/usr/bin/env bash
# ORE 0068 C0·1–C0·3 a escala, dentro del pod c0 (un n2-standard-16 spot: 16 vCPU, 64 GB).
#   kubectl -n c0-medida exec c0 -- bash /trabajo/c0/pool/medir-pod.sh [fase...]
# Fases: banco sf10 sf100 duckdb spark (por defecto, todas, en ese orden). Cada motor, solo en el nodo,
# con 48 GB y el límite de SpatialBench (1 200 s). Resultados en /trabajo/resultados/<fecha>-pool.jsonl
# (escritos desde fuera del motor) y copia en gs://…-banco-geo/computo/c0/.
set -uo pipefail
T=/trabajo
export C0_EXT=$T/ext C0_MEMORIA=48GB SB_QUERIES=$T/sb-queries HOME=$T/home
SAL=$T/resultados/$(date +%Y%m%d)-pool.jsonl
LIM=1200
mkdir -p $T/resultados
fases=${*:-banco sf10 sf100 duckdb spark}
marca() { echo "[$(date +%H:%M:%S)] $*"; }

for f in $fases; do case $f in
  banco)
    marca "banco (7 días de AIS)"
    python3 $T/c0/preparar-banco.py $T/ov $T/ov/ais $T/banco && rm -rf $T/banco/tmp ;;
  sf10|sf100)
    s=${f#sf}
    [ -d $T/sb/$f ] || { marca "SpatialBench escala $s"; ( time $T/cargo/bin/spatialbench-cli -s $s --format=parquet --output-dir $T/sb/$f ) 2>&1 | tail -4; }
    du -sh $T/sb/$f/* ;;
  duckdb)
    for d in sb/sf10 sb/sf100; do marca "duckdb $d"; python3 $T/c0/correr.py duckdb spatialbench $T/$d - $LIM | tee -a $SAL; done
    marca "duckdb banco"; python3 $T/c0/correr.py duckdb banco $T/banco - $LIM | tee -a $SAL ;;
  spark)
    CP=$(ls $T/ivy/jars/*.jar | tr '\n' ':')
    chispa() {
      /opt/spark/bin/spark-submit --master 'local[16]' --driver-memory 48g \
        --driver-class-path "$CP" --conf spark.executor.extraClassPath="$CP" \
        --conf spark.local.dir=$T/spark-tmp --conf spark.sql.shuffle.partitions=64 \
        --conf spark.sql.extensions=org.apache.sedona.sql.SedonaSqlExtensions \
        --conf spark.serializer=org.apache.spark.serializer.KryoSerializer \
        --conf spark.kryo.registrator=org.apache.sedona.core.serde.SedonaKryoRegistrator \
        $T/c0/correr.py spark "$@" 2>&1 | grep --line-buffered '^{' | tee -a $SAL
    }
    # Un proceso por consulta: si una mata al driver (Q10 en el laboratorio), las demás siguen.
    for d in sb/sf10 sb/sf100; do
      for q in q1 q2 q2d q3 q4 q5 q6 q7 q8 q9 q10 q11 q12; do marca "spark $d $q"; chispa spatialbench $T/$d - $LIM $q; done
    done
    for q in b1 b2 b2d b3 b3d b4; do marca "spark banco $q"; chispa banco $T/banco - $LIM $q; done ;;
esac; done

marca "copia al bucket"
TOK=$(curl -s -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token | python3 -c "import json,sys;print(json.load(sys.stdin)['access_token'])")
curl -sf -X POST -H "Authorization: Bearer $TOK" -H "Content-Type: application/x-ndjson" --data-binary @$SAL \
  "https://storage.googleapis.com/upload/storage/v1/b/project-8853a180-450d-47be-b83-banco-geo/o?uploadType=media&name=computo/c0/$(basename $SAL)" >/dev/null && echo copiado
marca fin
