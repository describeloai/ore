-- 0057 B4·6 · Build de SQL sobre una foreign database, en vivo (victor).
-- Va en el repositorio `transforms_sql_test` (test_project), como
-- `transforms/b46.sql`. Commit y Build: el build lee en vivo por la vista de la
-- foreign database y lo escribe el Transform (no una copia mantenida).
CREATE OR REPLACE DATASET sandbox.b46_sintetica_sql AS
SELECT nombre, count(*) AS n FROM bq_foreign.ventas.ore_e2e_sintetica GROUP BY nombre;
