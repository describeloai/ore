#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# DESPERTAR LA MALLA (ADR 0060 C1) — de los nodos a 0 a poder entrar, en orden.
#
#   bash malla/despertar.sh [--con-postgres]
#
# Lo medido en C1·0 (2026-10-10) es el guion:
#   1. `sistema-spot` a 1 y esperar el nodo (`Ready` en ~54 s).
#   2. Esperar a Kueue (~86 s). ⛔ Hasta que conteste, el API rechaza crear pods
#      en los namespaces que vigila (webhook `mpod.kb.io`, `failurePolicy: Fail`).
#   3. ⭐ EL EMPUJÓN: `rollout restart` de lo que no esté listo. Dormidos, los
#      ReplicaSets acumulan horas de `FailedCreate` y reintentan cada vez más tarde
#      (hasta ~16 min): sin esto, despertar tardaba ~20 min; con él, ~6.
#   4. Esperar a que todo esté listo (salvo `ore-pg` sin `--con-postgres`).
#   5. Relanzar el job `imagen` de la última corrida de `ci` en `main`: dormido, el
#      CI movió las etiquetas sin tocar los Deployments (el webhook de Kueue no
#      contesta); ahora releva lo que haya cambiado y comprueba que corre ESE commit.
#   6. Las entradas públicas contestan (`login`, las celdas).
#
# `--con-postgres`: además, el autoescalado del grupo `pg` (0058); sin él, Postgres
#   sigue dormido y `ore-pg` no se espera.
#
# Necesita `gcloud`, `kubectl` (con el contexto de ore-mesh) y `gh` autenticados.
# Idempotente: con la malla despierta, comprueba y relanza `imagen`, nada más.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=${P:-project-8853a180-450d-47be-b83}
ZONA=europe-west1-b
CLUSTER=ore-mesh
REPO=${REPO:-describeloai/ore}
CON_POSTGRES=no
[ "${1:-}" = --con-postgres ] && CON_POSTGRES=si

T0=$(date +%s)
paso() { printf '%4ss  %s\n' "$(( $(date +%s) - T0 ))" "$*"; }
esperar() {  # esperar <segundos> <qué> <orden…>: hasta que la orden salga bien
  local tope=$1 que=$2; shift 2
  local fin=$(( $(date +%s) + tope ))
  until "$@" >/dev/null 2>&1; do
    [ "$(date +%s)" -lt "$fin" ] || { paso "⛔ $que: no en ${tope}s"; return 1; }
    sleep 5
  done
}

gcloud container clusters get-credentials "$CLUSTER" --zone "$ZONA" --project "$P" >/dev/null 2>&1

# ── 1 · el nodo ─────────────────────────────────────────────────────────────
nodo_listo() { kubectl get nodes -l cloud.google.com/gke-nodepool=sistema-spot --no-headers 2>/dev/null | grep -q ' Ready'; }
if nodo_listo; then
  paso "sistema-spot ya tiene un nodo listo"
else
  paso "sistema-spot → 1"
  gcloud container clusters resize "$CLUSTER" --node-pool sistema-spot --num-nodes 1 \
    --zone "$ZONA" --project "$P" --quiet --async >/dev/null
  esperar 420 "el nodo de sistema-spot" nodo_listo
  paso "nodo listo"
fi

# ── 2 · Kueue: sin él no se crea ningún pod de una celda ───────────────────
kueue_listo() { [ -n "$(kubectl -n kueue-system get endpoints kueue-webhook-service -o jsonpath='{.subsets[*].addresses[*].ip}' 2>/dev/null)" ]; }
esperar 300 "el webhook de Kueue" kueue_listo
paso "Kueue contesta"

# ── 3 · el empujón ─────────────────────────────────────────────────────────
# Lo que no está listo, de los namespaces de ORE y de la plataforma (no kube-system,
# que GKE gestiona; no ore-pg sin --con-postgres: su grupo está a 0).
# ⚠️ Y fuera lo que vive en el grupo de Postgres (`ore.dev/pool: neon`) sin
#   --con-postgres: no sólo `ore-pg`, también cert-manager y el controlador de NeonVM
#   (leído el 2026-10-10). Con ese grupo a 0 no arrancan, y no hace falta.
FUERA="kube-system kube-public kube-node-lease gke-managed-system gke-gmp-system"
POOL_FUERA=neon
[ "$CON_POSTGRES" = si ] && POOL_FUERA=ninguno
no_listos() {  # «ns tipo/nombre» de lo que pide réplicas y no las tiene listas
  kubectl get deploy,sts -A -o jsonpath='{range .items[*]}{.metadata.namespace}|{.kind}/{.metadata.name}|{.spec.replicas}|{.status.readyReplicas}|{.spec.template.spec.nodeSelector.ore\.dev/pool}{"\n"}{end}' \
    | awk -F'|' -v fuera=" $FUERA " -v pool="$POOL_FUERA" \
        '$3>0 && ($4=="" || $4+0<$3+0) && index(fuera, " " $1 " ")==0 && $5!=pool {print $1, $2}'
}
EMPUJADOS=0
while read -r NS OBJ; do
  [ -n "$NS" ] || continue
  kubectl -n "$NS" rollout restart "$(echo "$OBJ" | tr 'A-Z' 'a-z')" >/dev/null 2>&1 && EMPUJADOS=$((EMPUJADOS + 1))
done <<< "$(no_listos)"
paso "empujón: $EMPUJADOS Deployments/StatefulSets"

# ── 4 · todo listo ─────────────────────────────────────────────────────────
todo_listo() { [ -z "$(no_listos)" ]; }
if ! esperar 900 "todo listo" todo_listo; then
  no_listos | sed 's/^/     no listo: /'
  exit 1
fi
paso "todo listo"

if [ "$CON_POSTGRES" = si ]; then
  gcloud container node-pools update pg --cluster "$CLUSTER" --zone "$ZONA" --project "$P" \
    --enable-autoscaling --min-nodes 0 --max-nodes 3 --location-policy ANY --quiet >/dev/null
  paso "pg: autoescalado 0–3 (Postgres despierta con su primer pod)"
fi

# ── 5 · relanzar `imagen` ──────────────────────────────────────────────────
RUN=$(gh run list -R "$REPO" --workflow ci --branch main --event push --limit 1 --json databaseId --jq '.[0].databaseId')
JOB=$(gh run view -R "$REPO" "$RUN" --json jobs --jq '.jobs[] | select(.name=="imagen") | .databaseId')
if [ -z "$JOB" ]; then
  paso "⚠ la última corrida de main ($RUN) no tiene job imagen: nada que relanzar"
else
  # Si esa corrida aún no terminó, se la espera: relanzar exige que esté acabada.
  esperar 1800 "que termine la corrida $RUN" sh -c "[ \"\$(gh run view -R $REPO $RUN --json status --jq .status)\" = completed ]"
  gh run rerun -R "$REPO" "$RUN" --job "$JOB" >/dev/null
  paso "imagen relanzado (corrida $RUN)"
  sleep 10
  esperar 1200 "el job imagen" sh -c "[ \"\$(gh run view -R $REPO $RUN --json status --jq .status)\" = completed ]"
  C=$(gh run view -R "$REPO" "$RUN" --json jobs --jq '.jobs[] | select(.name=="imagen") | .conclusion')
  paso "imagen: $C"
  [ "$C" = success ] || { echo "     https://github.com/$REPO/actions/runs/$RUN"; exit 1; }
fi

# ── 6 · las entradas públicas ──────────────────────────────────────────────
contesta() { local c; c=$(curl -s -o /dev/null -w '%{http_code}' --max-time 8 "$1"); case "$c" in 2*|3*|401|403|404) return 0;; *) return 1;; esac; }
for U in https://login.paladio.io/realms/rubix $(kubectl get httproute -A -o jsonpath='{range .items[*]}https://{.spec.hostnames[0]}/{"\n"}{end}'); do
  esperar 300 "$U" contesta "$U" && paso "contesta $U"
done
paso "✓ despierta"
