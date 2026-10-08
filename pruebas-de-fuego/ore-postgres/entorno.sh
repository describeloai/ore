#!/usr/bin/env bash
# El entorno de las pruebas de fuego de ORE Serverless Postgres (ADR 0058): se carga con
# `source entorno.sh` desde cada prueba. TODO lo que cambia entre un montaje y otro es una variable
# ORE_PG_* con valor por defecto; el estado de una prueba a la siguiente (ids, IPs, la clave del
# JWT, la especificación) vive en $ORE_PG_TRABAJO, nunca en el repositorio.
#
#   ORE_PG_NS           namespace del almacenamiento                   (ore-pg: el de la malla, P2·4)
#   ORE_PG_NS_COMPUTO   namespace de las VMs                           (ore-pg-computo: el del producto, P3·4)
#   ORE_PG_VM           nombre del cómputo                             (pg-prueba)
#   ORE_PG_COMPUTO      vm (NeonVM, P3) | pod (contenedor: sin KVM ni overlay, P2·6)   (pod)
#   ORE_PG_POOL         etiqueta ore.dev/pool de los nodos con KVM     (neon)
#   ORE_PG_BUCKET       bucket de GCS del almacenamiento               (obligatorio para almacen-gcs.yaml)
#   ORE_PG_GSA          cuenta de Google del almacenamiento (Workload Identity) (obligatoria para almacen-gcs.yaml)
#   ORE_PG_REGISTRO     registro de imágenes                           (el de ORE)
#   ORE_PG_COMMIT       commit de describeloai/neon de las imágenes    (el de ci/neon/*.yaml)
#   ORE_PG_TRABAJO      directorio de estado                           ($TMPDIR/ore-pg-<ns>)
set -u
# Git Bash reescribe los argumentos que parecen rutas (/tmp/… → C:/…/Temp/…) también los de
# `kubectl exec`: aquí todas las rutas que van a Windows ya son C:/…
export MSYS_NO_PATHCONV=1
# Y Python en UTF-8: en Windows escribe en cp1252 y un «✓» lo tumbaba (p431, medido).
export PYTHONUTF8=1
AQUI=$(cd "$(dirname "${BASH_SOURCE[0]}")" && { pwd -W 2>/dev/null || pwd; })   # C:/… en Git Bash: la entienden bash y Python

: "${ORE_PG_NS:=ore-pg}"
: "${ORE_PG_NS_COMPUTO:=ore-pg-computo}"
: "${ORE_PG_VM:=pg-prueba}"
: "${ORE_PG_COMPUTO:=pod}"
: "${ORE_PG_POOL:=neon}"
: "${ORE_PG_BUCKET:=}"
: "${ORE_PG_GSA:=}"
: "${ORE_PG_REGISTRO:=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore}"
: "${ORE_PG_COMMIT:=$(sed -n 's/^  _COMMIT: \([0-9a-f]\{40\}\).*/\1/p' "$AQUI/../../ci/neon/computo.yaml")}"
: "${ORE_PG_TRABAJO:=${TMPDIR:-/tmp}/ore-pg-$ORE_PG_NS}"
: "${ORE_PG_IMAGEN_NEON:=$ORE_PG_REGISTRO/neon:$ORE_PG_COMMIT}"
: "${ORE_PG_IMAGEN_VM:=$ORE_PG_REGISTRO/vm-compute-node-v17:$ORE_PG_COMMIT}"
: "${ORE_PG_IMAGEN_COMPUTO:=$ORE_PG_REGISTRO/compute-node-v17:$ORE_PG_COMMIT}"
export ORE_PG_NS ORE_PG_NS_COMPUTO ORE_PG_VM ORE_PG_COMPUTO ORE_PG_IMAGEN_COMPUTO ORE_PG_POOL ORE_PG_BUCKET ORE_PG_GSA ORE_PG_REGISTRO ORE_PG_COMMIT \
       ORE_PG_TRABAJO ORE_PG_IMAGEN_NEON ORE_PG_IMAGEN_VM
mkdir -p "$ORE_PG_TRABAJO"
# en Windows (Git Bash) bash y Python no ven el mismo /tmp: una ruta C:/… la entienden los dos
ORE_PG_TRABAJO=$(cd "$ORE_PG_TRABAJO" && { pwd -W 2>/dev/null || pwd; }); export ORE_PG_TRABAJO

# ── tiempo ──────────────────────────────────────────────────────────────────────────────────────
ms() { echo $(( $(date +%s%N) / 1000000 )); }
ts() { date -u +%H:%M:%S.%3N; }

# ── kubectl y estado ────────────────────────────────────────────────────────────────────────────
k() { kubectl -n "$ORE_PG_NS" "$@"; }
kc() { kubectl -n "$ORE_PG_NS_COMPUTO" "$@"; }                     # las VMs (P3·4)
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
# con ORE_PG_COMPUTO=pod no hay overlay: las dos dan la IP del pod
if [ "$ORE_PG_COMPUTO" = pod ]; then
  ip_pod()     { k get pod "${1:-$ORE_PG_VM}" -o jsonpath='{.status.podIP}' 2>/dev/null; }
  ip_overlay() { ip_pod "$@"; }
  qo() { q "$@"; }
else
  ip_pod()     { kc get neonvm "${1:-$ORE_PG_VM}" -o jsonpath='{.status.podIP}' 2>/dev/null; }
  ip_overlay() { kc get neonvm "${1:-$ORE_PG_VM}" -o jsonpath='{.status.extraNetIP}' 2>/dev/null; }
fi

# ── autenticación (P2·4): si el namespace tiene `almacen-jwt`, el almacenamiento la exige ─────────
# Los tokens se leen de los Secrets a $ORE_PG_TRABAJO/jwt-* (fuera del repo); la privada sólo hace
# falta para acuñar el token de tenant del cómputo (especificacion.py).
secreto() {  # secreto <secret> <clave> → fichero en $ORE_PG_TRABAJO
  local f="$ORE_PG_TRABAJO/jwt-$2"
  [ -s "$f" ] || k get secret "$1" -o go-template="{{index .data \"$2\"}}" | base64 -d > "$f"
  echo "$f"
}
if k get secret almacen-jwt >/dev/null 2>&1; then ORE_PG_AUTH=si; else ORE_PG_AUTH=no; fi
export ORE_PG_AUTH
[ "$ORE_PG_AUTH" = si ] && export ORE_PG_PRIVADA=$(secreto almacen-jwt-privada privada.pem)

# api <host> <puerto> <MÉTODO> <ruta> [json] [token] — HTTP desde el pod `cliente`, sin port-forward
api() {
  local cuerpo=${5:-} aut=""
  [ -n "${6:-}" ] && aut="Authorization: Bearer $6\r\n"
  k exec cliente -- bash -c "exec 3<>/dev/tcp/$1/$2
    printf '%s %s HTTP/1.0\r\nHost: x\r\n${aut}Content-Type: application/json\r\nContent-Length: %s\r\n\r\n%s' \
      '$3' '$4' '${#cuerpo}' '$cuerpo' >&3; timeout 120 cat <&3" 2>/dev/null | tail -1
}
# controlador <MÉTODO> <ruta> [json]   (token admin)
controlador() {
  local t=""; [ "$ORE_PG_AUTH" = si ] && t=$(cat "$(secreto almacen-jwt-privada admin)")
  api "storage-controller.$ORE_PG_NS.svc.cluster.local" 1234 "$1" "$2" "${3:-}" "$t"
}
# pageserver <MÉTODO> <ruta> [json]    (token pageserverapi)
pageserver() {
  local t=""; [ "$ORE_PG_AUTH" = si ] && t=$(cat "$(secreto almacen-jwt pageserverapi)")
  api "pageserver-0.$ORE_PG_NS.svc.cluster.local" 9898 "$1" "$2" "${3:-}" "$t"
}
