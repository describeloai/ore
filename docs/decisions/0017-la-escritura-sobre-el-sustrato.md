# 0017 · La escritura sobre el sustrato

**Estado:** **propuesto** · **Fecha:** 2026-09-03 · **Decide:** cómo se escribe sobre una copia —
qué la hace atómica, qué impide que dos escritores se pisen, y cuándo deja de bastar reescribirla
entera

> **Propuesto**, como el [0016](0016-el-testigo-y-el-rango.md), y por el mismo motivo: se escribe
> **antes** de construir porque toca el almacén, que ya tiene tres verbos y una prueba de fuego
> encima. Y como aquel, no razona desde cero: mira lo que tienen escrito Iceberg, Delta, Cognite y
> Palantir, que ya lo resolvieron.

---

## El problema

La ontología se sienta sobre la vista, y la vista se apoya en la tabla o en **su copia**
—[`sustrato.md`](../sustrato.md) §3.4—. Para **leer**, eso está construido y medido. Para
**escribir**, no hay forma decidida, y al ir a mirar aparecieron dos cosas que no eran obvias.

### 1 · El direccionamiento por contenido da atomicidad y **no detecta conflictos**

[ADR 0015](0015-el-protocolo-del-almacen.md) lo celebró, y para leer tenía razón:

> *«Cuando el nombre es el contenido, la carrera es inofensiva. Dos escritores que lleguen a la
> vez escriben los mismos bytes.»*

**Eso deja de valer en cuanto los escritores producen cosas distintas.** Dos funciones que
apliquen sobre la misma copia base producen **dos artefactos distintos**, los dos suben con éxito,
y **nada arbitra cuál gana**. No hay puntero mutable, así que no hay nadie a quien preguntar.

Es una **pérdida de actualización silenciosa** — el fallo clásico, y el que la concurrencia
optimista existe para impedir.

### 2 · Y nuestra copia es *copy-on-write* sin haberlo decidido

`fundir()` lee el Parquet anterior, funde y escribe **uno entero**. Eso tiene nombre en la
literatura y tiene un precio conocido: **amplificación de escritura**. Está bien mientras se lea
mucho y se actualice poco; deja de estarlo antes de lo que parece.

---

## Lo que hacen los demás · tres capas, y las tres las usan todos

No es contexto: es de dónde salen las dos decisiones.

### La atomicidad la da el formato

**Delta** usa concurrencia optimista en tres fases —*leer* qué ficheros hay que tocar, *escribir*
los nuevos, *validar y confirmar* comprobando conflictos **en el commit**—, con aislamiento
**write-serializable** para escrituras y **snapshot** para lecturas. Nadie bloquea por adelantado.
Iceberg tiene la misma forma.

**Cognite lo expone en su API**, y es lo más directamente traducible que hay:

> *«All instances in a request are applied atomically... all instances in a request are applied,
> or the entire request fails.»*

Con concurrencia optimista por **`existingVersion`**: cada instancia lleva versión, empieza en 1 y
sube; si la real supera a la esperada, **409**. Y `existingVersion: 0` significa *«asegúrate de
que no existe»*.

> **`existingVersion` es `If-None-Match` por fila.** Nosotros ya usamos exactamente eso, pero por
> artefacto.

### La escritura barata la da merge-on-read

| | cómo invalida una fila |
|---|---|
| **Iceberg** · *position deletes* | fichero + posición ordinal |
| **Iceberg** · *equality deletes* | **por sus valores** — o sea, por clave |
| **Delta** · *deletion vectors* | un Parquet con solo la fila nueva, más un binario que marca la vieja como inválida |

Y su razón, en una frase: *«avoids write amplification by marking rows as invalidated without
rewriting files»*.

**El *writeback dataset* de Foundry es esto con otro nombre**: las ediciones del usuario fundidas
sobre los datos de entrada, reconstruido cuando llega una transacción o cada seis horas.

### Y la propagación se paga en checkpoints

El **Object Data Funnel** de Foundry ofrece dos garantías **que se eligen** —`AT_LEAST_ONCE` y
`EXACTLY_ONCE`— y dice lo que cuesta la segunda:

> *«The most time-consuming part of Funnel streaming pipelines is Flink checkpointing to allow for
> "exactly once" streaming consistency, with a default checkpoint frequency of once every
> second.»*

No es magia: es Flink con checkpoints por segundo, y se paga en latencia. Conviene saberlo antes
de prometer nada.

---

## Decisión A · el recibo del sucesor se escribe **sobre su base**

> **Una copia nueva que parte de otra reserva a su base.** El recibo del sucesor se escribe con
> `If-None-Match` en una clave derivada de la base, así que **solo un sucesor por base gana**.

```text
ore/v1/plan/<plan>/<cabecera>          el recibo de una copia          (ya existe)
ore/v1/plan/<plan>/desde/<base>        quién sucedió a esa base        (nuevo)
```

El segundo escritor recibe un **412**, ve que la base ya tiene sucesor, y **reintenta sobre el
estado nuevo**. Es exactamente el ciclo de Delta —leer, escribir, validar y confirmar— con las
piezas que ya hay, y **no introduce ningún puntero mutable**: la clave sigue siendo contenido y se
escribe una vez.

**Y da lo que faltaba: una cadena lineal.** Con ella, *«¿de qué estado salió esta copia?»* y
*«¿alguien más escribió mientras yo computaba?»* tienen respuesta, y la segunda es la que hoy no
la tiene.

### Por qué por artefacto y no por fila

`existingVersion` de Cognite es por instancia, y es más fino. Aquí no hace falta, y añadirlo
costaría llevar una versión por fila dentro del Parquet — un campo nuestro dentro de una carga que
se eligió **porque cualquiera la lee**.

La granularidad de la copia es la correcta mientras un efecto sea *«una propuesta sobre una
vista»*: la propuesta ya declara su alcance, y su alcance es una vista. Si algún día una copia
recibe escrituras de muchos a la vez, esto se queda corto — **y se dice ahora, en vez de
descubrirlo**.

---

## Decisión B · copy-on-write hoy, y el criterio para cambiar

> **Se queda como está —reescribir la copia entera— y se cambia cuando una medida lo pida, no
> antes.**

COW es lo correcto para lo que esto es hoy: copias que se leen mucho más de lo que se escriben, y
un ciclo que ya funde y ya recoge. Y tiene una virtud que MOR no tiene: **una copia es un
artefacto y se lee sola**, sin resolver ninguna cadena. Eso es la mitad de por qué la carga es
Parquet.

**Pero la puerta de MOR está abierta y conviene decir por dónde entra**, porque nuestra forma la
invita:

> Un artefacto inmutable nombrado por su digest **es el sustrato natural de merge-on-read**. Un
> delta que referencia su base es exactamente el *delete file* de Iceberg al lado de su *data
> file*, y `base` ya viaja en la petición de sellado.

Lo que MOR traería y hay que aceptar entero: leer pasa a ser **fundir una cadena**, la cadena
crece, y hace falta **compactar** — que es por lo que Iceberg y Delta tienen `OPTIMIZE`. Tres
piezas nuevas para ahorrar una reescritura.

**El criterio para cambiar, escrito para poder comprobarlo:** cuando una medida al modo de
[ADR 0014](0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md) —**filas escritas**, no segundos—
enseñe que reescribir la copia entera cuesta más que el trabajo que hace. Con copias de mil filas
y refrescos de diez, no lo cuesta. Con copias de millones y escrituras de una, sí.

---

## Lo que se acepta a cambio

**Un objeto más por copia.** El recibo de sucesión son 71 bytes, y la recogida de basura ya sabe
enumerar por prefijo.

**Un reintento posible.** Quien pierda la carrera vuelve a computar. Es el precio de la
concurrencia optimista y es el que Delta paga: *«conflicts are detected only at commit time»*.

**Y la escritura pesada queda sin cubrir**, dicho: mientras sea COW, una copia grande con
escrituras pequeñas y frecuentes amplifica. La salida está descrita arriba y su criterio también.

---

## Lo que esto **no** decide

**Escribir en el origen.** Es otro documento y probablemente **otro producto**: el sector le puso
nombre —*reverse ETL*, definido como *«the activation layer that reads from an existing warehouse
and pushes data to downstream tools»*— y Foundry lo confirma desde su lado: cuando quiere tocar un
sistema externo **no invierte una vista, llama a un webhook**, en modo *writeback* —antes, y si
falla no se cambia nada— o *side effect* —después—, admitiendo por escrito que *«the external
request may succeed but Ontology changes could fail»*.

> **La escritura sobre el sustrato y la escritura sobre el origen son dos cosas, y todos los
> jugadores las tienen separadas.** Este ADR es la primera.

**Qué acepta el objeto físico.** `Table.writes` es `M1` de [`sustrato.md`](../sustrato.md) y `F0`
de [`functions.md`](../functions.md), y va de la tabla, no del almacén.

**La forma de la propuesta.** Es `F1`, y esto solo le dice dónde aterriza.

---

## Lo que falta decidir · con datos, en la siguiente

1. **¿La cadena se poda?** Una copia con cien sucesores tiene cien recibos de sucesión. ¿Se
   recogen con la copia, o son el historial y se quedan? Foundry conserva el *writeback dataset*
   entero; Delta poda con `VACUUM` y **pierde el viaje en el tiempo** a cambio.
2. **¿Qué pasa si dos escrituras no se solapan?** Dos efectos sobre filas distintas de la misma
   copia conflictúan hoy, y no tendrían por qué. Delta distingue conflictos reales de aparentes
   mirando **qué ficheros** toca cada uno; nosotros podríamos mirar **qué claves**. Es la
   diferencia entre serializar y bloquear de más.
3. **¿Y si el origen cambió mientras se computaba?** La propuesta lleva el testigo bajo el que se
   decidió. Que la copia haya avanzado **no siempre invalida** la escritura, y decidir cuándo sí
   es una pregunta de aislamiento que aquí no se contesta.

---

## Fuentes

- [Apache Iceberg · row-level operations](https://iceberg.apache.org/docs/latest/spark-writes/) ·
  [Dremio · copy-on-write vs merge-on-read](https://www.dremio.com/blog/row-level-changes-on-the-lakehouse-copy-on-write-vs-merge-on-read-in-apache-iceberg/)
- [Delta Lake · concurrency control](https://docs.delta.io/concurrency-control/)
- [Cognite · ingestion features](https://docs.cognite.com/cdf/dm/dm_concepts/dm_ingestion/) ·
  [Instances API](https://api-docs.cognite.com/20230101/tag/Instances)
- [Palantir · Object Data Funnel](https://www.palantir.com/docs/foundry/object-indexing/overview) ·
  [funnel streaming pipelines](https://www.palantir.com/docs/foundry/object-indexing/funnel-streaming-pipelines) ·
  [how edits are applied](https://www.palantir.com/docs/foundry/object-edits/how-edits-applied) ·
  [webhooks](https://www.palantir.com/docs/foundry/action-types/webhooks)
