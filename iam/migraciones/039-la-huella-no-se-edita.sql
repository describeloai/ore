-- 039 · LA HUELLA NO SE EDITA, Y AHORA ES VERDAD
--
-- La `008` lo prometio en su cabecera —«sin `update` y sin `delete` en el papel
-- de la aplicacion: una huella que se puede editar es un borrador»— y la `020`
-- lo deshizo sin querer, dos lineas despues de nombrarla:
--
--     grant select, insert, update, delete on all tables in schema iam to ore_iam;
--
-- `all tables` incluye `iam.huella`. Medido el 2026-09-28 (0047 M6): `ore_iam`
-- tenia `UPDATE` y `DELETE` sobre ella, y ni un trigger ni una regla lo
-- impedian. Ningun codigo la edita —medido tambien—, asi que no se rompe nada:
-- se cierra lo que estaba abierto.
--
-- ⭐ DOS CAPAS, y la segunda es la que importa.
--
--   ① los permisos: se quitan `update`, `delete` y `truncate` a `ore_iam` (y a
--     `public`, por si acaso). Pero un permiso se vuelve a dar con otro
--     `grant … on all tables`, y la `020` demuestra que eso pasa sin que nadie
--     lo vea.
--   ② un trigger que niega `update`, `delete` y `truncate` A CUALQUIERA, el
--     dueño y el superusuario incluidos. Un `grant` no lo esquiva. Quitarlo
--     exige una migracion que diga `drop trigger` por su nombre: si algun dia
--     hace falta una retencion, se escribe, se revisa y queda en el historial.
--
-- ⛔ Y no se exceptua al dueño. La base es del papel del IdP, que es
--   superusuario (0047 M6): una excepcion para el dueño seria una excepcion
--   para todo lo que entra con esa credencial.

revoke update, delete, truncate on iam.huella from ore_iam;
revoke update, delete, truncate on iam.huella from public;

create or replace function iam.la_huella_no_se_edita() returns trigger
language plpgsql as $$
begin
  raise exception 'iam.huella solo admite insert (%): una huella que se puede editar es un borrador', tg_op
    using errcode = 'insufficient_privilege';
end;
$$;

comment on function iam.la_huella_no_se_edita() is
  'Niega update, delete y truncate sobre iam.huella a cualquiera (039). Quitarlo es otra migracion.';

drop trigger if exists la_huella_no_se_edita on iam.huella;
create trigger la_huella_no_se_edita
  before update or delete on iam.huella
  for each row execute function iam.la_huella_no_se_edita();

drop trigger if exists la_huella_no_se_vacia on iam.huella;
create trigger la_huella_no_se_vacia
  before truncate on iam.huella
  for each statement execute function iam.la_huella_no_se_edita();
