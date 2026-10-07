#!/usr/bin/env bash
# D0b·4 · migración en vivo con dos sesiones abiertas escribiendo: por la IP overlay (estable) y por
# la IP del pod. B.6: VM pausada 51–66 ms; por la overlay la sesión sobrevive (mayor hueco 0,75 s),
# por la IP del pod se cuelga sin error.
source "$(dirname "$0")/entorno.sh"
IP=$(ip_pod); OV=$(ip_overlay)
[ -n "$IP" ] && [ -n "$OV" ] || { echo "✗ $ORE_PG_VM sin IP (¿vm.sh?)"; exit 1; }
k cp "$AQUI/latido.sh" cliente-overlay:/tmp/latido.sh >/dev/null && k exec cliente-overlay -- chmod +x /tmp/latido.sh
qo "$OV" "drop table if exists mig; create table mig(via text, n int, t timestamptz default clock_timestamp())" >/dev/null
for par in "overlay $OV" "pod $IP"; do
  set -- $par
  k exec cliente-overlay -- sh -c "nohup /tmp/latido.sh $1 $2 > /tmp/sesion-$1.log 2>&1 &"
done
sleep 20
M=mig-$ORE_PG_VM-$(date +%s)
echo "$(ts) MIGRAR desde $(k get neonvm "$ORE_PG_VM" -o jsonpath='{.status.node}')"
cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: vm.neon.tech/v1
kind: VirtualMachineMigration
metadata: { name: $M, namespace: $ORE_PG_NS }
spec: { vmName: $ORE_PG_VM, preventMigrationToSameHost: true, allowPostCopy: false }
EOF
until F=$(k get neonvmm "$M" -o jsonpath='{.status.phase}' 2>/dev/null); [ "$F" = Succeeded ] || [ "$F" = Failed ]; do sleep 1; done
echo "$(ts) migración: $F"
k get neonvmm "$M" -o jsonpath='{.status}' | python -c "import sys,json;d=json.load(sys.stdin);print({k:d[k] for k in d if k!='conditions'})" | cut -c1-600
echo "VM ahora en $(k get neonvm "$ORE_PG_VM" -o jsonpath='{.status.node} pod={.status.podName} podIP={.status.podIP} overlay={.status.extraNetIP}')"
sleep 30
k exec cliente-overlay -- sh -c 'cat /tmp/sesion-*.log'
k exec cliente-overlay -- env PGPASSWORD=cloud_admin psql -h "$OV" -p 55433 -U cloud_admin -d postgres -c "
select via, count(*) filas, max(n) ultimo, round(extract(epoch from max(t)-min(t))) segundos,
       round(max(extract(epoch from t - lag_t))::numeric, 2) mayor_hueco_s
from (select via, n, t, lag(t) over (partition by via order by n) lag_t from mig) s group by via"
guardar "pod-$ORE_PG_VM" "$(ip_pod)"
