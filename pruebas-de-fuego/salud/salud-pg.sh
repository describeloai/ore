set -e
DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])')
[ "$DICE" = "$EMISOR/realms/$REALM" ] || { echo "emisor inesperado"; exit 1; }
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "http://ore-cofre.t-victor.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos/fuente-postgresql_20260918_1920")
[ "$C" = 200 ] || { echo "custodio $C"; exit 1; }
export U="$(python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])')"; rm -f /tmp/s.json
export PGCONNECT_TIMEOUT=15
FASE="${FASE:-ver}"
q() { psql "$U" -XAtq -F ' | ' -c "set statement_timeout=60000" -c "$1" 2>&1; }
if [ "$FASE" = "preparar" ]; then
  q "create schema if not exists ore_salud"
  q "create table if not exists ore_salud.pedidos(id int primary key, cliente text, total numeric(10,2) not null, creado timestamptz not null default now())"
  q "insert into ore_salud.pedidos(id, cliente, total, creado) select g, case when g % 10 = 0 then null else 'cliente-' || (g % 37) end, round((g * 1.37)::numeric, 2), timestamptz '2026-10-01 00:00:00+00' + g * interval '1 minute' from generate_series(1, 1000) g on conflict do nothing"
fi
if [ -n "$SQL" ]; then echo "## $SQL"; q "$SQL" | sed 's/^/  /'; fi
echo "## estado de ore_salud.pedidos ($(date -u +%FT%TZ))"
q "select 'filas='||count(*), 'cliente_no_nulos='||count(cliente), 'total_no_nulos='||count(total), 'max_creado='||max(creado) from ore_salud.pedidos" | sed 's/^/  /'
q "select column_name, data_type, is_nullable from information_schema.columns where table_schema='ore_salud' and table_name='pedidos' order by ordinal_position" | sed 's/^/  /'
q "select 'lsn='||pg_current_wal_lsn(), 'stats: ins='||n_tup_ins||' upd='||n_tup_upd||' del='||n_tup_del from pg_stat_user_tables where schemaname='ore_salud' and relname='pedidos'" | sed 's/^/  /'
