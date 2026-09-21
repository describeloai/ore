# 0033 · El dataset: lo que tiene bytes es un documento

**Estado:** propuesto (decidido el 2026-09-21; nada construido ni medido todavía) ·
**Fecha:** 2026-09-21 · **Decide:** que lo que un inquilino **tiene** —bytes en su lago con
historia— se nombra con **un** documento de su paquete, `kind: Dataset`, y no con dos disfraces
(una `View` con `materialized` para la copia; una `Table` con `datasource: lago` para lo que
`write()` deja); que el Dataset **absorbe el plan** de la vista que lo produce (`from`, `fields`,
`where`) y por eso la costura del gobierno y el motor del refresco **se mueven a él, no se
pierden**; que una `View` vuelve a ser sólo la pregunta y una `Entity` puede respaldarse en un
Dataset; que la `Table` sigue siendo el puntero a lo físico, registrado una vez; y que se llega
en **dos tiempos**: el catálogo enseña el concepto ya (con lo que el backend tiene), y OOS lo
fija cuando la reforma esté medida. Revisa [`0031` §10](0031-el-puesto.md) (que decidió lo
contrario, y aquí se dice por qué cambia) y prepara [`0034`](0034-el-catalogo-de-assets.md).

## El problema

Una base *standard* del catálogo (0031 §11; la consola) copia lo que el cliente eligió de su
fuente. Hoy eso deja, por cada tabla, **dos documentos**: la `Table` (el puntero a lo físico,
con sus dos caras) y una `View` idéntica a ella con `materialized` (la copia). Y lo que un puesto
escribe con `write()` deja **una `Table` con `datasource: lago`**: un documento cuyo nombre dice
«puntero a algo de fuera» apuntando a algo nuestro. Lo medido en «Lo medido fuera del verbo»
§2 (0031) ya lo señaló: esa `Table` nace sin dueño ni etiquetas y no hay nada que negar sobre
ella; se gobierna poniéndole una `View` y una `Entity` encima.

Todo eso **funciona**: la copia es una tabla Iceberg con snapshots, el puntero
(`copias/<ns>_<v>.json`, `datasets/<ns>_<t>.json`) es el estado, `GET /datasets` los lista
juntos y `GET /datasets/{ns}/{n}` da la misma ficha para los dos. El backend ya piensa «dataset =
lo que tiene bytes», y 0031 §10 lo escribió como regla —*«dataset = bytes en el bucket con
historia, con un documento del árbol que los nombra y un puntero»*— eligiendo a propósito **no
hacerlo un kind**: *«la View no se convierte en nada: sigue siendo la declaración; cambia lo que
hay detrás»*.

Lo que no funciona es **lo que el cliente ve y nombra**. Foundry, que es el modelo mental de
quien viene a esto, no extrae de una ingesta una tabla y una vista: extrae **un dataset** —el
asset primario, con esquema, transacciones, linaje y markings— y la ontología (*object types*)
se respalda en datasets; las vistas son derivadas. Aquí, en cuanto el catálogo liste Views, una
base de 30 tablas enseña 30 tablas y 30 vistas que son la misma cosa; y el documento que nombra
lo que `write()` produjo se llama como lo que no es. Es la sensación exacta de «esto está raro»
que salió al revisar el catálogo (2026-09-21), y es un problema de **modelo**, no de pintura.

## Lo que se miró antes de decidir

**La objeción que había que responder: «si copiamos sin la View, se pierde la costura del
gobierno».** Es verdad para una `Table` pelada —sin proyección ni plan, nada que negar— y por eso
0031 §10 dejó la View como documento de la copia. **No es verdad para un Dataset que lleve el
plan.** Las etiquetas viven en la `Entity`, bajan por los campos de la pregunta (`backedBy` →
`fields`) y el conducto `materialization.payload` decide sobre **ese plan** (`OOS4002`). Un
documento con `from`, `fields` y `where` tiene exactamente la misma superficie: la costura se
**mueve**, y 0008 sigue valiendo entero (*la forma más fuerte de aplicar una máscara es no pedir
la columna*: la proyección del Dataset es donde una tabla copiada pierde una columna).

**Lo mismo para el refresco.** El testigo, el cursor, el rango, fundir por clave, «ya está» sin
leer una fila (0015, 0017, 0031 W3.6) cuelgan **del plan**, no de que el documento se llame
`View`. Un Dataset con plan hereda el motor sin tocarlo.

**La `Table` no se absorbe.** v1alpha8 retiró `Binding` para que dos preguntas sobre un mismo
objeto no repitieran su contrato físico; un Dataset que llevara `datasource` + `object` +
`columns` físicas sería `Binding` otra vez. La `Table` se queda como lo que es: el puntero a lo
físico, registrado una vez, con sus dos caras.

**Y el conteo de documentos no empeora**: una tabla copiada son hoy `Table` + `View`; serían
`Table` + `Dataset`. Lo que `write()` deja es hoy una `Table` disfrazada; sería un `Dataset`
honesto. Un `TrainedModel` (v1alpha11) ya es un dataset de ficheros con su documento propio y
no cambia.

## La decisión

> ### ① `kind: Dataset` (OOS v1alpha12 · **tener**): lo que tiene bytes en el lago.

Un documento del paquete, con `namespace`, `owner`, y **una de dos** procedencias:

```yaml
# la copia de una tabla (lo que hoy es View + materialized)
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: customers, namespace: olist }
spec:
  owner: team:olist
  from: { table: olist.customers }        # el plan: de qué sale
  fields: { id: customer_id, city: city } # opcional: sin fields, todas las columnas
  where: { country: BR }                  # opcional
  changes: { mode: append, key: [id] }    # cómo se refresca (lo de View hoy)
```

```yaml
# lo que el código produjo (lo que hoy es Table + datasource: lago)
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: resumen, namespace: ventas }
spec:
  owner: team:ventas
  columns: { pais: { type: String }, n: { type: Integer } }   # el esquema, desde Iceberg
  changes: { mode: upsert, key: [pais], witness: snapshot }
```

- Con `from` es **una copia con plan**: la costura del gobierno y el refresco viven aquí. `from`
  nombra una `Table` **o otro `Dataset`** (una copia derivada de una copia, que es lo que un
  transform hace).
- Sin `from` es **una salida de código**: su esquema sigue a la tabla Iceberg (como hoy la
  `Table` del lago) y **su linaje no está en el documento sino en el puntero**
  (`procedencia: {inputs, transform, codigo@commit}`, 0031 W3.7 ③), porque lo escribe quien lo
  produjo y cambia con cada escritura.
- El puntero (`datasets/<ns>_<n>.json`: `metadata_location`, `snapshot`, `filas`, `procedencia`,
  `escrito_por`) sigue siendo **el estado** y no entra en el documento: dos sitios que dicen lo
  mismo son dos sitios que discrepan.
- **No lleva `labels`**: la clasificación baja de la `Entity` por la pregunta, como hoy.

> ### ② `View` vuelve a ser sólo la pregunta; `Entity` se respalda en una View **o en un Dataset**.

`View.from` y `Entity.backedBy` admiten `dataset`. `View.materialized` **se retira** como se
retiró `Binding`: un documento que lo declare en v1alpha12 es `OOS1003` con el remedio dicho
(«esto es un `Dataset`»); en versiones anteriores sigue compilando, acotado y con fin.

> ### ③ La `Table` no cambia. Una base *standard* es Tables + Datasets; una *foreign*, Tables.

Copiar una tabla = escribir su Dataset (identidad: sin `fields` ni `where`). El catálogo enseña
**el Dataset** con su origen en la ficha; la Table sólo cuando no está copiada (0034).

> ### ④ Dos tiempos, y el primero no espera al segundo.

1. **El catálogo enseña el concepto ya**, con lo que el backend tiene: `GET /datasets`
   (= `copias/` ∪ `datasets/`) es el listado de lo que el cliente tiene; la copia identidad de
   una tabla se llama como su tabla y su ficha dice origen, definición, snapshots y procedencia.
   Es la forma final vista desde fuera; cuando llegue ②, cambia **de dónde lee**, no la forma.
2. **OOS v1alpha12 se mide antes de escribirse**: la reforma toca al compilador (linaje, el
   conducto, `OOS2025`), a `materialize`, `ask`, `datasets.rs` (`asegurar_table`), al catálogo
   REST de Iceberg, al SDK del puesto (`datos_de` resuelve por View/Table), a la consola (la base
   *standard* crea Views) y a los árboles que existen (`demo`, `victor`). Un primer recuento a
   ciegas: `materialized` aparece en 19 ficheros de ORE y 14 de la consola; `"lago"` en 4 de ORE.
   La medida es ese mapa con precisión, más la migración: un árbol de hoy convertido con `ore
   migrate` y compilando igual.

## Lo que se acepta a cambio

- **Es la reforma más grande desde v1alpha8**, y toca una regla que 0031 escribió hace un día en
  sentido contrario. Se acepta porque la razón de 0031 —no perder la costura— resultó no
  depender del nombre del documento sino del plan, y porque el modelo mental del cliente
  (Foundry) es el que este catálogo tiene que devolver.
- **Una migración** de `View + materialized` → `Dataset` y de `Table datasource: lago` →
  `Dataset`, con `ore migrate`, y una versión de OOS que retira una clave.
- **Dos formas bajo un kind** (con `from` / con `columns`). Se acepta porque son la misma
  cosa —bytes con historia— con dos procedencias, y la gramática las distingue por una clave y
  no por dos nombres; si al medir resultara que cada forma pide sus reglas propias, se abre en
  dos y se dice.

## Lo que esto no decide

- Cómo el catálogo lo enseña: [`0034`](0034-el-catalogo-de-assets.md).
- El orden y las fases de la reforma de OOS: sale de la medida.
- El gobierno de una escritura desde un puesto (el conducto que hoy no niega nada): sigue en
  0031 W3.7 gobierno.
