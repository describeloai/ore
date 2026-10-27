#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# EL SESSION POOL · donde corren las sesiones, que crecen en caliente
#                   (docs/decisions/0073-elastic-sessions.md §2.1, S1; D-E1 decidida 2026-10-24)
#
#   pool  sesiones  n2-highmem-8 (8 vCPU, 64 GB; 57 GiB asignables) **spot**, de 0 a NODOS_MAX
#                   (1 por defecto). **Mínimo 0**: lo enciende el autoescalador cuando Kueue
#                   admite una sesión en el sabor `sesiones` (`10-kueue.yaml`) y lo quita ~10 min
#                   después de vaciarse; se paga por uso. Un nodo siempre encendido en el horario
#                   es la rampa a producción (NODOS_MIN=1), no esto. Taint ore.dev/sesiones (sólo
#                   entra quien lo tolera, y lo inyecta el sabor) y etiqueta ore.dev/pool=sesiones.
#                   Una SSD local NVMe de 375 GB para el almacenamiento efímero: `/trabajo` y lo
#                   que DuckDB derrame (R0: el nodo de las sesiones de hoy tenía 7 GB libres).
#
#   ⭐ singleProcessOOMKill: true en el kubelet (D-E6). En cgroup v2 un OOM mata el CONTENEDOR
#                   entero (S0·b: `mode`, `histogram`, `approx_quantile` tumbaron el agente con
#                   la consulta). Con esto muere sólo el proceso que se pasa —el de la consulta—
#                   y el agente sigue y lo cuenta. GKE lo admite desde 1.32.4-gke.1132000
#                   (NodeKubeletConfig); el clúster va en 1.35. La primera prueba de S1 es
#                   justo ésa: `bash pruebas-de-fuego/estructura/medir.sh s1-mode`.
#
#   cuota           CPUS_ALL_REGIONS 32; las spot cuentan como CPU normal: +8 por nodo.
#
# ⭐ EN SECO POR DEFECTO: comprueba y dice lo que haría. `APLICAR=1` lo hace. Idempotente: si el
#   pool ya existe, sólo ajusta el autoescalado. Un pool no cambia de kubelet ni de disco en
#   caliente: `RECREAR=1` lo borra y lo crea si le falta la SSD o el singleProcessOOMKill, y sólo
#   a 0 nodos. Y el orden: el pool ANTES que la malla (el sabor y las colas), y el flag de
#   ore-serve (`--session-pool`) después de las dos: una cola sin nodos deja una sesión admitida
#   y su pod en Pending.
#
#   bash malla/75-el-session-pool.sh              # en seco
#   APLICAR=1 bash malla/75-el-session-pool.sh    # de verdad
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=project-8853a180-450d-47be-b83
ZONA=europe-west1-b
CLUSTER=ore-mesh
POOL=sesiones
MAQUINA=n2-highmem-8
NODOS_MIN=${NODOS_MIN:-0}
NODOS_MAX=${NODOS_MAX:-1}
APLICAR=${APLICAR:-}
RECREAR=${RECREAR:-}

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

# El kubelet del pool (NodeKubeletConfig): sólo lo que S1 necesita.
KUBELET=$(mktemp)
trap 'rm -f "$KUBELET"' EXIT
# ⛔ La clave del fichero de gcloud es `singleProcessOomKill` (la de la documentación de GKE,
#   `singleProcessOOMKill`, la rechaza: «unknown fields», medido el 2026-10-24 con el SDK 579).
printf 'kubeletConfig:\n  singleProcessOomKill: true\n' > "$KUBELET"
# gcloud en Windows (gcloud.cmd) no entiende el /tmp de Git Bash.
KUBELET_RUTA=$(cygpath -w "$KUBELET" 2>/dev/null || echo "$KUBELET")

echo "── pool $POOL ($MAQUINA spot, $NODOS_MIN–$NODOS_MAX, SSD local, singleProcessOOMKill)"
crear=1
if gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P >/dev/null 2>&1; then
  ssd=$(gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --format="value(config.ephemeralStorageLocalSsdConfig.localSsdCount)")
  oom=$(gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --format="value(config.kubeletConfig.singleProcessOomKill)")
  if [ "${ssd:-0}" -ge 1 ] && [ "${oom,,}" = "true" ]; then
    crear=
  else
    echo "   existe sin la SSD local o sin singleProcessOOMKill (ssd=${ssd:-0}, oom=${oom:-})"
    [ -n "$RECREAR" ] || { echo "⛔ RECREAR=1 para borrarlo y crearlo con ellos (a 0 nodos)"; exit 1; }
    nodos=$(kubectl get nodes -l ore.dev/pool=$POOL --no-headers 2>/dev/null | wc -l)
    [ "$nodos" -eq 0 ] || { echo "⛔ tiene $nodos nodos: se recrea sólo a 0"; exit 1; }
    haz gcloud container node-pools delete $POOL --cluster $CLUSTER --zone $ZONA --project $P --quiet
  fi
fi
if [ -n "$crear" ]; then
  haz gcloud container node-pools create $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --machine-type $MAQUINA --spot --num-nodes "$NODOS_MIN" \
    --enable-autoscaling --min-nodes "$NODOS_MIN" --max-nodes "$NODOS_MAX" --location-policy ANY \
    --image-type COS_CONTAINERD --enable-image-streaming \
    --disk-type pd-balanced --disk-size 100 --ephemeral-storage-local-ssd count=1 \
    --system-config-from-file "$KUBELET_RUTA" \
    --node-labels ore.dev/pool=sesiones --node-taints ore.dev/sesiones=true:NoSchedule \
    --enable-private-nodes --workload-metadata GKE_METADATA \
    --shielded-secure-boot --shielded-integrity-monitoring \
    --enable-autorepair --enable-autoupgrade
else
  echo "   ya existe: sólo el autoescalado"
  haz gcloud container node-pools update $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --enable-autoscaling --min-nodes "$NODOS_MIN" --max-nodes "$NODOS_MAX" --location-policy ANY --quiet
fi
[ -n "$APLICAR" ] && gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P \
  --format="value(name,config.machineType,config.spot,config.ephemeralStorageLocalSsdConfig.localSsdCount,config.kubeletConfig.singleProcessOomKill,autoscaling.minNodeCount,autoscaling.maxNodeCount,status)" | sed 's/^/   /'
if [ -n "$APLICAR" ]; then echo "── hecho de verdad"; else echo "── hecho en seco (APLICAR=1 para crearlo)"; fi
