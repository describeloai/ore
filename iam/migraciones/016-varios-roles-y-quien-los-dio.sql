-- 016 · VARIOS ROLES POR PERSONA, Y QUIÉN SE LOS DIO
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ EL ARGUMENTO DE `005` MURIÓ CON EL ORDINAL, Y HABÍA QUE IR A BUSCARLO
--
--   `005` puso la clave primaria en `(persona, organizacion)` —UN rol por
--   persona— y lo argumentó bien:
--
--     «No una lista de papeles: dos papeles a la vez obligan a decidir cuál
--      gana, y esa decisión no la toma nadie explícitamente — la toma el orden
--      en que se leyeron las filas.»
--
--   Eso es cierto **de una escalera**. Con dos peldaños hay que elegir uno. Con
--   CONJUNTOS no: dos roles son la UNIÓN de sus potestades, que está definida,
--   no depende del orden de lectura y no la decide nadie.
--
--   ⇒ La `014` cambió los roles de peldaños a conjuntos y dejó esta clave
--     primaria en pie. Era el resto de un modelo que ya no está.
--
-- ── ⛔ Y no es cosmético: derrota el motivo de `SECURITYADMIN` ─────────────
--
--   Con un solo rol, quien tenga que dar de alta Y vigilar no puede tener los
--   dos: hay que darle `ACCOUNTADMIN`. Y eso es exactamente el fallo que su
--   `76` §4 existe para evitar:
--
--     «Exigir el rol omnipotente para una emergencia significa que la
--      credencial omnipotente acaba circulando.»
--
--   La separación que la `014` compró —cortar sin poder nombrar— era inusable
--   mientras nadie pudiera tener las dos mitades.
--
-- ── ⭐ Y la pertenencia SIGUE SIENDO UNA ──────────────────────────────────
--
--   Se parte en dos cosas que siempre fueron distintas:
--
--       iam.pertenencia       ESTÁS dentro, y desde cuándo.  UNA fila.
--       iam.pertenencia_rol   qué CARGOS tienes.             CERO o varias.
--
--   Cero filas de cargo es «pertenece y nada más», que es lo que la `014`
--   expresaba con un `null` — y ahora se dice sin necesitar un valor nulo para
--   decirlo. La ausencia es la ausencia.
-- ══════════════════════════════════════════════════════════════════════════

create table if not exists iam.pertenencia_rol (
  persona      text        not null,
  organizacion text        not null,
  rol          text        not null references iam.rol(nombre),
  desde        timestamptz not null default now(),

  -- ⭐⭐ QUIÉN LO DIO. `null` significa **EL PROVEEDOR, en el aprovisionamiento**
  --   —es el arranque de su `76` §5— y es el único caso legítimo en toda la
  --   tabla: en una organización recién fundada no hay nadie dentro que pueda
  --   conceder. Sin esta columna, «te lo dio Ada» y «lo trajiste de fábrica»
  --   serían indistinguibles, que es justo lo que una auditoría necesita
  --   separar.
  otorgo       text        references iam.persona(id),

  primary key (persona, organizacion, rol),
  -- ⛔ Y el cargo cuelga de la PERTENENCIA, no de la persona: si alguien sale
  --   de la organizacion, sus cargos ahi se van con el. Sin esto quedarian
  --   filas de poder sin nadie a quien pertenecer.
  foreign key (persona, organizacion)
    references iam.pertenencia (persona, organizacion) on delete cascade
);

create index if not exists pertenencia_rol_por_organizacion
  on iam.pertenencia_rol (organizacion, rol);

comment on column iam.pertenencia_rol.otorgo is
  'Quien concedio el cargo. NULL = el proveedor, en el aprovisionamiento. Es el unico caso legitimo.';

-- Lo que había, tal cual. `null` no viaja: la ausencia de cargo es la ausencia
-- de fila.
insert into iam.pertenencia_rol (persona, organizacion, rol, desde)
select persona, organizacion, rol, desde
  from iam.pertenencia
 where rol is not null
on conflict do nothing;

-- ⛔ El índice de UN dueño colgaba de `pertenencia.rol`, que se va. Se rehace
--   sobre la tabla nueva: la organización sigue teniendo UN `ORGADMIN`.
drop index if exists iam.pertenencia_un_orgadmin;
create unique index if not exists pertenencia_rol_un_orgadmin
  on iam.pertenencia_rol (organizacion) where rol = 'ORGADMIN';

alter table iam.pertenencia drop column if exists rol;

-- ⭐ Lo efectivo de una persona en una organización, en un solo sitio: el estado
--   por defecto por pertenecer, MÁS lo que añada cada cargo que tenga.
create or replace view iam.potestades_de_persona as
  select pe.persona, pe.organizacion, pd.potestad
    from iam.pertenencia pe
    cross join iam.por_defecto pd
   union
  select pr.persona, pr.organizacion, rp.potestad
    from iam.pertenencia_rol pr
    join iam.rol_potestad rp on rp.rol = pr.rol;

do $$
declare n int;
begin
  -- Nadie puede haber perdido su cargo en el camino.
  select count(*) into n from iam.pertenencia_rol;
  raise notice 'cargos migrados: %', n;
end $$;
