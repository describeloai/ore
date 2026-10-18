#!/usr/bin/env bash
# ORE 0068 C1·2 · Las pruebas del SDK de Node DENTRO de la imagen del puesto de Node (réplica de
# laboratorio, node-lab/Dockerfile): @duckdb/node-api de provisto.txt y spatial/h3 en /opt/ore/duckdb.
# El SDK del árbol se copia junto a los node_modules de la imagen (los del árbol son de Windows).
#   docker build -f pruebas-de-fuego/geo/c1/node-lab/Dockerfile -t ore-puesto-node:local .
#   bash pruebas-de-fuego/geo/c1/node-en-la-imagen.sh ['pruebas/geo*.test.mjs']
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../.." && pwd -W 2>/dev/null || pwd)"
MSYS_NO_PATHCONV=1 docker run --rm -u 0 -v "$raiz":/repo:ro -w /opt/ore --entrypoint bash ore-puesto-node:local -c "
  cp -r /repo/puesto/node/ore /repo/puesto/node/pruebas /opt/ore/ && cp /repo/puesto/node/*.mjs /opt/ore/ 2>/dev/null
  node --test --test-reporter=tap ${1:-'pruebas/*.test.mjs'} 2>&1 | grep -E '^(not ok|ok|# (tests|pass|fail|skip))|SKIP|skip' | head -80"
