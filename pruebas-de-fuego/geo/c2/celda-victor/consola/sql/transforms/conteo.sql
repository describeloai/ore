-- ORE 0068 6c · el join espacial en SQL (Heavyweight: lo corre Spark, escrito como en DuckDB).
CREATE OR REPLACE DATASET geo.conteo_sql AS
SELECT z.id, count(*) AS n
FROM geo.zonas_20k z JOIN geo.puntos_20m p ON ST_Contains(z.zona, p.geom)
GROUP BY z.id;
