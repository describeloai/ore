-- 005 · LA PERTENENCIA — quién está dentro, y con qué papel
--
-- ⭐ Una persona, una organización, **un papel**. No una lista de papeles: dos
--   papeles a la vez obligan a decidir cuál gana, y esa decisión no la toma
--   nadie explícitamente — la toma el orden en que se leyeron las filas.
--
-- ⛔ La clave primaria es `(persona, organizacion)` y eso lo hace imposible por
--   construcción, que es mejor que comprobarlo.
--
-- ── Cómo se llega aquí ──────────────────────────────────────────────
--
--   Por una invitación redimida (`006`), y sólo por ahí. Es la idea de
--   `021-la-invitacion.sql`: la pertenencia NO sale del IdP. Si saliera,
--   este plano necesitaría una credencial de administración del emisor —y no
--   tenerla es una propiedad, no una carencia.

create table if not exists iam.pertenencia (
  persona      text        not null references iam.persona(id)      on delete cascade,
  organizacion text        not null references iam.organizacion(id) on delete cascade,
  papel        text        not null references iam.papel(nombre),
  desde        timestamptz not null default now(),
  primary key (persona, organizacion)
);

create index if not exists pertenencia_por_organizacion
  on iam.pertenencia (organizacion, papel);

-- ⛔ UN dueño por organización, y la base lo sostiene. Un `check` no puede
--   mirar otras filas; un índice único parcial sí.
create unique index if not exists pertenencia_un_dueno
  on iam.pertenencia (organizacion) where papel = 'dueno';
