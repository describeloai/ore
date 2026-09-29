-- 046 · LOS AGENTES QUE NINGUNA CELDA USA, RETIRADOS (0047 A9′.4)
--
-- Desde A9′, toda celda pregunta a `ore-iam` si quien llega pertenece a su
-- organización, y un agente pertenece si está en `iam.agente` de ella. Medido el
-- 2026-09-29, dos registrados no los usa ninguna celda:
--
--   `ore-agente`             el cliente viejo, de antes de un agente por celda,
--                            registrado en demo Y en prueba; su última huella es
--                            del 12 de septiembre, y aún tenía 5 concesiones
--                            vivas en demo (`usar` sobre secretos de fuentes)
--   `ore-agente-prueba-dos`  el de una celda retirada; sin una sola huella
--
-- Cada celda usa el suyo (`t-<n>-agente-cliente` = `ore-agente-<n>`). Seguir
-- registrados es seguir pudiendo entrar: con su secreto, cualquiera sacaría un
-- token que la pertenencia daría por bueno en demo o en prueba.
--
-- ⭐ Sus concesiones se REVOCAN, no se borran (la fila se queda, con fecha: «se lo
--   quitamos» no es «nunca lo tuvo»), y quien revoca va en `revoco_agente`, como
--   cuando revoca el operador (la `038`: `revoco` exige una persona); el agente sí sale de `iam.agente`, que es
--   el censo de quién pertenece. Y queda en la huella, con su organización.
-- ⚠️ El cliente de Keycloak no se toca aquí: sin estar en `iam.agente`, su token
--   ya no pertenece a ninguna organización.
with viejos as (
  select id, organizacion, nombre
    from iam.agente
   where nombre in ('ore-agente', 'ore-agente-prueba-dos')
), revocadas as (
  update iam.concesion c
     set revocada_en = now(), revoco_agente = 'migracion:046'
    from viejos v
   where c.sujeto = v.id and c.revocada_en is null
  returning c.id, v.id as agente
), retirados as (
  delete from iam.agente a
   using viejos v
   where a.id = v.id
  returning a.id, a.nombre, a.organizacion
)
insert into iam.huella (quien, operacion, sobre, detalle, organizacion)
select 'migracion:046', 'agente:retirar', r.id,
       jsonb_build_object(
         'nombre', r.nombre,
         'motivo', 'ninguna celda lo usa (0047 A9′.4)',
         'concesiones_revocadas', (select count(*) from revocadas x where x.agente = r.id)),
       r.organizacion
  from retirados r;
