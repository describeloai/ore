-- 055 · LA CELDA QUE NACE — el aprovisionador ve si ya tuvo su primera pasada
--
-- ══════════════════════════════════════════════════════════════════════════
-- Una celda pedida (`POST /organizaciones/{org}/celdas`) nace con
-- `estado = 'activa'`, el defecto de la 025: `estado` dice si está viva o
-- retirada, no si está montada. Lo que dice «montada» es `aprovisionada` (la
-- 032), y la consola la pinta Provisioning mientras es nula.
--
-- La pasada ligera del aprovisionador (`converger-inquilinos.sh`) sólo leía
-- `estado`: saltaba la celda nueva como si estuviera al día, y nacía en la
-- pasada de la hora en punto —hasta 60 min— en vez de en ~10. Medido con
-- `temporal-prueba` el 2026-10-29.
--
-- ⇒ `iam.celda_de` gana `aprovisionada` AL FINAL (`create or replace view`
--   sólo admite añadir columnas ahí, como la 033 y la 054), y el censo trata
--   como «fundándose» a la que no la tiene.
-- ══════════════════════════════════════════════════════════════════════════

create or replace view iam.celda_de as
  select o.nombre as organizacion, c.nombre as celda, c.tier, c.puerta, c.cluster, c.arbol, c.entrada,
         c.estado, c.punteros, c.aprovisionada
    from iam.celda c
    join iam.organizacion o on o.id = c.organizacion;
grant select on iam.celda_de to ore_aprovisionador;
