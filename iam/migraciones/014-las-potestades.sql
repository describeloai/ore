-- 014 · LAS POTESTADES — el rol deja de ser un peldaño y pasa a ser un CONJUNTO
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ LO PRIMERO: ESTO NO INVENTA UN VOCABULARIO. YA LO ESCRIBÍAMOS.
--
--   `iam.huella.operacion` guarda el nombre de cada acto, y desde el primer
--   verbo se escribió con la forma `recurso:verbo`. Eso **es** la gramática de
--   una potestad. Había ocho en producción y ninguna tabla que las cerrara:
--
--       organizacion:listar   invitacion:listar    concesion:conceder
--       organizacion:fundar   invitacion:emitir    concesion:revocar
--       miembro:listar        invitacion:redimir
--
--   ⇒ Esta migración no trae un modelo nuevo. Le pone tabla a uno que llevaba
--     un mes escribiéndose a mano.
-- ══════════════════════════════════════════════════════════════════════════
--
-- ── ⛔ POR QUÉ SE VA LA ESCALERA ───────────────────────────────────────────
--
--   `002` ordenó los roles con un `ordinal` para que la guarda del rodeo
--   pudiera comparar: *«nadie otorga un rol más alto que el suyo»*. Con una
--   escalera eso es una resta.
--
--   ⭐ Pero un ordinal **sólo sabe representar «más» y «menos»**, y hay una
--     separación que importa y no es de altura. Es la decisión de su `76` §4, y
--     su motivo es operativo:
--
--       «CORTAR una credencial filtrada es urgente y pasa a las 3 de la
--        mañana. Exigir el rol omnipotente para una emergencia significa que
--        la credencial omnipotente acaba circulando.»
--
--     ¿Es «el que corta» más o menos que «el que da de alta»? La pregunta no
--     tiene respuesta, y un ordinal obliga a inventarla.
--
--   ⇒ La guarda no desaparece: **cambia de forma**. De una resta a un
--     subconjunto — *puedes otorgar un rol si sus potestades están contenidas
--     en las tuyas*. Es más expresivo, se comprueba igual de fácil, y admite un
--     rol que corta sin dar de alta.
--
-- ── ⛔⛔ Y `pertenencia.rol` PASA A SER NULABLE ────────────────────────────
--
--   Era `not null`, así que «pertenecer sin cargo» no se podía decir y hubo que
--   inventar un rol `miembro` para tapar el hueco. Su modelo no lo necesita:
--   pertenecer YA da un estado por defecto, y los roles **añaden** sobre él.
--
--       «Pertenecer ya da lectura. Leer no es un rol.»
--
--   ⇒ `null` significa *pertenece y nada más*. Con eso se van `lector` y
--     `miembro`, que no aportaban ni una potestad de este plano — el único
--     significado de `miembro` era «responde la cola de `review`», y eso es el
--     ÁRBOL, que es el otro plano.

-- ══════════════════════════════════════════════════════════════════════════
-- ① EL CATÁLOGO
-- ══════════════════════════════════════════════════════════════════════════
--
-- ⛔ Lista CERRADA, y en una tabla y no en un `check` por el motivo de `002`:
--   un `check` crece con un `alter table` que cualquiera escribe de paso; un
--   `insert` en una migración es una decisión con fecha y con autor.

create table if not exists iam.potestad (
  nombre   text primary key,
  que_hace text not null,
  -- ⭐ Si el verbo existe HOY. Se guarda en vez de deducirse porque el catálogo
  --   lo lee una pantalla, y enseñar como disponible algo que devuelve 404 es
  --   peor que no enseñarlo.
  ejercida boolean not null default true,
  check (nombre ~ '^[a-z]+:[a-z-]+$')
);

comment on table iam.potestad is
  'Lo que se puede hacer en el plano de la ORGANIZACION. Nada de esto toca el arbol.';

insert into iam.potestad (nombre, que_hace, ejercida) values
  -- Las ocho que ya estaban en la huella.
  ('organizacion:leer',    'ver la organizacion a la que perteneces',            true),
  ('miembro:listar',       'ver quien esta dentro y con que rol',                true),
  ('rol:listar',           'ver quien tiene que rol. Es del estado por defecto', true),
  ('invitacion:listar',    'ver a quien se ha invitado. ⚠️ destapa correos',      true),
  ('invitacion:emitir',    'invitar a alguien',                                  true),
  ('concesion:conceder',   'dar un rol sobre un recurso del arbol',              true),
  ('concesion:revocar',    'quitarlo. No borra: deja la fila y su fecha',        true),
  -- Y las que la base ya soporta y ningun verbo usa. `ejercida = false` para
  -- que la pantalla pueda decir «todavia no» en vez de mentir.
  ('invitacion:revocar',   'retirar una invitacion viva',                        false),
  ('rol:conceder',         'investir a alguien que YA esta dentro',              false),
  ('org:traspasar',        'cambiar de dueño. Solo el dueño, y hay UNO',         false),
  ('actividad:leer-toda',  'leer la huella de los demas, no solo la propia',     false)
on conflict (nombre) do nothing;

-- ══════════════════════════════════════════════════════════════════════════
-- ② LOS ROLES — y `SECURITYADMIN` entra VACÍO, dicho en la propia tabla
-- ══════════════════════════════════════════════════════════════════════════
--
-- ⭐ Se toman sus tres nombres tal cual. Son los del mercado —Snowflake los usa
--   igual— y son los que la plataforma ya escribió: traducirlos daría dos
--   vocabularios para lo mismo, que es justo la ambigüedad que esto quita.
--
-- ⛔⛔ Y SECURITYADMIN ENTRA SIN NI UNA POTESTAD EJERCIDA, A PROPÓSITO.
--
--   Medido: de sus cuatro potestades, dos son de tokens —que emite y revoca
--   KEYCLOAK, y pedirlos exigiría `manage-realm`, medido en 403—, una necesita
--   un concepto de agente que aquí no existe, y `actividad:leer-toda` tiene
--   tabla y no tiene ruta.
--
--   Un rol vacío no significa nada **y además parece que sí**, así que se dice
--   en la columna: quien lo mire ve que hoy es una carcasa y por qué. Se
--   prefiere eso a no tenerlo, porque la separación que representa —cortar sin
--   poder nombrar— es la decisión que no queremos tener que redescubrir.

alter table iam.rol drop column if exists ordinal;
alter table iam.rol add column if not exists nota text;

-- ⛔ Antes de tocar el censo hay que soltar lo que apunta a él. LAS DOS
--   columnas: `pertenencia` e `invitacion`.
--
-- ⚠️⚠️ CORREGIDO EL 2026-09-08, Y LA INMUTABILIDAD SE SUSPENDE UNA VEZ.
--
--   Esto sólo soltaba `pertenencia`, y los `update … set rol = null` de abajo
--   tocaban ADEMÁS `invitacion`, cuyo `not null` no se quitaba hasta la `015`.
--   Contra una base con filas eso revienta:
--
--       ERROR: null value in column "rol" of relation "invitacion"
--
--   ⭐ Y CI no podía verlo: su base nace vacía en cada vuelta, asi que el
--     `update` no tocaba ni una fila. **Una suite de migraciones que sólo corre
--     contra una base vacía no prueba migraciones: prueba sintaxis.** Lo
--     descubrió el clúster, que sí tenía invitaciones de las pruebas.
--
--   ⇒ Se edita en vez de escribir una `016` porque la `016` no llegaría a
--     correr nunca: la `014` falla antes. Y se puede: el libro del clúster se
--     queda en la `013` —comprobado— así que **ninguna base que sobreviva ha
--     corrido ésta**. La regla protege a las que ya la corrieron, y no hay
--     ninguna. La próxima vez que alguien quiera editar una aplicada, la
--     respuesta es NO.
alter table iam.pertenencia alter column rol drop not null;
alter table iam.invitacion  alter column rol drop not null;

insert into iam.rol (nombre, que_puede, nota) values
  ('ORGADMIN', 'ademas traspasa la organizacion. Es UNO',
   'Nuestro, no suyo: su `019` no tiene restriccion de unicidad. Existe porque alguien tiene que poder quedarse sin administradores y aun asi entrar.'),
  ('ACCOUNTADMIN', 'todo. Y es el UNICO que concede roles',
   'Repartir poder no se delega. Se usa poco, a proposito.'),
  ('USERADMIN', 'quien esta: invita, retira invitaciones',
   'NO concede roles, y por eso no puede ascenderse a si mismo.'),
  ('SECURITYADMIN', 'vigila y corta',
   'CARCASA HOY: ninguna de sus potestades se ejerce todavia. Entra igual porque la separacion que representa —cortar sin poder nombrar— es una decision que no queremos redescubrir a las 3 de la mañana.')
on conflict (nombre) do update set que_puede = excluded.que_puede, nota = excluded.nota;

-- ⚠️⚠️ Y EL REMAPEO VA AQUI, DESPUES DEL `insert` Y ANTES DEL `delete`.
--
--   Estaba ARRIBA, antes de crear los roles nuevos, y contra una base con filas
--   eso revienta:
--
--       ERROR: Key (rol)=(ORGADMIN) is not present in table "rol"
--
--   Es el SEGUNDO fallo de orden de esta migracion y de la misma familia que el
--   primero: los dos invisibles para CI, porque durante las migraciones su base
--   esta vacia y ningun `update` toca una fila. El orden correcto lo dicta la
--   clave ajena — crear, remapear, borrar — y no se puede saltar ninguno.
--
-- `null` = pertenece y nada mas. Los dos roles que no aportaban potestades de
-- este plano se convierten en eso, que es lo que siempre significaron.
update iam.pertenencia set rol = null where rol in ('lector', 'miembro');
update iam.invitacion  set rol = null where rol in ('lector', 'miembro');
update iam.pertenencia set rol = 'ACCOUNTADMIN' where rol = 'administrador';
update iam.invitacion  set rol = 'ACCOUNTADMIN' where rol = 'administrador';
update iam.pertenencia set rol = 'ORGADMIN'     where rol = 'dueno';
update iam.invitacion  set rol = 'ORGADMIN'     where rol = 'dueno';

delete from iam.rol where nombre in ('lector', 'miembro', 'administrador', 'dueno');

-- ⛔ El dueño sigue siendo UNO. El indice de `005` apuntaba al nombre viejo.
drop index if exists iam.pertenencia_un_dueno;
create unique index if not exists pertenencia_un_orgadmin
  on iam.pertenencia (organizacion) where rol = 'ORGADMIN';

-- ══════════════════════════════════════════════════════════════════════════
-- ③ QUÉ AÑADE CADA ROL — y lo que da PERTENECER, que no es un rol
-- ══════════════════════════════════════════════════════════════════════════

create table if not exists iam.rol_potestad (
  rol      text not null references iam.rol(nombre)      on delete cascade,
  potestad text not null references iam.potestad(nombre) on delete cascade,
  primary key (rol, potestad)
);

-- ⭐ Lo que da PERTENECER va en una vista y no en un rol, y su argumento es
--   suyo y vale entero: *«pertenecer ya da lectura. Leer no es un rol»*.
--   Esconder quien manda seria seguridad por oscuridad (`76` §2).
create or replace view iam.por_defecto as
  select nombre as potestad from iam.potestad
   where nombre in ('organizacion:leer', 'miembro:listar', 'rol:listar');

insert into iam.rol_potestad (rol, potestad) values
  ('USERADMIN', 'invitacion:listar'),
  ('USERADMIN', 'invitacion:emitir'),
  ('USERADMIN', 'invitacion:revocar'),

  ('ACCOUNTADMIN', 'invitacion:listar'),
  ('ACCOUNTADMIN', 'invitacion:emitir'),
  ('ACCOUNTADMIN', 'invitacion:revocar'),
  ('ACCOUNTADMIN', 'concesion:conceder'),
  ('ACCOUNTADMIN', 'concesion:revocar'),
  -- ⛔ ÚNICO. Repartir poder no se delega.
  ('ACCOUNTADMIN', 'rol:conceder'),

  ('SECURITYADMIN', 'actividad:leer-toda'),

  ('ORGADMIN', 'invitacion:listar'),
  ('ORGADMIN', 'invitacion:emitir'),
  ('ORGADMIN', 'invitacion:revocar'),
  ('ORGADMIN', 'concesion:conceder'),
  ('ORGADMIN', 'concesion:revocar'),
  ('ORGADMIN', 'rol:conceder'),
  ('ORGADMIN', 'actividad:leer-toda'),
  -- ⛔ SUYA Y DE NADIE MAS. Cambiar de dueño no es administrar: es traspasar.
  ('ORGADMIN', 'org:traspasar')
on conflict do nothing;

-- ⭐ Lo efectivo de cada rol, para no repetir la union en cada consulta.
create or replace view iam.potestades_de_rol as
  select r.nombre as rol, p.potestad
    from iam.rol r
    cross join iam.por_defecto p
   union
  select rol, potestad from iam.rol_potestad;

-- ══════════════════════════════════════════════════════════════════════════
-- ④ LOS INVARIANTES — comprobados AQUÍ, no en una prueba que alguien borre
-- ══════════════════════════════════════════════════════════════════════════
--
-- Es la figura de su `potestades.mjs`, que los comprueba al cargar el módulo:
-- *«un catálogo de potestades incoherente autoriza mal desde la primera
-- petición»*. Aquí van en la migración, que es donde no se pueden saltar.

do $$
declare huerfanas text; sobran text;
begin
  -- ⭐ ORGADMIN tiene que cubrirlo TODO. Si una potestad nueva se le olvida a
  --   alguien, el omnipotente deja de serlo — y el sintoma seria «el dueño no
  --   puede hacer X», que manda a mirar al sitio equivocado.
  select string_agg(p.nombre, ', ') into huerfanas
    from iam.potestad p
   where p.nombre not in (select potestad from iam.potestades_de_rol where rol = 'ORGADMIN');
  if huerfanas is not null then
    raise exception 'ORGADMIN no cubre: %', huerfanas;
  end if;

  -- ⛔ Y nadie puede quedar con un rol que ya no existe.
  select string_agg(distinct rol, ', ') into sobran
    from iam.pertenencia
   where rol is not null and rol not in (select nombre from iam.rol);
  if sobran is not null then
    raise exception 'quedan pertenencias con roles inexistentes: %', sobran;
  end if;
end $$;
