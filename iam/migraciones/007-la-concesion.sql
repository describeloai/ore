-- 007 · LA CONCESIÓN — quién puede qué, sobre qué recurso
--
-- ⭐ El papel es sobre un RECURSO. Es la idea de `005`/`006`/`019` de la
--   plataforma, y la razón por la que el realm tiene **cero roles**: Keycloak
--   dice quién eres; esto dice qué puedes sobre qué. Mezclarlos obliga a
--   pedirle al IdP permiso para cambiar un permiso.
--
-- ── ⛔⛔ LA FRONTERA CON EL GOBIERNO DEL FLUJO ──────────────────────
--
--   ORE ya gobierna **qué puede fluir hasta dónde**: el retículo, los conductos
--   y `OOS4xxx`. Esto gobierna **quién alcanza qué superficie**. Y el orden
--   entre los dos está decidido:
--
--     > LA CONCESIÓN PUEDE NEGAR. NO PUEDE CONCEDER POR ENCIMA DEL CONDUCTO.
--
--   Una fila de esta tabla que ensanchara lo que el retículo cerró convertiría
--   el gobierno del flujo en una sugerencia. Quien implemente el decisor tiene
--   que preguntar a los dos y quedarse con el MÍNIMO.
--
-- ⚠️ `sujeto` y no `persona`: aquí también entra un agente —un Job de ORE
--   actuando por alguien—, y eso es RFC 8693. Por eso es texto y no una clave
--   ajena a `persona`.

create table if not exists iam.concesion (
  id          text        primary key,
  sujeto      text        not null,
  recurso     text        not null,
  papel       text        not null references iam.papel(nombre),
  concedio    text        not null references iam.persona(id),
  desde       timestamptz not null default now(),
  hasta       timestamptz,
  revocada_en timestamptz,
  revoco      text        references iam.persona(id),

  check ((revocada_en is null) = (revoco is null)),
  check (hasta is null or hasta > desde)
);

create index if not exists concesion_por_sujeto  on iam.concesion (sujeto, recurso);
create index if not exists concesion_por_recurso on iam.concesion (recurso);

-- Lo que vale AHORA. Se pregunta a la vista, no a la tabla: la tabla guarda
-- tambien lo que valio, que es lo que hace auditable una revocacion.
create or replace view iam.concesion_viva as
  select * from iam.concesion
   where revocada_en is null
     and desde <= now()
     and (hasta is null or hasta > now());
