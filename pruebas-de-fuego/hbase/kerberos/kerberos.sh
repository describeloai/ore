#!/bin/bash
# Levanta el laboratorio de HBase con Kerberos (X0b de 0072):  kerberos.sh [authentication|integrity|privacy]
#   red `krb`, KDC `kdc.ore.test`, HBase `hb.ore.test`, reino ORE.TEST; usuarios `lector` (clave: lector) e `intruso`.
set -e
export MSYS_NO_PATHCONV=1
AQUI="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
docker rm -f kdc hbk krbcli >/dev/null 2>&1 || true
docker network inspect krb >/dev/null 2>&1 || docker network create krb >/dev/null
docker volume rm -f krbkeys >/dev/null 2>&1 || true
# La imagen de Kerberos parte de la del laboratorio de HBase: si no esta (un runner de CI), se construye. `ESPEJO=` elige el espejo.
docker image inspect ore-hbase-lab >/dev/null 2>&1 || docker build -q -t ore-hbase-lab ${ESPEJO:+--build-arg ESPEJO="$ESPEJO"} "$AQUI/.." >/dev/null
docker build -q -t ore-hbase-krb -f "$AQUI/Dockerfile.hbase" "$AQUI" >/dev/null
docker build -q -t ore-kdc -f "$AQUI/Dockerfile.kdc" "$AQUI" >/dev/null
docker run -d --name kdc --network krb --hostname kdc.ore.test --network-alias kdc.ore.test -v krbkeys:/keytabs ore-kdc >/dev/null
sleep 6
docker run -d --name hbk --network krb --hostname hb.ore.test --network-alias hb.ore.test \
  -e PROTECCION="${1:-authentication}" -v krbkeys:/keytabs \
  -v "$AQUI/hbase-site.xml:/conf/hbase-site.xml:ro" -v "$AQUI/krb5.conf:/conf/krb5.conf:ro" -v "$AQUI/core-site.xml:/conf/core-site.xml:ro" -v "$AQUI/arranque-krb.sh:/arranque-krb.sh:ro" \
  --entrypoint /bin/bash ore-hbase-krb /arranque-krb.sh >/dev/null
echo "esperando a HBase..."; sleep 40
docker logs --tail 5 kdc 2>&1 | tail -3
