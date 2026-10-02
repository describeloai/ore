-- 048 · EL HANDLE DE LA PERSONA — `user:<handle>`, para que quien crea sea el dueño
--
-- ── Por qué ────────────────────────────────────────────────────────────────
--
--   0027 (el dueño es quien lo crea): lo que se crea en la plataforma —una base, un schema,
--   un origen, una vista, una colección— nace con `owner: user:<quien lo crea>`.
--   El `owner` de OOS es un handle (`OOS2009`: una letra y luego minúsculas,
--   dígitos y `-`), y lo único que el servidor sabía de una persona era su
--   `sub`: en `rubix` un UUID, que puede empezar por dígito y no dice quién es.
--
--   ⇒ Cada persona tiene aquí su handle: **sale del nombre de usuario que eligió
--     al registrarse** (`preferred_username`), con la regla de
--     `ore_core::pertenencia::handle_de`, y se le asigna UNA vez —la primera vez
--     que `ore-iam` ve su token—. Las celdas lo preguntan por el puente
--     (`POST /access/v1/quien`).
--
-- ── ⭐ Y no cambia ──────────────────────────────────────────────────────────
--
--   Es lo que queda escrito en los `owner` del árbol. Si cambiara con el correo
--   o con el usuario del IdP, los documentos dirían un nombre que ya no es de
--   nadie. Por eso no se refresca como `nombre` (la 013): se asigna si es NULL y
--   no se vuelve a tocar.
--
-- ── NULABLE, a propósito ────────────────────────────────────────────────────
--
--   Las personas que ya existen no tienen el token a mano en una migración, así
--   que nacen sin él y lo reciben la próxima vez que entren. Rellenarlas aquí con
--   el correo daría un handle que la persona no eligió, para siempre.
--
-- ⛔ Único entre TODAS las personas, no por organización: una persona es la
--   misma en todas las suyas (la 021), y su handle también.

alter table iam.persona add column if not exists handle text;

create unique index if not exists persona_handle_unico
  on iam.persona (handle) where handle is not null;

alter table iam.persona drop constraint if exists persona_handle_forma;
alter table iam.persona add constraint persona_handle_forma
  check (handle is null or handle ~ '^[a-z][a-z0-9-]*$');

comment on column iam.persona.handle is
  'El handle de `user:<handle>` (OOS2009). Sale del nombre de usuario que eligio al registrarse; se asigna una vez y no cambia. NULL = aun no ha entrado desde la 048.';
