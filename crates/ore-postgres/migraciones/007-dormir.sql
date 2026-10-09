-- ═══════════════════════════════════════════════════════════════════════════
-- 007 · DORMIR (ADR 0058, P6·3)
--
-- ultima_actividad  el `last_active` que dijo su compute_ctl la última vez que se le preguntó
--                   (o la hora en que quedó listo): sin consultas en curso desde entonces, y
--                   pasado `dormir_tras`, el reconciliador lo duerme.
-- dormido_en        cuándo se durmió (observado = 'dormido'): sin cómputo, coste 0.
-- lsn_al_dormir     el LSN que dijo `/terminate`: hasta ahí está todo en el almacenamiento.
-- ═══════════════════════════════════════════════════════════════════════════

alter table plano.endpoint
  add column if not exists ultima_actividad timestamptz,
  add column if not exists dormido_en       timestamptz,
  add column if not exists lsn_al_dormir    text;
