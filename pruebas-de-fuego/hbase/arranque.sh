#!/bin/bash
# HBase independiente vigilado, con REST (8080) y Thrift2 (9090) en primer y segundo plano.
set -e
mkdir -p /data
"$HBASE_HOME/bin/start-hbase.sh"
"$HBASE_HOME/bin/hbase" rest start -p 8080 >/tmp/rest.log 2>&1 &
REST=$!
"$HBASE_HOME/bin/hbase" thrift2 start -p 9090 >/tmp/thrift2.log 2>&1 &
(
  while kill -0 "$REST" 2>/dev/null; do
    sleep 15
    if ! pgrep -f org.apache.hadoop.hbase.master.HMaster >/dev/null; then
      echo "arranque: HBase no esta; se levanta de nuevo" >&2
      "$HBASE_HOME/bin/start-hbase.sh" || true
    fi
  done
) &
wait "$REST"
