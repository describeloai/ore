-- ═══════════════════════════════════════════════════════════════════════════
-- 005 · QUIÉN ENTRA (ADR 0058, P5·6)
--
-- Lo que el proxy de Neon comprueba antes de dejar pasar una conexión, por
-- proyecto (como en Neon), y que se le contesta en `get_endpoint_access_control`:
--
--   ips_permitidas    `allowed_ips`: vacía = todas. Una IP (`203.0.113.7`), una
--                     subred (`203.0.113.0/24`) o un rango (`203.0.113.1-203.0.113.9`),
--                     IPv4 o IPv6. ⛔ El proxy convierte una entrada que no entiende
--                     en «ninguna IP»: se validan en la API, nunca llega una mala.
--   bloquear_publico  `block_public_connections`: nadie entra por la entrada pública.
--   limites           `rate_limits.connection_attempts`: intentos de conexión por
--                     endpoint y protocolo (cubeta: por segundo y ráfaga). Siempre
--                     hay uno: es lo que corta la fuerza bruta.
--
-- Cambiarlos es una operación (`configurar-acceso`): cuando queda hecha, el
-- proxy olvida lo que guardaba (`olvidar.rs`) y lo nuevo vale ya.
-- ═══════════════════════════════════════════════════════════════════════════

alter table plano.proyecto
  add column if not exists ips_permitidas   text[]  not null default '{}'
    check (cardinality(ips_permitidas) <= 100),
  add column if not exists bloquear_publico boolean not null default false,
  add column if not exists limites          jsonb   not null default
    '{"tcp": {"por_segundo": 100, "rafaga": 1000},
      "ws": {"por_segundo": 100, "rafaga": 1000},
      "http": {"por_segundo": 1000, "rafaga": 10000}}';
