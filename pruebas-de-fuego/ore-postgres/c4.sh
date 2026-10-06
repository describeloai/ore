#!/usr/bin/env bash
# D0c·C4 · «sin fondo»: se pierde el pageserver CON su disco; uno vacío recupera el tenant desde GCS.
# Mide: RPO (¿se pierde algo?), tiempo hasta tenant activo, hasta la primera consulta, y la lectura en frío.
set -u
D=/c/tmp/d0b; read T TL < $D/ids-gcs.txt; RAMA=$(cat $D/rama-gcs.txt)
OV=$(cat $D/vm-ov.txt); OVR=$(cat $D/vm-rama-ov.txt)
ms() { echo $(( $(date +%s%N) / 1000000 )); }
ts() { date -u +%H:%M:%S.%3N; }
C="kubectl -n d0-neon exec cliente-overlay -- env PGPASSWORD=cloud_admin PGCONNECT_TIMEOUT=3"
q() { $C psql -h "$1" -p 55433 -U cloud_admin -d postgres -Atc "$2" 2>&1; }
PS() { kubectl -n d0-neon exec cliente-overlay -- bash -c "exec 3<>/dev/tcp/pageserver.d0-neon.svc.cluster.local/9898; printf '$1 $2 HTTP/1.0\r\nHost: p\r\nContent-Type: application/json\r\nContent-Length: ${#3}\r\n\r\n$3' >&3; timeout 120 cat <&3" 2>/dev/null | tail -1; }

echo "$(ts) ① marca subida a GCS"
q $OV "insert into marca values ('C4: subida a GCS')" >/dev/null
PS PUT "/v1/tenant/$T/timeline/$TL/checkpoint?wait_until_uploaded=true" "" >/dev/null
echo "   remote_consistent_lsn=$(PS GET /v1/tenant/$T/timeline/$TL "" | python -c "import sys,json;d=json.load(sys.stdin);print(d['remote_consistent_lsn'],'last',d['last_record_lsn'])")"
echo "$(ts) ② marca SOLO en los safekeepers (sin checkpoint)"
q $OV "insert into marca values ('C4: solo en safekeepers')" >/dev/null
LSN2=$(q $OV "select pg_current_wal_flush_lsn()"); echo "   lsn de la marca 2: $LSN2"

echo "$(ts) ③ DESASTRE: el pageserver y su disco"
T0=$(ms)
kubectl -n d0-neon delete pvc datos-pageserver-0 --wait=false >/dev/null
kubectl -n d0-neon delete pod pageserver-0 --grace-period=0 --force >/dev/null 2>&1
# (sondeo: una consulta a main cada segundo, en segundo plano)
( while true; do r=$(q $OV "select count(*) from marca"); echo "$(ts) sonda main: ${r:0:60}"; sleep 1; done ) > $D/c4-sonda.txt 2>&1 & SONDA=$!
until [ "$(kubectl -n d0-neon get pod pageserver-0 -o jsonpath='{.status.containerStatuses[0].ready}' 2>/dev/null)" = true ]; do sleep 1; done
T1=$(ms); echo "$(ts) pageserver nuevo listo: $(( T1-T0 )) ms (disco nuevo: $(kubectl -n d0-neon get pvc datos-pageserver-0 -o jsonpath='{.metadata.creationTimestamp}'))"
echo "   tenants que conoce: $(PS GET /v1/tenant "")"

echo "$(ts) ④ reenganchar el tenant (generación 2: lo que haría el storage_controller)"
PS PUT /v1/tenant/$T/location_config '{"mode":"AttachedSingle","generation":2,"tenant_conf":{"checkpoint_distance":16777216,"checkpoint_timeout":"30s"}}' >/dev/null
until [ "$(PS GET /v1/tenant/$T "" | python -c "import sys,json;print(json.load(sys.stdin)['state']['slug'])" 2>/dev/null)" = Active ]; do sleep 0.5; done
T2=$(ms); echo "$(ts) tenant Active: $(( T2-T1 )) ms tras el pageserver listo"
echo "   timelines: $(PS GET /v1/tenant/$T/timeline "" | python -c "import sys,json;print([(t['timeline_id'][:8], t['last_record_lsn']) for t in json.load(sys.stdin)])")"

until [ "$(q $OV 'select 1')" = 1 ]; do sleep 0.5; done
T3=$(ms); kill $SONDA
echo "$(ts) primera consulta en main: $(( T3-T0 )) ms desde el desastre"
echo "⑤ RPO"
echo "   main: $(q $OV "select string_agg(q, ' | ' order by q) from marca where q like 'C4%'")"
echo "   rama: $(q $OVR "select string_agg(q, ' | ' order by q) from marca")"
echo "⑥ lectura en frío desde GCS (la VM de la rama se recrea: sin caché local)"
kubectl -n d0-neon delete neonvm pg-d0-rama --wait=true >/dev/null; kubectl apply -f $D/vm-rama.yaml >/dev/null
until [ "$(q $(kubectl -n d0-neon get neonvm pg-d0-rama -o jsonpath='{.status.extraNetIP}' 2>/dev/null) 'select 1' 2>/dev/null)" = 1 ]; do sleep 2; done
OVR=$(kubectl -n d0-neon get neonvm pg-d0-rama -o jsonpath='{.status.extraNetIP}')
for i in 1 2; do t=$(ms); n=$(q $OVR "select count(*) from pgbench_accounts"); echo "   count(*) #$i en la rama: $n en $(( $(ms)-t )) ms"; done
echo "   capas locales del pageserver ahora: $(PS GET /v1/tenant/$T/timeline/$RAMA "" | python -c "import sys,json;d=json.load(sys.stdin);print(d.get('current_physical_size'),'bytes')")"
