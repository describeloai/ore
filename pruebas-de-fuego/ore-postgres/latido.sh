#!/bin/sh
# uso: latido.sh <via> <host> — escribe cada 0,2 s por UNA sesión; si la conexión cae, psql sale.
i=0
while [ $i -lt 1500 ]; do i=$((i+1)); echo "insert into mig(via,n) values ('$1', $i);"; sleep 0.2; done \
  | PGPASSWORD=cloud_admin psql -h "$2" -p 55433 -U cloud_admin -d postgres -q -v ON_ERROR_STOP=1 2>&1
echo "$1 rc=$?"
