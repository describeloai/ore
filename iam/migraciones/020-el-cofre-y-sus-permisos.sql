-- 020 · EL COFRE, Y LO QUE HACE QUE ESTÉ SEPARADO
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⛔⛔ MEDIDO ANTES DE ESCRIBIR: LA SEPARACIÓN NO ERA EXPRESABLE
--
--   La `0023` dice que `iam` dice quién puede y **no guarda nada**, y que el
--   material vive en otro sitio. Al ir a escribirlo salió que en esta base hay
--   **UN SOLO ROL**, `keycloak`, y es SUPERUSER con `Bypass RLS`. Lo usan
--   Keycloak, `ore-iam` y el runner de migraciones.
--
--   ⇒ Con un superusuario, un `grant` no separa nada: los salta todos. Poner el
--     material en otro esquema y quedarnos tranquilos habría sido llamar
--     separación a una convención de nombres.
--
--   Así que esto no crea sólo un esquema: crea los PAPELES que hacen que el
--   esquema signifique algo.
--
-- ── ⭐ Papeles sin `login`, y las credenciales fuera ───────────────────────
--
--   `ore_iam` y `ore_cofre` son papeles de GRUPO: no entran, se conceden. El
--   usuario que de verdad se conecta lo crea el operador con su contraseña, y
--   recibe uno de los dos.
--
--   ⇒ Así la POLÍTICA vive en git —quién puede tocar qué, revisable— y la
--     CREDENCIAL vive en un `Secret`. Es el mismo corte que el árbol hace con
--     `connectionEnv`: el documento dice dónde está el secreto, no cuál es.
--
-- ── ⭐⭐ Y LA ASIMETRÍA ES EL DISEÑO ──────────────────────────────────────
--
--       ore_iam    →  todo `iam`.  Y sobre `cofre`, NADA.
--       ore_cofre  →  todo `cofre`. Y sobre `iam`, SÓLO LEER lo que necesita
--                     para autorizar: la concesión viva y la llave de la
--                     organización.
--
--   No es simétrica a propósito. `ore-iam` autoriza y tiene que ser incapaz de
--   abrir; el custodio abre y necesita saber si puede. Que uno lea del otro y no
--   al revés es exactamente la frontera de la `0023`.
--
-- 📎 `docs/decisions/0023-donde-vive-un-secreto.md`
-- ══════════════════════════════════════════════════════════════════════════

-- ══════════════════════════════════════════════════════════════════════════
-- ① LOS PAPELES
-- ══════════════════════════════════════════════════════════════════════════
-- `create role` no admite `if not exists`, y los papeles son del SERVIDOR y no
-- de la base: si otra base los creó, aquí ya están.
do $$
begin
  if not exists (select 1 from pg_roles where rolname = 'ore_iam') then
    create role ore_iam nologin;
  end if;
  if not exists (select 1 from pg_roles where rolname = 'ore_cofre') then
    create role ore_cofre nologin;
  end if;
end $$;

comment on role ore_iam is
  'Quien DICE quien puede. No alcanza `cofre`: no debe poder abrir nada.';
comment on role ore_cofre is
  'Quien ABRE. Lee de `iam` lo justo para autorizar, y nada mas.';

-- ══════════════════════════════════════════════════════════════════════════
-- ② EL ESQUEMA, Y LO QUE GUARDA
-- ══════════════════════════════════════════════════════════════════════════
create schema if not exists cofre;

-- ⛔ Lo primero, antes de que haya nada dentro: `public` no entra. En Postgres,
--   `public` es un grupo al que pertenece todo el mundo, y un esquema recién
--   creado le da `usage` si no se lo quitas.
revoke all on schema cofre from public;

-- ── El CATÁLOGO. Nombres, no valores ──────────────────────────────────────
--
-- ⚠️ Vive aquí y no en `iam` a propósito, aunque `concesion.recurso` nombre
--   `secreto/<nombre>`: `iam` concede sobre recursos que no ve —ya lo hacía con
--   las vistas del árbol— y saber QUÉ secretos existen es del custodio.
create table if not exists cofre.secreto (
  id           text        primary key,
  organizacion text        not null references iam.organizacion(id) on delete cascade,
  -- El mismo nombre que aparece en `concesion.recurso` como `secreto/<nombre>`.
  -- Mismo alfabeto que la `018` exige alli: si no fueran el mismo, una concesion
  -- no alcanzaria al secreto que cree que alcanza.
  nombre       text        not null
                 check (nombre ~ '^[a-z0-9][a-z0-9_-]{0,62}$'),
  clase        text        not null
                 check (clase in ('contrasena', 'testigo', 'clave-api', 'conexion')),
  emitio       text        not null references iam.persona(id),
  en           timestamptz not null default now(),
  -- ⭐ `retirado_en` y no un `delete`: un secreto retirado tiene que seguir
  --   contando que existio, quien lo emitio y hasta cuando valio. Es el mismo
  --   argumento que `concesion.revocada_en` — sin la fila, «nunca existio» y
  --   «lo quitamos» son indistinguibles.
  retirado_en  timestamptz,
  retiro       text        references iam.persona(id),

  check ((retirado_en is null) = (retiro is null)),
  -- ⛔ Unico DENTRO de su organizacion, no en el mundo. Dos clientes pueden
  --   llamar `pg-produccion` a lo suyo, y tienen razon los dos.
  unique (organizacion, nombre)
);

-- ── El MATERIAL. Una fila POR VERSIÓN ─────────────────────────────────────
--
-- ⭐ Rotar es INSERTAR, no sustituir. Y no es un lujo: rotar una credencial y
--   que lo que ya estaba conectado siga funcionando hasta que se recicle es la
--   diferencia entre una rotación y una caída. Guardar la anterior es además lo
--   que permite decir «esta versión se usó hasta el martes».
create table if not exists cofre.material (
  secreto      text        not null references cofre.secreto(id) on delete cascade,
  version      integer     not null check (version > 0),
  -- El valor, cifrado con una DEK propia de ESTA version.
  cifrado      bytea       not null,
  -- Y la DEK, envuelta por la KEK de la organizacion. La KEK no esta aqui:
  --   `iam.organizacion.kek` dice COMO SE LLAMA, y abrirla es de otro proceso.
  dek_envuelta bytea       not null,
  -- Con que se envolvio, por si la organizacion cambia de llave: una fila vieja
  -- se abre con la llave con la que se cerro, no con la de ahora.
  kek          text        not null,
  alg          text        not null default 'AES-256-GCM',
  en           timestamptz not null default now(),

  primary key (secreto, version)
);

-- Lo que vale AHORA. Se pregunta a la vista, no a la tabla: la tabla guarda
-- tambien lo que valio, que es lo que hace auditable una rotacion.
create or replace view cofre.vigente as
  select m.*
    from cofre.material m
    join (select secreto, max(version) as version
            from cofre.material group by secreto) u
      on u.secreto = m.secreto and u.version = m.version;

comment on table cofre.secreto is
  'El catalogo: nombres, quien los emitio y cuando. NI UN VALOR.';
comment on table cofre.material is
  'El valor cifrado y su DEK envuelta. Sin la KEK —que esta en otro sitio— es ruido.';

-- ══════════════════════════════════════════════════════════════════════════
-- ③ LOS PERMISOS, Y SON LO QUE ESTA MIGRACIÓN VIENE A HACER
-- ══════════════════════════════════════════════════════════════════════════

-- ── `ore_iam`: todo lo suyo, y del cofre NADA ─────────────────────────────
grant usage on schema iam to ore_iam;
grant select, insert, update, delete on all tables in schema iam to ore_iam;
grant usage, select on all sequences in schema iam to ore_iam;
-- ⛔ Y lo que venga despues, tambien. Sin esto, la tabla que añada la `021`
--   nace inalcanzable y el fallo aparece en produccion, no aqui.
alter default privileges in schema iam
  grant select, insert, update, delete on tables to ore_iam;
alter default privileges in schema iam
  grant usage, select on sequences to ore_iam;

-- ⭐ Y NADA sobre `cofre`. No hace falta escribirlo —no conceder es no poder—
--   pero se deja dicho porque es LA PROPIEDAD que esta migracion existe para
--   crear, y una propiedad que solo se sostiene por una ausencia hay que
--   nombrarla o nadie sabe que estaba ahi.

-- ── `ore_cofre`: todo el cofre, y de `iam` lo justo ───────────────────────
grant usage on schema cofre to ore_cofre;
grant select, insert, update, delete on all tables in schema cofre to ore_cofre;
alter default privileges in schema cofre
  grant select, insert, update, delete on tables to ore_cofre;

grant usage on schema iam to ore_cofre;
-- ⭐ SOLO LEER, y solo esto. Lo que necesita para contestar «¿puede este sujeto
--   abrir este secreto?» y para saber con que llave envolver:
grant select on iam.concesion_viva          to ore_cofre;  -- ¿tiene `usar`?
grant select on iam.rol_de_recurso          to ore_cofre;  -- ¿que significa?
grant select on iam.organizacion            to ore_cofre;  -- su `kek`
grant select on iam.persona                 to ore_cofre;  -- quien es quien pide
grant select on iam.potestades_de_persona   to ore_cofre;  -- ¿puede emitir?
-- Y la huella se ESCRIBE: un custodio que abriera sin dejar rastro seria peor
-- que uno que no abre.
grant insert on iam.huella to ore_cofre;
grant usage, select on sequence iam.huella_id_seq to ore_cofre;

-- ══════════════════════════════════════════════════════════════════════════
-- ⚠️ LO QUE ESTA MIGRACIÓN NO PUEDE HACER, Y HAY QUE HACER FUERA
-- ══════════════════════════════════════════════════════════════════════════
--
-- Crear los usuarios que de verdad se conectan, con su contraseña, y darles uno
-- de los dos papeles. Eso no va en git:
--
--     create user iam_app   login password '…';  grant ore_iam   to iam_app;
--     create user cofre_app login password '…';  grant ore_cofre to cofre_app;
--
-- ⛔ Y mientras `ore-iam` siga conectándose como `keycloak` —SUPERUSER— esta
--   separación está escrita y **no está en vigor**. La migración la deja lista;
--   ponerla en vigor es cambiar una cadena de conexión, y eso se hace y se
--   comprueba, no se supone.
