#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# EL POOL DEL CATÁLOGO · donde vive el Postgres del catálogo transaccional (ADR 0079 §17, E1a·4b)
#
#   pool  catalogo   e2-medium (2 vCPU compartidas, 4 GB) **normal, no spot**, UN nodo fijo y sin
#                    autoescalado: el catálogo está en el camino de cada escritura y no se duerme
#                    (§7). Tampoco entra en el horario (18): de noche sigue encendido (~24 $/mes).
#                    E1a·1b midió 1.000 commits/s con 0,43 CPU. Taint ore.dev/catalogo y etiqueta
#                    ore.dev/pool=catalogo. Lo demás, como `pesado-driver` (74).
#
#   cuota            CPUS_ALL_REGIONS 32: +2.
#
#   Y lo que el manifiesto (76-el-catalogo.yaml) no puede hacer por sí mismo:
#     · el namespace y el Secret `catalogo-db` (usuario y clave de /dev/urandom, sin imprimirse);
#     · que `catalogo/copias` sea `ore-copias` en Google (Workload Identity), como las del IdP.
#
# ⭐ EN SECO POR DEFECTO, como 73 y 74. `APLICAR=1` lo hace. Idempotente.
#   bash malla/76-el-pool-del-catalogo.sh
#   APLICAR=1 bash malla/76-el-pool-del-catalogo.sh
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=project-8853a180-450d-47be-b83
ZONA=europe-west1-b
CLUSTER=ore-mesh
POOL=catalogo
MAQUINA=e2-medium
NS=catalogo
APLICAR=${APLICAR:-}
haz() {
  if [ -n "$APLICAR" ]; then "$@"; else echo "   (en seco) $*" | sed 's/--project [^ ]*/--project …/'; fi
}
echo "── la cuota"
usada=$(gcloud compute project-info describe --project $P --format=json \
  | python3 -c "import json,sys; q={x['metric']:x for x in json.load(sys.stdin)['quotas']}['CPUS_ALL_REGIONS']; print('%d %d' % (q['usage'], q['limit']))")
set -- $usada
echo "   CPUS_ALL_REGIONS: $1 usadas de $2; con este pool, +2"
[ $(( $1 + 2 )) -le "$2" ] || { echo "⛔ no cabe en la cuota"; exit 1; }

echo "── pool $POOL ($MAQUINA normal, 1 nodo fijo)"
if ! gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P >/dev/null 2>&1; then
  haz gcloud container node-pools create $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --machine-type $MAQUINA --num-nodes 1 \
    --image-type COS_CONTAINERD \
    --disk-type pd-balanced --disk-size 30 \
    --node-labels ore.dev/pool=catalogo --node-taints ore.dev/catalogo=true:NoSchedule \
    --enable-private-nodes --workload-metadata GKE_METADATA \
    --shielded-secure-boot --shielded-integrity-monitoring \
    --enable-autorepair --enable-autoupgrade
else
  echo "   ya existe"
fi

echo "── el namespace y el Secret catalogo-db (/dev/urandom, directo al Secret, sin imprimirse)"
if ! kubectl get ns $NS >/dev/null 2>&1; then
  haz kubectl create ns $NS
fi
if ! kubectl -n $NS get secret catalogo-db >/dev/null 2>&1; then
  if [ -n "$APLICAR" ]; then
    CLAVE=$(head -c 30 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)
    kubectl -n $NS create secret generic catalogo-db --from-literal=usuario=catalogo \
      --from-literal=clave="$CLAVE" >/dev/null
    unset CLAVE
    echo "   creado"
  else
    echo "   (en seco) kubectl -n $NS create secret generic catalogo-db …"
  fi
else
  echo "   ya existe: no se toca (la base ya se inicializó con él)"
fi

# 0079 ·5f: el `Cluster catalogo` de CloudNativePG lee su usuario de un Secret basic-auth. Sale del
# mismo `catalogo-db` (la misma clave): mudar la base a CloudNativePG no cambia a ningún cliente.
echo "── catalogo-app: el usuario del Cluster de CloudNativePG, de catalogo-db"
if ! kubectl -n $NS get secret catalogo-app >/dev/null 2>&1; then
  if [ -n "$APLICAR" ]; then
    U=$(kubectl -n $NS get secret catalogo-db -o jsonpath='{.data.usuario}' | base64 -d)
    C=$(kubectl -n $NS get secret catalogo-db -o jsonpath='{.data.clave}' | base64 -d)
    kubectl -n $NS create secret generic catalogo-app --type=kubernetes.io/basic-auth       --from-literal=username="$U" --from-literal=password="$C" >/dev/null
    unset U C
    echo "   creado"
  else
    echo "   (en seco) kubectl -n $NS create secret generic catalogo-app --type=kubernetes.io/basic-auth …"
  fi
else
  echo "   ya existe: no se toca"
fi

echo "── las copias: catalogo/copias es ore-copias en Google, como las del IdP"
haz gcloud iam service-accounts add-iam-policy-binding ore-copias@$P.iam.gserviceaccount.com --project $P \
  --role roles/iam.workloadIdentityUser --member "serviceAccount:$P.svc.id.goog[$NS/copias]" --quiet
# 0079 ·5f: el `Cluster catalogo` de CloudNativePG (su cuenta, `catalogo/catalogo`) archiva el WAL
# y sus copias base en el mismo bucket, con la misma cuenta de Google.
haz gcloud iam service-accounts add-iam-policy-binding ore-copias@$P.iam.gserviceaccount.com --project $P \
  --role roles/iam.workloadIdentityUser --member "serviceAccount:$P.svc.id.goog[$NS/catalogo]" --quiet
# 0079 ·5f·5: el auditor del catálogo (`catalogo/auditor`) lee, y sólo lee, el bucket de copia de cada
# inquilino: que cada puntero vivo existe. Una cuenta propia, no una que escribe. Al dar de alta un
# inquilino en el catálogo (78), su bucket se añade aquí.
haz gcloud iam service-accounts describe ore-auditor-catalogo@$P.iam.gserviceaccount.com --project $P >/dev/null 2>&1   || haz gcloud iam service-accounts create ore-auditor-catalogo --project $P        --display-name "ORE · auditor del catálogo (lee los buckets de copia)"
for b in t-victor-copia t-demo-copia; do
  haz gcloud storage buckets add-iam-policy-binding gs://$P-$b --project $P --condition=None     --role roles/storage.objectViewer --member "serviceAccount:ore-auditor-catalogo@$P.iam.gserviceaccount.com"
done
haz gcloud iam service-accounts add-iam-policy-binding ore-auditor-catalogo@$P.iam.gserviceaccount.com --project $P   --role roles/iam.workloadIdentityUser --member "serviceAccount:$P.svc.id.goog[$NS/auditor]" --quiet
# barman-cloud comprueba el bucket antes de archivar (`storage.buckets.get`), y ore-copias sólo
# escribía objetos (medido: 403 al crear el Cluster). Lo mínimo que lo da, sólo sobre este bucket.
haz gcloud storage buckets add-iam-policy-binding gs://$P-copias --project $P   --role roles/storage.legacyBucketReader --member "serviceAccount:ore-copias@$P.iam.gserviceaccount.com"

[ -n "$APLICAR" ] && gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P \
  --format="value(name,config.machineType,config.spot,initialNodeCount,status)" | sed 's/^/   /'
if [ -n "$APLICAR" ]; then echo "── hecho de verdad"; else echo "── hecho en seco (APLICAR=1 para crearlo)"; fi
