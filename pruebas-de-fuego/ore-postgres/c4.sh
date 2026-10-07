#!/usr/bin/env bash
# D0c·C4 · «sin fondo»: se pierde el pageserver CON su disco; uno vacío recupera el tenant desde GCS.
# Mide: RPO (¿se pierde algo?), tiempo hasta tenant activo, hasta la primera consulta, y la lectura
# en frío de una rama. B.8: RPO 0, ~21 s. Necesita: tenant.sh, una rama `r1` con su VM `pg-rama`
# (ORE_PG_VM=pg-rama ./vm.sh $(cat $ORE_PG_TRABAJO/rama-r1)) y pgbench inicializado en main.
#   ⚠️ En P2 el reenganche lo hace el storage_controller: entonces el paso ④ sobra y se mide sin mano.
source "$(dirname "$0")/entorno.sh"
T=$(leer tenant) && TL=$(leer main) && RAMA=$(leer rama-r1) || exit 1
OV=$(ip_overlay); OVR=$(ip_overlay pg-rama)
q "$(ip_pod)" "create table if not exists marca(q text)" >/dev/null

echo "$(ts) ① marca subida a GCS"
qo "$OV" "insert into marca values ('C4: subida a GCS')" >/dev/null
pageserver PUT "/v1/tenant/$T/timeline/$TL/checkpoint?wait_until_uploaded=true" >/dev/null
echo "   $(pageserver GET /v1/tenant/$T/timeline/$TL | python -c "import sys,json;d=json.load(sys.stdin);print('remote_consistent_lsn',d['remote_consistent_lsn'],'last',d['last_record_lsn'])")"
echo "$(ts) ② marca SOLO en los safekeepers (sin checkpoint)"
qo "$OV" "insert into marca values ('C4: solo en safekeepers')" >/dev/null

echo "$(ts) ③ DESASTRE: el pageserver y su disco"
T0=$(ms)
k delete pvc datos-pageserver-0 --wait=false >/dev/null
k delete pod pageserver-0 --grace-period=0 --force >/dev/null 2>&1
until [ "$(k get pod pageserver-0 -o jsonpath='{.status.containerStatuses[0].ready}' 2>/dev/null)" = true ]; do sleep 1; done
T1=$(ms); echo "$(ts) pageserver nuevo listo: $(( T1-T0 )) ms · tenants que conoce: $(pageserver GET /v1/tenant)"

echo "$(ts) ④ reenganchar el tenant (generación 2: lo que hará el storage_controller)"
pageserver PUT /v1/tenant/$T/location_config '{"mode":"AttachedSingle","generation":2,"tenant_conf":{}}' >/dev/null
until [ "$(pageserver GET /v1/tenant/$T | python -c "import sys,json;print(json.load(sys.stdin)['state']['slug'])" 2>/dev/null)" = Active ]; do sleep 0.5; done
T2=$(ms); echo "$(ts) tenant Active: $(( T2-T1 )) ms tras el pageserver listo"

until [ "$(qo "$OV" 'select 1')" = 1 ]; do sleep 0.5; done
T3=$(ms); echo "$(ts) primera consulta en main: $(( T3-T0 )) ms desde el desastre"
echo "⑤ RPO"
echo "   main: $(qo "$OV" "select string_agg(q, ' | ' order by q) from marca where q like 'C4%'")"
echo "   rama: $(qo "$OVR" "select count(*) from marca")"
echo "⑥ lectura en frío desde GCS (la VM de la rama se recrea: sin caché local)"
ORE_PG_VM=pg-rama "$AQUI/vm.sh" "$RAMA" >/dev/null
# medido dentro de la base (desde fuera, kubectl exec mete ~0,8 s y una vez 16 s sin explicar)
for i in 1 2; do echo "   count(*) #$i en la rama: $(qo "$(ip_overlay pg-rama)" "select count(*) || ' filas en ' || round(extract(epoch from clock_timestamp()-statement_timestamp())*1000) || ' ms' from pgbench_accounts")"; done
