#!/usr/bin/env bash
# El laboratorio de ClickHouse (ADR 0077 §4): un servidor con el usuario `kit` (sin contraseña, como pide el conector por HTTP en claro)
# y los perfiles que las pruebas necesitan. Lo usa el CI y se puede usar en local.
#
#   bash pruebas-de-fuego/clickhouse/laboratorio.sh <nombre> <versión> <puerto-http>
#   bash pruebas-de-fuego/clickhouse/laboratorio.sh ore-clickhouse-24 24.8 8124
#
# Deja: la base `kit`; `kit` (escribe en `kit`, ve `system.processes` y `system.parts`, puede matar consultas); y tres usuarios de
# lectura que son perfiles: `ore_r1` (`readonly=1`), `ore_r2` (`readonly=2`) y `ore_brk` (`result_overflow_mode='break'`, que el
# conector rechaza). El administrador es `ore`/`ore` y sólo se usa desde dentro del contenedor.
set -eu
NOMBRE=${1:?nombre del contenedor}
VERSION=${2:?versión de ClickHouse (24.8, 25.8, 26.4…)}
PUERTO=${3:?puerto HTTP en el anfitrión}

docker rm -f "$NOMBRE" >/dev/null 2>&1 || true
docker run -d --name "$NOMBRE" -p "$PUERTO:8123" \
  -e CLICKHOUSE_USER=ore -e CLICKHOUSE_PASSWORD=ore -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  --ulimit nofile=262144:262144 \
  "clickhouse/clickhouse-server:$VERSION" >/dev/null

for _ in $(seq 1 60); do
  docker exec "$NOMBRE" clickhouse-client -u ore --password ore -q 'SELECT 1' >/dev/null 2>&1 && break
  sleep 2
done
docker exec "$NOMBRE" clickhouse-client -u ore --password ore -q 'SELECT 1' >/dev/null

docker exec -i "$NOMBRE" clickhouse-client -u ore --password ore -n -m <<'SQL'
CREATE DATABASE IF NOT EXISTS kit;
CREATE USER IF NOT EXISTS kit IDENTIFIED WITH no_password;
GRANT ALL ON kit.* TO kit;
GRANT SELECT ON system.processes TO kit;
GRANT SELECT ON system.parts TO kit;
GRANT KILL QUERY ON *.* TO kit;
CREATE USER IF NOT EXISTS ore_r1 IDENTIFIED WITH no_password SETTINGS readonly = 1;
CREATE USER IF NOT EXISTS ore_r2 IDENTIFIED WITH no_password SETTINGS readonly = 2;
CREATE USER IF NOT EXISTS ore_brk IDENTIFIED WITH no_password SETTINGS result_overflow_mode = 'break', max_result_rows = 100, readonly = 2;
GRANT SELECT ON *.* TO ore_r1, ore_r2, ore_brk;
SQL

echo "$NOMBRE: ClickHouse $(curl -s "http://localhost:$PUERTO/?user=kit&query=SELECT+version()") en el puerto $PUERTO"
