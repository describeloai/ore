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
- **Lo que dicen los tres sistemas de tipos de referencia, cotejados el 2026-09-20** (Foundry
  `SchemaFieldType`: 15 tipos —BYTE/SHORT/INTEGER/LONG, FLOAT/DOUBLE, DECIMAL(p,s), BOOLEAN,
  STRING, BINARY, DATE, TIMESTAMP, ARRAY, MAP, STRUCT—; Databricks/Spark: los mismos más
  `TIMESTAMP_NTZ`, `INTERVAL`, `VARIANT`, `GEOGRAPHY`; Iceberg: 14 primitivos, `timestamp` sin
  zona y `timestamptz` **guardado en UTC**, `time`/`timestamp` en microsegundos, decimal P ≤ 38):
  1. **Todos tipan en la tabla/dataset**, y ninguno entrega texto a la celda. Lo que sí hacen
     todos con lo que llega **sin tipos** (CSV, JSON) es exactamente lo nuestro: columnas
     `string` hasta que alguien aplica un esquema (Foundry: «Apply a schema» infiere sobre una
     muestra; Databricks: `inferSchema=false` → todo cadena, y para producción «esquema
     explícito»). Un origen JDBC llega tipado en los tres porque el conector lee sus tipos —
     que es nuestro caso con Postgres y BigQuery, y donde hoy fallamos.
  2. **Dos instantes, no uno**: Spark/Databricks/Foundry `TIMESTAMP` = instante (normalizado a
     UTC, se pinta en la zona de sesión) y `TIMESTAMP_NTZ` = hora de pared; Iceberg
     `timestamptz` (UTC) y `timestamp`. Nuestro `DateTimeTz`/`DateTime` es ese par, letra por letra.
  3. **Anchos**: ellos distinguen BYTE/SHORT/INTEGER/LONG y FLOAT/DOUBLE; OOS tiene `Integer` y
     `Float`. Se resuelve ensanchando sin pérdida (`int64`, `float64`), que es la promoción que
     Iceberg admite (int → long, float → double, decimal(P) → decimal(P′ > P)) y **la única**
     evolución de tipo que una copia sucesora de la misma vista puede hacer; estrechar es otra vista.
  4. **Un cast que falla**: Databricks en modo ANSI **falla** (`CAST`) o da **null** (`try_cast`);
     Foundry infiere sobre una muestra y el lote que no encaja rompe el build. Lo nuestro es la
     tercera vía y se elige a propósito: la columna **se queda texto, la copia sale, y el informe
     lo dice**. Ni un null inventado ni un build roto por una fila.
  5. **Anidados**: struct/map/array son de primera clase en los tres; OOS los aplana en el
     binding (v1alpha1) y sólo tiene `list<T>`. Es una brecha conocida frente a orígenes JSON, y
     se deja dicha: la copia los guarda si vienen (Arrow los tiene), el contrato no los promete.
  6. **Semánticos**: Foundry añade GeoPoint/GeoShape/TimeSeries/Attachment; Databricks
     GEOGRAPHY/GEOMETRY/VARIANT. OOS tiene `Money<>`, `Quantity<>` y tipos importados
     (`iso.CountryAlpha2`): el mismo mecanismo, otro catálogo. Después.
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
| **el árbol** | el mismo driver, que ya lo traduce (`escalar()`), y el inductor, que lo escribe desde T2 (antes lo tiraba) | el escalar de OOS: `String · Integer · Decimal · Float · Boolean · Date · Time · DateTime · DateTimeTz · Opaque` |
| **la copia** | `ore-store::carga`, al sellar | Arrow/Parquet, estrechado desde el escalar (abajo) |
| **la celda** | el SDK de cada lenguaje | el tipo nativo columnar del lenguaje (abajo), y el JSON de la consola |

**La tabla del contrato** (escalar de OOS → Arrow → lenguaje). «·» = exacto por construcción.

| OOS | Arrow / Parquet | pyarrow / pandas(ArrowDtype) | Node (DuckDB tipado) | Java (Arrow) | JSON de la consola |
|---|---|---|---|---|---|
| `Integer` | `int64` | · / `int64[pyarrow]` (nulable) | `bigint` | `Long` | número si \|x\| ≤ 2⁵³, si no **cadena** |
| `Decimal` | `decimal128(38, 18)`: la precisión **no se declaró** (el mismo por defecto que Foundry). Hasta T5 esta fila decía «p, s del `physicalType`», y no había código que lo hiciera | `Decimal` · | `DuckDBDecimalValue` · | `BigDecimal` · | **cadena** siempre (`"12345.6789"`) |
| `Decimal<p, s>` (T5) | `decimal128(p, s)` | `Decimal` · | `DuckDBDecimalValue` · | `BigDecimal` · | **cadena** siempre |
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

El driver ya traduce; el inductor deja de tirarlo. Se pensaron dos caminos y se hizo **uno**:

- ~~**Ya**, sin tocar el espec: derivar el escalar de `physicalType` en el compilador.~~ Se
  descartó al mirar el inductor: escribía `physicalType` **sólo** cuando el driver *no* supo
  traducir (`sourceType`), y tiraba el escalar cuando sí («el tipo es de la entidad»). Así que
  derivar de la cita habría sido inventar exactamente lo que el driver declinó, y para las
  columnas traducidas —la mayoría— no había cita de la que derivar.
- **Como toca**, en OOS: `Table.columns.<c>.type` con el escalar (lo que ODCS llama
  `logicalType`, con nuestro vocabulario), **junto a** `physicalType`, la cita del origen (que
  lleva la precisión y la escala que el escalar no lleva). Cambio de espec en `C:\oos`
  (`01-table.md` §5.0, `table.schema.json`, conformidad `table-compiles`) + bump del submódulo.
  El driver de Postgres emite el tipo **y** la cita; el inductor escribe los dos; el compilador
  tipa la raíz con la tabla y la entidad **afina** encima (`Money<EUR, 2>` sobre un `Decimal`).
  Con esto una vista **tiene tipo** —el de su columna, o el del agregado (`count()` →
  `Integer`, `avg()` → `Float`, `sum(Decimal)` → `Decimal`)— y el plan lo lleva sin defaults.
  Una columna sin `type` es la que el conector no supo traducir: texto para quien la lea, y se
  sabe que lo es porque nadie dijo otra cosa.

### 4 · La celda ve columnas (como quedó en T3)

`over()` y `sql()` devuelven **valores tipados según la tabla**, en la forma natural de cada
lenguaje, y lo columnar está a mano para lo masivo:

- **Python**: DataFrame de pandas **con `ArrowDtype`** por defecto (es la misma memoria de
  Arrow: un `int64` con nulos sigue siendo `int64`, un `decimal128` es `Decimal` exacto, un
  instante lleva su zona; 16,7 M filas/s en 10 M); `como="arrow"` da la `pyarrow.Table`,
  `como="polars"` un DataFrame de polars si está en la capa. Se prefirió pandas por defecto y
  no `pyarrow.Table` (como decía el borrador) porque es lo que una celda espera, y con
  `ArrowDtype` **nada se degrada**: no hay razón para hacer pagar la conversión a mano.
- **Node**: **filas** (objetos) con los valores tipados de DuckDB —`bigint`,
  `DuckDBDecimalValue`, `DuckDBDateValue`, `DuckDBTimestampTZValue` (`.micros` UTC),
  `DuckDBBlobValue`…—, **hasta un límite** (`LIMITE = 100 000`) y diciéndolo: `filas.total`,
  `filas.truncada`, `filas.tipos` (columna → tipo de Arrow). `{ como: "columnas" }` da arrays
  por columna sin objeto por fila. Un `count(*)` es `3n`: el tipo dice lo que es. No Arrow JS
  (T4: 11/23 y 21 MB).
- **Java**: **`Filas`** (`List<Map<String,Object>>` con `tipos`, `total`, `truncada`, hasta
  `LIMITE = 1 000 000`) con `Long`, `BigDecimal`, `LocalDate`, `LocalTime`, `LocalDateTime`,
  `Instant`, `byte[]`, `List`, `Map` —construidos **desde Arrow** (`arrowExportStream`), no
  desde `getObject` de JDBC, que tenía cuatro tipos mal—; y `arrow("p.v")` / `arrowSql("…")`
  dan el `ArrowReader` por lotes (`VectorSchemaRoot`) para recorrer 10 M de filas a 13,5 M
  filas/s sin un objeto por fila. La imagen lleva 13 jars (4,8 MB, `puesto/jvm/jars.txt`) y
  `--add-opens=java.base/java.nio=ALL-UNNAMED`.
- **El JSON de la consola** lo hace **el SDK** (`ore.tabla(valor)` / `ore.jsonDe`), igual en
  los tres, y los agentes sólo le ponen el límite de filas; una celda puede pedirlo. `columnas`
  lleva el tipo de Arrow con el nombre de `pyarrow` (`int64`, `decimal128(18, 4)`,
  `timestamp[us, tz=UTC]`…). Una precisión que la tabla no tenía: un **decimal de escala 0**
  (el `HUGEINT` de un `sum(1)` o un `count`) es un entero y va como los enteros; la fecha-hora
  lleva `T`, segundos siempre y la fracción sólo si no es cero, sin ceros de más.

### 5 · Lo que no sobrevive, se dice

Cada conversión con pérdida posible tiene un sitio donde decirse: el informe de la copia
(`columnas_sin_estrechar`), la tabla de arriba (JSON), y **`estricto`** donde hay algo que
degradar: en Node y Java, el límite de filas —`over(v, { estricto: true })` /
`over(v, limite, true)` fallan en vez de recortar—. En Python no existe el parámetro porque
con `ArrowDtype` no hay conversión con pérdida que pedir que falle. Nada se degrada en
silencio: es la diferencia entre un contrato y una costumbre.

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

### T1, hecho: la tabla como código

- **`ore_core::tipos`**: `Fisico` (el tipo Arrow/Parquet de cada escalar: `Texto · Entero · Real ·
  Logico · Decimal{p,s} · Fecha · Hora · FechaHora · Instante`), `Fisico::de(&Type)` (la tabla de
  §1: `Money<EUR,2>` → `decimal128(38, 2)`; `Opaque`, `list<T>` e importados → texto por ahora),
  `Fisico::analizar(texto) -> Option<Valor>` (la forma canónica: la que `ore-read-postgres::texto`
  emite —`2020-02-29 10:00:00.5+00`, `true`, `12345.6789`— más `T`, `Z` y `±HH:MM` en instantes) y
  `Valor::texto(&Fisico)` (la vuelta; `analizar(v.texto()) == v`). Sin Arrow en el núcleo: el
  físico es un enum propio y `carga.rs` lo lleva a `DataType` en diez líneas. Lo que no analiza
  contesta `None` y **no se inventa** (`" 1"`, `+1`, `1.0` como `Integer`, `t` como `Boolean`,
  `2021-02-29`, `24:00:00`, un instante sin desfase, un `1.7E9` como `DateTimeTz`, un decimal con
  más de 18 decimales o de 38 dígitos).
- **`ore-store::carga`** estrecha **todos** los escalares por esa tabla (revisa 0015). Por columna:
  se analiza todo y sólo si todo analiza se estrecha; si un valor no cabe, **la columna entera
  queda `string`** y `escribir` devuelve `Carga { bytes, sin_estrechar: columna → "3 de 120
  valores no son DateTimeTz: el primero es `…`; la columna se queda como texto" }`. Ni nulo
  silencioso ni copia rota. `leer` vuelve cada columna estrechada a su forma canónica, así que
  base leída + incremento del origen y resellar dan **los mismos bytes** que sellar de golpe (hay
  prueba), y dos desfases del mismo instante son la misma copia.
- **El informe**: `sellar` saca `sin_estrechar`; `ore materialize` lo pone en
  `copias/<p>_<v>.json` como `columnas_sin_estrechar` y en la línea de la pasada
  («⚠ 2 columnas sin estrechar (se quedan como texto): id — 1 de 2 valores no son Integer…»).
  `pruebas-de-fuego/almacen-r2.sh` §4 pasa de «se niega» a «sella y lo dice».
- **Lo que cambia para quien lee**: nada hasta T2, porque la cabecera de las copias de hoy dice
  `String` para todo (el plan pone `String` a lo que la vista no tipa). Con `Integer`/`Decimal`
  declarados en una entidad (el e2e `la-pregunta-se-contesta.sh`), la copia ya sale con `int64` y
  `decimal128(38, 18)` y `ore ask` contesta lo mismo.

### T2, hecho: el escalar llega al árbol

- **OOS** `65715b8`: `Table.columns.<c>.type` (escalar de OOS, opcional; su ausencia significa
  «el conector no supo»), `physicalType` pasa a ser la cita que convive con él; `01-table.md`
  §5.0 «El tipo vive en la tabla»; `table-compiles` lleva las dos formas.
- **`ore-read-postgres`** emite `type` **y** `sourceType` (antes uno u otro); **el inductor**
  escribe `id: { type: Integer, physicalType: bigint }`, y `payload: { physicalType: jsonb }`
  para lo que no tradujo; **`ore-core::vistas::tipos_de_columnas`** lee la tabla;
  **`tipos_de_raiz`** tipa primero con las tablas y después la entidad afina; **`GET /esquema`**
  enseña `type` por columna.
- Lo que falta para que la precisión llegue al Parquet (`numeric(10,2)` → `decimal128(10, 2)`
  en vez de `(38, 18)`): la cabecera de la copia lleva sólo el escalar. Es cosmético —
  `decimal128` ocupa 16 bytes con cualquier precisión— y queda para T3 si el SDK lo necesita.
  **No era cosmético** (T5, medido el 2026-09-26): `(38, 18)` admite 20 cifras enteras, un
  `NUMERIC` de BigQuery 29, y basta un valor que no quepa para que la columna entera se quede
  como texto. Los 16 bytes eran ciertos; la conclusión no.

### T3, hecho: la celda ve el contrato

`pruebas-de-fuego/medida-w3-leer.py` (rehecha para medir el contrato: lo que la consola ve por
`ore.tabla()`, cotejado con la verdad de pyarrow; 2026-09-20, en local):

| | over · sql | tipo nativo | 10 M de filas |
|---|---|---|---|
| **Python** | 23/23 · 23/23 | `int64[pyarrow]`, `decimal128(18, 4)[pyarrow]`, `timestamp[us, tz=UTC][pyarrow]`… | `over()` entero 598 ms (16,7 M filas/s); sumar una columna 19 ms; `sql()` group by 109 ms |
| **Node** | 23/23 · 23/23 | `bigint`, `DuckDBDecimalValue`, `DuckDBTimestampTZValue`, `DuckDBBlobValue`… | `over()` 100 000 de 10 M en 381 ms y lo dice (`truncada`); `{como: "columnas", limite: 10 M}` 11,0 s; `sql()` group by 186 ms |
| **Java** | 23/23 · 23/23 | `Long`, `BigDecimal`, `LocalDate`, `LocalTime`, `LocalDateTime`, `Instant`, `byte[]` | `over()` 1 M de 10 M en 1,1 s y lo dice; `arrow()` recorre las 10 M en 741 ms (13,5 M filas/s); `sql()` group by 84 ms |

Antes de T3 (19-09): pandas perdía enteros con nulos y decimales grandes, Node daba los
`bigint` como cadena y el JSON de cada agente era distinto, Java tenía ns → 1970, `time` →
00:00, `map` → `{}` y decimal → double. Ahora los tres agentes emiten **el mismo JSON** desde el
SDK, y `--comprobar` (la imagen, al construirse) ejerce un `bigint`, un `decimal(4,2)` y un
`timestamptz` por el camino real: si faltan los jars o el `--add-opens`, falla la imagen y no la
primera celda de una persona.

Lo que la tabla de §1 no decía y T3 fijó: el decimal de escala 0 es entero (arriba); `-0.0` sale
`0` en Node (JSON no tiene −0; se acepta); los anidados se nombran `struct`/`map` en Node
(DuckDB no da los hijos por nombre sin más trabajo) y con los hijos en Python y Java.

## Lo que se acepta a cambio

- **Un cambio de espec** (`Table.columns.<c>.type`) y **rehacer las copias** para que lleven
  tipo: son artefactos nuevos con otro digest (una copia sucesora, 0017 §A); las viejas siguen
  siendo válidas como texto.
- **Sellar analiza texto**: cuesta CPU en el Job de copia (se mide en W3.5 con 200 M de filas) y
  puede dejar columnas sin estrechar cuando el origen mezcla formas; se dice en el informe.
- **`DateTimeTz` pierde la zona del origen** (se guarda el instante en UTC). Es la decisión de
  Arrow/Parquet/Spark/BigQuery; la alternativa (guardar zona por valor) no existe en Parquet.

## Lo que se aparca hasta W3.6: Iceberg como formato de dataset del bucket

Preguntado el 2026-09-20, decidido en principio y aplazado con su medida. Iceberg es tres cosas:
Parquet como fichero, una capa de metadatos (esquema, particiones, *snapshots* con sus
manifiestos) y un catálogo cuyo único deber es el *swap* atómico del puntero al `metadata.json`
vigente. Nosotros ya tenemos las tres en formato propio: Parquet dentro de `ORECOPY1`, el informe
`copias/<p>_<v>.json` como metadato, y **el árbol como catálogo** —el commit es el swap atómico,
`git log` es el *time travel*, y la cadena de copias sucesoras (0017 §A) es un log de snapshots
hecho a mano. Lo que hace un catálogo Iceberg (Polaris, Nessie, BigLake) es lo que hace nuestro
git: guardar la referencia actual de cada tabla y cambiarla de forma atómica y con historia.
Nuestro git *es* nuestro Polaris, hecho en casa y con la ontología al lado.

**Se adopta** Iceberg como **estándar de dataset del bucket** —lo que un `write()` produce
(W3.6) y todo lo incremental o multi-fichero—, con git como catálogo: el informe en el árbol lleva
el puntero al `metadata.json` y el commit lo cambia. Lo que se gana es que BigQuery (BigLake),
Snowflake, Spark/Databricks y DuckDB **leen el dataset en sitio, sin moverlo**, y que la
evolución de esquema con las promociones que este contrato adopta (int→long, decimal P↑) y el
*append* con aislamiento vienen resueltos en vez de reinventados sobre el sobre. El contrato de
tipos ya es el suyo (µs, timestamptz en UTC, decimal ≤ 38): no hay traducción.

**La copia sigue siendo `ORECOPY1`** mientras sea «una vista sellada en un fichero por digest»:
para 31 copias de 71–112 k filas y 2–20 columnas, manifiestos y snapshots son peso sin beneficio,
y el sobre es más simple y más honesto (0015). El día que una copia sea incremental
(`changes.mode: append`) o multi-fichero, es un dataset y va por Iceberg.

**Medido antes de diseñar `write()`** (`pruebas-de-fuego/medida-w3-iceberg.py`, 2026-09-20; PyIceberg
0.12, DuckDB 1.5 con la extensión `iceberg`, git; en local, y §7 contra el bucket de demo):

- **El catálogo es git, y funciona.** `ArbolCatalog` (en la medida) es un catálogo de PyIceberg
  cuyo estado es un repo: `catalogo/<ns>/<tabla>.json` apunta al `metadata.json` vigente y el
  commit es el *swap*. Dos escritores desde el mismo snapshot: el segundo **choca** contra el
  puntero del árbol (`assert-ref-snapshot-id`), refresca y reintenta —un *append* conmuta: 2
  snapshots, como Iceberg manda—; dos cambios de esquema a la vez: el segundo **se niega**. El
  commit cuesta 210–260 ms aquí (dos procesos `git` en Windows; con el cliente git de
  `ore-serve` en Linux son decenas de ms) y **no depende del tamaño del árbol** (2 000 ficheros:
  igual). `git log -- catalogo/ventas/grande.json` es la historia de la tabla.
- **Escribir 10 M de filas**: Iceberg 0,3 + 4,0 s, **39 MB** (zstd) en 1 fichero de datos + 4 de
  metadatos (9 KB); el sobre de hoy 4,0 s, 71 MB (snappy), 1 objeto. Mismo tiempo, la mitad de
  bytes.
- **Leer sin catálogo**: DuckDB `iceberg_scan('<metadata.json>')` con el puntero del árbol,
  `group by` sobre 10 M en 121 ms (la copia por `read_parquet`: 82); PyIceberg → Arrow 372 ms
  (pyarrow sobre la copia: 388); `scan(pais = 'A')` poda por estadísticas del manifiesto. ⇒
  `over("p.dataset")` en los tres SDK es el mismo DuckDB de hoy apuntado al `metadata.json`.
- **Evolucionar**: *append* de 1 M sobre 10 M en 542 ms **sin reescribir los 10 M** (la copia
  de hoy los reescribe enteros); `int → long` + columna nueva en 215 ms **sin tocar un fichero
  de datos**; el snapshot 1 se lee desde DuckDB en 14 ms y desde PyIceberg en 274. Es
  exactamente lo que los transforms incrementales (0031 §7.4) y las promociones de este
  contrato piden.
- **Los tipos**: **19 de 23** entran en Iceberg v2 y vuelven exactos por PyIceberg y por
  DuckDB. Los 4 que no: `uint64` (no existe), `timestamp[ns]` (v3, o bajar a µs: nuestro
  contrato ya es µs), un instante con zona que no sea UTC (hay que convertir antes: nuestro
  contrato ya lo hace), y el tipo `null`. `int8/16` suben a `int`, `large_string` y el
  diccionario a `string`. El contrato de 0032 §1 cabe entero; `write()` convierte lo que no
  entra y lo dice.
- **La copia pequeña** (100 k × 3, como las 31 de demo/victor): sobre 29 ms, 627 KB, 1 objeto;
  Iceberg **493 ms, 5 objetos y 2 commits** por 319 KB. Los metadatos pesan 9 KB (1,4 %); lo
  que cuesta son objetos y commits, y para una vista sellada por digest no compran nada.
- **Los metadatos crecen por commit**: 50 *appends* de 1 fila = 151 ficheros, 1,4 MB. Hace
  falta `expire_snapshots`/compactación como mantenimiento (un Job), igual que en cualquier
  lago; no es un problema, es una tarea.
- **En el bucket** (§7, contra GCS de verdad desde esta máquina, donde un PUT de 300 B cuesta
  473 ms): 1 M de filas, sobre 1,5 s (1 PUT), Iceberg 2,6 + 2,8 s; un commit de 1 fila **2,0 s**
  (4–5 idas serie a GCS + 250 ms de git); leer 1 M desde GCS con PyIceberg 2,0 s. En el
  clúster, misma región, cada ida son 20–50 ms: un commit queda en 150–300 ms. El coste de
  Iceberg es **latencia por commit**, no caudal.

**Lo que sale de la medida**: se confirma la decisión de arriba. Iceberg es el formato de
**dataset** del bucket —lo que `write()` produce y todo lo que evoluciona— con **git como
catálogo** (el puntero en el árbol, el commit como *swap*, la forja rechazando el push que no
es *fast-forward* como CAS entre pods), leído por el mismo DuckDB de los tres SDK y por
cualquier motor; la **copia** sigue `ORECOPY1`. Para W3.6 quedan por diseñar, ya con números:
dónde vive el puntero en el árbol (junto al documento del dataset), el mantenimiento de
snapshots, y el escritor de producción (PyIceberg en el puesto Python hoy; `iceberg-rust` en
`ore-store` cuando `write()` salga de un Job; Java por DuckDB). No medido, porque exige el
clúster o pago: BigQuery (BigLake) leyendo el `metadata.json` en sitio.

## Los peldaños

| | qué | acepta |
|---|---|---|
| **T1** ✓ 2026-09-20 | la tabla de arriba como código: `ore_core::tipos` (escalar ↔ físico ↔ forma canónica del texto) con sus pruebas; `carga.rs` estrecha por ella; el informe de la copia dice lo no estrechado | hecho, abajo; la copia de `olist.products` llevará `price: decimal`, `recorded_at: timestamp[us, UTC]` cuando T2 ponga el escalar en la cabecera (hoy el plan dice `String`); `medida-w3-tipos.py` deja de decir `string×218` con T2 + rehacer |
| **T2** ✓ 2026-09-20 | el escalar al árbol: `Table.columns.<c>.type` en OOS (`oos@65715b8`), el driver emite tipo y cita, el inductor los escribe, `tipos_de_raiz` tipa con la tabla y la entidad afina | hecho: `ore view` de una vista sobre una tabla tipada sin entidad da `id: Integer · sueldo: Decimal · desde: Date` (prueba `el_tipo_de_la_tabla_llega_al_esquema_del_plan_y_la_entidad_lo_afina`); en el clúster, tras desplegar: re-inducir el paquete (`POST /paquetes/{n}/copia` o rehacer la database) → la cabecera de la copia cambia → la pasada siguiente sella copias tipadas y `medida-w3-tipos.py` deja de decir `string×218` |
| **T3** ✓ 2026-09-20 | la celda: valores tipados en los tres (pandas `ArrowDtype` · DuckDB tipado con límite · Arrow Java con `Filas` y `arrow()`), el JSON único en el SDK (`ore.tabla`), `estricto` donde hay algo que degradar | `medida-w3-leer.py`: **23/23 en los seis caminos** (`over`/`sql` × Python/Node/Java), 0 degradadas; `el-puesto.sh` 1–9 con los tres agentes; `--comprobar` de las dos imágenes ejerce el contrato al construirse |
| **T4** ✓ 2026-09-20 | Arrow JS y Arrow Java medidos con la misma matriz antes de entrar en las imágenes | arriba: Java → Arrow (23/23, 14,6 M filas/s, 5 MB); Node → DuckDB tipado por columnas (22/23, sin añadir nada) y no Arrow JS (11/23, 21 MB); en Node no se materializan 10 M de filas |
| **T5** ✓ 2026-09-26 | la precisión en la gramática: `Decimal<p, s>` (oos `0e6abc1`, `a2abba1`, `98e40ab`, `cc959f0`); los drivers la dicen; la copia la usa; el motor de vistas opera con ella | abajo; conformidad 82/82; `bigquery-real.sh` en verde contra BigQuery real: `NUMERIC` → `Decimal<38, 9>` → `decimal(38, 9)` |

## T5 · `Decimal<p, s>` (2026-09-26)

**Lo que se midió.** Con el estrechado real (`Fisico::analizar`), `(38, 18)` no admite un valor
de 21 cifras enteras ni el máximo de un `NUMERIC` de BigQuery (29); `(38, 9)` los admite todos.
Un `numeric` de PostgreSQL con más de 18 decimales no cabe en ninguno, y el `BIGNUMERIC`
(76 cifras) tampoco: Iceberg y Parquet paran en 38 (`MAX_DECIMAL_PRECISION` de la crate
`iceberg`). La precisión del origen vivía solo en `physicalType` —una cita que no se
interpreta— y se perdía en `vistas::tipos_de_columnas`, en el esquema de `ore-view` y en la
cabecera de la copia.

**Por qué la gramática y no un camino paralelo.** Un prototipo de ~25 líneas (una variante de
`Type`) llevó el tipo de BigQuery real hasta `decimal(38, 9)` en Iceberg sin tocar nada más:
el tipo ya viajaba por todas partes. Lo que no viajaba se vio también: una vista que filtraba
la columna por un literal dejaba de tipar, y unos diez sitios degradaban el tipo nuevo a texto
en silencio. Un mapa paralelo de `physicalType` habría tenido que cruzar las mismas costuras
sin que el compilador señalara ninguna.

**Lo que decide.**

| | |
|---|---|
| la forma | `Decimal<p, s>`, `1 ≤ p ≤ 38`, `0 ≤ s ≤ p` (02-entity §3.2). Fuera de rango, `OOS3002`; también en las `columns` de `Table` y `Dataset`, donde antes un tipo mal escrito se descartaba en silencio |
| `Decimal` a secas | sigue: precisión no declarada, físico `(38, 18)` |
| lo que no cabe en 38 | `String`, con la cita del origen: el valor exacto viaja como texto |
| el ensanche | sin perder cifras por ningún lado; declarar o retirar la precisión es `OOS5010` (02 §3.4, 91 §5.1) |
| la vista | `sum` → `Decimal<38, s>`, `avg` → `Decimal<38, max(s, 9)>`, `min`/`max` conservan; comparaciones y uniones en el supertipo; más de 38 cifras, no tipa; un `Integer` no se mezcla (02 §3.5) |
| la fusión | un decimal solo se convierte si ensancha: `arrow_cast` redondea `0.005` a `0.01` con `Ok`, también con `safe: false` (medido), así que la guarda es de `carga::al_esquema` y no del cast |
| la vuelta | `decimal(p, s)` de Iceberg vuelve como `Decimal<p, s>`, y `(38, 18)` como `Decimal`: ida y vuelta exactas |

**Lo que no decide.** La nulabilidad (`REQUIRED` del origen → `required` en Iceberg) se midió en
el mismo trabajo y se sacó: pide un cambio de spec, un análisis de nulabilidad en las vistas
(una junta por la izquierda, una columna calculada), y que Iceberg no deja endurecer una tabla
que existe. Queda para su propio ADR.
