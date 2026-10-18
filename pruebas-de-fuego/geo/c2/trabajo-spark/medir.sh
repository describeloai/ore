#!/usr/bin/env bash
# ORE 0068 C2·3 pieza 0 · El puesto pesado de verdad: el AGENTE de Python dentro de `puesto-spark`,
# reclamando un puesto de un ore-serve de verdad (el banco de C2·4: release en rust:1, S3 de mentira,
# cola con la plantilla del inquilino, identidad de cabecera), y celdas PySpark que escriben por /v1.
#   bash pruebas-de-fuego/geo/c2/trabajo-spark/medir.sh
#     MEMORIA=4g   el límite del contenedor (el del pod: 51-el-puesto.yaml, limits 4Gi)
#     DRIVER=2g    spark.driver.memory
#     IMAGEN=ore-puesto-spark:local
#     PUNTOS=2000000  los puntos del join (caso 4)
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
IMAGEN="${IMAGEN:-ore-puesto-spark:local}"
MEMORIA="${MEMORIA:-4g}"
DRIVER="${DRIVER:-2g}"
P=puesto-ana-python

docker rm -f c2-banco c2-agente >/dev/null 2>&1 || true
MSYS_NO_PATHCONV=1 docker run -d --name c2-banco -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
  bash /repo/pruebas-de-fuego/geo/c2/spark-por-v1/banco.sh >/dev/null
for i in $(seq 1 120); do docker logs c2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
docker logs c2-banco 2>&1 | grep -q "banco listo" || { docker logs c2-banco 2>&1 | tail -30; exit 1; }

# Ana abre el puesto (lo que hace la consola); el Job no lo corre nadie: el agente va aquí al lado.
MSYS_NO_PATHCONV=1 docker exec c2-banco curl -s -o /dev/null -w "abrir el puesto · %{http_code}\n" \
  -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' -d '{}' http://127.0.0.1:8080/puestos

# El agente, con lo que la plantilla le pone (ORE_MEMORIA_MB = el límite) y el uid de la imagen.
MB=$(( ${MEMORIA%g} * 1024 ))
MSYS_NO_PATHCONV=1 docker run -d --name c2-agente --network container:c2-banco --memory "$MEMORIA" \
  -e ORE_SERVE=http://127.0.0.1:8080 -e PUESTO=$P -e ORE_SUJETO=agente:local -e TTL=1800 \
  -e ORE_MEMORIA_MB=$MB -e ORE_ALMACEN=dir:/tmp/almacen -e DRIVER="$DRIVER" "$IMAGEN" >/dev/null

echo "== memoria del contenedor $MEMORIA · spark.driver.memory $DRIVER"
MSYS_NO_PATHCONV=1 docker exec c2-banco python3 /repo/pruebas-de-fuego/geo/c2/trabajo-spark/conducir.py $P "${PUNTOS:-2000000}" || true
echo "== el agente (las últimas líneas)"
docker logs c2-agente 2>&1 | grep -v -E "WARN|^\s+at |log4j|setLogLevel|NativeCodeLoader|Stage [0-9]" | tail -8 | cut -c1-240
echo "== ¿lo mató el límite? $(docker inspect c2-agente --format '{{.State.OOMKilled}} · {{.State.Status}}')"
