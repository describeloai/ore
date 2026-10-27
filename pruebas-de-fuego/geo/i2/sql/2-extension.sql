SELECT
  min(ST_XMin(_g)) FILTER (WHERE isfinite(ST_XMin(_g))) AS xmin,
  min(ST_YMin(_g)) FILTER (WHERE isfinite(ST_YMin(_g))) AS ymin,
  max(ST_XMax(_g)) FILTER (WHERE isfinite(ST_XMax(_g))) AS xmax,
  max(ST_YMax(_g)) FILTER (WHERE isfinite(ST_YMax(_g))) AS ymax,
  count(*) AS filas,
  (coalesce(sum(_id_u::HUGEINT), 0) % 18446744073709551616)::UBIGINT AS suma_ids
FROM _ore_fuente;
