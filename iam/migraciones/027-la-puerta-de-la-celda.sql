-- 027 · LA PUERTA DE LA CELDA
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ MEDIDO ANTES DE ESCRIBIR (`medida-la-puerta-de-la-celda.py`)
--
--   La `022` puso la ENTRADA de una organización en su fila —`demo.ore.paladio.io`—
--   y dijo a propósito que la IP, el certificado y el `Gateway` NO van ahí: son
--   carretera. La 0024-⑥ dice que cada celda tiene su puerta y que el plano de
--   control escribe el DNS. Entre las dos había un hueco medible:
--
--     iam.celda            id, organizacion, nombre, tier, proveedor, region, estado
--                          → la celda NO dice su puerta
--     *.ore.paladio.io     → 136.69.102.80, la IP de la ÚNICA celda, por un comodín
--     con una segunda celda  la entrada de `acme` llegaría a la celda de `demo`,
--                          el Gateway contestaría 404 y la consola diría «no
--                          responde» señalando al clúster equivocado
--
-- ── ⭐ Un NOMBRE, y no una IP — la misma figura por sexta vez ─────────────
--
--     `50-jwks.yaml`  `EMISOR` es quién firma · `DIRECCION` dónde se le busca
--     `017`           el ÁRBOL se llama `<propietario>/<repositorio>`
--     `019`           la LLAVE se llama `<llavero>/<clave>`
--     `022`           la ENTRADA se llama `demo.ore.paladio.io`
--     `027`           la PUERTA de la celda se llama `ore-mesh.ore.paladio.io`
--
--   La IP que hay detrás es del balanceador y cambia sin que nadie mienta. Lo
--   que esta fila fija es la RELACIÓN: la entrada de una organización tiene que
--   resolver a la puerta de su celda. Es un `CNAME` en el mundo, y el mundo
--   converge hacia la fila — la escribe el aprovisionador cuando la zona es
--   nuestra, y dice qué registro falta cuando no lo es.
--
-- ── ⚠️ Hoy la relación la cumple un comodín, y se dice ────────────────────
--
--   `*.ore.paladio.io → 136.69.102.80` hace que TODA entrada resuelva a la IP
--   de `ore-mesh`. Con una celda es lo mismo que un `CNAME` por organización;
--   con dos deja de serlo. El comodín no se toca aquí: es la carretera del
--   compartido, y la 0024 lo nombra.
--
-- 📎 `docs/decisions/0024-donde-corre-el-inquilino.md` ⑥
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.celda add column if not exists puerta text;

-- Las celdas que ya había son `ore-mesh`, y su puerta es la del Gateway
-- `ore-system/puerta`. Se siembra ANTES del `not null`: la columna nace
-- entera o no nace.
update iam.celda set puerta = 'ore-mesh.ore.paladio.io'
 where puerta is null and nombre = 'ore-mesh';

alter table iam.celda alter column puerta set not null;

-- El mismo alfabeto que `entrada`: RFC 1123, sin esquema ni puerto ni camino.
-- Un `https://` aquí sería carretera dentro de la identidad, otra vez.
do $$
begin
  if not exists (select 1 from pg_constraint where conname = 'celda_puerta_es_host') then
    alter table iam.celda add constraint celda_puerta_es_host
      check (puerta ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)+$');
  end if;
end $$;

comment on column iam.celda.puerta is
  'El NOMBRE de la puerta de la celda. La entrada de la organizacion (022) debe '
  'resolver a el; la IP que hay detras es carretera. 0024-6.';

-- ── ⭐ Y EL APROVISIONADOR LA LEE, sin ver `id` ────────────────────────────
--
--   El paso que coteja que la entrada resuelva a la puerta lo corre el
--   aprovisionador, y su papel (`023`) lee cuatro columnas de `iam.organizacion`
--   y ningún `id` — a propósito. Un `join` por `organizacion` le exigiría el
--   `id`, así que se le da la RELACIÓN ya hecha: una vista por NOMBRES, con la
--   que sigue sin poder correlacionar nada que no deba.
create or replace view iam.celda_de as
  select o.nombre as organizacion, c.nombre as celda, c.tier, c.puerta
    from iam.celda c
    join iam.organizacion o on o.id = c.organizacion;
comment on view iam.celda_de is
  'La celda de cada organizacion, por NOMBRES. Es lo que el aprovisionador lee para converger el DNS.';
grant select on iam.celda_de to ore_aprovisionador;
