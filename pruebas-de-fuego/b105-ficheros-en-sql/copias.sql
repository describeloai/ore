-- 0049 B10·5 · Copiar sin función: `data` es el ítem, y sus bytes se copian.
--
-- En el mismo repositorio transforms-sql (p. ej. `transforms/copias.sql`):
-- Preview, commit y Build. Cada contrato da un fichero, `<contrato>/copia.pdf`,
-- con los mismos bytes.
create or replace media collection sandbox.default.b105_copias media document formats (pdf)
comment '0049 B10·5: una copia de cada contrato, desde SQL' as
select 'copia.pdf' as name, c._item as data
from s3_stuff.nueva_carpeta.contratos as c
where c.content_type = 'application/pdf'
