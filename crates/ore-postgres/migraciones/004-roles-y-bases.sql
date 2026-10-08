-- ═══════════════════════════════════════════════════════════════════════════
-- 004 · ROLES Y BASES (ADR 0058, P4·4)
--
-- Son de cada RAMA, como en Lakebase y en Neon: viven en los datos, así que una
-- rama nueva hereda los de su padre (se copian las filas al crearla).
--
-- ⛔ De un rol sólo se guarda el verificador SCRAM-SHA-256: la contraseña se
--   enseña una vez, al crearla o regenerarla, y no se puede volver a leer.
--
-- Borrar no es quitar la fila: es `deseado = 'borrado'`, y la fila se va cuando
-- un cómputo de escritura de la rama lo ha aplicado (`delta_operations` de
-- compute_ctl). Si no hay ninguno arrancado, espera al siguiente: si se quitara
-- antes, el rol seguiría en los datos y podría entrar.
-- ═══════════════════════════════════════════════════════════════════════════

create table if not exists plano.rol (
  organizacion text        not null,
  proyecto     text        not null,
  rama         text        not null,
  nombre       text        not null
    check (nombre ~ '^[a-z_][a-z0-9_-]{0,62}$' and nombre !~ '^pg_'
           and nombre not in ('cloud_admin', 'neon_superuser', 'public', 'postgres', 'zenith_admin')),
  verificador  text        not null check (verificador like 'SCRAM-SHA-256$%'),
  deseado      text        not null default 'vivo' check (deseado in ('vivo', 'borrado')),
  creado       timestamptz not null default now(),
  primary key (organizacion, proyecto, rama, nombre),
  foreign key (organizacion, proyecto, rama) references plano.rama (organizacion, proyecto, id)
    on delete cascade
);

create table if not exists plano.base (
  organizacion text        not null,
  proyecto     text        not null,
  rama         text        not null,
  nombre       text        not null
    check (nombre ~ '^[a-z_][a-z0-9_-]{0,62}$' and nombre not in ('postgres', 'template0', 'template1')),
  dueno        text        not null,
  deseado      text        not null default 'vivo' check (deseado in ('vivo', 'borrado')),
  creada       timestamptz not null default now(),
  primary key (organizacion, proyecto, rama, nombre),
  foreign key (organizacion, proyecto, rama, dueno) references plano.rol (organizacion, proyecto, rama, nombre)
    on delete cascade
);
