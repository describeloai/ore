# Laboratorio de Apache Phoenix para el origen 0071: HBase en modo independiente
# (ZooKeeper embebido) + el servidor de Phoenix + el Phoenix Query Server (PQS).
# No hay imagen oficial con PQS; esto se arma desde los binarios de Apache.
#
#   docker build -t ore-phoenix-lab pruebas-de-fuego/phoenix
#   docker run -d --name phx -p 8765:8765 ore-phoenix-lab
#
# El PQS 6.0.0 publicado está compilado contra el cliente de Phoenix 5.1 (con javax.servlet
# empaquetado): con Phoenix 5.2+ no arranca (PHOENIX-6861: «not a javax.servlet.Servlet»), y
# habría que compilarlo sin el perfil `shade-javax-servlet`. Por eso el laboratorio es 5.1.3.
FROM eclipse-temurin:11-jdk

ARG HBASE=2.4.18
ARG LINEA=2.4
ARG PHOENIX=5.1.3
ARG PQS=6.0.0
ENV HBASE_HOME=/opt/hbase PQS_HOME=/opt/pqs PHOENIX=${PHOENIX} LINEA=${LINEA}

RUN apt-get update && apt-get install -y --no-install-recommends curl procps \
    && rm -rf /var/lib/apt/lists/*

# archive.apache.org va a ~170 KB/s: HBase y el PQS salen de un espejo y los jars de Phoenix
# de Maven Central (los mismos ficheros, el tarball de Phoenix pesa 400 MB).
ARG ESPEJO=https://mirrors.huaweicloud.com/apache
ARG MAVEN=https://repo1.maven.org/maven2/org/apache/phoenix
RUN mkdir -p /opt/hbase /opt/pqs \
    && curl -fsSL ${ESPEJO}/hbase/${HBASE}/hbase-${HBASE}-bin.tar.gz | tar xz -C /opt/hbase --strip-components=1 \
    && curl -fsSL -o /opt/hbase/lib/phoenix-server-hbase-${LINEA}-${PHOENIX}.jar ${MAVEN}/phoenix-server-hbase-${LINEA}/${PHOENIX}/phoenix-server-hbase-${LINEA}-${PHOENIX}.jar \
    && curl -fsSL ${ESPEJO}/phoenix/phoenix-queryserver-${PQS}/phoenix-queryserver-${PQS}-bin.tar.gz | tar xz -C /opt/pqs --strip-components=1 \
    && curl -fsSL -o /opt/pqs/phoenix-client-embedded.jar ${MAVEN}/phoenix-client-embedded-hbase-${LINEA}/${PHOENIX}/phoenix-client-embedded-hbase-${LINEA}-${PHOENIX}.jar \
    && curl -fsSL -o /opt/pqs/lib/commons-collections-3.2.2.jar https://repo1.maven.org/maven2/commons-collections/commons-collections/3.2.2/commons-collections-3.2.2.jar

COPY hbase-site.xml /opt/hbase/conf/hbase-site.xml
COPY arranque.sh /arranque.sh
RUN chmod +x /arranque.sh
EXPOSE 8765
ENTRYPOINT ["/arranque.sh"]
