-- 018 · EL SECRETO ENTRA EN EL MODELO DE ROLES QUE YA HABÍA
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ NO SE INVENTA UN PARADIGMA. Se usan los dos planos que ya existen
--
--   La frase que abrió esto resultó ser el modelo entero:
--
--     «cualquier usuario con suficiente rol puede EMITIR un secreto; ahora
--      LEERLO o USARLO ya es otra historia»
--
--   Y es exactamente el corte que la `011` peleó y ganó:
--
--     EMITIR    no habla de ningún secreto concreto —todavía no existe— así que
--               es una POTESTAD de la organización, como `invitacion:emitir`
--     LEER/USAR hablan de ESE secreto y de ninguno más: una CONCESIÓN
--
--   ⭐ Y como entre los dos planos NO hay herencia, la asimetría que hace seguro
--     este producto sale sola: **quien puede crear secretos no queda con derecho
--     sobre los que ya había**. Un almacén donde el permiso de crear arrastra el
--     de leer es un almacén con una sola cerradura.
--
-- 📎 `docs/decisions/0023-donde-vive-un-secreto.md`. Aquí va sólo la mitad que
--    NO necesita custodio: `iam` no tiene que saber nada del material para saber
--    quién puede.
-- ══════════════════════════════════════════════════════════════════════════

-- ══════════════════════════════════════════════════════════════════════════
-- ① `usar` — Y NO ES `lector`
-- ══════════════════════════════════════════════════════════════════════════
--
--   lector   VE EL VALOR             una persona que copia una contraseña
--   usar     lo RESUELVE SIN VERLO   un Job que se conecta
--   owner    lo rota, lo revoca y lo concede
--
-- ⛔ Y la mayoría del acceso a un secreto tiene que ser `usar`. Si conectarse
--   exigiera poder leer la contraseña, cada permiso de ejecución arrastraría uno
--   de lectura — y la auditoría no podría distinguir «se conectó» de «se la
--   llevó», que es la única pregunta que importa el día que hay un incidente.
--
-- ⭐ Cabe insertando UNA FILA, y eso no es suerte: `iam.rol_de_recurso` se creó
--   **sin ordinal a propósito**. `usar` no implica `lector` por la misma razón
--   por la que `owner` no lo implica — *dos hechos, no una regla*. Con una
--   escalera, meter un peldaño en medio habría obligado a decidir qué hay encima
--   de qué, y esa decisión no la habría tomado nadie: la habría tomado el orden.
insert into iam.rol_de_recurso (nombre, que_puede) values
  ('usar', 'resuelve el valor SIN verlo. Es lo que hace un proceso que se conecta')
on conflict (nombre) do nothing;

-- ══════════════════════════════════════════════════════════════════════════
-- ② LAS DOS POTESTADES, y `SECURITYADMIN` deja de ser una carcasa
-- ══════════════════════════════════════════════════════════════════════════
--
-- ⭐⭐ Y darle `secreto:emitir` a `SECURITYADMIN` es SEGURO precisamente por la
--   asimetría de arriba: emitir vive en el plano de la organización y no
--   concede **nada** sobre ningún valor. Puede crear credenciales y no puede
--   leer las que ya existían. Es la separación que la `014` compró —cortar sin
--   poder nombrar— ganándose por fin el sitio.
--
-- ⚠️ `ejercida = false` en las dos, y es la verdad: no hay ningún verbo que las
--   use todavía. La `014` inventó esa columna justo para esto — para que la
--   pantalla pueda decir «todavía no» en vez de enseñar un permiso que no hace
--   nada.
insert into iam.potestad (nombre, que_hace, ejercida) values
  ('secreto:emitir',  'crear un secreto. NO da derecho sobre los que ya habia', false),
  -- ⚠️ Ver los NOMBRES de todos, incluidos los que no te han concedido. Destapa
  --   qué sistemas hay y cómo se llaman, así que cuelga de esta potestad y no
  --   del estado por defecto — mismo criterio que `invitacion:listar` con los
  --   correos. Los tuyos los ves porque te los concedieron, no por esto.
  ('secreto:listar',  'ver que secretos hay. ⚠️ destapa nombres, no valores',   false)
on conflict (nombre) do nothing;

insert into iam.rol_potestad (rol, potestad) values
  -- ⭐ El que le da sentido al rol: vigila y emite, y no puede leer.
  ('SECURITYADMIN', 'secreto:emitir'),
  ('SECURITYADMIN', 'secreto:listar'),

  ('ACCOUNTADMIN',  'secreto:emitir'),
  ('ACCOUNTADMIN',  'secreto:listar'),

  ('ORGADMIN',      'secreto:emitir'),
  ('ORGADMIN',      'secreto:listar')
  -- ⛔ `USERADMIN` NO. Administra personas, no credenciales. Que las dos cosas
  --   suenen a «administrar» es lo único que las junta.
on conflict do nothing;

-- ⚠️ Y SU NOTA DEJA DE SER VERDAD, así que se cambia en la misma migración.
--
--   Decía: «CARCASA HOY: ninguna de sus potestades se ejerce todavia».
--
-- La primera mitad se acaba de caer —ya tiene un motivo propio— y la segunda
-- sigue en pie: `secreto:emitir` no tiene verbo aún. Una nota que se quedara
-- como estaba diría en la pantalla que este rol no significa nada, justo el día
-- en que empieza a significar algo. Y una nota vieja es peor que ninguna:
-- ninguna se nota, una vieja se cree.
update iam.rol
   set nota = 'Emite credenciales y NO puede leer las que ya habia: emitir es una '
              || 'potestad de la organizacion y leer es una concesion sobre UN secreto. '
              || 'Sus verbos todavia no existen —`ejercida = false`— pero su motivo si.'
 where nombre = 'SECURITYADMIN';

-- ══════════════════════════════════════════════════════════════════════════
-- ③ Y `concesion.recurso` DEJA DE SER TEXTO LIBRE
-- ══════════════════════════════════════════════════════════════════════════
--
-- Para una vista del árbol se aguantaba. Para un secreto no: el nombre del
-- recurso es **el asidero** — lo que se concede, lo que se audita y lo que un
-- manifiesto referencia. Sin una forma cerrada, dos escrituras del mismo secreto
-- no son el mismo recurso, y la concesión no alcanza a lo que creías que
-- alcanzaba. Y eso no da error: da acceso donde no lo hay, o al revés.
--
-- ⇒ La forma es `<clase>/<nombre>`, con la clase de un conjunto CERRADO:
--
--     vista/<paquete>.<Vista>    lo que ya se concedía sobre el árbol
--     secreto/<nombre>           lo nuevo
--
-- ⭐ Es la misma disciplina que `ore-serve` aplica a un segmento antes de
--   convertirlo en un camino: *«`..` no es un caso especial que haya que
--   recordar, es algo que el alfabeto ya no admite»*.
--
-- ⚠️ Y añadir una clase mañana es añadir una alternativa a esta comprobación —
--   una decisión con fecha, como una migración. Un patrón que admitiera
--   cualquier prefijo dejaría de decir nada al tercero.

-- Lo que hay se prefija. Medido antes de escribir esto: **una sola forma de
-- recurso en toda la base**, `ventas.Clientes`, con 7 concesiones. El `where`
-- deja fuera lo que ya llevara clase, para que esto sea idempotente si alguien
-- lo corre dos veces sobre datos a medias.
update iam.concesion
   set recurso = 'vista/' || recurso
 where recurso not like 'vista/%'
   and recurso not like 'secreto/%';

alter table iam.concesion
  add constraint concesion_recurso_forma
    check (
      recurso ~ '^vista/[a-z][a-z0-9_]*\.[A-Za-z][A-Za-z0-9_]*$'
      or recurso ~ '^secreto/[a-z0-9][a-z0-9_-]{0,62}$'
    );

comment on column iam.concesion.recurso is
  'El asidero: `<clase>/<nombre>`, con la clase de un conjunto cerrado.';

-- ══════════════════════════════════════════════════════════════════════════
-- ⚠️ LO QUE ESTA MIGRACIÓN NO HACE, Y NO ES UN OLVIDO
-- ══════════════════════════════════════════════════════════════════════════
--
-- ⛔ NO guarda ningún secreto, ni el nombre de ninguno. `iam` dice quién puede;
--   el material y el catálogo son del custodio, y la `0023` los deja fuera a
--   propósito.
--
-- ⇒ Consecuencia que hay que mirar de frente: **se puede conceder `usar` sobre
--   un secreto que no existe**. Es exactamente lo que ya pasaba con
--   `vista/ventas.Clientes` —`iam` nunca ha comprobado que esa vista exista— y
--   es una propiedad de que los dos planos estén separados, no un agujero de
--   esto. Una concesión sobre algo que no existe no concede nada; quien resuelve
--   comprueba las dos cosas.
--
-- ⛔ Y NO impone que quien emite quede `owner` de lo emitido. Eso lo decidió la
--   `0023` —y evita el secreto huérfano, igual que `fundar` evita la
--   organización sin administrador— pero es trabajo de un VERBO que todavía no
--   existe. Escribirlo aquí como disparador sería poner una regla de negocio en
--   un sitio donde nadie la va a buscar.
