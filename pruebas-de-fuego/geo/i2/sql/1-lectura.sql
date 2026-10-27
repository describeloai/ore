SET geometry_always_xy = true;
SET preserve_insertion_order = true;
CREATE OR REPLACE TEMP TABLE _ore_ficheros(ordinal INTEGER, ruta VARCHAR, uri VARCHAR);
INSERT INTO _ore_ficheros VALUES
  (0, '/datos/a.parquet', 'gs://cubo/t/viajes/data/a.parquet'),
  (1, '/datos/b.parquet', 'gs://cubo/t/viajes/data/b.parquet');
CREATE OR REPLACE TEMP VIEW _ore_fuente AS
SELECT
  ((f.ordinal::UBIGINT << 40) | s.file_row_number::UBIGINT) AS _id_u,
  f.ordinal AS _fichero,
  s.file_row_number AS _fila,
  CASE WHEN s."geom" IS NULL THEN NULL ELSE ST_Transform(ST_GeomFromWKB(s."geom"), 'EPSG:4326', 'EPSG:3857') END AS _g,
  s."tarifa",
  s."cuando"
FROM read_parquet(['/datos/a.parquet', '/datos/b.parquet'], filename = true, file_row_number = true) s
JOIN _ore_ficheros f ON f.ruta = s.filename
WHERE NOT EXISTS (
  SELECT 1 FROM read_parquet(['/datos/borrados.parquet']) d
  WHERE d.file_path = f.uri AND d.pos = s.file_row_number);
