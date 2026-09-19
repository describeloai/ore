# 0032 · El contrato de tipos: del origen a la celda

**Estado:** propuesto (medido el 2026-09-19 y 20; nada construido todavía) ·
**Fecha:** 2026-09-20 · **Decide:** que un valor tiene **un tipo en cuatro sitios** —el origen,
el árbol, la copia y el lenguaje de la celda— y que el contrato es la **tabla que los une**, con lo
que no sobrevive dicho en la tabla y no descubierto en la consola; que la copia deja de ser
«todo texto» y lleva **el tipo de OOS estrechado a Arrow** cuando el árbol lo sabe; que la verdad
en la celda es **columnar y Arrow**, y las filas-objeto una vista sobre ella; y que la salida
`tabla` de los tres agentes obedece **un solo JSON**. Es el verbo **leer** de
[`0031` §9](0031-el-puesto.md) (W3.5), y revisa el mapa conservador de
[`0015`](0015-el-protocolo-del-almacen.md).

## El problema

`over("p.v")` y `sql()` existen en Python, TS y Java (0031 W3.1–W3.4) y cada uno devuelve lo suyo.
Medido antes de decidir nada (`medida-w3-leer.py`, `medida-w3-tipos.py`):

- **Las 218 columnas de las 31 copias de demo y victor son `string`.** Todas. `price`,
  `recorded_at`, `created_at`, `event_timestamp`: texto. El origen sabía que eran `numeric`,
  `timestamptz`, `int4`; el driver lo tradujo al escalar de OOS (`ore-read-postgres::escalar`:
  `Decimal`, `DateTimeTz`, `Integer`); el inductor guardó en la `Table` sólo el `physicalType`
  (ODCS, opaco); la `View` no tiene tipo; el compilador puso `String` por defecto en el `esquema`
  del plan; y `ore-store::carga` —conservador a propósito (0015)— estrecha sólo `Integer` y
  `Boolean`. Resultado: la copia no sabe nada, y un `sum(price)` en DuckDB funciona por un *cast*
  implícito que nadie prometió.
- **Cuando la copia sí tiene tipos** (un Parquet con 23 tipos difíciles), cada lenguaje pierde lo
  suyo: pandas convierte enteros con nulos en `float64` (2⁵³+1 → …992) y decimales grandes en
  `float`; Node entrega `int64` como cadena y `decimal(38,10)` como número; Java (mi mapeo JDBC a
  mano) tiene cuatro tipos mal (ns → 1970, time → `00:00`, map → `{}`, decimal → double); y en
  los tres `NaN`/`inf` no viajan en JSON.
- **Materializar filas como objetos no escala**: 10 M de filas → Python 0,8 s (columnar), Node
  35 s, Java 16 s. `sql()` con `group by` tarda 60–140 ms en los tres: el motor no es el problema.

## Lo mirado

- **ODCS** (Open Data Contract Standard, absorbido por OOS en `Table.columns`): cada columna lleva
  `physicalType` (el del origen) **y `logicalType`** (`string`, `integer`, `number`, `boolean`,
  `date`, `timestamp`, `time`, `object`, `array`). OOS absorbió el primero y no el segundo; el
  escalar de OOS (`String, Integer, Decimal, Float, Boolean, Date, Time, DateTime, DateTimeTz,
  Opaque`, `Money<>`, `Quantity<>`, `list<>`) es más fino que el `logicalType` de ODCS y vive hoy
  sólo en la `Entity`.
- **Arrow** es el tipo lógico de facto: pyarrow, Arrow JS, Arrow Java, DuckDB, Polars, Spark,
  Snowflake y BigQuery lo hablan; Parquet es su almacén. Los tipos que importan aquí —enteros con
  nulos, decimales exactos, instantes con zona, ns, listas y structs— tienen forma en Arrow y no
  en JSON ni en pandas clásico.
- **Foundry** tipa el dataset en el *schema* del dataset (Spark types) y cada transform lo lee
  tipado; **Databricks** en Delta/Iceberg; **Snowflake** en la tabla. Ninguno entrega texto y
  deja al lector adivinar.
- **pandas 3 con `ArrowDtype`** (`to_pandas(types_mapper=pd.ArrowDtype)`) conserva enteros
  nulables, decimales y zonas; **`@duckdb/node-api`** tiene valores tipados por tipo
  (`DuckDBDecimalValue` exacto, `DuckDBTimestampTZValue`, `DuckDBMapValue`…) y acceso por
  columna (`DuckDBDataChunk`), sin Arrow JS; **DuckDB JDBC** exporta Arrow
  (`arrowExportStream`) si `arrow-vector` está en el classpath.

## Decisión

### 1 · Cuatro sitios, una tabla

| sitio | quién lo pone | vocabulario |
|---|---|---|
| **el origen** | el driver, al descubrir (`ore-read-postgres`, `ore-read-bigquery`, `ore-read-jsonl`) | el físico del origen (`numeric(10,2)`, `timestamptz`, `INT64`), citado en `Table.columns.<c>.physicalType` |
| **el árbol** | el mismo driver, que ya lo traduce (`escalar()`), y el inductor, que **hoy lo tira** | el escalar de OOS: `String · Integer · Decimal · Float · Boolean · Date · Time · DateTime · DateTimeTz · Opaque` |
| **la copia** | `ore-store::carga`, al sellar | Arrow/Parquet, estrechado desde el escalar (abajo) |
| **la celda** | el SDK de cada lenguaje | el tipo nativo columnar del lenguaje (abajo), y el JSON de la consola |

**La tabla del contrato** (escalar de OOS → Arrow → lenguaje). «·» = exacto por construcción.

| OOS | Arrow / Parquet | pyarrow / pandas(ArrowDtype) | Node (DuckDB tipado) | Java (Arrow) | JSON de la consola |
|---|---|---|---|---|---|
| `Integer` | `int64` | · / `int64[pyarrow]` (nulable) | `bigint` | `Long` | número si \|x\| ≤ 2⁵³, si no **cadena** |
| `Decimal` | `decimal128(p, s)` (p, s del `physicalType`; sin ellos, (38, 9)) | `Decimal` · | `DuckDBDecimalValue` · | `BigDecimal` · | **cadena** siempre (`"12345.6789"`) |
| `Float` | `float64` | · | `number` | `Double` | número; `NaN`, `inf`, `-inf` como **cadena** |
| `Boolean` | `bool` | · | `boolean` | `Boolean` | booleano |
| `String` | `string` (`large_string` si > 2 GB de columna) | · | `string` | `String` | cadena |
| `Date` | `date32` | `date` · | `DuckDBDateValue` | `LocalDate` | `"YYYY-MM-DD"` |
| `Time` | `time64[us]` | `time` · | `DuckDBTimeValue` | `LocalTime` | `"HH:MM:SS.ffffff"` |
| `DateTime` | `timestamp[us]` (sin zona: hora de pared) | `datetime` sin zona · | `DuckDBTimestampValue` | `LocalDateTime` | `"YYYY-MM-DDTHH:MM:SS.ffffff"` sin desfase |
| `DateTimeTz` | `timestamp[us, tz=UTC]` (**un instante**; la zona del origen no se guarda: se convierte) | `datetime` con `tzinfo=UTC` · | `DuckDBTimestampTZValue` | `Instant` | `"…Z"` en UTC, siempre |
| `Opaque` | `binary` (o `string` si el origen lo da como texto: `json`, `xml`, `inet`) | `bytes` / `str` | `DuckDBBlobValue` / `string` | `byte[]` / `String` | **base64** / cadena |
| `list<T>` | `list<T>` | `list` · | `DuckDBListValue` | `List<T>` | array |
| `Money<CUR,n>` · `Quantity<u,n>` | `decimal128(38, n)` + la unidad en los metadatos del campo | `Decimal` | `DuckDBDecimalValue` | `BigDecimal` | cadena |
| sin tipo (el árbol no lo sabe) | `string`, como hoy | | | | cadena |

Lo que **no** está en la tabla no se promete: `struct`, `map`, `uint64` y `timestamp[ns]` no
salen de ningún driver y no tienen escalar en OOS; si un `write()` (W3.6) los produce, la copia
los guarda tal cual (Arrow los tiene) y el JSON los entrega como objeto, lista de pares, cadena y
cadena — y el contrato lo dice, aquí, en vez de que la consola lo descubra.

### 2 · La copia lleva el tipo (revisa 0015)

`0015` estrechaba sólo `Integer` y `Boolean` porque «un `Decimal` metido en un `double` pierde y
una fecha reinterpretada a un huso ajeno es peor que una cadena honesta». Sigue siendo verdad, y
por eso la tabla de arriba no tiene ni un `double` para `Decimal` ni un huso ajeno: `Decimal` →
`decimal128` exacto, `DateTimeTz` → instante en UTC. Con eso, la objeción de 0015 se satisface y
el estrechamiento se completa: **`carga.rs` estrecha todo escalar de OOS según la tabla**. El
texto que el driver entrega se analiza al sellar con la forma canónica de cada escalar (ISO 8601
para fechas e instantes, decimal en texto para `Decimal`, `true`/`false`); **un valor que no
analiza no se inventa**: la columna entera queda como `string` y el informe de la copia lo dice
(`columnas_sin_estrechar: [recorded_at: "3 valores no son DateTimeTz"]`). Determinista: mismos
bytes para la misma entrada, que es lo que el digest exige.

### 3 · El escalar llega al árbol

El driver ya traduce; el inductor deja de tirarlo. Dos caminos, y se hacen los dos:

- **Ya**, sin tocar el espec: el compilador deriva el escalar de `physicalType` con la misma
  tabla del driver (`escalar()` de `ore-read-postgres`, la de BigQuery), sabiendo el `type` del
  `datasource` que hay en `ontology.config.yaml`. Es la traducción que el driver hizo, rehecha
  en el mismo sitio con la misma tabla; un `physicalType` que no traduce queda sin tipo (texto).
- **Como toca**, en OOS: `Table.columns.<c>.type` con el escalar (lo que ODCS llama
  `logicalType`, con nuestro vocabulario). Es un cambio de espec en `C:\oos` + bump del
  submódulo; el inductor lo escribe y el compilador lo prefiere sobre la derivación. Con esto una
  vista **tiene tipo** —el de su columna, o el del agregado (`count()` → `Integer`, `avg()` →
  `Float`, `sum(Decimal)` → `Decimal`)— y el plan lo lleva sin defaults.

### 4 · La celda ve columnas

`over()` devuelve **la tabla columnar del lenguaje**, y las filas-objeto son una vista:

- **Python**: `pyarrow.Table`; `como="pandas"` con `types_mapper=pd.ArrowDtype` (enteros
  nulables, decimales, zonas); `como="polars"` cuando esté en la capa.
- **Node**: hoy el resultado tipado de DuckDB por columnas (`DuckDBResultReader.getColumns()`,
  valores `DuckDBDecimalValue`, `DuckDBTimestampTZValue`…), que ya cumple la tabla sin Arrow
  JS; `filas()` como comodidad. Arrow JS (`apache-arrow`) **se mide antes de meterlo** en la
  imagen: no tiene decimales de verdad ni ns sin pérdida.
- **Java**: Arrow Java (`VectorSchemaRoot` que DuckDB JDBC exporta con `arrowExportStream`),
  **medido antes**: ~10 MB de jars y `--add-opens=java.base/java.nio`. Si no compensa, el
  camino JDBC con el mapeo por tipo arreglado (ns, time, map, decimal).
- **El JSON de la consola** (la salida `tabla` de los tres agentes) es la última columna de la
  tabla, igual en los tres: hoy cada agente tiene su `llano`, y discrepan.

### 5 · Lo que no sobrevive, se dice

Cada conversión con pérdida posible tiene un sitio donde decirse: el informe de la copia
(`columnas_sin_estrechar`), la tabla de arriba (JSON), y `over(…, estricto=True)` que falla en
vez de degradar. Nada se degrada en silencio: es la diferencia entre un contrato y una costumbre.

## Lo medido

`pruebas-de-fuego/medida-w3-tipos.py` (2026-09-20, demo y victor) y `medida-w3-leer.py`
(2026-09-19, en local, los tres SDK de verdad): arriba, en «El problema». La matriz completa por
tipo y lenguaje está en [`0031`, «Lo medido para W3.5 · leer»](0031-el-puesto.md).

### T4, medido (`pruebas-de-fuego/medida-w3-arrow.py`, 2026-09-20, en local): Arrow en Node y en Java

El mismo Parquet de 23 tipos y el de 10 M de filas, por los caminos **columnares** que existen,
antes de meter nada en una imagen:

| camino | fidelidad (de 23) | 10 M filas: cargar · sumar una columna | pesa |
|---|---|---|---|
| **Java · DuckDB JDBC → Arrow Java** (`arrowExportStream`, arrow-vector + c-data + memory-unsafe) | **23/23** | **684 ms · 166 ms → 14,6 M filas/s** (JDBC filas-objeto: 15,7 s) | 5,0 MB en 12 jars + `--add-opens=java.base/java.nio` |
| **Node · DuckDB tipado por columnas** (`getColumns()`: `DuckDBDecimalValue`, `DuckDBTimestampTZValue`, `DuckDBMapValue`…) | 22/23 (sólo −0 → 0) | 14,5 s · 244 ms → 0,7 M filas/s (filas-objeto: 34,6 s) | 0 (ya está en la imagen) |
| **Node · parquet-wasm + apache-arrow** | 11/23: fechas e instantes como **número de ms** (ns pierde), `time` como número, decimales como `Uint32Array(4)` sin aritmética, una lista con nulos → `[1,2,0]`, struct y map como arrays | 10,0 s · 344 ms → 1,0 M filas/s | 15,3 + 5,8 MB |

**Lo que decide:**

- **Java: Arrow.** Exacto en los 23 tipos y 20× más rápido que las filas-objeto por JDBC; el
  coste (5 MB, un `--add-opens` en el `CMD` de `puesto-jvm:1`) es nada. `over()` en Java
  devuelve `VectorSchemaRoot` y el mapeo por vector es el de `medida-w3-arrow.py` (ya escrito).
- **Node: DuckDB tipado por columnas, no Arrow JS.** La API de valores de Arrow JS no cumple
  el contrato (ni decimal, ni ns, ni nulos en listas) y pesa 21 MB para ir sólo 1,4× más rápido.
  Los valores tipados de DuckDB cumplen la tabla de 0032 §1 sin añadir nada a la imagen. Lo que
  NO arregla es la velocidad: materializar 40 M de valores en JS cuesta 14 s en cualquier camino
  (0,7–1 M filas/s), porque JS crea un objeto por valor. ⇒ **en Node no se materializan 10 M
  de filas**: `over()` devuelve columnas tipadas hasta un límite dicho (y `sql()` agrega en
  DuckDB, 60–140 ms); lo masivo es SQL o Python, no un `for` en JS. Cuando `@duckdb/node-api`
  exporte Arrow (está en su hoja de ruta), los `TypedArray` numéricos serán zero-copy.
- **Python: pyarrow** ya lo era (13 M filas/s); sólo cambia el `to_pandas` (`ArrowDtype`).

## Lo que se acepta a cambio

- **Un cambio de espec** (`Table.columns.<c>.type`) y **rehacer las copias** para que lleven
  tipo: son artefactos nuevos con otro digest (una copia sucesora, 0017 §A); las viejas siguen
  siendo válidas como texto.
- **Sellar analiza texto**: cuesta CPU en el Job de copia (se mide en W3.5 con 200 M de filas) y
  puede dejar columnas sin estrechar cuando el origen mezcla formas; se dice en el informe.
- **`DateTimeTz` pierde la zona del origen** (se guarda el instante en UTC). Es la decisión de
  Arrow/Parquet/Spark/BigQuery; la alternativa (guardar zona por valor) no existe en Parquet.

## Los peldaños

| | qué | acepta |
|---|---|---|
| **T1** | la tabla de arriba como código: `ore_core::tipos` (escalar ↔ Arrow ↔ forma canónica del texto) con sus pruebas; `carga.rs` estrecha por ella; el informe de la copia dice lo no estrechado | la copia de `olist.products` lleva `price: decimal`, `recorded_at: timestamp[us, UTC]`; `medida-w3-tipos.py` deja de decir `string×218` |
| **T2** | el escalar al árbol: derivación desde `physicalType` en el compilador (ya) y `Table.columns.<c>.type` en OOS (como toca) | un `sum(price)` en una vista es `Decimal` en el plan sin defaults |
| **T3** | la celda: `over()` columnar en los tres, el JSON único, `estricto` | `medida-w3-leer.py` sin `≠` fuera de lo que la tabla dice |
| **T4** ✓ 2026-09-20 | Arrow JS y Arrow Java medidos con la misma matriz antes de entrar en las imágenes | arriba: Java → Arrow (23/23, 14,6 M filas/s, 5 MB); Node → DuckDB tipado por columnas (22/23, sin añadir nada) y no Arrow JS (11/23, 21 MB); en Node no se materializan 10 M de filas |
