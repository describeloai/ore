-- 052 · LAS EJECUCIONES DE TODOS (docs/ejecuciones.md X2, DX1)
--
-- `system.default.executions` guarda una fila por ejecución: qué corrió, quién,
-- sobre qué datos. Cada persona lee la suya, sin pedir nada (la tabla es suya:
-- su credencial no alcanza otra). La de toda la organización, sólo quien la
-- administra: ORGADMIN, y nadie más —ni SECURITYADMIN, a diferencia de
-- `actividad:leer-toda`—. Lo decidió el usuario el 2026-10-24.
--
-- `ore-serve` lo pregunta al abrir el puesto, con el token de la persona, y lo
-- guarda con él: desde el puesto sólo habla el agente.
insert into iam.potestad (nombre, que_hace, ejercida) values
  ('ejecuciones:leer-todas', 'leer las ejecuciones de toda la organizacion en system.default.executions, no solo las propias', true)
on conflict (nombre) do nothing;

insert into iam.rol_potestad (rol, potestad) values
  ('ORGADMIN', 'ejecuciones:leer-todas')
on conflict do nothing;
