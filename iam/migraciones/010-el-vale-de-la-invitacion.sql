-- 010 · EL VALE — el `id` de una invitación no es lo que se redime
--
-- ── ⛔⛔ LO QUE ESTO EVITA, y estaba a punto de heredarse ────────────────────
--
--   En la tabla de la plataforma el `id` es la clave primaria **y** lo que
--   viaja en el enlace del correo. Dos consecuencias:
--
--     · quien pueda LISTAR invitaciones puede redimirlas
--     · un `pg_dump` es una carpeta de vales al portador
--
--   ⇒ Se parten en dos. El `id` es público —se lista, se cita en una huella— y
--     el VALE sólo existe en el correo. De él se guarda **el resumen, no el
--     vale**: la misma disciplina que una contraseña, y por el mismo motivo.
--
-- ── Y el correo, normalizado ────────────────────────────────────────────────
--
--   Es una cita, no una identidad. Dos invitaciones que sólo se diferencian en
--   la caja serían dos vales para la misma persona.
--
--   ⚠️ Y esto **no contradice** que el `sub` no se normalice, que es la regla
--     de `003`: el `sub` es la CLAVE de identidad y plegarlo fundiría dos
--     personas en una. La distinción es suya —`021`— y vale entera.

do $$
begin
  if exists (select 1 from iam.invitacion) then
    raise exception 'iam.invitacion tiene filas: esta migracion asumia que estaba vacia';
  end if;
end $$;

alter table iam.invitacion
  -- sha256 en hexadecimal: 64 caracteres. El vale nunca se guarda.
  add column vale_resumen text not null,
  add constraint invitacion_vale_es_un_resumen
    check (vale_resumen ~ '^[0-9a-f]{64}$');

-- El correo se guarda ya plegado. Un `check` y no un `trigger`: lo que la base
-- garantiza no depende de que nadie se acuerde de llamar a nada.
alter table iam.invitacion
  add constraint invitacion_correo_normalizado check (correo = lower(correo));

-- Y un vale no vale dos veces aunque dos invitaciones coincidieran.
create unique index if not exists invitacion_vale_unico
  on iam.invitacion (vale_resumen);

comment on column iam.invitacion.vale_resumen is
  'sha256 del vale. El vale solo existe en el correo; aqui vive su resumen.';
