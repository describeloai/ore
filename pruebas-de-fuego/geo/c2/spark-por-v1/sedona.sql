-- ORE 0068 C2·4 · Con Sedona 1.9.1: lo que un Transform Heavyweight calcularía y escribiría por /v1.
-- (Corre después de spark.sql: lee ore.geo.lugares.)
-- S5 · Sedona calcula (ST_Buffer en EPSG:25830) y escribe WKB `binary` con su `ore.geo.<col>`.
CREATE TABLE ore.geo.zonas (id BIGINT, zona BINARY) USING iceberg
  TBLPROPERTIES ('ore.geo.zona' = '{"crs":"EPSG:25830","type":"Geometry<Polygon, EPSG:25830>"}');
INSERT INTO ore.geo.zonas
  SELECT id, ST_AsBinary(ST_Buffer(ST_GeomFromWKB(geom), 100)) FROM ore.geo.lugares;
SELECT id, round(ST_Area(ST_GeomFromWKB(zona))) AS area FROM ore.geo.zonas ORDER BY id;
-- S6 · El GEOMETRY nativo de Spark 4.1 por /v1: ¿qué pasa? (en OGC:CRS84, el que Spark admite)
SET spark.sql.geospatial.enabled=true;
CREATE TABLE ore.geo.nativa (id BIGINT, g GEOMETRY(4326)) USING iceberg TBLPROPERTIES ('format-version' = '3');
INSERT INTO ore.geo.nativa SELECT id, st_geomfromwkb(ST_AsBinary(ST_Transform(ST_GeomFromWKB(geom), 'EPSG:25830', 'EPSG:4326')), 4326) FROM ore.geo.lugares;
SELECT count(*) AS nativas FROM ore.geo.nativa;
