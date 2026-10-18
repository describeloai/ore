#!/usr/bin/env bash
# ORE 0068 C1·1 · Las pruebas del SDK de Python DENTRO de la imagen del puesto (la etapa `puesto-python`
# del Dockerfile, construida en local): las versiones fijadas de provisto.txt y `spatial`/`h3` en
# /opt/ore/duckdb, como en producción. Con el SDK del árbol, no el de la imagen.
# Por qué: en Windows `spatial` carga del directorio por defecto y en la imagen no; las pruebas geo
# pasaban aquí y el puesto habría fallado (pieza 0).
#   docker build --target puesto-python -t ore-puesto-python:local .   # una vez, o al cambiar la imagen
#   bash pruebas-de-fuego/geo/c1/sdk-en-la-imagen.sh [patrón]           # p. ej. 'test_*geo*.py'
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../.." && pwd -W 2>/dev/null || pwd)"
# El árbol entero (algunas pruebas leen `crates/…` y las plantillas), en solo lectura.
MSYS_NO_PATHCONV=1 docker run --rm -u 0 -v "$raiz":/repo:ro -e PYTHONPATH=/repo/puesto/python -w /repo \
  --entrypoint python ore-puesto-python:local \
  -m unittest discover -s /repo/puesto/python/pruebas -p "${1:-test*.py}" -v
