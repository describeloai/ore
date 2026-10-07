#!/usr/bin/env bash
# D0c·C5 · ¿el primer recorrido escribe WAL? hint bits + wal_log_hints, en main y en una rama.
# B.8: en la rama 16 MB = una página entera por página (2 062 × 8 KB); en main 250 kB.
source "$(dirname "$0")/entorno.sh"
OV=$(ip_overlay)
wal() { qo "$1" "select pg_current_wal_lsn()"; }
dif() { qo "$1" "select pg_size_pretty(pg_wal_lsn_diff('$3','$2'))"; }

echo "== preparar en main: 100 000 filas, VACUUM (hint bits puestos), y luego UPDATE de todas (filas nuevas SIN hint bits)"
qo "$OV" "drop table if exists h; create table h(id int primary key, v text); insert into h select g, repeat('x',50) from generate_series(1,100000) g" >/dev/null
qo "$OV" "vacuum h" >/dev/null
qo "$OV" "update h set v = repeat('y',50)" >/dev/null
echo "   páginas de h: $(qo "$OV" "analyze h; select relpages from pg_class where relname='h'")"
# el LSN en un comando APARTE: dentro de la misma transacción saldría de antes del commit (B.8 C3)
LSN=$(wal "$OV"); echo "   LSN del punto de rama: $LSN"; sleep 3

"$AQUI/tenant.sh" rama hints "$LSN" || exit 1
ORE_PG_VM=pg-hints "$AQUI/vm.sh" "$(leer rama-hints)" >/dev/null || exit 1
# por la IP del pod: al recrear una VM la overlay puede tardar ~1 min en aprender la MAC nueva (C4)
OVR=$(ip_pod pg-hints)

for x in "rama $OVR" "main $OV"; do
  set -- $x
  a=$(wal "$2"); qo "$2" "select count(*) from h" >/dev/null; b=$(wal "$2"); qo "$2" "select count(*) from h" >/dev/null; c=$(wal "$2")
  echo "== $1: 1er recorrido → WAL $(dif "$2" "$a" "$b"); 2º recorrido → WAL $(dif "$2" "$b" "$c")"
done
echo "== wal_log_hints $(qo "$OV" 'show wal_log_hints') · checksums $(qo "$OV" 'show data_checksums')"
k delete neonvm pg-hints --wait=false >/dev/null
