-- 013 · EL NOMBRE — porque una pantalla de personas tiene que decir QUIÉN
--
-- ── Por qué esto no estaba, y por qué entra ahora ──────────────────────────
--
--   `003` guarda `(emisor, sub)` y el correo, y nada más. Es coherente con la
--   regla del esquema —*«personas y permisos, nada más»*— y aguantó mientras
--   nadie tuvo que PINTAR una lista.
--
--   La consola pide un `Member` con `name`, y su propio comentario dice para
--   qué: *«el IRI OPACO es la identidad real — el nombre es decoración
--   legible»*. Sin él, la pantalla de personas enseña identificadores opacos y
--   correos, que es exactamente lo que nadie reconoce.
--
-- ── ⛔ Y NO se pide al IdP ─────────────────────────────────────────────────
--
--   La alternativa obvia —preguntarle a Keycloak quiénes son— exige
--   `manage-realm`, **el realm entero**, medido en 403 el 2026-08-28. No tener
--   esa credencial es una propiedad de este plano, no una carencia: la
--   pertenencia sale de la invitación, no del emisor.
--
--   ⇒ El nombre llega por donde llega todo lo demás: **en el token**, cada vez
--     que la persona entra. Es la misma figura que `subjectResolver`, y su nota
--     vale entera: se refresca *«sólo si cambió: esto corre en cada petición, y
--     escribir lo mismo un millón de veces al día es carga inútil»*.
--
-- ── ⚠️ Lo que esto mete aquí, dicho ────────────────────────────────────────
--
--   Un nombre propio es un dato personal, y este esquema tenía sólo dos
--   —el correo y el `sub`—. Ahora tiene tres. La regla no cambia: sigue sin
--   entrar ni un dato del cliente ni un documento de la ontología. Pero la
--   fila de una persona es ahora más borrable que antes, y el día que haya
--   supresión, esta columna es de las que se van.
--
-- ── ⭐ NULABLE, y no es descuido ───────────────────────────────────────────
--
--   `null` significa **el token no lo dijo**, que no es lo mismo que «no
--   tiene». La consola ya distingue los dos casos con su campo `conocido`, y
--   una cadena vacía por defecto los fundiría.

alter table iam.persona add column if not exists nombre text;

comment on column iam.persona.nombre is
  'Lo que el emisor afirma que se llama. NULL = el token no lo dijo. Se refresca al entrar, solo si cambio.';
