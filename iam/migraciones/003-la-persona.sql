-- 003 · LA PERSONA — `(emisor, sub) → un identificador NUESTRO`
--
-- ⭐⭐ La idea es de `014-sujeto.sql` y es la que más se paga después: **el
--   sujeto del producto NO es el `sub` del emisor.** Está atado a él por una
--   correspondencia, y por eso:
--
--     · cambiar de realm, o de IdP, no renombra a nadie
--     · dos emisores pueden traer el mismo `sub` sin colisionar
--     · el identificador que viaja por nuestras tablas es opaco, y no filtra
--       en qué directorio vive la persona
--
-- ⛔ Y por eso la unicidad es `(emisor, sub)` y no `sub`: un `sub` sin su
--   emisor no identifica a nadie. Es el mismo error que aceptar un token sin
--   mirar el `iss`.
--
-- ⚠️ `correo` y `nombre` son una COPIA de lo que dijo el emisor el día que
--   entró, y se dice para que nadie los trate como la verdad. La verdad está
--   en el emisor; esto es para poder pintar una lista sin llamarle.

create table if not exists iam.persona (
  id        text        primary key,
  emisor    text        not null,
  sub       text        not null,
  correo    text,
  nombre    text,
  creada_en timestamptz not null default now(),
  vista_en  timestamptz,
  unique (emisor, sub)
);

comment on column iam.persona.id     is 'Opaco. No se deriva del `sub`: se le asigna.';
comment on column iam.persona.correo is 'Copia de lo que dijo el emisor. La verdad esta alli.';
