#!/usr/bin/env bash
# nodos.sh — P3·2 · el autoescalado de nodos del pool `pg` (ADR 0058): un pod que NO cabe en los nodos
# que hay pero SÍ en uno nuevo, y se mide. Un n2-standard-2 da 1930m; los DaemonSets de GKE piden 483m
# (en uno nuevo quedan ~1447m) y el almacenamiento otros ~190m (en el que hay quedan ~1257m). Con
# 1500m GKE NO sube un nodo: no cabría en él tampoco (medido: no.scale.up.mig.failing.predicate).
#   CPU=1350m nodos.sh
#   ① cuánto tarda GKE en traer un nodo y que el pod corra en él
#   ② cuánto tarda en quitarlo cuando el pod se va (perfil BALANCED: ~10 min de nodo sobrante)
# Es lo que pasará cuando una VM de cómputo no quepa (P3·7 lo mide con VMs de verdad).
source "$(dirname "$0")/entorno.sh"
CPU=${CPU:-1350m}
nodos() { kubectl get nodes -l ore.dev/pool=$ORE_PG_POOL --no-headers 2>/dev/null | grep -c ' Ready'; }
N0=$(nodos); echo "$(ts) nodos en el pool: $N0"

cat <<EOF | k apply -f - >/dev/null
apiVersion: v1
kind: Pod
metadata: { name: p32-no-cabe, labels: { ore.dev/rol: p32 } }
spec:
  nodeSelector: { ore.dev/pool: $ORE_PG_POOL }
  tolerations: [{ key: ore.dev/neon, operator: Exists, effect: NoSchedule }]
  terminationGracePeriodSeconds: 0
  containers:
    - name: ocupa
      image: $ORE_PG_REGISTRO/postgres:16
      command: [sleep, infinity]
      resources: { requests: { cpu: $CPU, memory: 256Mi } }
EOF
T0=$(ms); echo "$(ts) pod de $CPU creado"
until [ "$(k get pod p32-no-cabe -o jsonpath='{.status.phase}')" = Running ]; do
  [ -z "${visto:-}" ] && k get events --field-selector involvedObject.name=p32-no-cabe 2>/dev/null | grep -q TriggeredScaleUp && { visto=1; echo "$(ts) GKE pide un nodo (TriggeredScaleUp): $(( $(ms)-T0 )) ms"; }
  sleep 2
done
T1=$(ms); echo "$(ts) ① el pod corre en $(k get pod p32-no-cabe -o jsonpath='{.spec.nodeName}'): $(( T1-T0 )) ms · nodos: $(nodos)"

k delete pod p32-no-cabe --wait=true >/dev/null; T2=$(ms); echo "$(ts) pod borrado; esperando a que GKE quite el nodo"
until [ "$(nodos)" -le "$N0" ]; do sleep 15; done
T3=$(ms); echo "$(ts) ② de vuelta a $N0 nodo(s): $(( (T3-T2)/1000 )) s después de irse el pod"
