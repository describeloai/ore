-- 0049 B10·5 · Ficheros que dan ficheros, en SQL: un PNG por página de cada contrato.
--
-- En un repositorio transforms-sql de victor (p. ej. `transforms/paginas.sql`).
-- Usa la función que ya publicaste en Functions, `functions.pdf_a_png`.
--
--   ① Preview (Statement 1): lo que serían los ficheros, sin escribir nada;
--   ② commit y Build: la colección nace y se llena;
--   ③ Build otra vez: nada que hacer, nada escrito;
--   ④ descomenta el `and` de abajo, commit y Build: la consulta cambia, así que
--      se recalcula todo, y el contrato escaneado se queda sin páginas (su
--      marca `empty`) y sus PNG se retiran.
--
-- Después, `b105_ver.py` (Run) dice lo que quedó.
create or replace media collection sandbox.default.b105_paginas media image formats (png)
comment '0049 B10·5: un PNG por página, desde SQL' as
select p.name, p.data, p.anchor
from s3_stuff.nueva_carpeta.contratos as c
cross join lateral functions.pdf_a_png(c._item) as p
where c.content_type = 'application/pdf'
-- and c.path not like '%escaneado%'
