#!/usr/bin/env bash
# La imagen de un laboratorio de pruebas (HBase, Phoenix…), por la huella de su carpeta.
#
#   bash ci/laboratorio.sh <nombre-local> <carpeta> [args de docker build…]
#
# Se construía en CADA CI desde el archivo de Apache (10 min Phoenix, 9 min HBase, medido
# 2026-10-23) aunque su carpeta no cambiara. Ahora vive en GHCR con la etiqueta de la huella
# de lo que la construye (`git ls-files -s <carpeta>`): si está, se baja (segundos); si no, se
# construye UNA vez y se sube. Queda etiquetada como `<nombre-local>` para lo de después.
# Pide `packages: write` en el job y `GITHUB_TOKEN` en el entorno.
set -euo pipefail
NOMBRE=${1:?nombre local}; CARPETA=${2:?carpeta}; shift 2
H=$(git ls-files -s "$CARPETA" | sha256sum | cut -c1-16)
DUENO=$(echo "${GITHUB_REPOSITORY_OWNER:?}" | tr '[:upper:]' '[:lower:]')
IMG="ghcr.io/$DUENO/$NOMBRE:$H"
echo "$GITHUB_TOKEN" | docker login ghcr.io -u "${GITHUB_ACTOR:?}" --password-stdin >/dev/null
if docker pull -q "$IMG" >/dev/null 2>&1; then
  echo "$NOMBRE: $IMG, ya construida"
else
  echo "$NOMBRE: $IMG no está; se construye y se sube"
  docker build -t "$IMG" "$@" "$CARPETA"
  docker push -q "$IMG" >/dev/null || echo "::warning::$NOMBRE: no se pudo subir $IMG (la próxima la vuelve a construir)"
fi
docker tag "$IMG" "$NOMBRE"
