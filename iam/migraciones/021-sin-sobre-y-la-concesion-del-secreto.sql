-- 021 · EL SOBRE SOBRABA, Y LA CONCESIÓN QUE NACE CON EL SECRETO
--
-- ══════════════════════════════════════════════════════════════════════════
-- ① ⛔⛔ FUERA EL SOBRE — y lo destapó ir a escribir el binario
-- ══════════════════════════════════════════════════════════════════════════
--
--   La `0023` decidió cifrado de sobre: una DEK por secreto, envuelta por la
--   KEK. Al ir a implementarlo salió lo que la decisión no había mirado:
--
--       generar una DEK y cifrar con ella **es escribir criptografía nuestra**.
--
--   Y este árbol tiene una frase para eso, escrita dos veces —en `ore-core` para
--   Ed25519 y en `ore-entrada` para RSA—:
--
--     *«el modo de fallo de equivocarse en el relleno no es que las firmas dejen
--      de verificar: es que verifiquen firmas inválidas, en silencio y en la
--      dirección insegura»*
--
--   ⇒ Un AES-GCM mal usado —un nonce repetido, un tag que no se comprueba— falla
--     exactamente igual: sigue funcionando y deja de proteger.
--
-- ── Y resulta que no hacía falta ──────────────────────────────────────────
--
--   El sobre existe para dos cosas: cifrar payloads grandes, y rotar la maestra
--   sin descifrar. Ninguna de las dos aplica:
--
--     · una contraseña, un token o una cadena de conexión caben de sobra en los
--       64 KiB que un KMS simétrico cifra directamente;
--     · y rotar en un KMS crea una VERSIÓN NUEVA: lo cifrado con la anterior se
--       sigue abriendo solo, sin re-envolver nada.
--
--   ⇒ Así que el material se cifra **directamente con la KEK de la
--     organización**, y no queda ni una línea de criptografía de nuestra parte.
--     Las dos propiedades que se compraron siguen en pie: el material y la llave
--     no están en el mismo sitio, y la llave es por organización.
--
-- ⚠️ Se corrige con una migración y no editando la `020` porque la `020` ya está
--   publicada. El libro guarda su huella, y una migración que cambia después de
--   aplicarse deja de ser un registro.
--
-- 📎 Se anota también en `docs/decisions/0023-donde-vive-un-secreto.md`.

-- ⚠️ La vista se tira y se rehace, no se reemplaza. `cofre.vigente` es un
--   `select m.*`, así que **depende de la columna** y Postgres se niega a
--   quitarla debajo: *«cannot drop column … because other objects depend on
--   it»*. Y `create or replace view` tampoco vale — no admite que cambie la
--   lista de columnas. Es la misma vuelta que dio la `011` con `concesion_viva`.
drop view if exists cofre.vigente;

alter table cofre.material drop column if exists dek_envuelta;

create or replace view cofre.vigente as
  select m.*
    from cofre.material m
    join (select secreto, max(version) as version
            from cofre.material group by secreto) u
      on u.secreto = m.secreto and u.version = m.version;

grant select, insert, update, delete on cofre.vigente to ore_cofre;

alter table cofre.material alter column alg set default 'GOOGLE-KMS-SIMETRICA';
update cofre.material set alg = 'GOOGLE-KMS-SIMETRICA' where alg = 'AES-256-GCM';

comment on table cofre.material is
  'El valor cifrado con la KEK de su organizacion. Sin esa llave es ruido.';
comment on column cofre.material.kek is
  'Con QUE llave se cerro. Una fila vieja se abre con la suya, no con la de ahora.';

-- ══════════════════════════════════════════════════════════════════════════
-- ② LA CONCESIÓN QUE NACE CON EL SECRETO — y por qué no es un `grant` más
-- ══════════════════════════════════════════════════════════════════════════
--
--   La `0023` decidió que **quien emite queda `owner` de lo que emitió**, y no
--   por generosidad: un secreto que nace sin nadie que pueda darlo no se lo
--   puede dar nadie nunca. Es la organización huérfana otra vez.
--
--   ⇒ Eso obliga a que el custodio escriba en `iam.concesion` — y ahí está el
--     problema: darle `insert` sobre esa tabla le permitiría conceder **lo que
--     quisiera**, incluidas vistas del árbol, que no son asunto suyo.
--
-- ── ⭐⭐ Así que no se le da `insert`: se le da UNA PUERTA ────────────────
--
--   Una función `security definer` que corre con los permisos de quien la
--   escribió —el operador— y que **sólo sabe insertar concesiones de secreto**.
--   El custodio puede llamarla y no puede pasar por su lado.
--
--   Es la misma figura que el testigo de la forja: lo que ata no es el ámbito
--   declarado, es que no exista el camino. Aquí el camino es una función con un
--   `if` dentro, y ese `if` lo comprueba la base y no una revisión.
--
-- ⚠️ `set search_path` no es adorno en una función `security definer`: sin él,
--   quien la llama puede anteponer un esquema suyo y hacer que `insert into
--   iam.concesion` apunte a una tabla que él controla. Es la escalada clásica de
--   Postgres, y se cierra en la declaración.
create or replace function iam.conceder_de_secreto(
  p_id           text,
  p_sujeto       text,
  p_recurso      text,
  p_rol          text,
  p_concedio     text,
  p_organizacion text
) returns void
language plpgsql
security definer
set search_path = pg_catalog, iam
as $funcion$
begin
  -- ⛔ La puerta. Sin esto, esta funcion seria `insert` con otro nombre.
  if p_recurso not like 'secreto/%' then
    raise exception
      'esta funcion solo concede sobre secretos, y se le paso `%`', p_recurso
      using hint = 'lo demas se concede por `POST /organizaciones/{org}/concesiones`';
  end if;
  insert into iam.concesion (id, sujeto, recurso, rol, concedio, organizacion)
  values (p_id, p_sujeto, p_recurso, p_rol, p_concedio, p_organizacion);
end;
$funcion$;

comment on function iam.conceder_de_secreto(text, text, text, text, text, text) is
  'La UNICA forma que tiene el custodio de escribir una concesion, y solo de secreto.';

-- `public` es un grupo al que pertenece todo el mundo, y una funcion nace
-- ejecutable por el si no se le quita.
revoke all on function iam.conceder_de_secreto(text, text, text, text, text, text) from public;
grant execute on function iam.conceder_de_secreto(text, text, text, text, text, text) to ore_cofre;
