-- 038 · LA BAJA DE OPERADOR
--
-- La `037` dio la baja del secreto a una persona (`retiro references
-- iam.persona`). Pero lo que quedaba en el custodio el 2026-09-18 —19
-- credenciales en `demo`, 2 en `victor`— era de fuentes que YA NO ESTAN en el
-- arbol: `DELETE /fuentes/{n}` no las alcanza, y nadie las va a pulsar desde una
-- ficha que no existe. Se retiran con un verbo de operador (`ore-cofre
-- retirar-huerfanos`), que corre EN el inquilino como `mudar` (0024-⑤).
--
-- ⛔ Y NO se atribuyen a una persona que no lo hizo. `retiro` sigue siendo la
--   persona cuando la hay; cuando es el operador, va en `retiro_agente` —el
--   verbo, tal cual— y la huella lo dice igual (`quien = operador`, `agente`).
--   Una baja atribuida al dueño de la organizacion porque «alguien tenia que
--   ser» seria la mentira exacta que la `008` existe para no contar.
alter table cofre.secreto
  add column if not exists retiro_agente text;

-- La `020` no le puso nombre: se busca por lo que dice, como hizo la `011`.
do $$
declare vieja text;
begin
  for vieja in
    select con.conname
      from pg_constraint con
      join pg_class c on c.oid = con.conrelid
      join pg_namespace n on n.oid = c.relnamespace
     where n.nspname = 'cofre' and c.relname = 'secreto' and con.contype = 'c'
       and pg_get_constraintdef(con.oid) like '%retirado_en%'
  loop
    execute format('alter table cofre.secreto drop constraint %I', vieja);
  end loop;
end $$;
alter table cofre.secreto
  add constraint secreto_retiro_check
  check ((retirado_en is null) = (retiro is null and retiro_agente is null));

comment on column cofre.secreto.retiro_agente is
  'Quien retiro cuando no fue una persona: el verbo de operador (ore-cofre retirar-huerfanos).';

-- ── Y las concesiones del secreto huerfano, revocadas por el operador ──────
--
-- La `007` exige una persona en `revoco`. Mismo criterio: cuando revoca el
-- operador, va en `revoco_agente`, y la fila sigue diciendo cuando y quien.
alter table iam.concesion
  add column if not exists revoco_agente text;

do $$
declare vieja text;
begin
  for vieja in
    select con.conname
      from pg_constraint con
      join pg_class c on c.oid = con.conrelid
      join pg_namespace n on n.oid = c.relnamespace
     where n.nspname = 'iam' and c.relname = 'concesion' and con.contype = 'c'
       and pg_get_constraintdef(con.oid) like '%revocada_en%'
  loop
    execute format('alter table iam.concesion drop constraint %I', vieja);
  end loop;
end $$;
alter table iam.concesion
  add constraint concesion_revoco_check
  check ((revocada_en is null) = (revoco is null and revoco_agente is null));

-- La gemela de la `037` para el operador: revoca con `revoco_agente`.
create or replace function iam.revocar_de_secreto_operador(
  p_recurso      text,
  p_organizacion text,
  p_agente       text
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
      'esta funcion solo revoca sobre secretos, y se le paso `%`', p_recurso;
  end if;
  if p_agente is null or p_agente = '' then
    raise exception 'el operador tiene que decir su verbo';
  end if;
  update iam.concesion
     set revocada_en = now(), revoco_agente = p_agente
   where recurso = p_recurso and organizacion = p_organizacion
     and revocada_en is null;
  get diagnostics n = row_count;
  return n;
end;
$funcion$;

revoke all on function iam.revocar_de_secreto_operador(text, text, text) from public;
grant execute on function iam.revocar_de_secreto_operador(text, text, text) to ore_cofre;
