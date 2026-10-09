-- ═══════════════════════════════════════════════════════════════════════════
-- 008 · EL POOL PRECALENTADO (ADR 0058, P6·5)
--
-- pool_computo  cómputos ya arrancados SIN tenant: compute_ctl en `empty`, esperando un
--               /configure (P6·0). `arrancando` hasta que lo dice; entonces `libre`. Despertar
--               reclama uno libre (se borra la fila: desde ahí es del endpoint) y le manda la
--               especificación. ⛔ Un cómputo que sirvió a un tenant NUNCA vuelve al pool: al
--               dormir, se destruye.
-- endpoint.computo  el cómputo que le sirve ahora: `pool-…` si salió del pool, su propio `vm`
--               si arrancó en frío, null dormido. El nombre público (`vm`, el del SNI) no cambia.
-- ═══════════════════════════════════════════════════════════════════════════

create table if not exists plano.pool_computo (
  computo     text        primary key check (computo ~ '^pool-[0-9a-f]{12}$'),
  estado      text        not null default 'arrancando' check (estado in ('arrancando', 'libre')),
  ip_pod      text,
  creado      timestamptz not null default now(),
  libre_en    timestamptz
);

alter table plano.endpoint add column if not exists computo text;
update plano.endpoint set computo = vm
 where computo is null and observado in ('arrancando', 'listo', 'durmiendo', 'borrando');
