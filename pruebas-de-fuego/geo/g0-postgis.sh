# g0-postgis.sh (ADR 0067 G0·2): la fuente PostGIS del banco geo, DENTRO de la celda.
# Con la credencial del agente: la URL de Neon sale del cofre (la fuente $FUENTE) y no toca el disco
# ni la salida. Crea la base `banco_geo`, activa PostGIS, carga las tablas desde gs://…/carga/ (hex
# WKB: municipios en EPSG:25830, lugares en 4326 y en geography, carreteras en 4326), con índice GiST,
# y da de alta la fuente `banco_geo_pg` con POST /fuentes. Imprime «## ok» o «## falla».
set -e
B=project-8853a180-450d-47be-b83-banco-geo
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "http://ore-cofre.$NS.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos/fuente-$FUENTE")
[ "$C" = 200 ] || { echo "## falla: el custodio contesto $C"; exit 1; }
U="$(python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])')"; rm -f /tmp/s.json
# La misma URL, con la base cambiada a banco_geo.
G="$(U="$U" python3 -c 'import os,urllib.parse as p;u=p.urlsplit(os.environ["U"]);print(p.urlunsplit(u._replace(path="/banco_geo")))')"
export PGCONNECT_TIMEOUT=15
echo "# la base"
psql "$U" -XAtq -c "select 1 from pg_database where datname='banco_geo'" | grep -q 1 || psql "$U" -XAtq -v ON_ERROR_STOP=1 -c "create database banco_geo"
echo "# los ficheros de carga (cuenta: $(curl -s -H 'Metadata-Flavor: Google' http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/email))"
GT=$(curl -sSf -H 'Metadata-Flavor: Google' http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
for f in pg_municipios pg_lugares pg_carreteras; do
  C=$(curl -s -o /tmp/$f.csv -w '%{http_code}' -H "authorization: Bearer $GT" "https://storage.googleapis.com/storage/v1/b/$B/o/carga%2F$f.csv?alt=media")
  [ "$C" = 200 ] || { echo "## falla: GCS contesto $C para $f: $(head -c 200 /tmp/$f.csv)"; exit 1; }
  # Neon gratuito tiene 0,5 GB por proyecto: las carreteras, 300 000 tramos (el resto está en el bucket).
  [ $f = pg_carreteras ] && head -300001 /tmp/$f.csv > /tmp/x.csv && mv /tmp/x.csv /tmp/$f.csv
  echo "  $f: $(wc -l < /tmp/$f.csv) líneas"
done
echo "# las tablas"
T0=$(date +%s)
psql "$G" -XAq -v ON_ERROR_STOP=1 <<'SQL'
create extension if not exists postgis;
drop table if exists municipios, lugares, lugares_geog, carreteras, _m, _l, _c;
create temp table _m (id text, nombre text, tipo text, g text);
create temp table _l (id text, nombre text, categoria text, g text);
create temp table _c (id text, nombre text, clase text, g text);
\copy _m from '/tmp/pg_municipios.csv' csv header
\copy _l from '/tmp/pg_lugares.csv' csv header
\copy _c from '/tmp/pg_carreteras.csv' csv header
-- Como los daría una IDE española: ETRS89 / UTM 30N, multipolígono.
create table municipios (id text primary key, nombre text, tipo text, geom geometry(MultiPolygon, 25830));
insert into municipios select id, nombre, tipo, ST_Multi(ST_SetSRID(ST_GeomFromWKB(decode(g, 'hex')), 25830)) from _m;
create table lugares (id text primary key, nombre text, categoria text, geom geometry(Point, 4326));
insert into lugares select id, nombre, categoria, ST_SetSRID(ST_GeomFromWKB(decode(g, 'hex')), 4326) from _l;
-- La misma geografía como `geography` (esférica).
create table lugares_geog (id text primary key, nombre text, categoria text, geog geography(Point, 4326));
insert into lugares_geog select id, nombre, categoria, geom::geography from lugares;
-- Sin restricción de tipo: la tabla mezcla LineString y MultiLineString si las hay.
create table carreteras (id text primary key, nombre text, clase text, geom geometry(Geometry, 4326));
insert into carreteras select id, nombre, clase, ST_SetSRID(ST_GeomFromWKB(decode(g, 'hex')), 4326) from _c;
create index on municipios using gist (geom);
create index on lugares using gist (geom);
create index on lugares_geog using gist (geog);
create index on carreteras using gist (geom);
analyze;
SQL
echo "  cargado en $(( $(date +%s) - T0 )) s"
psql "$G" -XAtq -F ' | ' -c "select f_table_name, f_geometry_column, type, srid from geometry_columns union all select f_table_name, f_geography_column, type, srid from geography_columns order by 1" -c "select 'municipios', count(*) from municipios union all select 'lugares', count(*) from lugares union all select 'carreteras', count(*) from carreteras"
echo "# la fuente"
BODY=$(G="$G" python3 -c 'import json,os;print(json.dumps({"name":"banco_geo_pg","url":os.environ["G"]}))')
C=$(curl -s -o /tmp/r.json -w '%{http_code}' -X POST -H "authorization: Bearer $TOK" -H 'content-type: application/json' --data-binary "$BODY" "http://ore-serve.$NS.svc.cluster.local:8080/fuentes")
echo "  POST /fuentes: $C $(head -c 300 /tmp/r.json | sed 's/postgres[^"]*/<url>/g')"
case "$C" in 2*|409) echo "## ok" ;; *) echo "## falla: alta" ;; esac
