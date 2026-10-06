#!/usr/bin/env bash
# D0c·C5 · ¿el primer recorrido escribe WAL? hint bits + wal_log_hints, en main y en una rama.
D=/c/tmp/d0b; read T TL < $D/ids-gcs.txt; OV=$(cat $D/vm-ov.txt)
C="kubectl -n d0-neon exec cliente-overlay -- env PGPASSWORD=cloud_admin PGCONNECT_TIMEOUT=10"
q() { $C psql -h "$1" -p 55433 -U cloud_admin -d postgres -Atc "$2" 2>&1; }
PS() { kubectl -n d0-neon exec cliente-overlay -- bash -c "exec 3<>/dev/tcp/pageserver.d0-neon.svc.cluster.local/9898; printf '$1 $2 HTTP/1.0\r\nHost: p\r\nContent-Type: application/json\r\nContent-Length: ${#3}\r\n\r\n$3' >&3; timeout 60 cat <&3" 2>/dev/null | tail -1; }
wal() { q $1 "select pg_current_wal_lsn()"; }
diff() { q $1 "select pg_size_pretty(pg_wal_lsn_diff('$3','$2'))"; }

echo "== preparar en main: 100 000 filas, VACUUM (hint bits puestos), y luego UPDATE de todas (filas nuevas SIN hint bits)"
q $OV "drop table if exists h; create table h(id int primary key, v text); insert into h select g, repeat('x',50) from generate_series(1,100000) g" >/dev/null
q $OV "vacuum h" >/dev/null
q $OV "update h set v = repeat('y',50)" >/dev/null
echo "   páginas de h: $(q $OV "select relpages from pg_class where relname='h'")  (tras analyze: $(q $OV "analyze h; select relpages from pg_class where relname='h'"))"
LSN=$(wal $OV); echo "   LSN del punto de rama: $LSN"; sleep 3

RAMA=$(python -c "import secrets;print(secrets.token_hex(16))")
PS POST /v1/tenant/$T/timeline/ "{\"new_timeline_id\":\"$RAMA\",\"ancestor_timeline_id\":\"$TL\",\"ancestor_start_lsn\":\"$LSN\",\"pg_version\":17}" >/dev/null
python - "$RAMA" <<'EOF'
import json,sys
d=json.load(open('C:/tmp/d0b/config.json'))
for x in d['spec']['cluster']['settings']:
    if x['name']=='neon.timeline_id': x['value']=sys.argv[1]
json.dump(d,open('C:/tmp/d0b/config-rama.json','w'),indent=1)
EOF
kubectl -n d0-neon delete neonvm pg-d0-rama --wait=true >/dev/null
kubectl -n d0-neon create configmap pg-d0-rama-config --from-file=config.json=$D/config-rama.json --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl apply -f $D/vm-rama.yaml >/dev/null
until OVR=$(kubectl -n d0-neon get neonvm pg-d0-rama -o jsonpath='{.status.extraNetIP}' 2>/dev/null); [ "$(q "$OVR" 'select 1' 2>/dev/null)" = 1 ]; do sleep 3; done
echo "$OVR" > $D/vm-rama-ov.txt

for x in "rama $OVR" "main $OV"; do
  set -- $x
  a=$(wal $2); q $2 "select count(*) from h" >/dev/null; b=$(wal $2); q $2 "select count(*) from h" >/dev/null; c=$(wal $2)
  echo "== $1: 1er recorrido → WAL $(diff $2 $a $b); 2º recorrido → WAL $(diff $2 $b $c)"
done
echo "== y con wal_log_hints=off en una sesión no se puede (es de servidor): $(q $OV 'show wal_log_hints') · checksums $(q $OV 'show data_checksums')"
