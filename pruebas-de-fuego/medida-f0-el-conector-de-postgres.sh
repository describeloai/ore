#!/usr/bin/env bash
# MEDIDA F0 · M3 (ADR 0053): el conector de Postgres de hoy contra un Postgres de
# pruebas con un millon de filas, todo en Docker y sin credencial de nadie.
#
# Uso:  ORE_CONECTOR=<ruta al binario Linux de ore-read-postgres> \
#         bash pruebas-de-fuego/medida-f0-el-conector-de-postgres.sh
# Deja las cifras en $M3_SALIDA (por defecto ./m3.json).
set -eu
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
: "${ORE_CONECTOR:?ORE_CONECTOR: el binario Linux de ore-read-postgres}"
SALIDA="${M3_SALIDA:-$PWD/m3.json}"
FILAS="${M3_FILAS:-1000000}"
RED=ore-m3-$$; PG=ore-m3-pg-$$
limpiar() { docker rm -f "$PG" >/dev/null 2>&1 || true; docker network rm "$RED" >/dev/null 2>&1 || true; }
trap limpiar EXIT
export MSYS_NO_PATHCONV=1

docker network create "$RED" >/dev/null
docker run -d --name "$PG" --network "$RED" -e POSTGRES_USER=ore -e POSTGRES_PASSWORD=ore -e POSTGRES_DB=m3 \
  postgres:16-alpine -c max_connections=40 >/dev/null
for _ in $(seq 1 60); do docker exec "$PG" pg_isready -U ore -d m3 >/dev/null 2>&1 && break; sleep 1; done
docker exec -i "$PG" psql -q -U ore -d m3 <<SQL
create table ventas (id bigint primary key, pais text, importe numeric(10,2), creado timestamptz, nota text);
insert into ventas
select g, (array['ES','FR','DE','IT','PT','NL','BE','AT','IE','PL','SE','DK','FI','CZ','GR','HU','RO','BG','HR','SK'])[1 + g % 20],
       round((g % 10000) / 7.0, 2), timestamptz '2026-01-01' + (g || ' seconds')::interval,
       'nota ' || md5(g::text)
from generate_series(1, $FILAS) g;
analyze ventas;
SQL
echo "· ${FILAS} filas en public.ventas"

# Lo que cuesta en el propio origen, para comparar (psql dentro del contenedor)
echo "· en el origen, sin conector:"
docker exec "$PG" psql -U ore -d m3 -c '\timing on' -c 'select * from ventas limit 10' -c "select count(*) from ventas where pais = 'ES'" \
  | grep -E "^Time" | sed 's/^/    /'

docker run --rm --network "$RED" -v "$RAIZ/pruebas-de-fuego:/m:ro" -v "$ORE_CONECTOR:/usr/local/bin/ore-read-postgres:ro" \
  -v "$(dirname "$SALIDA"):/salida" -v ore-pip-cache:/root/.cache/pip \
  -e PG_URL="postgres://ore:ore@$PG:5432/m3?sslmode=disable" -e M3_FILAS="$FILAS" -e M3_SALIDA="/salida/$(basename "$SALIDA")" \
  python:3.12-slim bash -c 'apt-get update -qq >/dev/null && apt-get install -y -qq musl >/dev/null 2>&1; pip install -q pyarrow 2>/dev/null; python3 /m/medida-f0-el-conector-de-postgres.py'
