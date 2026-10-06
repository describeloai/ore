#!/usr/bin/env bash
# D0b·3 · autoescalado: carga → sube; sin carga → baja; una sesión abierta todo el rato.
read P IP < /c/tmp/d0b/vm-pod.txt
E="kubectl -n d0-neon exec cliente -- env PGPASSWORD=cloud_admin"
ts() { date -u +%H:%M:%S; }
LOG=/c/tmp/d0b/medida-escalado.txt; : > $LOG
# seguimiento de la VM
( while true; do echo "$(ts) VM cpus=$(kubectl -n d0-neon get neonvm pg-d0 -o jsonpath='{.status.cpus}') mem=$(kubectl -n d0-neon get neonvm pg-d0 -o jsonpath='{.status.memorySize}')" >> $LOG; sleep 2; done ) & W=$!
# (d) una sesión abierta con un contador: si la conexión cae, psql sale y se ve
( $E psql -h $IP -p 55433 -U cloud_admin -d postgres -At -c "create table if not exists latido(n int, t timestamptz default now())" >/dev/null 2>&1
  $E sh -c "for i in \$(seq 1 600); do echo \"insert into latido(n) values (\$i);\"; sleep 1; done" \
    | $E psql -h $IP -p 55433 -U cloud_admin -d postgres -q -v ON_ERROR_STOP=1 > /dev/null 2>/c/tmp/d0b/latido.err; echo "$(ts) SESIÓN TERMINÓ rc=$?" >> $LOG ) & S=$!
echo "$(ts) -- reposo 30 s" >> $LOG; sleep 30
echo "$(ts) -- pgbench -i -s 20" >> $LOG
$E pgbench -h $IP -p 55433 -U cloud_admin -i -s 20 -q postgres > /dev/null 2>&1
echo "$(ts) -- CARGA CPU: pgbench 8 clientes 150 s" >> $LOG
$E pgbench -h $IP -p 55433 -U cloud_admin -c 8 -j 4 -T 150 -S postgres 2>&1 | grep -E "tps|latency average" >> $LOG
echo "$(ts) -- CARGA MEMORIA: sort grande con work_mem 1GB" >> $LOG
$E psql -h $IP -p 55433 -U cloud_admin -d postgres -Atc "set work_mem='1500MB'; select count(*) from (select * from pgbench_accounts order by filler desc, abalance) s; select count(*) from (select * from pgbench_accounts order by filler, aid desc) s;" >> $LOG 2>&1
echo "$(ts) -- SIN CARGA 240 s" >> $LOG; sleep 240
kill $W
echo "$(ts) -- latido: $($E psql -h $IP -p 55433 -U cloud_admin -d postgres -Atc "select count(*), max(n), max(t)-min(t) from latido")" >> $LOG
kill $S 2>/dev/null
