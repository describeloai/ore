#!/usr/bin/env bash
# PG2 · TIPOS, FILAS Y POOLER (ADR 0065, D-PG4/5/6), en Docker y sin cuentas. Lo que PG0 midió roto:
#
#   tipos   las 38 columnas de PG0 (PG17 + pgvector, hstore, citext, ltree, y PostGIS) se leen por
#           v1 y por Arrow; el tipo Arrow de cada una es el que el catálogo promete; `infinity` y
#           `NaN` caben con su aviso; los valores exactos salen como antes (UTC, `true`, `\x…`)
#   filas   `rowsEstimate`: por `n_live_tup` antes de ANALYZE, por `reltuples` después, la
#           particionada como suma de sus hojas, y nada para una vacía sin analizar
#   pooler  PgBouncer en modo transacción SIN prepared statements: 4 servir + 4 v1 a la vez,
#           todo verde (PG0: 14/20 y 7/12, `42P05`)
#
#   bash pruebas-de-fuego/pg2-tipos-filas-pooler.sh
#
# Necesita Docker. Contraseñas de laboratorio; los Postgres sin TLS (ORE_PG_LABORATORIO=1).
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
RUST_IMG=rust:1-bookworm
RED=ore-pg2
PW=lab

if [ "${1:-}" = "--dentro" ]; then
  cd /w
  command -v psql >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq postgresql-client python3-pip >/dev/null 2>&1; }
  python3 -c 'import pyarrow' 2>/dev/null || pip install -q --break-system-packages pyarrow >/dev/null 2>&1
  cargo build --quiet -p ore-read-postgres 2>&1 | tail -3
  D="${CARGO_TARGET_DIR:-/w/target}/debug/ore-read-postgres"
  export ORE_PG_LABORATORIO=1
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  for h in pgv pgis pg16; do for _ in $(seq 90); do pg_isready -q -h $h && break; sleep 1; done; done
  for h in pgb; do for _ in $(seq 60); do pg_isready -q -h $h -p 6432 && break; sleep 1; done; done
  url() { echo "postgres://postgres:$PW@$1:${2:-5432}/${3:-postgres}"; }
  sql() { PGPASSWORD=$PW psql -h "$1" -U postgres -d "${3:-postgres}" -XAtq -v ON_ERROR_STOP=1 -c "$2"; }

  echo "── tipos"
  PGPASSWORD=$PW psql -h pgv -U postgres -XAtq -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
create extension if not exists vector; create extension if not exists hstore;
create extension if not exists citext; create extension if not exists ltree;
create type par as (a int, b text);
create type animo as enum ('feliz','triste');
create table tipos (
  id int primary key,
  a_int int[], a_text text[], a_2d int[], a_num numeric[], a_bool bool[], a_ts timestamptz[],
  iv interval, ttz timetz, ip inet, cid cidr, mac macaddr, mac8 macaddr8,
  mon money, bt bit(4), vb varbit, r int4range, tr tstzrange, mr int4multirange,
  comp par, hs hstore, ci citext, lt ltree, tsv tsvector, vec vector(3),
  pt point, bx box, cir circle, x xml, ch "char", lsn pg_lsn, rc regclass,
  num numeric, n2 numeric(6,2), ts timestamp, tstz timestamptz, d date, en animo, j jsonb, by bytea,
  u uuid, f8 float8, b bool
);
insert into tipos values
 (1, '{1,2,NULL}', '{"a","b,c","x\"y"}', '{3,4}', '{1.5,NULL}', '{t,f}', '{"2026-10-16 10:00+02"}',
  '1 year 2 mons 3 days 04:05:06.789', '12:34:56+02', '192.168.1.5/24', '10.0.0.0/8',
  '08:00:2b:01:02:03', '08:00:2b:01:02:03:04:05', '12.34', B'1010', B'101', '[1,5)',
  '[2026-01-01,2026-02-01)', '{[1,3),[7,9)}', '(1,"x y")', 'a=>1,b=>NULL', 'MiXto',
  'top.ciencia.astro', 'el gato come', '[1,2.5,3]', '(1,2)', '((0,0),(1,1))', '<(0,0),2>',
  '<a>1</a>', 'z', '16/B374D848', 'tipos', 1.25, 12.5, '2026-10-16 10:00', '2026-10-16 10:00+02',
  '2026-10-16', 'feliz', '{"k":1}', '\xdeadbeef', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', 0.1, true),
 (2, '{}', '{}', NULL, NULL, NULL, NULL, '-1 day', NULL, '::1', NULL, NULL, NULL, '-0.01', NULL, NULL,
  'empty', '(,)', '{}', NULL, '', '', '', '', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
  'NaN', 'NaN', 'infinity', '-infinity', 'infinity', NULL, 'null', '\x', NULL, 'Infinity', false),
 (3, NULL, NULL, '{{1,2},{3,4}}', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
  NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
  NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
SQL
  PGPASSWORD=$PW psql -h pgis -U postgres -XAtq -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
create extension if not exists postgis;
create table geo (id int primary key, g geometry(Point,4326), gg geography, gpol geometry);
insert into geo values (1, 'SRID=4326;POINT(-8.4 43.37)', 'POINT(-3.7 40.4)', 'POLYGON((0 0,1 0,1 1,0 0))'), (2, NULL, NULL, NULL);
SQL
  printf '%s' "$(url pgv)" | "$D" catalogo lab 2>/dev/null >/tmp/cat-pgv.json
  printf '%s' "$(url pgis)" | "$D" catalogo lab 2>/dev/null >/tmp/cat-pgis.json
  # Columna a columna: v1 (filas 1 y 2) y Arrow (filas 1 y 2; la 3 sólo para a_2d).
  : >/tmp/tipos.txt
  for h_t in pgv:tipos pgis:geo; do
    h=${h_t%%:*}; t=${h_t#*:}
    cols=$(sql $h "select string_agg(attname, ' ' order by attnum) from pg_attribute where attrelid='$t'::regclass and attnum>1 and not attisdropped")
    for c in $cols; do
      v1=$(printf '{"url":"%s","objeto":"public.%s","proyeccion":{"c":"%s"},"claveColumnas":["id"],"claves":[["1"],["2"]],"filtros":[]}' "$(url $h)" "$t" "$c" | "$D" leer lab 2>&1 | tr '\n' ' ')
      printf '{"url":"%s","objeto":"public.%s","proyeccion":{"c":"%s"},"formato":"arrow","filtros":[{"columna":"id","operador":"in","valor":["1","2"]}]}' "$(url $h)" "$t" "$c" | "$D" leer >/tmp/a-$t-$c.bin 2>/tmp/a-$t-$c.err
      echo "$h $t $c $? $v1" >>/tmp/tipos.txt
    done
  done
  python3 - <<'PY'
import json, pyarrow.ipc as ipc
cat = {}
for f in ("/tmp/cat-pgv.json", "/tmp/cat-pgis.json"):
    s = open(f).read(); s = s[s.index("{"):]
    for t in json.loads(s)["tables"]:
        for c in t["columns"]:
            cat[(t["name"].split(".")[-1], c["name"])] = c.get("type")
def esperado(oos):
    if oos is None or oos in ("Opaque", "Decimal") or oos == "String": return "string"
    if oos.startswith("list<"): return "list<item: %s>" % esperado(oos[5:-1])
    if oos.startswith("Decimal<"):
        p, s = oos[8:-1].split(","); return "decimal128(%s, %s)" % (p.strip(), s.strip())
    return {"Integer": "int64", "Float": "double", "Boolean": "bool", "Date": "date32[day]",
            "Time": "time64[us]", "DateTime": "timestamp[us]", "DateTimeTz": "timestamp[us, tz=+00:00]"}[oos]
mal = 0; filas = []
for linea in open("/tmp/tipos.txt"):
    h, t, c, rc, v1 = linea.rstrip("\n").split(" ", 4)
    err = open(f"/tmp/a-{t}-{c}.err").read().strip().replace("\n", " ")
    oos = cat.get((t, c))
    if rc != "0":
        print(f"  ✗ {t}.{c}: Arrow rc={rc} {err[:160]}"); mal += 1; continue
    tab = ipc.open_stream(open(f"/tmp/a-{t}-{c}.bin", "rb")).read_all()
    tipo = str(tab.schema.field(0).type)
    if tipo != esperado(oos):
        print(f"  ✗ {t}.{c}: el catálogo dice {oos} y Arrow da {tipo}"); mal += 1
    if "ore-read-postgres:" in v1 and "aviso" not in v1:
        print(f"  ✗ {t}.{c}: v1 falla: {v1[:160]}"); mal += 1
    filas.append((t, c, oos, tipo, tab.column(0).to_pylist(), v1[:90], err[:110]))
for f in filas:
    print("    %-10s %-6s %-16s %-28s %s · v1 %s %s" % (f[0], f[1], f[2], f[3], str(f[4])[:70], f[5], ("· " + f[6]) if f[6] else ""))
print("  %s tipos: %d columnas, %d mal" % ("✓" if mal == 0 else "✗", len(filas) + mal, mal))
open("/tmp/tipos.mal", "w").write(str(mal))
PY
  fallos=$((fallos + $(cat /tmp/tipos.mal)))
  # a_2d en la fila 3: un array de dos dimensiones no cabe en una lista, y se dice.
  r=$(printf '{"url":"%s","objeto":"public.tipos","proyeccion":{"c":"a_2d"},"formato":"arrow"}' "$(url pgv)" | "$D" leer 2>&1 >/dev/null)
  echo "$r" | grep -q "dimensión" && echo "$r" | grep -q "a_2d" && dice "a_2d de dos dimensiones: se niega nombrando la columna" || falla "a_2d: $r"
  r=$(printf '{"url":"%s","objeto":"public.tipos","proyeccion":{"b":"b","tstz":"tstz","f8":"f8","by":"by","n2":"n2"},"claveColumnas":["id"],"claves":[["1"]],"filtros":[]}' "$(url pgv)" | "$D" leer lab 2>&1)
  [ "$r" = '{"b":"true","by":"\\xdeadbeef","f8":"0.1","n2":"12.50","tstz":"2026-10-16 08:00:00+00"}' ] && dice "v1 escribe como antes: $r" || falla "v1: $r"

  echo "── filas"
  sql pg16 "create database filas" >/dev/null
  sql pg16 "create table t1 as select g id from generate_series(1,210) g; create table vacia (id int);
            create table parte (id int, d date) partition by range (d);
            create table parte_1 partition of parte for values from ('2026-01-01') to ('2026-07-01');
            create table parte_2 partition of parte for values from ('2026-07-01') to ('2027-01-01');
            insert into parte select g, date '2026-01-01' + (g % 365) from generate_series(1, 4000) g;" filas >/dev/null
  sleep 1
  est() { printf '%s' "$(url pg16 5432 filas)" | "$D" catalogo lab 2>/dev/null | python3 -c '
import json,sys; s=sys.stdin.read(); c=json.loads(s[s.index("{"):])
print(" ".join("%s=%s" % (t["name"].split(".")[1], t.get("rowsEstimate","-")) for t in c["tables"]))'; }
  antes=$(est); echo "    antes de ANALYZE: $antes · reltuples: $(sql pg16 "select string_agg(relname||'='||reltuples, ' ' order by relname) from pg_class where relname in ('t1','parte_1','vacia')" filas)"
  echo "$antes" | grep -q "t1=210" && echo "$antes" | grep -q "parte=4000" && echo "$antes" | grep -q "vacia=-" \
    && dice "antes de ANALYZE: por n_live_tup (t1=210, parte=4000, vacia sin estimación)" || falla "antes: $antes"
  sql pg16 "analyze" filas
  despues=$(est); echo "    después: $despues"
  echo "$despues" | grep -q "t1=210" && echo "$despues" | grep -q "parte=4000" && echo "$despues" | grep -q "vacia=0" \
    && dice "después de ANALYZE: por reltuples (vacia=0, ya es un hecho del planificador)" || falla "después: $despues"
  printf '%s' "$(url pg16 5432 filas)" | "$D" catalogo lab 2>/dev/null | grep -q '"rows"' && falla "el catálogo emite rows" || dice "rows no se emite nunca"

  echo "── pooler (modo transacción, sin prepared statements)"
  sql pg16 "create table pool_t as select g id, md5(g::text) t from generate_series(1,50000) g; alter table pool_t add primary key (id)" >/dev/null
  U=$(url pgb 6432)
  for k in 1 2 3 4; do
    ( for n in 1 2 3 4 5; do
        printf '{"id":"c%s-%s","url":"%s","objeto":"public.pool_t","proyeccion":{"id":"id","t":"t"},"formato":"arrow","limit":%s}\n' $k $n "$U" $((2000 * n)); sleep 0.3
      done | timeout 90 "$D" servir >/tmp/c$k.bin 2>/tmp/c$k.err ) &
    ( for n in 1 2 3; do printf '{"url":"%s","objeto":"public.pool_t","proyeccion":{"t":"t"},"claveColumnas":["id"],"claves":[["%s"]],"filtros":[]}' "$U" $((k * 10 + n)) | timeout 30 "$D" leer lab >>/tmp/v$k.out 2>>/tmp/v$k.err; done ) &
  done
  wait
  ok=$(cat /tmp/c?.bin | grep -a -c '"fin":"ok"'); v1=$(cat /tmp/v?.out | grep -c '"t"')
  [ "$ok" = 20 ] && [ "$v1" = 12 ] && dice "4 servir + 4 v1 a la vez: $ok/20 y $v1/12" \
    || falla "pooler: $ok/20 y $v1/12 · $(cat /tmp/c?.bin | grep -a -o '"mensaje":"[^"]*"' | sort | uniq -c) $(cat /tmp/v?.err | sort | uniq -c)"
  printf '{"url":"%s","objeto":"public.pool_t","proyeccion":{"id":"id"},"formato":"arrow"}' "$U" | "$D" estimar 2>&1 | grep -q '"filas":50000' && dice "estimar por el pooler" || falla "estimar"

  echo
  [ $fallos = 0 ] && echo "PG2: todo verde" || echo "PG2: $fallos fallos"
  exit $fallos
fi

command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
limpiar() { docker rm -f pgv pgis pg16 pgb >/dev/null 2>&1; docker network rm $RED >/dev/null 2>&1; }
limpiar
docker network create $RED >/dev/null
docker run -d --name pgv --network $RED -e POSTGRES_PASSWORD=$PW pgvector/pgvector:pg17 >/dev/null || exit 1
docker run -d --name pgis --network $RED -e POSTGRES_PASSWORD=$PW postgis/postgis:17-3.5-alpine >/dev/null || exit 1
docker run -d --name pg16 --network $RED -e POSTGRES_PASSWORD=$PW postgres:16-alpine >/dev/null || exit 1
docker run -d --name pgb --network $RED alpine:3.20 sh -c "
  apk add -q pgbouncer >/dev/null
  printf '[databases]\n* = host=pg16 port=5432 user=postgres password=$PW\n[pgbouncer]\nlisten_addr=0.0.0.0\nlisten_port=6432\nauth_type=trust\nauth_file=/etc/pgbouncer/u.txt\npool_mode=transaction\nmax_prepared_statements=0\ndefault_pool_size=1\nignore_startup_parameters=extra_float_digits,application_name,options\n' > /etc/pgbouncer/b.ini
  printf '\"postgres\" \"$PW\"\n' > /etc/pgbouncer/u.txt
  adduser -D pgb && chown -R pgb /etc/pgbouncer
  exec su -s /bin/sh pgb -c 'pgbouncer /etc/pgbouncer/b.ini'" >/dev/null || exit 1
docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main -e PW=$PW \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/pg2-tipos-filas-pooler.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
rc=${PIPESTATUS[0]}
limpiar
exit $rc
