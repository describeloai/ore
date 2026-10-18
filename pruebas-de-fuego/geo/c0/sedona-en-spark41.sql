-- ORE 0068 C0·0 · ¿Sedona 1.9.1 corre sobre Spark 4.1? Predicados, medidas, H3 y el join espacial.
SELECT ST_Intersects(ST_GeomFromWKT('POINT(1 1)'), ST_Buffer(ST_Point(0, 0), 2)) AS intersecta,
       ST_Area(ST_Buffer(ST_Point(0, 0), 1)) AS area,
       ST_SRID(ST_SetSRID(ST_Point(-3.7, 40.4), 4326)) AS srid,
       ST_AsText(ST_Transform(ST_SetSRID(ST_Point(-3.7, 40.4), 4326), 'EPSG:4326', 'EPSG:25830')) AS utm,
       ST_H3CellIDs(ST_Point(-3.7, 40.4), 9, false) AS h3;
CREATE OR REPLACE TEMP VIEW pts AS
  SELECT id, ST_Point(rand(1) * 10, rand(2) * 10) AS g FROM range(200000);
CREATE OR REPLACE TEMP VIEW zonas AS
  SELECT id AS z, ST_Buffer(ST_Point(rand(3) * 10, rand(4) * 10), 0.3) AS g FROM range(500);
EXPLAIN SELECT z, count(*) FROM zonas JOIN pts ON ST_Contains(zonas.g, pts.g) GROUP BY z;
SELECT count(*) AS pares FROM zonas JOIN pts ON ST_Contains(zonas.g, pts.g);
EXPLAIN SELECT count(*) FROM pts a JOIN pts b ON ST_DWithin(a.g, b.g, 0.01) WHERE a.id < 20000 AND b.id < 20000;
