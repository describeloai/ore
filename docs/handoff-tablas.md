# Handoff · la tabla, y las dos caras de una fuente

> **Este documento es desechable.** Se borra el día que `spec/v1alpha8` sea el que manda sobre
> lo físico y ningún documento del árbol —fuera de las suites de conformidad de versiones
> anteriores— sea un `Binding` ni una `View` con `from.datasource`. Un plan que sobrevive a su
> ejecución deja de ser un plan.
>
> Fecha: 2026-09-02 · Escrito después de mirar cómo lo resolvieron Databricks (foreign tables,
> streaming tables), Foundry (virtual tables), Fabric (shortcuts y mirroring), BigQuery, Snowflake
> (external tables y `STREAM`), Trino y Flink/Confluent (dynamic tables, Tableflow), y
> **decidido**: el puntero físico se registra una vez, con dos caras, y la vista compone encima.
>
> Sustituye a [`handoff-vistas.md`](handoff-vistas.md) en sus peldaños V4 y V5, que quedan
> absorbidos aquí. Lo que aquel documento dice de la **naturaleza** de una vista sigue valiendo
> entero; lo que cambia es dónde vive lo físico.

---

## 1. Qué es una tabla

> **Una tabla es el puntero a un objeto físico, registrado una vez, con sus dos caras declaradas:
> qué se le puede pedir (`reads`) y qué cambios produce (`changes`). No contiene datos, no lleva
> significado y no decide quién ve.**

### 1.1 · Es lo que todos tienen, con el nombre que cada uno le puso

*Foreign table* en Databricks, *virtual table* en Foundry, *shortcut* en Fabric, *external table*
en BigQuery y Snowflake, *connector table* en Trino. La descripción es la misma en todos:
**una referencia a algo de fuera que se registra en el catálogo, es de solo lectura, y sobre la
que las vistas componen**. Databricks lo dice con precisión: *«for each foreign table referenced,
Databricks schedules a subquery in the remote system… and returns the result… over a single
stream»*. El puntero no ejecuta: traduce.

Hoy ese puntero vive **dentro** de la `View` de v1alpha7 —`from: {datasource, object}`,
`fields`, `capabilities`, `version`— porque el `Binding` lo tenía así. Es defendible como puente
y es la forma equivocada para quedarse: cada vista que toca una fuente repite el contrato físico,
y una vista sobre otra vista no tiene ninguno. El contrato es **del objeto**, no de quien lo
consulta.

### 1.2 · Tiene dos caras, y no son dos cosas

Nadie tiene *«foreign stream»*. Lo que todos tienen es el puntero de lectura y, aparte, un
**changelog** que nunca se lee por clave y que solo se convierte en tabla materializándolo — la
*streaming table* de Databricks, Tableflow de Confluent, el *mirroring* de Fabric. Y hay una
frase que une las dos, escrita por Flink:

> *A stream is the changelog of a table.* Y al revés: una tabla es un changelog integrado.

El `STREAM` de Snowflake lo tiene aún más claro: no es una fuente, es **el registro de cambios
de una tabla**, un objeto derivado. Eso es literalmente `D(tabla)`.

Así que una fuente no es *o* tabla *o* stream: es un objeto con **dos caras**, y cada una se
declara por separado.

| cara | pregunta | quién la usa |
|---|---|---|
| **`reads`** — la cara `I` | ¿qué se le puede pedir, y con qué filtros? | el Pushdown Planner, la *upquery*, la fase ③ del ejecutor |
| **`changes`** — la cara `D` | ¿qué cambios emite, con qué codificación y con qué testigo? | el mantenedor, el Refresh Analyzer, el Cost Model |

Un topic de Kafka es una tabla cuya cara de lectura es `none`. Una API con `updated_since` es
una tabla cuya cara de cambio es `append`. Un Postgres con ranura de replicación las tiene las
dos. **Un «stream» es el nombre corriente de una tabla sin cara de lectura**, y no hace falta
un `kind` para él: dos nombres para un concepto es el error que este proyecto persigue.

### 1.3 · Las tres codificaciones del cambio son nuestro Z-set

Flink documenta exactamente tres formas de convertir una tabla dinámica en stream, y son las
tres que el Delta Compiler ya entiende:

| Flink | qué manda | en nuestro Z-set |
|---|---|---|
| **append-only** | solo altas | solo `+1` |
| **retract** | `DELETE` = retractar; `UPDATE` = retractar la vieja + añadir la nueva | `-1` y `+1` con peso |
| **upsert** | exige clave única; `DELETE` = mensaje de borrado (tombstone) | `+1` por clave, `-1` por tombstone |

Delta *Change Data Feed* —`insert · update_preimage · update_postimage · delete`— es *retract*
con cuatro nombres. Los tombstones de un topic compactado son *upsert*: Tableflow *«removes a row
when it encounters a tombstone with a matching key»*. **El motor ya tenía el concepto en el
álgebra; lo que no tenía era la gramática.**

### 1.4 · No contiene datos, y por eso `discover` la puede crear sola

Una tabla es un **hecho** del origen: el objeto existe, tiene estas columnas, admite estos
filtros, emite estos cambios. Ninguna de esas cuatro cosas es una conjetura. Por eso el
descubrimiento puede emitirla **mecánicamente y sin inventar** —es lo que hace Databricks al
crear un catálogo foráneo: espejar el esquema— y por eso la regla del inductor se cumple
literalmente: *se emite lo que es un hecho; se reporta lo que es una conjetura*. Lo que sigue
siendo conjetura —qué es una entidad, qué significa una columna— sigue reportándose.

## 2. Qué **no** es una tabla

- **No lleva significado.** Ni `labels`, ni `is`, ni conceptos: lo mismo que la vista, y por lo
  mismo. Su incumplimiento es `OOS1005`.
- **No decide quién ve qué.** El conducto y las políticas Cedar siguen decidiendo.
- **No tiene lógica.** Ni `fields` con renombre, ni `where`. Renombrar y recortar son de la vista;
  la tabla es el objeto tal cual está. Es la diferencia entre el *shortcut* de Fabric —la
  referencia— y la *shortcut transformation* — otra cosa.
- **No es una vista.** Una tabla no se materializa, no tiene frescura y no tiene dueño de negocio:
  tiene dueño técnico por el `datasource`. Materializar es una decisión sobre **una consulta**, y
  la consulta es la vista.

---

## 3. La forma

Normativo cuando esté en `spec/v1alpha8`; esto es el boceto exacto del que sale.

### 3.1 · `kind: Table`

```yaml
apiVersion: oos.dev/v1alpha8
kind: Table
metadata:
  name: employees
  namespace: erp
spec:
  datasource: erp                    # declarado en el manifiesto (OOS2004)
  object: "public.employees"         # opaco: las reglas son del origen

  # Las columnas físicas, tal cual. Nombre opaco; tipo físico en vocabulario de ODCS.
  columns:
    employee_id: { physicalType: varchar(16) }
    national_id: { physicalType: varchar(16) }
    country:     { physicalType: char(2) }
    deleted:     { physicalType: boolean }

  # La cara I. Es `capabilities` de v1alpha7, mudado sin cambios — o `none`.
  reads:
    predicatePushdown: [eq, neq, in, range, isNull]
    fullScan: expensive
    requiredFilters: []

  # La cara D. Qué cambios emite, cómo los codifica y qué los atestigua.
  changes:
    mode: retract                    # none | append | retract | upsert
    witness: log                     # none | snapshot | log | field
    # key: [employee_id]             # obligatorio con `upsert`
    # field: updated_at              # obligatorio con `witness: field`
    # retention: 7d                  # cuánto guarda el origen el changelog, si se sabe
```

Un topic, una API y un lago, en la misma forma:

```yaml
# Kafka: se escribe, no se pregunta. Sin cara de lectura.
spec:
  datasource: bus
  object: "orders.v2"
  columns: { order_id: {}, customer_id: {}, total: {} }
  reads: none
  changes: { mode: upsert, key: [order_id], witness: log, retention: 7d }

# Una API con `modified_since`: se pregunta por clave, y solo sabe de altas.
spec:
  datasource: workday
  object: "Worker"
  columns: { "Worker_Reference.ID": {}, "Compensation_Data.Base_Pay.Amount": {} }
  reads: { predicatePushdown: [eq], fullScan: forbidden, requiredFilters: ["Worker_Reference.ID"] }
  changes: { mode: append, witness: field, field: "Last_Modified" }

# Iceberg en el lago: se recorre entero sin drama, y cada snapshot dice qué cambió.
spec:
  datasource: lago
  object: "ventas.pedidos"
  columns: { id: {}, pais: {}, total: { physicalType: "decimal(18,2)" } }
  reads: { predicatePushdown: [eq, in, range], fullScan: cheap }
  changes: { mode: retract, witness: snapshot }
```

### 3.2 · `kind: View`, adelgazada

Pierde `capabilities` y `version` —son de la tabla— y `from` nombra una tabla o una vista.
Todo lo demás sigue: `owner`, `fields`, `where`, `materialized`, `freshness`.

```yaml
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: empleados, namespace: hr }
spec:
  owner: team:rrhh
  from: { table: erp.employees }     # o { view: … }
  fields:
    employeeId: employee_id          # cada valor DEBE ser una columna de la tabla (OOS2018)
    nationalId: national_id
    pais: country
  where: { deleted: "false" }
  freshness: 15m
```

```yaml
# Una vista sobre un stream. Sin `materialized` no compila (OOS2020).
kind: View
metadata: { name: pedidos, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: bus.orders }
  fields: { id: order_id, cliente: customer_id, total: total }
  materialized: { datasource: lago, table: "cache.pedidos" }
  freshness: 1m
```

### 3.3 · `kind: Entity`, sin cambios

`backedBy` sigue nombrando **una vista**, nunca una tabla. Si nombrara una tabla, las propiedades
de la entidad tendrían que llamarse como las columnas físicas, y lo semántico volvería a saber
de lo físico. La vista es la capa de renombre, y para eso existe. Una tabla que se quiere exponer
tal cual cuesta una vista de tres líneas, y esas tres líneas son las que dicen *«esto se expone»*.

### 3.4 · Las reglas, con su código

| | regla | código |
|---|---|---|
| **tabla** | `datasource` declarado en el manifiesto | `OOS2004` — reutilizado |
| **tabla** | `changes.mode: upsert` exige `key`; `witness: field` exige `field`; los dos nombran columnas de `columns` | `OOS2018` — reutilizado |
| **vista** | `from.table` y `from.view` resuelven; cada valor de `fields` y cada clave de `where` es una columna de la tabla o un campo de la vista de abajo | `OOS2018` — reutilizado; **y por primera vez comprobable contra columnas reales** |
| **vista** | la cadena no vuelve sobre sí misma | `OOS2019` — reutilizado |
| **vista** | la vista que respalda una entidad expone su clave y sus `via` | `OOS2011` — reutilizado |
| **vista** | **lo que no se puede leer se debe materializar**: una vista cuya raíz de lectura es una tabla con `reads: none` DEBE llevar `materialized` | **`OOS2020`** — nuevo |
| **vista** | **un cambio sin retractación solo mantiene lo que solo se anexa**: una vista `materialized` cuya raíz tiene `changes.mode: append` no puede respaldar una entidad `nature: entity` | **`OOS2021`** — nuevo |
| **vista** | la copia lleva lo que llevan sus columnas, por `materialization.payload` | `OOS4001` `OOS4002` `OOS4011` — reutilizados |

**Raíz de lectura** de una vista: la vista `materialized` más cercana bajando por la cadena, o
la tabla si no hay ninguna. Una vista virtual sobre una vista materializada sobre un stream **sí**
compila: lee de la copia.

Los dos códigos nuevos son las dos reglas que en el análisis previo eran prosa y aquí pasan a ser
compilación. `OOS2021` es la limitación que Foundry documenta —*«incremental support… is
currently limited to append-only changes»*— convertida en algo que el compilador rechaza en vez
de algo que la vista deriva en silencio: es **el peor modo de fallo** de todo el motor, el que no
produce ningún síntoma.

### 3.5 · La regla de la versión

Cada versión gobierna un verbo y aporta una regla. v1alpha8 gobierna **de qué está hecha una
fuente**, y su regla es la dualidad:

> `Table = I(changes)` · `View = Q(Table)` · `materialized = I(Q^Δ)`

Con dos corolarios que son `OOS2020` y `OOS2021`: sin `I` no hay lectura, y sin `-1` no hay
`I` de una cosa que cambia.

---

## 4. Proyección sobre el sustrato: qué se toca y qué no

| pieza | hoy | con v1alpha8 |
|---|---|---|
| **`ore-view`** — las doce piezas | `Nodo::Lee(Lectura{datasource, objeto, campos})` | **nada**. `Lectura` **es** la tabla; el mantenedor ya recibe deltas por hoja. La pieza se construyó libre y encaja sin moverla |
| **`ore-maintain`** | sesión por stdin, deltas por hoja | **nada** en el protocolo. `changes.mode` decide qué pesos son legales en un delta: un `append` que trajera un `-1` se rechaza |
| **`ore-core/document.rs`** | `Kind::View` con 8 claves | `Kind::Table` nuevo (`since: V1Alpha8`); `View` pierde `capabilities` y `version`; `Binding` gana `hasta: V1Alpha8` — **retirado, no borrado** (§5.3) |
| **`ore-core/vistas.rs`** | `fuente()` devuelve `Datasource{…}` o `Vista` | `Fuente::Tabla(qname)` sustituye a `Datasource`; `raiz()` llega a la `Table` y de ella saca datasource, objeto y **columnas reales**; `comprobar()` gana las comprobaciones de tabla, `OOS2020` y `OOS2021` |
| **`ore-core/flow.rs`** | hereda del datasource de la raíz | igual, leyendo el datasource de la tabla |
| **`ore-core/governance.rs`** | `datasources_de` | igual |
| **`ore-core/link.rs`** | `bindings()` | se queda para documentos v1alpha1–7; no se toca |
| **`ore-cli/vista.rs`** — la costura | `cuerpo()` construye `Lee` desde `from.datasource`; `capacidades_por_fuente` lee `capabilities` de la vista | `Lee` desde la tabla, con `campos` de `columns` y tipos de la entidad como hoy; capacidades de `reads` |
| **`ore-cli/inductor.rs`** | emite `Entity` + `Binding` | emite **`Table`** por objeto + `Entity` con `backedBy` + la vista trivial; ningún `Binding` — §6 |
| **`ore-read-<tipo>`** | `catalogo` devuelve tablas, columnas, claves, foráneas | el catálogo devuelve además `reads` —lo que **ese** driver sabe empujar— y `changes` —lo que **sondeó** del origen: `wal_level`, `REPLICA IDENTITY`, formato de tabla | 
| **`ore-exec/plan.rs`** | `Motor::fisicas` desde binding o raíz de vista | igual; la raíz es ahora una `Table` |
| **`ore validate`** | | `OOS2020`, `OOS2021` |
| **`ore view`** | | imprime las dos caras de cada raíz y qué regla las usó |

Lo que hace que esto entre **de una pieza** es la columna izquierda de `ore-view` y
`ore-maintain`: **nada**. El motor nunca supo qué era un paquete, y por eso una reforma de la
gramática no lo toca. Es la misma razón por la que la absorción V0–V3 no tocó las doce piezas.

---

## 5. La migración, y por qué no roza

### 5.1 · Es mecánica, y está escrita en una tabla

| de | a |
|---|---|
| `View` v1alpha7 con `from: {datasource, object}` | **`Table`** con ese `datasource`/`object`, `columns` = los valores de `fields` más las claves de `where`, `reads` = `capabilities`, `changes.witness` = `version.witness`, `changes.mode` = §5.2 — **más** la misma `View` con `from: {table}` y sin `capabilities` ni `version` |
| `View` v1alpha7 con `from: {view}` | la misma, con `apiVersion` nuevo |
| `Binding` | **`Table`** con `datasourceRef`→`datasource`, `source`→`object`, `columns` = los valores de `properties` más las claves de `selector`, `reads` = `capabilities`, `changes` = §5.2 — **más** una **`View`** trivial con `fields` = `properties`, `where` = `selector`, `materialized`/`freshness` de `materialization.payload` si lo había — **más** `backedBy` en la entidad |
| `materialization.topology` | **no se migra**: es derivable —el índice es una vista de aristas— y se computa (P2) |
| `Entity` | sin cambios salvo `apiVersion` y `backedBy` |

Dos bindings sobre la misma tabla con selectores disjuntos (`OOS2014`) pasan a ser **una tabla y
dos vistas**, que es la forma correcta que aquel código tenía a medias: el objeto era uno.

### 5.2 · `changes.mode` para lo que no lo declaraba

Un documento anterior a v1alpha8 no sabe de codificaciones. La migración lo deduce de lo que sí
declaraba, **y lo deja escrito como deducción**:

| tenía | `mode` | por qué |
|---|---|---|
| `strategy: table_version` o `witness: snapshot` | `retract` | dos snapshots se restan, y la resta tiene signo |
| `strategy: cdc` o `witness: log` | `retract` | un changelog de base de datos trae la imagen previa; **si no la trae**, quien migra baja a `append` y `OOS2021` se lo cobrará donde toque |
| `strategy: poll` o `witness: field` | `append` | una marca de agua no ve borrados |
| nada | `none` | no se sabe, y no se inventa |

### 5.3 · El `Binding` se retira, no se borra

`handoff-vistas` decía *«el Binding se borra»*. Es más preciso decir **se retira de la gramática
a partir de v1alpha8**: `Kind::Binding` gana `hasta: V1Alpha8`, y un documento v1alpha8 que sea
un `Binding` falla con `OOS1003` diciendo *«se retiró en v1alpha8: es una `Table` y una `View`»*.

Un documento v1alpha1 que sea un `Binding` **sigue compilando**, porque declara su versión y
v1alpha1 es normativo. La suite de conformidad de v1alpha1 no se toca. El código que lee bindings
en `link`, `flow`, `selector` y `plan` **se queda**, congelado, mientras v1alpha1 mande.

Esto es lo que hace que la migración no roce: **el `apiVersion` es por documento**. Un paquete
puede tener entidades v1alpha8 respaldadas por vistas sobre tablas, y al lado un binding v1alpha1
que nadie ha migrado todavía. Los dos caminos convergen en `vistas::datasources_de` y en
`Motor::fisicas`, que ya son una noción para las dos cosas. Se migra documento a documento, y
ningún día es el día en que todo se rompe.

### 5.4 · Lo que se migra en el árbol

- `vendor/oos/examples/acme-retail` — dos bindings; pasa a v1alpha8 entero, y es el escaparate.
- `vendor/oos/packages/*` — lo que tenga bindings.
- `crates/ore-exec/casos/*` — cuatro casos con bindings y uno con vistas.
- `conformance/v1alpha7` — trece casos; cada uno tiene su gemelo v1alpha8, y el árbol v1alpha7 se
  queda como está, porque es la suite de una versión que existió.

---

## 6. `discover` cae resuelto: crear el catálogo foráneo

Con la tabla, descubrir deja de ser inferir y pasa a ser **espejar** — lo que Databricks hace al
crear un *foreign catalog*, lo que Fabric llama *metadata mirroring*. El inductor emite, **por
cada objeto del catálogo**, una `Table` con:

- `columns`: las columnas, con `physicalType` citado tal cual lo dijo el origen — un hecho;
- `reads`: lo que **ese driver** sabe empujar, y viene en el catálogo porque solo el driver lo
  sabe: `ore-read-postgres` declara `[eq, neq, in, range, isNull]` y `fullScan: cheap`; un
  lector de una API con cuota declara `forbidden` y sus `requiredFilters` — un hecho;
- `changes`: lo que el driver **sondeó**: en Postgres, `wal_level = logical` y `REPLICA IDENTITY
  FULL` dan `{retract, log}`; sin ranura, `{none, none}` y un aviso; en un lago, el formato de
  tabla da `{retract, snapshot}` — un hecho, o `none` dicho como tal.

Y encima, lo de siempre: una `Entity` en `DRAFT` por tabla, con `backedBy` a una vista trivial
que expone las columnas con nombre de identificador. Las decisiones pendientes —clave, familia
fechada, colisión— siguen siendo diagnósticos, y `ore review` sigue reinduciendo. Lo que cambia
es que **el inductor ya no emite ningún `Binding`** y que la mitad física de lo que emite no es
un borrador: es un espejo.

Lo que `discover` **no** propone, y no por falta de tiempo: `materialized` y `freshness`. Son
decisiones de operación con coste, y proponerlas sería exactamente inventar. Se proponen
**vacías**, y `OOS2020` dice dónde no puede quedar vacío.

---

## 7. Los casos de conformidad

`conformance/v1alpha8/`, y todos medidos contra el binario antes de escribir su `expects`.

| | caso | espera |
|---|---|---|
| valid | `table-compiles` — una tabla con las dos caras, sin nadie que la use | accept |
| valid | `view-over-table` — la vista renombra columnas reales | accept |
| valid | `entity-backed-by-view-over-table` | accept |
| valid | `stream-table-materialized` — `reads: none`, `materialized` puesto | accept |
| valid | `virtual-over-materialized-over-stream` — la raíz de lectura es la copia | accept |
| valid | `append-changes-back-an-event` — `mode: append` respaldando `nature: event` | accept |
| valid | `mixed-versions` — un binding v1alpha1 y una tabla v1alpha8 en el mismo paquete | accept |
| invalid | `table-datasource-undeclared` | `OOS2004` |
| invalid | `field-not-a-column` — la vista nombra una columna que la tabla no tiene | `OOS2018` |
| invalid | `upsert-without-key` | `OOS2018` |
| invalid | `witness-field-not-a-column` | `OOS2018` |
| invalid | `from-table-does-not-exist` | `OOS2018` |
| invalid | `stream-view-not-materialized` — **lo que no se puede leer se debe materializar** | `OOS2020` |
| invalid | `append-changes-back-a-mutable-entity` — **sin retractación no se mantiene lo mutable** | `OOS2021` |
| invalid | `binding-in-v1alpha8` — el kind retirado | `OOS1003` |
| invalid | `materialized-view-leaks-entity-label` — el gemelo del de v1alpha7 | `OOS4002` |

Los dos que valen dinero son `OOS2020` y `OOS2021`: son los que ningún competidor comprueba al
compilar. Databricks lo descubre cuando `readStream` no existe para una foreign table; Foundry lo
documenta como limitación. Aquí no compila.

---

## 8. Los peldaños

> **Desde aquí es desechable.** Cada peldaño dice qué es, en qué teoría o práctica se apoya,
> y **cuándo está listo** con algo que se puede medir. Un peldaño sin criterio de listo es una
> intención.

| | qué | dónde | listo cuando |
|---|---|---|---|
| **T0** | v1alpha8 en la especificación | `C:\oos` | el marcador `borrador_de_v1alpha8` corre en verde con los 16 casos; v1alpha1–7 **sin un solo cambio** |
| **T1** | el núcleo lee tablas | `ore-core` | `cargo test -p ore-core` verde con `Fuente::Tabla`; `OOS2020` y `OOS2021` emitidos; `casos/con-vista` migrado y CI verde en `ore-exec` |
| **T2** | la costura y el ejecutor | `ore-cli`, `ore-exec` | `ore view` imprime las dos caras; las tres pruebas de `vistas.rs` pasan sobre tablas; `Motor::fisicas` llega a la tabla |
| **T3** | el catálogo foráneo | `ore-cli/inductor.rs`, `ore-read-*`, `ore-driver` | `pruebas-de-fuego/descubrimiento.sh` verde emitiendo `Table` y **cero** `Binding`; lo inducido falla en `validate` solo por decisiones pendientes, nunca por una tabla |
| **T4** | la migración del árbol | `vendor/oos`, `crates/ore-exec/casos` | `examples.rs` verde con `acme-retail` en v1alpha8; ningún `from.datasource` ni `kind: Binding` fuera de `conformance/v1alpha1–7` |
| **T4b** | la federación, decidida | `C:\oos` | `00-scope` §6 dice que una entidad servida desde N objetos **no entra**, y §6.1 dice por qué las cinco exclusiones son una sola frontera — la de la invertibilidad. **HECHO** |
| **T5** | el retiro | `ore-core/document.rs`, docs | `Binding` con `hasta`; `03-binding` marcado histórico; **este documento y `handoff-vistas.md` borrados** |

### T0 · la especificación

**Qué.** `spec/v1alpha8/{00-scope, 01-table, 02-view}`, `schemas/v1alpha8/{table, view,
entity}.schema.json`, `OOS2020` y `OOS2021` en `99-errors`, los dieciséis casos, el README.

**Teoría.** La dualidad stream/tabla de Flink como regla de la versión; las tres codificaciones
como vocabulario cerrado de `changes.mode`. Cerrado por lo mismo que `predicatePushdown`: si un
perfil pudiera inventar una codificación, el mantenedor no podría razonar sobre sus pesos.

**Listo cuando.** El marcador corre y está en verde. Y la comprobación que importa: `cargo test
-p ore-cli --test conformance` **entero** igual que antes — v1alpha8 no puede cambiar un solo
resultado de v1alpha1 a v1alpha7.

**No hace.** No toca ningún esquema anterior. `schemas/v1alpha7/view.schema.json` se queda:
describe los documentos v1alpha7 que siguen siendo válidos.

### T1 · el núcleo

**Qué.** `Kind::Table`, claves, `hasta` en `Kind`; `vistas.rs` con `Fuente::Tabla`, `raiz()`
llegando a la tabla, `comprobar()` con las reglas de tabla y las dos nuevas; `flow` y
`governance` sin cambio de forma.

**Práctica.** La de la absorción: **una operación, tres consumidores**. `raiz()` es la única que
sabe llegar de una vista a su objeto físico, y `flow`, `governance` y el ejecutor la llaman. Si
la tabla se resolviera en tres sitios, divergiría en el que ninguna prueba ejerce.

**Listo cuando.** Todas las pruebas de `vistas.rs` pasan con tablas debajo, más una por regla
nueva; `flow::vistas_materializadas` sigue rechazando el caso del DNI dos eslabones arriba; CI
verde en `ore-exec` con `casos/con-vista` reescrito sobre una `Table`.

**No hace.** No borra el camino del binding. Un documento v1alpha1 lo sigue necesitando.

### T2 · la costura y el ejecutor

**Qué.** `ore-cli/vista.rs`: `cuerpo()` construye `Lectura` desde la tabla, con `campos` de
`columns`; `capacidades_por_fuente` lee `reads`. `ore view` enseña, por raíz, las dos caras y qué
regla aplicó. `Motor::fisicas` llega a la tabla.

**Práctica.** `Capacidades::de_oos` ya lee el vocabulario de OOS una vez para dos consumidores;
`reads` es el mismo vocabulario en otro sitio, así que **no se escribe una segunda traducción**.

**Listo cuando.** Las tres pruebas de `tests/vistas.rs` pasan sobre tablas —incluida la que se
niega por la arista `INDIRECT`—; `ore view` sobre `stream-table-materialized` imprime `reads:
none · raíz de lectura: la copia`.

### T3 · el catálogo foráneo

**Qué.** El catálogo del driver gana `reads` y `changes` por tabla; el inductor emite `Table`,
`Entity` con `backedBy` y la vista trivial; ningún `Binding`. `ore-read-postgres` sondea
`wal_level` y `REPLICA IDENTITY`; `ore-read-jsonl` declara `{append, none}` — un fichero no
retracta.

**Teoría.** *Se emite lo que es un hecho.* Las dos caras son hechos del origen o del driver, y
por eso pueden emitirse sin revisión. Es la diferencia entre este descubrimiento y uno con un
modelo dentro: aquí no hay nada que adivinar en la mitad física.

**Listo cuando.** `descubrimiento.sh` en CI: `init → source add → discover → review → validate`,
con `Table` en la salida y `grep -c "kind: Binding"` a cero; y la afirmación que importa: `ore
validate` sobre lo inducido falla **solo** con códigos de decisión pendiente (`OOS2010` y
familia), nunca con un código de tabla.

**No hace.** No propone `materialized` ni `freshness`. No acuña conceptos. No singulariza.

### T4 · la migración del árbol

**Qué.** `acme-retail`, los paquetes publicados, los casos del ejecutor, y los gemelos v1alpha8 de
los trece casos de v1alpha7. A mano, siguiendo §5.1, porque son pocos y porque un comando de
migración sería una segunda implementación de la tabla de §5.1 que habría que mantener.

**Listo cuando.** `examples.rs` verde; `fuentes-reales.sh` verde contra PostgreSQL con
`acme-retail` en v1alpha8; y el recuento: cero `from.datasource`, cero `kind: Binding` fuera de
las suites de conformidad anteriores.

### T5 · el retiro

**Qué.** `Kind::Binding` con `hasta: V1Alpha8` y su `OOS1003`; `03-binding.md` con la cabecera
*«histórico: sustituido por `Table` y `View` en v1alpha8»*; este documento y `handoff-vistas.md`
borrados; `docs/view-engine.md` §6 actualizado.

**Listo cuando.** No queda ningún documento de trabajo que describa la migración, porque no queda
migración que describir. Es la condición de borrado de la cabecera.

---

## 8b. Dónde sigue esto cuando se acabe

Este documento se borra en T5. Lo que **no** se borra es
[`docs/sustrato.md`](sustrato.md): tablas y vistas no son la capa física de la ontología, son el
**sustrato**, y el modelo ontológico —versionado y ramificado— se construye encima y reposa ahí.
Allí están los tres movimientos que siguen —la cara `writes`, la entidad que deja de repetir, la
función que aterriza— y la medida que los motiva: nueve de once nombres de `acme-retail` están
escritos dos veces.

---

## 9. Lo que **no** entra, y no por falta de tiempo

**Unir, agregar, deduplicar, limitar en la gramática.** El IR los tiene con sus reglas y sus
medidas; entran cuando se decida su precio, y la tabla no lo cambia.

**`UNNEST`.** Un documento con un array no se aplana sin un operador que la gramática no tiene.

**Nulos.** `Valor` no tiene nulo y el almacén exige que una fila traiga exactamente las columnas
del plan. Es el límite real de lo semi-estructurado, y la tabla lo hace **visible** —`columns`
dice qué hay— sin resolverlo.

**Escritura.** Toda la industria coincide: el puntero es de solo lectura, y `05-ejecutor` §6.2
ya lo era. Databricks solo escribe sobre el metastore heredado; Snowflake acaba de abrir la
escritura sobre Iceberg externo y es otra pieza.

**Un `kind: Stream`.** §1.2. Sería un segundo nombre para una tabla sin cara de lectura.
