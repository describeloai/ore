# 0051 · ORE Null Contract

**Estado:** **aceptado · en construcción** (2026-10-02): Fase 0, P0, P1, P2 y el código de P3 hechos · **Decide:** cómo dice ORE, de punta a punta, qué
columnas **nunca son nulas**: el origen lo **declara**, las vistas lo **derivan** y el lago lo
**impone**, con una sola regla para cambiarlo. Cierra el «REQUIRED» de
[0042](0042-origin-rest-bigquery.md) y la parte de nulabilidad de
[0032](0032-el-contrato-de-tipos.md). Se apoya en [0045](0045-source-pointer.md) (el puntero lleva
la verdad del objeto) y en [0043](0043-ore-arrow-stream.md) (al contrato, sin modo seguro).

## Qué es

**ORE Null Contract es la garantía «esta columna nunca es nula», llevada sin perderse del origen
al consumidor.** Hoy el **dato** llega entero y la **garantía** se pierde por el camino. Todo
consumidor ve cualquier columna como posible nulo, los identificadores incluidos:
- GraphQL da `String` donde debería dar `String!`, y lo mismo el SDK del puesto;
- los motores (Spark, Trino, DuckDB) no pueden apoyarse en ella;
- un contrato no puede afirmar que una clave nunca es nula aunque el origen lo garantice.

No es un campo nuevo: son **tres capas**, cada una en su sitio, más una regla para cambiarla:

| capa | dónde | qué hace |
|---|---|---|
| **declarar** | en el origen: el driver y el Source Pointer | traer la garantía del origen como verdad del objeto |
| **derivar** | central: el compilador de vistas | calcular si una columna derivada puede ser nula |
| **imponer** | central: el almacén, en lo que se materializa | `required` en Iceberg, y rechazar el nulo al escribir |
| **evolucionar** | central: una regla | aflojar, libre; endurecer, sólo tras verificar o reescribir |

## Lo medido (2026-10-02)

**Los cuatro drivers ya leen la garantía.** El catálogo de cada uno trae `obligatoria` por
columna:

| driver | de dónde | qué es |
|---|---|---|
| `ore-read-bigquery` | `tables.get` → `mode: REQUIRED` (`catalogo.rs`) | **garantía** del origen |
| `ore-read-postgres` | `pg_attribute.attnotnull` (`main.rs:95`) | **garantía** del origen |
| `ore-read-s3` | el `nullable` del esquema del Parquet (`tabular.rs:58`) | **garantía** del fichero |
| `ore-read-jsonl` | «apareció con valor en todas las líneas» (`catalogo.rs:295`) | ⚠️ **observación**, no garantía: la línea siguiente puede traer un nulo |

**Se pierde en tres sitios:**
1. **El inductor la escribe como comentario** (`# NOT NULL en el origen`, `inductor.rs:1991`).
   Sí la usa para el `required` de las relaciones de una entidad (una foránea con todas sus
   columnas `NOT NULL`).
2. **La spec no tiene dónde decirlo en la capa física.** En v1alpha21, una columna de `Table` sólo
   admite `type`, `physicalType` y `description`.
3. **El almacén pone toda columna opcional** al escribir Iceberg (`ore-store/src/carga.rs:722`,
   `with_nullable(true)`).

**Lo que OOS ya tiene, y no hay que duplicar:**
- `required` en las propiedades de una `Entity`: la capa **semántica**, «el concepto exige el
  valor» (`document.rs`, `property_keys`);
- las aserciones de calidad **ODCS** en un `Ruleset`: la capa de **comprobación**.

Falta la capa **física**: las columnas de `Table`, `Dataset` y `View`.

## Lo que hace la industria

Tres capas distintas, nunca una:

1. **Declarar, en el origen.** Todo formato lleva la marca:
   - el `mode` de BigQuery, `NOT NULL` en Postgres y Delta;
   - `required` / `optional` en Iceberg y Parquet, el `nullable` de Arrow.

   Los conectores la traen: Debezium (`"optional": false`), Airbyte (que arregló llevarla al
   destino) y el esquema de un dataset de Foundry (`nullable`).
2. **Derivar, en el motor.** Spark (Catalyst) y Calcite calculan la nulabilidad de cada
   expresión: el lado que genera nulos de un `LEFT JOIN`, un `CASE` sin `ELSE`, un agregado sobre
   un grupo vacío. **Pero casi nadie la persiste en vistas:**
   - una tabla creada con SQL o una vista de BigQuery sale toda `NULLABLE`;
   - Spark fuerza nulable al leer Parquet (*«una pista, no una garantía»*);
   - **dbt no admite restricciones en vistas**: `not_null` sólo en tablas e incrementales.
3. **Imponer, central, en lo materializado.**
   - `NOT NULL` es una **restricción que se impone** en Databricks, Snowflake y BigQuery: la
     escritura que la viola falla. Las claves primarias y foráneas son **informativas** (`RELY`
     en Snowflake).
   - dbt la mete en el DDL del contrato.
   - Foundry la comprueba como *data expectation*.
   - ODCS separa el `required` del esquema de sus reglas de `quality`.

**La regla de evolución es universal: aflojar sí, endurecer no** sobre datos que existen.
- BigQuery deja pasar de `REQUIRED` a `NULLABLE`, nunca al revés.
- Iceberg igual; en v3 deja **añadir** una columna `required` si lleva `initial-default` y
  `write-default` no nulos.
- Databricks endurece (`SET NOT NULL`) **sólo tras comprobar todas las filas**.

**La tensión, a la vista:** imponer la garantía del origen al aterrizar rompe la carga cuando el
origen cambia (Airbyte con ClickHouse, un cursor `NOT NULL` que el origen manda nulo). Por eso los
ingestores son tolerantes y la imposición se pone **donde se materializa**, con la deriva
detectada.

## Lo propuesto

### N1 · Declarar: `required` en la columna de una `Table` (spec v1alpha22)

- `columns.<c>.required: true`: **«el origen garantiza que nunca es nula»**. Sin la clave, o con
  `false`, es nulable, que es lo de hoy: ningún árbol cambia de significado.
- El nombre es el de Iceberg y ODCS (`required`), y el mismo que ya usa la propiedad de una
  `Entity`. Una palabra en todas las capas.
- **Lo escribe el Source Pointer** (`ore source induce`), desde el catálogo del driver.
- **Sólo si es garantía.** JSONL declara una observación (lo visto en una muestra), y eso **no**
  es `required`. Su sitio es una aserción de un `Ruleset` («hasta hoy, nunca nula»), o nada.

### N2 · Derivar: las vistas calculan, no declaran

- **Una `View` y un `Dataset` no escriben `required`: se deriva.** Es la lección de dbt: una vista
  que lo declara a mano miente el día que alguien cambia un `JOIN`.
- `ore-view` lo calcula junto al tipo, con reglas fijas y **conservadoras** (ante la duda,
  nulable):
  - una columna leída tal cual hereda el `required` de su origen;
  - el lado que genera nulos de un `LEFT`, `RIGHT` o `FULL JOIN` es nulable;
  - un `CASE` sin `ELSE`, nulable; con `ELSE`, es obligatorio sólo si todas sus ramas lo son;
  - `COALESCE(a, …, x)` es obligatorio si **algún** argumento lo es;
  - `COUNT(*)` y `COUNT(x)` son obligatorios; `SUM`, `MIN`, `MAX` y `AVG`, nulables (un grupo
    vacío o todo nulo);
  - un literal no nulo es obligatorio; un `CAST`, lo que fuera su entrada;
  - `WHERE x IS NOT NULL` hace obligatoria a `x` aguas abajo;
  - una función que no se conoce, nulable.
- El resultado va al esquema de la vista, a la cabecera, a GraphQL (`!`) y al SDK.

### N3 · Imponer: `required` en Iceberg, y el nulo se rechaza

- **Un `Dataset` nuevo** escribe en Iceberg `required` las columnas que N2 deriva obligatorias.
- **Al contrato** (`carga::al_contrato`, 0043): un nulo en una columna `required` **no** se
  convierte ni se cuela. La carga falla con la tabla, la columna y la fila, sin snapshot. Es la
  misma regla «sin modo seguro» que ya rige para tipos y decimales.

### N4 · Evolucionar: aflojar sigue a la deriva, endurecer se verifica

- **El origen afloja** (pasa de `REQUIRED` a `NULLABLE`): el catálogo lo ve, `ore drift-detect`
  lo da como deriva, el Source Pointer quita el `required`, y el almacén **afloja** la columna
  Iceberg (lo permite) **antes** de la carga siguiente. Sin esto, la copia fallaría al primer
  nulo.
- **El origen endurece**, o una columna pasa a obligatoria por N2: en una tabla que ya existe
  **no se endurece Iceberg en el sitio**. Sigue opcional, y se dice (`ore lint`, la ficha), hasta
  una reescritura explícita que verifique todas las filas, como Databricks.
- **El lago de hoy** (todo opcional): son datos de prueba. Se reescriben al adoptar N3, o se
  quedan opcionales; se decide en N0.

### N5 · Las tres palabras, sin pisarse

| dónde | qué dice | quién la escribe |
|---|---|---|
| `Table.columns.<c>.required` | el origen garantiza el valor (física) | el Source Pointer, del driver |
| `View` / `Dataset` | derivado (física) | `ore-view`, nunca a mano |
| `Entity.properties.<p>.required` | el concepto exige el valor (semántica) | quien modela |
| aserción de un `Ruleset` | se **comprueba** que no hay nulos (calidad) | quien gobierna |

Una propiedad `required` de una entidad mapeada a una columna **no** obligatoria es un aviso
nuevo: la semántica pide lo que la física no garantiza, y el `Ruleset` es quien lo comprueba.

## La abstracción: `Nulabilidad`

Hoy la garantía es un `bool` suelto en cada capa: `obligatoria` en el catálogo, un comentario en
el inductor, un `true` fijo en el almacén. **Una sola abstracción, pegada al tipo**, la llevan
todas las capas (como el `nullable` del tipo de Calcite):

```
Nulabilidad = Garantizada { por: origen }   // la declara el origen (N1)
            | Derivada                       // la calcula ore-view (N2)
            | Nulable                        // por defecto: lo de hoy
```

Cada columna es `(Type, Nulabilidad)`. Hay **una** regla de combinación (la de las vistas, N2) y
**una** regla de evolución (N4). Ninguna capa decide por su cuenta.

## Los pasos

Los pasos se llaman **P** para no pisar las secciones **N** de lo propuesto.

### Fase 0 · La espiga desechable

En un worktree aparte (`ORE-0051-espiga`) que **se borra al terminar**. Un solo hilo de punta a
punta: una tabla de Postgres con una columna `NOT NULL`. Lo que sale es **conocimiento** (las
respuestas y la lista de sitios reales que hay que tocar), escrito en este ADR. **El código se
tira.** Límite: un día.

| # | qué se toca, a lo bruto | qué contesta |
|---|---|---|
| **E1** | el parser acepta `required` en la columna de una `Table`, sin spec ni esquema | cuántos sitios de `ore-core` hay que tocar para que un campo nuevo de columna viaje |
| **E2** | el inductor escribe `required: true` desde `obligatoria` | si llega intacto al Source Pointer |
| **E3** | `ore-view` lo deja pasar en una lectura directa, sin reglas | dónde se pierde hoy entre la `Table` y el esquema de un `Dataset` |
| **E4** | `ore-store` crea el campo Iceberg `required`; `al_contrato` rechaza un nulo | si `iceberg` 0.10.1 lo admite, y qué error da |
| **E5** | **el riesgo mayor:** aflojar `required → optional` en una tabla Iceberg que ya existe | si se puede sin reescribir; si no, N4 cambia de forma |
| **E6** | leer esa tabla con DuckDB y por el catálogo `/v1` desde Spark | si los motores respetan el `required` |
| **E7** | GraphQL y el SDK del puesto | si sale `String!`, y qué se rompe |

**E5 manda:** si no se puede aflojar sin reescribir, se para y se rediseña N4 antes de seguir.

#### Lo que contestó la espiga · E1–E3 (2026-10-02)

Hilo medido: un catálogo con `id` y `cliente` `required` y `nota` nulable → `ore discover` →
`Table` → la `View` que induce → una `View` encadenada con una expresión → una `View` con
`LEFT JOIN`. Compilado en Docker (`ore-core` ya no compila en local: `cedar` → `stacker` pide C).

| # | respuesta | qué cambia en el plan |
|---|---|---|
| **E1** | **El validador nativo no mira las claves de una columna de `Table`**: `required: true` pasa hoy, y `basura: 7` también. Sólo el esquema JSON publicado (`additionalProperties: false` desde v1alpha13) lo rechazaría, y eso lo arbitra la conformidad. Leerlo es **una función** (`vistas::obligatorias_de_tabla`) | P2 debe añadir la comprobación nativa de las claves de columna (hoy es un hueco aparte de 0051), o una errata en `required` pasa en silencio |
| **E2** | **Una línea en el inductor** y llega a la `Table` (`id: { type: Integer, physicalType: bigint, required: true }`). Pero se pierde en **cuatro consumidores** de las columnas: `assets.rs` (`tipos_de_tabla`), la API de `ore-serve` (`rutas.rs:2256`), la deriva (`deriva.rs` sólo compara `physicalType`: **hoy un origen que afloja no es deriva**) y `migrar.rs:272`, que al pasar una `Table` a `Dataset` **lo arrastraría** a un documento donde no se escribe | P3 toca esos cuatro; P5 añade «la nulabilidad» a la deriva, con su dirección (aflojar = ancha) |
| **E3** | **Hay dos mundos de vistas.** Las estructuradas van por el IR de `ore-view` (`Lectura`, 41 sitios, y entra en el digest del plan); son la forma de antes de v1alpha14, que `migrar_v14` convierte. **Las de hoy son SQL** (`vista_sql`), y ahí **la consulta no tipa: el tipo lo declara el contrato** (`spec.columns`). Derivar es una pasada sobre `vista_sql::Consulta`: salió `nunca nula id, cliente` en la inducida, `id, quien` en la encadenada (el renombre conserva, `upper()` no), y `—` en la del `LEFT JOIN` | P4 va en `vista_sql`, no en `ore-view`. `Consulta` **no guarda el tipo de join por relación**: hay que añadirlo, o el lado conservado de un `LEFT JOIN` sale nulable (la espiga lo da todo nulable). Y una decisión nueva: en una vista SQL el contrato **declara** el tipo; la nulabilidad se deriva y el contrato, si la lleva, se **coteja** contra lo derivado |

#### Lo que contestó la espiga · E4–E7 (2026-10-02)

Una tabla Iceberg real (`id` `required`, `nota` opcional), escrita con el `Lago` de `ore-store`
sobre un almacén en disco, y leída después con DuckDB, PyIceberg y Spark 3.5.6 (Iceberg 1.6.1).

| # | respuesta | qué cambia en el plan |
|---|---|---|
| **E4** | `iceberg` 0.10.1 **crea** la columna `required` sin queja, y su esquema en Arrow sale `nullable = false`. Un lote de hoy (todo nulable en Arrow) **sin nulos se escribe**. **Un nulo lo para el propio escritor**, antes del snapshot: `Column 'id' is declared as non-nullable but contains null values`. `al_contrato`, con el destino que sale de la tabla, lo para igual. Lo leído después: 2 filas, 0 nulos | **Imponer sale casi gratis**: basta con que `esquema_deseado` (hoy siempre `NestedField::optional`) escriba `required`. El mensaje es de Arrow y **no dice la fila**: eso sí es trabajo de P6 |
| **E5** | **Aflojar en el sitio funciona**: `AddSchema` (mismo id, `optional`) + `SetCurrentSchema`, sin reescribir; un nulo después se escribe, y se leen las 4 filas, viejas y nuevas. **Pero `Lago::esquema` no lo ve**: `mismo_esquema` compara nombre, tipo e id, no `required`, y contesta «no cambió» | **E5 pasa: N4 se queda como está.** P5 arregla `mismo_esquema` (una condición) |
| **E5b** | **`iceberg` 0.10.1 deja endurecer en el sitio con nulos dentro**, sin comprobar nada | **La guarda es nuestra**: endurecer sólo al crear o al **sobrescribir** (todo se reescribe, y el escritor ya comprueba cada fila); nunca al anexar ni al fundir. Va en P5, **antes** de P6 |
| **E6** | **Spark respeta `required`**: lo lee `nullable = false` y no deja escribir un nulo (`Null value appeared in non-nullable field`; con un `NULL` literal, 3.5.6 da un error interno del optimizador). **PyIceberg** también (`required` → Arrow no nulable). **DuckDB lo ignora**: `iceberg_scan` da todo `is_nullable = YES`. Y lo grave: **sobre la tabla endurecida con un nulo dentro, Spark contesta `count(id) = 4` y 0 nulos**, cuando hay uno. Se fía de la marca y la cifra sale mal sin error | Es la prueba de que **endurecer sin verificar es dar cifras falsas**, no un riesgo teórico. La guarda de E5b no es opcional |
| **E7** | **GraphQL ignora hoy el `required` de una propiedad escalar**: con `cliente: { type: String, required: true }` sale `cliente: String`. Sólo la clave sale `ID!`; `required` sólo se usa en relaciones y en parámetros de funciones. **El SDK del puesto lee con DuckDB** (`iceberg_scan`), así que la nulabilidad del lago no le llega; el contrato de Python es sólo de las firmas de `@function` | El `!` de GraphQL y el del SDK **tienen que salir del árbol**, de lo que P4 deriva, no del lago. Y una regla: `!` sólo cuando la física lo garantiza; el `required` semántico por sí solo **no** lo pone, porque mentiría |

**Lo que la Fase 0 cambia del plan, en limpio:**
- **P4 va en `vista_sql`** y necesita guardar el tipo de join por relación (E3).
- **P5 crece y va primero**: la deriva de la nulabilidad, `mismo_esquema` con `required`, y
  **la guarda que no deja endurecer** al anexar o fundir (E5b, E6).
- **P6 encoge**: el escritor ya rechaza el nulo. Queda escribir `required` en `esquema_deseado`
  y un mensaje con tabla, columna y fila.
- **P7**: GraphQL y el SDK leen la garantía **del árbol**. El `!` es físico y derivado; el
  `required` de una entidad sin respaldo físico es el aviso de N5, no un `!`.

**Sobre la abstracción**, lo medido la afina: la `Nulabilidad` va pegada al tipo, pero **se escribe
sólo cuando es garantizada** (p. ej. `Integer!` en la forma de texto). Así la cabecera de una copia
(`materializar::cabecera`, que hashea `t.to_string()`) y el digest de un plan **no cambian** en
ningún árbol sin `required`, y cambian, con su recálculo, justo en las copias que pasan a
imponer.

### Fase 1 · Lo robusto

Cada paso es **compatible hacia atrás por construcción** (sin `required`, todo significa lo de
hoy), tiene su go y su prueba.

| paso | qué | hecho cuando | riesgo | vuelta atrás |
|---|---|---|---|---|
| **P0 · medir en vivo** | en los catálogos de `demo` y `victor`, cuántas columnas son `obligatoria`, por driver; en el lago, **contar los nulos** de las que serían `required` (un origen que miente se ve aquí, no en producción) | una tabla en este ADR: candidatas y nulos reales (0 esperado) | ninguno, sólo lectura | — |
| **P1 · la abstracción** | `Nulabilidad` junto a `Type` en `ore-core`; siempre `Nulable`, sin tocar la spec ni el comportamiento | `cargo test --workspace` en verde y la salida de `view`, `lint`, `report` y `datasets` idéntica byte a byte en `demo` y `victor` | bajo | revertir el commit |
| **P2 · la spec v1alpha22** | en `C:\oos`: `required` en las columnas de `Table`, su esquema, su prosa y sus casos de conformidad (válido; inválido en `View` o `Dataset`; ausente = nulable); bump del submódulo | la conformidad en verde, los árboles v1alpha21 sin cambios | bajo | la versión es nueva; la vieja no se toca |
| **P3 · declarar** | el Source Pointer escribe `required` desde el catálogo (BigQuery, Postgres, S3; **JSONL no**); `ore migrate` lo añade a los árboles vivos | `el_puntero_es_del_objeto.rs` con dos casos nuevos; en vivo sólo cambian las columnas medidas en P0 | bajo: aún nada lo impone | `migrate` en seco primero; revertir el commit del árbol |
| **P4 · derivar** | `ore-view` aplica las reglas de N2; llega a la cabecera, a GraphQL (`!`) y al SDK | una prueba por regla, y **una propiedad**: nunca deriva obligatoria una columna que pueda ser nula (datos generados contra DuckDB) | medio: una regla mal hecha miente | reglas conservadoras; ante la duda, `Nulable` |
| **P5 · evolucionar antes de imponer** | `drift-detect` ve que el origen afloja; el Source Pointer quita el `required`; el almacén afloja la columna Iceberg **antes** de cargar, por la vía que valide E5 | prueba de fuego: el origen afloja, la copia siguiente pasa y la columna queda opcional | medio | aflojar es seguro por definición |
| **P6 · imponer** | un `Dataset` **nuevo** escribe `required` en Iceberg; `al_contrato` rechaza el nulo con tabla, columna y fila; las tablas existentes no cambian | prueba de fuego: nulo inyectado → falla sin snapshot; caso sano → `required` en el esquema Iceberg | **el más alto**: una copia puede empezar a fallar | **un interruptor por celda**; binario antes que malla; primero `demo`, luego `victor` |
| **P7 · lo visible y el lago** | el aviso entidad ↔ columna; la ficha enseña «nunca nula · lo garantiza el origen»; el lago de prueba se reescribe o no, según P0 | consola y `lint` | bajo | — |
| **P8 · cerrar** | este ADR, reescrito limpio con lo medido; las deudas, cerradas | «aceptado y en vivo» | — | — |

**El orden no se negocia:**
- **P5 antes que P6, siempre.** Imponer sin poder aflojar convierte el primer cambio de un origen
  en una copia rota.
- **La spec primero**, en `C:\oos` con bump del submódulo; **el binario antes que la malla**.
- **En vivo sólo P3, P6 y P7**, cada uno con su go. P1, P2, P4 y P5 no cambian nada visible
  mientras P6 esté apagado.

#### P0 · lo medido en vivo (2026-10-02)

Sólo lectura. Los árboles, de la forja de cada inquilino (`demo` en `a06883e`, `victor` en
`337d53f`); los números del lago, de los punteros de cada copia (`filas` y los no nulos por columna
de la última escritura; todas son `creada` o `sobrescrita`, así que cuentan la tabla entera).

**Los catálogos**, una vez por origen (hay paquetes que repiten el mismo: `olist` sale seis veces
en `demo` y dos en `victor`):

| origen | driver | tablas | columnas | `required` |
|---|---|---|---|---|
| olist (`demo` y `victor`) | Postgres | 48 | 342 | **88** (26 %) |
| standard (`victor`) | Postgres | 19 | 157 | **67** (43 %) |
| `bigquery_20260927_1428` (`victor`) | BigQuery | 3 | 13 | **3** |
| `bq` (`demo`, paquete `ventas`) | BigQuery | 2 | 8 | 3 |
| `s3_demo`, `s3_rol` (`victor`) | S3: 10 CSV, 1 JSONL, 1 Parquet | 12 | 69 / 80 | **0** |

- **Postgres y BigQuery declaran**, y mucho: una de cada cuatro columnas en `olist`, casi una de
  cada dos en `standard`.
- **S3 no declara nada.** Un CSV no puede, y el único Parquet no marca ninguna columna. **El JSONL
  tampoco promueve lo observado**: 0, como manda N1.
- El catálogo `ventas` de `demo` nombra una fuente `bq` que el manifiesto no declara: es un resto,
  ajeno a 0051.

**El lago**: 26 copias de una `Table` con puntero (3 en `demo`, 23 en `victor`). Hay 65 columnas
candidatas, todas en `victor`; **61 tienen cuenta, en 19 copias, y ninguna tiene un nulo**.

| | |
|---|---|
| copias medidas | 19: 17 de `standard_test` (Postgres) y 2 de `bq.ventas` (BigQuery) |
| la mayor | `prueba_de_pk`, 1 000 filas; el resto, de 0 a 57 |
| sin candidatas | las tres de `olist_copia` (`products`, 32 951 filas): `products`, `sellers` y `orders` **no tienen ningún `NOT NULL`** en el origen, ni clave primaria. Lo mismo `ore_e2e_sintetica` (2 000 000) y los pedidos de S3 (1 500) |
| fuera | `brain_embeddings` (0 filas, sin cuentas: sus 4 candidatas), `prisma_migrations` (no la crucé: su `Table` se llama distinto del objeto) y las colecciones de medios (no son copias) |

**Lo que se concluye:**
- **Ningún origen ha mentido**: 0 nulos en 61 columnas. Pero la prueba es **sobre poco dato**: las
  tablas grandes de hoy no tienen nada que garantizar. La prueba de verdad la da P6, que rechaza el
  primer nulo.
- **El lago de hoy (N4, P7) se reescribe al adoptar P6**, sin más: es poco (la mayor copia con
  candidatas tiene 1 000 filas), y todas las copias ya se escriben enteras (`creada` o
  `sobrescrita`), que es justo el único momento en que la guarda de E5b deja endurecer.
- **P3 cambia, en vivo, 158 columnas**: 88 + 67 en dos orígenes de Postgres y 3 en uno de
  BigQuery (más las 3 del resto `bq`), en cada paquete que los repite. Ninguna de S3.

#### P1 · P2 · hechos (2026-10-02)

- **P1 · la abstracción.** `types::Nulabilidad` (`Nulable < Derivada < Garantizada`, `y` toma la
  menor, `aguas_abajo` convierte lo garantizado en derivado, `sufijo` es `!` sólo si nunca es
  nula) y `types::Tipado`, que se escribe como su tipo más ese sufijo.
  `vistas::nulabilidad_de_columnas` lee `required` de una `Table`; nadie lo consume todavía.
  **Criterio cumplido:** `validate`, `lint`, `report`, `view` y `datasets` sobre los árboles de
  `demo` y `victor` dan **las 10 salidas idénticas byte a byte** antes y después.
- **P2 · la spec v1alpha22** (OOS `01-nunca-nula`): `columns.<c>.required` en `Table`, sólo
  garantía; `View` y `Dataset` no lo declaran; aflojar sigue al origen y endurecer exige verificar.
  Conformidad **5/5** (1 acepta; `OOS1005` antes de v1alpha22, en una vista y con
  `nullable: false`; `OOS1004` si no es booleano).
- **El hueco de E1 se cierra a medias, a propósito.** ORE comprueba ahora las claves de una columna,
  pero **todas sólo en una `Table` de v1alpha22**. Antes sólo mira `required`: la suite destapó
  árboles con `labels` en columnas de `Table` (el esquema JSON nunca las admitió, y
  `ore migrate punteros` las cuida). Cerrarlas en versiones viejas cambiaría lo que un árbol de
  ayer significa. Una tabla con `labels` en sus columnas no sube a v1alpha22 hasta quitarlas.

#### P3 · declarar · el código (2026-10-02)

- **El Source Pointer escribe `required: true`** en la columna que el catálogo trae obligatoria, y
  la `Table` declara entonces v1alpha22; una sin garantías sigue en la suya.
- **El lector JSONL deja de emitir `required`**: «con valor en todas las líneas» era una
  observación, y N1 no deja convertirla en garantía. Arreglado en el lector, no en el inductor: el
  inductor se fía del catálogo de cualquier driver.
- **Los consumidores la llevan**: la ficha del activo (`assets.rs`) y las dos rutas de
  `ore-serve` que enseñan columnas (de la `Table` y del catálogo), sólo cuando es `true`. Y la
  migración `Table` → `Dataset` (`migrar.rs`) **la quita**: en un dataset se deriva.
- **Aflojar sigue al origen** sin código nuevo: re-inducir regenera la tabla, y la columna que el
  origen dejó de garantizar pierde `required`. Lo fija una prueba.
- El comentario `# NOT NULL en el origen` de la entidad se queda: la deuda se cierra porque la
  garantía ya va en la `Table`, y quitarlo cambiaría ficheros vivos sin necesidad.

**En vivo, en seco** (`ore source induce` de cada fuente, desde su catálogo guardado, sobre copias
de los árboles): la parte de 0051 cuadra con P0. **21 tablas de `victor` pasan a v1alpha22 con
`required` en 70 columnas** (las 67 de `standard` y las 3 de BigQuery); en `demo` ninguna tabla
existente cambia. **Pero re-inducir escribe además lo pendiente de 0045 E5′**: +237 punteros en
`demo` y +70 en `victor`, y los `exports` de 7 paquetes. Aplicarlo en vivo, y cómo, se decide
aparte.

## Lo que no se hace

- **Imponer en el aterrizaje crudo.** La garantía se impone donde se materializa un `Dataset`, y
  la deriva del origen se detecta antes de cargar.
- **`required` escrito a mano en vistas.** Se deriva.
- **Endurecer una tabla Iceberg existente en el sitio.** Sólo con una reescritura verificada.
- **Claves primarias o únicas impuestas.** Son otra decisión: en la industria son informativas, y
  aquí la clave (`changes.key`) ya existe con su propio papel.
- **Convertir una observación (JSONL) en garantía.**

## Deudas que cierra

- [0042](0042-origin-rest-bigquery.md) · «REQUIRED → `required` en Iceberg».
- [0032](0032-el-contrato-de-tipos.md) · la nulabilidad, que T2 llamó «cosmética» y no lo era.
- `ore-store` · «todo opcional» (`carga.rs:722`).
- El inductor · la garantía del origen escrita como comentario.

## Fuentes

- [Apache Iceberg spec](https://iceberg.apache.org/spec/) · [valores por defecto en Iceberg v3](https://www.dremio.com/blog/dremio-iceberg-v3-default-column-values/) · [Snowflake: NOT NULL ADD COLUMN en Iceberg v3](https://docs.snowflake.com/en/release-notes/bcr-bundles/2026_05/bcr-2351)
- [BigQuery: modificar esquemas](https://docs.cloud.google.com/bigquery/docs/managing-table-schemas) · [BigQuery: esquemas](https://docs.cloud.google.com/bigquery/docs/schemas)
- [Databricks: restricciones](https://docs.databricks.com/aws/en/tables/constraints) · [Delta Lake: restricciones](https://docs.delta.io/delta-constraints/) · [Snowflake: restricciones](https://docs.snowflake.com/en/sql-reference/sql/create-table-constraint)
- [dbt: model contracts](https://docs.getdbt.com/docs/mesh/govern/model-contracts)
- [Spark: NullPropagation](https://jaceklaskowski.gitbooks.io/mastering-spark-sql/content/spark-sql-Optimizer-NullPropagation.html) · [Spark, Parquet y los nulos](https://medium.com/@weshoffman/apache-spark-parquet-and-troublesome-nulls-28712b06f836) · [Calcite RelDataTypeFactory](https://calcite.apache.org/javadocAggregate/org/apache/calcite/rel/type/RelDataTypeFactory.html)
- [Debezium, PostgreSQL](https://debezium.io/documentation/reference/connectors/postgresql.html) · [Airbyte #68516](https://github.com/airbytehq/airbyte/pull/68516) · [Airbyte #76476](https://github.com/airbytehq/airbyte/issues/76476)
- [Foundry: data expectations](https://www.palantir.com/docs/foundry/pipeline-builder/dataexpectations-overview) · [ODCS v3.2.0](https://bitol-io.github.io/open-data-contract-standard/v3.2.0/schema/)
