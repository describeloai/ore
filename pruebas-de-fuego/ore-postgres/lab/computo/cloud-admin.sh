#!/bin/sh
# Como en la VM: cloud_admin es superusuario y entra sin contraseña desde la propia máquina
# (pgbouncer lo usa para leer el verificador del rol que se conecta: auth_user + auth_query).
set -e
psql -v ON_ERROR_STOP=1 -U postgres -c "create role cloud_admin superuser login"
sed -i '1i host all cloud_admin 127.0.0.1/32 trust\nhost all cloud_admin ::1/128 trust' "$PGDATA/pg_hba.conf"
