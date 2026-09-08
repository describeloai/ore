-- 008 · LA HUELLA — cada acto privilegiado deja rastro, incluido MIRAR
--
-- ⭐⭐ La idea es de `020` y `022` de la plataforma, y su frase se toma entera:
--
--     > «un acto privilegiado sin rastro no es un control»
--
--   Y la más fina de las dos es la `022`: **mirar lo que hicieron los demás es
--   la potestad más barata del catálogo y también la más íntima**, así que leer
--   la huella deja huella.
--
-- ── Quién y con qué ─────────────────────────────────────────────────
--
--   `quien` es la persona; `agente` es lo que actuó por ella. Es `sub` + `act`
--   de RFC 8693, la misma forma que `ore-serve` ya escribe en el autor y el
--   committer de cada commit de la forja. Fundirlos no sirve para contestar
--   ninguna de las dos preguntas.
--
-- ⛔ Esta tabla NO se borra ni se actualiza. Sin `update` y sin `delete` en el
--   papel de la aplicación: una huella que se puede editar es un borrador.

create table if not exists iam.huella (
  id        bigserial   primary key,
  cuando    timestamptz not null default now(),
  quien     text        not null,
  agente    text,
  operacion text        not null,
  sobre     text,
  detalle   jsonb
);

create index if not exists huella_por_cuando    on iam.huella (cuando desc);
create index if not exists huella_por_quien     on iam.huella (quien, cuando desc);
create index if not exists huella_por_operacion on iam.huella (operacion, cuando desc);

comment on table iam.huella is
  'Solo se inserta. Una huella que se puede editar es un borrador.';
