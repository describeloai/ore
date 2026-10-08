#!/usr/bin/env bash
# P4·7 · Aceptación (ADR 0058). Hecho cuando: todo por la API —proyecto, rama, endpoint en la rama, rol,
# base, escribir— y borrarlo todo por la API no deja huella: ni filas en `ore_postgres`, ni VMs, ni
# ConfigMaps, ni prefijos en GCS, ni WAL en los safekeepers, ni tenant en el pageserver.
#
#   p47.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p47-$(date +%s | tail -c 6)"
PRUEBA=p47
BUCKET=gs://ore-pg-almacen-euw1
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
R="/v1/postgres/proyectos/$P/ramas"
gcs_ls() { env -u MSYS_NO_PATHCONV gcloud storage ls -r "$1"; }
filas() { k exec storcon-db-0 -- sh -c "psql -U \"\$POSTGRES_USER\" -d ore_postgres -Atc \"select
  (select count(*) from plano.proyecto where id='$P') + (select count(*) from plano.rama where proyecto='$P')
  + (select count(*) from plano.endpoint where proyecto='$P') + (select count(*) from plano.rol where proyecto='$P')
  + (select count(*) from plano.base where proyecto='$P')\""; }
huella() {   # huella → «filas vms configmaps gcs safekeepers pageserver»
  local vms cms g sk ps
  vms=$(kc get neonvm -l "ore.dev/proyecto=$P" -o name | wc -l)
  cms=$(kc get configmap -o name | grep -c "$VMS_RE")
  g=$(gcs_ls "$BUCKET/**" 2>/dev/null | grep -c "$TENANT")
  sk=0; for i in 0 1 2; do sk=$((sk + $(k exec safekeeper-$i -- sh -c 'ls -1 /data' 2>/dev/null | grep -c "$TENANT"))); done
  ps=$(pageserver GET /v1/tenant | grep -c "$TENANT")
  echo "filas=$(filas) vms=$vms configmaps=$cms gcs=$g safekeepers=$sk pageserver=$ps"
}

echo "── $A crea $P con dueño, una rama dev con su cómputo, un rol y una base"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\",\"dueno\":\"user:p47\"}'")
OP=$(campo "${L[0]}" operacion id); TENANT=$(campo "${L[0]}" proyecto tenant)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide POST $R '{\"id\":\"dev\"}'")
[ "$(campo "${L[0]}" estado)" = hecha ] || { echo "  ✗ proyecto: ${L[0]:0:300}"; exit 1; }
OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide POST $R/dev/endpoints '{\"id\":\"uno\"}'")
OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide POST $R/dev/roles '{\"nombre\":\"app\"}'")
OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide POST $R/dev/bases '{\"nombre\":\"datos\",\"dueno\":\"app\"}'")
OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $R/main/endpoints/principal" "pide GET $R/dev/endpoints/uno")
[ "$(campo "${L[0]}" estado)" = hecha ] || { echo "  ✗ la base: ${L[0]:0:300}"; exit 1; }
DIR=$(campo "${L[1]}" direccion); DIR_DEV=$(campo "${L[2]}" direccion)
VMS_RE="$(campo "${L[1]}" vm)\|$(campo "${L[2]}" vm)"
echo "  ✓ tenant $TENANT · principal $DIR · dev/uno $DIR_DEV"
qo "$DIR" "create table t as select g from generate_series(1, 200000) g" >/dev/null
qo "$DIR_DEV" "select count(*) from pg_database where datname='datos'" | grep -qx 1 && echo "  ✓ la base datos está en dev" || echo "  ✗ sin la base datos en dev"
for i in $(seq 1 18); do [ "$(gcs_ls "$BUCKET/safekeeper/$TENANT/**" 2>/dev/null | grep -c '^gs://')" -gt 0 ] && break; sleep 10; done
echo "  · durante: $(huella)"

echo "── se borra por la API: el endpoint de dev, la rama, el proyecto"
mapfile -t L < <(en "$A" "pide DELETE $R/dev/endpoints/uno"); OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide DELETE $R/dev"); OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide DELETE /v1/postgres/proyectos/$P"); OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] || { echo "  ✗ borrar: ${L[0]:0:300}"; exit 1; }
sleep 20
D=$(huella); echo "  · después: $D"
[ "$D" = "filas=0 vms=0 configmaps=0 gcs=0 safekeepers=0 pageserver=0" ] && echo && echo "P4·7 ✓ no queda huella" \
  || { echo; echo "P4·7 ✗ queda huella"; exit 1; }
