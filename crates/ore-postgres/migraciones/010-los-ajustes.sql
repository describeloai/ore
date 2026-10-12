-- ═══════════════════════════════════════════════════════════════════════════
-- 010 · LOS AJUSTES DE LA INSTANCIA (ADR 0058, P8·4b)
--
-- · `nombre`: el que se ve. El `id` no cambia nunca (va en los caminos, en el
--   tenant, en los nombres de las VMs); renombrar es cambiar esto. Nulo = el id.
-- · Los valores por defecto de un cómputo NUEVO (`crear_endpoint` sin cu_min,
--   cu_max o dormir_tras): los mismos que ponía el código, ahora de la instancia.
--   No tocan a los cómputos que ya hay.
-- · `retencion_pitr`: la ventana de historia del tenant (`pitr_interval`), en
--   horas (`6h`). Nula = la del pageserver.toml (malla/83, «1 day»). La aplica el
--   reconciliador en el controller (`PUT /v1/tenant/config`).
-- ═══════════════════════════════════════════════════════════════════════════

alter table plano.proyecto
  add column if not exists nombre text,
  add column if not exists defecto_cu_min double precision not null default 0.25,
  add column if not exists defecto_cu_max double precision not null default 1,
  add column if not exists defecto_dormir_tras integer not null default 300,
  add column if not exists retencion_pitr text;

alter table plano.proyecto drop constraint if exists defectos_validos;
alter table plano.proyecto add constraint defectos_validos check (
  defecto_cu_min >= 0.25 and defecto_cu_min <= defecto_cu_max and defecto_cu_max <= 2
  and (defecto_dormir_tras = 0 or defecto_dormir_tras between 60 and 604800)
  and (retencion_pitr is null or retencion_pitr ~ '^[0-9]{1,2}h$')
);
