#!/usr/bin/env bash
# ORE 0068 C2·3 pieza 5 · EL PREVIEW HEAVYWEIGHT ES UN JOB CORTO, de punta a punta en local: el banco de
# C2·4 con los repositorios Heavyweight de `build-pesado` (SQL, Python y Java), una sesión de Python de
# ana, y por cada caso `POST /puestos/{sesion}/preview` con el texto del EDITOR (sin guardar): ORE lanza
# un trabajo en `spark`, cuyo agente corre aquí en `ore-puesto-spark:local` (como el Job). Luego, lo que
# la consola lee (`GET /puestos/{trabajo}/celdas/1`) y, al final, que nada se escribió.
#   1 · los binarios: cargo build --release -p ore-cli --bin ore -p ore-serve --bin ore-serve
#       -p ore-store --bin ore-store-r2   (en rust:1, con el volumen ore-target)
#   2 · bash pruebas-de-fuego/geo/c2/preview-pesado/medir.sh
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
pesado="$raiz/pruebas-de-fuego/geo/c2/build-pesado"
IMAGEN="${IMAGEN:-ore-puesto-spark:local}"
SDK=(-v "$raiz/puesto/python/ore":/opt/ore/ore:ro)
ex() { MSYS_NO_PATHCONV=1 docker exec c2-banco "$@"; }
pedir() { ex python3 /repo/pruebas-de-fuego/geo/c2/preview-pesado/pedir.py "$@"; }

docker rm -f c2-banco >/dev/null 2>&1 || true
MSYS_NO_PATHCONV=1 docker run -d --name c2-banco -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
  bash /repo/pruebas-de-fuego/geo/c2/spark-por-v1/banco.sh >/dev/null
for i in $(seq 1 120); do docker logs c2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
docker logs c2-banco 2>&1 | grep -q "banco listo" || { docker logs c2-banco 2>&1 | tail -30; exit 1; }

echo "== 1 · sembrar y los repositorios Heavyweight (los de build-pesado)"
MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco -v "$pesado":/l:ro "${SDK[@]}" "$IMAGEN" \
  python3 /l/sembrar.py 2>/dev/null | tail -1
ex bash /repo/pruebas-de-fuego/geo/c2/build-pesado/repo.sh | tail -1
sesion=$(pedir sesion)
echo "   la sesión de ana: $sesion"

for caso in ${CASOS:-py java py-fuera java-techo java-no-compila sql}; do
  echo "== 2 · Preview $caso"
  r=$(pedir lanzar "$caso" "$sesion")
  echo "   $r" | cut -c1-260
  trabajo=$(echo "$r" | python3 -c "import json,sys; print(json.load(sys.stdin).get('trabajo') or '')")
  if [ -z "$trabajo" ]; then
    echo "$r" | python3 -c "import json,sys; d=json.load(sys.stdin); print('OK    · sql · no es un trabajo:', d['codigo'], d['motivo']) if d['codigo'] == 422 and d['motivo'] == 'not_supported' else print('FALLA · sin trabajo')"
    continue
  fi
  t0=$(date +%s)
  MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco --memory 4g "${SDK[@]}" \
    -e ORE_SERVE=http://127.0.0.1:8080 -e PUESTO="$trabajo" -e TRABAJO="preview@editor" -e ORE_SUJETO=agente:local \
    -e TTL=300 -e ORE_MEMORIA_MB=4096 -e PUESTO_DIR=/tmp -e TRABAJO_DIR=/tmp "$IMAGEN" 2>&1 \
    | grep -E "^agente" | tail -3 | cut -c1-200 || true
  echo "   el agente, de arrancar a terminar: $(( $(date +%s) - t0 )) s"
  pedir ver "$caso" "$trabajo"
done
echo "== 3 · nada escrito"
pedir lago
