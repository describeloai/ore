#!/usr/bin/env bash
# 0047 · M3 · El camino: por dónde llega `ore-serve` a `ore-iam`, y quién dice que es.
#
# Tres preguntas, medidas en el cluster:
#
#   1. LA RED     ¿llega un pod con las etiquetas de `ore-serve` (`ore.dev/rol=control`) a
#                 `ore-iam:8090`? ¿y al IdP? Un pod efímero por inquilino, con esas etiquetas
#                 para que le apliquen las mismas NetworkPolicy, y se borra al terminar.
#   2. LA CELDA   ¿con qué se puede presentar la celda? El token de identidad de GCP de la
#                 cuenta de `ore-serve` (Workload Identity, audiencia `ore-iam`): se piden sus
#                 CLAIMS, nunca se imprime el token. Y quién más podría ser esa cuenta.
#   3. EL AGENTE  el precedente del informador (0026): quién lee la credencial del agente de
#                 cada celda, y qué agentes están registrados en `iam` y vivos en el realm.
#
#     bash pruebas-de-fuego/medida-el-camino.sh [inquilino…]     (por defecto: demo prueba victor)
#
# Necesita el contexto del cluster `ore-mesh` y `gcloud` con el proyecto por defecto. Crea y
# borra pods en los namespaces de los inquilinos; no toca nada más.
set -uo pipefail
export MSYS_NO_PATHCONV=1   # Git Bash: que no reescriba los caminos de kubectl

INQUILINOS=${*:-demo prueba victor}
# ⚠️ gcloud SIN `MSYS_NO_PATHCONV` (con él no arranca en Git Bash), y `tr -d '\r'`: en Windows
#   termina la línea con CR, y la imagen salía `InvalidImageName`.
gcloud() { env -u MSYS_NO_PATHCONV gcloud "$@"; }
PROYECTO=$(gcloud config get-value project 2>/dev/null | tr -d '\r')
IMG="europe-west1-docker.pkg.dev/$PROYECTO/ore/ore-drivers:main"
IAM=http://ore-iam.identidad.svc.cluster.local:8090/salud
IDP=http://idp-service.identidad.svc.cluster.local:8080/realms/rubix/.well-known/openid-configuration
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

# Un pod de medida: etiquetas de `ore-serve`, sin privilegios, y la orden que se le pase.
pod() { # <ns> <nombre> <serviceAccount> <orden>
  cat > "$TMP/$2.yaml" <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: $2
  labels: {ore.dev/rol: control, ore.dev/tenant: ${1#t-}}
spec:
  restartPolicy: Never
  serviceAccountName: $3
  containers:
  - name: m3
    image: $IMG
    command: ["sh", "-c", $(printf '%s' "$4" | python -c 'import json,sys;print(json.dumps(sys.stdin.read()))')]
    resources: {requests: {cpu: 10m, memory: 32Mi}, limits: {cpu: 100m, memory: 64Mi}}
    securityContext: {allowPrivilegeEscalation: false, runAsNonRoot: true, runAsUser: 1000, capabilities: {drop: [ALL]}, seccompProfile: {type: RuntimeDefault}}
EOF
  kubectl delete pod -n "$1" "$2" --ignore-not-found --wait=true >/dev/null 2>&1   # el de una pasada anterior
  # ⚠️ Un reintento: la cuota del namespace se actualiza con bloqueo optimista y, con tres
  #   pods a la vez, un alta puede chocar («the object has been modified»).
  kubectl apply -n "$1" -f - < "$TMP/$2.yaml" >/dev/null 2>&1 \
    || { sleep 3; kubectl apply -n "$1" -f - < "$TMP/$2.yaml" >/dev/null; } || return 1
}
esperar_y_leer() { # <ns> <nombre>
  kubectl get pod -n "$1" "$2" >/dev/null 2>&1 || { echo "   (no se creó el pod)"; return; }
  for _ in $(seq 1 30); do
    case $(kubectl get pod -n "$1" "$2" -o jsonpath='{.status.phase}' 2>/dev/null) in
      Succeeded|Failed) break ;;
    esac
    sleep 5
  done
  kubectl logs -n "$1" "$2" 2>&1 | sed 's/^/   /'
  kubectl delete pod -n "$1" "$2" --wait=false >/dev/null 2>&1
}

echo "== 1 · la red: desde un pod con las etiquetas de ore-serve"
for t in $INQUILINOS; do
  pod "t-$t" m3-camino ore-serve "for u in $IAM $IDP; do echo \"\${u%%/realms*} -> \$(curl -s -m 5 -o /dev/null -w %{http_code} \$u; echo \" rc=\$?\")\"; done"
done
for t in $INQUILINOS; do echo "-- t-$t"; esperar_y_leer "t-$t" m3-camino; done
echo "   (000 rc=28 = la NetworkPolicy lo corta: no hay respuesta en 5 s)"
echo "-- lo que deja entrar identidad/ore-iam"
kubectl get networkpolicy -n identidad -o json | python -c '
import json,sys
for p in json.load(sys.stdin)["items"]:
  s=p["spec"]
  if (s.get("podSelector",{}).get("matchLabels") or {}).get("ore.dev/rol")!="iam-servidor": continue
  for e in s.get("ingress",[]):
    for f in e.get("from",[]):
      print("  ",p["metadata"]["name"],"<-",f.get("namespaceSelector",{}).get("matchLabels"),f.get("podSelector",{}).get("matchLabels"),f.get("ipBlock",{}).get("cidr"),[x.get("port") for x in e.get("ports",[])])'

echo
echo "== 2 · la celda: el token de identidad de GCP de ore-serve (sólo claims)"
for t in $INQUILINOS; do
  pod "t-$t" m3-quien ore-serve "curl -s -m 10 -H 'Metadata-Flavor: Google' 'http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/identity?audience=ore-iam&format=full' | cut -d. -f2 | python3 -c 'import sys,json,base64; s=sys.stdin.read().strip(); d=json.loads(base64.urlsafe_b64decode(s+\"=\"*(-len(s)%4))); print(\"iss\",d[\"iss\"],\"| aud\",d[\"aud\"],\"| email\",d[\"email\"],\"| sub\",d[\"sub\"],\"| vida\",d[\"exp\"]-d[\"iat\"],\"s\")' || echo 'sin token'"
done
for t in $INQUILINOS; do echo "-- t-$t"; esperar_y_leer "t-$t" m3-quien; done
echo "-- quién puede ser cada ore-serve-<n> (workloadIdentityUser, tokenCreator)"
for t in $INQUILINOS; do
  gcloud iam service-accounts get-iam-policy "ore-serve-$t@$PROYECTO.iam.gserviceaccount.com" --format=json 2>/dev/null \
    | python -c 'import json,sys;[print("   '"$t"'",b["role"].split("/")[-1],b["members"]) for b in json.load(sys.stdin).get("bindings",[])]'
done
echo "-- quién puede crear pods o tokens en el namespace (el que crea un pod elige su cuenta)"
for t in $INQUILINOS; do
  for sa in ore-serve puesto driver informador cofre default; do
    printf "   t-%-7s %-11s pods:%-3s tokens:%s\n" "$t" "$sa" \
      "$(kubectl auth can-i create pods -n "t-$t" --as="system:serviceaccount:t-$t:$sa")" \
      "$(kubectl auth can-i create serviceaccounts/token -n "t-$t" --as="system:serviceaccount:t-$t:$sa")"
  done
done

echo
echo "== 3 · el agente: el precedente del informador"
echo "-- quién lee la credencial del agente de cada celda"
for t in $INQUILINOS; do
  gcloud secrets get-iam-policy "t-$t-agente-secreto" --format=json 2>/dev/null \
    | python -c 'import json,sys;[print("   t-'"$t"'-agente-secreto",b["role"].split("/")[-1],[m.split(":",1)[1].split("@")[0] for m in b["members"]]) for b in json.load(sys.stdin).get("bindings",[])]'
done
echo "-- agentes registrados en iam (a qué organización, y qué celdas tiene esa organización)"
kubectl exec -n identidad idp-db-0 -- sh -c 'psql -U "$POSTGRES_USER" -d iam -Atc "
  select a.nombre, count(*) over (partition by a.sub) as orgs_del_mismo_sub,
         (select string_agg(c.nombre||\$\$:\$\$||c.estado, \$\$,\$\$ order by c.nombre) from iam.celda c where c.organizacion = a.organizacion)
    from iam.agente a order by a.nombre"' | sed 's/^/   /'
echo "-- clientes ore-* en el realm (habilitados)"
kubectl exec -n identidad idp-db-0 -- sh -c 'psql -U "$POSTGRES_USER" -d keycloak -Atc "
  select c.client_id, c.enabled from client c join realm r on r.id = c.realm_id
   where r.name = \$\$rubix\$\$ and c.client_id like \$\$ore-%\$\$ order by 1"' | sed 's/^/   /'
