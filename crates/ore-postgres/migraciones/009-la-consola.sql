-- ═══════════════════════════════════════════════════════════════════════════
-- 009 · LA CONSOLA (ADR 0058, P8·1, D-P8a)
--
-- El SQL de la consola corre como el rol que se elige, con su contraseña: se
-- guarda CIFRADA (AES-256-GCM, la clave en un Secret, nunca aquí) al crear el
-- rol y al regenerarla. Nula si no se sabe (los roles de antes de P8·1, o un
-- plano sin `--clave-contrasenas`): se regenera una vez para usarlo en la consola.
--
-- Y quién ejecutó qué, sin el texto: el SQL de una persona puede llevar datos.
-- ═══════════════════════════════════════════════════════════════════════════

alter table plano.rol add column if not exists contrasena_cifrada text;

create table if not exists plano.sql_ejecutado (
  id           bigint generated always as identity primary key,
  cuando       timestamptz not null default now(),
  quien        text,
  organizacion text        not null,
  proyecto     text        not null,
  rama         text        not null,
  rol          text        not null,
  base         text        not null,
  ms           integer     not null,
  filas        integer     not null default 0,
  error        text
);

create index if not exists sql_ejecutado_por_proyecto
  on plano.sql_ejecutado (organizacion, proyecto, cuando desc);
