-- ORE 0068 C2·4 · Spark 4.1 + Iceberg 1.12 contra el catálogo /v1 de ORE (catálogo `ore`).
-- S1 · ¿crea y escribe? Una tabla con la geometría en WKB `binary` y su `ore.geo.<col>`, el estado
--      de hoy del lago (0067 G2): lo que un Transform Heavyweight escribiría.
CREATE NAMESPACE IF NOT EXISTS ore.geo;
CREATE TABLE ore.geo.lugares (id BIGINT, nombre STRING, geom BINARY) USING iceberg
  TBLPROPERTIES ('ore.geo.geom' = '{"crs":"EPSG:25830","type":"Geometry<Point, EPSG:25830>"}');
INSERT INTO ore.geo.lugares VALUES
  (1, 'sol',   unhex('010100000000000000B0DF1A4100000000A1115141')),
  (2, 'retiro', unhex('010100000000000000B0F81A41000000003D115141'));
-- S2 · ¿lo lee?
SELECT id, nombre, hex(geom) AS wkb FROM ore.geo.lugares ORDER BY id;
-- S3 · ¿añade? (un segundo commit)
INSERT INTO ore.geo.lugares VALUES (3, 'cibeles', unhex('01010000000000000030EC1A4100000080AD115141'));
SELECT count(*) AS filas FROM ore.geo.lugares;
-- S4 · ¿qué propiedades quedan? (las de ORE y las de Iceberg)
SHOW TBLPROPERTIES ore.geo.lugares;
