#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# EL POOL DEL DRIVER · donde corre el driver de un Build pesado elástico (ADR 0068 C3·c, D2)
#
#   pool  pesado-driver   n2-highmem-4 (4 vCPU, 32 GB) **normal, no spot**, de 0 a 1 nodos: si spot
#                         se lleva un ejecutor, Spark lo recomputa; si se lleva el driver, se pierde el
#                         Build entero. Caben dos drivers (1,5 CPU / 12 GiB). A cero sin Builds pesados.
#                         Taint ore.dev/pesado-driver y etiqueta ore.dev/pool=pesado-driver. Lo demás,
#                         como `pesado` (73): COS, image streaming, nodos privados, Workload Identity.
#                         Los ejecutores siguen en `pesado` (spot).
#
#   cuota                 CPUS_ALL_REGIONS 32: sistema 4 + pg 4 + jobs-s 2 + driver 4 + pesado 16 = 30.
#
# ⭐ EN SECO POR DEFECTO, como 73. `APLICAR=1` lo hace. Idempotente.
#   bash malla/74-el-pool-del-driver.sh
#   APLICAR=1 bash malla/74-el-pool-del-driver.sh
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=project-8853a180-450d-47be-b83
ZONA=europe-west1-b
CLUSTER=ore-mesh
POOL=pesado-driver
MAQUINA=n2-highmem-4
NODOS_MAX=${NODOS_MAX:-1}
APLICAR=${APLICAR:-}
haz() {
  if [ -n "$APLICAR" ]; then "$@"; else echo "   (en seco) $*" | sed 's/--project [^ ]*/--project …/'; fi
}
echo "── la cuota"
usada=$(gcloud compute project-info describe --project $P --format=json \
  | python3 -c "import json,sys; q={x['metric']:x for x in json.load(sys.stdin)['quotas']}['CPUS_ALL_REGIONS']; print('%d %d' % (q['usage'], q['limit']))")
set -- $usada
echo "   CPUS_ALL_REGIONS: $1 usadas de $2; con este pool lleno, +$((4 * NODOS_MAX))"
[ $(( $1 + 4 * NODOS_MAX )) -le "$2" ] || { echo "⛔ no cabe en la cuota"; exit 1; }
echo "── pool $POOL ($MAQUINA normal, 0–$NODOS_MAX)"
if ! gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P >/dev/null 2>&1; then
  haz gcloud container node-pools create $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --machine-type $MAQUINA --num-nodes 0 \
    --enable-autoscaling --min-nodes 0 --max-nodes "$NODOS_MAX" --location-policy ANY \
    --image-type COS_CONTAINERD --enable-image-streaming \
    --disk-type pd-balanced --disk-size 50 \
    --node-labels ore.dev/pool=pesado-driver --node-taints ore.dev/pesado-driver=true:NoSchedule \
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
