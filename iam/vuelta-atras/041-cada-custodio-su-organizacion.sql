-- VUELTA ATRAS DE LA 041 (0047 A7a.4) — escrita ANTES de aplicarla.
--
-- ⛔ NO vive en `iam/migraciones/`: el runner aplica todo lo que hay alli, y
--   esto no es un paso hacia delante. Se aplica A MANO, como superusuario, si
--   la 041 deja a alguien ciego en produccion:
--
--     kubectl exec -i -n identidad idp-db-0 -- sh -c \
--       'psql -U "$POSTGRES_USER" -d iam -v ON_ERROR_STOP=1' \
--       < iam/vuelta-atras/041-cada-custodio-su-organizacion.sql
--
--   y despues se escribe la migracion siguiente que diga lo mismo, para que el
--   historial no mienta (una migracion aplicada es inmutable; `migrar.sh`).
--
-- Deja las cosas como antes de la 041 en lo que importa —nadie pasa por la
-- seguridad por fila y las vistas leen como su dueño—. Las politicas y la
-- guarda de las funciones se quedan: sin la seguridad por fila activa, las
-- politicas no hacen nada, y la guarda solo niega a un custodio lo que no es
-- suyo.
begin;
alter table cofre.secreto       disable row level security;
alter table iam.agente          disable row level security;
alter table iam.celda           disable row level security;
alter table iam.concesion       disable row level security;
alter table iam.organizacion    disable row level security;
alter table iam.persona         disable row level security;
alter table iam.pertenencia     disable row level security;
alter table iam.pertenencia_rol disable row level security;
alter view iam.concesion_viva        reset (security_invoker);
alter view iam.potestades_de_persona reset (security_invoker);
commit;
