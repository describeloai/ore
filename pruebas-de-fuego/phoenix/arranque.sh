#!/bin/bash
# HBase independiente en segundo plano (y vigilado) y el PQS en primer plano.
# Los scripts del PQS 6.0.0 son Python 2 (no corren con python3): se lanza la clase con java.
# El cliente "embedded" de Phoenix no trae Hadoop ni el registro: se toman de la carpeta lib
# de HBase, como hace un PQS puesto en un nodo del cluster.
set -e
mkdir -p /data
"$HBASE_HOME/bin/start-hbase.sh"
CP="$HBASE_HOME/conf:$PQS_HOME/phoenix-queryserver-6.0.0.jar:$PQS_HOME/phoenix-client-embedded.jar:$PQS_HOME/lib/*:$HBASE_HOME/lib/*:$HBASE_HOME/lib/client-facing-thirdparty/*"
java -Xmx${PQS_XMX:-1g} -cp "$CP" -Dphoenix.queryserver.http.port=8765 \
  org.apache.phoenix.queryserver.server.QueryServer &
PQS=$!
# Si HBase se aborta (una sesion de ZooKeeper caducada, un OOM), se levanta de nuevo con sus datos:
# el laboratorio no debe quedarse con el PQS vivo y nada detras.
(
  while kill -0 "$PQS" 2>/dev/null; do
    sleep 15
    if ! pgrep -f org.apache.hadoop.hbase.master.HMaster >/dev/null; then
      echo "arranque: HBase no esta; se levanta de nuevo" >&2
      "$HBASE_HOME/bin/start-hbase.sh" || true
    fi
  done
) &
wait "$PQS"
