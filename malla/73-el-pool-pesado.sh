#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# EL POOL PESADO · donde corren los Builds y los Previews Heavyweight (ADR 0068 C2·3 pieza 6)
#
#   pool  pesado   n2-highmem-8 (8 vCPU, 64 GB) **spot**, de 0 a 2 nodos (decidido 2026-10-18:
#                  «2 nodos de 8 por ahora»; C0 midió con 64 GB). A cero cuando no hay un build
#                  pesado: lo enciende el autoescalador cuando Kueue admite uno en el sabor
#                  `pesado` (`10-kueue.yaml`) y lo apaga al sobrar. Taint ore.dev/pesado (sólo
#                  entra quien lo tolera, y lo inyecta el sabor) y etiqueta ore.dev/pool=pesado.
#                  Disco pd-balanced de 100 GB: la imagen (1,26 GB comprimida) y lo que Spark
#                  derrame en /tmp. Image streaming (GCFS): un nodo frío no espera a bajar la
#                  imagen entera (se mide en 6c). Lo demás, como `jobs-s`: COS, nodos privados,
#                  Workload Identity, arranque seguro.
#
#   cuota          CPUS_ALL_REGIONS 32 (era 12: ADR 0060). PREEMPTIBLE_CPUS es 0 en la región,
#                  así que las spot cuentan contra la normal: el pico (sistema 4 + pg 4 + jobs-s 2
#                  + pesado 16) son 26/32.
#
# ⭐ EN SECO POR DEFECTO: comprueba y dice lo que haría. `APLICAR=1` lo hace. Idempotente: si el
#   pool ya existe, sólo ajusta el autoescalado (NODOS_MAX). Y el orden: el pool ANTES que la malla
#   (el sabor y las colas): una cola sin nodos deja un build admitido y su pod en Pending.
#
#   bash malla/73-el-pool-pesado.sh              # en seco
#   APLICAR=1 bash malla/73-el-pool-pesado.sh    # de verdad
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=project-8853a180-450d-47be-b83
REGION=europe-west1
ZONA=europe-west1-b
CLUSTER=ore-mesh
POOL=pesado
MAQUINA=n2-highmem-8
NODOS_MAX=${NODOS_MAX:-2}
APLICAR=${APLICAR:-}

haz() {
  if [ -n "$APLICAR" ]; then "$@"; else echo "   (en seco) $*" | sed 's/--project [^ ]*/--project …/'; fi
}

echo "── la máquina $MAQUINA en $ZONA"
gcloud compute machine-types describe $MAQUINA --zone $ZONA --project $P \
  --format="value(name,guestCpus,memoryMb)" | sed 's/^/   /'

echo "── la cuota (las spot cuentan como CPU normal: PREEMPTIBLE_CPUS es 0)"
usada=$(gcloud compute project-info describe --project $P --format=json \
  | python3 -c "import json,sys; q={x['metric']:x for x in json.load(sys.stdin)['quotas']}['CPUS_ALL_REGIONS']; print('%d %d' % (q['usage'], q['limit']))")
set -- $usada
echo "   CPUS_ALL_REGIONS: $1 usadas de $2; con el pool lleno, +$((8 * NODOS_MAX))"
[ $(( $1 + 8 * NODOS_MAX )) -le "$2" ] || { echo "⛔ no cabe en la cuota"; exit 1; }

echo "── image streaming (containerfilesystem.googleapis.com)"
if gcloud services list --enabled --project $P --filter="config.name=containerfilesystem.googleapis.com" \
     --format="value(config.name)" | grep -q containerfilesystem; then
  echo "   habilitada"
else
  echo "   no habilitada: se habilita"
  haz gcloud services enable containerfilesystem.googleapis.com --project $P
fi

echo "── pool $POOL ($MAQUINA spot, 0–$NODOS_MAX)"
if ! gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P >/dev/null 2>&1; then
  haz gcloud container node-pools create $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --machine-type $MAQUINA --spot --num-nodes 0 \
    --enable-autoscaling --min-nodes 0 --max-nodes "$NODOS_MAX" --location-policy ANY \
    --image-type COS_CONTAINERD --enable-image-streaming \
    --disk-type pd-balanced --disk-size 100 \
    --node-labels ore.dev/pool=pesado --node-taints ore.dev/pesado=true:NoSchedule \
    --enable-private-nodes --workload-metadata GKE_METADATA \
    --shielded-secure-boot --shielded-integrity-monitoring \
    --enable-autorepair --enable-autoupgrade
else
  echo "   ya existe: sólo el autoescalado"
  haz gcloud container node-pools update $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --enable-autoscaling --min-nodes 0 --max-nodes "$NODOS_MAX" --location-policy ANY --quiet
fi
[ -n "$APLICAR" ] && gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P \
  --format="value(name,config.machineType,config.spot,autoscaling.maxNodeCount,status)" | sed 's/^/   /'
if [ -n "$APLICAR" ]; then echo "── hecho de verdad"; else echo "── hecho en seco (APLICAR=1 para crearlo)"; fi
