-- 025 · LA CELDA — dónde corre el inquilino, escrito
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⛔⛔ LO QUE LO FORZÓ
--
--   La consola tiene una vista de clústeres, y pinta CLÚSTERES DE MENTIRA. Al
--   ir a emitir el de verdad salió que no hay de dónde: `iam.organizacion`
--   dice cómo se llama el árbol, la llave y la puerta de cada organización,
--   y NO dice en qué clúster vive. El clúster era implícito — todo está en
--   `ore-mesh` — y lo implícito no se puede emitir.
--
--   ⇒ Es el hueco que la 0024 nombra: «el clúster es un hecho del plano de
--     control». Esta es la tabla.
--
-- ── ⭐⭐ POR QUÉ ES UNA TABLA Y NO CUATRO COLUMNAS EN `organizacion` ───────
--
--   Porque el árbol, la llave y la puerta son IDENTIDAD de la organización
--   —cómo se llama lo suyo— y la celda es CAMINO: dónde está. La misma
--   separación que `EMISOR`/`DIRECCION` en `017`, `019` y `022`. Una
--   organización que se mueva de compartido a dedicado cambia de celda y no
--   cambia de nombre, de árbol ni de llave.
--
--   Y porque hoy es una por organización y mañana puede no serlo: un
--   inquilino con el árbol en su casa y el modelo en la nuestra son dos
--   celdas. El índice único de abajo dice «hoy una», y quitarlo es una
--   migración, no un rediseño.
--
-- ── ⭐ `tier` ES EL DE LA 0024, Y `compartido` ES LO QUE HAY ────────────────
--
--   compartido   nuestro clúster, un namespace por inquilino  ← todo, hoy
--   dedicado     un clúster nuestro por inquilino
--   byoc         la cuenta del cliente, con un agente que tira
--
-- ── ⚠️ LO QUE ESTA TABLA NO DICE, A PROPÓSITO ──────────────────────────────
--
--   Si el clúster RESPONDE. Eso no lo sabe `iam` y no debe fingirlo: `estado`
--   es administrativo —aprovisionando, activa, suspendida, retirada—, y si
--   contesta se pregunta por el camino, al `/salud` de su `ore-serve`. La
--   0024 ④: identidad de un plano, vida del otro.
--
-- ══════════════════════════════════════════════════════════════════════════

create table if not exists iam.celda (
  id           text        primary key,
  organizacion text        not null references iam.organizacion(id) on delete cascade,
  -- Cómo se llama el clúster. `ore-mesh` hoy; el nombre del GKE dedicado o el
  -- que el cliente le ponga al suyo, mañana.
  nombre       text        not null,
  tier         text        not null
                 check (tier in ('compartido', 'dedicado', 'byoc')),
  -- `gcp`, `aws`, `azure`, `onprem`. Texto y no enum: el día que haya uno
  -- nuevo no puede exigir una migración.
  proveedor    text        not null,
  -- Como la nombra el proveedor: `europe-west1-b`, `eu-central-1`.
  region       text        not null,
  estado       text        not null default 'activa'
                 check (estado in ('aprovisionando', 'activa', 'suspendida', 'retirada')),
  creada_en    timestamptz not null default now()
);

comment on table iam.celda is
  'Dónde corre el plano del árbol de una organización. Camino, no identidad: '
  'moverse de tier cambia la celda y no cambia la organización. 0024.';
comment on column iam.celda.estado is
  'Administrativo. Si el clúster RESPONDE no lo sabe esta tabla: se pregunta al /salud de su ore-serve.';

-- ⭐ Una por organización, HOY. Es un índice y no una clave para que el día
--   que un inquilino tenga dos —árbol en su casa, modelo en la nuestra—
--   quitarlo sea una línea.
create unique index if not exists celda_una_por_organizacion
  on iam.celda (organizacion);

-- ── LA SIEMBRA ─────────────────────────────────────────────────────────────
--
-- Las organizaciones que ya existen viven donde han vivido siempre. Se
-- escribe aquí, en la migración, y no se deja para que alguien lo haga a
-- mano: es la misma figura que la `019` con la llave y la `022` con la
-- puerta — lo que ya está se rellena aquí, y lo que viene lo escribe
-- `fundar`.
--
-- ⚠️ TODA organización sin celda, y no `demo` y `prueba` por su nombre: lo
--   cierto el día de esta migración es que todo lo que existe vive en
--   `ore-mesh`, y nombrar dos instancias donde vale la clase es el error que
--   este árbol ya cometió cuatro veces. Es idempotente por el `not exists`.
--
-- ⛔ Y NO hay una comprobación de «ninguna organización sin celda» después.
--   La hubo, y era código muerto: con la siembra justo encima, no podía
--   saltar nunca. Medido en un postgres desechable. Quien garantiza que una
--   organización NUEVA tenga celda es `fundar`, que la escribe en el mismo
--   acto — y se comprueba allí, donde puede fallar.
insert into iam.celda (id, organizacion, nombre, tier, proveedor, region)
select 'cel_' || substr(md5(o.id), 1, 12), o.id, 'ore-mesh', 'compartido', 'gcp', 'europe-west1-b'
  from iam.organizacion o
 where not exists (select 1 from iam.celda c where c.organizacion = o.id);
