-- ORE 0068 C0·3 · Por qué Spark + Sedona tarda en Q2 y en B2: sus planes.
CREATE TEMPORARY VIEW lugares USING parquet OPTIONS (path '/banco/lugares.parquet');
CREATE TEMPORARY VIEW municipios USING parquet OPTIONS (path '/banco/municipios.parquet');
CREATE TEMPORARY VIEW trip USING parquet OPTIONS (path '/sb/sf1/trip.parquet');
CREATE TEMPORARY VIEW zone USING parquet OPTIONS (path '/sb/sf1/zone.parquet');
EXPLAIN select count(*) as municipios, sum(n) as lugares from (
  select m.id, count(*) as n from lugares l join municipios m
  on ST_Contains(ST_GeomFromWKB(m.geom), ST_GeomFromWKB(l.geom)) group by m.id) x;
EXPLAIN SELECT COUNT(*) FROM trip t WHERE ST_Intersects(ST_GeomFromWKB(t.t_pickuploc),
  (SELECT ST_GeomFromWKB(z.z_boundary) FROM zone z WHERE z.z_name = 'Coconino County' LIMIT 1));
