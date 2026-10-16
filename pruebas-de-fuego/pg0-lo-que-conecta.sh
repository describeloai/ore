#!/usr/bin/env bash
# PG0 · LO QUE CONECTA HOY (ADR 0065), en Docker, sin cuentas y SIN CAMBIAR EL DRIVER: mide el
# `ore-read-postgres` de main tal cual contra lo que un cliente tiene de verdad.
#
#   tls      PG 13–17 con una CA propia, el mismo servidor por un nombre que su certificado no
#            lleva (pg-mal) y un Postgres sin TLS (pg-plano); con cada `sslmode` (ninguno,
#            disable, prefer, require, verify-ca, verify-full); antes y después de poner la CA en
#            el almacén del sistema del contenedor del driver
#   pooler   pgbouncer en modo transacción y en modo sesión: check, catalogo, leer (v1 y Arrow),
#            estimar y servir con tres peticiones seguidas
#   tipos    una tabla con los tipos de §5 de la investigación (pgvector, hstore, citext, ltree) y
#            otra de PostGIS, columna a columna por `leer` v1 y Arrow, y lo que dice el catálogo
#   filas    300 tablas y una particionada: reltuples/relpages antes y después de ANALYZE, y lo que
#            tarda pg_total_relation_size frente a relpages
#   replica  una primaria y su réplica: una lectura larga en la réplica contra un VACUUM en la
#            primaria (max_standby_streaming_delay=1s); qué error da el driver; el testigo
#
#   bash pruebas-de-fuego/pg0-lo-que-conecta.sh [fase…]     (sin fases: todas)
#
# Necesita Docker. Las contraseñas son de laboratorio. Deja el informe en el volumen pg0-trabajo
# (/trabajo/informe.txt) y lo imprime.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
RUST_IMG=rust:1-bookworm
RED=ore-pg0
PW=lab

# ── dentro: el contenedor de Rust, en la red del laboratorio ─────────────────
if [ "${1:-}" = "--dentro" ]; then
  shift
  cd /w
  command -v psql >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq postgresql-client python3-pip >/dev/null 2>&1; }
  python3 -c 'import pyarrow' 2>/dev/null || pip install -q --break-system-packages pyarrow >/dev/null 2>&1
  cargo build --quiet -p ore-read-postgres 2>&1 | grep -v '^\s*$' | tail -3
  D="${CARGO_TARGET_DIR:-/w/target}/debug/ore-read-postgres"
  [ -x "$D" ] || { echo "no se construyó el driver"; exit 1; }
  I=/trabajo/informe.txt
  linea() { printf '%s\n' "$*" | tee -a "$I"; }
  corto() { tr '\n' ' ' | sed 's/  */ /g' | cut -c1-"${1:-220}"; }
  url() { echo "postgres://postgres:$PW@$1:${3:-5432}/${4:-postgres}${2:+?sslmode=$2}"; }
  check() { printf '{"url":"%s"}' "$1" | timeout 30 "$D" check 2>&1 | corto; }
  catalogo() { printf '%s' "$1" | timeout 60 "$D" catalogo lab 2>&1; }
  espera() { for h in "$@"; do for _ in $(seq 60); do pg_isready -q -h "$h" -p "${PUERTO:-5432}" && break; sleep 1; done; done; }

  for fase in "$@"; do
  linea ""
  linea "════ $fase ════"
  case $fase in

  tls)
    espera pg13 pg14 pg15 pg16 pg17 pg-plano
    for almacen in sin-ca con-ca; do
      if [ $almacen = con-ca ]; then
        cp /certs/ca.crt /usr/local/share/ca-certificates/ore-lab.crt && update-ca-certificates >/dev/null 2>&1
      fi
      for h in pg13 pg14 pg15 pg16 pg17 pg-mal pg-plano; do
        for m in "" disable prefer require verify-ca verify-full; do
          r=$(check "$(url $h "$m")")
          linea "tls · $almacen · $h · sslmode=${m:-∅} · $r"
        done
      done
    done
    # La bajada a texto plano, vista desde el servidor: prefer contra un servidor sin TLS.
    linea "tls · catalogo contra pg-plano sin sslmode: $(catalogo "$(url pg-plano)" | head -c 120 | corto)"
    ;;

  pooler)
    PUERTO=6432 espera pgb-t pgb-t0 pgb-s
    PGPASSWORD=$PW psql -h pg16 -U postgres -XAtq -c "create table if not exists pool_t as select g id, md5(g::text) t from generate_series(1,50000) g; alter table pool_t drop constraint if exists pool_t_pk; alter table pool_t add constraint pool_t_pk primary key (id)" >/dev/null
    for p in pgb-t pgb-t0 pgb-s; do
      U=$(url $p "" 6432)
      linea "pooler · $p · check · $(check "$U")"
      c=$(catalogo "$U"); linea "pooler · $p · catalogo · rc=$? · $(echo "$c" | grep -c '"nombre"') nombres · $(echo "$c" | head -c 160 | corto 160)"
      v1=$(printf '{"url":"%s","objeto":"public.pool_t","proyeccion":{"t":"t"},"claveColumnas":["id"],"claves":[["7"]],"filtros":[]}' "$U" | timeout 60 "$D" leer lab 2>&1)
      linea "pooler · $p · leer v1 · $(echo "$v1" | corto 160)"
      for n in 1 2; do
        printf '{"url":"%s","objeto":"public.pool_t","proyeccion":{"id":"id","t":"t"},"formato":"arrow"}' "$U" | timeout 60 "$D" leer >/trabajo/a.bin 2>/trabajo/a.err
        linea "pooler · $p · leer arrow #$n · rc=$? · $(stat -c %s /trabajo/a.bin) bytes · $(corto 200 </trabajo/a.err)"
      done
      e=$(printf '{"url":"%s","objeto":"public.pool_t","proyeccion":{"id":"id"},"formato":"arrow"}' "$U" | timeout 60 "$D" estimar 2>&1)
      linea "pooler · $p · estimar · $(echo "$e" | corto 160)"
      for n in 1 2 3; do
        printf '{"id":"r%s","url":"%s","objeto":"public.pool_t","proyeccion":{"id":"id","t":"t"},"formato":"arrow","limit":1000}\n' $n "$U"
        sleep 1
      done | timeout 60 "$D" servir >/trabajo/s.bin 2>/trabajo/s.err
      linea "pooler · $p · servir ×3 · rc=$? · ok=$(grep -a -c '"fin":"ok"' /trabajo/s.bin) · errores: $(grep -a -o '"mensaje":"[^"]*"' /trabajo/s.bin | sort | uniq -c | corto 300) $(corto 200 </trabajo/s.err)"
      # Lo de verdad: varios lectores a la vez sobre UNA conexión del servidor (default_pool_size=1).
      for k in 1 2 3 4; do
        ( for n in 1 2 3 4 5; do
            printf '{"id":"c%s-%s","url":"%s","objeto":"public.pool_t","proyeccion":{"id":"id","t":"t"},"formato":"arrow","limit":%s}\n' $k $n "$U" $((2000 * n))
            sleep 0.3
          done | timeout 90 "$D" servir >/trabajo/c$k.bin 2>/trabajo/c$k.err ) &
        ( for n in 1 2 3; do printf '{"url":"%s","objeto":"public.pool_t","proyeccion":{"t":"t"},"claveColumnas":["id"],"claves":[["%s"]],"filtros":[]}' "$U" $((k * 10 + n)) | timeout 30 "$D" leer lab >/dev/null 2>>/trabajo/v$k.err; done ) &
      done
      wait
      linea "pooler · $p · 4 servir + 4 leer v1 a la vez · ok=$(cat /trabajo/c?.bin | grep -a -c '"fin":"ok"')/20 · errores: $(cat /trabajo/c?.bin | grep -a -o '"mensaje":"[^"]*"' | sort | uniq -c | corto 300) · v1: $(cat /trabajo/v?.err 2>/dev/null | sort | uniq -c | corto 300)"
      rm -f /trabajo/c?.bin /trabajo/c?.err /trabajo/v?.err
    done
    ;;

  tipos)
    espera pgv pgis
    PGPASSWORD=$PW psql -h pgv -U postgres -XAtq -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
create extension if not exists vector; create extension if not exists hstore;
create extension if not exists citext; create extension if not exists ltree;
drop table if exists tipos; drop type if exists par;
create type par as (a int, b text);
create type animo as enum ('feliz','triste');
create table tipos (
  id int primary key,
  a_int int[], a_text text[], a_2d int[][], a_num numeric[],
  iv interval, ttz timetz, ip inet, cid cidr, mac macaddr, mac8 macaddr8,
  mon money, bt bit(4), vb varbit, r int4range, tr tstzrange, mr int4multirange,
  comp par, hs hstore, ci citext, lt ltree, tsv tsvector, vec vector(3),
  pt point, bx box, cir circle, x xml, ch "char", lsn pg_lsn, rc regclass,
  num numeric, ts timestamp, tstz timestamptz, d date, en animo, j jsonb, by bytea, u uuid, f8 float8
);
insert into tipos values
 (1, '{1,2,NULL}', '{"a","b,c"}', '{{1,2},{3,4}}', '{1.5,NULL}',
  '1 year 2 mons 3 days 04:05:06.789', '12:34:56+02', '192.168.1.5/24', '10.0.0.0/8',
  '08:00:2b:01:02:03', '08:00:2b:01:02:03:04:05', '12.34', B'1010', B'101', '[1,5)',
  '[2026-01-01,2026-02-01)', '{[1,3),[7,9)}', '(1,"x y")', 'a=>1,b=>NULL', 'MiXto',
  'top.ciencia.astro', 'el gato come', '[1,2.5,3]', '(1,2)', '((0,0),(1,1))', '<(0,0),2>',
  '<a>1</a>', 'z', '16/B374D848', 'tipos', 1.25, '2026-10-16 10:00', '2026-10-16 10:00+02',
  '2026-10-16', 'feliz', '{"k":1}', '\xdeadbeef', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', 1.5),
 (2, '{}', '{}', NULL, NULL, '-1 day', NULL, '::1', NULL, NULL, NULL, '-0.01', NULL, NULL,
  'empty', '(,)', '{}', NULL, '', '', '', '', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
  'NaN', 'infinity', '-infinity', 'infinity', NULL, 'null', '\x', NULL, 'Infinity');
SQL
    linea "tipos · tabla creada en pgv (PG17 + pgvector) · $(PGPASSWORD=$PW psql -h pgv -U postgres -XAtq -c 'select count(*) from tipos') filas"
    PGPASSWORD=$PW psql -h pgis -U postgres -XAtq -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
create extension if not exists postgis;
drop table if exists geo;
create table geo (id int primary key, g geometry(Point,4326), gg geography, gpol geometry);
insert into geo values (1, 'SRID=4326;POINT(-8.4 43.37)', 'POINT(-3.7 40.4)', 'POLYGON((0 0,1 0,1 1,0 0))'), (2, NULL, NULL, NULL);
SQL
    linea "tipos · tabla geo creada en pgis (PostGIS)"
    catalogo "$(url pgv)" >/trabajo/catalogo-tipos.json 2>/trabajo/catalogo-tipos.err
    linea "tipos · catalogo pgv · rc=$? · $(corto 200 </trabajo/catalogo-tipos.err)"
    catalogo "$(url pgis)" >/trabajo/catalogo-geo.json 2>/trabajo/catalogo-geo.err
    linea "tipos · catalogo pgis · rc=$? · $(corto 200 </trabajo/catalogo-geo.err)"
    col() { # HOST TABLA COLUMNA
      local U; U=$(url "$1")
      v1=$(printf '{"url":"%s","objeto":"public.%s","proyeccion":{"c":"%s"},"claveColumnas":["id"],"claves":[["1"],["2"]],"filtros":[]}' "$U" "$2" "$3" | timeout 30 "$D" leer lab 2>&1 | corto 150)
      printf '{"url":"%s","objeto":"public.%s","proyeccion":{"c":"%s"},"formato":"arrow"}' "$U" "$2" "$3" | timeout 30 "$D" leer >/trabajo/t.bin 2>/trabajo/t.err
      rc=$?
      tipo=$(python3 - "$3" 2>/dev/null <<'PY' </trabajo/t.bin
import sys
try:
    import pyarrow.ipc as ipc
    t = ipc.open_stream(sys.stdin.buffer).read_all()
    print(t.schema.field(0).type, "|", t.column(0).to_pylist())
except Exception as e:
    print("sin-pyarrow" if isinstance(e, ImportError) else f"arrow-ilegible: {e}")
PY
)
      linea "tipos · $2.$3 · v1: $v1 · arrow: rc=$rc $(stat -c %s /trabajo/t.bin)B ${tipo:-} $(grep -v '^$' /trabajo/t.err | corto 150)"
    }
    for c in a_int a_text a_2d a_num iv ttz ip cid mac mac8 mon bt vb r tr mr comp hs ci lt tsv vec pt bx cir x ch lsn rc num ts tstz d en j by u f8; do
      col pgv tipos $c
    done
    for c in g gg gpol; do col pgis geo $c; done
    ;;

  filas)
    espera pg17
    PGPASSWORD=$PW psql -h pg17 -U postgres -XAtq -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
create database filas;
SQL
    PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
do $$ begin
  for i in 1..300 loop
    execute format('create table t%s (id int primary key, v text)', i);
    execute format('insert into t%s select g, md5(g::text) from generate_series(1, %s) g', i, 200 + i * 10);
  end loop;
end $$;
create table parte (id int, d date, v text) partition by range (d);
create table parte_1 partition of parte for values from ('2026-01-01') to ('2026-04-01');
create table parte_2 partition of parte for values from ('2026-04-01') to ('2026-07-01');
create table parte_3 partition of parte for values from ('2026-07-01') to ('2026-10-01');
create table parte_4 partition of parte for values from ('2026-10-01') to ('2027-01-01');
insert into parte select g, date '2026-01-01' + (g % 365), md5(g::text) from generate_series(1, 40000) g;
create table vacia (id int);
SQL
    q="select c.relname, c.relkind, c.reltuples, c.relpages from pg_class c where c.relname in ('t1','t300','parte','parte_1','vacia') order by 1"
    linea "filas · antes de ANALYZE (autovacuum puede haber pasado): $(PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -F, -c "$q" | corto 400)"
    for n in 1 2 3; do
      s=$(date +%s%N); catalogo "$(url pg17 disable 5432 filas)" 2>/dev/null | sed -n '/^{/,$p' >/trabajo/cat-filas.json; e=$(( ($(date +%s%N) - s) / 1000000 ))
      linea "filas · catalogo #$n · ${e} ms · $(grep -c '"kind"' /trabajo/cat-filas.json) relaciones · rows=$(grep -c '"rows"' /trabajo/cat-filas.json) rowsEstimate=$(grep -c '"rowsEstimate"' /trabajo/cat-filas.json)"
    done
    PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -c "analyze" >/dev/null
    linea "filas · después de ANALYZE: $(PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -F, -c "$q" | corto 400)"
    linea "filas · la verdad: t1=$(PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -c 'select count(*) from t1') t300=$(PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -c 'select count(*) from t300') parte=$(PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -c 'select count(*) from parte')"
    linea "filas · el padre como suma de hijas: $(PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -c "select sum(c.reltuples) from pg_inherits i join pg_class c on c.oid=i.inhrelid where i.inhparent='parte'::regclass")"
    for sql in \
      "select count(*), sum(greatest(reltuples,0)), sum(relpages::bigint*current_setting('block_size')::int) from pg_class c join pg_namespace n on n.oid=c.relnamespace where relkind in ('r','p','m') and nspname='public'" \
      "select count(*), sum(pg_total_relation_size(c.oid)) from pg_class c join pg_namespace n on n.oid=c.relnamespace where relkind in ('r','p','m') and nspname='public'"; do
      t=$(PGPASSWORD=$PW psql -h pg17 -U postgres -d filas -XAtq -c '\timing on' -c "$sql" | tr '\n' ' ')
      linea "filas · $(echo "$sql" | cut -c1-60)… · $t"
    done
    ;;

  replica)
    espera prim rep
    PGPASSWORD=$PW psql -h prim -U postgres -XAtq -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
drop table if exists grande;
create table grande (id int primary key, v text);
insert into grande select g, repeat(md5(g::text), 4) from generate_series(1, 2000000) g;
SQL
    for _ in $(seq 60); do
      n=$(PGPASSWORD=$PW psql -h rep -U postgres -XAtq -c "select count(*) from grande" 2>/dev/null) && [ "$n" = 2000000 ] && break; sleep 2
    done
    linea "replica · en la réplica: $n filas · pg_is_in_recovery=$(PGPASSWORD=$PW psql -h rep -U postgres -XAtq -c 'select pg_is_in_recovery()')"
    linea "replica · testigo primaria · $(printf '{"url":"%s","objeto":"public.grande"}' "$(url prim)" | "$D" testigo lab 2>&1 | corto 160)"
    linea "replica · testigo réplica · $(printf '{"url":"%s","objeto":"public.grande"}' "$(url rep)" | "$D" testigo lab 2>&1 | corto 160)"
    # La lectura larga: quien lee va despacio (el tubo se llena y el portal se queda abierto).
    ( printf '{"url":"%s","objeto":"public.grande","proyeccion":{"id":"id","v":"v"},"formato":"arrow"}' "$(url rep)" \
        | "$D" leer 2>/trabajo/r.err | (sleep 20; cat >/dev/null); echo "${PIPESTATUS[1]}" >/trabajo/r.rc ) &
    sleep 4
    PGPASSWORD=$PW psql -h prim -U postgres -XAtq -c "delete from grande where id % 2 = 0" -c "vacuum grande" >/dev/null
    wait
    linea "replica · lectura larga contra VACUUM · rc=$(cat /trabajo/r.rc) · $(corto 400 </trabajo/r.err)"
    ;;
  esac
  done
  echo
  echo "informe: volumen pg0-trabajo, informe.txt"
  exit 0
fi

# ── fuera: los servicios ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
FASES="${*:-tls pooler tipos filas replica}"
limpiar() {
  docker rm -f pg13 pg14 pg15 pg16 pg17 pg-plano pgb-t pgb-t0 pgb-s pgv pgis prim rep >/dev/null 2>&1
  docker volume rm -f pg0-certs >/dev/null 2>&1
  docker network rm $RED >/dev/null 2>&1
}
limpiar
docker volume rm -f pg0-trabajo >/dev/null 2>&1
docker network create $RED >/dev/null
docker volume create pg0-certs >/dev/null

# La CA del laboratorio y un certificado de servidor para pg13…pg17 (pg-mal NO está en él).
docker run --rm -v pg0-certs:/c alpine:3.20 sh -c '
  apk add -q openssl >/dev/null
  cd /c
  openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.crt -days 2 -subj "/CN=ORE PG0 CA" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -keyout srv.key -out srv.csr -subj "/CN=pg17" 2>/dev/null
  printf "subjectAltName=DNS:pg13,DNS:pg14,DNS:pg15,DNS:pg16,DNS:pg17\n" > ext
  openssl x509 -req -in srv.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out srv.crt -days 2 -extfile ext 2>/dev/null
  chmod 644 ca.crt srv.crt srv.key' || exit 1

# Postgres con TLS: la clave se copia fuera del volumen, del usuario postgres y 0600.
con_tls='cp /c/srv.crt /c/srv.key /var/lib/postgresql/ && chown postgres /var/lib/postgresql/srv.* && chmod 600 /var/lib/postgresql/srv.key && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/var/lib/postgresql/srv.crt -c ssl_key_file=/var/lib/postgresql/srv.key -c log_connections=on'
case " $FASES " in *" tls "*|*" pooler "*|*" filas "*)
  for v in 13 14 15 16 17; do
    extra=""; [ $v = 17 ] && extra="--network-alias pg-mal"
    docker run -d --name pg$v --network $RED $extra -e POSTGRES_PASSWORD=$PW -v pg0-certs:/c:ro \
      --entrypoint sh postgres:$v-alpine -c "$con_tls" >/dev/null || exit 1
  done ;;
esac
case " $FASES " in *" tls "*)
  docker run -d --name pg-plano --network $RED -e POSTGRES_PASSWORD=$PW postgres:16-alpine -c log_connections=on >/dev/null ;;
esac
case " $FASES " in *" pooler "*)
  for modo in t t0 s; do
    case $modo in t*) pm=transaction;; *) pm=session;; esac; mps=200; [ $modo = t0 ] && mps=0
    docker run -d --name pgb-$modo --network $RED alpine:3.20 sh -c "
      apk add -q pgbouncer >/dev/null
      printf '[databases]\n* = host=pg16 port=5432 user=postgres password=$PW\n[pgbouncer]\nlisten_addr=0.0.0.0\nlisten_port=6432\nauth_type=trust\nauth_file=/etc/pgbouncer/u.txt\npool_mode=$pm\nmax_prepared_statements=$mps\ndefault_pool_size=1\nignore_startup_parameters=extra_float_digits,application_name,options\n' > /etc/pgbouncer/b.ini
      printf '\"postgres\" \"$PW\"\n' > /etc/pgbouncer/u.txt
      adduser -D pgb && chown -R pgb /etc/pgbouncer
      exec su -s /bin/sh pgb -c 'pgbouncer /etc/pgbouncer/b.ini'" >/dev/null || exit 1
  done ;;
esac
case " $FASES " in *" tipos "*)
  docker run -d --name pgv --network $RED -e POSTGRES_PASSWORD=$PW pgvector/pgvector:pg17 >/dev/null || exit 1
  docker run -d --name pgis --network $RED -e POSTGRES_PASSWORD=$PW postgis/postgis:17-3.5-alpine >/dev/null || exit 1 ;;
esac
case " $FASES " in *" replica "*)
  docker run -d --name prim --network $RED -e POSTGRES_PASSWORD=$PW --entrypoint sh postgres:16-alpine -c '
    printf "#!/bin/sh\necho \"host replication all all scram-sha-256\" >> \"\$PGDATA/pg_hba.conf\"\n" > /docker-entrypoint-initdb.d/rep.sh
    exec docker-entrypoint.sh postgres -c wal_level=replica -c max_wal_senders=4' >/dev/null || exit 1
  docker run -d --name rep --network $RED -e PGPASSWORD=$PW --entrypoint sh postgres:16-alpine -c '
    until pg_isready -q -h prim; do sleep 1; done; sleep 3
    D=/var/lib/postgresql/data; rm -rf $D/*
    until pg_basebackup -h prim -U postgres -D $D -R -X stream; do sleep 2; rm -rf $D/*; done
    chown -R postgres $D; chmod 700 $D
    exec su-exec postgres postgres -D $D -c hot_standby=on -c max_standby_streaming_delay=1s' >/dev/null || exit 1 ;;
esac

docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" -v pg0-trabajo:/trabajo \
  -v pg0-certs:/certs:ro -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt \
  -e CARGO_TARGET_DIR=/tt/main -e PW=$PW -e ORE_PG_LABORATORIO=1 -w /w "$RUST_IMG" \
  bash pruebas-de-fuego/pg0-lo-que-conecta.sh --dentro $FASES 2>&1 | grep -v '^info:\|^warn:'
# Lo que vio el servidor: cuántas sesiones llegaron con TLS y cuántas sin él.
for h in pg17 pg-plano; do
  docker logs $h 2>&1 | grep 'connection authorized' >"$TEMP/pg0-$h.log" 2>/dev/null || continue
  echo "servidor $h · autorizadas con TLS: $(grep -c 'SSL enabled' "$TEMP/pg0-$h.log") · sin TLS: $(grep -vc 'SSL enabled' "$TEMP/pg0-$h.log")"
done
[ "${PG0_DEJAR:-}" = 1 ] || limpiar
