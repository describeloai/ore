#!/bin/sh
# ORE 0074 AR0 · previos P1 (mvn deploy), P2 (uv publish y twine) y P4 (el mismo GAV en ORE y en
# Central), contra el repositorio de juguete. Local, en Docker. Deja registro.jsonl y los .log en
# resultados/.
set -eu
AQUI=$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)
RES="$AQUI/resultados"; mkdir -p "$RES"; rm -rf "$RES"/*
export MSYS_NO_PATHCONV=1
CLAVE=token-de-juguete
docker rm -f ar0-juguete >/dev/null 2>&1 || true
docker network create ar0 >/dev/null 2>&1 || true
docker run -d --name ar0-juguete --network ar0 -e CLAVE=$CLAVE -e RAIZ=/res \
  -v "$AQUI/juguete.py:/juguete.py:ro" -v "$RES:/res" python:3.14-slim python -u /juguete.py >/dev/null
sleep 2

echo "── P1 · mvn deploy:deploy-file ──"
docker run --rm --network ar0 -v "$RES:/res" maven:3.9-eclipse-temurin-21 sh -c '
  set -e; cd /tmp; mkdir -p c/com/banco; echo "class X {}" > c/com/banco/X.txt; jar cf motor-riesgo-1.0.0.jar -C c .
  cat > s.xml <<X
<settings><servers><server><id>ore</id><username>publicar</username><password>'$CLAVE'</password></server></servers></settings>
X
  sed "s/'$CLAVE'/mala/" s.xml > mala.xml
  D="org.apache.maven.plugins:maven-deploy-plugin:3.1.3:deploy-file -Durl=http://ar0-juguete:8080/maven -DrepositoryId=ore -Dfile=motor-riesgo-1.0.0.jar -DgroupId=com.banco -DartifactId=motor-riesgo -Dversion=1.0.0 -Dpackaging=jar"
  echo "· publicar 1.0.0"; mvn -B -q -s s.xml $D > /res/p1-publicar.log 2>&1 && echo "  ok" || echo "  FALLA"
  echo "· republicar 1.0.0"; mvn -B -q -s s.xml $D > /res/p1-republicar.log 2>&1 && echo "  ok (!)" || { echo "  falla:"; grep -m1 -o "status code: [0-9]*[^,]*" /res/p1-republicar.log || tail -3 /res/p1-republicar.log; }
  cp motor-riesgo-1.0.0.jar motor-riesgo-1.0.1.jar
  echo "· publicar 1.0.1 con la clave mala"; mvn -B -q -s mala.xml $(echo $D | sed s/1.0.0/1.0.1/g) > /res/p1-mala.log 2>&1 && echo "  ok (!)" || { echo "  falla:"; grep -m1 -o "status code: [0-9]*[^,]*" /res/p1-mala.log || tail -3 /res/p1-mala.log; }
'

echo "── P4 · el mismo GAV en ORE y en Central ──"
docker run --rm --network ar0 -v "$RES:/res" maven:3.9-eclipse-temurin-21 sh -c '
  set -e; cd /tmp; mkdir -p f; echo falso > f/FALSO.txt; jar cf commons-lang3-3.17.0.jar -C f .
  cat > s.xml <<X
<settings><servers><server><id>ore</id><username>publicar</username><password>'$CLAVE'</password></server></servers></settings>
X
  mvn -B -q -s s.xml org.apache.maven.plugins:maven-deploy-plugin:3.1.3:deploy-file -Durl=http://ar0-juguete:8080/maven -DrepositoryId=ore \
    -Dfile=commons-lang3-3.17.0.jar -DgroupId=org.apache.commons -DartifactId=commons-lang3 -Dversion=3.17.0 -Dpackaging=jar > /res/p4-publicar.log 2>&1
  FALSO=$(sha1sum commons-lang3-3.17.0.jar | cut -c1-40)
  mkdir p; cat > p/pom.xml <<X
<project><modelVersion>4.0.0</modelVersion><groupId>x</groupId><artifactId>x</artifactId><version>1</version>
<repositories><repository><id>ore</id><url>http://ar0-juguete:8080/maven</url></repository></repositories>
<dependencies>
 <dependency><groupId>org.apache.commons</groupId><artifactId>commons-lang3</artifactId><version>3.17.0</version></dependency>
 <dependency><groupId>org.apache.commons</groupId><artifactId>commons-text</artifactId><version>1.12.0</version></dependency>
</dependencies></project>
X
  # Sin bloqueador: un espejo con el id del de Maven (maven-default-http-blocker) que no espeja nada.
  sed "s,<servers>,<mirrors><mirror><id>maven-default-http-blocker</id><mirrorOf>ninguno</mirrorOf><url>http://0.0.0.0/</url></mirror></mirrors><servers>," s.xml > abierto.xml
  cd p
  for ajuste in s abierto; do
    rm -rf /tmp/m2 /tmp/jars
    mvn -B -s ../$ajuste.xml -Dmaven.repo.local=/tmp/m2 org.apache.maven.plugins:maven-dependency-plugin:3.8.1:copy-dependencies -DoutputDirectory=/tmp/jars > /res/p4-resolver-$ajuste.log 2>&1 || { echo "  resolver falla"; tail -5 /res/p4-resolver-$ajuste.log; }
    echo "· con $ajuste.xml: $(grep -ci "blocked mirror" /res/p4-resolver-$ajuste.log) avisos de bloqueo"
    [ "$(sha1sum /tmp/jars/commons-lang3-3.17.0.jar | cut -c1-40)" = "$FALSO" ] && echo "  commons-lang3 3.17.0: EL DE ORE (falso)" || echo "  commons-lang3 3.17.0: el de Central"
    for f in /tmp/m2/org/apache/commons/commons-lang3/3.17.0/_remote.repositories /tmp/m2/org/apache/commons/commons-text/1.12.0/_remote.repositories; do echo "  $(basename $(dirname $(dirname $f))): $(grep "jar>" $f)"; done
  done
'

echo "── P2 · uv publish y twine upload ──"
docker run --rm --network ar0 -v "$RES:/res" python:3.14-slim sh -c '
  set -e; pip install -q uv twine 2>/dev/null; cd /tmp; mkdir -p br/src/banco_riesgo; cd br
  printf "[project]\nname = \"Banco_Riesgo\"\nversion = \"1.0.0\"\n[build-system]\nrequires = [\"hatchling\"]\nbuild-backend = \"hatchling.build\"\n" > pyproject.toml
  echo "def calcular(): return 42" > src/banco_riesgo/__init__.py
  uv build -q > /res/p2-build.log 2>&1; ls dist
  echo "· uv publish"; uv publish --publish-url http://ar0-juguete:8080/legacy/ -u __token__ -p '$CLAVE' dist/*.whl > /res/p2-uv.log 2>&1 && echo "  ok" || { echo "  falla"; tail -3 /res/p2-uv.log; }
  echo "· twine upload (el sdist)"; twine upload --non-interactive --repository-url http://ar0-juguete:8080/legacy/ -u __token__ -p '$CLAVE' dist/*.tar.gz > /res/p2-twine.log 2>&1 && echo "  ok" || { echo "  falla"; tail -3 /res/p2-twine.log; }
  echo "· republicar la rueda"; uv publish --publish-url http://ar0-juguete:8080/legacy/ -u __token__ -p '$CLAVE' dist/*.whl > /res/p2-re.log 2>&1 && echo "  ok (!)" || { echo "  falla:"; tail -2 /res/p2-re.log; }
  echo "· instalar desde el índice simple"; uv venv -q /tmp/v; VIRTUAL_ENV=/tmp/v uv pip install -q --index-url http://ar0-juguete:8080/simple/ banco-riesgo > /res/p2-instalar.log 2>&1 && /tmp/v/bin/python -c "import banco_riesgo; print(\"  importa:\", banco_riesgo.calcular())" || { echo "  falla"; tail -3 /res/p2-instalar.log; }
'
docker rm -f ar0-juguete >/dev/null
echo "── registro ──"; cat "$RES/registro.jsonl"
