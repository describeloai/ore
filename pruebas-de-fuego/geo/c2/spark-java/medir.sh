#!/usr/bin/env bash
# ORE 0068 C2·3 pieza 4 · El previo de Spark Java: `previo.py` en la imagen pesada, sin red y como el
# uid de los puestos.
#   bash pruebas-de-fuego/geo/c2/spark-java/medir.sh
set -euo pipefail
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
MSYS_NO_PATHCONV=1 docker run --rm --network none --hostname localhost -v "$aqui":/l:ro -w /tmp \
  "${IMAGEN:-ore-puesto-spark:local}" python3 /l/"${1:-previo.py}" 2>/dev/null
