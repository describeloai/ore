#!/usr/bin/env bash
# ORE 0068 C2·4 · Spark 4.1 + Iceberg 1.12 contra el /v1 de un ore-serve de verdad, en Docker.
#   1 · los binarios (una vez): cargo build --release -p ore-cli --bin ore -p ore-serve --bin ore-serve
#       -p ore-store --bin ore-store-r2   (en rust:1, con el volumen ore-target)
#   2 · bash pruebas-de-fuego/geo/c2/spark-por-v1/medir.sh [fichero.sql]
#       SEGUIR=1  no rehace el banco (para un .sql que lee lo que dejó otro)
#       SEDONA=1  con Sedona 1.9.1 (sedona.sql)
# El banco (rust:1, banco.sh) y Spark en la MISMA red (la del banco): todo en 127.0.0.1.
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
sql="${1:-spark.sql}"
ICEBERG=1.12.0
PAQUETES="org.apache.iceberg:iceberg-spark-runtime-4.1_2.13:$ICEBERG,org.apache.iceberg:iceberg-aws-bundle:$ICEBERG"
EXTENSIONES="org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions"
if [ "${SEDONA:-}" = 1 ]; then
  PAQUETES="$PAQUETES,org.apache.sedona:sedona-spark-shaded-4.1_2.13:1.9.1,org.datasyslab:geotools-wrapper:1.9.1-33.5"
  EXTENSIONES="$EXTENSIONES,org.apache.sedona.sql.SedonaSqlExtensions"
fi

if [ "${SEGUIR:-}" != 1 ]; then
  docker rm -f c2-banco >/dev/null 2>&1 || true
  MSYS_NO_PATHCONV=1 docker run -d --name c2-banco -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
    bash /repo/pruebas-de-fuego/geo/c2/spark-por-v1/banco.sh >/dev/null
  for i in $(seq 1 120); do docker logs c2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
  docker logs c2-banco 2>&1 | grep -q "banco listo" || { docker logs c2-banco 2>&1 | tail -30; exit 1; }
fi
echo "== el banco: ore-serve en 127.0.0.1:8080, el S3 de mentira en :9000"

echo "== Spark 4.1 + Iceberg $ICEBERG por /v1 ($sql)"
MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco -u 0 -e HOME=/tmp \
  -v "$raiz/pruebas-de-fuego/geo/c2/spark-por-v1":/c2:ro -v c2-ivy:/ivy \
  apache/spark:4.1.0-scala2.13-java17-python3-ubuntu /opt/spark/bin/spark-sql -S \
  --packages "$PAQUETES" \
  --conf spark.jars.ivy=/ivy \
  --conf "spark.sql.extensions=$EXTENSIONES" \
  --conf spark.serializer=org.apache.spark.serializer.KryoSerializer \
  --conf spark.sql.catalog.ore=org.apache.iceberg.spark.SparkCatalog \
  --conf spark.sql.catalog.ore.type=rest \
  --conf spark.sql.catalog.ore.uri=http://127.0.0.1:8080 \
  --conf spark.sql.catalog.ore.header.x-ore-sujeto=persona:ana \
  --conf spark.sql.catalog.ore.header.X-Iceberg-Access-Delegation=vended-credentials \
  --conf spark.sql.catalog.ore.io-impl=org.apache.iceberg.aws.s3.S3FileIO \
  --conf spark.sql.catalog.ore.s3.path-style-access=true \
  --conf spark.sql.catalog.ore.client.region=auto \
  -f "/c2/$sql" > "$(dirname "$0")/ultimo.log" 2>&1 || true
# Lo que dice Spark, sin las trazas (enteras en ultimo.log).
grep -v -E "^\s*$|^\s+at |^\s*\.\.\. [0-9]+ more|WARN|SLF4J|incubator|resolving|found org|downloading|:: |artifacts|confs|SUCCESSFUL|from central|Ivy|added as|jars for|log4j|setLogLevel|^\s*-+$|modules|^\s*\|" "$(dirname "$0")/ultimo.log" | cut -c1-300 | head -40

for t in lugares zonas nativa; do
  echo "== lo que ORE ve de geo.$t"
  MSYS_NO_PATHCONV=1 docker exec c2-banco bash -c "curl -s -H 'x-ore-sujeto: persona:ana' http://127.0.0.1:8080/v1/namespaces/geo/tables/$t" \
    | python3 -c "
import json, sys
j = json.load(sys.stdin)
if 'error' in j:
    print('  ', j['error'].get('code'), str(j['error'].get('message'))[:200]); sys.exit()
m = j.get('metadata') or {}
s = next((x for x in m.get('schemas', []) if x.get('schema-id') == m.get('current-schema-id')), {})
print('  v%s' % m.get('format-version'), '· snapshots', len(m.get('snapshots') or []), '· esquema', [(f['name'], f['type']) for f in s.get('fields', [])])
print('  ore.*', {k: v for k, v in (m.get('properties') or {}).items() if k.startswith('ore.')})
"
done
echo "== el log de ore-serve (los errores)"
MSYS_NO_PATHCONV=1 docker exec c2-banco bash -c "grep -i -E 'error|fall|409|500' /tmp/banco/serve.log | tail -8 | cut -c1-240" || true
