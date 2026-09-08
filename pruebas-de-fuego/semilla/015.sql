-- Datos con la forma que tiene el esquema DESPUÉS de la `015` — el modelo de
-- potestades ya puesto, y `pertenencia.rol` todavía en su sitio.
--
-- ⭐ Es el estado desde el que la `016` migra los cargos a su tabla propia, y
--   por eso hay una fila CON cargo y otra SIN: la `016` tiene que llevarse la
--   primera y dejar la segunda como está —«pertenece y nada más»—, y esas dos
--   cosas se rompen de formas distintas.

insert into iam.persona (id, emisor, sub, correo, nombre) values
  ('per_semilla0150000000000000000001', 'https://login.paladio.io/realms/rubix',
   'persona:semilla-015-uno', 'uno@semilla.invalido', 'Uno (semilla 015)'),
  ('per_semilla0150000000000000000002', 'https://login.paladio.io/realms/rubix',
   'persona:semilla-015-dos', 'dos@semilla.invalido', null)
on conflict do nothing;

insert into iam.organizacion (id, nombre, estado) values
  ('org_semilla0150000000000000000001', 'semilla-015', 'activa')
on conflict do nothing;

insert into iam.pertenencia (persona, organizacion, rol) values
  ('per_semilla0150000000000000000001', 'org_semilla0150000000000000000001', 'ORGADMIN'),
  -- ⛔ Sin cargo. La `016` NO debe inventarle una fila en `pertenencia_rol`:
  --   la ausencia de cargo es la ausencia de fila, no un cargo vacio.
  ('per_semilla0150000000000000000002', 'org_semilla0150000000000000000001', null)
on conflict do nothing;
