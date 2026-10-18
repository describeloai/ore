#!/usr/bin/env bash
# ORE 0068 C1·3 · El SDK de la JVM DEL ÁRBOL, compilado y probado dentro de la imagen del puesto JVM
# (réplica de laboratorio, jvm-lab/Dockerfile): los jars de jars.txt, el DuckDB JDBC y spatial/h3 en
# /opt/ore/duckdb, como en producción.
#   docker build -f pruebas-de-fuego/geo/c1/jvm-lab/Dockerfile -t ore-puesto-jvm:local .
#   bash pruebas-de-fuego/geo/c1/jvm-en-la-imagen.sh [ore.GeoPrueba | ore.SondaJvm]
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../.." && pwd -W 2>/dev/null || pwd)"
clase="${1:-ore.GeoPrueba}"
MSYS_NO_PATHCONV=1 docker run --rm -u 0 -v "$raiz":/repo:ro -w /repo --entrypoint bash ${IMAGEN:-ore-puesto-jvm:local} -c "
  set -e
  mkdir -p /tmp/c
  javac -Xlint:-options -nowarn --release 21 -cp '/opt/ore/lib/*' -d /tmp/c \
    /repo/puesto/jvm/ore/*.java /repo/puesto/jvm/pruebas/GeoPrueba.java /repo/pruebas-de-fuego/geo/c1/SondaJvm.java 2>&1 | grep -v '^Note:' || true
  java --add-opens=java.base/java.nio=ALL-UNNAMED -cp '/tmp/c:/opt/ore/lib/*' $clase"
