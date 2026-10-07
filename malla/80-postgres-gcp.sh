#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# ORE SERVERLESS POSTGRES · lo que vive en Google y no en Kubernetes (ADR 0058, P2·2)
#
#   bucket   ore-pg-almacen-euw1   los bytes del almacenamiento: capas del pageserver
#                                  y WAL de los safekeepers. Regional, soft delete 7 d.
#   cuenta   ore-pg-almacen        la identidad del almacenamiento: objectUser SOLO en
#                                  ese bucket; Workload Identity para ore-pg/neon.
#                                  ⛔ Cero claves (además la organización las prohíbe).
#   copias   ore-copias            la cuenta de las copias del IdP, también para ore-pg/copias.
#   secreto  storcon-db            la contraseña de la base del controller: /dev/urandom al crearlo;
#                                  no se imprime ni se guarda fuera del Secret.
#   jwt      almacen-jwt[-privada] el par Ed25519 del almacenamiento y sus tokens (P2·4).
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

echo "── las copias de la base del controller: ore-copias (la de las del IdP) también para ore-pg/copias"
gcloud iam service-accounts add-iam-policy-binding ore-copias@$P.iam.gserviceaccount.com --project $P \
  --role roles/iam.workloadIdentityUser --member "serviceAccount:$P.svc.id.goog[$KSA_NS/copias]" >/dev/null

echo "── el Secret storcon-db (P2·3): /dev/urandom, directo al Secret, sin imprimirse"
kubectl get ns $KSA_NS >/dev/null 2>&1 || kubectl create ns $KSA_NS
if ! kubectl -n $KSA_NS get secret storcon-db >/dev/null 2>&1; then
  CLAVE=$(head -c 30 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)
  kubectl -n $KSA_NS create secret generic storcon-db --from-literal=usuario=storcon --from-literal=clave="$CLAVE" \
    --from-literal=url="postgresql://storcon:$CLAVE@storcon-db.$KSA_NS.svc.cluster.local:5432/storage_controller" >/dev/null
  unset CLAVE
  echo "   creado"
else
  echo "   ya existe (no se toca: rotarla es otra operación)"
fi

echo "── la autenticación del almacenamiento (P2·4): un par Ed25519 y sus tokens, directos a Secrets"
# Neon firma con EdDSA y no exige `exp` (libs/utils/src/auth.rs). Scopes:
#   pageserverapi    controller → pageserver
#   safekeeperdata   controller → safekeepers, pageserver → safekeepers, safekeeper ↔ safekeeper
#   generations_api  pageserver → controller (re-attach, validate)
#   infra            controller → plano de control (notify-attach); hasta P4 lo recibe `avisos`
#   admin            nosotros y el plano de control (P4) → controller
# almacen-jwt          lo que montan las piezas: la pública y sus tokens
# almacen-jwt-privada  la privada y el token admin: sólo para quien acuña (P4 y las pruebas)
if ! kubectl -n $KSA_NS get secret almacen-jwt >/dev/null 2>&1; then
  T=$(mktemp -d)
  python - "$T" <<'PY'
import sys, pathlib, jwt
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
d = pathlib.Path(sys.argv[1]); k = Ed25519PrivateKey.generate()
priv = k.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption())
pub = k.public_key().public_bytes(serialization.Encoding.PEM, serialization.PublicFormat.SubjectPublicKeyInfo)
(d / "privada.pem").write_bytes(priv); (d / "publica.pem").write_bytes(pub)
for scope in ["pageserverapi", "safekeeperdata", "generations_api", "infra", "admin"]:
    (d / scope).write_text(jwt.encode({"scope": scope}, priv, algorithm="EdDSA"))
PY
  kubectl -n $KSA_NS create secret generic almacen-jwt --from-file=publica.pem=$T/publica.pem \
    --from-file=pageserverapi=$T/pageserverapi --from-file=safekeeperdata=$T/safekeeperdata \
    --from-file=generations_api=$T/generations_api --from-file=infra=$T/infra >/dev/null
  kubectl -n $KSA_NS create secret generic almacen-jwt-privada --from-file=privada.pem=$T/privada.pem \
    --from-file=admin=$T/admin >/dev/null
  rm -rf "$T"
  echo "   creados almacen-jwt y almacen-jwt-privada"
else
  echo "   ya existen (no se tocan: rotar el par es otra operación)"
fi

echo "── el token del scrubber (P2·5): scope 'scrubber', acuñado con la privada, añadido a almacen-jwt"
if [ -z "$(kubectl -n $KSA_NS get secret almacen-jwt -o jsonpath='{.data.scrubber}')" ]; then
  T=$(mktemp -d)
  kubectl -n $KSA_NS get secret almacen-jwt-privada -o jsonpath='{.data.privada\.pem}' | base64 -d > $T/privada.pem
  python -c "import jwt,sys;print(jwt.encode({'scope':'scrubber'}, open(sys.argv[1]).read(), algorithm='EdDSA'), end='')" $T/privada.pem > $T/scrubber
  kubectl -n $KSA_NS patch secret almacen-jwt --type merge -p "{\"data\":{\"scrubber\":\"$(base64 -w0 < $T/scrubber)\"}}" >/dev/null
  rm -rf "$T"; echo "   añadido"
else
  echo "   ya existe"
fi
