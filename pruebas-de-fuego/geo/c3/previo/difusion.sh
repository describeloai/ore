#!/usr/bin/env bash
# ORE 0068 C3 · previo · `difusion.py` en `ore-puesto-spark:local` (la imagen de un Build pesado), sin red.
#   bash pruebas-de-fuego/geo/c3/previo/difusion.sh        (PUNTOS=2000000 LIMITE_S=180 por defecto)
set -euo pipefail
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
MSYS_NO_PATHCONV=1 docker run --rm --network none --hostname localhost --memory 8g -v "$aqui":/l:ro \
  -e HOME=/tmp -e PUNTOS="${PUNTOS:-2000000}" -e LIMITE_S="${LIMITE_S:-180}" -e HEAP="${HEAP:-5g}" \
  -w /tmp "${IMAGEN:-ore-puesto-spark:local}" python3 /l/difusion.py 2>/dev/null | grep --line-buffered '^C3|'
