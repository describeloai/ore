-- 002 · EL PAPEL — y por qué es una TABLA y no un `check`
--
-- ⛔ Un `check (papel in (...))` crece con un `alter table` que cualquiera
--   escribe de paso. Una tabla con clave ajena obliga a un `insert`, y un
--   `insert` en una migración es una decisión con fecha y con autor.
--
-- ⭐ Es el censo de ORE aplicado aquí: **una lista que puede crecer sin una
--   decisión deja de significar nada.** Añadir un papel es escribir la
--   migración siguiente, no editar ésta.
--
-- ── Los cuatro, y qué separa a cada uno del anterior ────────────────
--
--   lector         ve. No cambia nada
--   miembro        ve y decide sobre la ontología — responde `review`
--   administrador  además invita, revoca y concede
--   dueno          además puede traspasar la organización, y es UNO
--
-- ⚠️ `dueno` no está aquí por jerarquía: está porque **alguien tiene que poder
--   quedarse sin administradores y aun así entrar**. Una organización cuyo
--   último administrador se va sin traspasar es una organización perdida.

create table if not exists iam.papel (
  nombre    text primary key,
  ordinal   smallint not null unique,
  que_puede text not null
);

insert into iam.papel (nombre, ordinal, que_puede) values
  ('lector',        1, 've la ontologia y no la cambia'),
  ('miembro',       2, 'decide sobre la ontologia: responde la cola de `review`'),
  ('administrador', 3, 'ademas invita, revoca y concede'),
  ('dueno',         4, 'ademas traspasa la organizacion. Es uno')
on conflict (nombre) do nothing;
