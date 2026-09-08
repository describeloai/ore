-- 009 · EL ÁMBITO DE LA CONCESIÓN — un hueco de la `007`, cerrado por delante
--
-- ⛔⛔ Y NO SE ARREGLA EDITANDO LA `007`. Está aplicada, y el runner se niega:
--   *«lo que corrió y lo que dice el fichero dejan de ser lo mismo»*. Corregir
--   no es editar, es escribir la siguiente. Ésta es la primera vez que esa
--   regla cobra, y funciona exactamente como se probó el día que se escribió.
--
-- ── Qué faltaba ─────────────────────────────────────────────────────────────
--
--   `007` guarda `(sujeto, recurso, papel)` y **no dice de qué organización es
--   el recurso**. Con eso nadie puede contestar quién está autorizado a
--   conceder sobre él: la guarda necesita saber en qué organización mirar el
--   papel de quien concede.
--
--   Sin esta columna, `conceder` sólo podía comprobar «¿eres alguien?», que es
--   literalmente el agujero que su `019` censó: *«la única comprobación era
--   ¿eres de esta organización?, así que cualquier miembro veía la lista
--   completa»*.
--
-- ⚠️ `not null` sobre una tabla con filas exigiría un valor por defecto, y un
--   valor por defecto aquí sería adivinar de quién es un permiso. La tabla está
--   vacía —se comprueba abajo—, así que se puede exigir de verdad.

do $$
begin
  if exists (select 1 from iam.concesion) then
    raise exception 'iam.concesion tiene filas: esta migracion asumia que estaba vacia';
  end if;
end $$;

alter table iam.concesion
  add column organizacion text not null references iam.organizacion(id) on delete cascade;

create index if not exists concesion_por_organizacion
  on iam.concesion (organizacion, recurso);

comment on column iam.concesion.organizacion is
  'En que organizacion mirar el papel de quien concede. Sin esto, conceder solo puede comprobar que quien pide existe.';

-- Y la vista se rehace: una vista no es una tabla, no guarda nada, y por eso
-- SÍ se puede reemplazar sin mentirle a nadie.
create or replace view iam.concesion_viva as
  select * from iam.concesion
   where revocada_en is null
     and desde <= now()
     and (hasta is null or hasta > now());
