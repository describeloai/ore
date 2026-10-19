#!/usr/bin/env bash
# ORE 0068 · cierre de C1 · `geometry_always_xy`: el previo (`previo.py`) en el DuckDB del puesto de Python,
# y luego (`sdk`) lo que dan los tres SDK con el ajuste puesto, cada uno en su imagen, sin red.
#   bash pruebas-de-fuego/geo/c1/ejes/medir.sh          # el previo
#   bash pruebas-de-fuego/geo/c1/ejes/medir.sh sdk      # los tres SDK del árbol
set -euo pipefail
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
d() { MSYS_NO_PATHCONV=1 docker run --rm --network none --hostname localhost -v "$aqui":/l:ro -w /tmp "$@"; }
if [ "${1:-}" != "sdk" ]; then
  d ore-puesto-python:local python3 /l/previo.py
  exit
fi
echo "== python"; d -v "$raiz/puesto/python/ore":/opt/ore/ore:ro ore-puesto-python:local python3 /l/sdk.py
echo "== node";   d -v "$raiz/puesto/node/ore":/opt/ore/ore:ro ore-puesto-node:local node /l/sdk.mjs
echo "== java";   d -v "$raiz/puesto/jvm/ore":/r/ore:ro ore-puesto-jvm:local bash -c "mkdir -p /tmp/c && javac -proc:none -nowarn -cp '/opt/ore/lib/*' -d /tmp/c /r/ore/*.java 2>/dev/null && java --add-opens=java.base/java.nio=ALL-UNNAMED -cp '/tmp/c:/opt/ore/lib/*' /l/Sdk.java"
