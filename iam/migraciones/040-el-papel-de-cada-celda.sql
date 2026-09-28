-- 040 · EL PAPEL DE CADA CELDA (0047 A7a.1)
--
-- Los tres custodios (`t-demo`, `t-prueba`, `t-victor`) entran a esta base con
-- EL MISMO login, `cofre_app`, cuya URL es un secreto de plataforma
-- (`cofre-url`). Medido el 2026-09-28 (0047 M4): el custodio de un inquilino
-- lee el censo, las concesiones y los metadatos de los secretos de TODAS las
-- organizaciones. Los valores no —viven en el Secret Manager de cada celda—,
-- pero quien es quien, y que secretos tiene cada cual, si.
--
-- ⇒ El arreglo es seguridad por fila por organizacion (A7a.4). Y para eso cada
--   custodio tiene que entrar con UN LOGIN SUYO, que diga de que celda es. Esto
--   es solo esa pieza: el papel por celda y como saber, desde dentro de la base,
--   de que organizacion es quien llama.
--
-- ⭐ Y NO CAMBIA NADA DE LO QUE CORRE. Sin politicas todavia, un `cofre_<celda>`
--   es miembro de `ore_cofre` y ve exactamente lo que `cofre_app` ve hoy. El
--   orden importa (0047 § A7a): las politicas no entran hasta que ningun
--   custodio use `cofre_app`, porque ese login no tiene celda y con ellas no
--   veria nada — y los tres caerian a la vez.

-- ── ① de que celda es cada login ───────────────────────────────────────────
create table if not exists iam.papel_de_celda (
  papel text        primary key,
  celda text        not null unique references iam.celda(id) on delete cascade,
  desde timestamptz not null default now()
);

comment on table iam.papel_de_celda is
  'El login con el que entra a la base el custodio de cada celda (0047 A7a). Lo escribe iam.dar_papel_de_celda.';

-- `ore-iam` lo lee (y su `grant` general de la `020` ya se lo daria; se dice).
grant select on iam.papel_de_celda to ore_iam;

-- ── ② de que organizacion es quien llama ───────────────────────────────────
--
-- ⭐ `session_user` y no `current_user`: el login con el que se conecto, que no
--   cambia con un `set role`. Es lo que las politicas de la A7a.4 van a mirar.
--   `null` si quien llama no es el papel de ninguna celda: y una politica que
--   compare con `null` no deja pasar nada, que es como tiene que fallar.
create or replace function iam.mi_organizacion() returns text
language sql stable security definer
set search_path = pg_catalog, iam
as $$
  select c.organizacion
    from iam.papel_de_celda p
    join iam.celda c on c.id = p.celda
   where p.papel = session_user
$$;

comment on function iam.mi_organizacion() is
  'La organizacion de la celda con cuyo papel se conecto quien llama; null si no es el de ninguna (0047 A7a).';

revoke all on function iam.mi_organizacion() from public;
grant execute on function iam.mi_organizacion() to ore_cofre, ore_iam;

-- ── ③ dar el papel de una celda ────────────────────────────────────────────
--
-- ⛔ El aprovisionador NO tiene `CREATEROLE`, y no se le da: con el podria
--   crearse cualquier papel. Lo que tiene es ESTA funcion, que solo sabe hacer
--   una cosa —el login del custodio de una celda que existe— y deja huella.
--
-- Devuelve `papel:clave`, UNA VEZ. La clave no se guarda aqui (Postgres guarda
-- su resumen) ni en la huella: va del resultado a Secret Manager
-- (`t-<n>-cofre-base`, A7a.2) y de ahi al custodio.
--
-- ⚠️ Llamarla otra vez ROTA la clave: el custodio que use la vieja deja de
--   entrar hasta que se le de la nueva. Es lo que se quiere al rotar, y lo que
--   no se quiere por descuido: el aprovisionador solo la llama si el secreto de
--   la celda no existe todavia.
create or replace function iam.dar_papel_de_celda(p_celda text) returns text
language plpgsql security definer
set search_path = pg_catalog, iam
as $$
declare
  v_id    text;
  v_papel text;
  v_otra  text;
  v_clave text;
begin
  select id into v_id from iam.celda where nombre = p_celda and estado <> 'retirada';
  if v_id is null then
    raise exception 'no hay una celda viva que se llame «%»', p_celda;
  end if;

  v_papel := 'cofre_' || regexp_replace(lower(p_celda), '[^a-z0-9]', '_', 'g');

  -- Dos nombres que dan el mismo papel (`a-b` y `a_b`) no se pisan en silencio.
  select celda into v_otra from iam.papel_de_celda where papel = v_papel;
  if v_otra is not null and v_otra <> v_id then
    raise exception 'el papel «%» ya es de otra celda', v_papel;
  end if;

  -- 256 bits del generador fuerte de Postgres (`gen_random_uuid`, que usa
  -- `pg_strong_random`), sin extensiones.
  v_clave := replace(gen_random_uuid()::text || gen_random_uuid()::text, '-', '');

  if exists (select 1 from pg_roles where rolname = v_papel) then
    execute format('alter role %I with login password %L', v_papel, v_clave);
  else
    execute format('create role %I login password %L', v_papel, v_clave);
  end if;
  execute format('grant ore_cofre to %I', v_papel);

  insert into iam.papel_de_celda (papel, celda) values (v_papel, v_id)
    on conflict (papel) do nothing;

  -- El hecho y su huella, en la misma transaccion. Sin la clave.
  insert into iam.huella (quien, operacion, sobre, detalle)
  values (session_user, 'celda:papel-de-base', v_id,
          jsonb_build_object('papel', v_papel, 'celda', p_celda,
                             'rotada', v_otra is not null));

  return v_papel || ':' || v_clave;
end;
$$;

comment on function iam.dar_papel_de_celda(text) is
  'Crea (o rota) el login del custodio de una celda y devuelve papel:clave una vez (0047 A7a.1).';

revoke all on function iam.dar_papel_de_celda(text) from public;
grant execute on function iam.dar_papel_de_celda(text) to ore_aprovisionador;
