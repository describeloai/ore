#!/usr/bin/env bash
# El entorno de las pruebas de fuego de ORE Serverless Postgres (ADR 0058): se carga con
# `source entorno.sh` desde cada prueba. TODO lo que cambia entre un montaje y otro es una variable
# ORE_PG_* con valor por defecto; el estado de una prueba a la siguiente (ids, IPs, la clave del
# JWT, la especificación) vive en $ORE_PG_TRABAJO, nunca en el repositorio.
#
#   ORE_PG_NS           namespace de la prueba                         (ore-pg-prueba)
#   ORE_PG_VM           nombre de la VM de cómputo                     (pg-prueba)
#   ORE_PG_POOL         etiqueta ore.dev/pool de los nodos con KVM     (neon)
#   ORE_PG_BUCKET       bucket de GCS del almacenamiento               (obligatorio para almacen-gcs.yaml)
#   ORE_PG_GSA          cuenta de Google del almacenamiento (Workload Identity) (obligatoria para almacen-gcs.yaml)
#   ORE_PG_REGISTRO     registro de imágenes                           (el de ORE)
#   ORE_PG_COMMIT       commit de describeloai/neon de las imágenes    (el de ci/neon/*.yaml)
#   ORE_PG_TRABAJO      directorio de estado                           ($TMPDIR/ore-pg-<ns>)
set -u
AQUI=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

: "${ORE_PG_NS:=ore-pg-prueba}"
: "${ORE_PG_VM:=pg-prueba}"
: "${ORE_PG_POOL:=neon}"
: "${ORE_PG_BUCKET:=}"
: "${ORE_PG_GSA:=}"
: "${ORE_PG_REGISTRO:=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore}"
: "${ORE_PG_COMMIT:=$(sed -n 's/^  _COMMIT: \([0-9a-f]\{40\}\).*/\1/p' "$AQUI/../../ci/neon/computo.yaml")}"
: "${ORE_PG_TRABAJO:=${TMPDIR:-/tmp}/ore-pg-$ORE_PG_NS}"
: "${ORE_PG_IMAGEN_NEON:=$ORE_PG_REGISTRO/neon:$ORE_PG_COMMIT}"
: "${ORE_PG_IMAGEN_VM:=$ORE_PG_REGISTRO/vm-compute-node-v17:$ORE_PG_COMMIT}"
export ORE_PG_NS ORE_PG_VM ORE_PG_POOL ORE_PG_BUCKET ORE_PG_GSA ORE_PG_REGISTRO ORE_PG_COMMIT \
       ORE_PG_TRABAJO ORE_PG_IMAGEN_NEON ORE_PG_IMAGEN_VM
mkdir -p "$ORE_PG_TRABAJO"

# ── tiempo ──────────────────────────────────────────────────────────────────────────────────────
ms() { echo $(( $(date +%s%N) / 1000000 )); }
ts() { date -u +%H:%M:%S.%3N; }

# ── kubectl y estado ────────────────────────────────────────────────────────────────────────────
k() { kubectl -n "$ORE_PG_NS" "$@"; }
guardar() { printf '%s\n' "$2" > "$ORE_PG_TRABAJO/$1"; }              # guardar <clave> <valor>
leer() { cat "$ORE_PG_TRABAJO/$1" 2>/dev/null || { echo "✗ falta $1 en $ORE_PG_TRABAJO (¿se corrió el paso anterior?)" >&2; return 1; }; }

# ── manifiestos: ${ORE_PG_*} → su valor (sin envsubst, que no está en todos los sitios) ──────────
plantilla() { python "$AQUI/plantilla.py" "$@"; }                     # plantilla <fichero> [VAR=valor…]

# ── Postgres ────────────────────────────────────────────────────────────────────────────────────
# q <host> <sql>: desde el pod `cliente` (red de pods); qo: desde `cliente-overlay` (la overlay de
# las VMs: sobrevive a la migración, B.6). cloud_admin no cuenta para last_active (B.5).
PGX="env PGPASSWORD=cloud_admin PGCONNECT_TIMEOUT=${PGCONNECT_TIMEOUT:-5}"
q()  { k exec cliente         -- $PGX psql -h "$1" -p 55433 -U cloud_admin -d postgres -Atc "$2" 2>&1; }
qo() { k exec cliente-overlay -- $PGX psql -h "$1" -p 55433 -U cloud_admin -d postgres -Atc "$2" 2>&1; }

# ip_pod / ip_overlay de una VM (por defecto $ORE_PG_VM)
ip_pod()     { k get neonvm "${1:-$ORE_PG_VM}" -o jsonpath='{.status.podIP}' 2>/dev/null; }
ip_overlay() { k get neonvm "${1:-$ORE_PG_VM}" -o jsonpath='{.status.extraNetIP}' 2>/dev/null; }

# ── la API HTTP del pageserver, desde dentro (sin port-forward) ─────────────────────────────────
# pageserver <MÉTODO> <ruta> [json]   (no `ps`: taparía el comando)
pageserver() {
  local cuerpo=${3:-}
  k exec cliente-overlay -- bash -c "exec 3<>/dev/tcp/pageserver.$ORE_PG_NS.svc.cluster.local/9898
    printf '%s %s HTTP/1.0\r\nHost: p\r\nContent-Type: application/json\r\nContent-Length: %s\r\n\r\n%s' \
      '$1' '$2' '${#cuerpo}' '$cuerpo' >&3; timeout 120 cat <&3" 2>/dev/null | tail -1
}
