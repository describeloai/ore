#!/bin/sh
# Postgres y, cuando escucha por TCP (el servidor definitivo, no el de la inicialización), pgbouncer.
docker-entrypoint.sh postgres -p 5432 &
until pg_isready -q -h 127.0.0.1 -p 5432 -U postgres; do sleep 0.5; done
exec gosu postgres pgbouncer /etc/pgbouncer.ini
