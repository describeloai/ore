-- 012 · EL ROL DE LA ORGANIZACIÓN — la otra mitad del corte de la `011`
--
-- ── Por qué `rol` y no `papel` ─────────────────────────────────────────────
--
--   `papel` entró por una distinción real: un **rol de realm** es un atributo de
--   la identidad, lo estampa el IdP en el token y lo administra Keycloak; lo de
--   aquí es la **relación** entre un sujeto y algo. `007` sigue en pie entera:
--
--     > el realm tiene CERO roles. Keycloak dice quién eres; esto dice qué
--     > puedes sobre qué. Mezclarlos obliga a pedirle al IdP permiso para
--     > cambiar un permiso.
--
--   ⭐ Pero la distinción no se defiende con una palabra propia: se defiende
--     con que **el realm siga sin roles**, que es comprobable. Y lo que sí paga
--     la palabra estándar es que **viaja aguas abajo** — de la pertenencia a la
--     concesión, y de ahí a quien consuma esto. Un nombre interno distinto
--     obliga a traducir en cada frontera, y cada traducción es una oportunidad
--     de equivocarse.
--
-- ── ⚠️ LO QUE ESTA MIGRACIÓN **NO** DECIDE, y hay que decidir ───────────────
--
--   `002` describe dos de los cuatro con frases del **plano de abajo**:
--
--       lector    «ve la ontologia y no la cambia»
--       miembro   «decide sobre la ontologia: responde la cola de `review`»
--
--   Eso es capacidad sobre el ÁRBOL viviendo en la tabla de la ORGANIZACIÓN —
--   la misma contaminación que la `011` acaba de cortar, en la otra dirección.
--   Aquí se corrige **la prosa** de esas dos filas, que es una columna de texto,
--   y **no se toca el censo**: si `lector` y `miembro` deben existir arriba, o
--   si la escalera de la organización son sólo `miembro` y `administrador` con
--   el árbol gobernado por `rol_de_recurso`, es una decisión y va en su
--   migración. Cambiar la lista de tapadillo aquí sería justo lo que `002`
--   prohíbe: *«una lista que puede crecer sin una decisión deja de significar
--   nada»* — y encogerla sin una tampoco.

alter table iam.papel rename to rol;

comment on table iam.rol is
  'El rol EN LA ORGANIZACION. Ordenado: la guarda del rodeo compara alturas.';

alter table iam.pertenencia rename column papel to rol;
alter table iam.invitacion  rename column papel to rol;

-- ⛔ Igual que en la `011`: `create or replace view` no puede cambiar el nombre
--   de una columna, así que la vista se borra y se vuelve a crear. Si no,
--   seguiría sirviendo `papel` sobre una tabla que ya dice `rol`.
drop view if exists iam.invitacion_estado;
create view iam.invitacion_estado as
  select i.*,
         case
           when i.revocada_en is not null then 'revocada'
           when i.redimida_en is not null then 'redimida'
           when i.caduca_en   <  now()    then 'caducada'
           else 'pendiente'
         end as estado
    from iam.invitacion i;

-- La prosa de las dos filas contaminadas. El censo NO cambia: cuatro antes,
-- cuatro después.
update iam.rol set que_puede = 've la organizacion y no cambia nada en ella'
 where nombre = 'lector';
update iam.rol set que_puede = 'pertenece a la organizacion; no invita ni concede'
 where nombre = 'miembro';

do $$
declare n int;
begin
  select count(*) into n from iam.rol;
  if n <> 4 then
    raise exception 'iam.rol deberia tener 4 filas y tiene %: esta migracion no toca el censo', n;
  end if;
end $$;
