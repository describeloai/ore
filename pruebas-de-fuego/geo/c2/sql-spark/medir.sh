#!/usr/bin/env bash
# ORE 0068 C2·3 2b·0 · El banco de C2·4 y, en su red, `dialecto.py` en la imagen pesada.
#   bash pruebas-de-fuego/geo/c2/sql-spark/medir.sh [programa.py]
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
prog="${1:-dialecto.py}"
docker rm -f c2-banco >/dev/null 2>&1 || true
MSYS_NO_PATHCONV=1 docker run -d --name c2-banco -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
  bash /repo/pruebas-de-fuego/geo/c2/spark-por-v1/banco.sh >/dev/null
for i in $(seq 1 120); do docker logs c2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
docker logs c2-banco 2>&1 | grep -q "banco listo" || { docker logs c2-banco 2>&1 | tail -30; exit 1; }
# El SDK del árbol encima del de la imagen: lo que se prueba es lo que se va a commitear.
MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco -v "$aqui":/l:ro \
  -v "$raiz/puesto/python/ore":/opt/ore/ore:ro \
  "${IMAGEN:-ore-puesto-spark:local}" python3 "/l/$prog" 2>/dev/null
echo "== el log de ore-serve (los errores)"
MSYS_NO_PATHCONV=1 docker exec c2-banco bash -c "grep -E ' (4[0-9][0-9]|5[0-9][0-9]) ' /tmp/banco/serve.log | tail -8 | cut -c1-200" || true
