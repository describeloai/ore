# Laboratorio de HBase puro para el origen 0072: HBase independiente (ZooKeeper embebido) con los
# gateways REST y Thrift2. Sin Phoenix: el de Phoenix esta en ../phoenix.
#
#   docker build -t ore-hbase-lab pruebas-de-fuego/hbase
#   docker run -d --name hb -p 2181:2181 -p 16000:16000 -p 16020:16020 -p 16030:16030 -p 8080:8080 -p 9090:9090 ore-hbase-lab
#
# Los puertos se publican con el mismo numero dentro y fuera y HBase se anuncia como 127.0.0.1:
# el cliente nativo descubre los RegionServers por ZooKeeper y se conecta al nombre que ellos anuncian.
FROM eclipse-temurin:11-jdk

ARG HBASE=2.4.18
ARG ESPEJO=https://mirrors.huaweicloud.com/apache
ENV HBASE_HOME=/opt/hbase

RUN apt-get update && apt-get install -y --no-install-recommends curl procps iproute2 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /opt/hbase \
    && curl -fsSL ${ESPEJO}/hbase/${HBASE}/hbase-${HBASE}-bin.tar.gz | tar xz -C /opt/hbase --strip-components=1

COPY hbase-site.xml /opt/hbase/conf/hbase-site.xml
COPY arranque.sh /arranque.sh
RUN chmod +x /arranque.sh
EXPOSE 2181 16000 16020 16030 8080 9090
ENTRYPOINT ["/arranque.sh"]
