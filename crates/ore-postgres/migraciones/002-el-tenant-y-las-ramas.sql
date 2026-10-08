-- ═══════════════════════════════════════════════════════════════════════════
-- 002 · EL TENANT Y LAS RAMAS (ADR 0058, P4·2)
--
-- Un proyecto es un tenant del almacenamiento; una rama, un timeline. Los ids
-- de los dos los pone `ore-postgres` y se guardan ANTES de llamar al
-- `storage_controller`: un reintento usa el mismo id y nunca crea dos.
--
-- Las operaciones ya no nacen hechas: las termina el reconciliador, que las
-- recorre por `siguiente` y espera más entre intentos cuantos más lleva.
-- ═══════════════════════════════════════════════════════════════════════════

alter table plano.proyecto add column if not exists tenant text
  check (tenant ~ '^[0-9a-f]{32}$');
create unique index if not exists proyecto_tenant on plano.proyecto (tenant);

create table if not exists plano.rama (
  organizacion    text        not null,
  proyecto        text        not null,
  id              text        not null
    check (id ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'),
  timeline        text        not null unique
    check (timeline ~ '^[0-9a-f]{32}$'),
  -- De dónde sale: la rama padre y, si se dijo, un LSN o un instante. Sin
  -- padre, es la primera (`main`).
  padre           text,
  lsn_origen      text,
  instante_origen timestamptz,
  deseado         text        not null default 'viva'
    check (deseado in ('viva', 'borrada')),
  observado       text        not null default 'nueva',
  creada          timestamptz not null default now(),
  primary key (organizacion, proyecto, id),
  foreign key (organizacion, proyecto) references plano.proyecto on delete cascade
);

alter table plano.operacion
  add column if not exists rama      text,
  add column if not exists intentos  integer     not null default 0,
  add column if not exists siguiente timestamptz not null default now();

create index if not exists operaciones_pendientes
  on plano.operacion (siguiente)
  where estado = 'en-curso';
