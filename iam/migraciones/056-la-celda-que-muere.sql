-- 056 · LA CELDA QUE MUERE — retirar es la petición; `desmontada`, que ya bajó
--
-- ══════════════════════════════════════════════════════════════════════════
-- Nacer tiene su cierre: la celda pedida no tiene `aprovisionada` (la 032) y
-- el aprovisionador la reintenta hasta que la tiene (la 055). Morir no lo
-- tenía: `estado = 'retirada'` es lo que pidió el dueño, no lo que pasó. El
-- aprovisionador quitaba el enganche, decía «lo poda Flux» y nadie miraba si
-- el namespace caía. Medido:
--
--   temporalpruebav2 (2026-10-29) retirada a las 17:32, viva a las 18:16:
--     la pasada ligera no la veía y la de la hora no tenía nodo donde correr
--   corte-catalogo-prueba (2026-10-30) retirada «✓» a las 09:00, viva a las
--     09:13: el nodo spot se reemplazó y Flux se quedó en la revisión vieja
--
-- y la consola, que lista `estado <> 'retirada'`, ya no la enseñaba.
--
-- ⇒ `desmontada`: la escribe `ore-iam` cuando el aprovisionador dice
--   (`POST /celdas/{c}/desmontada`) que su namespace ya no existe. Una
--   retirada sin ella se está desmontando: la pasada ligera la repite cada
--   cinco minutos y la consola la pinta «Deleting».
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.celda add column if not exists desmontada timestamptz;
comment on column iam.celda.desmontada is
  'Cuando el aprovisionador vio caído el namespace de la celda retirada. Nulo en una retirada: se está desmontando (0025 E6).';

-- Sólo se desmonta lo retirado: retirar no tiene vuelta, así que esto no se rompe nunca.
do $$ begin
  if not exists (select 1 from pg_constraint where conname = 'celda_desmontada_es_retirada') then
    alter table iam.celda add constraint celda_desmontada_es_retirada
      check (desmontada is null or estado = 'retirada');
  end if;
end $$;

-- ⛔ Las retiradas de antes de esta migración, desmontadas: medido el 2026-10-30
--   (`kubectl get ns`) ninguna tiene namespace, sólo `t-demo` y `t-victor`. Se
--   nombran: una retirada de después, aunque llegue antes que esta migración,
--   la marca el aprovisionador cuando de verdad cae.
update iam.celda set desmontada = now()
 where estado = 'retirada' and desmontada is null
   and nombre in ('cobalt-polar-camel', 'corte-catalogo-prueba', 'prueba', 'prueba-dos',
                  'temporal-prueba', 'temporalpruebav2');

create or replace view iam.celda_de as
  select o.nombre as organizacion, c.nombre as celda, c.tier, c.puerta, c.cluster, c.arbol, c.entrada,
         c.estado, c.punteros, c.aprovisionada, c.desmontada
    from iam.celda c
    join iam.organizacion o on o.id = c.organizacion;
grant select on iam.celda_de to ore_aprovisionador;
