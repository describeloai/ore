#!/usr/bin/env bash
# P3·4 · la barrera 2 (la red de pods) de `ore-pg-computo` (ADR 0058, malla/85-…): lo que una VM alcanza
# y quién la alcanza. Montaje: entorno.sh, cliente.yaml, tenant.sh; crea dos VMs (pg-prueba, pg-otra).
#
# Desde una VM se sondea en su pod `runner`: es su identidad de red (el huésped sale por el NAT del
# runner), y lo que el runner no alcanza, la VM tampoco.
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
fallos=0
# abre <desde> <host> <puerto> → "sí" si conecta en 3 s
abre_vm()  { kc exec "$1" -c neonvm-runner -- socat -T3 /dev/null "TCP:$2:$3,connect-timeout=3" >/dev/null 2>&1 && echo sí || echo no; }
abre_pod() { kubectl -n "$1" exec "$2" -- bash -c "timeout 3 bash -c '</dev/tcp/$3/$4'" >/dev/null 2>&1 && echo sí || echo no; }
espera() {  # espera <qué> <esperado> <obtenido>
  local m="✓"; [ "$2" = "$3" ] || { m="✗"; fallos=$((fallos+1)); }
  printf '   %s %-58s %s (debe: %s)\n' "$m" "$1" "$3" "$2"
}

echo "== dos VMs en $ORE_PG_NS_COMPUTO (la otra sobre una rama: dos primarios en un timeline se pelean)"
ORE_PG_VM=pg-prueba "$AQUI/vm.sh" | tail -1; [ "${PIPESTATUS[0]}" = 0 ] || exit 1
leer rama-otra >/dev/null 2>&1 || "$AQUI/tenant.sh" rama otra >/dev/null
ORE_PG_VM=pg-otra "$AQUI/vm.sh" "$(leer rama-otra)" | tail -1; [ "${PIPESTATUS[0]}" = 0 ] || exit 1
R=$(kc get neonvm pg-prueba -o jsonpath='{.status.podName}')
OTRA=$(ip_pod pg-otra)
echo "   la VM escribe (DNS + safekeepers + pageserver): $(qo "$(ip_overlay pg-prueba)" "create table if not exists p34(x int); insert into p34 values (1); select 'ok'")"

echo "== lo que alcanza una VM"
espera "safekeeper-0 :5454 (su WAL)"                 sí "$(abre_vm $R safekeeper-0.$ORE_PG_NS.svc.cluster.local 5454)"
espera "pageserver-0 :6400 (sus páginas)"            sí "$(abre_vm $R pageserver-0.$ORE_PG_NS.svc.cluster.local 6400)"
espera "pageserver-0 :9898 (su API de gestión)"      no "$(abre_vm $R pageserver-0.$ORE_PG_NS.svc.cluster.local 9898)"
espera "storage-controller :1234"                    no "$(abre_vm $R storage-controller.$ORE_PG_NS.svc.cluster.local 1234)"
espera "otra VM (pg-otra, red de pods) :55433"       no "$(abre_vm $R $OTRA 55433)"
espera "ore-serve de t-demo :8080"                   no "$(abre_vm $R ore-serve.t-demo.svc.cluster.local 8080)"
espera "el cofre de t-demo :8095"                    no "$(abre_vm $R ore-cofre.t-demo.svc.cluster.local 8095)"
espera "la API de Kubernetes :443"                   no "$(abre_vm $R kubernetes.default.svc.cluster.local 443)"
espera "el servidor de metadatos de Google :80"      no "$(abre_vm $R 169.254.169.254 80)"
espera "internet (1.1.1.1:443)"                      no "$(abre_vm $R 1.1.1.1 443)"

echo "== quién alcanza a una VM"
IP=$(ip_pod pg-prueba)
espera "cliente de ore-pg con pg-acceso :55433"     sí "$(abre_pod $ORE_PG_NS cliente $IP 55433)"
k label pod cliente ore.dev/pg-acceso- >/dev/null; sleep 15   # Cilium tarda 5–15 s en aplicar (P2·6)
espera "el mismo cliente SIN pg-acceso :55433"      no "$(abre_pod $ORE_PG_NS cliente $IP 55433)"
k label pod cliente ore.dev/pg-acceso=si >/dev/null
# el peor caso de «fuera»: un namespace SIN ninguna política (un t-* tiene las suyas, que ya cierran)
kubectl create namespace p34-fuera >/dev/null 2>&1
kubectl -n p34-fuera run fuera --image=postgres:17 --restart=Never --labels=ore.dev/pg-acceso=si \
  --overrides='{"spec":{"nodeSelector":{"ore.dev/pool":"neon"},"tolerations":[{"key":"ore.dev/neon","operator":"Exists"}]}}' \
  --command -- sleep infinity >/dev/null
kubectl -n p34-fuera wait --for=condition=Ready pod/fuera --timeout=120s >/dev/null
espera "un pod de otro namespace (¡con pg-acceso!) :55433" no "$(abre_pod p34-fuera fuera $IP 55433)"
espera "ese pod a safekeeper-0 :5454"                    no "$(abre_pod p34-fuera fuera safekeeper-0.$ORE_PG_NS.svc.cluster.local 5454)"
espera "ese pod a pageserver-0 :6400"                    no "$(abre_pod p34-fuera fuera pageserver-0.$ORE_PG_NS.svc.cluster.local 6400)"
kubectl delete namespace p34-fuera --wait=false >/dev/null

kc delete neonvm pg-otra --wait=false >/dev/null
echo "== $([ $fallos = 0 ] && echo "✓ la barrera 2 aguanta" || echo "✗ $fallos fallos")"
exit $fallos
