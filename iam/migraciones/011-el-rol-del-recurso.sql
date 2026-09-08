-- 011 · EL ROL DEL RECURSO — el plano de abajo deja de compartir vocabulario
--
-- ── ⛔⛔ QUÉ ESTABA MAL, y era UNA LÍNEA de la `007` ─────────────────────────
--
--       papel text not null references iam.papel(nombre)
--
--   La concesión —sobre `ventas.Clientes`— referenciaba **la misma tabla** que la
--   pertenencia —sobre la organización—. Dos planos, un vocabulario, y encima
--   ordenado. Y las dos consecuencias estaban vivas en el clúster:
--
--   ① se podía conceder `dueno` sobre una tabla. El índice único parcial que
--      garantiza UN dueño por organización protege `pertenencia`; no alcanza a
--      `concesion`. La clave ajena estaba satisfecha y la guarda pasaba.
--
--   ② `no_por_encima` comparaba mi ordinal en `pertenencia` contra el ordinal de
--      lo que doy en `concesion`. Dos escalas distintas sobre la misma recta
--      numérica: parecía funcionar porque los números existen, no porque
--      significaran lo mismo.
--
-- ── ⭐ POR QUÉ `owner` Y NO `dueno` ─────────────────────────────────────────
--
--   Los dos planos tienen dueño y NO son el mismo:
--
--     arriba   UNO por organización · se traspasa · da invitar y conceder
--     abajo    MUCHOS, uno por ámbito · se nombra · da LA FIRMA
--
--   Separar las tablas arregla el motor. La confusión sobrevive en la prosa, en
--   una pantalla y en una conversación mientras la palabra sea la misma — con
--   dos palabras no queda nada que recordar. Y `owner` es además la que usan los
--   catálogos del mercado para exactamente esto.
--
-- ── ⛔⛔ SIN `ordinal`, Y NO ES UN OLVIDO ───────────────────────────────────
--
--   `owner` **no implica** `lector`. Es de `modelo/puerta/concesion.mjs`, y su
--   motivo es que escribirlo sería herencia de roles: *«la travesía deja de ser
--   un JOIN sobre un árbol y empieza a ser un motor de políticas»*. La salida
--   fue **dos hechos, no una regla** — al declarar una conexión se emiten DOS
--   concesiones.
--
--   ⇒ Una columna `ordinal` aquí **es** esa regla implícita. Ordenar
--     `lector < owner` sería escribir `owner ⇒ lector` en una columna. Esta
--     tabla tiene dos filas y **ninguna es más alta que la otra**.
--
-- ── ⚠️ LO QUE ESTA MIGRACIÓN NO TRAE ───────────────────────────────────────
--
--   El cierre de contención de la plataforma recorre `origen → contenedor →
--   dataset`, que es la forma de una AMP y **no se migra** — ya lo dice el
--   README. El nuestro es `paquete → vista` y vive en la forja, no aquí.
--
--   La guarda de verdad de `conceder` —*«para nombrar owner de un ámbito hay que
--   ser owner de ese ámbito, o de uno que lo contenga»*— necesita esa travesía.
--   Hasta que exista, **`conceder` niega `owner`**: omitir es cerrar, no abrir.

create table if not exists iam.rol_de_recurso (
  nombre    text primary key,
  que_puede text not null
);

comment on table iam.rol_de_recurso is
  'El rol SOBRE UN RECURSO. Sin ordinal a proposito: owner no implica lector.';

insert into iam.rol_de_recurso (nombre, que_puede) values
  ('lector', 've el recurso y no lo cambia'),
  ('owner',  'firma la certificacion, y nombra owner dentro de su ambito')
on conflict (nombre) do nothing;

-- ⛔ Antes de reapuntar: ninguna fila puede quedar colgando. Si alguna concesión
--   lleva un papel del plano de arriba, esta migración **para** — reapuntar la
--   clave ajena la dejaría apuntando a nada, y el error saldría meses después.
do $$
declare huerfanos text;
begin
  select string_agg(distinct c.papel, ', ') into huerfanos
    from iam.concesion c
   where c.papel not in (select nombre from iam.rol_de_recurso);
  if huerfanos is not null then
    raise exception
      'hay concesiones con roles que no son de recurso: %. Reapuntar la clave ajena las dejaria colgando',
      huerfanos;
  end if;
end $$;

-- La columna se llama `rol`, como el estándar, y porque **viaja aguas abajo**:
-- el mismo nombre desde el token hasta quien consume. Traducirlo en cada
-- frontera es una oportunidad de equivocarse por frontera.
alter table iam.concesion rename column papel to rol;

-- La clave ajena vieja se suelta por su definición y no por su nombre: el nombre
-- lo puso Postgres y no es parte de ningún contrato.
do $$
declare vieja text;
begin
  select con.conname into vieja
    from pg_constraint con
    join pg_class t     on t.oid = con.conrelid
    join pg_namespace n on n.oid = t.relnamespace
   where n.nspname = 'iam' and t.relname = 'concesion' and con.contype = 'f'
     and pg_get_constraintdef(con.oid) like '%iam.papel(nombre)%';
  if vieja is not null then
    execute format('alter table iam.concesion drop constraint %I', vieja);
  end if;
  if not exists (select 1 from pg_constraint where conname = 'concesion_rol_de_recurso_fkey') then
    alter table iam.concesion
      add constraint concesion_rol_de_recurso_fkey
      foreign key (rol) references iam.rol_de_recurso(nombre);
  end if;
end $$;

-- ⛔ La vista se BORRA y se vuelve a crear, no se reemplaza. `create or replace
--   view` exige los mismos nombres de columna, así que dejaría la vista
--   sirviendo `papel` sobre una tabla que ya dice `rol` — y nadie se enteraría
--   hasta que alguien leyera la vista buscando la columna nueva.
drop view if exists iam.concesion_viva;
create view iam.concesion_viva as
  select * from iam.concesion
   where revocada_en is null
     and desde <= now()
     and (hasta is null or hasta > now());
