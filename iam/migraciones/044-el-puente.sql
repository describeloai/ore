-- 044 · EL PUENTE: LA HUELLA SABE DE QUÉ ORGANIZACIÓN ES, LA CELDA DICE QUIÉN ES,
--       Y LAS DECISIONES SE RECUERDAN UN DÍA (0047 A2.1)
--
-- Lo que `ore-iam` necesita para contestar el contrato de 0047 § «El contrato».
--
-- ① LA HUELLA, CON ORGANIZACIÓN Y CELDA. M6 midió que la organización no era
--   columna (sólo 206 filas la llevaban, dentro de `detalle`) y que ningún índice
--   servía «la actividad de mi organización». Las filas VIEJAS se quedan sin
--   ellas: la `039` no deja editar la huella a nadie, y rellenarlas sería
--   escribir hoy lo que se hizo ayer.
alter table iam.huella add column if not exists organizacion text;
alter table iam.huella add column if not exists celda text;
create index if not exists huella_por_organizacion
  on iam.huella (organizacion, cuando desc)
  where organizacion is not null;

-- ② LA IDENTIDAD DE CADA CELDA (M3). Una celda se presenta ante `ore-iam` con el
--   token de Workload Identity de su `ore-serve`, y de él sólo cuenta
--   `(emisor, sub)`: un correo se puede volver a crear tras borrado, un `sub` no.
--   La escribe el aprovisionador, que es quien crea la cuenta; no se deduce del
--   nombre. Las dos o ninguna, y una identidad no la comparten dos celdas vivas.
alter table iam.celda add column if not exists identidad_emisor text;
alter table iam.celda add column if not exists identidad_sub text;
alter table iam.celda drop constraint if exists celda_identidad_entera;
alter table iam.celda add constraint celda_identidad_entera
  check ((identidad_emisor is null) = (identidad_sub is null));
create unique index if not exists celda_por_identidad
  on iam.celda (identidad_emisor, identidad_sub)
  where identidad_sub is not null and estado <> 'retirada';

-- ③ LAS DECISIONES DE ESCRITURA, UN DÍA. Un reintento de `hizo` llega sin el
--   token de la persona —vive 300 s—, y `ore-iam` toma el sujeto de la decisión
--   que él mismo tomó (0047 § «Cuándo `hizo` va antes, y cuándo después»). No es
--   la huella: se poda, y por eso vive aparte. M2 Q3: decenas de filas al día.
create table if not exists iam.decision (
  id            text        primary key,
  cuando        timestamptz not null default now(),
  quien         text        not null,
  agente        text,
  organizacion  text        not null,
  celda         text        not null,
  accion        text        not null,
  recurso_tipo  text        not null,
  recurso_id    text        not null,
  decision      boolean     not null
);
create index if not exists decision_por_cuando on iam.decision (cuando);
grant select, insert, delete on iam.decision to ore_iam;
