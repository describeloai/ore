-- La semilla de `pruebas-de-fuego/bigquery-real.sh`: 5 clientes y 8 pedidos,
-- todos con id `ore-e2e-*` para poder borrarlos sin tocar nada mas:
--   DELETE FROM ventas.pedidos WHERE id LIKE 'ore-e2e-%';
--   DELETE FROM ventas.clientes WHERE id LIKE 'ore-e2e-%';
--
-- Cada fila esta para romper algo concreto: el texto 'null' (c4), un NULL en
-- cada tipo (c3, p5, p6), UTF-8 (c5), el 29 de febrero (c5), microsegundos
-- (p2), un desfase distinto de UTC (p4), la epoca (p3), un NUMERIC de 29
-- cifras significativas (p4) y el menor positivo de su escala (p7).
--
-- Las tablas (esquema medido el 2026-09-26):
--   ventas.clientes (id STRING REQUIRED, email STRING REQUIRED, alta DATE, pais STRING)
--   ventas.pedidos  (id STRING REQUIRED, cliente_id STRING, total NUMERIC, ts TIMESTAMP)
--
-- Cargarla desde Windows: `bq` desde Git Bash no arranca (python3.14) y por
-- una tuberia de PowerShell le llega un BOM; lo que funciona es
--   cmd /c "bq query --use_legacy_sql=false --project_id=<p> < semilla.sql"
INSERT INTO ventas.clientes (id, email, alta, pais) VALUES
  ('ore-e2e-c1', 'ana@ejemplo.test',   DATE '2024-01-15', 'ES'),
  ('ore-e2e-c2', 'luis@ejemplo.test',  DATE '1999-12-31', 'FR'),
  ('ore-e2e-c3', 'eva@ejemplo.test',   NULL,              NULL),
  ('ore-e2e-c4', 'null@ejemplo.test',  DATE '2026-09-26', 'null'),
  ('ore-e2e-c5', 'ñandú@ejemplo.test', DATE '2000-02-29', 'PT');
INSERT INTO ventas.pedidos (id, cliente_id, total, ts) VALUES
  ('ore-e2e-p1', 'ore-e2e-c1', NUMERIC '19.99',               TIMESTAMP '2026-09-01 10:00:00+00'),
  ('ore-e2e-p2', 'ore-e2e-c1', NUMERIC '0',                   TIMESTAMP '2026-09-02 23:59:59.123456+00'),
  ('ore-e2e-p3', 'ore-e2e-c2', NUMERIC '-5.5',                TIMESTAMP '1970-01-01 00:00:00+00'),
  ('ore-e2e-p4', 'ore-e2e-c2', NUMERIC '12345678901234567890.123456789', TIMESTAMP '2026-09-03 12:00:00+02'),
  ('ore-e2e-p5', 'ore-e2e-c3', NULL,                          NULL),
  ('ore-e2e-p6', NULL,         NUMERIC '100',                 TIMESTAMP '2026-09-04 08:30:00+00'),
  ('ore-e2e-p7', 'ore-e2e-c4', NUMERIC '0.000000001',         TIMESTAMP '2026-09-05 00:00:00+00'),
  ('ore-e2e-p8', 'ore-e2e-c5', NUMERIC '7',                   TIMESTAMP '2026-09-26 18:00:00+00');
