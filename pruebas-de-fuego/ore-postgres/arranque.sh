#!/usr/bin/env bash
# D0b·2 · arranque de una VM de Postgres: kubectl apply → primera consulta con éxito (×N, 3 por
# defecto), con el desglose dentro de la VM (qemu → compute_ctl → running). Medido en B.4: 15,8–17 s.
export ORE_PG_COMPUTO=vm   # P3·7: las VMs viven en $ORE_PG_NS_COMPUTO (kc); clientes y almacenamiento en $ORE_PG_NS (k)
source "$(dirname "$0")/entorno.sh"
T=$(leer tenant) && M=$(leer main) || exit 1
python "$AQUI/especificacion.py" spec "$ORE_PG_VM" "$T" "$M" > "$ORE_PG_TRABAJO/$ORE_PG_VM.json"
kc create configmap "$ORE_PG_VM-config" --from-file=config.json="$ORE_PG_TRABAJO/$ORE_PG_VM.json" \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
for i in $(seq 1 "${1:-3}"); do
  kc delete neonvm "$ORE_PG_VM" --wait=true >/dev/null 2>&1
  kc wait --for=delete pod -l vm.neon.tech/name="$ORE_PG_VM" --timeout=120s >/dev/null 2>&1   # el runner viejo contesta con la misma IP (P3·5)
  T0=$(ms); plantilla "$AQUI/vm.yaml" | kubectl apply -f - >/dev/null
  until P=$(kc get neonvm "$ORE_PG_VM" -o jsonpath='{.status.podName}' 2>/dev/null); IP=$(kc get pod "$P" -o jsonpath='{.status.podIP}' 2>/dev/null); [ -n "$IP" ]; do sleep 0.5; done
  T1=$(ms)
  until [ "$(q "$IP" 'select 1' 2>/dev/null)" = 1 ]; do sleep 0.5; [ $(( $(ms)-T0 )) -gt 240000 ] && break; done
  T2=$(ms)
  N=$(kc get pod "$P" -o jsonpath='{.spec.nodeName}')
  L=$(kc logs "$P" --tail=5000 2>/dev/null)
  QEMU=$(echo "$L" | grep -m1 'calling qemu-system' | python -c "import sys,json;print(json.loads(sys.stdin.read())['ts'])" 2>/dev/null)
  CTL=$(echo "$L" | grep -m1 'logging and tracing started' | cut -c1-26)
  RUN=$(echo "$L" | grep -m1 'from init to running' | cut -c1-26)
  echo "#$i nodo=${N##*-} pod+IP=$(( T1-T0 ))ms primera-consulta=$(( T2-T0 ))ms | qemu_ts=$QEMU compute_ctl=$CTL running=$RUN"
done
guardar "pod-$ORE_PG_VM" "$IP"; guardar "ov-$ORE_PG_VM" "$(ip_overlay)"
