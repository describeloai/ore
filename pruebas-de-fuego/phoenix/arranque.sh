#!/bin/bash
# HBase independiente en segundo plano y el PQS en primer plano.
# Los scripts del PQS 6.0.0 son Python 2 (no corren con python3): se lanza la clase con java.
# El cliente "embedded" de Phoenix no trae Hadoop ni el registro: se toman de la carpeta lib
# de HBase, como hace un PQS puesto en un nodo del cluster.
set -e
mkdir -p /data
"$HBASE_HOME/bin/start-hbase.sh"
CP="$HBASE_HOME/conf:$PQS_HOME/phoenix-queryserver-6.0.0.jar:$PQS_HOME/phoenix-client-embedded.jar:$PQS_HOME/lib/*:$HBASE_HOME/lib/*:$HBASE_HOME/lib/client-facing-thirdparty/*"
exec java -Xmx512m -cp "$CP" -Dphoenix.queryserver.http.port=8765 \
  org.apache.phoenix.queryserver.server.QueryServer
