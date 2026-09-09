-- 017 · CÓMO SE LLAMA EL ÁRBOL DE UNA ORGANIZACIÓN
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⛔⛔ HOY LA CORRESPONDENCIA ES TIPOGRÁFICA, Y ESO NO ES UN DATO
--
--   Medido el 2026-09-09: dos organizaciones —`demo` y `prueba`— y UN
--   repositorio, `t-demo/ontologia`. Que ese repositorio sea de `demo` es que
--   un namespace se llama parecido. Nadie puede consultarlo, así que nadie
--   puede equivocarse al leerlo: no hay nada que leer.
--
--   ⇒ Y la consecuencia estaba viva: `ORE_SERVE_URL` en la consola es una
--     constante que apunta a `t-demo`. El dueño de `prueba` habría entrado y
--     visto el árbol de `demo`.
--
-- ── ⭐ ES UN NOMBRE, NO UNA DIRECCIÓN. Y la distinción ya estaba escrita ───
--
--   `50-jwks.yaml` la hizo explícita para el emisor:
--
--     «EMISOR     lo que el IdP tiene que AFIRMAR ser
--      DIRECCION  por dónde se le alcanza HOY
--      No son lo mismo y no tienen por qué serlo.»
--
--   Aquí igual. Esta columna guarda `<propietario>/<repositorio>` — la
--   IDENTIDAD del árbol dentro de una forja. Dónde vive esa forja es
--   configuración del despliegue, y cambia sin que ninguna fila mienta. Meter
--   aquí una URL de clon ataría cada inquilino a una carretera.
--
-- ── ⛔ ¿Y por qué una columna, si `t-<nombre>/ontologia` es derivable? ─────
--
--   P2 dice que lo derivable no se declara, y la objeción es buena. Tres cosas
--   la vencen, y ninguna es de gusto:
--
--     ① `nombre` es de la organización y **puede cambiar**. Un repositorio no.
--        Derivar uno del otro haría que renombrar apuntara en silencio a un
--        repositorio que no existe — y el síntoma aparecería lejos.
--     ② `estado = 'retirada'` es una lápida: conserva sus huellas. Su árbol
--        tiene que seguir nombrado, y su nombre no puede reciclarse aunque
--        alguien funde otra organización que se llame igual.
--     ③ El día que una organización traiga SU repositorio, la derivación no
--        tiene dónde ponerlo.
--
--   ⇒ Se deriva el VALOR POR DEFECTO —eso sí es P2, y lo hace `fundar`— y se
--     guarda el resultado. Misma figura que `ore source add`, que deriva el
--     nombre de la variable del manifiesto y aun así lo escribe en el árbol.
--
-- ── ⚠️ Y LO QUE ESTA COLUMNA NO AFIRMA ────────────────────────────────────
--
--   **No dice que el repositorio exista.** Dice cómo se llama el que le toca.
--   Los dos actos no pueden ser uno: esta fila es una transacción y crear un
--   repositorio es un efecto externo. Se elige cuál es la verdad, y es la fila:
--   quien llegue antes de que exista se encuentra «tu árbol todavía no está
--   aprovisionado», que dice qué falta. Al revés quedarían repositorios
--   huérfanos —creados y sin fila que los nombre—, y eso no lo ve nadie.
--
--   ⇒ La prueba está en el relleno de abajo: a `demo` le pone un nombre que
--     existe y a `prueba` uno que todavía no. Las dos filas son correctas.
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.organizacion
  add column if not exists arbol text;

-- El valor que la convención ya implicaba, para lo que hay. `t-` delante porque
-- es lo que hace el namespace del inquilino y así una migración no reescribe la
-- historia de nadie: se limita a poner por escrito lo que se venía suponiendo.
update iam.organizacion
   set arbol = 't-' || nombre || '/ontologia'
 where arbol is null;

alter table iam.organizacion
  alter column arbol set not null;

-- ⛔ ÚNICO, Y SIN PREDICADO. Dos organizaciones que nombren el mismo árbol son
--   la fuga entre inquilinos con un paso de más, y una errata basta para
--   escribirla. Incluye a las retiradas a propósito (ver ② arriba).
alter table iam.organizacion
  add constraint organizacion_arbol_unico unique (arbol);

-- ⛔ Y un ALFABETO CERRADO, por lo mismo que `ore-serve` valida un segmento
--   antes de convertirlo en un camino: «`..` no es un caso especial que haya
--   que recordar, es algo que el alfabeto ya no admite». Este valor acaba
--   compuesto dentro de una URL de clon.
alter table iam.organizacion
  add constraint organizacion_arbol_forma
    check (arbol ~ '^[a-z0-9][a-z0-9_-]{0,62}/[a-z0-9][a-z0-9._-]{0,62}$');

comment on column iam.organizacion.arbol is
  'Como se llama su arbol —`<propietario>/<repositorio>`—, NO donde vive ni si existe todavia.';
