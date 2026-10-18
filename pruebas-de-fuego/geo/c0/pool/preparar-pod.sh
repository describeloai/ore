#!/usr/bin/env bash
# ORE 0068 C0 · Dentro del pod c0: DuckDB 1.5.6 con spatial y h3, los jars de Sedona, el generador de
# SpatialBench y los datos del banco desde el bucket (con la identidad del pod).
#   kubectl -n c0-medida exec c0 -- bash /trabajo/c0/pool/preparar-pod.sh
set -euo pipefail
T=/trabajo
B=project-8853a180-450d-47be-b83-banco-geo
mkdir -p $T/home $T/ext $T/ivy $T/ov/ais $T/banco $T/sb
cd $T

echo "== DuckDB 1.5.6"
pip install -q duckdb==1.5.6 2>&1 | tail -1
C0_EXT=$T/ext python3 -c "
import duckdb, os
c = duckdb.connect(); c.execute(\"set extension_directory = '$T/ext'\")
c.execute('install spatial'); c.execute('install h3 from community')
c.execute('load spatial'); c.execute('load h3'); print('spatial y h3', duckdb.__version__)"

echo "== Sedona 1.9.1"
if ! ls $T/ivy/jars/*sedona-spark-shaded* >/dev/null 2>&1; then
  /opt/spark/bin/spark-sql -S --packages org.apache.sedona:sedona-spark-shaded-4.1_2.13:1.9.1,org.datasyslab:geotools-wrapper:1.9.1-33.5 \
    --conf spark.jars.ivy=$T/ivy -e 'select 1' >/dev/null 2>&1
fi
ls $T/ivy/jars | wc -l

echo "== SpatialBench"
if [ ! -x $T/cargo/bin/spatialbench-cli ]; then
  apt-get update -qq && apt-get install -y -qq build-essential pkg-config libssl-dev >/dev/null
  curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal >/dev/null
  $T/home/.cargo/bin/cargo install -q spatialbench-cli@0.2.0 --root $T/cargo
fi
$T/cargo/bin/spatialbench-cli --help | head -1

echo "== El banco desde el bucket"
TOK=$(curl -s -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token | python3 -c "import json,sys;print(json.load(sys.stdin)['access_token'])")
baja() { # objeto destino
  [ -s "$2" ] || curl -sf -H "Authorization: Bearer $TOK" -o "$2" \
    "https://storage.googleapis.com/storage/v1/b/$B/o/$(python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1")?alt=media"
}
for t in edificios_es lugares_es divisiones_es; do baja vectorial/overture/2026-09-23.1/$t.parquet $T/ov/$t.parquet; done
for d in 07 08 09 10 11 12 13; do
  f=$T/ov/ais/aisdk-2026-10-$d.csv
  [ -s $f ] || { baja movimiento/ais-dk/zip/aisdk-2026-10-$d.zip $T/ov/ais/z.zip && (cd $T/ov/ais && python3 -c "import zipfile;zipfile.ZipFile('z.zip').extractall('.')" && rm z.zip); }
done
ls -la $T/ov $T/ov/ais
