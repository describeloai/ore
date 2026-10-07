#!/usr/bin/env bash
# D0b·3 · autoescalado: carga → sube; sin carga → baja; una sesión abierta todo el rato (si la
# conexión cae, psql sale y se ve). Medido en B.5: 9 s a subir, ~3 min a bajar, 0 cortes.
export ORE_PG_COMPUTO=vm   # P3·7: las VMs viven en $ORE_PG_NS_COMPUTO (kc); clientes y almacenamiento en $ORE_PG_NS (k)
source "$(dirname "$0")/entorno.sh"
IP=$(leer "pod-$ORE_PG_VM") || exit 1
E="k exec cliente -- env PGPASSWORD=cloud_admin"
LOG=$ORE_PG_TRABAJO/medida-escalado.txt; : > "$LOG"
t() { date -u +%H:%M:%S; }
( while true; do echo "$(t) VM cpus=$(kc get neonvm "$ORE_PG_VM" -o jsonpath='{.status.cpus}') mem=$(kc get neonvm "$ORE_PG_VM" -o jsonpath='{.status.memorySize}')" >> "$LOG"; sleep 2; done ) & W=$!
# la sesión que escribe todo el rato corre DENTRO del clúster (nohup en `cliente`): por un `kubectl exec`
# desde fuera, un corte de la red de quien lanza la prueba la mataba y parecía un corte de la VM (P3·7)
$E psql -h "$IP" -p 55433 -U cloud_admin -d postgres -Atc "drop table if exists mig; create table mig(via text, n int, t timestamptz default clock_timestamp())" >/dev/null
k exec -i cliente -- sh -c 'cat > /tmp/latido.sh && chmod +x /tmp/latido.sh' < "$AQUI/latido.sh"
k exec cliente -- sh -c "nohup /tmp/latido.sh escalado $IP 2400 > /tmp/sesion-escalado.log 2>&1 &"   # 8 min
echo "$(t) -- reposo 30 s" >> "$LOG"; sleep 30
echo "$(t) -- pgbench -i -s 20" >> "$LOG"
$E pgbench -h "$IP" -p 55433 -U cloud_admin -i -s 20 -q postgres > /dev/null 2>&1
echo "$(t) -- CARGA CPU: pgbench 8 clientes 150 s" >> "$LOG"
$E pgbench -h "$IP" -p 55433 -U cloud_admin -c 8 -j 4 -T 150 -S postgres 2>&1 | grep -E "tps|latency average" >> "$LOG"
echo "$(t) -- CARGA MEMORIA: sort grande con work_mem 1500MB" >> "$LOG"
$E psql -h "$IP" -p 55433 -U cloud_admin -d postgres -Atc "set work_mem='1500MB'; select count(*) from (select * from pgbench_accounts order by filler desc, abalance) s; select count(*) from (select * from pgbench_accounts order by filler, aid desc) s;" >> "$LOG" 2>&1
echo "$(t) -- SIN CARGA 240 s" >> "$LOG"; sleep 240
kill $W
echo "$(t) -- latido: $($E psql -h "$IP" -p 55433 -U cloud_admin -d postgres -Atc "select count(*) || ' filas, última ' || max(n) || ', mayor hueco ' || round(max(d)::numeric,2) || ' s' from (select n, extract(epoch from t - lag(t) over (order by n)) d from mig where via='escalado') s") · sesión: $(k exec cliente -- cat /tmp/sesion-escalado.log | tail -1)" >> "$LOG"
cat "$LOG"
