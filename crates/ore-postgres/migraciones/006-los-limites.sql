-- ═══════════════════════════════════════════════════════════════════════════
-- 006 · LOS LÍMITES DEL CLIENTE (ADR 0058, P6·2)
--
-- Cuánto tiempo sin actividad hasta dormir, por endpoint: 300 s por defecto (como Neon);
-- 0 = nunca; si no, de 60 s a 7 días. «Actividad» es la de compute_ctl (last_active): una
-- consulta en curso, no una sesión ociosa (P6·0). Las CU mínimas y máximas ya estaban (003);
-- desde P6·2 se cambian en un endpoint vivo (`POST …/endpoints/{e}/ajustes`).
-- ═══════════════════════════════════════════════════════════════════════════

alter table plano.endpoint
  add column if not exists dormir_tras integer not null default 300
    check (dormir_tras = 0 or dormir_tras between 60 and 604800);
