-- 051 · LAS POTESTADES DE POSTGRES (0058 P4·6, ORE Serverless Postgres)
--
-- `ore-serve` atiende `/v1/postgres/…` y, antes de pasárselo a `ore-postgres`,
-- pregunta aquí:
--
--   postgres:ver        GET de todo: proyectos, ramas, endpoints, roles, bases
--   postgres:crear      crear un proyecto (su dueño es quien lo crea, 0052)
--   postgres:usar       roles: crear uno, regenerar su contraseña, borrarlo
--   postgres:gestionar  ramas, endpoints, bases y borrar, en un proyecto AJENO
--
-- Dentro de su proyecto, el dueño lo gestiona todo sin `postgres:gestionar`:
-- lo decide `ore-serve` con el `dueno` que guarda `ore-postgres`, no esta base.
--
-- ── A quién ─────────────────────────────────────────────────────────────────
--
-- ver, crear y usar, a todo el que pertenece (`iam.por_defecto`): es el estándar
-- —en Databricks (Lakebase) todo usuario del workspace crea proyectos, en Neon
-- todo miembro—, decidido así el 2026-10-08. Gestionar lo ajeno, a quien manda
-- en la organización: ORGADMIN (que lo cubre todo, el invariante de la 014) y
-- ACCOUNTADMIN.
insert into iam.potestad (nombre, que_hace, ejercida) values
  ('postgres:ver',       'ver los proyectos Postgres de la organizacion: ramas, endpoints, roles y bases', true),
  ('postgres:crear',     'crear un proyecto Postgres. Su dueño es quien lo crea', true),
  ('postgres:usar',      'crear roles de Postgres y regenerar sus contraseñas: conectarse', true),
  ('postgres:gestionar', 'ramas, endpoints, bases y borrar en un proyecto Postgres de otro', true)
on conflict (nombre) do nothing;

create or replace view iam.por_defecto as
  select nombre as potestad from iam.potestad
   where nombre in ('organizacion:leer', 'miembro:listar', 'rol:listar', 'puesto:abrir',
                    'postgres:ver', 'postgres:crear', 'postgres:usar');

insert into iam.rol_potestad (rol, potestad) values
  ('ORGADMIN',     'postgres:gestionar'),
  ('ACCOUNTADMIN', 'postgres:gestionar')
on conflict do nothing;
