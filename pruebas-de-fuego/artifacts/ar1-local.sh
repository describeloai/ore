#!/usr/bin/env bash
# ORE 0074 AR1·1 + AR1·2 · UN `mvn deploy` DE VERDAD contra `ore-serve` y `ore-medios` locales.
#
#   Maven (Docker) ──► ore-serve :8080 (decide, pasa el flujo) ──► ore-medios :8097 ──► S3 local
#                                                                                       (VersityGW, como R2)
#
#   1  deploy-file de un jar con una clase (y uno de 20 MB: el camino del temporal)
#   2  republicar → 409 · -SNAPSHOT → 400 · sin token → 401 · token de otro registro → 403
#   3  `mvn deploy` de un proyecto con <distributionManagement> (el flujo del CI de un equipo)
#   4  otro proyecto declara la dependencia y la resuelve de `…/artifacts/maven` con sumas
#      ESTRICTAS (-C), el bloqueo de http:// anulado sólo para ORE (AR0 P4); el jar es el publicado
#   5  la coordenada en otro registro → 409
#
#   bash pruebas-de-fuego/artifacts/ar1-local.sh
#
# Necesita Docker y el volumen `ore-target-q1` (compila ahí). Credenciales de prueba, de cada corrida.
set -u
RAIZ="$(cd "$(dirname "$0")/../.." && (pwd -W 2>/dev/null || pwd))"
export MSYS_NO_PATHCONV=1
VERSITY_IMG=versity/versitygw@sha256:30292fc2eeacc67a36993b01f7a7a5e3361a19cced0e80c1d71cfa2a4b0a2499
AWS_IMG=amazon/aws-cli@sha256:e3e329e1d2894b7b4bbb0aacacd0a155262159b2e7a3b4275eb1f24046d3e06c
MAVEN_IMG=maven:3.9-eclipse-temurin-21
RED=ore-ar1
SECRETO="ar1-$(date +%s)-$RANDOM"
VK="k$RANDOM$RANDOM"; VS="s$RANDOM$RANDOM$RANDOM"

limpiar() { docker rm -f ar1-s3 ar1-ore >/dev/null 2>&1; docker volume rm -f ar1-s3 >/dev/null 2>&1; }
limpiar; [ -n "${CONSERVAR:-}" ] || trap limpiar EXIT
docker network create $RED >/dev/null 2>&1 || true

# ── el S3 local ─────────────────────────────────────────────────────────────
docker volume create ar1-s3 >/dev/null
docker run -d --name ar1-s3 --network $RED -v ar1-s3:/datos "$VERSITY_IMG" \
  --access "$VK" --secret "$VS" --port :7070 posix /datos >/dev/null || exit 2
sleep 2
docker run --rm --network $RED -e AWS_ACCESS_KEY_ID=$VK -e AWS_SECRET_ACCESS_KEY=$VS -e AWS_DEFAULT_REGION=us-east-1 \
  "$AWS_IMG" --endpoint-url http://ar1-s3:7070 s3api create-bucket --bucket lago >/dev/null || exit 2

# ── ore-serve y ore-medios: compilar y arrancar ─────────────────────────────
docker run -d --name ar1-ore --network $RED -v "$RAIZ:/src" -v ore-cargo:/usr/local/cargo/registry \
  -v ore-target-q1:/target -e CARGO_TARGET_DIR=/target -w /src \
  -e ORE_STORE=r2 -e ORE_R2_S3_ENDPOINT=http://ar1-s3:7070 -e ORE_R2_BUCKET=lago -e ORE_R2_REGION=us-east-1 \
  -e ORE_R2_ACCESS_KEY_ID=$VK -e ORE_R2_SECRET_ACCESS_KEY=$VS \
  -e ORE_MEDIOS_DIRECCION=127.0.0.1:8097 -e ORE_ARTEFACTOS_SECRETO=$SECRETO \
  rust:1-bookworm sh -c '
    set -e
    cargo build -q -p ore-serve -p ore-medios -p ore-cli 2>&1 | tail -5
    A=/tmp/arbol; rm -rf $A; mkdir -p $A
    cp -r /src/vendor/oos/conformance/v1alpha34/valid/a-maven-registry/input/. $A/
    # Un segundo registro, para la coordenada que ya tiene dueño.
    sed "s/name: libs/name: otras/" $A/packages/banco/riesgo/registries/libs.yaml > $A/packages/banco/riesgo/registries/otras.yaml
    git -C $A init -q && git -C $A -c user.name=t -c user.email=t@t add -A && git -C $A -c user.name=t -c user.email=t@t commit -qm arbol
    /target/debug/ore-medios > /tmp/medios.log 2>&1 &
    /target/debug/ore-serve --repo $A --ore /target/debug/ore --bind 0.0.0.0:8080 --identidad cabecera --no-es-produccion > /tmp/serve.log 2>&1 &
    touch /tmp/listo; wait' >/dev/null || exit 2
echo "· compilando y arrancando (la primera vez tarda)…"
for _ in $(seq 1 900); do
  docker exec ar1-ore sh -c 'test -f /tmp/listo && curl -s -o /dev/null http://127.0.0.1:8080/salud' 2>/dev/null && break
  docker ps -q -f name=ar1-ore | grep -q . || { echo "✗ ar1-ore murió"; docker logs ar1-ore | tail -20; exit 2; }
  sleep 2
done

# Un token como los firma ore-serve: ore_art_<hex carga>.<hex hmac-sha256(sha256(secreto), carga)>.
token() { # persona registro verbo
  docker exec ar1-ore sh -c '
    c=$(printf "{\"e\":%s,\"p\":\"%s\",\"r\":\"%s\",\"v\":\"%s\"}" $(( $(date +%s) + 600 )) "$1" "$2" "$3")
    k=$(printf %s "$ORE_ARTEFACTOS_SECRETO" | sha256sum | cut -c1-64)
    f=$(printf %s "$c" | openssl dgst -sha256 -mac HMAC -macopt hexkey:$k | sed "s/.*= //")
    printf "ore_art_%s.%s" "$(printf %s "$c" | od -An -tx1 | tr -d " \n")" "$f"' _ "$1" "$2" "$3"
}
PUBLICAR=$(token persona:ana banco.riesgo.libs publicar)
OTRO=$(token persona:ana banco.riesgo.otras publicar)
LEER=$(token capa '*' leer)

docker run --rm --network $RED -e PUBLICAR="$PUBLICAR" -e OTRO="$OTRO" -e LEER="$LEER" "$MAVEN_IMG" bash -c '
  fallos=0; falla() { echo "  ✗ $*"; fallos=$((fallos+1)); }; dice() { echo "  ✓ $*"; }
  cd /tmp; U=http://ar1-ore:8080/artifacts/banco/riesgo/libs/maven
  ajustes() { # token -> settings.xml (y el bloqueo de http:// anulado sólo para ORE)
    printf "<settings><mirrors><mirror><id>maven-default-http-blocker</id><mirrorOf>ninguno</mirrorOf><url>http://0.0.0.0/</url></mirror></mirrors><servers><server><id>ore</id><username>publicar</username><password>%s</password></server></servers></settings>" "$1"
  }
  ajustes "$PUBLICAR" > pub.xml; ajustes "$OTRO" > otro.xml; ajustes "" > sin.xml; ajustes "$LEER" > leer.xml
  D=org.apache.maven.plugins:maven-deploy-plugin:3.1.3:deploy-file
  # Un jar con una clase de verdad.
  mkdir -p src/com/banco/riesgo out
  printf "package com.banco.riesgo;\npublic class Calculadora { public static int riesgo() { return 42; } }\n" > src/com/banco/riesgo/Calculadora.java
  javac -d out src/com/banco/riesgo/Calculadora.java && jar cf motor-riesgo-1.0.0.jar -C out .
  publicar() { # ajustes version fichero -> codigo de Maven
    mvn -B -q -s $1 $D -Durl=$U -DrepositoryId=ore -Dfile=$3 -DgroupId=com.banco -DartifactId=motor-riesgo -Dversion=$2 -Dpackaging=jar > /tmp/log 2>&1 && echo ok || (grep -o "status code: [0-9]*" /tmp/log | head -1 || tail -3 /tmp/log)
  }
  echo "1 · publicar"
  T0=$(date +%s%N); r=$(publicar pub.xml 1.0.0 motor-riesgo-1.0.0.jar); T=$(( ($(date +%s%N)-T0)/1000000 ))
  [ "$r" = ok ] && dice "deploy-file 1.0.0 en $T ms (sin las esperas de Expect: 100-continue)" || falla "deploy-file 1.0.0: $r"
  head -c 20000000 /dev/urandom > grande.bin; mkdir -p g && cp grande.bin g/ && jar cf0 grande.jar -C g .
  r=$(mvn -B -q -s pub.xml $D -Durl=$U -DrepositoryId=ore -Dfile=grande.jar -DgroupId=com.banco -DartifactId=grande -Dversion=1.0.0 -Dpackaging=jar > /tmp/log 2>&1 && echo ok || tail -3 /tmp/log)
  [ "$r" = ok ] && dice "un jar de 20 MB (por el temporal de ore-medios)" || falla "20 MB: $r"
  echo "2 · lo que no entra"
  r=$(publicar pub.xml 1.0.0 motor-riesgo-1.0.0.jar); [ "$r" = "status code: 409" ] && dice "republicar 1.0.0 → 409" || falla "republicar: $r"
  cp motor-riesgo-1.0.0.jar x.jar
  r=$(publicar pub.xml 1.1.0-SNAPSHOT x.jar); [ "$r" = "status code: 400" ] && dice "-SNAPSHOT → 400" || falla "snapshot: $r"
  r=$(publicar sin.xml 1.2.0 x.jar); [ "$r" = "status code: 401" ] && dice "sin token → 401" || falla "sin token: $r"
  r=$(publicar otro.xml 1.2.0 x.jar); [ "$r" = "status code: 403" ] && dice "token de otro registro → 403" || falla "otro registro: $r"
  echo "3 · mvn deploy desde un proyecto"
  mkdir -p proyecto/src/main/java/com/banco/riesgo && cp src/com/banco/riesgo/Calculadora.java proyecto/src/main/java/com/banco/riesgo/
  cat > proyecto/pom.xml <<X
<project><modelVersion>4.0.0</modelVersion><groupId>com.banco</groupId><artifactId>motor-riesgo</artifactId><version>1.1.0</version>
<properties><maven.compiler.release>21</maven.compiler.release></properties>
<distributionManagement><repository><id>ore</id><url>$U</url></repository></distributionManagement></project>
X
  (cd proyecto && mvn -B -q -s ../pub.xml deploy > /tmp/log 2>&1) && dice "mvn deploy 1.1.0 (jar y pom de un proyecto)" || { falla "mvn deploy"; tail -5 /tmp/log; }
  echo "4 · resolverlo como la capa de la JVM"
  mkdir -p consumidor && cat > consumidor/pom.xml <<X
<project><modelVersion>4.0.0</modelVersion><groupId>x</groupId><artifactId>x</artifactId><version>1</version>
<repositories><repository><id>ore</id><url>http://ar1-ore:8080/artifacts/maven</url></repository></repositories>
<dependencies><dependency><groupId>com.banco</groupId><artifactId>motor-riesgo</artifactId><version>1.0.0</version></dependency></dependencies></project>
X
  (cd consumidor && mvn -B -C -s ../leer.xml -Dmaven.repo.local=/tmp/m2 org.apache.maven.plugins:maven-dependency-plugin:3.8.1:copy-dependencies -DoutputDirectory=/tmp/jars > /tmp/log 2>&1) \
    && dice "resuelto con sumas estrictas (-C) desde …/artifacts/maven" || { falla "resolver"; grep -m3 "ERROR" /tmp/log; }
  [ "$(sha256sum < /tmp/jars/motor-riesgo-1.0.0.jar)" = "$(sha256sum < motor-riesgo-1.0.0.jar)" ] && dice "el jar resuelto es el publicado, byte a byte" || falla "el jar resuelto no es el publicado"
  (mkdir -p usa && cd usa && printf "public class Usa { public static void main(String[] a) { System.out.println(com.banco.riesgo.Calculadora.riesgo()); } }\n" > Usa.java && javac -cp /tmp/jars/motor-riesgo-1.0.0.jar Usa.java && java -cp /tmp/jars/motor-riesgo-1.0.0.jar:. Usa) | grep -qx 42 \
    && dice "una clase importa com.banco.riesgo.Calculadora y da 42" || falla "la clase no se usa"
  grep -q "<version>1.1.0</version>" <(curl -s -u x:$LEER http://ar1-ore:8080/artifacts/maven/com/banco/motor-riesgo/maven-metadata.xml) \
    && dice "maven-metadata.xml generado con 1.0.0 y 1.1.0" || falla "la metadata"
  echo "5 · la coordenada es de un registro"
  r=$(mvn -B -q -s otro.xml $D -Durl=http://ar1-ore:8080/artifacts/banco/riesgo/otras/maven -DrepositoryId=ore -Dfile=x.jar -DgroupId=com.banco -DartifactId=motor-riesgo -Dversion=9.9.9 -Dpackaging=jar > /tmp/log 2>&1 && echo ok || grep -o "status code: [0-9]*" /tmp/log | head -1)
  [ "$r" = "status code: 409" ] && dice "com.banco:motor-riesgo en banco.riesgo.otras → 409" || falla "otro registro misma coordenada: $r"
  echo; [ $fallos -eq 0 ] && echo "AR1 local: todo en verde" || echo "AR1 local: $fallos fallo(s)"
  exit $fallos'
r=$?
echo "── lo que dijo ore-serve (artefactos) ──"
docker exec ar1-ore sh -c 'grep -a "artifacts" /tmp/serve.log | tail -25'
exit $r
