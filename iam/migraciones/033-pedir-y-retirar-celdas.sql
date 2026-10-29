-- 033 · PEDIR Y RETIRAR CELDAS: dos potestades, y el estado a la vista del aprovisionador
--
-- ══════════════════════════════════════════════════════════════════════════
-- La 0025 E6. `POST /organizaciones/{org}/celdas` y `POST /celdas/{c}/retirar`
-- son actos de la CUENTA —pedir una serverless mas, retirarla—, y como todo
-- acto de la cuenta se ejercen por potestad (014), no por «ser miembro». Hoy
-- las tiene `ORGADMIN` y nadie mas: repartir capacidad se decide en la cuenta,
-- y un `USERADMIN` invita personas, no pide clusters.
--
-- Y `iam.celda_de` gana `estado` AL FINAL (`create or replace view` no admite
-- cambiar columnas, solo añadir). Es lo que el aprovisionador lee para saber
-- que una celda `retirada` se DESMONTA en vez de converger. La 023 le nego
-- `estado` DE LA ORGANIZACION a proposito («quien esta suspendido no hace falta
-- para crear una clave»); el de la celda es tecnico —¿existe o se retiro?— y
-- sin el, retirar seria un acto de operador otra vez.
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md`, E6
-- ══════════════════════════════════════════════════════════════════════════

insert into iam.potestad (nombre, que_hace, ejercida) values
  ('celda:crear',   'pedir una celda mas —una serverless— para la organizacion', true),
  ('celda:retirar', 'retirar una celda que no es la de casa; el reconciliador la desmonta', true)
on conflict (nombre) do nothing;

insert into iam.rol_potestad (rol, potestad) values
  ('ORGADMIN', 'celda:crear'),
  ('ORGADMIN', 'celda:retirar')
on conflict do nothing;

create or replace view iam.celda_de as
  select o.nombre as organizacion, c.nombre as celda, c.tier, c.puerta, c.cluster, c.arbol, c.entrada,
         c.estado
    from iam.celda c
    join iam.organizacion o on o.id = c.organizacion;
grant select on iam.celda_de to ore_aprovisionador;
