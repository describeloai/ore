-- 035 · EL PERFIL DE LA ORGANIZACIÓN: título y logo
--
-- ══════════════════════════════════════════════════════════════════════════
-- `nombre` es un IDENTIFICADOR (etiqueta: de él salen la celda de casa y la
-- dirección `<nombre>.ore.paladio.io`) y no se cambia. Lo que una cuenta enseña
-- de sí misma es otra cosa: un TÍTULO («Acme Corp S.L.») y un LOGO. Los dos son
-- del perfil, los edita quien tiene `organizacion:editar` (ORGADMIN), y la
-- consola los pinta donde antes ponía el identificador.
--
-- El logo va EMBEBIDO (`data:image/...;base64,...`, ≤ 200 KB): este plano no
-- tiene almacén de ficheros, y un logo es pequeño, se lee en cada pantalla y
-- cambia una vez al año. Una URL externa sería una dependencia de un tercero en
-- cada carga de la consola.
--
-- El título nace con `fundar` (el registro de Keycloak pregunta «Organización»
-- y eso es el título; el identificador se deriva) y se puede cambiar después.
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md` (E6, «lo que sigue»)
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.organizacion add column if not exists titulo text;
alter table iam.organizacion add column if not exists logo   text;

do $$ begin
  if not exists (select 1 from pg_constraint where conname = 'organizacion_logo_es_imagen') then
    alter table iam.organizacion add constraint organizacion_logo_es_imagen
      check (logo is null or (logo like 'data:image/%;base64,%' and length(logo) <= 200000));
  end if;
  if not exists (select 1 from pg_constraint where conname = 'organizacion_titulo_cabe') then
    alter table iam.organizacion add constraint organizacion_titulo_cabe
      check (titulo is null or (length(btrim(titulo)) between 1 and 80));
  end if;
end $$;

comment on column iam.organizacion.titulo is 'Como se llama de cara a la gente. `nombre` es el identificador y no cambia; esto si.';
comment on column iam.organizacion.logo   is 'Imagen embebida (data:image/...;base64,...), hasta 200 KB. NULL = sin logo.';

insert into iam.potestad (nombre, que_hace, ejercida) values
  ('organizacion:editar', 'cambiar el titulo y el logo de la organizacion', true)
on conflict (nombre) do nothing;
insert into iam.rol_potestad (rol, potestad) values ('ORGADMIN', 'organizacion:editar')
on conflict do nothing;
