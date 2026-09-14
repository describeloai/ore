-- 031 · RECORTAR (2 de 2): la organización pierde lo que era de la celda
--
-- ══════════════════════════════════════════════════════════════════════════
-- La segunda mitad de la 030. Se aplica cuando el `fundar` que ya no escribe
-- `organizacion.arbol/entrada` está desplegado —lo dice la guarda: cero
-- lectores y cero escritores— y `iam.discrepancias` sigue vacía.
--
-- ⛔ Se niega si alguna organización lleva un arbol distinto del de su celda
--   de casa: sería perder un dato. Y se niega si alguna lo lleva IGUAL pero la
--   celda no existe — no debería pasar (029 sembró todas), y si pasa hay que
--   mirarlo, no borrarlo.
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md`, E3
-- ══════════════════════════════════════════════════════════════════════════

do $$
declare cuantas int; lista text;
begin
  select count(*), string_agg(organizacion || ': ' || que, '; ') into cuantas, lista from iam.discrepancias;
  if cuantas > 0 then
    raise exception using message = format('iam.discrepancias tiene %s filas y no se recorta nada: %s', cuantas, lista);
  end if;
  if exists (select 1 from iam.organizacion o where o.arbol is not null
              and not exists (select 1 from iam.celda c where c.organizacion = o.id and c.arbol = o.arbol)) then
    raise exception 'hay organizaciones con un arbol que ninguna celda suya tiene: mirarlas antes de borrar';
  end if;
end $$;

drop view if exists iam.discrepancias;
revoke select (arbol, entrada) on iam.organizacion from ore_aprovisionador;
alter table iam.organizacion drop column if exists arbol;
alter table iam.organizacion drop column if exists entrada;
comment on table iam.organizacion is
  'La CUENTA: personas, pertenencia, roles, llave, agente, concesiones. Donde vive un arbol es de iam.celda. 0025.';
