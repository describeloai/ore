#!/usr/bin/env bash
# ORE 0068 C3·b · Compila `puesto/spark/ReglasDeOre.java` en la imagen pesada (como la etapa del Dockerfile) y
# corre `regla.py` con ella, sin red y con pocos datos (cabe en el WSL de 6 GB).
#   bash pruebas-de-fuego/geo/c3/b/regla.sh
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
IMAGEN="${IMAGEN:-ore-puesto-spark:local}"
MSYS_NO_PATHCONV=1 docker run --rm --network none -v "$raiz/puesto/spark":/src:ro -v "$aqui":/sal -u 0 "$IMAGEN" \
  sh -c 'mkdir -p /tmp/r && javac -proc:none -Xlint:-options --release 21 -cp "$SPARK_HOME/jars/*" -d /tmp/r \
         /src/ReglasDeOre.java && jar cf /sal/ore-spark-reglas.jar -C /tmp/r .'
MSYS_NO_PATHCONV=1 docker run --rm --network none --hostname localhost --memory 4g -v "$aqui":/l:ro -e HOME=/tmp \
  -w /tmp "$IMAGEN" python3 /l/regla.py 2>/dev/null
