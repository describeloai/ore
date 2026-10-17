# ADR 0067 G2·5 · El arnés local de los lectores de JVM (Spark 4.1 + Iceberg 1.12, Iceberg Java genérico
# y, con IMAGEN=apache/sedona:1.9.1, Sedona). Para iterar; el veredicto es en victor.
#   bash pruebas-de-fuego/geo/v5/local-jvm.sh "$(cygpath -w pruebas-de-fuego/geo/v5)" [imagen] [runtime]
M=$1
IMAGEN=${2:-apache/spark:4.1.0-scala2.13-java17-python3-ubuntu}
RT=${3:-org.apache.iceberg:iceberg-spark-runtime-4.1_2.13:1.12.0}
for c in EPSG:25830 OGC:CRS84; do d=${c/:/-}
 v0=$(ls /c/tmp/v5/$d/tabla/ore/v2/v5/metadata/00000-*.json | xargs -n1 basename | sed 's/.metadata.json//')
 v1=$(ls /c/tmp/v5/$d/tabla/ore/v2/v5/metadata/00001-*.json | xargs -n1 basename | sed 's/.metadata.json//')
 MSYS_NO_PATHCONV=1 docker run --rm -u 0 --memory 4g -e ARTEFACTOS="A=/v5/$d/ore-geo.parquet B=/v5/$d/tabla/ore/v2/v5@$v0 C=/v5/$d/tabla/ore/v2/v5@$v1" \
   -v "C:/tmp/v5:/v5" -v "$M:/m" -v "C:/tmp/banco-geo/ivy:/ivy" --entrypoint sh "$IMAGEN" -c \
   "echo ':load /m/lectores-spark.scala' | /opt/spark/bin/spark-shell --driver-memory 2g --conf spark.jars.ivy=/ivy --packages $RT --conf spark.sql.geospatial.enabled=true 2>&1" \
   | grep -o 'V5R|.*' > /c/tmp/v5/jvm-$d.txt
 python /c/ORE-pg/pruebas-de-fuego/geo/v5/juzgar.py /c/ORE-pg/pruebas-de-fuego/geo/v5/fijos.json $c < /c/tmp/v5/jvm-$d.txt | cut -c1-250
done
