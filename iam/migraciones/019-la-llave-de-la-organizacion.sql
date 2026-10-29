-- 019 · CÓMO SE LLAMA LA LLAVE MAESTRA DE UNA ORGANIZACIÓN
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ ES LA MISMA FIGURA QUE LA `017`, Y NO SE PARECE: ES LA MISMA
--
--   Allí: cómo se llama su ÁRBOL —`<propietario>/<repositorio>`—, no dónde vive
--   ni si existe. Aquí: cómo se llama su LLAVE —`<llavero>/<clave>`—, no dónde
--   vive ni de qué nube es.
--
--   El proyecto y la región son la carretera y cambian; el nombre es la
--   identidad y no. Es la distinción que `50-jwks.yaml` hizo primero entre
--   `EMISOR` y `DIRECCION`, y la tercera vez que este árbol la escribe.
--
-- ── ⛔⛔ Y ESTA COLUMNA ES EL SEGUNDO CERROJO ─────────────────────────────
--
--   La `0023` decidió cifrado de sobre: una DEK por secreto, envuelta por una
--   KEK. Lo que hace que eso sea una separación y no un adorno es que **la KEK
--   sea POR ORGANIZACIÓN**, y ésta es la columna que lo dice.
--
--   ⇒ Con ella, la tabla del material puede ser compartida sin peligro: el
--     custodio de un inquilino puede LEER el cifrado de otro y **no puede
--     abrirlo**, porque su cuenta de Google no puede usar esa clave.
--
--   ⭐ Y el cerrojo no lo ponemos nosotros: lo pone el IAM de la nube sobre una
--     llave. Es la misma figura que el testigo de la forja —donde lo que ata no
--     es el ámbito del token sino de quién es y de qué es colaborador— aplicada
--     a una llave en vez de a un repositorio.
--
-- ── ⚠️ Y por eso `unique`, sin predicado ──────────────────────────────────
--
--   Dos organizaciones compartiendo KEK sería el cerrojo abierto sin que nada
--   lo dijera: cualquiera de las dos abriría lo de la otra, y el `select` que
--   lo demostrara nunca lo mira nadie. Incluye a las retiradas, por lo mismo
--   que el árbol: una lápida conserva su material, y reciclar su llave
--   permitiría abrirlo.
--
-- 📎 `docs/decisions/0023-donde-vive-un-secreto.md`
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.organizacion
  add column if not exists kek text;

-- Lo que ya se creó en el clúster para las dos que hay: el llavero `ore` y una
-- clave por organización, con rotación a 90 días. El relleno pone por escrito
-- la correspondencia que si no viviría en la cabeza de alguien.
update iam.organizacion
   set kek = 'ore/' || nombre
 where kek is null;

alter table iam.organizacion
  alter column kek set not null;

alter table iam.organizacion
  add constraint organizacion_kek_unica unique (kek);

-- ⛔ El mismo alfabeto cerrado que el árbol, y por el mismo motivo: este valor
--   acaba compuesto dentro del nombre de un recurso de la nube. `..` no es un
--   caso especial que haya que recordar — es algo que el alfabeto no admite.
alter table iam.organizacion
  add constraint organizacion_kek_forma
    check (kek ~ '^[a-z0-9][a-z0-9_-]{0,62}/[a-z0-9][a-z0-9_-]{0,62}$');

comment on column iam.organizacion.kek is
  'Como se llama su llave maestra —`<llavero>/<clave>`—, NO donde vive ni de que nube es.';

-- ══════════════════════════════════════════════════════════════════════════
-- ⚠️ LO QUE ESTA MIGRACIÓN NO AFIRMA
-- ══════════════════════════════════════════════════════════════════════════
--
-- ⛔ No dice que la llave EXISTA. Igual que la `017` con el árbol: la fila es la
--   verdad y el recurso converge hacia ella. Quien llegue antes de que exista se
--   encuentra un error de la nube con su nombre puesto, no un silencio.
--
-- ⛔ Y no guarda ni un byte de material. Sigue sin haber almacén: esto es el
--   NOMBRE de la llave con la que otro proceso —que todavía no existe— envolverá
--   claves de datos. `iam` no puede abrir nada y no debe poder.
--
-- ⚠️ Y una consecuencia que hay que mirar: el día que un cliente traiga SU
--   llave, esta columna pasa a nombrar un llavero que no es el nuestro. La forma
--   lo admite —`<llavero>/<clave>`— y el resto de la fila no cambia. Era la
--   mitad de la gracia del cifrado de sobre.
