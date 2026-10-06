#!/usr/bin/env bash
# D0b·4 · migración en vivo con dos sesiones abiertas: por la IP overlay (estable) y por la IP del pod.
read P IP < /c/tmp/d0b/vm-pod.txt
OV=$(kubectl -n d0-neon get neonvm pg-d0 -o jsonpath='{.status.extraNetIP}')
C="kubectl -n d0-neon exec cliente-overlay --"
$C env PGPASSWORD=cloud_admin psql -h $OV -p 55433 -U cloud_admin -d postgres -Atqc "drop table if exists mig; create table mig(via text, n int, t timestamptz default clock_timestamp())"
for par in "overlay $OV" "pod $IP"; do
  set -- $par
  $C sh -c "nohup /tmp/latido.sh $1 $2 > /tmp/sesion-$1.log 2>&1 &"
done
sleep 20
N0=$(kubectl -n d0-neon get neonvm pg-d0 -o jsonpath='{.status.node}')
echo "$(date -u +%T.%3N) MIGRAR desde $N0"
cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: vm.neon.tech/v1
kind: VirtualMachineMigration
metadata: { name: mig-d0-2, namespace: d0-neon }
spec: { vmName: pg-d0, preventMigrationToSameHost: true, allowPostCopy: false }
EOF
until F=$(kubectl -n d0-neon get neonvmm mig-d0-2 -o jsonpath='{.status.phase}' 2>/dev/null); [ "$F" = Succeeded ] || [ "$F" = Failed ]; do sleep 1; done
echo "$(date -u +%T.%3N) migración: $F"
kubectl -n d0-neon get neonvmm mig-d0-2 -o jsonpath='{.status}' | python -c "import sys,json;d=json.load(sys.stdin);print({k:d[k] for k in d if k!='conditions'})" | cut -c1-600
echo "VM ahora en $(kubectl -n d0-neon get neonvm pg-d0 -o jsonpath='{.status.node} pod={.status.podName} podIP={.status.podIP} overlay={.status.extraNetIP}')"
sleep 30
$C sh -c 'cat /tmp/sesion-*.log'
$C env PGPASSWORD=cloud_admin psql -h $OV -p 55433 -U cloud_admin -d postgres -c "
select via, count(*) filas, max(n) ultimo, round(extract(epoch from max(t)-min(t))) segundos,
       round(max(extract(epoch from t - lag_t))::numeric, 2) mayor_hueco_s,
       (select t from (select t, t - lag(t) over (order by n) h from mig m2 where m2.via=s.via) x order by h desc nulls last limit 1) cuando
from (select via, n, t, lag(t) over (partition by via order by n) lag_t from mig) s group by via"
