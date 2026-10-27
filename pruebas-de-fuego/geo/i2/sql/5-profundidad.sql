WITH grupos AS (
  SELECT file_name, row_group_id, min(try_cast(stats_min_value AS DOUBLE)) FILTER (WHERE path_in_schema = 'geom_bbox, xmin') AS xmin, min(try_cast(stats_min_value AS DOUBLE)) FILTER (WHERE path_in_schema = 'geom_bbox, ymin') AS ymin, max(try_cast(stats_max_value AS DOUBLE)) FILTER (WHERE path_in_schema = 'geom_bbox, xmax') AS xmax, max(try_cast(stats_max_value AS DOUBLE)) FILTER (WHERE path_in_schema = 'geom_bbox, ymax') AS ymax
  FROM parquet_metadata('/trabajo/indice/*.parquet') GROUP BY ALL
), puntos AS (
  SELECT row_number() OVER () AS n,
    ("geom_bbox".xmin + "geom_bbox".xmax) / 2 AS x, ("geom_bbox".ymin + "geom_bbox".ymax) / 2 AS y
  FROM read_parquet('/trabajo/indice/*.parquet') WHERE "geom_bbox" IS NOT NULL
  USING SAMPLE reservoir(2000 ROWS) REPEATABLE (7)
)
SELECT coalesce(avg(c), 0) AS profundidad, (SELECT count(*) FROM grupos) AS grupos FROM (
  SELECT count(*) AS c FROM puntos JOIN grupos
    ON x BETWEEN grupos.xmin AND grupos.xmax AND y BETWEEN grupos.ymin AND grupos.ymax
  GROUP BY n);
