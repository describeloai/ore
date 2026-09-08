-- 004 · LA ORGANIZACIÓN — la del producto, no la del IdP
--
-- ⚠️ Keycloak 26 tiene *Organizations* y el realm las trae activadas. **No son
--   éstas.** Las suyas existen para estampar un claim en el token; ésta es la
--   entidad de negocio: la que tiene estado, la que se suspende y la que
--   alguien posee.
--
-- ⛔ Y no se sincronizan. Sincronizar dos censos de organizaciones es tener
--   dos y creerse que hay uno; el día que divergen, nadie sabe cuál manda. El
--   claim del token dice **a qué organización dice pertenecer quien pregunta**;
--   esta tabla dice **cuáles existen**. La pertenencia la decide `005`.
--
-- ── El estado, y por qué `retirada` no es `delete` ──────────────────
--
--   Borrar una organización se llevaría por delante las huellas de lo que pasó
--   dentro, que es justo lo que no se puede perder. `retirada` es una lápida:
--   ya no admite a nadie y sigue pudiendo contestar qué ocurrió.

create table if not exists iam.organizacion (
  id        text        primary key,
  nombre    text        not null,
  estado    text        not null default 'activa'
              check (estado in ('activa', 'suspendida', 'retirada')),
  creada_en timestamptz not null default now(),
  creada_por text       references iam.persona(id)
);

comment on column iam.organizacion.estado is
  '`retirada` es una lapida: no admite a nadie y conserva sus huellas.';
