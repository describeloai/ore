-- 041 · CADA CUSTODIO, SU ORGANIZACION: SEGURIDAD POR FILA (0047 A7a.4)
--
-- Desde la A7a.3 cada custodio entra con el login de su celda (`cofre_<celda>`,
-- 040), y `iam.mi_organizacion()` sabe de que organizacion es. Pero todos son
-- miembros de `ore_cofre`, y `ore_cofre` lee el censo, las concesiones y los
-- metadatos de los secretos de TODAS las organizaciones (0047 M4). Esto lo
-- corta: un login de `ore_cofre` ve las filas de su organizacion y ninguna mas.
--
-- ⛔ PRECONDICION, medida antes de aplicar: ninguna sesion de `cofre_app` (el
--   login compartido, sin celda). Con esto puesto no veria nada.
--
-- ── Lo medido (0047 § «A7a.4, medido») ─────────────────────────────────────
--
--   quien entra    ore_iam (iam_app) lo lee todo · ore_aprovisionador lee
--                  `iam.organizacion` por columnas · ore_cofre, lo del custodio
--                  · keycloak es superusuario y no pasa por aqui
--   el custodio    cofre.secreto, iam.agente, iam.celda, iam.organizacion,
--                  iam.persona, y por las vistas concesion_viva y
--                  potestades_de_persona: concesion, pertenencia,
--                  pertenencia_rol, rol_potestad
--
-- ⭐ `(select iam.mi_organizacion())` y no la llamada a secas: asi se evalua una
--   vez por consulta y no una por fila.

-- ── ① las vistas, con los permisos de quien las lee ─────────────────────────
--
-- Son del superusuario, y una vista lee con los permisos de su dueño: sin
-- esto, `concesion_viva` enseñaria todas las concesiones aunque `concesion`
-- tuviera su politica. Con `security_invoker` leen como quien pregunta, y por
-- eso `ore_cofre` necesita leer lo de debajo (con su politica encima).
alter view iam.concesion_viva        set (security_invoker = true);
alter view iam.potestades_de_persona set (security_invoker = true);
grant select on iam.concesion, iam.pertenencia, iam.pertenencia_rol,
                iam.rol_potestad, iam.por_defecto
   to ore_cofre;

-- ── ② la seguridad por fila, en las ocho tablas ─────────────────────────────
--
-- NO en los catalogos (`rol_potestad`, `potestad`, `rol_de_recurso`), que son
-- de todos, ni en `iam.huella` (el custodio solo inserta; leer no puede).
alter table cofre.secreto       enable row level security;
alter table iam.agente          enable row level security;
alter table iam.celda           enable row level security;
alter table iam.concesion       enable row level security;
alter table iam.organizacion    enable row level security;
alter table iam.persona         enable row level security;
alter table iam.pertenencia     enable row level security;
alter table iam.pertenencia_rol enable row level security;

-- ── ③ `ore-iam` lo sigue viendo todo ────────────────────────────────────────
do $$
declare t text;
begin
  foreach t in array array['agente', 'celda', 'concesion', 'organizacion',
                           'persona', 'pertenencia', 'pertenencia_rol'] loop
    execute format('drop policy if exists ore_iam_todo on iam.%I', t);
    execute format('create policy ore_iam_todo on iam.%I for all to ore_iam '
                   'using (true) with check (true)', t);
  end loop;
end $$;

-- ── ④ el aprovisionador, lo de siempre: sus `grant` por columnas son el limite
drop policy if exists aprovisionador_lee on iam.organizacion;
create policy aprovisionador_lee on iam.organizacion
  for select to ore_aprovisionador using (true);

-- ── ⑤ el custodio, su organizacion ──────────────────────────────────────────
do $$
declare t text;
begin
  foreach t in array array['agente', 'celda', 'concesion', 'pertenencia',
                           'pertenencia_rol'] loop
    execute format('drop policy if exists custodio_la_suya on iam.%I', t);
    execute format('create policy custodio_la_suya on iam.%I for select to ore_cofre '
                   'using (organizacion = (select iam.mi_organizacion()))', t);
  end loop;
end $$;

drop policy if exists custodio_la_suya on iam.organizacion;
create policy custodio_la_suya on iam.organizacion
  for select to ore_cofre
  using (id = (select iam.mi_organizacion()));

-- De `persona`, las que pertenecen a su organizacion. Quien no pertenece no
-- existe para el custodio: la misma frase de `potestad::exige`, un piso abajo.
drop policy if exists custodio_la_suya on iam.persona;
create policy custodio_la_suya on iam.persona
  for select to ore_cofre
  using (exists (select 1 from iam.pertenencia p
                  where p.persona = persona.id
                    and p.organizacion = (select iam.mi_organizacion())));

-- Los secretos: leer y escribir, los suyos.
drop policy if exists custodio_la_suya on cofre.secreto;
create policy custodio_la_suya on cofre.secreto
  for all to ore_cofre
  using (organizacion = (select iam.mi_organizacion()))
  with check (organizacion = (select iam.mi_organizacion()));

-- ── ⑥ y las tres funciones, que escriben por encima de las politicas ────────
--
-- Son `security definer` del superusuario y reciben la organizacion por
-- argumento: el custodio de un inquilino podia conceder o revocar sobre un
-- secreto de otro. Ahora, si quien llama es un custodio, la organizacion
-- tiene que ser la suya. Quien no es custodio (el operador, `ore-iam`) sigue
-- como estaba.
--
-- ⛔ La pertenencia se mira en `pg_auth_members`, NO con `pg_has_role`: para un
--   superusuario `pg_has_role(…, 'member')` es verdad con CUALQUIER papel, y la
--   guarda trataba al operador (`keycloak`) como a un custodio sin celda —y le
--   negaba conceder—. Lo cazo `el-cofre.sh` 9, que siembra como superusuario.
create or replace function iam.solo_la_mia(p_organizacion text) returns void
language plpgsql stable security definer
set search_path = pg_catalog, iam
as $$
begin
  if exists (select 1
               from pg_auth_members a
               join pg_roles g on g.oid = a.roleid
               join pg_roles m on m.oid = a.member
              where g.rolname = 'ore_cofre' and m.rolname = session_user)
     and p_organizacion is distinct from iam.mi_organizacion() then
    raise exception 'el custodio de una celda solo concede y revoca en su organizacion'
      using errcode = 'insufficient_privilege';
  end if;
end;
$$;
revoke all on function iam.solo_la_mia(text) from public;

create or replace function iam.conceder_de_secreto(p_id text, p_sujeto text, p_recurso text, p_rol text, p_concedio text, p_organizacion text)
 returns void
 language plpgsql
 security definer
 set search_path to 'pg_catalog', 'iam'
as $function$
begin
  perform iam.solo_la_mia(p_organizacion);
  -- ⛔ La puerta. Sin esto, esta funcion seria `insert` con otro nombre.
  if p_recurso not like 'secreto/%' then
    raise exception
      'esta funcion solo concede sobre secretos, y se le paso `%`', p_recurso
      using hint = 'lo demas se concede por `POST /organizaciones/{org}/concesiones`';
  end if;
  insert into iam.concesion (id, sujeto, recurso, rol, concedio, organizacion)
  values (p_id, p_sujeto, p_recurso, p_rol, p_concedio, p_organizacion);
end;
$function$;

create or replace function iam.revocar_de_secreto(p_recurso text, p_organizacion text, p_revoco text)
 returns integer
 language plpgsql
 security definer
 set search_path to 'pg_catalog', 'iam'
as $function$
declare
  n integer;
begin
  perform iam.solo_la_mia(p_organizacion);
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
$function$;

create or replace function iam.revocar_de_secreto_operador(p_recurso text, p_organizacion text, p_agente text)
 returns integer
 language plpgsql
 security definer
 set search_path to 'pg_catalog', 'iam'
as $function$
declare
  n integer;
begin
  perform iam.solo_la_mia(p_organizacion);
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
$function$;
