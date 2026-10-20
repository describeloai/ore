#!/usr/bin/env bash
# ORE 0068 C4·2 · El cotejo de Enrich con PostGIS, en un laboratorio de Docker:
#   1. ORE genera el SQL de DuckDB de `arbol/…/cotejo.enrich.yaml` (la prueba `escribe_el_sql_de_un_arbol`);
#   2. PostGIS crea los datos y su respuesta, escrita a mano (`postgis.sql`);
#   3. el DuckDB de la imagen del puesto corre el SQL de ORE sobre esos datos y lo coteja (`cotejar.py`);
#   4. y el Spark + Sedona de la imagen pesada, el de Spark (C4·3), con el objetivo en 8 particiones.
#   bash pruebas-de-fuego/geo/c4/cotejo/cotejar.sh        # y BORRAR=1 para retirar el laboratorio
set -uo pipefail
aqui="$(cd "$(dirname "$0")" && pwd)"
raiz="$(cd "$aqui/../../../.." && pwd)"
NET=c4-cotejo; PG=c4-cotejo-pg; VOL=c4-cotejo-x
if [ "${BORRAR:-0}" = 1 ]; then
  docker rm -f $PG >/dev/null 2>&1; docker volume rm $VOL >/dev/null 2>&1; docker network rm $NET >/dev/null 2>&1
  echo "== laboratorio retirado"; exit 0
fi
win() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
docker network inspect $NET >/dev/null 2>&1 || docker network create $NET >/dev/null
docker volume inspect $VOL >/dev/null 2>&1 || docker volume create $VOL >/dev/null
export MSYS_NO_PATHCONV=1
docker run --rm -v $VOL:/x alpine chmod 777 /x

echo "== 1 · el SQL de ORE"
docker run --rm -v "$(win "$raiz"):/src" -v ore-cargo:/usr/local/cargo/registry -v ore-target:/t -v $VOL:/x \
  -e CARGO_TARGET_DIR=/t/c4 -w /src \
  -e ENRICH_ARBOL=/src/pruebas-de-fuego/geo/c4/cotejo/arbol \
  -e ENRICH_FICHERO=packages/ventas/etl/cotejo.enrich.yaml -e ENRICH_SALIDA=/x \
  rust:1 bash -c 'set -o pipefail; cargo test -q -p ore-core --lib -- --ignored escribe_el_sql_de_un_arbol 2>&1 | tail -3' || exit 1
docker run --rm -v $VOL:/x alpine cat /x/enrich.sql

echo "== 2 · PostGIS"
docker ps --format '{{.Names}}' | grep -qx $PG || docker run -d --name $PG --network $NET -v $VOL:/x \
  -e POSTGRES_PASSWORD=cotejo-local postgis/postgis:16-3.4 >/dev/null
# La imagen crea `postgis` al arrancar: se espera a que esté (antes, `create extension` choca con ella).
until docker exec $PG psql -U postgres -tAc "select 1 from pg_extension where extname = 'postgis'" 2>/dev/null | grep -q 1; do sleep 1; done
docker cp "$(win "$aqui/postgis.sql")" $PG:/tmp/postgis.sql
t=$(date +%s); docker exec $PG psql -q -v ON_ERROR_STOP=1 -U postgres -f /tmp/postgis.sql || exit 1
echo "== PostGIS: $(( $(date +%s) - t )) s"

echo "== 3 · DuckDB con el SQL de ORE, contra PostGIS"
docker run --rm -v $VOL:/x -v "$(win "$aqui"):/c:ro" -e HOME=/tmp ore-puesto-python:local python3 /c/cotejar.py
r=$?
echo "== 4 · Spark + Sedona con el SQL de ORE, contra PostGIS"
docker run --rm -v $VOL:/x -v "$(win "$aqui"):/c:ro" -e HOME=/tmp -e MOTOR=spark ore-puesto-spark:local python3 /c/cotejar.py
exit $(( r || $? ))
