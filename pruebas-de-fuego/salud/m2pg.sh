set -e
DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])')
[ "$DICE" = "$EMISOR/realms/$REALM" ] || { echo "emisor inesperado"; exit 1; }
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "http://ore-cofre.t-victor.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos/fuente-postgresql_20260918_1920")
[ "$C" = 200 ] || { echo "custodio $C"; exit 1; }
export U="$(python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])')"; rm -f /tmp/s.json
export PGCONNECT_TIMEOUT=15
FASE="${FASE:-a}"
q() { psql "$U" -XAtq -F ' | ' -c "set statement_timeout=30000" -c "$1" 2>&1; }
lsn() { q "select pg_current_wal_lsn()"; }
testigo() { python3 -c 'import json,os,sys;sys.stdout.write(json.dumps({"url":os.environ["U"],"objeto":sys.argv[1]})+chr(10))' "$1" | /opt/ore/conectores/ore-read-postgres testigo 2>&1 | tr '\n' ' '; }
stats() { q "select relname, 'ins='||n_tup_ins, 'upd='||n_tup_upd, 'del='||n_tup_del, 'mod_desde_analyze='||n_mod_since_analyze, 'vivas='||n_live_tup from pg_stat_user_tables where schemaname='ore_salud_m2' order by 1" | sed 's/^/    /'; }
ms() { python3 -c 'import time;print(int(time.time()*1000))'; }

echo "## entorno ($(date -u +%FT%TZ))"
q "select 'wal_level='||current_setting('wal_level'), 'track_commit_timestamp='||current_setting('track_commit_timestamp'), 'arranque='||pg_postmaster_start_time(), 'replicacion='||(select rolreplication from pg_roles where rolname=current_user), 'max_slots='||current_setting('max_replication_slots'), 'checkpoint_timeout='||current_setting('checkpoint_timeout')"

if [ "$FASE" = "b" ]; then
  echo "## M3·c tras el reposo: ¿siguen los contadores por tabla?"
  stats
  echo "## limpieza"
  q "select count(*) from pg_replication_slots where slot_name='ore_salud_m2'"
  q "drop schema if exists ore_salud_m2 cascade"
  echo "hecho"
  exit 0
fi

if [ "$FASE" = "c" ]; then
  echo "## M2·b el testigo DEL DRIVER sobre a"
  T0=$(testigo ore_salud_m2.a); echo "  T0 = $T0"
  sleep 20; T0b=$(testigo ore_salud_m2.a); echo "  20 s después, sin escribir: $T0b"
  echo "## M2·c se cambia OTRA tabla (b)"
  q "insert into ore_salud_m2.b values (10,'otro')" >/dev/null
  T1=$(testigo ore_salud_m2.a); echo "  T1 = $T1"
  [ "$T0b" = "$T1" ] && echo "  => el testigo de a NO se movió" || echo "  => el testigo de a SE MOVIÓ sin que a cambiara"
  echo "## M2·d se cambia a (update)"
  q "update ore_salud_m2.a set v='x9' where id=1" >/dev/null; echo "  tras update de a: $(testigo ore_salud_m2.a)"
  echo "## M3·a coste del testigo del driver"
  for i in 1 2 3; do a=$(ms); testigo ore_salud_m2.a >/dev/null; b=$(ms); echo "  $((b-a)) ms"; done
  echo "## fin ($(date -u +%FT%TZ)) · arranque del cómputo: $(q "select pg_postmaster_start_time()")"
  stats
  exit 0
fi

echo "## M2·a deriva en reposo: el LSN sin escribir nada, cada 60 s"
for i in 1 2 3 4 5; do
  echo "  $(date -u +%T) $(lsn)"
  [ "$i" -lt 5 ] && sleep 60
done

echo "## preparación: un esquema propio, dos tablas"
q "create schema if not exists ore_salud_m2"
q "create table if not exists ore_salud_m2.a(id int primary key, v text, ts timestamptz default now())"
q "create table if not exists ore_salud_m2.b(id int primary key, v text)"
q "insert into ore_salud_m2.a(id,v) values (1,'x'),(2,'y'),(3,'z') on conflict do nothing"
sleep 2

echo "## M2·b el testigo del driver (ore-read-postgres testigo) sobre a, y el LSN"
T0=$(testigo ore_salud_m2.a); echo "  T0 = $T0"
echo "  ¿y otra vez, sin escribir? $(testigo ore_salud_m2.a)"
stats

echo "## M2·c se cambia OTRA tabla (b); a no se toca"
q "insert into ore_salud_m2.b values (1,'otro')"
T1=$(testigo ore_salud_m2.a); echo "  T1 = $T1"
[ "$T0" = "$T1" ] && echo "  => el testigo de a NO se movió" || echo "  => el testigo de a SE MOVIÓ sin que a cambiara"
sleep 1; stats

echo "## M2·d se cambia a: update y delete"
q "update ore_salud_m2.a set v='x2' where id=1"; echo "  tras update: $(testigo ore_salud_m2.a)"
q "delete from ore_salud_m2.a where id=3";      echo "  tras delete: $(testigo ore_salud_m2.a)"
sleep 1; stats

echo "## M2·e ¿una lectura sola mueve el LSN?"
L1=$(lsn); q "select count(*) from ore_salud_m2.a" >/dev/null; q "select count(*) from ore_salud_m2.b" >/dev/null; L2=$(lsn)
echo "  antes $L1 · después $L2"

echo "## M3·a coste del testigo del driver (5 veces)"
for i in 1 2 3 4 5; do a=$(ms); testigo ore_salud_m2.a >/dev/null; b=$(ms); echo "  $((b-a)) ms"; done

echo "## M3·b decodificación lógica filtrada por tabla (ranura temporal)"
limpiar() { q "select pg_drop_replication_slot('ore_salud_m2') from pg_replication_slots where slot_name='ore_salud_m2'" >/dev/null; }
trap limpiar EXIT
if q "select pg_create_logical_replication_slot('ore_salud_m2','test_decoding')" | grep -qi error; then
  echo "  no se pudo crear la ranura:"; q "select pg_create_logical_replication_slot('ore_salud_m2','test_decoding')" | sed 's/^/    /'
else
  echo "  ranura creada"
  q "insert into ore_salud_m2.b values (2,'otro2')"
  q "insert into ore_salud_m2.b values (3,'otro3')"
  echo "  (sólo cambios en b) ⇒ cambios de a: $(q "select count(*) filter (where data like 'table ore_salud_m2.a:%') || ' de ' || count(*) from pg_logical_slot_peek_changes('ore_salud_m2', null, null)")"
  q "update ore_salud_m2.a set v='x3' where id=1"
  a=$(ms)
  echo "  (y uno en a)     ⇒ cambios de a: $(q "select count(*) filter (where data like 'table ore_salud_m2.a:%') || ' de ' || count(*) from pg_logical_slot_peek_changes('ore_salud_m2', null, null)")"
  b=$(ms); echo "  mirar la ranura: $((b-a)) ms"
  q "select data from pg_logical_slot_peek_changes('ore_salud_m2', null, null) where data like 'table ore_salud_m2.a:%'" | sed 's/^/    /'
  limpiar; echo "  ranura borrada: $(q "select count(*) from pg_replication_slots where slot_name='ore_salud_m2'") quedan"
fi

echo "## M3·d marcas de commit por fila (sólo si track_commit_timestamp=on)"
q "select id, pg_xact_commit_timestamp(xmin) from ore_salud_m2.a order by id" | sed 's/^/  /'

echo "## final ($(date -u +%FT%TZ)) · arranque del cómputo: $(q "select pg_postmaster_start_time()")"
stats
