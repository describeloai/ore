#!/usr/bin/env bash
# P5·5 en el laboratorio (ADR 0058): el pool. `ep-…-pooler.europe-west1.pg.paladio.io` es la misma VM
# por su pgbouncer (6432, modo transaction, el de la imagen de Neon): muchas conexiones de cliente
# sobre pocas de servidor. Hecho cuando, por el proxy de Neon y con verify-full:
#   · más clientes a la vez que los que admite Postgres (max_connections + 40) por -pooler, sin fallos,
#     con a lo sumo el pool por base en Postgres (los dos, de la API: P5·5b);
#   · los mismos directos, sin pool, NO caben: es lo que el pool resuelve;
#   · el protocolo extendido (-M extended) también, y otra contraseña por el pool, fuera.
#
#   lab.sh arriba && p55.sh
source "$(dirname "$0")/lab.sh"
fallos=0
P="p55-$(date +%s | tail -c 6)"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p55\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(campo "$(pide demo GET "/v1/postgres/proyectos/$P/ramas/main/endpoints/principal")" vm)
[ -n "$VM" ] || { echo "✗ no crea: ${r:0:300}"; exit 1; }
reconcilia "$VM" computo-a
echo "── $P en computo-a ($VM), rol $ROL"

# por SNI HOST [PGOPTS…] → pgbench/psql con libpq por el proxy (PGHOST = el nombre, PGHOSTADDR = el proxy)
por() { local h=$1; shift
  dc exec -T -e PGPASSWORD="${CLAVE_USADA:-$CLAVE}" -e PGHOST="$h.$DOMINIO" -e PGHOSTADDR=172.29.51.20 -e PGPORT=4432 \
    -e PGUSER="$ROL" -e PGDATABASE="$P" -e PGSSLMODE=verify-full -e PGSSLROOTCERT=/llaves/tls/tls.crt \
    -e PGCONNECT_TIMEOUT=15 cliente "$@"; }
en_postgres() { dc exec -T computo-a psql -U postgres -p 5432 -Atc "$1"; }

por "$VM" pgbench -i -s 2 -q >/dev/null 2>&1 && echo "  ✓ pgbench -i por el endpoint directo" || { echo "  ✗ pgbench -i"; exit 1; }
r=$(por "$VM-pooler" psql -Atc "select current_user || ' ' || coalesce(current_setting('application_name'), '')" 2>&1 | tail -1)
case "$r" in "$ROL "*) echo "  ✓ por -pooler entra ($r)";; *) echo "  ✗ por -pooler: $r"; fallos=$((fallos+1));; esac

EP=$(pide demo GET "/v1/postgres/proyectos/$P/ramas/main/endpoints/principal")
MAX=$(campo "$EP" conexiones maximas); POOL=$(campo "$EP" conexiones pool_por_base); N=$((MAX + 40))
echo "── $N clientes a la vez, 20 s (Postgres admite $MAX; pool por base $POOL)"
pico() { local m=0 n; while [ ! -f "$LAB_TMP/fin" ]; do
  n=$(en_postgres "select count(*) from pg_stat_activity where usename = '$ROL' and backend_type = 'client backend'"); [ "${n:-0}" -gt $m ] && m=$n; sleep 0.5
  done; echo $m > "$LAB_TMP/pico"; }
LAB_TMP=$(mktemp -d); pico &
s=$(por "$VM-pooler" pgbench -n -c $N -j 8 -T 20 2>&1); touch "$LAB_TMP/fin"; wait
m=$(cat "$LAB_TMP/pico"); tps=$(echo "$s" | sed -n 's/^tps = \([0-9.]*\).*/\1/p'); fallidas=$(echo "$s" | sed -n 's/^number of failed transactions: \([0-9]*\).*/\1/p')
hechas=$(echo "$s" | sed -n 's/^number of transactions actually processed: \([0-9]*\).*/\1/p')
[ -n "$hechas" ] && [ "${fallidas:-0}" = 0 ] && echo "  ✓ por -pooler: $hechas transacciones, ${tps%.*} tps, 0 fallidas" \
  || { echo "  ✗ por -pooler: $(echo "$s" | grep -iE 'error|fatal|failed' | head -2)"; fallos=$((fallos+1)); }
[ "$m" -gt 0 ] && [ "$m" -le "$POOL" ] && echo "  ✓ en Postgres, como mucho $m conexiones del rol (pool $POOL, max_connections $MAX)" \
  || { echo "  ✗ $m conexiones en Postgres: el pool no agrupa"; fallos=$((fallos+1)); }
s=$(por "$VM" pgbench -n -c $N -j 8 -T 5 2>&1)
echo "$s" | grep -qiE "too many clients|remaining connection slots|connection to server .* failed" \
  && echo "  ✓ los mismos $N directos no caben: $(echo "$s" | grep -oiE 'too many clients already|remaining connection slots are reserved[^\"]*' | head -1)" \
  || { echo "  ✗ directos no falla (¿max_connections?): $(echo "$s" | tail -2 | tr '\n' ' ')"; fallos=$((fallos+1)); }
rm -rf "$LAB_TMP"

echo "── el protocolo extendido y otra contraseña"
s=$(por "$VM-pooler" pgbench -n -M extended -c 20 -j 4 -T 5 2>&1)
[ "$(echo "$s" | sed -n 's/^number of failed transactions: \([0-9]*\).*/\1/p')" = 0 ] && echo "  ✓ -M extended por el pool, 0 fallidas" \
  || { echo "  ✗ -M extended: $(echo "$s" | grep -iE 'error|failed' | head -2)"; fallos=$((fallos+1)); }
r=$(CLAVE_USADA=mala por "$VM-pooler" psql -Atc 'select 1' 2>&1 | tail -1)
[ "$r" = 1 ] && { echo "  ✗ entra por el pool con otra contraseña"; fallos=$((fallos+1)); } || echo "  ✓ otra contraseña por el pool, no: ${r:0:100}"

pide demo DELETE "/v1/postgres/proyectos/$P" >/dev/null; barre
echo
[ $fallos = 0 ] && echo "P5·5 (laboratorio) ✓ todo" || { echo "P5·5 (laboratorio) ✗ $fallos fallos"; exit 1; }
