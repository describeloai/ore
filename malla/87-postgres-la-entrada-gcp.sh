#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# ORE SERVERLESS POSTGRES · LA ENTRADA, LO QUE VIVE EN GOOGLE (ADR 0058, P5·2)
#
#   ip       ore-pg-entrada        IP externa REGIONAL (europe-west1): la del balanceador L4
#                                  del proxy (P5·3, malla/postgres-entrada/). Una IP fija:
#                                  el DNS no cambia aunque el Service se recree.
#   cuenta   ore-pg-dns            la identidad de cert-manager para el reto DNS-01:
#                                  roles/dns.admin SÓLO sobre la zona pg-paladio-io (nada del
#                                  resto del proyecto, ni de ore-paladio-io); Workload Identity
#                                  para cert-manager/cert-manager. ⛔ Cero claves.
#   dns      *.europe-west1.pg.paladio.io  A → la IP. Un comodín: cada endpoint es
#                                  ep-….europe-west1.pg.paladio.io (y su -pooler), y el driver
#                                  serverless entra por api.europe-west1.pg.paladio.io (P5·4).
#   limpieza _delegacion.pg.paladio.io     el TXT con que se comprobó la delegación desde
#                                  GoDaddy (2026-10-08): ya no hace falta.
#   malla    flux-system/ore-pg-entrada    ConfigMap con IP_ENTRADA: Flux la sustituye en el
#                                  Service del proxy (postBuild de malla/87-postgres-la-entrada.yaml).
#
# La zona pg-paladio-io (Cloud DNS) y la delegación NS en GoDaddy ya existen (P5, 2026-10-08).
# El certificado lo pide cert-manager (malla/postgres-entrada/), no este guion.
#
# Idempotente: lo que ya existe se deja; el registro A se corrige si apunta a otra IP.
# Antes de la malla: la IP tiene que existir antes que el Service que la usa.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=project-8853a180-450d-47be-b83
REGION=europe-west1
IP_NOMBRE=ore-pg-entrada
ZONA_DNS=pg-paladio-io
DOMINIO=pg.paladio.io
COMODIN="*.$REGION.$DOMINIO."
GSA=ore-pg-dns
KSA_NS=cert-manager
KSA=cert-manager
g() { env -u MSYS_NO_PATHCONV gcloud "$@"; }   # gcloud rompe con MSYS_NO_PATHCONV

echo "── IP regional $IP_NOMBRE"
if ! g compute addresses describe $IP_NOMBRE --region $REGION --project $P >/dev/null 2>&1; then
  g compute addresses create $IP_NOMBRE --region $REGION --project $P --network-tier PREMIUM \
    --description "ORE Postgres · la entrada pública del proxy (ADR 0058 P5·2)"
fi
IP=$(g compute addresses describe $IP_NOMBRE --region $REGION --project $P --format='value(address)')
echo "   $IP"

echo "── cuenta $GSA: dns.admin sólo sobre la zona $ZONA_DNS"
if ! g iam service-accounts describe $GSA@$P.iam.gserviceaccount.com --project $P >/dev/null 2>&1; then
  g iam service-accounts create $GSA --project $P \
    --display-name "ORE Postgres · cert-manager, reto DNS-01 en pg.paladio.io"
fi
g dns managed-zones add-iam-policy-binding $ZONA_DNS --project $P \
  --member serviceAccount:$GSA@$P.iam.gserviceaccount.com --role roles/dns.admin >/dev/null
g iam service-accounts add-iam-policy-binding $GSA@$P.iam.gserviceaccount.com --project $P \
  --role roles/iam.workloadIdentityUser --member "serviceAccount:$P.svc.id.goog[$KSA_NS/$KSA]" >/dev/null
echo "   roles en el proyecto: $(g projects get-iam-policy $P --flatten='bindings[].members' \
  --filter="bindings.members:serviceAccount:$GSA@$P.iam.gserviceaccount.com" --format='value(bindings.role)' | wc -l) (tiene que ser 0)"
echo "   claves de usuario: $(g iam service-accounts keys list --iam-account $GSA@$P.iam.gserviceaccount.com \
  --managed-by user --format='value(name)' | wc -l) (tiene que ser 0)"

echo "── $COMODIN A $IP"
ACTUAL=$(g dns record-sets describe "$COMODIN" --type A --zone $ZONA_DNS --project $P --format='value(rrdatas[0])' 2>/dev/null || true)
if [ -z "$ACTUAL" ]; then
  g dns record-sets create "$COMODIN" --type A --ttl 300 --rrdatas "$IP" --zone $ZONA_DNS --project $P >/dev/null
  echo "   creado"
elif [ "$ACTUAL" != "$IP" ]; then
  g dns record-sets update "$COMODIN" --type A --ttl 300 --rrdatas "$IP" --zone $ZONA_DNS --project $P >/dev/null
  echo "   corregido (apuntaba a $ACTUAL)"
else
  echo "   ya está"
fi

echo "── el TXT de la delegación, fuera"
if g dns record-sets describe "_delegacion.$DOMINIO." --type TXT --zone $ZONA_DNS --project $P >/dev/null 2>&1; then
  g dns record-sets delete "_delegacion.$DOMINIO." --type TXT --zone $ZONA_DNS --project $P >/dev/null
  echo "   quitado"
else
  echo "   ya no está"
fi

echo "── la IP, para la malla: ConfigMap flux-system/ore-pg-entrada (postBuild de 87-postgres-la-entrada.yaml)"
kubectl -n flux-system create configmap ore-pg-entrada --from-literal=IP_ENTRADA="$IP"   --dry-run=client -o yaml | kubectl apply -f - >/dev/null
echo "   IP_ENTRADA=$IP"

echo
echo "Comprobar desde fuera (la propagación tarda lo que el TTL):"
echo "   dig +short api.$REGION.$DOMINIO      → $IP"
echo "   dig +short ep-prueba.$REGION.$DOMINIO → $IP"
