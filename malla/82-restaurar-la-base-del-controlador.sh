#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# RESTAURAR LA BASE DEL storage_controller (ADR 0058, P2·3)
#
#   82-restaurar-la-base-del-controlador.sh                 → el volcado más reciente
#   82-restaurar-la-base-del-controlador.sh <gs://…dump>    → ése
#
# ⛔ NUNCA un `pg_restore` a pelo. Medido en P2·1 (pruebas-de-fuego/ore-postgres/
# controlador-local): con una copia vieja el controller vuelve a repartir
# generaciones YA USADAS, y dos pageservers pueden creerse con derecho a escribir
# el mismo tenant. Por eso, entre restaurar y arrancar:
#
#   tenant_shards.generation  += SALTO   (las de los pageservers)
#   timelines.generation      += SALTO   (las de los safekeepers, si el controller
#                                         los gestiona: --timelines-onto-safekeepers)
#
# SALTO = 1000 por defecto: más que todos los reenganches que caben entre dos
# copias diarias. Probado en local: reparte 1010 y el timeline sigue sano.
#
# Orden: parar el controller → restaurar → saltar → arrancar. Si el controller
# todavía no existe (P2·3, antes de P2·4), se salta lo de pararlo.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
NS=ore-pg
SALTO=${SALTO:-1000}
ORIGEN=gs://project-8853a180-450d-47be-b83-copias/ore-pg
VOLCADO=${1:-$(gcloud storage ls "$ORIGEN/storage_controller-*.dump" | sort | tail -1)}
[ -n "$VOLCADO" ] || { echo "✗ no hay volcados en $ORIGEN"; exit 1; }
echo "── restaurar $VOLCADO (salto de generaciones +$SALTO)"

REPLICAS=0
if kubectl -n $NS get deploy storage-controller >/dev/null 2>&1; then
  REPLICAS=$(kubectl -n $NS get deploy storage-controller -o jsonpath='{.spec.replicas}')
  echo "── parar el controller ($REPLICAS réplica/s → 0)"
  kubectl -n $NS scale deploy storage-controller --replicas 0
  kubectl -n $NS wait --for=delete pod -l ore.dev/rol=storage-controller --timeout=120s 2>/dev/null || true
fi

DB=$(kubectl -n $NS get pod -l ore.dev/rol=storcon-db -o jsonpath='{.items[0].metadata.name}')
TMP=$(mktemp)
gcloud storage cp "$VOLCADO" "$TMP" >/dev/null
echo "── volcado bajado: $(wc -c < "$TMP") bytes"
# dentro del pod de la base: su usuario y su contraseña ya están en su entorno
psqlx() { kubectl -n $NS exec -i "$DB" -- sh -c "PGPASSWORD=\$POSTGRES_PASSWORD $*"; }
psqlx 'psql -U $POSTGRES_USER -d postgres -qc "drop database if exists storage_controller with (force)"'
psqlx 'psql -U $POSTGRES_USER -d postgres -qc "create database storage_controller"'
psqlx 'pg_restore -U $POSTGRES_USER -d storage_controller --no-owner' < "$TMP"
rm -f "$TMP"

echo "── saltar las generaciones +$SALTO"
psqlx "psql -U \$POSTGRES_USER -d storage_controller -v ON_ERROR_STOP=1 -At" <<SQL
update tenant_shards set generation = generation + $SALTO where generation is not null;
do \$\$ begin
  if exists (select from information_schema.columns where table_name='timelines' and column_name='generation') then
    execute 'update timelines set generation = generation + $SALTO';
  end if;
end \$\$;
select 'tenant_shards: ' || count(*) || ' · generación máxima ' || coalesce(max(generation)::text, '-') from tenant_shards;
SQL

if [ "$REPLICAS" != 0 ]; then
  echo "── arrancar el controller ($REPLICAS)"
  kubectl -n $NS scale deploy storage-controller --replicas "$REPLICAS"
  kubectl -n $NS rollout status deploy storage-controller --timeout=180s
fi
echo "── hecho"
