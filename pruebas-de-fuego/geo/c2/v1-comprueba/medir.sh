#!/usr/bin/env bash
# ORE 0068 C2·3 pieza 3 · `/v1` comprueba: el banco de C2·4 (binarios release con lo de la pieza 3) y,
# en su red, `comprobar.py` en la imagen pesada con el SDK del árbol.
#   1 · cargo build --release -p ore-cli --bin ore -p ore-serve --bin ore-serve -p ore-store --bin ore-store-r2
#   2 · bash pruebas-de-fuego/geo/c2/v1-comprueba/medir.sh
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
docker rm -f c2-banco >/dev/null 2>&1 || true
MSYS_NO_PATHCONV=1 docker run -d --name c2-banco -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
  bash /repo/pruebas-de-fuego/geo/c2/spark-por-v1/banco.sh >/dev/null
for i in $(seq 1 120); do docker logs c2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
docker logs c2-banco 2>&1 | grep -q "banco listo" || { docker logs c2-banco 2>&1 | tail -30; exit 1; }
MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco -v "$aqui":/l:ro \
  -v "$raiz/puesto/python/ore":/opt/ore/ore:ro "${IMAGEN:-ore-puesto-spark:local}" python3 /l/comprobar.py 2>/dev/null || true
echo "== el log de ore-serve (lo que no fue 2xx)"
MSYS_NO_PATHCONV=1 docker exec c2-banco bash -c "grep -E ' (4[0-9][0-9]|5[0-9][0-9]) ' /tmp/banco/serve.log | tail -12 | cut -c1-160" || true
