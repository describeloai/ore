-- ═══════════════════════════════════════════════════════════════════════════
-- 003 · LOS ENDPOINTS (ADR 0058, P4·3·3)
--
-- Un endpoint es un cómputo sobre una rama: una VM de NeonVM en `ore-pg-computo`
-- con su ConfigMap (la especificación). Su `vm` es el nombre en Kubernetes,
-- único en todo el clúster (`ep-` + 20 hex): el id que pone el usuario sólo es
-- único dentro de su proyecto.
--
-- ⭐ El cerco, capa 1: UNA rama, UN endpoint de escritura vivo. Lo cierra la base
--   con un índice único; dos peticiones a la vez, una gana y la otra es un 409.
--   La capa 2 es la generación (la VM nueva no nace mientras quede un runner de
--   la vieja) y la 3, los términos de los safekeepers.
-- ═══════════════════════════════════════════════════════════════════════════

create table if not exists plano.endpoint (
  organizacion text             not null,
  proyecto     text             not null,
  rama         text             not null,
  id           text             not null
    check (id ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'),
  tipo         text             not null default 'lectura-escritura'
    check (tipo in ('lectura-escritura', 'lectura')),
  vm           text             not null unique
    check (vm ~ '^ep-[0-9a-f]{20}$'),
  -- Unidades de cómputo (1 CU = 1 vCPU y 4 GiB), los límites del autoescalado.
  cu_min       double precision not null default 0.25,
  cu_max       double precision not null default 1,
  check (cu_min >= 0.25 and cu_min <= cu_max and cu_max <= 2),
  generacion   bigint           not null default 1,
  deseado      text             not null default 'vivo'
    check (deseado in ('vivo', 'borrado')),
  observado    text             not null default 'nuevo',
  -- La IP de la VM en la overlay (la que usará el proxy, P5) y la de su pod.
  direccion    text,
  ip_pod       text,
  creado       timestamptz      not null default now(),
  primary key (organizacion, proyecto, id),
  -- En cascada sólo para que borrar un proyecto entero no choque: antes de llegar aquí el
  -- reconciliador ya ha quitado sus VMs, y el API no deja borrar una rama con endpoints.
  foreign key (organizacion, proyecto, rama) references plano.rama (organizacion, proyecto, id)
    on delete cascade
);

create unique index if not exists endpoint_escritura_por_rama
  on plano.endpoint (organizacion, proyecto, rama)
  where tipo = 'lectura-escritura' and deseado = 'vivo';

alter table plano.operacion add column if not exists endpoint text;
