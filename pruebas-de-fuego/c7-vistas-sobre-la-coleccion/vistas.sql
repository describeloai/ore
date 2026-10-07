-- 0057 C7 · Vistas sobre una colección del lago, en vivo (victor).
--
-- En SQL una colección es su LISTADO: una fila por fichero, sin leer un byte
-- (OOS v1alpha17 `04`), con `_item`, `path`, `version`, `digest`, `size`,
-- `content_type`, `content_type_detected`, `checksum`, `modified` y
-- `transaction`. Estas vistas se guardan en `sandbox` (schema `default`) y se
-- calculan al leerlas: la preview de cada una la hace ore-motor.
--
-- Se ejecuta como un `.sql` en el editor (cada sentencia, una celda).

-- 1 · Inventario: cuántos ficheros hay de cada tipo, cuánto ocupan y desde cuándo.
CREATE OR REPLACE VIEW sandbox.contratos_inventario AS
SELECT content_type,
       count(*)      AS ficheros,
       sum(size)     AS bytes,
       min(modified) AS el_primero,
       max(modified) AS el_ultimo
FROM s3_stuff.nueva_carpeta.contratos
GROUP BY content_type;

-- 2 · Por mes: lo que entró cada mes.
CREATE OR REPLACE VIEW sandbox.contratos_por_mes AS
SELECT date_trunc('month', modified) AS mes,
       count(*)                      AS ficheros,
       sum(size)                     AS bytes
FROM s3_stuff.nueva_carpeta.contratos
GROUP BY date_trunc('month', modified);

-- 3 · Duplicados: el mismo contenido (la misma huella) con más de un nombre.
CREATE OR REPLACE VIEW sandbox.contratos_duplicados AS
SELECT digest,
       count(*)              AS copias,
       string_agg(path, ', ') AS rutas
FROM s3_stuff.nueva_carpeta.contratos
GROUP BY digest
HAVING count(*) > 1;

-- 4 · Los escaneados: por su nombre, con su tamaño (para revisarlos aparte).
CREATE OR REPLACE VIEW sandbox.contratos_escaneados AS
SELECT path, size, modified
FROM s3_stuff.nueva_carpeta.contratos
WHERE lower(path) LIKE '%escaneado%';

-- Y leerlas.
SELECT * FROM sandbox.contratos_inventario;
