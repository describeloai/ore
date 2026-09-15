-- 036 · EL ESTADO DE LA CELDA, tal como su informador lo mide (0026 E1)
--
-- Lo que sólo el API server sabe de una celda —cuota usada, jobs, si el control
-- está listo— lo observa un informador que vive en la celda y lo EMPUJA aquí
-- (`POST /celdas/{celda}/estado`). Es telemetría: vale el ÚLTIMO, así que es una
-- fila por celda y se sobreescribe. No es un acto: la fila que se sobreescribe
-- cada minuto no deja huella; sí la deja EMPEZAR a informar y VOLVER tras un
-- silencio, que eso sí le interesa a quien lea la actividad.
--
-- `cuerpo` es el snapshot de la 0026-② tal cual llegó (validado en ore-iam).
-- `medido_en` es lo que el informador dice; `recibido_en`, cuando llegó: la
-- diferencia es el retraso del camino, y la consola pinta «medido hace 40 s»
-- con el primero.
create table if not exists iam.celda_estado (
  celda        text        primary key references iam.celda(id) on delete cascade,
  medido_en    timestamptz not null,
  recibido_en  timestamptz not null default now(),
  cuerpo       jsonb       not null
);
comment on table iam.celda_estado is
  'El ultimo snapshot que el informador de la celda empujo (0026-②). Una fila por celda; se sobreescribe.';

grant select, insert, update on iam.celda_estado to ore_iam;
