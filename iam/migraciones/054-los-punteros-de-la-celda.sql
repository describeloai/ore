-- 054 · LOS PUNTEROS DE LA CELDA — dónde viven: en git o en el catálogo transaccional
--
-- ══════════════════════════════════════════════════════════════════════════
-- La 0079 ·5g·1b. Una celda guarda los punteros de sus datasets en `ficheros`
-- (el árbol de git, lo de siempre) o en `catalogo` (el catálogo transaccional,
-- `t_<org>` en el `Cluster catalogo`). El render del inquilino lo lee
-- (`gen-inquilino.py`): con `catalogo`, `ore-serve` arranca con
-- `--punteros catalogo` y los Jobs con `ORE_CATALOGO_ESQUEMA`.
--
-- ── ⭐ Las que hay, en `ficheros`; las que nazcan, en `catalogo` ───────────
--
--   La columna entra con `ficheros` por defecto, y así se rellenan las filas
--   que ya existen: nada cambia para ellas al aplicar esto. Después el defecto
--   pasa a `catalogo`: `ore-iam fundar` no nombra la columna, así que una celda
--   nueva nace en el catálogo sin tocar su código. Mudar una celda existente
--   (·5g·2) es un `update`, y volver atrás, otro.
--
-- 📎 `docs/products/0079-transactional-assets-catalog.md`, §19.6
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.celda add column if not exists punteros text not null default 'ficheros';

do $$ begin
  if not exists (select 1 from pg_constraint where conname = 'celda_punteros_conocidos') then
    alter table iam.celda add constraint celda_punteros_conocidos
      check (punteros in ('ficheros', 'catalogo'));
  end if;
end $$;

alter table iam.celda alter column punteros set default 'catalogo';

comment on column iam.celda.punteros is
  'dónde viven los punteros de los datasets: ficheros (git) o catalogo (0079); las nuevas, catalogo';

-- `create or replace view` sólo admite añadir columnas, al final (como la 033).
create or replace view iam.celda_de as
  select o.nombre as organizacion, c.nombre as celda, c.tier, c.puerta, c.cluster, c.arbol, c.entrada,
         c.estado, c.punteros
    from iam.celda c
    join iam.organizacion o on o.id = c.organizacion;
grant select on iam.celda_de to ore_aprovisionador;
