#!/usr/bin/env bash
# L7 · quién se come la memoria con N usuarios (docs/investigacion/ore-serve-lentitud.md).
#
# Sobre el árbol que L5 dejó en el volumen `ore-l5`: ore-serve con los límites del pod, N
# usuarios como en L5, y cada 2 s, dentro del contenedor, la memoria por proceso (RSS sumado
# por nombre y cuántos hay), la anónima y la de ficheros del cgroup, y los hilos de ore-serve.
#
#   BIN=/target/l5-f7a USUARIOS=50 SEGUNDOS=40 bash l7-quien-come.sh
set -u
BIN=${BIN:-/target/release}
USUARIOS=${USUARIOS:-50}
SEGUNDOS=${SEGUNDOS:-40}
RAMAS=${RAMAS:-50}
AQUI=$(cd "$(dirname "$0")" && { pwd -W 2>/dev/null || pwd; })
IMG=rust:1-bookworm
RED=ore-l5
export MSYS_NO_PATHCONV=1
docker rm -f ore-serve-l7 >/dev/null 2>&1
docker run -d --name ore-serve-l7 --network $RED --network-alias ore-serve-l7 --cpus 0.5 --memory 512m --memory-swap 512m \
  -v ore-l5:/lab -v ore-target:/target -e FORJA_TOKEN=no-hace-falta -e BIN=$BIN ${MALLOC_ARENA_MAX:+-e MALLOC_ARENA_MAX=$MALLOC_ARENA_MAX} $IMG \
  bash -c 'apt-get -qq update >/dev/null 2>&1; apt-get -qq install -y procps >/dev/null 2>&1; git config --global --add safe.directory "*"; exec $BIN/ore-serve --forja file:///lab/arbol.git --ore $BIN/ore --bind 0.0.0.0:8080 --identidad cabecera --no-es-produccion --organizacion l7' >/dev/null
until docker exec ore-serve-l7 bash -c 'exec 3<>/dev/tcp/127.0.0.1/8080' 2>/dev/null; do sleep 0.5; done
( docker run --rm --network $RED -v "$AQUI:/l5:ro" $IMG python3 /l5/l5-carga.py cargar \
    --base http://ore-serve-l7:8080 --usuarios $USUARIOS --segundos $SEGUNDOS --ramas $RAMAS > /tmp/l7-carga.json 2>&1 ) &
CARGA=$!
while kill -0 $CARGA 2>/dev/null && [ "$(docker inspect -f '{{.State.Running}}' ore-serve-l7)" = true ]; do
  docker exec ore-serve-l7 bash -c '
    a=$(awk "/^anon /{print int(\$2/1048576)}" /sys/fs/cgroup/memory.stat)
    f=$(awk "/^file /{print int(\$2/1048576)}" /sys/fs/cgroup/memory.stat)
    h=$(ls /proc/$(pgrep -o ore-serve)/task | wc -l)
    p=$(ps -eo rss=,comm= | awk "{m[\$2]+=\$1; n[\$2]++} END {for (k in m) if (m[k]>4096) printf \"%s %d×%dMiB  \", k, n[k], m[k]/1024}")
    echo "anon=${a} file=${f} hilos=${h} · $p"' 2>/dev/null
  sleep 2
done
wait $CARGA
echo "oom=$(docker inspect -f '{{.State.OOMKilled}}' ore-serve-l7)"
head -c 300 /tmp/l7-carga.json; echo
docker logs ore-serve-l7 2>&1 | grep -E "acceso · GET" | awk -F' · ' '{print $2}' | sort | uniq -c
docker rm -f ore-serve-l7 >/dev/null 2>&1
