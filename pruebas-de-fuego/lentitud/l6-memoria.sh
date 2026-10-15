#!/usr/bin/env bash
# L6 · la memoria de ore-serve al hacer y servir un índice (docs/investigacion/ore-serve-lentitud.md).
#
# Sobre el árbol que L5 dejó en el volumen `ore-l5` (`l5.sh` lo siembra): ore-serve con los
# límites del pod, y de una en una —sin carga—: `/assets` de main, de tres ramas, y otra vez
# las mismas (de la caché). Tras cada petición, la memoria del cgroup partida en anónima
# (el proceso) y de ficheros (los árboles montados, que el kernel puede soltar), y el pico.
#
#   BIN=/target/l6 bash l6-memoria.sh
set -u
BIN=${BIN:-/target/release}
IMG=rust:1-bookworm
RED=ore-l5
export MSYS_NO_PATHCONV=1
docker rm -f ore-serve-l6 >/dev/null 2>&1
docker run -d --name ore-serve-l6 --network $RED --network-alias ore-serve-l6 --cpus 0.5 --memory 512m --memory-swap 512m \
  -v ore-l5:/lab -v ore-target:/target -e FORJA_TOKEN=no-hace-falta -e BIN=$BIN ${MALLOC_ARENA_MAX:+-e MALLOC_ARENA_MAX=$MALLOC_ARENA_MAX} $IMG \
  bash -c 'git config --global --add safe.directory "*"; exec $BIN/ore-serve --forja file:///lab/arbol.git --ore $BIN/ore --bind 0.0.0.0:8080 --identidad cabecera --no-es-produccion --organizacion l6' >/dev/null
until docker exec ore-serve-l6 bash -c 'exec 3<>/dev/tcp/127.0.0.1/8080' 2>/dev/null; do sleep 0.5; done
mem() { docker exec ore-serve-l6 bash -c 'a=$(awk "/^anon /{print int(\$2/1048576)}" /sys/fs/cgroup/memory.stat); f=$(awk "/^file /{print int(\$2/1048576)}" /sys/fs/cgroup/memory.stat); p=$(( $(cat /sys/fs/cgroup/memory.peak 2>/dev/null || echo 0) / 1048576 )); r=$(ps -o rss= -C ore-serve | awk "{print int(\$1/1024)}"); echo "anon=${a}MiB file=${f}MiB rss=${r}MiB pico=${p}MiB"'; }
echo "arranque · $(mem)"
for r in "" l5/r001 l5/r002 l5/r003 "" l5/r001 l5/r002 l5/r003; do
  t=$(docker run --rm --network $RED $IMG bash -c "curl -s -o /tmp/a -w '%{http_code} %{time_total}s' -H 'x-ore-sujeto: persona:ana' ${r:+-H 'x-ore-rama: $r'} http://ore-serve-l6:8080/assets; echo \" \$(wc -c < /tmp/a)B\"")
  echo "/assets ${r:-main} · $t · $(mem) · oom=$(docker inspect -f '{{.State.OOMKilled}}' ore-serve-l6)"
done
docker logs ore-serve-l6 2>&1 | grep "acceso · GET /assets" | cut -c1-200
docker rm -f ore-serve-l6 >/dev/null 2>&1
