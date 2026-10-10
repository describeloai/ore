#!/usr/bin/env bash
# El muestreador del soak en GCP (ADR 0058, P7·4, objetivos 8 y 9): cada CADA segundos apunta en
# $MEDIDAS/recursos.jsonl la CPU y la memoria de cada pod de `ore-pg` (ore-postgres, el proxy,
# Redis, el pageserver, los safekeepers, el controller), cuántas VMs de cómputo hay (las del pool,
# aparte) y cuántos nodos del pool `pg`. El informe lo lee: si algo crece (objetivo 9) y si de
# noche no queda ni VM ni nodo (objetivo 8).
#
# Corre en la máquina del cliente, con el kubectl de la cuenta de servicio de esa máquina (sin
# llaves: las credenciales del metadata server; ver la receta).
#
#   MEDIDAS=~/libro-medidas muestrea.sh [CADA=300]
#   SECO=1 muestrea.sh 1      → una vuelta con datos de mentira, sin kubectl (para probarlo)
set -uo pipefail
M="${MEDIDAS:-$HOME/libro-medidas}"
CADA="${1:-300}"
mkdir -p "$M"
ahora() { date -u +%Y-%m-%dT%H:%M:%S.000Z; }

if [ -n "${SECO:-}" ]; then
  k() {
    case "$*" in
      "top pod -n ore-pg --no-headers") printf 'ore-postgres-7d9f-abcde 4m 31Mi\nproxy-postgres-5c6b-fghij 9m 58Mi\nproxy-postgres-5c6b-klmno 8m 55Mi\npageserver-0 40m 610Mi\nsafekeeper-0 6m 120Mi\n';;
      "get neonvm -n ore-pg-computo --no-headers -o custom-columns=N:.metadata.name") printf 'ep-123abc\npool-9f8e7d\n';;
      "get nodes -l ore.dev/pool=pg --no-headers") printf 'gke-ore-mesh-pg-1 Ready\n';;
    esac
  }
else
  k() { kubectl "$@"; }
fi

rol_de() {
  case "$1" in
    ore-postgres-*) echo ore-postgres;; proxy-postgres-*) echo proxy;; redis-postgres-*) echo redis;;
    pageserver-*) echo pageserver;; safekeeper-*) echo safekeeper;; storage-controller-*) echo storage-controller;;
    storage-broker-*) echo storage-broker;; storcon-db-*) echo storcon-db;; *) echo otro;;
  esac
}

vuelta() {
  local t; t=$(ahora)
  local pods; pods=$(k top pod -n ore-pg --no-headers 2>&1) || { echo "{\"t\":\"$t\",\"tipo\":\"error\",\"que\":\"top pod\"}" >> "$M/recursos.jsonl"; pods=""; }
  while read -r pod cpu mem; do
    [ -n "${pod:-}" ] || continue
    printf '{"t":"%s","tipo":"pod","pod":"%s","rol":"%s","cpu_m":%s,"mem_mi":%s}\n' \
      "$t" "$pod" "$(rol_de "$pod")" "${cpu%m}" "${mem%Mi}" >> "$M/recursos.jsonl"
  done <<<"$pods"
  local vms; vms=$(k get neonvm -n ore-pg-computo --no-headers -o custom-columns=N:.metadata.name 2>/dev/null)
  local n_vms n_pool n_nodos
  n_vms=$(grep -c . <<<"$vms"); n_pool=$(grep -c '^pool-' <<<"$vms")
  n_nodos=$(k get nodes -l ore.dev/pool=pg --no-headers 2>/dev/null | grep -c .)
  printf '{"t":"%s","tipo":"computo","vms":%s,"vms_pool":%s,"nodos_pg":%s}\n' \
    "$t" "$(( n_vms - n_pool ))" "$n_pool" "$n_nodos" >> "$M/recursos.jsonl"
}

if [ -n "${SECO:-}" ]; then vuelta; exit 0; fi
while :; do vuelta; sleep "$CADA"; done
