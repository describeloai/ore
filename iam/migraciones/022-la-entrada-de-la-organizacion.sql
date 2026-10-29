-- 022 · CÓMO SE LLAMA LA ENTRADA DE UNA ORGANIZACIÓN
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ LA CUARTA VEZ DE LA MISMA FIGURA, Y YA NO ES UNA COINCIDENCIA
--
--     `50-jwks.yaml`  `EMISOR` es quién firma · `DIRECCION` es dónde se le va a buscar
--     `017`           el ÁRBOL se llama `<propietario>/<repositorio>` · dónde vive, no
--     `019`           la LLAVE se llama `<llavero>/<clave>` · de qué nube es, no
--     `022`           la ENTRADA se llama `demo.ore.paladio.io` · qué IP hay detrás, no
--
--   El nombre es la identidad y va en la fila; la carretera es configuración y
--   cambia sin que nadie mienta. La IP del balanceador, el emisor del
--   certificado y el nombre del `Gateway` **no** están aquí, y por eso migrar
--   de un balanceador a otro no toca ni una fila.
--
-- ── ⛔ Qué es esto, porque «entrada» se confunde con «alta» ────────────────
--
--   NO es el alta. El alta ya tiene nombre y proceso: `fundar` escribe la fila,
--   el aprovisionador crea la llave, las cuentas, el repositorio y el testigo,
--   y la semilla pone el árbol.
--
--   ⇒ Esto es **la puerta**: el nombre por el que se llega a ese inquilino
--     desde fuera. Existe porque la consola tiene que poder contestar *«¿a qué
--     URL le pregunto por el árbol de `acme`?»*, y hoy no puede: lee una
--     constante, y por eso apunta a `127.0.0.1`.
--
-- ── ⭐ Por qué se guarda algo que se puede derivar (P2) ────────────────────
--
--   Por lo mismo que el árbol y la llave: **el valor por defecto se deriva; el
--   valor no**. `<nombre>.ore.paladio.io` sirve para el noventa por ciento, y
--   el diez restante es justo el que paga — un cliente que quiere
--   `ontologia.acme.com` en su propio dominio. Derivarlo CADA VEZ haría que
--   ese caso no existiera.
--
--   ⚠️ Y aquí hay una diferencia con las otras tres que conviene decir: el
--     árbol y la llave nombran cosas que creamos NOSOTROS. Una entrada en el
--     dominio del cliente sólo funciona si **su** DNS apunta a nuestro
--     balanceador, y eso no lo controlamos. La fila sigue siendo la verdad y el
--     mundo converge hacia ella; lo que cambia es quién tiene que mover ficha.
--
-- ── ⚠️ Y por eso `unique`, sin predicado ──────────────────────────────────
--
--   Dos organizaciones con el mismo hostname no es un empate: es que la
--   segunda **se queda con el tráfico de la primera**. Incluye a las retiradas,
--   y aquí el motivo es más fuerte que en el árbol: reciclar la entrada de un
--   cliente que se fue manda sus marcadores, sus enlaces guardados y sus
--   testigos todavía vivos a un inquilino que no es el suyo.
--
-- 📎 `docs/decisions/0022-el-inquilino-es-un-repositorio.md`, E6
-- ══════════════════════════════════════════════════════════════════════════

alter table iam.organizacion
  add column if not exists entrada text;

-- El relleno es el derivado, que es lo que la `b` de la E6 sirve sin coste por
-- cliente: un `Gateway` compartido y una `HTTPRoute` por inquilino.
update iam.organizacion
   set entrada = nombre || '.ore.paladio.io'
 where entrada is null;

alter table iam.organizacion
  alter column entrada set not null;

alter table iam.organizacion
  add constraint organizacion_entrada_unica unique (entrada);

-- ⛔ Un nombre de dominio, y el alfabeto lo fija el mundo, no nosotros: RFC
--   1123, minúsculas, al menos dos etiquetas, cada una de 63 como mucho y sin
--   empezar ni acabar en `-`.
--
-- ⚠️ Sin puerto, sin esquema y sin camino. Un `https://` aquí sería carretera
--   metida en la identidad, y el día que cambiara el esquema habría que
--   reescribir filas. Lo que se guarda es el HOST.
alter table iam.organizacion
  add constraint organizacion_entrada_forma
    check (
      entrada ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)+$'
      and length(entrada) <= 253
    );

comment on column iam.organizacion.entrada is
  'Como se llama su puerta —un host, `demo.ore.paladio.io`—, NO que IP hay detras ni con que esquema. No es el alta.';

-- ══════════════════════════════════════════════════════════════════════════
-- ⚠️ LO QUE ESTA MIGRACIÓN NO AFIRMA
-- ══════════════════════════════════════════════════════════════════════════
--
-- ⛔ No dice que la puerta EXISTA, ni que resuelva, ni que tenga certificado.
--   Igual que la `017` con el árbol y la `019` con la llave: la fila es la
--   verdad y el recurso converge. Quien llegue antes se encuentra un fallo de
--   DNS con el nombre puesto, no un silencio.
--
-- ⛔ Y no dice quién puede entrar por ella. La puerta es dónde se llama; quién
--   pasa lo sigue decidiendo el testigo y `concesion_viva`, que no han cambiado.
--   Una entrada abierta a un inquilino sin concesión da un 403, no un árbol.
--
-- ⚠️ Y una consecuencia que hay que mirar: el día que un cliente traiga su
--   dominio, esta columna nombra un host que no es nuestro. La forma lo admite
--   y el resto de la fila no cambia — pero el certificado pasa a depender de
--   que **ellos** muevan un registro DNS, y eso no es una llamada nuestra a la
--   nube: es un correo. El alta deja de ser un acto y pasa a ser una espera.
