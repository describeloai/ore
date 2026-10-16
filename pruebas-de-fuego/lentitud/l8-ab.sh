#!/usr/bin/env bash
# L8 · A/B de dos binarios de ore-serve sobre el árbol de L5 (docs/investigacion/ore-serve-lentitud.md).
#
# Alternando A y B, cada vuelta con un servidor recién arrancado (0,5 CPU, 512 MiB): `/assets`
# de main y de RAMAS ramas en frío, de una en una. Por petición, el total y lo que dice su
# línea de acceso (git local e índice).
#
#   A=/target/l5-musl B=/target/l5-f7b VUELTAS=2 bash l8-ab.sh
set -u
A=${A:?}; B=${B:?}
VUELTAS=${VUELTAS:-2}
RAMAS=${RAMAS:-"l5/r001 l5/r002 l5/r003"}
IMG=rust:1-bookworm
RED=ore-l5
export MSYS_NO_PATHCONV=1
for v in $(seq 1 "$VUELTAS"); do
  for BIN in "$A" "$B"; do
    docker rm -f ore-serve-l8 >/dev/null 2>&1
    docker run -d --name ore-serve-l8 --network $RED --network-alias ore-serve-l8 --cpus 0.5 --memory 512m --memory-swap 512m \
      -v ore-l5:/lab -v ore-target:/target -e FORJA_TOKEN=no-hace-falta -e BIN="$BIN" $IMG \
      bash -c 'git config --global --add safe.directory "*"; exec $BIN/ore-serve --forja file:///lab/arbol.git --ore $BIN/ore --bind 0.0.0.0:8080 --identidad cabecera --no-es-produccion --organizacion l8' >/dev/null
    until docker exec ore-serve-l8 bash -c 'exec 3<>/dev/tcp/127.0.0.1/8080' 2>/dev/null; do sleep 0.5; done
    t=$(docker run --rm --network $RED $IMG bash -c "for r in '' $RAMAS; do curl -s -o /dev/null -w '%{time_total} ' -H 'x-ore-sujeto: persona:ana' \${r:+-H \"x-ore-rama: \$r\"} http://ore-serve-l8:8080/assets; done")
    f=$(docker logs ore-serve-l8 2>&1 | grep "acceso · GET /assets" \
        | sed -E 's/.* ([0-9]+) ms · persona.*git-local [0-9]+×([0-9]+) ms.*índice [a-zé]+ ([0-9]+) ms.*/\1:git\2:idx\3/' | tr '\n' ' ')
    echo "vuelta $v · $BIN · s: $t· ms: $f"
  done
done
docker rm -f ore-serve-l8 >/dev/null 2>&1
