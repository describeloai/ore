# g0-iceberg-v3.sh (ADR 0067 G0·2): lanza g0-iceberg-v3.sql en Spark 4.1 + Iceberg 1.11 (Docker local),
# con GCSFileIO y el token de quien lo corre. Necesita /c/tmp/banco-geo/lug_wkb.parquet (g0-ficheros).
#   bash pruebas-de-fuego/geo/g0-iceberg-v3.sh
set -e
D=$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)
gcloud auth print-access-token > /c/tmp/banco-geo/.tk
trap 'rm -f /c/tmp/banco-geo/.tk' EXIT
MSYS_NO_PATHCONV=1 docker run --rm -u 0 -v "C:/tmp/banco-geo:/salida" -v "$D:/g:ro" \
  apache/spark:4.1.0-scala2.13-java17-python3-ubuntu sh -c '
  EXP=$(( ($(date +%s) + 3000) * 1000 ))
  /opt/spark/bin/spark-shell --conf spark.jars.ivy=/salida/ivy \
    --packages org.apache.iceberg:iceberg-spark-runtime-4.1_2.13:1.11.0,org.apache.iceberg:iceberg-gcp-bundle:1.11.0,org.xerial:sqlite-jdbc:3.46.1.3 \
    --conf spark.sql.extensions=org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions \
    --conf spark.sql.catalog.banco=org.apache.iceberg.spark.SparkCatalog \
    --conf spark.sql.catalog.banco.type=jdbc \
    --conf spark.sql.catalog.banco.uri=jdbc:sqlite:/salida/lakehouse-catalogo.db \
    --conf spark.sql.catalog.banco.warehouse=gs://project-8853a180-450d-47be-b83-banco-geo/lakehouse \
    --conf spark.sql.catalog.banco.io-impl=org.apache.iceberg.gcp.gcs.GCSFileIO \
    --conf spark.sql.catalog.banco.gcs.oauth2.token=$(cat /salida/.tk) \
    --conf spark.sql.catalog.banco.gcs.oauth2.token-expires-at=$EXP \
    --conf spark.sql.catalog.banco.gcs.project-id=project-8853a180-450d-47be-b83 \
    -I /g/g0-iceberg-v3.scala 2>&1 | grep -v " INFO \| WARN " | grep -E "RESULTADO|error|Exception" | head -20'
gcloud storage cp /c/tmp/banco-geo/lakehouse-catalogo.db gs://project-8853a180-450d-47be-b83-banco-geo/lakehouse/catalogo.db
