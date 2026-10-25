#!/usr/bin/env bash
# ORE 0074 AR2·1 + AR2·2 · `uv publish`, `twine upload` y `uv lock` DE VERDAD contra `ore-serve` y
# `ore-medios` locales.
#
#   uv / twine (Docker) ──► ore-serve :8080 (lee el multipart, decide, pasa el fichero) ──► ore-medios ──► S3 local
#
#   0  dos registros con `pypi` por la API; el token trae las instrucciones de PyPI
#   1  `uv build` + `uv publish` (rueda y sdist, D-A15) y `twine upload` de otra versión
#   2  lo que no entra: republicar → 409; sin token → 401; token de otro registro → 403; un registro
#      sin `pypi` → 400; `sha256_digest` que no casa → 400 y no queda; el fichero no es el último → 400
#   3  el índice simple: HTML con `#sha256=`; nombre sin normalizar → 301; lo que no hay o el token no
#      lee → 404 (nunca 403, AR2·0); token que no vale → 401
#   4  resolver como la capa: `uv lock` con ORE delante de PyPI, `==1.0.0` encuentra `1.0` (PEP 440),
#      instalar y `import` da 42; el lock sin el token
#   5  el mismo nombre en otro registro → 409 (D-A13)
#   6  AR2·3: la capa de Python de verdad (resolver.sh y capa.py): ORE delante de PyPI, la caja con la
#      rueda publicada, `de_artefactos` en el informe, el token fuera del trabajo; 2.0 sin publicar → error
#
#   bash pruebas-de-fuego/artifacts/ar2-local.sh        (CONSERVAR=1 deja los contenedores)
#
# Necesita Docker y el volumen `ore-target-q1`; el contenedor de Python baja uv, twine y `uv_build` de PyPI.
set -u
RAIZ="$(cd "$(dirname "$0")/../.." && (pwd -W 2>/dev/null || pwd))"
export MSYS_NO_PATHCONV=1
VERSITY_IMG=versity/versitygw@sha256:30292fc2eeacc67a36993b01f7a7a5e3361a19cced0e80c1d71cfa2a4b0a2499
AWS_IMG=amazon/aws-cli@sha256:e3e329e1d2894b7b4bbb0aacacd0a155262159b2e7a3b4275eb1f24046d3e06c
PY_IMG=python:3.13-slim
RED=ore-ar2
SECRETO="ar2-$(date +%s)-$RANDOM"
VK="k$RANDOM$RANDOM"; VS="s$RANDOM$RANDOM$RANDOM"

limpiar() { docker rm -f ar2-s3 ar2-ore >/dev/null 2>&1; docker volume rm -f ar2-s3 >/dev/null 2>&1; }
limpiar; [ -n "${CONSERVAR:-}" ] || trap limpiar EXIT
docker network create $RED >/dev/null 2>&1 || true

docker volume create ar2-s3 >/dev/null
docker run -d --name ar2-s3 --network $RED -v ar2-s3:/datos "$VERSITY_IMG" \
  --access "$VK" --secret "$VS" --port :7070 posix /datos >/dev/null || exit 2
sleep 2
docker run --rm --network $RED -e AWS_ACCESS_KEY_ID=$VK -e AWS_SECRET_ACCESS_KEY=$VS -e AWS_DEFAULT_REGION=us-east-1 \
  "$AWS_IMG" --endpoint-url http://ar2-s3:7070 s3api create-bucket --bucket lago >/dev/null || exit 2

docker run -d --name ar2-ore --network $RED -v "$RAIZ:/src" -v ore-cargo:/usr/local/cargo/registry \
  -v ore-target-q1:/target -e CARGO_TARGET_DIR=/target -w /src \
  -e ORE_STORE=r2 -e ORE_R2_S3_ENDPOINT=http://ar2-s3:7070 -e ORE_R2_BUCKET=lago -e ORE_R2_REGION=us-east-1 \
  -e ORE_R2_ACCESS_KEY_ID=$VK -e ORE_R2_SECRET_ACCESS_KEY=$VS \
  -e ORE_MEDIOS_DIRECCION=127.0.0.1:8097 -e CLAVE_PRUEBA=$SECRETO -e ORE_ARTEFACTOS_SECRETO_FICHERO=/tmp/artefactos \
  rust:1-bookworm sh -c '
    set -e
    cargo build -q -p ore-serve -p ore-medios -p ore-cli 2>&1 | tail -5
    A=/tmp/arbol; rm -rf $A; mkdir -p $A
    cp -r /src/vendor/oos/conformance/v1alpha34/valid/a-maven-registry/input/. $A/
    git -C $A init -q && git -C $A -c user.name=t -c user.email=t@t add -A && git -C $A -c user.name=t -c user.email=t@t commit -qm arbol
    /target/debug/ore-medios > /tmp/medios.log 2>&1 &
    # AR1·5a: la clave, de un fichero, como en la malla (el tmpfs del contenedor de inicio).
    printf %s "$CLAVE_PRUEBA" > /tmp/artefactos
    echo "/target/debug/ore-serve --repo $A --ore /target/debug/ore --bind 0.0.0.0:8080 --identidad cabecera --no-es-produccion >> /tmp/serve.log 2>&1 & echo \$! > /tmp/serve.pid" > /tmp/serve.sh
    sh /tmp/serve.sh
    touch /tmp/listo; wait' >/dev/null || exit 2
echo "· compilando y arrancando (la primera vez tarda)…"
for _ in $(seq 1 900); do
  docker exec ar2-ore sh -c 'test -f /tmp/listo && curl -s -o /dev/null http://127.0.0.1:8080/salud' 2>/dev/null && break
  docker ps -q -f name=ar2-ore | grep -q . || { echo "✗ ar2-ore murió"; docker logs ar2-ore | tail -20; exit 2; }
  sleep 2
done

# El de leer de la capa (AR2·3 lo pondrá `rendir_capa`), firmado como `artefactos::token_de_leer`.
token() { # persona registro verbo
  docker exec ar2-ore sh -c '
    c=$(printf "{\"e\":%s,\"p\":\"%s\",\"r\":\"%s\",\"v\":\"%s\"}" $(( $(date +%s) + 600 )) "$1" "$2" "$3")
    k=$(printf %s "$CLAVE_PRUEBA" | sha256sum | cut -c1-64)
    f=$(printf %s "$c" | openssl dgst -sha256 -mac HMAC -macopt hexkey:$k | sed "s/.*= //")
    printf "ore_art_%s.%s" "$(printf %s "$c" | od -An -tx1 | tr -d " \n")" "$f"' _ "$1" "$2" "$3"
}
api() { docker exec ar2-ore curl -s -o /tmp/r.json -w '%{http_code}' -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' "$@"; }
pide_token() { # registro (b/s/r)
  c=$(api -X POST "http://127.0.0.1:8080/artifact-registries/$1/token")
  [ "$c" = 201 ] || { echo "✗ el token de $1: $c $(docker exec ar2-ore cat /tmp/r.json)" >&2; return 1; }
  docker exec ar2-ore sh -c 'sed "s/.*\"token\":\"\([^\"]*\)\".*/\1/" /tmp/r.json'
}
f0=0
echo "0 · dos registros con pypi por la API, y sus tokens"
for r in py py2; do
  c=$(api -X PUT -d '{"spec":{"formats":["pypi"]}}' http://127.0.0.1:8080/documentos/ArtifactRegistry/banco/riesgo/$r)
  [ "$c" = 201 ] && echo "  ✓ banco.riesgo.$r (formats: [pypi]) → 201" || { echo "  ✗ crear $r: $c $(docker exec ar2-ore cat /tmp/r.json)"; f0=$((f0+1)); }
done
PUBLICAR=$(pide_token banco/riesgo/py) || f0=$((f0+1))
docker exec ar2-ore sh -c 'grep -q "\"upload_url\":\"http://127.0.0.1:8080/artifacts/banco/riesgo/py/pypi/\"" /tmp/r.json && grep -q "uv publish" /tmp/r.json && grep -q "__token__" /tmp/r.json' \
  && echo "  ✓ el token de banco.riesgo.py trae las instrucciones de PyPI (upload_url, uv publish, __token__)" \
  || { echo "  ✗ sin instrucciones de PyPI: $(docker exec ar2-ore cat /tmp/r.json)"; f0=$((f0+1)); }
OTRO=$(pide_token banco/riesgo/py2) || f0=$((f0+1))
MAVEN=$(pide_token banco/riesgo/libs) || f0=$((f0+1))
docker exec ar2-ore sh -c '! grep -q upload_url /tmp/r.json' \
  && echo "  ✓ el de banco.riesgo.libs (sólo maven) no trae PyPI" || { echo "  ✗ libs trae PyPI"; f0=$((f0+1)); }
LEER=$(token capa '*' leer)

docker run --rm --network $RED -v "$RAIZ/pruebas-de-fuego/artifacts/ar2:/ar2:ro" -v "$RAIZ/puesto/python/capa:/capa:ro" \
  -e PUBLICAR="$PUBLICAR" -e OTRO="$OTRO" -e MAVEN="$MAVEN" -e LEER="$LEER" -e ORE=http://ar2-ore:8080 \
  "$PY_IMG" bash /ar2/dentro.sh
r=$?
echo "7 · la clave sobrevive a un reinicio de ore-serve (AR1·5a)"
f7=0
docker exec ar2-ore sh -c 'kill $(cat /tmp/serve.pid); sleep 1; sh /tmp/serve.sh'
for _ in $(seq 1 30); do docker exec ar2-ore curl -s -o /dev/null http://127.0.0.1:8080/salud && break; sleep 1; done
c=$(docker exec ar2-ore curl -s -o /dev/null -w '%{http_code}' -u "__token__:$PUBLICAR" http://127.0.0.1:8080/artifacts/pypi/simple/banco-riesgo/)
[ "$c" = 200 ] && echo "  ✓ el token emitido antes del reinicio sigue valiendo → 200 (la clave es la de la celda)" \
  || { echo "  ✗ tras el reinicio, el token de antes → $c"; f7=1; }
docker exec ar2-ore sh -c 'grep -q "sin la clave de la celda" /tmp/serve.log' \
  && { echo "  ✗ ore-serve dice que no tiene la clave de la celda"; f7=$((f7+1)); } || echo "  ✓ ore-serve no avisa de clave por proceso: leyó el fichero"
r=$((r + f0 + f7))
echo; [ $r -eq 0 ] && echo "AR2·1–AR2·3 y AR1·5a local: todo en verde" || echo "AR2 local: $r fallo(s) en total"
echo "── lo que dijo ore-serve (pypi) ──"
docker exec ar2-ore sh -c 'grep -a "pypi" /tmp/serve.log | tail -20'
exit $r
