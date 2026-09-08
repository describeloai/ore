-- 001 · EL LIBRO — qué corrió, cuándo, y con qué contenido
--
-- ⭐ La idea es suya y se toma entera (`006-dueno.sql`): **una migración
--   aplicada es inmutable**. El runner guarda la huella del fichero y falla si
--   cambia, porque «lo que corrió y lo que dice el fichero dejan de ser lo
--   mismo» — y entonces el historial miente sin que nadie lo note.
--
-- ⇒ Corregir una migración ya aplicada NO es editarla: es escribir la
--   siguiente. Eso cuesta un fichero y ahorra un historial falso.

create schema if not exists iam;

create table if not exists iam.migracion (
  nombre     text        primary key,
  huella     text        not null,
  corrida_en timestamptz not null default now()
);

comment on table iam.migracion is
  'El libro del runner. La huella es sha256 del fichero: si cambia, se para.';
