-- ORE 0068 C4·2 · El cotejo con PostGIS: los datos (deterministas, `setseed`) y la respuesta de PostGIS al
-- enriquecimiento de `arbol/packages/ventas/etl/cotejo.enrich.yaml`, escrita a mano y sin mirar el SQL que
-- genera ORE. Exporta los datos en WKB hexadecimal y la respuesta a /x (lo lee cotejar.py).
create extension if not exists postgis;
select setseed(0.42);
drop table if exists clientes, municipios, secciones, competidores, respuesta;

create table municipios as
select 'm' || lpad(i::text, 5, '0') nombre,
       ST_Buffer(ST_SetSRID(ST_Point((i % 45) * 6700 + random() * 2000, (i / 45) * 6700 + random() * 2000), 25830),
                 4200, 'quad_segs=125') geom
from generate_series(0, 1999) i;
create table secciones as
select 's' || lpad(i::text, 5, '0') id, (random() * 2000)::int poblacion, random() * 30000 renta_media,
       (array['residencial', 'comercial', 'industrial', 'verde'])[1 + (random() * 3.999)::int] uso,
       ST_Buffer(ST_SetSRID(ST_Point((i % 95) * 3160 + random() * 500, (i / 95) * 3160 + random() * 500), 25830),
                 1900, 'quad_segs=16') geom
from generate_series(0, 8999) i;
create table competidores as
select 'c' || lpad(i::text, 6, '0') nombre, (random() * 1e6)::int ventas,
       ST_SetSRID(ST_Point(random() * 300000, random() * 300000), 25830) geom
from generate_series(0, 24999) i;
create table clientes as
select 'k' || lpad(i::text, 6, '0') cliente_id,
       ST_SetSRID(ST_Point(random() * 300000, random() * 300000), 25830) ubicacion
from generate_series(0, 19999) i;
create index on municipios using gist (geom);
create index on secciones using gist (geom);
create index on competidores using gist (geom);
analyze;

create table respuesta as
with sec as (
  select c.cliente_id, s.id, s.poblacion, s.renta_media, s.uso,
         ST_Area(ST_Intersection(ST_Buffer(c.ubicacion, 500), s.geom)) a, ST_Area(s.geom) al
  from clientes c join secciones s
    on ST_DWithin(c.ubicacion, s.geom, 500) and ST_Intersects(ST_Buffer(c.ubicacion, 500), s.geom)
), uso as (
  select distinct on (cliente_id) cliente_id, uso
  from (select cliente_id, uso, sum(a) w from sec group by 1, 2) x
  order by cliente_id, w desc, uso
), una as (
  select distinct on (cliente_id) cliente_id, id from sec order by cliente_id, a desc, id
), agregado as (
  select cliente_id, sum(poblacion * a / al) poblacion_500m, sum(renta_media * a) / nullif(sum(a), 0) renta_500m
  from sec group by 1
)
select c.cliente_id,
       (select min(m.nombre) from municipios m where ST_Within(c.ubicacion, m.geom)) municipio,
       (select count(*) from competidores k where ST_DWithin(c.ubicacion, k.geom, 500)) competidores_500m,
       (select sum(k.ventas) from competidores k where ST_DWithin(c.ubicacion, k.geom, 500)) ventas_500m,
       n.nombre competidor_cercano, n.d dist_competidor,
       g.poblacion_500m, g.renta_500m, u.uso uso_500m, s.id seccion
from clientes c
left join lateral (
  select k.nombre, ST_Distance(c.ubicacion, k.geom) d from competidores k
  where ST_DWithin(c.ubicacion, k.geom, 5000) order by k.geom <-> c.ubicacion limit 1) n on true
left join agregado g using (cliente_id)
left join uso u using (cliente_id)
left join una s using (cliente_id);

copy (select nombre, ST_AsHEXEWKB(geom) geom from municipios) to '/x/municipios.csv' csv header;
copy (select id, poblacion, renta_media, uso, ST_AsHEXEWKB(geom) geom from secciones) to '/x/secciones.csv' csv header;
copy (select nombre, ventas, ST_AsHEXEWKB(geom) geom from competidores) to '/x/competidores.csv' csv header;
copy (select cliente_id, ST_AsHEXEWKB(ubicacion) ubicacion from clientes) to '/x/clientes.csv' csv header;
copy (select * from respuesta order by cliente_id) to '/x/postgis.csv' csv header;
