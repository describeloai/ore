-- Copia literal de `lector::consulta` (crates/ore-cli/src/lector.rs), con `{d}`
-- por el dataset. La lee `grabar-bigquery-rest.py`; A3 muda la receta al driver
-- y un test compara ambas, para que esta copia no envejezca en silencio.
WITH kc AS (
  SELECT k.table_name, k.column_name
  FROM `{d}`.INFORMATION_SCHEMA.KEY_COLUMN_USAGE k
  JOIN `{d}`.INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc ON tc.constraint_name = k.constraint_name
  WHERE tc.constraint_type = 'PRIMARY KEY'
), fk AS (
  SELECT k.table_name, k.column_name, ANY_VALUE(u.table_name) AS ref_table
  FROM `{d}`.INFORMATION_SCHEMA.KEY_COLUMN_USAGE k
  JOIN `{d}`.INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc ON tc.constraint_name = k.constraint_name
   AND tc.constraint_type = 'FOREIGN KEY'
  JOIN `{d}`.INFORMATION_SCHEMA.CONSTRAINT_COLUMN_USAGE u ON u.constraint_name = k.constraint_name
  GROUP BY k.table_name, k.column_name
), n AS (SELECT table_id, row_count FROM `{d}.__TABLES__`
), o AS (
  SELECT table_name,
         MAX(IF(option_name = 'require_partition_filter', option_value, NULL)) AS exige_filtro,
         MAX(IF(option_name = 'enable_change_history', option_value, NULL)) AS historial
  FROM `{d}`.INFORMATION_SCHEMA.TABLE_OPTIONS GROUP BY table_name
)
SELECT c.table_name, t.table_type, n.row_count, c.column_name, c.ordinal_position,
       c.is_nullable, c.data_type, fp.description AS column_description,
       kc.column_name IS NOT NULL AS is_key, fk.ref_table,
       c.is_partitioning_column, o.exige_filtro, o.historial
FROM `{d}`.INFORMATION_SCHEMA.COLUMNS c
JOIN `{d}`.INFORMATION_SCHEMA.TABLES t ON t.table_name = c.table_name
LEFT JOIN n ON n.table_id = c.table_name
LEFT JOIN `{d}`.INFORMATION_SCHEMA.COLUMN_FIELD_PATHS fp
  ON fp.table_name = c.table_name AND fp.field_path = c.column_name
LEFT JOIN kc ON kc.table_name = c.table_name AND kc.column_name = c.column_name
LEFT JOIN fk ON fk.table_name = c.table_name AND fk.column_name = c.column_name
LEFT JOIN o ON o.table_name = c.table_name
ORDER BY c.table_name, c.ordinal_position
