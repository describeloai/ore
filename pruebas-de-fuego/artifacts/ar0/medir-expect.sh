#!/bin/sh
# ORE 0074 AR1·0 · ¿manda Maven `Expect: 100-continue` en el PUT, y cuánto espera si nadie contesta
# `100`? Contra el juguete (python http.server, HTTP/1.0: no contesta `100 Continue`), con un jar de
# 5 MB. Deja resultados/expect.jsonl.
set -eu
AQUI=$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)
RES="$AQUI/resultados"; rm -rf "$RES"; mkdir -p "$RES"
export MSYS_NO_PATHCONV=1
CLAVE=token-de-juguete
docker rm -f ar0-juguete >/dev/null 2>&1 || true
docker network create ar0 >/dev/null 2>&1 || true
docker run -d --name ar0-juguete --network ar0 -e CLAVE=$CLAVE -e RAIZ=/res \
  -v "$AQUI/juguete.py:/juguete.py:ro" -v "$RES:/res" python:3.14-slim python -u /juguete.py >/dev/null
sleep 2
docker run --rm --network ar0 maven:3.9-eclipse-temurin-21 sh -c '
  set -e; cd /tmp; mkdir -p c; head -c 5000000 /dev/urandom > c/datos.bin; jar cf0 grande-1.0.0.jar -C c .
  printf "<settings><servers><server><id>ore</id><username>publicar</username><password>'$CLAVE'</password></server></servers></settings>" > s.xml
  for d in "" "-Daether.connector.http.expectContinue=false"; do
    rm -rf ~/.m2/repository/com/banco
    V=$([ -z "$d" ] && echo 1.0.0 || echo 1.0.1); [ "$V" = 1.0.0 ] || cp grande-1.0.0.jar grande-$V.jar
    T0=$(date +%s%N)
    mvn -B -q -s s.xml $d org.apache.maven.plugins:maven-deploy-plugin:3.1.3:deploy-file -Durl=http://ar0-juguete:8080/maven -DrepositoryId=ore \
      -Dfile=grande-$V.jar -DgroupId=com.banco -DartifactId=grande -Dversion=$V -Dpackaging=jar >/tmp/log 2>&1 && R=ok || R=falla
    echo "· ${d:-por defecto}: $R en $(( ($(date +%s%N)-T0)/1000000 )) ms"
  done
'
docker rm -f ar0-juguete >/dev/null
grep '"PUT"' "$RES/registro.jsonl" | grep -v 'sha1\|md5' > "$RES/expect.jsonl"; cat "$RES/expect.jsonl"
