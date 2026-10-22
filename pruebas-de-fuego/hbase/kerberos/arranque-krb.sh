#!/bin/bash
# Pone la configuracion con Kerberos (y la proteccion del RPC que se pida) y arranca como el laboratorio normal.
set -e
sed "s/\${PROTECCION}/${PROTECCION:-authentication}/" /conf/hbase-site.xml > /opt/hbase/conf/hbase-site.xml
cp /conf/krb5.conf /etc/krb5.conf
cp /conf/core-site.xml /opt/hbase/conf/core-site.xml
# El HBase independiente no hace el login con el keytab por si solo: la JVM usa la cache de tickets.
kinit -kt /keytabs/hbase.keytab hbase/hb.ore.test@ORE.TEST
# Sin REST ni Thrift: con Kerberos piden su propio principal y, al fallar, tiraban el contenedor. Aqui solo el RPC nativo.
mkdir -p /data
/opt/hbase/bin/start-hbase.sh
while pgrep -f org.apache.hadoop.hbase.master.HMaster >/dev/null; do sleep 15; done
