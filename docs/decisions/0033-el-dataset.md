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

- Con `from` es un dataset **mantenido**: el documento lleva el plan y el sistema lo cumple
  (la copia con plan; fuera: la *materialized view* de Databricks, la *dynamic table* de
  Snowflake, el modelo `table`/`incremental` de dbt). La costura del gobierno y el refresco viven
  aquí. `from` nombra una `Table`, **una `View`** (sus filas guardadas: la pregunta sigue siendo
  la vista, y es la migración directa de `View + materialized` sin fundir el plan) **o otro
  `Dataset`** (una copia derivada de una copia, que es lo que un transform hace). `freshness`
  (cada cuánto se cumple el plan; el *target lag*) es suyo, como hoy de la View. `changes` **no
  lo lleva**: lo deriva de su raíz (`mode` y `key` los de ella, `witness: snapshot`), igual que
  hoy la View no lo lleva y el refresco lo lee de la `Table` raíz (v1alpha8 `OOS2024`).
- Sin `from` es un dataset **escrito**: lo llena código (`write()`, un transform, un trabajo) y
  el sistema registra lo que llegó. Su esquema (`columns`) **sigue a la tabla Iceberg** —nace
  con la primera escritura, cambia cuando ella cambia, y lo que se le añada a mano (descripción,
  tipo declarado) se conserva; es el «schema inferido, editable» de Foundry— y **su linaje no
  está en el documento sino en el puntero** (`procedencia: {inputs, transform, codigo@commit}`,
  0031 W3.7 ③), porque lo escribe quien lo produjo y cambia con cada escritura. Aquí `changes`
  no dice cómo se refresca sino **qué escrituras admite**: `mode: append` niega un upsert,
  `mode: upsert` exige `key` y es lo que `write()` funde por ella; es la costura de la escritura
  desde un puesto (0031 W3.7 gobierno), que la `Table` del lago no tenía.
- El puntero (`datasets/<ns>_<n>.json`: `metadata_location`, `snapshot`, `filas`, `procedencia`,
  `escrito_por`) sigue siendo **el estado** y no entra en el documento: dos sitios que dicen lo
  mismo son dos sitios que discrepan. **Sus versiones son sus snapshots** (Foundry: las
  transacciones; Iceberg: la historia): `historia`, leer a un snapshot, `revert` del puntero.
- **Vive en la rama**: el puntero es de la rama del árbol (Foundry: «dataset branches follow
  code branches», con *fallback* a `master`; aquí 0031 §4, ya hecho). La tabla Iceberg es una
  y la rama apunta a su snapshot, como una *ref* de Iceberg. (Se comprueba en la medida: hoy
  dos ramas que escriben el mismo dataset comparten la tabla del lago.)
- **Es tabular**: una tabla Iceberg, con columnas. Lo que son ficheros con digest es un
  `TrainedModel` (v1alpha11), y si un día hace falta «dataset de ficheros» a secas, se abre
  entonces. Foundry hace de los ficheros la base y del esquema una capa; aquí el esquema es
  la base porque **el gobierno cuelga de las columnas** (0008, la `Entity`).
- **No lleva `labels`**: la clasificación baja de la `Entity` por la pregunta, como hoy. **Ni
  calidad**: las reglas son un `Ruleset` que apunta al dataset (0034 ③, Rules), como los *asset
  checks* de Dagster viven al lado del asset y no dentro. **Sí `owner`** (obligatorio) y
  `description`: quién responde y qué es.
- `history` (**opcional**, `{maxAge, minSnapshots}`; no `retention`, que en `Table.changes`
  ya significa otra cosa): la decisión de cuánta historia se guarda, que hoy vive sólo en las
  propiedades de la tabla (`history.expire.*`, `--retencion`) y que es del documento por la
  misma razón que `freshness`: es una decisión, no un estado. Sin ella, la del inquilino.

> ### ② `View` vuelve a ser sólo la pregunta; `Entity` se respalda en una View **o en un Dataset**.

`View.from` y `Entity.backedBy` admiten `dataset`. `View.materialized` **y `View.freshness` se
retiran** como se retiró `Binding` (la frescura era de la copia, y la copia es el dataset): un
documento que las declare en v1alpha12 es `OOS1005` —una clave que no es de aquí— con el
remedio dicho («esto es un `Dataset` con `from: { view }`»); en versiones anteriores siguen
compilando, acotadas y con fin. Conformance: 13 casos (oos `9255ad5`), marcador
`borrador_de_v1alpha12` en 1 / 13 antes de construir. Escrito: `vendor/oos/spec/v1alpha12/`
(`00-scope`, `01-dataset`, `02-la-vista-y-la-entidad`) y `schemas/v1alpha12/dataset.schema.json`
(21 documentos de prueba, 5 que acepta y 16 que niega, contra el schema).

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
   *standard* crea Views) y a los árboles que existen (`demo`, `victor`). El mapa está medido
   («Lo medido: cuántos sitios tocan `materialized` y `datasource: lago`»); falta la migración:
   un árbol de hoy convertido con `ore migrate` y compilando igual.

## Lo medido: cuántos sitios tocan `materialized` y `datasource: lago` (2026-09-21)

`grep` sobre ORE (sin `target/`), `vendor/oos` y la consola (`lib/`, `components/`, `app/`;
sin `node_modules/`, `.next*`). Se cuenta **ficheros / ocurrencias** y se dice qué es cada
sitio, porque el número a secas engaña: la mitad son fixtures y prosa.

**`materialized` en ORE, código que decide** (lo que la reforma toca de verdad):

| dónde | ficheros / ocurr. | qué hace con la clave |
|---|---|---|
| `ore-core/src/vistas.rs` | 1 / 23 | el compilador: `OOS2004` (`materialized.datasource` sin declarar), `OOS2020` (raíz que no se deja leer sin copia), `OOS2025` (una vista escrita por una Function tiene que ser copia), las dos reglas de stream/tabla (sólo si se copia), «la copia más cercana bajando por la cadena» (`copia_de`), `NEUTRAS` (claves que no cambian el contrato); 8 de las 23 son tests |
| `ore-core/src/flow.rs` | 1 / 4 | el conducto: `materialization.payload` decide sobre el plan de la vista que copia (`OOS4002`/`OOS4011`) |
| `ore-core/src/document.rs` | 1 / 3 | la forma de `View` (v1alpha7 y v1alpha8): `materialized` es clave del `spec` |
| `ore-core/src/normalize.rs`, `diff.rs` | 2 / 3 | `materialized.table` es nombre físico (no se normaliza); el diff «una vista deja de copiarse» |
| `ore-cli/src/inductor.rs` | 1 / 16 | `ore discover` para una base *standard* **emite** `materialized: { datasource, table: copia.<v> }` en cada vista; 10 de las 16 son tests |
| `ore-cli/src/materializar.rs` | 1 / 3 | `ore materialize`: filtra «las View del paquete con `materialized`» (dos sitios) |
| `ore-cli/src/registro.rs` | 1 / 3 | el registro de copias: «las que el paquete declara: una View con `materialized`» |
| `ore-cli/src/preguntar.rs`, `vista.rs`, `invocar.rs` | 3 / 7 | `ask` (el 422 «no declara `materialized`»), `ore view` (dónde sostener una edición, el linaje), `invoke` (una Function lee la copia: `over` tiene que declararla, 0029 ③) |
| `ore-cli/src/main.rs`, `autoria.rs` | 2 / 3 | prosa de ayuda |
| `ore-serve/src/copia.rs` | 1 / 10 | la copia de la consola: pone `materialized` en cada vista de una base *standard*, lista las vistas con `materialized` de un paquete y del árbol, cuenta la clase |
| `ore-serve/src/rutas.rs`, `funciones.rs`, `puestos.rs`, `cola.rs` | 4 / 4 | `/esquema` marca `copiada` por vista; `/funciones` comprueba que `over` se copia; `datos_del_puesto` (el mensaje); un comentario |
| `malla/aprovisionar-inquilino.sh` | 1 / 2 | **decide si se rinde el Job de la copia (45)** con `re.search(r"^\s+materialized:")` sobre el árbol |
| `malla/48-la-copia.yaml`, `.github/workflows/ci.yml`, `README.md` | 3 / 5 | prosa |

Es decir: **13 ficheros de código de ORE** (5 de `ore-core`, 8 de `ore-cli`/`ore-serve`) más
**1 de la malla** deciden algo por `materialized`; de las ~90 ocurrencias en `crates/*/src`,
unas 30 son tests dentro del mismo fichero. Los 3 ficheros que dicen `materializedView`
(`ore-driver/catalogo.rs`, `ore-read-postgres`, `ore-cli/lector.rs`) son la **clase del objeto
en el origen** y no cambian.

**`datasource: lago` en ORE** (la `Table` que `write()` deja):

| dónde | ocurr. | qué hace |
|---|---|---|
| `ore-cli/src/datasets.rs` | ~10 | **el sitio**: `asegurar_lago` (declara el datasource `lago` en `ontology.config.yaml` una vez), `asegurar_table` (escribe la `Table` v1alpha8 con `datasource: lago`), `seguir_esquema` (sus columnas siguen a Iceberg), `puntero_del_lago`; 4 tests |
| `ore-serve/src/puestos.rs` | 3 | `datos_del_puesto` resuelve una `Table` con `datasource == "lago"` por `datasets/`; 1 test |
| `ore-cli/src/materializar.rs` | 1 | `copiar`: la View sobre una tabla del lago va por `ore-store copiar` en vez de por el lector |
| `puesto/{python,node,jvm}` | 5 / 5 / 6 | sólo mensajes («ni Table del lago») |
| `malla/48-la-copia.yaml`, `53-el-mantenimiento.yaml` | 1 / 1 | `LAGO_URL` (el bucket como fuente); comentarios |

El resto de `"lago"` en `crates/` (`ore-view`, `ore-maintain`, `ore-core/cache.rs`,
`ore-read-jsonl`: ~60 ocurrencias) es el **nombre de un datasource de fixture** en tests y no
tiene que ver con el concepto. En la consola `lago` no aparece: la `Table` del lago le llega
por `/esquema` como una tabla más.

**`materialized` en OOS (`vendor/oos`)**: spec 11 / 31 (`v1alpha7/01-view`, `v1alpha8/02-view`
y `01-table`, los `00-scope`, `v1alpha1/*` históricos); schemas 2 / 2 (`view.schema.json` de
v1alpha7 y v1alpha8); conformance **85 ficheros / 97** (casi todo fixtures `views/*.yaml` de
v1alpha5–v1alpha10 que declaran una copia, y que **no cambian**: cada caso está fijado a su
`apiVersion`; los que hablan de la clave por nombre son 8 READMEs de v1alpha7/v1alpha8);
examples 3 / 4 (`acme-retail`).

**`materialized` en la consola**: 14 ficheros / 28 ocurrencias, de tres clases:

| clase | ficheros | qué |
|---|---|---|
| el flujo «Create › View › Materialized» | `lib/server/borrador-de-vista.ts` (4), `components/catalog/CatalogClient.tsx` (2), `SchemaDetail.tsx` (4), `DatabaseTypeSelect.tsx` (1), `code-workspace/ramas.tsx` (1), `app/…/workspaces/page.tsx` (2) | el borrador escribe `materialized: { datasource, table: "copia.<n>" }`; la query `?materialized=1`; el tipo de base se deduce de «`materialized` en todas» |
| la forma de View en el cliente | `lib/server/documentos.ts` (1), `lib/ejecucion/como-sql.ts` (2), `components/ontology/datos.ts` (1), `secciones/Views.tsx` (3), `Explore.tsx` (1) | el tipo `spec.materialized?`, la etiqueta «materializada / virtual», el comentario en el SQL |
| mock | `lib/banco/arbol.ts` (2), `ejecuciones.ts` (1), `components/ontology/acme.ts` (3) | el árbol y las ejecuciones del banco de pruebas; la ontología de ejemplo |

**Lo que el número dice.** La reforma no es «19 ficheros»: son **14 sitios de decisión en ORE**
(13 en `crates/*/src` + la malla), **1 sitio en `datasets.rs`** para la Table del lago, **6
ficheros de flujo** en la consola, y el resto es forma, prosa, fixtures y tests que siguen a
los primeros. Lo más caro no está en la lista: es la **migración** de los árboles (`demo`,
`victor`: cada View con `materialized` → `Dataset`; cada Table `datasource: lago` → `Dataset`)
y que `copia.rs` + `inductor.rs` + `aprovisionar-inquilino.sh` dejen de buscar la clave en la
View para buscar el `kind`. Lo que no se ha medido: cuántos documentos de cada árbol real
cambian (requiere el clúster).

## Lo cotejado: qué es un dataset fuera, y qué es aquí (2026-09-21)

Antes de fijar el kind se miró cómo lo nombran los sistemas de los que viene el cliente, y qué
de eso vale aquí. Lo que sale: **el asset es la cosa, no cómo se produce**; el plan puede ir
dentro; el linaje va por transacción; las ramas siguen al código; el gobierno y la calidad van
al lado.

| fuera | qué es su «dataset» | qué confirma o cambia aquí |
|---|---|---|
| **Foundry** (datasets) | ficheros + transacciones (`SNAPSHOT`, `APPEND`, `UPDATE`, `DELETE`), cada una un registro inmutable; el esquema es metadato «que se puede inferir o editar»; «dataset branches follow code repository branches», con *fallback* a `master`; el linaje se registra por *build*: cada ejecución deja una transacción con el trabajo, las entradas y la salida; *virtual tables* para lo que se queda fuera | **confirma el modelo entero**: un documento por lo que se tiene, sin importar quién lo llenó; transacción = snapshot Iceberg (`changes.mode` append/upsert ≈ APPEND/UPDATE; el `sellar` de la copia ≈ SNAPSHOT); esquema que sigue a los bytes; la rama del puesto con fallback (0031 §4); la `procedencia` por snapshot = el *build record*; la `Table` = la *virtual table*. Lo que no se copia: los ficheros como base (aquí el esquema es la base) |
| **Dagster** (software-defined assets) | «An `AssetKey`, a set of upstream asset keys, and a Python function»: el asset es la tabla o el fichero; la función lo calcula; las dependencias se declaran en el asset (`deps`) o por los parámetros; los *asset checks* van al lado | confirma **inputs declarados** (`transform(inputs, output)`) y que la función no es el asset (0031 W3.7 ②: un transform no es un documento); confirma la calidad como capa (Ruleset), no como campo |
| **dbt** (models + materializations) | «a model is defined by its query while the materialization is a config»: `view`, `table`, `incremental`, `materialized view`; los *contracts* fijan el esquema | es el argumento **del modelo de hoy** (`View` + `materialized` como config), y se responde: dbt es una herramienta de construcción y su objeto es la consulta; aquí el catálogo es el registro de lo que el cliente **tiene**, y lo que tiene es el dataset. La pregunta sigue existiendo (`View`), y `from` en el Dataset es la consulta que lo produce; `incremental` ≈ `changes` con `key`/`witness`. Lo que se toma prestado con nombre: **`columns` como contrato exigible** es una opción futura (hoy sigue a Iceberg) |
| **Snowflake** (dynamic tables) | «materializes the results of a SELECT query and keeps them up to date»; *target lag*; refresco incremental o completo; «when dynamic tables read from each other, they form a pipeline» y la dependencia se infiere de la consulta | confirma el dataset **mantenido**: plan dentro, `freshness` = *target lag*, incremental por el testigo (0015/0017), y `from: { dataset }` para encadenar copias |
| **Databricks / Unity Catalog** | tipos: *managed*, *external*, *foreign* («read-only tables managed by a foreign catalog»), *view*, *materialized view* («datasets … that materialize query results using managed flow logic»), *streaming table*; **MV y ST son ambas tablas gestionadas** del catálogo, distintas por la semántica del flujo (batch / streaming), no por ser objetos de otra clase | confirma **un kind con dos formas**: lo que las distingue es cómo se llenan (plan mantenido / código), no qué son; la `Table` ≈ *foreign table*; `paquete.carpeta.nombre` ≈ `catalog.schema.table` (0034 ④) |
| **Iceberg** (branches y tags) | «named references to snapshots with their own independent lifecycles»; retención por rama (`min-snapshots-to-keep`, `max-snapshot-age`, `max-ref-age`); *write-audit-publish*: escribir en una rama, validar, `fast_forward` a `main` | confirma que **el puntero es una ref**: la rama del árbol apunta a un snapshot de la misma tabla; `retention` en el documento con los mismos dos ejes; y WAP es exactamente escribir en una rama del árbol y fundir (0030/0031) |
| **ODCS** (Open Data Contract Standard) | un contrato por dataset: *fundamentals*, *schema*, *quality*, *SLA*, *team/roles*, *servers*, *tags* | confirma `owner` y `description` en el documento; el resto son capas aquí (Access, Rules) o estado (el puntero). Lo que no se toma: el contrato como documento aparte del dataset |

**Lo que el cotejo cambia en ①**: el nombre de las dos formas (**mantenido** / **escrito**),
`changes` con un sentido por forma (cómo se refresca / qué escrituras admite), `freshness` y
`retention` como decisiones del documento, la rama como ref del puntero, «tabular» dicho, y la
calidad fuera. Lo que no cambia: un kind, sin `labels`, el puntero como estado, la `Table`
intacta, la `View` como sólo la pregunta.

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
