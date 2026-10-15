#!/usr/bin/env bash
# L5 · ore-serve a escala (docs/investigacion/ore-serve-lentitud.md).
#
# ore-serve (release) en su propio contenedor con LOS LÍMITES DEL POD (0,5 CPU, 512 MiB),
# una forja `file://` con un árbol sintético, y usuarios a la vez desde otro contenedor.
# Mide latencias por ruta, la memoria del contenedor de ore-serve y si muere por OOM.
#
#   ACTIVOS="1000 10000" USUARIOS="1 10 50" SEGUNDOS=60 bash l5.sh
# Necesita los binarios en el volumen `ore-target` (/target/release/{ore-serve,ore}).
set -u
ACTIVOS=${ACTIVOS:-"1000 10000"}
USUARIOS=${USUARIOS:-"1 10 50"}
SEGUNDOS=${SEGUNDOS:-60}
RAMAS=${RAMAS:-50}
AQUI=$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || cd "$(dirname "$0")" && pwd)
IMG=rust:1-bookworm
RED=ore-l5
export MSYS_NO_PATHCONV=1
docker network create $RED >/dev/null 2>&1 || true
docker volume create ore-l5 >/dev/null
cliente() { docker run --rm --network $RED -v ore-l5:/lab -v ore-target:/target -v "$AQUI:/l5:ro" \
  -e PATH=/target/release:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin $IMG python3 /l5/l5-carga.py "$@"; }

for N in $ACTIVOS; do
  P=$(( N / 50 )); [ $P -lt 5 ] && P=5
  echo "== $N activos ($P paquetes, $RAMAS ramas)"
  cliente sembrar --dir /lab --activos $N --paquetes $P --ramas $RAMAS --historia 200
  docker rm -f ore-serve-l5 >/dev/null 2>&1
  docker run -d --name ore-serve-l5 --network $RED --network-alias ore-serve --cpus 0.5 --memory 512m --memory-swap 512m \
    -v ore-l5:/lab -v ore-target:/target -e FORJA_TOKEN=no-hace-falta $IMG \
    bash -c 'git config --global --add safe.directory "*"; exec /target/release/ore-serve --forja file:///lab/arbol.git --ore /target/release/ore --bind 0.0.0.0:8080 --identidad cabecera --no-es-produccion --organizacion l5' >/dev/null
  until docker exec ore-serve-l5 bash -c 'exec 3<>/dev/tcp/127.0.0.1/8080' 2>/dev/null; do sleep 0.5; done
  for U in $USUARIOS; do
    ( pico=0; while docker inspect -f '{{.State.Running}}' ore-serve-l5 2>/dev/null | grep -q true; do
        m=$(docker stats --no-stream --format '{{.MemUsage}}' ore-serve-l5 2>/dev/null | awk '{print $1}')
        echo "$m"; sleep 1; done ) > /tmp/l5-mem-$N-$U.txt 2>/dev/null &
    MUESTREO=$!
    R=$(cliente cargar --base http://ore-serve:8080 --usuarios $U --segundos $SEGUNDOS --ramas $RAMAS)
    kill $MUESTREO 2>/dev/null
    pico=$(python -c "
import re,sys
v=[]
for l in open(sys.argv[1]):
    m=re.match(r'([0-9.]+)(KiB|MiB|GiB)',l.strip())
    if m: v.append(float(m[1])*{'KiB':1/1024,'MiB':1,'GiB':1024}[m[2]])
print(round(max(v)) if v else '?')" /tmp/l5-mem-$N-$U.txt)
    oom=$(docker inspect -f '{{.State.OOMKilled}}' ore-serve-l5)
    vivo=$(docker inspect -f '{{.State.Running}}' ore-serve-l5)
    echo "L5 · activos=$N usuarios=$U · memoria_pico=${pico:-?}MiB · oom=$oom vivo=$vivo · $R"
    if [ "$vivo" != "true" ]; then
      docker logs ore-serve-l5 2>&1 | tail -3
      docker start ore-serve-l5 >/dev/null
      until docker exec ore-serve-l5 bash -c 'exec 3<>/dev/tcp/127.0.0.1/8080' 2>/dev/null; do sleep 0.5; done
    fi
  done
  docker logs ore-serve-l5 2>&1 | grep -E "acceso ·.*/assets" | tail -3 | cut -c1-240
done
docker rm -f ore-serve-l5 >/dev/null 2>&1
