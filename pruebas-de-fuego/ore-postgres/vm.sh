#!/usr/bin/env bash
# vm.sh [timeline] — (re)crea la VM $ORE_PG_VM sobre un timeline (por defecto `main`) y espera a que
# responda por la overlay. Es lo que hará el plano de control al crear un endpoint (P4):
# especificación + JWKS en un ConfigMap, VirtualMachine, esperar a `select 1`.
#   ORE_PG_VM=pg-rama ./vm.sh $(cat $ORE_PG_TRABAJO/rama-r1)
source "$(dirname "$0")/entorno.sh"
T=$(leer tenant) || exit 1
TL=${1:-$(leer main)} || exit 1
python "$AQUI/especificacion.py" spec "$ORE_PG_VM" "$T" "$TL" > "$ORE_PG_TRABAJO/$ORE_PG_VM.json"
k delete neonvm "$ORE_PG_VM" --wait=true >/dev/null 2>&1
k create configmap "$ORE_PG_VM-config" --from-file=config.json="$ORE_PG_TRABAJO/$ORE_PG_VM.json" \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
T0=$(ms); plantilla "$AQUI/vm.yaml" | kubectl apply -f - >/dev/null
until OV=$(ip_overlay); [ -n "$OV" ] && [ "$(qo "$OV" 'select 1' 2>/dev/null)" = 1 ]; do
  sleep 1; [ $(( $(ms)-T0 )) -gt 300000 ] && { echo "✗ $ORE_PG_VM no responde en 5 min"; exit 1; }
done
guardar "ov-$ORE_PG_VM" "$OV"; guardar "pod-$ORE_PG_VM" "$(ip_pod)"
echo "$ORE_PG_VM sobre $TL: overlay $OV, pod $(ip_pod), en $(( $(ms)-T0 )) ms"
