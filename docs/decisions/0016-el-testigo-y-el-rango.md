# 0016 · El testigo y el rango

**Estado:** **propuesto** · **Fecha:** 2026-09-03 · **Decide:** dos ampliaciones del protocolo del
driver — preguntarle al origen **hasta dónde está**, y poder pedirle las filas **de un rango**

> **Propuesto y no aceptado**, que es un estado nuevo en esta carpeta. Las quince anteriores se
> escribieron después de construir lo que decidían; esta se escribe **antes**, porque son dos
> cambios en un protocolo que ya tiene tres implementaciones.
>
> Lo que sí está hecho es **mirar cómo lo resuelve quien ya lo resolvió**. Las cinco preguntas que
> este documento tenía abiertas en su primer borrador están contestadas abajo, y ninguna con una
> opinión: con lo que Debezium, Iceberg, Delta, Snowflake, BigQuery y Airbyte tienen escrito.

---

## El problema

El ciclo de materialización corre entero —[ADR 0015](0015-el-protocolo-del-almacen.md)— y su paso
③ dice *«le pregunta al origen su testigo»*. **No hay con qué preguntarlo.**
[ADR 0008](0008-el-protocolo-del-driver.md) define dos verbos, `catalogo` y `leer`, y ninguno
contesta *hasta dónde estás ahora*.

Lo que hay hoy en su lugar está en `ore-exec/src/main.rs:305`:

```rust
// La marca de agua la pone quien construye: el motor no lee el reloj, y un
// índice que se fechara a sí mismo dejaría de ser reproducible byte a byte.
let marca = valor(args, "--marca").unwrap_or_default();
```

**La escribe una persona a mano.** El comentario tiene razón en lo que dice —el motor no debe leer
el reloj— pero la alternativa nunca fue el reloj: **es preguntarle al origen**.

Sin eso, tres cosas están rotas:

1. **La copia no se refresca nunca.** Con el testigo vacío la cabecera del sobre es idéntica cada
   vez, el recibo dice *«ya está»* y no se vuelve a leer el origen. `ore materialize` **puebla una
   vez**; no mantiene.
2. **El `gt` del refresco incremental no tiene quién lo rellene** — `ore-exec/src/main.rs:602`.
3. **`freshness` no puede degradar**: el registro tiene la marca desde I3 y el valor vacío.

---

## Lo que hacen los demás

No es contexto: es de dónde salen las decisiones de abajo. Seis sistemas, y **coinciden**.

### El testigo se toma ANTES de leer, y se acepta re-entregar

Debezium es el caso puro, porque tiene exactamente este problema y lo documenta paso a paso. Lee
la posición del log **al empezar** la instantánea y, textualmente:

> *«After the connector completes its initial snapshot, the PostgreSQL connector continues
> streaming from the position that it read in Step 2.»*

Es decir: los cambios ocurridos **durante** la copia se re-entregan. Es la fila «testigo antes» de
la tabla de más abajo, elegida a propósito por el sistema de referencia de esta categoría.

**Nuestro ciclo ya lo hace en ese orden.** Lo que faltaba no era el orden: era el verbo.

### El testigo caduca, y todos se niegan en voz alta

| | qué pasa cuando la posición ya no está |
|---|---|
| **Debezium** | *«A previously recorded offset specifies a log position that is not available on the server»* → vuelve a hacer la instantánea |
| **Snowflake** | el *stream* pasa a **STALE**; los cambios no consumidos dejan de ser accesibles y **hay que recrearlo** |
| **Delta Lake** | si la `startingVersion` ya no está en el historial, **el flujo no arranca**; `VACUUM` borra también el *change data* |
| **BigQuery** | error si el instante es anterior a lo que permite el *time travel* (7 días por defecto) |

**Ninguno degrada en silencio a un recorrido completo.** Y dos de ellos **publican el horizonte**
antes de que llegue: Snowflake expone `STALE_AFTER` como una predicción consultable, y BigQuery y
Delta publican su retención.

### Los límites del rango son argumentos de la lectura, no otra llamada

- **Iceberg**: `start-snapshot-id` y `end-snapshot-id` son **opciones de la lectura**, y omitir el
  final **usa la instantánea actual**.
- **Delta**: `startingVersion` / `startingTimestamp` son opciones, y sin ellas el flujo devuelve la
  instantánea actual como altas y luego los cambios.
- **BigQuery**: `CHANGES(TABLE t, start_timestamp, end_timestamp)` los toma como **argumentos de
  la función**, con un tope documentado de **un día** por ventana.

Nadie tiene un verbo aparte para «leer en un punto». **Es un parámetro de leer.**

### Y el modo `field` es, por escrito, at-least-once

Airbyte llama a esto *cursor field* y es exactamente nuestro `witness: field`. Su documentación,
literal:

> *«When replicating data incrementally, Airbyte provides an at-least-once delivery guarantee.»*

> *«you may see the same row being emitted during each sync [...] as the cursor field will always
> be greater than or equal to itself.»*

Y admite que se pueden **perder** filas:

> *«if modifications to the underlying records are made without properly updating the cursor
> field, then the updated records won't be picked up by the Incremental sync as expected.»*

Eso no es un defecto de Airbyte. **Es una propiedad del modo**, y la tiene cualquiera que fecha
por una columna.

---

## Decisión A · el verbo `testigo`

> **Un tercer verbo, con la forma de los dos que hay: devuelve un ordinal y nada más.**

```
ore-read-<tipo> testigo <fuente>
  stdin  ← {"objeto":"public.employees","url":"postgres://…"}
  stdout → {"modo":"log","valor":"0/1A2B3C4"}
```

Un verbo y no un campo de los otros dos porque **las tres cosas caducan a ritmos distintos** —la
misma lección que [ADR 0006](0006-el-artefacto-de-topologia.md) sacó al poner las dos cabeceras
fuera del cuerpo del `.oretopo`:

| | contesta | cambia cuando |
|---|---|---|
| `catalogo` | qué columnas hay y qué cambios emite | alguien altera la tabla |
| **`testigo`** | **hasta dónde está el origen ahora** | **cada confirmación** |
| `leer` | las filas | cada consulta |

Meterlo en `catalogo` obligaría a describir el objeto entero para pedir un ordinal. Meterlo en
`leer` es peor: llegaría **con** las filas, y el paso ④ del ciclo existe para decidir **antes de
leer una sola**.

Un modo `none` **se niega, con código distinto de cero**. Devolver «ahora» inventaría una marca
que el origen no respalda, y `OOS2021` dejaría de morder.

---

## Decisión B · `leer` acepta un rango

> **`Peticion` gana `desde` y `hasta`. Sin ninguno, las filas de ahora; con ellos, las de ese
> rango — y el driver que no sepa honrarlos se niega.**

```json
{"desde":"0/1A2B3C4","hasta":"0/1B0000","objeto":"public.employees","proyeccion":{…},"url":"…"}
```

**Un rango y no un instante**, que es la corrección que la evidencia impuso al primer borrador de
este documento. Todos implementan la lectura incremental, no la puntual: la puntual es el caso
degenerado con `desde` ausente. Diseñarlo como «leer en un punto» habría dejado fuera el caso que
de verdad se usa.

**Campo y no verbo**, porque es lo que hacen los cuatro. Y la disciplina de esta casa —*declarar
lo que no se sabe*— se respeta igual con un campo, siempre que **ignorarlo esté prohibido**:

> Un driver que reciba `desde` y no sepa honrarlo **debe fallar**. Ignorarlo sería devolver otras
> filas de las que se pidieron, y eso no falla: se sirve.

### Lo que esto convierte en una propiedad declarada

Entre preguntar el testigo y leer las filas el origen se mueve, y el error no es simétrico:

| orden | qué pasa | gravedad |
|---|---|---|
| testigo **antes** | la copia trae filas más nuevas que su marca; un refresco desde ahí **las re-entrega** | con `upsert` es idempotente; con `append`, **duplica** |
| testigo **después** | la copia puede **faltar** filas anteriores a su marca; un refresco **se las salta** | **pérdida silenciosa** |

Con `hasta`, el desfase desaparece del todo para quien pueda honrarlo. Y eso hace que el modo del
testigo conteste una pregunta que hasta ahora no contestaba:

| modo | ¿puede la copia ser atómica? | garantía |
|---|---|---|
| `snapshot` | **sí** — se lee en la instantánea | exacta |
| `log` | **sí** — la posición es replayable | exacta, con solape re-entregado |
| `field` | **no** — `MAX(col)` es la foto de un reloj que corre | **at-least-once**, documentado |
| `none` | no hay copia que fechar | — |

> **`witness: snapshot` no es otro nombre para fechar: es otra propiedad.** Dice si la copia
> **puede ser consistente**, y eso deja de ser una nota de implementación para ser una afirmación
> del origen que el planificador lee — el mismo movimiento que ya hizo `reads`.

---

## La regla que sale de las dos, y que el compilador puede comprobar

Es lo más valioso que salió de mirar a los demás, y **no hace falta gramática nueva**: la pareja
`(mode, witness)` ya está declarada y **determina la garantía de entrega**.

| `witness` | `mode` | veredicto |
|---|---|---|
| `field` | `append` | **se rechaza** — at-least-once sin clave con la que deduplicar: cada refresco duplica el solape, para siempre |
| `field` | `upsert` / `retract` | **se acepta** — la re-entrega es idempotente por clave |
| `log` / `snapshot` | cualquiera | se acepta |

La primera fila es la que vale la pena. Hoy compila, y **es el modo en el que Airbyte documenta
que se duplican y se pierden filas**. Con la regla, el compilador lo rechaza antes de que alguien
lo descubra en la factura.

---

## Las cinco preguntas del borrador, contestadas

**1 · ¿`testigo` lleva `objeto`, o vale por fuente?** → **El modo decide el alcance**, y es lo que
hacen los demás: un *snapshot-id* de Iceberg y una versión de Delta son **de una tabla**; un LSN
es **del servidor**. Así que `objeto` viaja siempre y el driver decide si lo usa. Y hay que
escribirlo en `01-table`, porque un `log` compartido permite fechar N copias con **una** llamada.

**2 · ¿Qué pasa si el testigo retrocede o caduca?** → **Se refusa en voz alta**, que es lo que
hacen los cuatro. Y el horizonte **ya se puede declarar**: `changes.retention` existe en la
gramática desde v1alpha8 —*«cuánto guarda el origen su changelog, si se sabe. Informativo: quien
planifique un refresco lo usa para saber si puede llegar tarde»*— y **nadie lo lee**. Ahí está la
mitad de la respuesta ya escrita, esperando un consumidor. La otra mitad es un código de error
para «tu testigo cayó fuera de la retención: hay que recopiar entera», que es literalmente el
`STALE` de Snowflake y el re-snapshot de Debezium.

**3 · ¿`en` es un campo o un verbo?** → **Campo**, y son `desde`/`hasta`. Unánime en los cuatro, y
con la condición de que ignorarlo esté prohibido.

**4 · ¿Quién guarda el testigo entre refrescos?** → **El artefacto**. Todos lo guardan fuera del
origen —Debezium en un tópico de *offsets*, Snowflake en el objeto *stream*, Iceberg y Delta en el
consumidor— y nosotros ya tenemos el sitio: **la cabecera del sobre**, que es exactamente *hasta
cuándo fue cierta esta copia*. La consecuencia es que la cabecera del `.oretopo` y el sobre de
0015 **son el mismo campo en dos formatos**, y deben converger.

**5 · ¿Se materializa contra un testigo dado por el usuario?** → **Sí, y no es un concepto
aparte**: es `desde`/`hasta` con valores puestos a mano. Reproducir una copia vieja es *time
travel*, y Iceberg y Delta lo tratan como la misma operación con otros argumentos. `--marca`
desaparece como mecanismo y sobrevive como parámetro.

---

## Lo que se acepta a cambio

**Un verbo más y dos campos más en cada driver.** 0008 ya lo dijo de los dos primeros: *«cada
driver nuevo implementa los dos o declara cuál no sabe»*.

**`desde`/`hasta` no lo sabrá hacer todo el mundo.** Un directorio de NDJSON no tiene versiones.
La respuesta correcta es **negarse**, no ignorarlos.

**Puede haber un tope de ventana.** BigQuery limita `CHANGES` a **un día** por consulta. Así que
un refresco puede necesitar **varias** lecturas encadenadas, y eso es del planificador — pero el
protocolo tiene que permitir que el driver diga *«ese rango no lo puedo servir de una vez»*.

**Dos ordinales de orígenes distintos no se comparan**, y no hace falta: cada copia compara con
**su propia** marca anterior. Que el vocabulario diga «ordinal» y no «instante» es para no invitar
a esa comparación.

---

## Lo que esto cierra, y lo que no

**Cierra** el paso ③ del ciclo de 0015 y, con él, las tres cosas rotas del principio.

**No cierra:**

- **cada cuánto se refresca.** El testigo dice hasta dónde está el origen; **cuándo** volver a
  preguntar es de `freshness`;
- **la recogida de basura**, y **A la vuelve urgente**: cada refresco escribe otro artefacto con
  otro nombre. 0015 ya la dejó abierta;
- **el encadenado de ventanas** cuando el origen limita el rango, que es del planificador;
- **la cara `writes`**, que sigue siendo M1 de [`sustrato.md`](../sustrato.md).

---

## Fuentes

- [Debezium · connector for PostgreSQL](https://debezium.io/documentation/reference/stable/connectors/postgresql.html)
  ([fuente del documento](https://raw.githubusercontent.com/debezium/debezium/main/documentation/modules/ROOT/pages/connectors/postgresql.adoc))
- [Airbyte · Incremental Sync — Append](https://docs.airbyte.com/platform/using-airbyte/core-concepts/sync-modes/incremental-append)
- [Apache Iceberg · Spark Queries](https://iceberg.apache.org/docs/latest/spark-queries/)
- [Delta Lake · Change data feed](https://docs.delta.io/delta-change-data-feed/)
- [Snowflake · Introduction to streams](https://docs.snowflake.com/en/user-guide/streams-intro)
- [BigQuery · Work with change history](https://docs.cloud.google.com/bigquery/docs/change-history)
