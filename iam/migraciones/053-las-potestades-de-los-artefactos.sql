-- 053 · LAS POTESTADES DE LOS REGISTROS DE ARTEFACTOS (0074 AR1·3, Artifact Registries)
--
-- `ore-serve` emite el token con el que Maven publica en un registro
-- (`POST /artifact-registries/{b}/{s}/{r}/token`) y, antes, pregunta aquí:
--
--   artefacto:publicar    pedir un token de publicar. Es de todo el que pertenece:
--                         lo que lo acota es de quién es el registro (abajo)
--   artefacto:gestionar   publicar en el registro de OTRO
--
-- En su registro, el dueño publica sin `artefacto:gestionar`: lo decide
-- `ore-serve` con el `owner` del documento (0052), como un proyecto Postgres
-- (051). Y `artefacto:publicar` existe aunque lo tengan todos por una razón: Maven
-- no trae el token de la persona, así que cada publicación se anota citando la
-- decisión con que se emitió su token (`ore-iam` atribuye por ella, A6.3). Sin
-- una pregunta, no habría decisión que citar.
--
-- ── A quién ─────────────────────────────────────────────────────────────────
--
-- publicar, a todo el que pertenece (`iam.por_defecto`): cualquiera crea su
-- registro y publica en él, como cualquiera crea su proyecto Postgres. Gestionar
-- lo ajeno, a quien manda en la organización: ORGADMIN (que lo cubre todo, el
-- invariante de la 014) y ACCOUNTADMIN.
insert into iam.potestad (nombre, que_hace, ejercida) values
  ('artefacto:publicar',  'pedir el token con el que se publican librerias en un registro de artefactos propio', true),
  ('artefacto:gestionar', 'publicar librerias en el registro de artefactos de otro', true)
on conflict (nombre) do nothing;

create or replace view iam.por_defecto as
  select nombre as potestad from iam.potestad
   where nombre in ('organizacion:leer', 'miembro:listar', 'rol:listar', 'puesto:abrir',
                    'postgres:ver', 'postgres:crear', 'postgres:usar',
                    'artefacto:publicar');

insert into iam.rol_potestad (rol, potestad) values
  ('ORGADMIN',     'artefacto:gestionar'),
  ('ACCOUNTADMIN', 'artefacto:gestionar')
on conflict do nothing;
