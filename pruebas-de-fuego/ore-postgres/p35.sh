#!/usr/bin/env bash
# P3·5 · la overlay cerrada (ADR 0058, malla/postgres-computo/neonvm/cerrada.yaml y Multus con
# namespaceIsolation). Montaje: el de p34.sh, con las dos VMs (pg-prueba y pg-otra) vivas.
#
# De VM a VM se prueba DESDE DENTRO de la VM: su Postgres abre una conexión (dblink) a la otra por la
# overlay. Antes de P3·5 esto daba OK (medido): es el agujero que se cierra.
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
fallos=0
espera() {  # espera <qué> <esperado> <obtenido>
  local m="✓"; [ "$2" = "$3" ] || { m="✗"; fallos=$((fallos+1)); }
  printf '   %s %-60s %s (debe: %s)\n' "$m" "$1" "$3" "$2"
}
A=$(ip_overlay pg-prueba); B=$(ip_overlay pg-otra)
[ -n "$A" ] && [ -n "$B" ] || { echo "✗ faltan pg-prueba o pg-otra (p34.sh / vm.sh)"; exit 1; }
# desde_vm <ip de la VM> <host> → "sí" si su Postgres conecta al de <host> en 3 s
desde_vm() {
  qo "$1" "create extension if not exists dblink; select dblink_connect('p35', 'host=$2 port=55433 user=cloud_admin password=cloud_admin dbname=postgres connect_timeout=3')" \
    2>&1 | grep -q '^OK$' && echo sí || echo no
  qo "$1" "select dblink_disconnect('p35')" >/dev/null 2>&1
}

echo "== el lado del proxy ve a las VMs (cliente-overlay, $(k get pod cliente-overlay -o jsonpath='{.metadata.annotations.k8s\.v1\.cni\.cncf\.io/networks}'))"
espera "cliente-overlay → pg-prueba ($A)" 1 "$(qo "$A" 'select 1' 2>&1 | tail -1)"
espera "cliente-overlay → pg-otra ($B)"   1 "$(qo "$B" 'select 1' 2>&1 | tail -1)"

echo "== de VM a VM, por la overlay"
espera "pg-prueba → pg-otra" no "$(desde_vm "$A" "$B")"
espera "pg-otra → pg-prueba" no "$(desde_vm "$B" "$A")"

echo "== nadie de fuera entra en la overlay (Multus, namespaceIsolation)"
kubectl create namespace p35-fuera >/dev/null 2>&1
for nad in ore-pg/overlay-del-proxy ore-pg-computo/neonvm-overlay-for-vms; do
  n=intruso-$(echo "$nad" | cut -d/ -f1)
  cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: v1
kind: Pod
metadata: { name: $n, namespace: p35-fuera, annotations: { k8s.v1.cni.cncf.io/networks: $nad } }
spec:
  nodeSelector: { ore.dev/pool: neon }
  tolerations: [{ key: ore.dev/neon, operator: Exists }]
  terminationGracePeriodSeconds: 0
  containers: [{ name: c, image: postgres:17, command: [sleep, infinity] }]
EOF
  sleep 20
  st=$(kubectl -n p35-fuera get pod $n -o jsonpath='{.status.phase}')
  red=$(kubectl -n p35-fuera get pod $n -o jsonpath='{.metadata.annotations.k8s\.v1\.cni\.cncf\.io/network-status}' | grep -c 10.100.)
  ev=$(kubectl -n p35-fuera get events --field-selector involvedObject.name=$n -o jsonpath='{.items[*].message}' | grep -o 'namespace isolation[^"]*' | head -1)
  espera "un pod de p35-fuera pide $nad (fase $st${ev:+: $ev})" no "$([ "$red" -gt 0 ] && echo sí || echo no)"
done
kubectl delete namespace p35-fuera --wait=false >/dev/null

echo "== $([ $fallos = 0 ] && echo "✓ la overlay está cerrada" || echo "✗ $fallos fallos")"
exit $fallos
