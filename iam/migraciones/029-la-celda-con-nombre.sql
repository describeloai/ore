-- 029 · LA CELDA CON NOMBRE — ensanchar, sin quitar nada
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ MEDIDO ANTES (`medida-la-celda-con-nombre.py`): la celda no tenía nombre
--   propio —usaba el de la organización— y `arbol` y `entrada` vivían en
--   `iam.organizacion` cuando son de la celda. Una organización no podía tener
--   dos serverless por una confusión de nombres, no por una decisión.
--
-- ── ⛔ ESTO SOLO AÑADE. Es la R1 de la 0025: ensanchar y recortar, nunca mover ──
--
--   `arbol` y `entrada` se DUPLICAN en la celda y se rellenan desde la
--   organización. Las columnas viejas se quedan, con su `grant` y sus lectores;
--   cada lector cambia en su propio commit (E2), la guarda los cuenta, y sólo con
--   cero se recortan (030). Mientras tanto `iam.discrepancias` dice si las dos
--   verdades dejan de coincidir — y la guarda la lee en cada puerta.
--
-- ── Lo que cambia de significado, dicho ───────────────────────────────────
--
--   `iam.celda.nombre` era el CLUSTER (`ore-mesh`). Pasa a ser el nombre de la
--   CELDA —de él se deriva todo lo técnico: `t-<celda>`, `<celda>.ore.paladio.io`,
--   `ore-cofre-<celda>`…— y el clúster se va a `cluster`. Las celdas que ya hay se
--   llaman como su organización (`demo`, `prueba`), que es exactamente lo que ya
--   se llamaba todo lo suyo: ni un manifiesto, ni una cuenta, ni un DNS cambian.
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md`, E1
-- ══════════════════════════════════════════════════════════════════════════

-- ① el clúster, a su columna; el nombre, a la celda
alter table iam.celda add column if not exists cluster text;
update iam.celda set cluster = nombre where cluster is null;
alter table iam.celda alter column cluster set not null;

update iam.celda c set nombre = o.nombre
  from iam.organizacion o
 where o.id = c.organizacion and c.nombre = c.cluster;

-- El mismo alfabeto que el primer segmento del árbol (017): acaba en un
-- namespace, en un host y en una cuenta de Google.
do $$
begin
  if not exists (select 1 from pg_constraint where conname = 'celda_nombre_es_etiqueta') then
    alter table iam.celda add constraint celda_nombre_es_etiqueta
      check (nombre ~ '^[a-z0-9][a-z0-9-]{0,38}$');
  end if;
end $$;

-- ② el árbol y la entrada, duplicados desde la organización
alter table iam.celda add column if not exists arbol   text;
alter table iam.celda add column if not exists entrada text;
update iam.celda c set arbol = o.arbol, entrada = o.entrada
  from iam.organizacion o
 where o.id = c.organizacion and (c.arbol is null or c.entrada is null);
alter table iam.celda alter column arbol   set not null;
alter table iam.celda alter column entrada set not null;

comment on column iam.celda.nombre is
  'El nombre de la CELDA, del que se deriva todo lo tecnico (t-<n>, <n>.ore.paladio.io, ore-cofre-<n>). 0025-2.';
comment on column iam.celda.cluster is
  'En que cluster corre (ore-mesh). Carretera.';
comment on column iam.celda.arbol is
  'Donde vive su arbol, <propietario>/<repositorio>, en SU forja. Vino de iam.organizacion.arbol (029).';
comment on column iam.celda.entrada is
  'El host por el que se llega a su arbol. Vino de iam.organizacion.entrada (029).';

-- ③ la doble verdad, vigilada: mientras las columnas viejas existan, dicen lo mismo
create or replace view iam.discrepancias as
  select o.nombre as organizacion, 'arbol: ' || o.arbol || ' vs ' || c.arbol as que
    from iam.organizacion o join iam.celda c on c.organizacion = o.id
   where o.arbol is distinct from c.arbol
  union all
  select o.nombre, 'entrada: ' || o.entrada || ' vs ' || c.entrada
    from iam.organizacion o join iam.celda c on c.organizacion = o.id
   where o.entrada is distinct from c.entrada
  union all
  select o.nombre, 'sin celda'
    from iam.organizacion o
   where not exists (select 1 from iam.celda c where c.organizacion = o.id);
comment on view iam.discrepancias is
  'Vacia o la etapa no pasa. Existe mientras arbol/entrada vivan en las dos tablas (029..030).';

-- ④ el secreto sabe de que celda es (0025-4). La unicidad vieja se queda hasta la 030.
--
-- ⛔ Y NO es `not null` todavia: el cofre de hoy emite sin saber su celda, y un
--   `not null` aqui lo rompería en el mismo instante de migrar — que es lo que la
--   R1 prohíbe. Hasta que el cofre escriba `celda` (E2), un disparador la pone
--   por defecto: la ÚNICA celda de la organización. Con dos celdas y sin decir
--   cuál, se niega — y eso es correcto, porque entonces no hay defecto posible.
alter table cofre.secreto add column if not exists celda text references iam.celda(id);
update cofre.secreto s set celda = c.id
  from iam.celda c
 where c.organizacion = s.organizacion and s.celda is null;
comment on column cofre.secreto.celda is
  'De que celda es: donde vive su material (t-<celda>-cofre-<nombre>). Quien puede usarlo sigue siendo de la organizacion.';

create or replace function cofre.celda_por_defecto() returns trigger
language plpgsql as $$
declare cuantas int; unica text;
begin
  if new.celda is null then
    select count(*), min(id) into cuantas, unica from iam.celda where organizacion = new.organizacion;
    if cuantas = 1 then
      new.celda := unica;
    else
      raise exception 'el secreto % no dice de que celda es y la organizacion tiene % celdas', new.nombre, cuantas;
    end if;
  end if;
  return new;
end $$;
drop trigger if exists secreto_celda_por_defecto on cofre.secreto;
create trigger secreto_celda_por_defecto
  before insert on cofre.secreto
  for each row execute function cofre.celda_por_defecto();

-- ⑤ quien lee la celda: el cofre (para saber su id al emitir) y el aprovisionador (por nombres)
grant select on iam.celda to ore_cofre;
create or replace view iam.celda_de as
  -- ⚠️ Las columnas nuevas AL FINAL: `create or replace view` no admite cambiar
  --   las que ya tenia (organizacion, celda, tier, puerta), solo añadir detras.
  select o.nombre as organizacion, c.nombre as celda, c.tier, c.puerta, c.cluster, c.arbol, c.entrada
    from iam.celda c
    join iam.organizacion o on o.id = c.organizacion;
grant select on iam.celda_de to ore_aprovisionador;
