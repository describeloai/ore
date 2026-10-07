#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# ORE SERVERLESS POSTGRES · lo que vive en Google y no en Kubernetes (ADR 0058, P2·2)
#
#   bucket   ore-pg-almacen-euw1   los bytes del almacenamiento: capas del pageserver
#                                  y WAL de los safekeepers. Regional, soft delete 7 d.
#   cuenta   ore-pg-almacen        la identidad del almacenamiento: objectUser SOLO en
#                                  ese bucket; Workload Identity para ore-pg/neon.
#                                  ⛔ Cero claves (además la organización las prohíbe).
#   pool     pg                    n2-standard-2, NO spot (3 safekeepers en spot caerían
#                                  a la vez: adiós quórum), virtualización anidada (las
#                                  VMs de cómputo, P3), Ubuntu (KVM + módulos de la
#                                  overlay, B.3), pd-standard (la cuota SSD está llena),
#                                  taint ore.dev/neon, etiqueta ore.dev/pool=neon.
#
# Idempotente: lo que ya existe se deja. Crecer el pool (P3: 3 nodos, luego nodos
# grandes en la puerta de producción) es cambiar NODOS y volver a pasar.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=project-8853a180-450d-47be-b83
REGION=europe-west1
ZONA=europe-west1-b
CLUSTER=ore-mesh
BUCKET=ore-pg-almacen-euw1
GSA=ore-pg-almacen
KSA_NS=ore-pg
KSA=neon
POOL=pg
NODOS=${NODOS:-1}

echo "── bucket gs://$BUCKET"
if ! gcloud storage buckets describe gs://$BUCKET --project $P >/dev/null 2>&1; then
  gcloud storage buckets create gs://$BUCKET --project $P --location $REGION \
    --uniform-bucket-level-access --public-access-prevention --soft-delete-duration 7d
fi

echo "── cuenta $GSA"
if ! gcloud iam service-accounts describe $GSA@$P.iam.gserviceaccount.com --project $P >/dev/null 2>&1; then
  gcloud iam service-accounts create $GSA --project $P \
    --display-name "ORE Postgres · almacenamiento (pageserver, safekeepers)"
fi
gcloud storage buckets add-iam-policy-binding gs://$BUCKET \
  --member serviceAccount:$GSA@$P.iam.gserviceaccount.com --role roles/storage.objectUser >/dev/null
gcloud iam service-accounts add-iam-policy-binding $GSA@$P.iam.gserviceaccount.com --project $P \
  --role roles/iam.workloadIdentityUser --member "serviceAccount:$P.svc.id.goog[$KSA_NS/$KSA]" >/dev/null
echo "   claves de usuario: $(gcloud iam service-accounts keys list --iam-account $GSA@$P.iam.gserviceaccount.com --managed-by user --format='value(name)' | wc -l)"

echo "── pool $POOL ($NODOS nodo/s)"
if ! gcloud container node-pools describe $POOL --cluster $CLUSTER --zone $ZONA --project $P >/dev/null 2>&1; then
  gcloud container node-pools create $POOL --cluster $CLUSTER --zone $ZONA --project $P \
    --machine-type n2-standard-2 --num-nodes $NODOS \
    --enable-nested-virtualization --image-type UBUNTU_CONTAINERD \
    --disk-type pd-standard --disk-size 50 \
    --node-labels ore.dev/pool=neon --node-taints ore.dev/neon=true:NoSchedule \
    --enable-private-nodes --workload-metadata GKE_METADATA \
    --shielded-secure-boot --shielded-integrity-monitoring \
    --enable-autorepair --enable-autoupgrade
else
  gcloud container clusters resize $CLUSTER --node-pool $POOL --num-nodes $NODOS --zone $ZONA --project $P --quiet
fi
