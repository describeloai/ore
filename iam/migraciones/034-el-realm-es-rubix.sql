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
-- Idempotente: la segunda vez no encuentra nada que cambiar.
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md` (E6, «lo que sigue»)
-- ══════════════════════════════════════════════════════════════════════════

update iam.persona set emisor = replace(emisor, '/realms/rubix-dev', '/realms/rubix')
 where emisor like '%/realms/rubix-dev';
update iam.agente  set emisor = replace(emisor, '/realms/rubix-dev', '/realms/rubix')
 where emisor like '%/realms/rubix-dev';
