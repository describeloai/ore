-- 015 · INVITAR SIN CARGO — la otra mitad de la `014`
--
-- ⛔⛔ Y NO SE ARREGLA EDITANDO LA `014`. Es la segunda vez que esa regla se
--   cobra, y funciona igual de bien la segunda: *«lo que corrió y lo que dice
--   el fichero dejan de ser lo mismo»*. Da igual que la `014` sólo haya corrido
--   contra una base efímera de CI — la regla existe **para no tener que razonar
--   sobre quién la ha aplicado ya**. En cuanto hay que pensarlo, ya no protege.
--
-- ── Qué faltaba ────────────────────────────────────────────────────────────
--
--   La `014` hizo nulable `pertenencia.rol` —`null` es *pertenece y nada más*—
--   y se dejó `invitacion.rol`, que sigue siendo `not null` desde `006`.
--
--   ⇒ Se podía ENTRAR sin cargo pero no se podía INVITAR sin cargo, que es el
--     único camino por el que se entra. La mitad nulable no servía de nada.
--
--   El síntoma, medido en CI: `{"error":"db error"}` — un 422 sin una sola
--   pista, que es su propio hallazgo aparte.

-- ⚠️ Desde que la `014` se corrigio, esto ya lo hace ella. Se queda porque es
--   idempotente y porque su cabecera es el registro de POR QUE hizo falta: la
--   mitad nulable que no servia de nada. Borrarlo dejaria la historia coja.
alter table iam.invitacion alter column rol drop not null;

comment on column iam.invitacion.rol is
  'NULL = se invita a pertenecer y nada mas. Los roles AÑADEN sobre el estado por defecto.';

-- ⛔ Y la guarda que esto NO relaja: invitar CON rol sigue exigiendo dos
--   potestades —`invitacion:emitir` y `rol:conceder`— porque conceder aplazado
--   sigue siendo conceder. Sin rol sólo hace falta la primera, que es
--   exactamente la diferencia entre dar de alta y repartir poder.
