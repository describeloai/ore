#!/usr/bin/env bash
# ORE 0072 F2·1·3 · El laboratorio de punta a punta: el banco (ore-serve + ore-teselas + el S3 de
# mentira) en `f2-banco`, con el 8080 publicado; los edificios escritos con `ore.write` del SDK (como
# ana, en la imagen del puesto, con el SDK y el `ore-store-r2` de este árbol); y el arnés.
#   bash pruebas-de-fuego/geo/f2/banco/medir.sh            # monta, escribe, mide y lo deja montado
#   docker rm -f f2-banco                                  # para desmontarlo
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
docker rm -f f2-banco >/dev/null 2>&1 || true
MSYS_NO_PATHCONV=1 docker run -d --name f2-banco -p 8080:8080 -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
  bash /repo/pruebas-de-fuego/geo/f2/banco/banco.sh >/dev/null
for i in $(seq 1 120); do docker logs f2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
docker logs f2-banco 2>&1 | grep -q "banco listo" || { docker logs f2-banco 2>&1 | tail -30; exit 1; }
MSYS_NO_PATHCONV=1 docker run --rm --network container:f2-banco -v "$aqui":/l -v ore-target:/t:ro \
  -v "$raiz/puesto/python/ore":/opt/ore/ore:ro -e PATH=/t/release:/usr/local/bin:/usr/bin:/bin \
  "${IMAGEN:-ore-puesto-python:local}" python3 /l/arnes.py
echo "== el registro de ore-teselas"
docker exec f2-banco tail -5 /tmp/banco/teselas.log || true
