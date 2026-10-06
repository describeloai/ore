#!/usr/bin/env bash
# D0b·2 · arranque de una VM de Postgres: kubectl apply → primera consulta con éxito.
ms() { echo $(( $(date +%s%N) / 1000000 )); }
q() { kubectl -n d0-neon exec cliente -- env PGPASSWORD=cloud_admin PGCONNECT_TIMEOUT=2 psql -h "$1" -p 55433 -U cloud_admin -d postgres -Atc "$2" 2>/dev/null; }
for i in 1 2 3; do
  kubectl -n d0-neon delete neonvm pg-d0 --wait=true >/dev/null 2>&1
  T0=$(ms); kubectl apply -f /c/tmp/d0b/vm.yaml >/dev/null
  until P=$(kubectl -n d0-neon get neonvm pg-d0 -o jsonpath='{.status.podName}' 2>/dev/null); IP=$(kubectl -n d0-neon get pod "$P" -o jsonpath='{.status.podIP}' 2>/dev/null); [ -n "$IP" ]; do sleep 0.5; done
  T1=$(ms)
  until [ "$(q $IP 'select 1')" = 1 ]; do sleep 0.5; [ $(( $(ms)-T0 )) -gt 240000 ] && break; done
  T2=$(ms)
  N=$(kubectl -n d0-neon get pod $P -o jsonpath='{.spec.nodeName}')
  # dentro de la VM: qemu arranca → compute_ctl arranca → compute running
  L=$(kubectl -n d0-neon logs $P --tail=5000 2>/dev/null)
  QEMU=$(echo "$L" | grep -m1 'calling qemu-system' | python -c "import sys,json;print(json.loads(sys.stdin.read())['ts'])" 2>/dev/null)
  CTL=$(echo "$L" | grep -m1 'logging and tracing started' | cut -c1-26)
  RUN=$(echo "$L" | grep -m1 'from init to running' | cut -c1-26)
  echo "#$i nodo=${N##*-} pod+IP=$(( T1-T0 ))ms primera-consulta=$(( T2-T0 ))ms | qemu_ts=$QEMU compute_ctl=$CTL running=$RUN"
  echo "$P $IP" > /c/tmp/d0b/vm-pod.txt
done
