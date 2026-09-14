-- 032 · LA CELDA DICE CUÁNDO LA APROVISIONARON POR ÚLTIMA VEZ
--
-- ══════════════════════════════════════════════════════════════════════════
-- La 0025 E5. Es el `status` del patrón de operador: la fila de la celda es la
-- `spec` —qué tiene que haber—, y esta columna la escribe **quien reconcilia**,
-- no quien pide. La escribe el aprovisionador por `POST /celdas/{celda}/
-- aprovisionada` al acabar una pasada entera sobre esa celda, con su propia
-- identidad (`rubix_tipo=aprovisionador`) y con huella.
--
-- ⭐ Nulo significa «ninguna pasada ha acabado todavía»: la consola lo pinta
--   `Provisioning` sin preguntar por el camino. Con una fecha, la sonda decide
--   si VIVE (0024-④): identidad de un plano, vida del otro.
--
-- ⛔ No es `estado`. `estado` es administrativo (activa / suspendida) y lo
--   escribe `ore-iam` por una persona; esto es un hecho del reconciliador. El
--   estado `aprovisionando` que la 025 dejó y nadie escribía se sustituye por
--   «`aprovisionada` es nulo», que sí lo escribe alguien.
--
-- Ensanchar: nulable, sin defecto. `demo` y `prueba` la rellenan en la
-- siguiente pasada del CronJob (cada 5 min), no esta migración.
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md`, E5
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.celda add column if not exists aprovisionada timestamptz;
comment on column iam.celda.aprovisionada is
  'Cuando el aprovisionador acabo su ultima pasada entera sobre esta celda. Nulo: ninguna todavia (0025 E5).';
