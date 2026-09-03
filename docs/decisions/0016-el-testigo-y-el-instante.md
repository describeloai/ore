# 0016 · El testigo y el instante

**Estado:** **propuesto** · **Fecha:** 2026-09-03 · **Decide:** dos ampliaciones del protocolo del
driver — preguntarle al origen **hasta dónde está**, y poder pedirle las filas **de ese punto**

> **Propuesto y no aceptado**, que es un estado nuevo en esta carpeta. Las quince anteriores se
> escribieron después de construir lo que decidían; esta se escribe **antes**, porque son dos
> cambios en un protocolo que ya tiene tres implementaciones y la segunda es más grande de lo que
> parece. El sitio para discrepar es este, no el código.

---

## El problema

El ciclo de materialización corre entero —[ADR 0015](0015-el-protocolo-del-almacen.md)— y su paso
③ dice *«le pregunta al origen su testigo»*. **No hay con qué preguntarlo.**
[ADR 0008](0008-el-protocolo-del-driver.md) define dos verbos, `catalogo` y `leer`, y ninguno
contesta *hasta dónde estás ahora*.

Lo que hay hoy en su lugar está escrito en `ore-exec/src/main.rs:305`:

```rust
// La marca de agua la pone quien construye: el motor no lee el reloj, y un
// índice que se fechara a sí mismo dejaría de ser reproducible byte a byte.
let marca = valor(args, "--marca").unwrap_or_default();
```

**La escribe una persona a mano**, y la prueba de fuego la teclea: `--marca
2026-08-31T09:30:00Z`. El comentario tiene razón en lo que dice —el motor no debe leer el reloj—
pero la alternativa nunca fue el reloj: **es preguntarle al origen**.

Y sin eso, tres cosas están rotas de una forma que no se ve hasta que se mira:

1. **La copia no se refresca nunca.** Con el testigo vacío la cabecera del sobre es idéntica cada
   vez, así que el recibo dice *«ya está»* y no se vuelve a leer el origen. `ore materialize`
   sirve para **poblar una vez**; no sirve para mantener.
2. **El `gt` del refresco incremental no tiene quién lo rellene.** La maquinaria está entera
   —`ore-exec/src/main.rs:602` construye el filtro con `ámbito: "marca-de-agua"`, y
   `ore-driver` documenta por qué `gt` es admisible ahí y no en un ámbito— y le falta el número.
3. **`freshness` no puede degradar.** El registro tiene la **marca** desde I3 y el **valor**
   vacío, así que una copia vencida no se distingue de una fresca.

---

## Dos decisiones, no una

Van juntas en un documento porque se explican juntas, y **se aceptan por separado** porque cuestan
cosas muy distintas:

| | qué añade | qué desbloquea | tamaño |
|---|---|---|---|
| **A** | un verbo `testigo` | que la copia **se refresque** | pequeño |
| **B** | un campo `en` en `leer` | que la copia sea **atómica** | grande |

**A** se puede aceptar sola y sirve. **B** sin **A** no tiene sentido, porque el instante que se
pediría es justamente el que **A** devuelve.

---

## Decisión A · el verbo `testigo`

> **Un tercer verbo, con la forma de los dos que hay: devuelve un ordinal y nada más.**

```
ore-read-<tipo> testigo <fuente>
  stdin  ← {"objeto":"public.employees","url":"postgres://…"}
  stdout → {"modo":"log","valor":"0/1A2B3C4"}
```

### Por qué un verbo y no un campo de los otros dos

Porque **las tres cosas caducan a ritmos distintos**, que es exactamente la lección que
[ADR 0006](0006-el-artefacto-de-topologia.md) ya sacó al sacar las dos cabeceras fuera del cuerpo
del `.oretopo`:

| | contesta | cambia cuando |
|---|---|---|
| `catalogo` | qué columnas hay y qué cambios emite | alguien altera la tabla |
| **`testigo`** | **hasta dónde está el origen ahora** | **cada confirmación** |
| `leer` | las filas | cada consulta |

Meterlo en `catalogo` obligaría a describir el objeto entero para preguntar un ordinal. Meterlo en
`leer` es peor: llegaría **con** las filas, o sea después de leerlas, y el paso ④ del ciclo existe
precisamente para decidir **antes de leer una sola**.

Y 0008 ya cerró el argumento de forma cuando pasó de uno a dos: *«con dos verbos, deducirlo del
contenido de stdin sería adivinar»*.

### Qué contesta, por modo

El vocabulario **no se inventa**: es el de `changes.witness` de la tabla, y los cuatro son
**ordinales** — el motor los compara, no los interpreta ni los convierte.

| modo | qué devuelve |
|---|---|
| `snapshot` | el *snapshot-id* de Iceberg, la versión de Delta |
| `log` | LSN de PostgreSQL, SCN de Oracle, *offset* de Kafka |
| `field` | `MAX(<la columna que declara `changes.field`>)` |
| `none` | **se niega, con código distinto de cero** |

La última fila es la que sostiene el resto. Un driver que devolviera «ahora» para `none` estaría
inventando una marca que el origen no respalda, y `OOS2021` —que existe para no materializar sobre
un flujo que no retracta— dejaría de morder.

### Lo que el verbo **no** hace

No refresca, no decide, y no sabe qué es una copia. Devuelve un ordinal. Comparar, decidir si
degradar y construir el `gt` ya está escrito de este lado.

---

## Decisión B · `leer` acepta un instante

> **`Peticion` gana un campo `en`. Con él, el driver devuelve las filas tal y como estaban en ese
> punto; sin él, las de ahora.**

```json
{"en":"snapshot:42","objeto":"public.employees","proyeccion":{…},"url":"…"}
```

### El problema que resuelve, y que no es de comodidad

Entre el paso ③ —preguntar el testigo— y el ⑤ —leer las filas— **el origen se mueve**. Y el error
no es simétrico:

| orden | qué pasa | gravedad |
|---|---|---|
| testigo **antes** de las filas | la copia trae filas más nuevas que su marca; un refresco desde ahí **las re-entrega** | con `upsert` es idempotente; con `append`, **duplica** |
| testigo **después** | la copia puede **faltar** filas anteriores a su marca; un refresco desde ahí **se las salta** | **pérdida silenciosa** |

Por eso el ciclo pregunta el testigo primero: es la respuesta segura, y explica que
`changes.mode` no fuera decoración. Pero sigue siendo una copia que **afirma una verdad que no
tiene**: dice ser cierta hasta `T` y contiene cosas posteriores a `T`.

### Y aquí está lo que este ADR quiere dejar escrito

> **`witness: snapshot` no es otro nombre para fechar: es otra propiedad.**

Un origen con snapshots puede servir *«las filas tal y como estaban en el snapshot 42»*. Entonces
el testigo y las filas son **el mismo instante**, no hay desfase, y la copia es atómica. Un origen
con `field` no puede: `MAX(updated_at)` es una foto de un reloj que sigue corriendo.

Así que el modo del testigo pasa a contestar una pregunta que hasta ahora no contestaba:

| modo | ¿puede la copia ser consistente? |
|---|---|
| `snapshot` | **sí** — se lee en el snapshot |
| `log` | **normalmente sí** — LSN, SCN y *offset* nombran una posición replayable |
| `field` | **no** — se acepta el desfase, y se dice |
| `none` | no hay copia que fechar |

Eso deja de ser una nota de implementación y pasa a ser **una propiedad declarada del objeto que
el planificador puede leer**. Es el mismo movimiento que hizo `reads`: lo que era una suposición
del motor pasa a ser una afirmación del origen.

### Lo que ya existe a medias

`ore-exec::Consulta` tiene un `instante: Option<String>`, con el comentario de que *«el instante es
una entrada más, como los atributos»*. **La `Peticion` del driver no lo lleva**, así que hoy no se
puede pedir. La mitad de arriba está; falta la de abajo.

---

## Lo que se acepta a cambio

**Un verbo más que implementar en cada driver.** 0008 ya lo dijo de los dos primeros —*«cada driver
nuevo implementa los dos o declara cuál no sabe»*— y aquí vale igual: un driver que no sepa
fechar contesta `none` y ya está diciendo algo cierto.

**`en` no lo sabrá hacer todo el mundo, y está bien.** Un directorio de NDJSON no tiene snapshots.
La respuesta correcta es negarse a la petición con `en`, no ignorarlo — **ignorar un `en` sería
devolver otras filas de las que se pidieron**, y eso es el fallo que este árbol no comete.

**Dos ordinales que no se pueden comparar entre orígenes.** Un LSN de PostgreSQL y un snapshot de
Iceberg no se comparan, y no hace falta: cada copia compara con **su propia** marca anterior. Que
el vocabulario diga «ordinal» y no «instante» es exactamente para no invitar a esa comparación.

---

## Lo que esto cierra, y lo que no

**Cierra** el paso ③ del ciclo de 0015, que hoy va vacío, y con él las tres cosas rotas del
principio: la copia se refresca, el `gt` se rellena solo y `freshness` degrada contra algo real.

**No cierra:**

- **cada cuánto se refresca.** El testigo dice hasta dónde está el origen; **cuándo** volver a
  preguntarlo es de `freshness` y de quien programe el refresco. Aquí no se decide;
- **la recogida de basura.** Cada refresco escribe **otro** artefacto, con otro nombre. Que eso
  esté bien —una copia vieja sigue siendo cierta hasta su marca— no quita que el almacén crezca.
  0015 ya lo dejó abierto y esto lo hace urgente antes;
- **la cara `writes`.** Sigue siendo M1 de [`sustrato.md`](../sustrato.md).

---

## Lo que falta decidir · para la siguiente iteración

Escrito como preguntas, no como respuestas, que es el estado en el que está:

1. **¿`testigo` lleva `objeto`, o vale por fuente?** Un LSN es del servidor; un `MAX(updated_at)`
   es de una tabla. Si vale por fuente, un solo `testigo` fecha N copias y se ahorran N-1
   llamadas — pero entonces `field` no cabe. Puede que la respuesta sea que **el modo decide el
   alcance**, y eso habría que escribirlo en `01-table`.
2. **¿Qué pasa si el testigo retrocede?** Un origen restaurado desde un respaldo puede devolver un
   ordinal **menor** que el de la copia anterior. Hoy nada lo mira. `0006` ya se topó con esto en
   otra forma —*«la marca DEBE avanzar»*— y la respuesta que dio allí quizá sirva aquí.
3. **¿`en` es un campo de `Peticion` o un verbo `leer-en`?** El campo es más pequeño; el verbo
   hace que un driver pueda declarar que no sabe, en vez de fallar al recibir un campo que ignora.
   La disciplina de esta casa —*declarar lo que no se sabe*— empuja hacia el verbo, y el
   argumento de 0008 —*no multiplicar verbos sin motivo*— hacia el campo.
4. **¿Quién guarda el testigo entre refrescos?** Hoy la marca del `.oretopo` va en su cabecera y
   el sobre de 0015 lleva la suya. Son dos sitios para lo mismo, y el registro de copias podría
   ser el tercero. Conviene que sea uno.
5. **¿Se materializa contra un testigo que el usuario da?** `--marca` desaparecería, pero
   reproducir una copia vieja exige poder fijarlo. Probablemente el verbo se pregunta **y** el
   valor se puede sobrescribir, y entonces hay que decidir si esa copia se marca como distinta.
