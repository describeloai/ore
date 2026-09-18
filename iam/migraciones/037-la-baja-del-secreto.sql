-- 037 · LA BAJA DEL SECRETO
--
-- Medido el 2026-09-18 (`medida-los-huecos-de-la-baja.py`): en `demo`, 19
-- fuentes dadas de alta y retiradas dejaron 19 credenciales vivas en el custodio
-- (cota superior); en `victor`, 2. `DELETE /fuentes/{n}` retira la conexion del
-- arbol y lo DICE —«la credencial sigue en el custodio»— porque el cofre no
-- tenia verbo de baja. Esta migracion le da el modelo; el verbo va en
-- `ore-cofre` (`DELETE /organizaciones/{org}/secretos/{nombre}`).
--
-- ⭐ La `020` ya lo previo: `cofre.secreto.retirado_en` y `retiro` estan desde
--   el primer dia, «y no un `delete`: un secreto retirado tiene que seguir
--   contando que existio». Aqui no se toca la tabla: se le da la potestad y la
--   funcion que faltaban.
--
-- ── La potestad ────────────────────────────────────────────────────────────
--
-- `secreto:retirar` es de la ORGANIZACION, como `secreto:emitir`, y a los mismos
-- roles: quien puede crear credenciales puede darlas de baja. Y ademas —sin
-- potestad— el `owner` del secreto (quien lo emitio, `0023`): es suyo.
--
-- ⛔ `USERADMIN` NO, por lo mismo que en la `018`.
insert into iam.potestad (nombre, que_hace, ejercida) values
  ('secreto:retirar', 'dar de baja un secreto. No borra la fila: retirado_en y quien', true)
on conflict (nombre) do nothing;

insert into iam.rol_potestad (rol, potestad) values
  ('SECURITYADMIN', 'secreto:retirar'),
  ('ACCOUNTADMIN',  'secreto:retirar'),
  ('ORGADMIN',      'secreto:retirar')
on conflict do nothing;

-- Y las dos de la `018` ya se ejercen: `secreto:emitir` desde el alta de fuentes
-- (0022) y `secreto:listar` desde el cofre. La columna existe para que la
-- pantalla no ensene un permiso que no hace nada; que diga la verdad.
update iam.potestad set ejercida = true
 where nombre in ('secreto:emitir', 'secreto:listar') and ejercida = false;

-- ── Revocar las concesiones de un secreto ──────────────────────────────────
--
-- Al retirar un secreto, `owner` y `usar` sobre `secreto/<n>` dejan de tener
-- sentido y se revocan CON FECHA (la `007`: una concesion revocada sigue
-- contando). El custodio no puede escribir en `iam.concesion` —a proposito,
-- `020`— asi que va por una funcion que solo sabe revocar sobre `secreto/…`,
-- gemela de `iam.conceder_de_secreto` (`021`).
create or replace function iam.revocar_de_secreto(
  p_recurso      text,
  p_organizacion text,
  p_revoco       text
) returns integer
language plpgsql
security definer
set search_path = pg_catalog, iam
as $funcion$
declare
  n integer;
begin
  if p_recurso not like 'secreto/%' then
    raise exception
      'esta funcion solo revoca sobre secretos, y se le paso `%`', p_recurso
      using hint = 'lo demas se revoca por `DELETE /organizaciones/{org}/concesiones/{id}`';
  end if;
  update iam.concesion
     set revocada_en = now(), revoco = p_revoco
   where recurso = p_recurso and organizacion = p_organizacion
     and revocada_en is null;
  get diagnostics n = row_count;
  return n;
end;
$funcion$;

comment on function iam.revocar_de_secreto(text, text, text) is
  'La UNICA forma que tiene el custodio de revocar una concesion, y solo de secreto.';

revoke all on function iam.revocar_de_secreto(text, text, text) from public;
grant execute on function iam.revocar_de_secreto(text, text, text) to ore_cofre;
