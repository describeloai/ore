-- 045 · LAS PRIMERAS POTESTADES DEL ÁRBOL: P2 (0044 B.7, 0047 A5)
--
-- Hasta hoy todas las potestades eran del plano de control (0047 § «Lo que hay»
-- 1). Éstas son las primeras que pregunta un módulo del plano de datos, por el
-- puente (`ore-acceso`), y son las tres de P2:
--
--   propuesta:fusionar-sin-revision  fusionar en una `main` protegida sin la
--                                    aprobación de otra persona —la suya
--                                    también—, y queda dicho en el merge y en la
--                                    huella ANTES de fusionar (0044 B.7)
--   rama:proteger                    cambiar la política de `main`: protegerla o
--                                    proponer dejarla libre (0044 B.4)
--   fuente:crear                     dar de alta un origen: va a `main` sin
--                                    mediación, y se gobierna con potestad, no con
--                                    revisión (0044 B.2, B.7)
--
-- ── A quién ─────────────────────────────────────────────────────────────────
--
-- ORGADMIN tiene todas, como siempre: otorgar exige que lo otorgado esté
-- contenido en lo propio, y el dueño no puede quedarse fuera de nada.
-- ACCOUNTADMIN, las tres: administra la cuenta.
-- `fuente:crear`, además, a SECURITYADMIN: los tres roles que emiten secretos
-- (`secreto:emitir`, 018/037), que es lo que el alta de un origen con credencial
-- ya exigía en el custodio. Quien podía guardar la credencial puede dar de alta
-- el origen; quien no, tampoco podía antes.
--
-- ⚠️ Medido el 2026-09-29: las únicas personas con cargo son las tres ORGADMIN
--   (una por organización). Nadie pierde nada de lo que hacía.
insert into iam.potestad (nombre, que_hace, ejercida) values
  ('propuesta:fusionar-sin-revision', 'fusionar en una main protegida sin la aprobacion de otra persona. Queda dicho en el merge y en la huella', true),
  ('rama:proteger',                   'proteger main, o proponer dejarla libre', true),
  ('fuente:crear',                    'dar de alta un origen de datos', true)
on conflict (nombre) do nothing;

insert into iam.rol_potestad (rol, potestad) values
  ('ORGADMIN',      'propuesta:fusionar-sin-revision'),
  ('ACCOUNTADMIN',  'propuesta:fusionar-sin-revision'),
  ('ORGADMIN',      'rama:proteger'),
  ('ACCOUNTADMIN',  'rama:proteger'),
  ('ORGADMIN',      'fuente:crear'),
  ('ACCOUNTADMIN',  'fuente:crear'),
  ('SECURITYADMIN', 'fuente:crear')
on conflict do nothing;
