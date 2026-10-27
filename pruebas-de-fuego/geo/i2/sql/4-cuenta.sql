SELECT count(*) AS filas,
  (coalesce(sum("_ore_id"::HUGEINT + CASE WHEN "_ore_id" < 0 THEN 18446744073709551616 ELSE 0 END), 0)
    % 18446744073709551616)::UBIGINT AS suma_ids
FROM read_parquet(['/trabajo/indice/indice-0000.parquet', '/trabajo/indice/indice-0001.parquet'])
WHERE "_ore_trozo" = 0;
