COPY (
WITH base AS (
  SELECT *, ST_XMin(_g) AS _x0, ST_YMin(_g) AS _y0, ST_XMax(_g) AS _x1, ST_YMax(_g) AS _y1,
    ST_GeometryType(_g)::VARCHAR AS _tipo
  FROM _ore_fuente
), partir AS (
  SELECT *, coalesce(
    NOT ST_IsEmpty(_g)
    AND (_x1 - _x0 > 19567.87924100512 OR _y1 - _y0 > 19567.87924100512)
    AND (_tipo IN ('LINESTRING', 'MULTILINESTRING')
      OR (_tipo IN ('POLYGON', 'MULTIPOLYGON') AND ST_Area(_g) < 0.1 * (_x1 - _x0) * (_y1 - _y0))),
    false) AS _parte
  FROM base
), del_tramo AS (
  SELECT * FROM partir WHERE _parte OR ((CASE WHEN _g IS NULL OR ST_IsEmpty(_g) THEN NULL ELSE ST_Hilbert(
    ST_Point(greatest(least((ST_XMin(_g) + ST_XMax(_g)) / 2, -8150000.0), -8300000.0),
             greatest(least((ST_YMin(_g) + ST_YMax(_g)) / 2, 5050000.0), 4900000.0)),
    ST_MakeBox2D(ST_Point(-8300000.0, 4900000.0), ST_Point(-8150000.0, 5050000.0))) END) >= 715827882 OR (CASE WHEN _g IS NULL OR ST_IsEmpty(_g) THEN NULL ELSE ST_Hilbert(
    ST_Point(greatest(least((ST_XMin(_g) + ST_XMax(_g)) / 2, -8150000.0), -8300000.0),
             greatest(least((ST_YMin(_g) + ST_YMax(_g)) / 2, 5050000.0), 4900000.0)),
    ST_MakeBox2D(ST_Point(-8300000.0, 4900000.0), ST_Point(-8150000.0, 5050000.0))) END) IS NULL)
), enteras AS (
  SELECT _id_u, _fichero, _fila, 0 AS _trozo, _g, "tarifa", "cuando" FROM del_tramo WHERE NOT _parte
), celdas AS (
  SELECT p._id_u, p._fichero, p._fila, rx.ix, ry.iy,
    ST_Intersection(p._g, ST_MakeEnvelope(-20037508.342789244 + rx.ix * 4891.96981025128, -20037508.342789244 + ry.iy * 4891.96981025128, -20037508.342789244 + (rx.ix + 1) * 4891.96981025128, -20037508.342789244 + (ry.iy + 1) * 4891.96981025128)) AS _g, p."tarifa", p."cuando"
  FROM del_tramo p,
    range(floor((p._x0 - -20037508.342789244) / 4891.96981025128)::BIGINT, floor((p._x1 - -20037508.342789244) / 4891.96981025128)::BIGINT + 1) rx(ix),
    range(floor((p._y0 - -20037508.342789244) / 4891.96981025128)::BIGINT, floor((p._y1 - -20037508.342789244) / 4891.96981025128)::BIGINT + 1) ry(iy)
  WHERE p._parte AND ST_Intersects(p._g, ST_MakeEnvelope(-20037508.342789244 + rx.ix * 4891.96981025128, -20037508.342789244 + ry.iy * 4891.96981025128, -20037508.342789244 + (rx.ix + 1) * 4891.96981025128, -20037508.342789244 + (ry.iy + 1) * 4891.96981025128))
), trozos AS (
  SELECT _id_u, _fichero, _fila,
    (row_number() OVER (PARTITION BY _id_u ORDER BY ix, iy) - 1) AS _trozo, _g, "tarifa", "cuando"
  FROM celdas WHERE NOT ST_IsEmpty(_g)
), todo AS (
  SELECT * FROM enteras UNION ALL BY NAME SELECT * FROM trozos
), ordenado AS (
  SELECT *, CASE WHEN _g IS NULL OR ST_IsEmpty(_g) THEN NULL ELSE ST_Hilbert(
    ST_Point(greatest(least((ST_XMin(_g) + ST_XMax(_g)) / 2, -8150000.0), -8300000.0),
             greatest(least((ST_YMin(_g) + ST_YMax(_g)) / 2, 5050000.0), 4900000.0)),
    ST_MakeBox2D(ST_Point(-8300000.0, 4900000.0), ST_Point(-8150000.0, 5050000.0))) END AS _k FROM todo
)
SELECT
  (CASE WHEN _id_u >= 9223372036854775808 THEN _id_u::HUGEINT - 18446744073709551616
        ELSE _id_u::HUGEINT END)::BIGINT AS "_ore_id",
  _fichero::INTEGER AS "_ore_fichero",
  _fila::BIGINT AS "_ore_fila",
  NULL::VARCHAR AS "_ore_clave",
  _trozo::INTEGER AS "_ore_trozo",
  ST_AsWKB(_g) AS "geom",
  CASE WHEN _g IS NULL OR ST_IsEmpty(_g) THEN NULL ELSE {'xmin': ST_XMin(_g), 'ymin': ST_YMin(_g), 'xmax': ST_XMax(_g), 'ymax': ST_YMax(_g)} END AS "geom_bbox", "tarifa", "cuando"
FROM ordenado
WHERE (_k >= 715827882 OR _k IS NULL)
ORDER BY _k NULLS LAST, _id_u, _trozo
) TO '/trabajo/indice/indice-0001.parquet' (FORMAT parquet, COMPRESSION zstd, ROW_GROUP_SIZE 100000, FIELD_IDS {'_ore_id': 1, '_ore_fichero': 2, '_ore_fila': 3, '_ore_clave': 4, '_ore_trozo': 5, 'geom': 6, 'geom_bbox': {'__duckdb_field_id': 7, 'xmin': 10, 'ymin': 11, 'xmax': 12, 'ymax': 13}, 'tarifa': 8, 'cuando': 9});
