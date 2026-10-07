#!/usr/bin/env bash
# P3·6 · la IP reutilizada (ADR 0058; C4 lo vio: ~1 min sin llegar). Se recrea una VM; el IPAM de NeonVM le
# da la IP MÁS BAJA libre, que es la suya de antes, con OTRA MAC. Quien tenía la MAC vieja en su caché ARP
# (el lado del proxy: aquí `cliente-overlay`) no llega hasta que esa entrada caduca.
#   p36.sh [veces]   → por cada vez: cuándo responde por la IP del pod y cuándo por la overlay
#   GARP=mano p36.sh → además, en cuanto el huésped acepta ssh, le hace anunciar su IP y su MAC (ARP
#                      gratuito, garp.pl): lo que hará la imagen al arrancar. Para medirlo antes de cambiarla.
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
V=${ORE_PG_VM:-pg-prueba}
for n in $(seq 1 "${1:-1}"); do
  OV0=$(ip_overlay "$V")
  qo "$OV0" 'select 1' >/dev/null 2>&1        # el lado del proxy habla con la VM: su MAC entra en la caché
  MAC0=$(k exec cliente-overlay -- awk -v ip="$OV0" '$1==ip {print $4}' /proc/net/arp)
  kc delete neonvm "$V" --wait=true >/dev/null; kc wait --for=delete pod -l vm.neon.tech/name="$V" --timeout=120s >/dev/null
  plantilla "$AQUI/vm.yaml" ORE_PG_VM="$V" | kubectl apply -f - >/dev/null; T0=$(ms)
  if [ "${GARP:-}" = mano ]; then
    until R=$(kc get neonvm "$V" -o jsonpath='{.status.podName}') && [ -n "$R" ] && OVN=$(ip_overlay "$V") && [ -n "$OVN" ]       && kc exec -i "$R" -c neonvm-runner -- ssh -o ConnectTimeout=2 -o LogLevel=ERROR guest-vm 'cat > /tmp/garp.pl' < "$AQUI/garp.pl" 2>/dev/null; do sleep 0.5; done
    kc exec "$R" -c neonvm-runner -- ssh -o LogLevel=ERROR guest-vm "perl /tmp/garp.pl eth1 $OVN" >/dev/null
    echo "   ARP gratuito a los $(( $(ms)-T0 )) ms"
  fi
  until [ -n "$(ip_pod "$V")" ] && [ "$(q "$(ip_pod "$V")" 'select 1' 2>/dev/null)" = 1 ]; do sleep 0.5; done; T1=$(ms)
  OV=$(ip_overlay "$V")
  until [ "$(qo "$OV" 'select 1' 2>/dev/null)" = 1 ]; do sleep 0.5; [ $(( $(ms)-T1 )) -gt 300000 ] && break; done; T2=$(ms)
  MAC=$(k exec cliente-overlay -- awk -v ip="$OV" '$1==ip {print $4}' /proc/net/arp)
  echo "$n · $V: IP overlay $OV0 → $OV · MAC $MAC0 → $MAC · responde por el pod a los $(( T1-T0 )) ms · por la overlay $(( T2-T1 )) ms DESPUÉS"
done
