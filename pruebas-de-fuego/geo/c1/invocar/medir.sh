#!/usr/bin/env bash
# ORE 0068 · cierre de C1 · La geometría que devuelve una función, por su invocación: las celdas DE VERDAD
# (las escribe ore-serve en su prueba) corridas en las imágenes de Python y de Node, con el SDK del árbol.
#   1 · las celdas: ORE_CELDAS_GEO=… cargo test -p ore-serve --bin ore-serve las_celdas_de_una_funcion_que_da_una_geometria
#       (este guion lo hace, en rust:1 con los volúmenes de siempre)
#   2 · bash pruebas-de-fuego/geo/c1/invocar/medir.sh
set -euo pipefail
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
celdas="$aqui/.celdas"
mkdir -p "$celdas"
MSYS_NO_PATHCONV=1 docker run --rm -v "$raiz":/repo -v ore-target:/repo/target -v ore-cargo:/usr/local/cargo/registry \
  -v ore-rustup:/usr/local/rustup -w /repo -e ORE_CELDAS_GEO=/repo/pruebas-de-fuego/geo/c1/invocar/.celdas rust:1 \
  bash -c "git config --global --add safe.directory '*'; cargo test -q -p ore-serve --bin ore-serve las_celdas_de_una_funcion_que_da_una_geometria 2>&1 | grep -E 'test result|FAILED|panicked'"
d() { MSYS_NO_PATHCONV=1 docker run --rm --network none --hostname localhost -v "$aqui":/l:ro -v "$celdas":/celdas:ro -w /tmp "$@"; }
d -v "$raiz/puesto/python/ore":/opt/ore/ore:ro ore-puesto-python:local python3 /l/correr.py
d -v "$raiz/puesto/node/ore":/opt/ore/ore:ro ore-puesto-node:local node /l/correr.mjs
