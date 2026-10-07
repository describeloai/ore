-- ═══════════════════════════════════════════════════════════════════════════
-- 001 · EL ESQUELETO (ADR 0058, P4·1)
--
-- Lo que `ore-postgres` recuerda. Cada fila lleva lo DESEADO (lo que alguien
-- pidió) y lo OBSERVADO (lo que hay de verdad en el almacenamiento y en NeonVM):
-- desde P4·2 el reconciliador lleva lo uno hacia lo otro.
--
-- ⛔ La organización es la de la CELDA que pidió (la dice `ore-iam`), nunca la
--   del cuerpo. Es la primera columna de cada clave: dos organizaciones pueden
--   tener cada una su proyecto `ventas`, y ninguna consulta mira sin ella.
-- ═══════════════════════════════════════════════════════════════════════════

create schema if not exists plano;

-- Un proyecto: desde P4·2, un tenant del almacenamiento.
create table if not exists plano.proyecto (
  organizacion text        not null,
  id           text        not null
    check (id ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'),
  -- La celda por la que se creó (`iam.celda.id`): quién lo pidió, no de quién es.
  celda        text        not null,
  -- `user:<handle>` de quien lo creó (0052). Lo pone `ore-serve` (P4·6).
  dueno        text
    check (dueno is null or dueno ~ '^user:[a-z][a-z0-9-]*$'),
  creado       timestamptz not null default now(),
  deseado      text        not null default 'vivo'
    check (deseado in ('vivo', 'borrado')),
  observado    text        not null default 'nuevo',
  -- Sube cada vez que cambia lo deseado: el reconciliador sabe si lo que hizo
  -- sigue siendo lo que se quiere.
  generacion   bigint      not null default 1,
  primary key (organizacion, id)
);

-- Toda creación, cambio o borrado: lo que el usuario sondea.
create table if not exists plano.operacion (
  id           text        primary key,
  organizacion text        not null,
  proyecto     text        not null,
  tipo         text        not null,
  estado       text        not null default 'en-curso'
    check (estado in ('en-curso', 'hecha', 'fallida')),
  error        text,
  celda        text        not null,
  creada       timestamptz not null default now(),
  terminada    timestamptz,
  check ((estado = 'en-curso') = (terminada is null))
);

-- ⭐ UNA operación en curso por proyecto: la segunda choca aquí y es un 409. El
--   cerco lo pone la base, no el código que pregunta antes.
create unique index if not exists una_en_curso_por_proyecto
  on plano.operacion (organizacion, proyecto)
  where estado = 'en-curso';

create index if not exists operaciones_por_proyecto
  on plano.operacion (organizacion, proyecto, creada desc);
