-- 047 · LA ORGANIZACION `prueba`, RETIRADA (0048, deuda 1)
--
-- `prueba` nacio el 2026-09-08 para probar los verbos de `ore-iam` con un token
-- de verdad, cuando aun no habia registro ni consola: su fundadora y unica
-- miembro es una MAQUINA, el cliente `iam-agente` (7f26c36, `98-…`), con
-- `ORGADMIN` por `client_credentials` y sin segundo factor. Desde A9′ la celda
-- `t-prueba` deja entrar a quien pertenece a `prueba`: un secreto de cliente era
-- administrar una organizacion. Medido el 2026-10-01: ninguna persona, ninguna
-- concesion viva, ninguna invitacion pendiente, ningun secreto en el cofre; su
-- ultima huella, del 29-sep (las pasadas del aprovisionador). Es desechable, y
-- se retira entera.
--
-- ⭐ Lo que hace, todo con huella y nada borrado que sea historia:
--
--   la celda `prueba`   → `retirada`. El reconciliador desmonta lo suyo en su
--                         pasada (0025 E6): secretos `t-prueba-*`, copia,
--                         cuentas, registro DNS y su compartimento en la forja.
--                         `retirar` por el puente la niega por ser la de casa;
--                         aqui la retira el operador, con la organizacion
--   su agente           → fuera de `iam.agente`, concesiones revocadas (la 046)
--   sus miembros        → fuera de `iam.pertenencia` (y su rol, en cascada)
--   la organizacion     → `retirada`. La fila se queda: es historia, y su
--                         fundadora (`creada_por`) tambien
--   `cofre_prueba`      → sin papel y sin login: el custodio de una celda
--                         retirada no entra a la base
--
-- ⚠️ El cliente `iam-agente` del IdP no se toca aqui: lo borra
--   `identidad/aplicar.mjs` (`RETIRADOS`). Sin pertenencia, su token ya no es
--   nadie en ninguna organizacion.
-- ⚠️ La llave de KMS de la organizacion se queda: Google no borra llaves (ver
--   `malla/desaprovisionar-inquilino.sh`). El reconciliador le quita el permiso.
with org as (
  select id from iam.organizacion where nombre = 'prueba' and estado <> 'retirada'
), celdas as (
  update iam.celda c set estado = 'retirada'
    from org where c.organizacion = org.id and c.estado <> 'retirada'
  returning c.id, c.nombre, c.organizacion
), agentes as (
  select a.id, a.nombre, a.organizacion from iam.agente a join org on a.organizacion = org.id
), revocadas as (
  update iam.concesion c
     set revocada_en = now(), revoco_agente = 'migracion:047'
    from agentes a
   where c.sujeto = a.id and c.revocada_en is null
  returning c.id
), sin_agentes as (
  delete from iam.agente a using agentes x where a.id = x.id
  returning a.id, a.nombre, a.organizacion
), miembros as (
  delete from iam.pertenencia p using org where p.organizacion = org.id
  returning p.persona, p.organizacion
), orgs as (
  update iam.organizacion o set estado = 'retirada'
    from org where o.id = org.id
  returning o.id
)
insert into iam.huella (quien, operacion, sobre, detalle, organizacion)
select 'migracion:047', 'celda:retirar', id,
       jsonb_build_object('celda', nombre, 'motivo', 'su organizacion se retira (0048)'), organizacion
  from celdas
union all
select 'migracion:047', 'agente:retirar', id,
       jsonb_build_object('nombre', nombre, 'motivo', 'su organizacion se retira (0048)'), organizacion
  from sin_agentes
union all
select 'migracion:047', 'miembro:quitar', persona,
       jsonb_build_object('motivo', 'su organizacion se retira (0048); era el cliente iam-agente'), organizacion
  from miembros
union all
select 'migracion:047', 'organizacion:retirar', id,
       jsonb_build_object('nombre', 'prueba', 'motivo', 'desechable: la fundo y la administraba una maquina (0048)'), id
  from orgs;

-- `cofre_prueba`: sin papel de celda y sin login. `if exists`: en las bases de
-- prueba nunca existio. `drop owned` se lleva sus permisos (no posee nada:
-- medido el 2026-10-01).
delete from iam.papel_de_celda where papel = 'cofre_prueba';
do $$
begin
  if exists (select 1 from pg_roles where rolname = 'cofre_prueba') then
    revoke ore_cofre from cofre_prueba;
    drop owned by cofre_prueba;
    drop role cofre_prueba;
  end if;
end $$;
