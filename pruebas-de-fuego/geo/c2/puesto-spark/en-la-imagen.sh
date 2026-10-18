#!/usr/bin/env bash
# ORE 0068 C2·2 · La imagen pesada, sin red: construye la etapa `puesto-spark` del Dockerfile y corre
# `sonda.py` dentro como el uid de los puestos.
#   bash pruebas-de-fuego/geo/c2/puesto-spark/en-la-imagen.sh
#     IMAGEN=…      otra imagen ya construida (no construye)
#     BINARIOS=…    la imagen de binarios para `--build-arg BINARIOS` (por defecto, la del registro)
#
# ⚠️ `--hostname localhost`: con `--network none` Docker no pone el hostname del contenedor en
#   /etc/hosts, y Spark lo resuelve al arrancar. Un pod sí lo tiene; esto imita el pod, no lo arregla.
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
REGISTRO=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore
if [ -z "${IMAGEN:-}" ]; then
  IMAGEN=ore-puesto-spark:local
  docker build -q --target puesto-spark --build-arg "BINARIOS=${BINARIOS:-$REGISTRO/ore-binarios:5321c3517837}" \
    -t "$IMAGEN" "$raiz" >/dev/null
fi
echo "imagen: $IMAGEN · $(docker save "$IMAGEN" | gzip -1 | wc -c | awk '{printf "%.2f GB comprimida", $1/1e9}')"
MSYS_NO_PATHCONV=1 docker run --rm --network none --hostname localhost \
  -v "$aqui/sonda.py":/sonda.py:ro "$IMAGEN" python3 /sonda.py 2>/dev/null \
  | grep -o "\(OK\|FALLA\) *·.*\|[0-9]*/[0-9]* casos"
