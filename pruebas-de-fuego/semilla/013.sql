-- Datos con la forma que tiene el esquema DESPUÉS de la `013` y antes de la
-- `014` — que es exactamente el estado en el que estaba el clúster cuando la
-- `014` reventó dos veces seguidas.
--
-- ── Qué hay aquí y por qué cada fila ───────────────────────────────────────
--
--   una persona y una organización         para colgar lo demás
--   una pertenencia con `dueno`            la `014` la remapea a `ORGADMIN`, y
--                                          eso exige que el rol nuevo EXISTA
--                                          antes ⇒ caza el fallo de orden ②
--   una pertenencia con `miembro`          la `014` la pone a `null` ⇒ exige
--                                          que la columna sea nulable ya
--   una invitación con `miembro`           lo mismo, en la OTRA tabla — que es
--                                          justo la que se olvidó ⇒ fallo ①
--   una invitación con `administrador`     el otro remapeo
--
-- ⛔ Y los nombres son los de ANTES: `rol` se llama `rol` desde la `012`, y los
--   valores son los de la escalera —`dueno`, `miembro`, `administrador`—, que
--   es lo que había escrito de verdad. Poner aquí los nombres nuevos haría que
--   la prueba pasara sin probar nada.

insert into iam.persona (id, emisor, sub, correo, nombre) values
  ('per_semilla0000000000000000000001', 'https://login.paladio.io/realms/rubix',
   'persona:semilla-uno', 'uno@semilla.invalido', 'Uno (semilla)'),
  ('per_semilla0000000000000000000002', 'https://login.paladio.io/realms/rubix',
   'persona:semilla-dos', 'dos@semilla.invalido', null)
on conflict do nothing;

insert into iam.organizacion (id, nombre, estado) values
  ('org_semilla0000000000000000000001', 'semilla', 'activa')
on conflict do nothing;

insert into iam.pertenencia (persona, organizacion, rol) values
  ('per_semilla0000000000000000000001', 'org_semilla0000000000000000000001', 'dueno'),
  ('per_semilla0000000000000000000002', 'org_semilla0000000000000000000001', 'miembro')
on conflict do nothing;

insert into iam.invitacion
  (id, organizacion, correo, rol, invito, caduca_en, vale_resumen) values
  ('inv_semilla0000000000000000000001', 'org_semilla0000000000000000000001',
   'tres@semilla.invalido', 'miembro', 'per_semilla0000000000000000000001',
   now() + interval '7 days',
   '0000000000000000000000000000000000000000000000000000000000000001'),
  ('inv_semilla0000000000000000000002', 'org_semilla0000000000000000000001',
   'cuatro@semilla.invalido', 'administrador', 'per_semilla0000000000000000000001',
   now() + interval '7 days',
   '0000000000000000000000000000000000000000000000000000000000000002')
on conflict do nothing;
