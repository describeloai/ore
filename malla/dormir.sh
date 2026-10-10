#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# DORMIR LA MALLA (ADR 0060 C1) — los grupos de nodos a 0; los discos se quedan.
#
#   bash malla/dormir.sh [--forzar | --si-inactiva <min>]
#
#   `--si-inactiva 15` (C1·2): sólo duerme si nadie la usó en esos minutos. Lo que cuenta
#      como uso: una línea `persona` o `delegado` en el registro de `ore-serve` (cada
#      celda) o de `ore-iam` —no las de `fondo`, que son refrescos de la consola sin nadie
#      delante, ni las de sondas, agentes o el aprovisionador—, un puesto abierto, o
#      trabajo en Kueue. Y nunca antes de esos minutos despierta: el registro sería corto.
#      Si no duerme, sale 0 y lo dice (para el workflow que la mira cada pocos minutos).
#
#   1. ¿Hay trabajo? Workloads de Kueue sin terminar (builds, copias, puestos) o
#      Jobs de usuario corriendo: entonces NO duerme (sale 3), salvo `--forzar`.
#   2. ⭐ Las copias, al dormir: los datos sólo cambian con la malla despierta, así
#      que la copia de ahora es la última que hace falta (las CronJobs diarias no
#      corren dormidas). La forja y el idp; la base del controlador de Postgres si
#      su grupo está en pie.
#   3. `cordon` + `drain` de los nodos: Keycloak, las bases y las forjas se paran
#      limpias (C1·0: 130 s), no de golpe.
#   4. `sistema-spot`, `jobs-s` y `pg` a 0 (`pg`, además, sin autoescalado: sin
#      taint, el autoescalador lo subía por los pods de kube-system; ADR 0060 A1).
#
# Necesita `gcloud` y `kubectl` autenticados. Idempotente: dormida, no hace nada.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=${P:-project-8853a180-450d-47be-b83}
ZONA=europe-west1-b
CLUSTER=ore-mesh
FORZAR=no
INACTIVA=""
case "${1:-}" in
  --forzar) FORZAR=si ;;
  --si-inactiva) INACTIVA=${2:?--si-inactiva <minutos>} ;;
esac

T0=$(date +%s)
paso() { printf '%4ss  %s\n' "$(( $(date +%s) - T0 ))" "$*"; }

gcloud container clusters get-credentials "$CLUSTER" --zone "$ZONA" --project "$P" >/dev/null 2>&1

NODOS=$(kubectl get nodes -o name 2>/dev/null || true)
if [ -z "$NODOS" ]; then
  paso "ya duerme: ningún nodo"
  exit 0
fi

# ── 0 · ¿la usa alguien? (sólo con --si-inactiva) ──────────────────────────
if [ -n "$INACTIVA" ]; then
  # Despierta hace menos de esos minutos: no se sabe aún (el registro empieza al arrancar).
  DESDE=$(kubectl get nodes -l cloud.google.com/gke-nodepool=sistema-spot -o jsonpath='{.items[0].metadata.creationTimestamp}' 2>/dev/null)
  if [ -n "$DESDE" ] && [ $(( $(date +%s) - $(date -d "$DESDE" +%s) )) -lt $(( INACTIVA * 60 )) ]; then
    paso "despierta desde hace menos de ${INACTIVA} min: sigue"
    exit 0
  fi
  USO=""
  for D in $(kubectl get ns -l ore.dev/rol=cargas -o jsonpath='{range .items[*]}{.metadata.name}/ore-serve {end}') identidad/ore-iam; do
    N=$(kubectl -n "${D%%/*}" logs "deploy/${D##*/}" --since="${INACTIVA}m" 2>/dev/null | grep -cE '^acceso · .*· (persona|delegado)$' || true)
    [ "${N:-0}" -gt 0 ] && USO="$USO $D:$N"
  done
  PUESTOS=$(kubectl get pods -A -l ore.dev/rol=puesto --field-selector=status.phase=Running --no-headers 2>/dev/null | wc -l)
  [ "$PUESTOS" -gt 0 ] && USO="$USO puestos:$PUESTOS"
  if [ -n "$USO" ]; then
    paso "en uso en los últimos ${INACTIVA} min:$USO — sigue despierta"
    exit 0
  fi
  paso "nadie en ${INACTIVA} min: a dormir"
fi

# ── 1 · ¿hay trabajo? ──────────────────────────────────────────────────────
# Un workload de Kueue admitido y sin terminar es trabajo de alguien.
EN_CURSO=$(kubectl get workloads.kueue.x-k8s.io -A -o jsonpath='{range .items[*]}{.metadata.namespace}/{.metadata.name} {.status.conditions[?(@.type=="Admitted")].status} {.status.conditions[?(@.type=="Finished")].status}{"\n"}{end}' 2>/dev/null \
  | awk '$2=="True" && $3!="True" {print $1}')
if [ -n "$EN_CURSO" ] && [ "$FORZAR" = no ]; then
  paso "hay trabajo en curso; no duerme (--forzar para dormir igual):"
  echo "$EN_CURSO" | sed 's/^/     /'
  [ -n "$INACTIVA" ] && exit 0   # para quien la mira cada pocos minutos, «sigue» no es un fallo
  exit 3
fi

# ── 2 · las copias ─────────────────────────────────────────────────────────
copiar() {  # copiar <ns> <cronjob>: un Job desde la CronJob, y esperarlo
  local ns=$1 cj=$2 job
  kubectl -n "$ns" get cronjob "$cj" >/dev/null 2>&1 || return 0
  job="$cj-al-dormir-$(date -u +%Y%m%d%H%M%S)"
  kubectl -n "$ns" create job "$job" --from="cronjob/$cj" >/dev/null
  if kubectl -n "$ns" wait --for=condition=complete "job/$job" --timeout=600s >/dev/null 2>&1; then
    paso "copia $ns/$cj hecha"
  else
    paso "⛔ copia $ns/$cj no terminó (kubectl -n $ns logs job/$job)"
    [ "$FORZAR" = si ] || exit 1
  fi
}
copiar forja copia-de-la-forja &
copiar identidad copia-del-idp &
if kubectl get nodes -l ore.dev/pool=neon --no-headers 2>/dev/null | grep -q ' Ready'; then
  copiar ore-pg copia-de-la-base-del-controlador &
fi
wait

# ── 3 · vaciar los nodos ───────────────────────────────────────────────────
for N in $NODOS; do kubectl cordon "$N" >/dev/null; done
for N in $NODOS; do
  kubectl drain "$N" --ignore-daemonsets --delete-emptydir-data --timeout=240s >/dev/null 2>&1 \
    || paso "⚠ $N no se vació del todo en 240 s; se apaga igual"
done
paso "nodos vaciados"

# ── 4 · a 0 ────────────────────────────────────────────────────────────────
gcloud container node-pools update pg --cluster "$CLUSTER" --zone "$ZONA" --project "$P" \
  --no-enable-autoscaling --quiet >/dev/null 2>&1 || true
for POOL in sistema-spot jobs-s pg; do
  gcloud container clusters resize "$CLUSTER" --node-pool "$POOL" --num-nodes 0 \
    --zone "$ZONA" --project "$P" --quiet >/dev/null 2>&1 &
done
wait
paso "✓ dormida: los grupos de nodos a 0 (GKE retira las máquinas)"
