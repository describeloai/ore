-- 034 · EL REALM DE PRODUCCIÓN ES `rubix`
--
-- ══════════════════════════════════════════════════════════════════════════
-- El 2026-09-14 el realm `rubix-dev` —donde vivían las personas, las
-- organizaciones y los agentes— se RENOMBRÓ a `rubix` en Keycloak, y el `rubix`
-- vacío de antes se borró. Un rename conserva usuarios, clientes y `sub`; lo
-- único que cambia es el emisor (`iss`) de cada token.
--
-- La identidad de una persona y de un agente es `(emisor, sub)` (003, 024): con
-- el emisor viejo en la fila, nadie volvería a coincidir consigo mismo. Esto
-- cambia el emisor y NADA más — los `sub` son los de siempre.
--
-- ⛔ Y lo que la primera aplicación destapó: el aprovisionador corre cada cinco
--   minutos y, entre el rename y esta migración, ya había registrado por el
--   verbo a los agentes con el emisor NUEVO —fila nueva, mismo `sub`, con sus
--   concesiones heredadas—. Cambiar el emisor de la vieja chocaba con la nueva
--   (`agente_emisor_sub_organizacion_key`). Así que: donde ya hay fila nueva,
--   la vieja sobra y se va con sus concesiones (la nueva heredó las mismas);
--   donde no la hay, se cambia el emisor. La huella conserva los ids viejos:
--   es historia.
--
-- Idempotente: la segunda vez no encuentra nada que cambiar.
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md` (E6, «lo que sigue»)
-- ══════════════════════════════════════════════════════════════════════════

-- personas: ninguna se registró dos veces (nadie entró entre el rename y esto)
update iam.persona set emisor = replace(emisor, '/realms/rubix-dev', '/realms/rubix')
 where emisor like '%/realms/rubix-dev'
   and not exists (select 1 from iam.persona n
                    where n.emisor = replace(iam.persona.emisor, '/realms/rubix-dev', '/realms/rubix')
                      and n.sub = iam.persona.sub);

-- agentes con fila nueva: la vieja sobra
delete from iam.concesion c
 using iam.agente v
 where c.sujeto = v.id and v.emisor like '%/realms/rubix-dev'
   and exists (select 1 from iam.agente n
                where n.emisor = replace(v.emisor, '/realms/rubix-dev', '/realms/rubix')
                  and n.sub = v.sub and n.organizacion = v.organizacion);
delete from iam.agente v
 where v.emisor like '%/realms/rubix-dev'
   and exists (select 1 from iam.agente n
                where n.emisor = replace(v.emisor, '/realms/rubix-dev', '/realms/rubix')
                  and n.sub = v.sub and n.organizacion = v.organizacion);

-- y los demás, al emisor nuevo
update iam.agente set emisor = replace(emisor, '/realms/rubix-dev', '/realms/rubix')
 where emisor like '%/realms/rubix-dev';
