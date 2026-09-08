-- 006 · LA INVITACIÓN — pertenecer deja de ser implícito
--
-- ⭐⭐ El estado NO es una columna. Se DERIVA de qué marcas de tiempo están
--   puestas, y eso es `P2` de ORE —*lo derivable no se declara*— aplicado aquí:
--   una columna `estado` junto a `redimida_en` son dos fuentes para el mismo
--   hecho, y el día que discrepan gana la que alguien miró primero.
--
--     revocada_en  puesta  →  revocada
--     redimida_en  puesta  →  redimida
--     caduca_en    pasada  →  caducada
--     si no                →  pendiente
--
--   El orden importa: una invitación revocada DESPUÉS de redimirse sigue
--   habiendo dejado entrar a alguien, y la huella lo cuenta.
--
-- ⚠️ `correo` y no `persona`: se invita a alguien que **todavía no existe**
--   aquí. La persona nace al redimir, y ése es el momento en que el emisor
--   dice quién es.

create table if not exists iam.invitacion (
  id           text        primary key,
  organizacion text        not null references iam.organizacion(id) on delete cascade,
  correo       text        not null,
  papel        text        not null references iam.papel(nombre),
  invito       text        not null references iam.persona(id),
  emitida_en   timestamptz not null default now(),
  caduca_en    timestamptz not null,
  redimida_en  timestamptz,
  redimio      text        references iam.persona(id),
  revocada_en  timestamptz,
  revoco       text        references iam.persona(id),

  -- Redimir sin decir quién redimió es media huella.
  check ((redimida_en is null) = (redimio is null)),
  check ((revocada_en is null) = (revoco  is null))
);

create index if not exists invitacion_por_correo on iam.invitacion (lower(correo));
create index if not exists invitacion_por_org    on iam.invitacion (organizacion);

create or replace view iam.invitacion_estado as
  select i.*,
         case
           when i.revocada_en is not null then 'revocada'
           when i.redimida_en is not null then 'redimida'
           when i.caduca_en   <  now()    then 'caducada'
           else 'pendiente'
         end as estado
    from iam.invitacion i;
