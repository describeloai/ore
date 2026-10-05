-- 049 · CREAR NO ES ROTAR (0054 I2 e I3)
--
-- El 2026-10-04 a las 21:35 el aprovisionador —un `CronJob` que corre cada
-- cinco minutos— llamo a `iam.dar_papel_de_celda` para `demo` y para `victor`,
-- porque una llamada a gcloud fallo y su respuesta vacia se leyo como «la celda
-- no tiene secreto». Esa funcion ROTABA: la clave nueva nacio aqui, no llego a
-- Secret Manager, y a la mañana siguiente los dos custodios no podian entrar.
--
-- ⇒ Dos cosas que esta base hace cumplir, y no el guion que la llama:
--
--   I2  quien converge solo CREA. `crear_papel_de_celda` se niega si la celda
--       ya tiene papel: un descuido del guion es ahora un error, no una rotacion.
--   I3  rotar no deja a nadie fuera. Dos papeles por celda, alternos
--       (`cofre_<c>` y `cofre_<c>_b`). `preparar` pone clave al que NO esta
--       vigente —no puede tocar el vigente— y `confirmar` sólo da el cambio por
--       hecho si el papel nuevo ya tiene una sesion abierta: el custodio ya
--       entra con el. Hasta ese momento entran los dos.
--
-- Y `dar_papel_de_celda` se va: era la unica forma de rotar sin querer.

-- ── ① dos papeles por celda, y uno vigente ─────────────────────────────────
--
-- Los que hay hoy son los vigentes: son los que dicen los secretos.
alter table iam.papel_de_celda drop constraint if exists papel_de_celda_celda_key;
alter table iam.papel_de_celda add column if not exists vigente boolean not null default true;
create unique index if not exists papel_de_celda_un_vigente
  on iam.papel_de_celda (celda) where vigente;

comment on column iam.papel_de_celda.vigente is
  'El papel cuya clave guarda el secreto de la celda. Uno por celda; el otro, si lo hay, esta preparado o retirado (0054 I3).';

-- ── el nombre base y la celda viva, una vez ────────────────────────────────
create or replace function iam.papel_base_de_celda(p_celda text, out v_id text, out v_papel text)
language plpgsql stable security definer
set search_path = pg_catalog, iam
as $$
begin
  select id into v_id from iam.celda where nombre = p_celda and estado <> 'retirada';
  if v_id is null then
    raise exception 'no hay una celda viva que se llame «%»', p_celda;
  end if;
  v_papel := 'cofre_' || regexp_replace(lower(p_celda), '[^a-z0-9]', '_', 'g');
end;
$$;
revoke all on function iam.papel_base_de_celda(text) from public;

-- clave nueva para un papel (lo crea si no existe), con `ore_cofre`
create or replace function iam.poner_clave_de_papel(p_papel text, p_celda_id text) returns text
language plpgsql security definer
set search_path = pg_catalog, iam
as $$
declare
  v_otra  text;
  v_clave text;
begin
  -- Dos nombres que dan el mismo papel (`a-b` y `a_b`, o `x` y el `_b` de otra)
  -- no se pisan en silencio.
  select celda into v_otra from iam.papel_de_celda where papel = p_papel;
  if v_otra is not null and v_otra <> p_celda_id then
    raise exception 'el papel «%» ya es de otra celda', p_papel;
  end if;
  -- 256 bits del generador fuerte de Postgres, sin extensiones.
  v_clave := replace(gen_random_uuid()::text || gen_random_uuid()::text, '-', '');
  if exists (select 1 from pg_roles where rolname = p_papel) then
    execute format('alter role %I with login password %L', p_papel, v_clave);
  else
    execute format('create role %I login password %L', p_papel, v_clave);
  end if;
  execute format('grant ore_cofre to %I', p_papel);
  return v_clave;
end;
$$;
revoke all on function iam.poner_clave_de_papel(text, text) from public;

-- ── ② CREAR: lo unico que puede el aprovisionador (I2) ─────────────────────
--
-- Devuelve `papel:clave` UNA vez. Si la celda ya tiene papel, se niega: no hay
-- forma de que converger rote.
create or replace function iam.crear_papel_de_celda(p_celda text) returns text
language plpgsql security definer
set search_path = pg_catalog, iam
as $$
declare
  v_id    text;
  v_papel text;
  v_clave text;
begin
  select b.v_id, b.v_papel into v_id, v_papel from iam.papel_base_de_celda(p_celda) b;
  if exists (select 1 from iam.papel_de_celda where celda = v_id) then
    raise exception 'la celda «%» ya tiene papel: crear no rota (rotar es preparar y confirmar, 0054 I3)', p_celda
      using errcode = 'unique_violation';
  end if;
  v_clave := iam.poner_clave_de_papel(v_papel, v_id);
  insert into iam.papel_de_celda (papel, celda, vigente) values (v_papel, v_id, true);
  insert into iam.huella (quien, operacion, sobre, detalle)
  values (session_user, 'celda:papel-creado', v_id,
          jsonb_build_object('papel', v_papel, 'celda', p_celda));
  return v_papel || ':' || v_clave;
end;
$$;
comment on function iam.crear_papel_de_celda(text) is
  'Crea el login del custodio de una celda que no tiene ninguno; se niega si ya lo tiene (0054 I2).';
revoke all on function iam.crear_papel_de_celda(text) from public;
grant execute on function iam.crear_papel_de_celda(text) to ore_aprovisionador;

-- ── ③ PREPARAR: clave nueva al papel que NO esta vigente (I3) ──────────────
--
-- No es del aprovisionador: rotar es un acto de una persona
-- (`malla/rotar-base-del-cofre.sh`), y lo corre el dueño de la base.
create or replace function iam.preparar_papel_de_celda(p_celda text) returns text
language plpgsql security definer
set search_path = pg_catalog, iam
as $$
declare
  v_id      text;
  v_base    text;
  v_vigente text;
  v_otro    text;
  v_clave   text;
begin
  select b.v_id, b.v_papel into v_id, v_base from iam.papel_base_de_celda(p_celda) b;
  select papel into v_vigente from iam.papel_de_celda where celda = v_id and vigente;
  if v_vigente is null then
    raise exception 'la celda «%» no tiene papel vigente: eso es crear, no rotar', p_celda;
  end if;
  v_otro := case when v_vigente = v_base then v_base || '_b' else v_base end;
  -- ⛔ La garantia entera de I3 es esta linea: el vigente no se toca.
  if v_otro = v_vigente then
    raise exception 'preparar tocaria el papel vigente «%»', v_vigente;
  end if;
  v_clave := iam.poner_clave_de_papel(v_otro, v_id);
  insert into iam.papel_de_celda (papel, celda, vigente) values (v_otro, v_id, false)
    on conflict (papel) do update set vigente = false, desde = now();
  insert into iam.huella (quien, operacion, sobre, detalle)
  values (session_user, 'celda:papel-preparado', v_id,
          jsonb_build_object('papel', v_otro, 'vigente', v_vigente, 'celda', p_celda));
  return v_otro || ':' || v_clave;
end;
$$;
comment on function iam.preparar_papel_de_celda(text) is
  'Pone clave nueva al papel de la celda que no esta vigente y la devuelve una vez; nunca toca el vigente (0054 I3).';
revoke all on function iam.preparar_papel_de_celda(text) from public;

-- ── ④ CONFIRMAR: sólo si el custodio ya entra con el nuevo (I3) ────────────
--
-- La prueba es una sesion abierta de ese papel en esta base: `ore-cofre` tiene
-- una toda su vida. Sin ella, confirmar dejaria la celda sin base, y se niega.
create or replace function iam.confirmar_papel_de_celda(p_celda text, p_papel text) returns text
language plpgsql security definer
set search_path = pg_catalog, iam
as $$
declare
  v_id    text;
  v_base  text;
  v_viejo text;
begin
  select b.v_id, b.v_papel into v_id, v_base from iam.papel_base_de_celda(p_celda) b;
  if not exists (select 1 from iam.papel_de_celda where celda = v_id and papel = p_papel and not vigente) then
    raise exception '«%» no es un papel preparado de la celda «%»', p_papel, p_celda;
  end if;
  if not exists (select 1 from pg_stat_activity
                  where usename = p_papel and datname = current_database()) then
    raise exception 'nadie ha entrado todavia con «%»: confirmar ahora dejaria la celda «%» sin base', p_papel, p_celda;
  end if;
  select papel into v_viejo from iam.papel_de_celda where celda = v_id and vigente;
  update iam.papel_de_celda set vigente = false where celda = v_id and vigente;
  update iam.papel_de_celda set vigente = true, desde = now() where papel = p_papel;
  -- El viejo se queda sin login y sin clave; sus sesiones abiertas, si queda
  -- alguna, terminan solas.
  if v_viejo is not null then
    execute format('alter role %I with nologin password null', v_viejo);
  end if;
  insert into iam.huella (quien, operacion, sobre, detalle)
  values (session_user, 'celda:papel-confirmado', v_id,
          jsonb_build_object('papel', p_papel, 'retirado', v_viejo, 'celda', p_celda));
  return p_papel;
end;
$$;
comment on function iam.confirmar_papel_de_celda(text, text) is
  'Da por vigente el papel preparado si ya tiene una sesion abierta, y retira el otro (0054 I3).';
revoke all on function iam.confirmar_papel_de_celda(text, text) from public;

-- ── ⑤ y la que rotaba sin querer, fuera ───────────────────────────────────
drop function if exists iam.dar_papel_de_celda(text);
