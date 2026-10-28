# 0077 · Origen ClickHouse

**Leer ClickHouse** —servidor propio (OSS) y ClickHouse Cloud— como origen de ORE. Es el primer origen
**columnar analítico**: SQL propio con muchos tipos compuestos, sin transacciones, y un motor
(`MergeTree` y familia) cuya semántica de duplicados y mutaciones condiciona la lectura incremental.

**Estado:** **X0 y X1 hechos y el paso 1 de la hoja de ruta (§6) hecho** (2026-10-27). X0 (§3.1–§3.9) midió el comportamiento de ClickHouse en 25.8, 25.3 y 24.8; X1 (§3.10) lo contrastó con la documentación oficial, con cómo leen sus clientes y con cuatro pruebas más (incluido Arrow Flight en 26.4). El binario `ore-read-clickhouse` existe y el kit tiene su banco: `--casos 13` pasa en 24.8, 25.3, 25.8 y 26.4. **Nada decidido todavía** (§1): falta cerrar qué formato, cómo se reparte (o no) la lectura, qué consistencia se declara y el suelo de versión; el paso 2 empieza con lo que no los toca. **Sin medir:** red con latencia, compresión en el cable, ClickHouse Cloud, Altinity, chDB, réplicas y `Distributed`, mTLS y JWT.

Sigue a [`0069`](0069-origin-mongodb.md), [`0070`](0070-origin-cassandra.md) y
[`0075`](0075-origen-oracle.md): medir primero, construir después; esqueleto `ore_driver::origen`; el kit
de conformidad como puerta.

## 1. Qué se decide

Decidido, con el paso de la hoja de ruta (§6) que lo construyó y lo prueba.

| Qué | Decisión | Paso |
|---|---|---|
| **Transporte** | HTTP síncrono con `ureq` 2 y el TLS del sistema (mínimo 1.2, sin apagarlo por la URL). **No** la crate `clickhouse`, **no** el protocolo nativo, **no** Flight (26.4 materializa el resultado) | 1 |
| **Formato** | `RowBinaryWithNamesAndTypes`: conserva el tipo exacto, la cabecera lo trae aunque no haya filas, y decodificarlo es barato (§3.5). Los tipos salen de esa cabecera, no del catálogo | 3 |
| **Flujo** | Uno solo por lectura: el servidor ya paraleliza, y una lectura por rangos no es consistente entre rangos (§3.3). El error a mitad de flujo se detecta por versión (§3.7) | 1 |
| **Versión mínima** | **24.8** (la más antigua medida). Por debajo, o si no se entiende, error `origen` | 2 |
| **Perfil** | `readonly=1` funciona (no manda ajustes); `readonly=2` manda los suyos; un `*_overflow_mode = 'break'` se rechaza, porque trunca en silencio | 2 |
| **Credencial** | usuario sin contraseña por HTTP en claro, o con contraseña sólo con `tls=true`; una CA propia por fichero no se ofrece | 1 |
| **Valores** | atados como parámetros del servidor con el tipo de la columna; **uno que no cabe es un error**, no otro valor | 5 |
| **Consistencia** | instantánea por lectura, ninguna entre lecturas; ver la nota del paso 8 | 8 |
| **`FINAL`** | **sí** en `Replacing`/`Collapsing`, no en `Summing`/`Aggregating`; el catálogo lo avisa | 8 |
| **Tipos** | ver la tabla del paso 4: `UInt64` → `Decimal<20, 0>`; `Enum`, `UUID`, `IPv4/6`, `FixedString` → texto; `Int128/256` y `Decimal` > 38 dígitos → error `esquema`; compuestos, `Time`, `JSON` y `AggregateFunction` aún no | 4 |
| **Claves** | ClickHouse no tiene claves únicas: `clave` y `unicas` van vacías | 8 |
| **Tiempo y cancelar** | `max_execution_time` + tope del cliente; cancelar es `KILL QUERY` por `query_id`. Límite medido: `readonly=1` sin `KILL` deja viva la consulta que no emite nada | 6 |

## 2. Lo que hacen los demás

Leído el 2026-10-27, sin contrastar con un servidor:

- **Airbyte** — `source-clickhouse` 0.4.0, sobre `source-jdbc` con el driver JDBC. Full refresh e incremental
  por cursor, SSL y túnel SSH. Servidor ≥ 21.3.10.1, usuario de sólo `SELECT`, un origen por base de datos.
  CDC y borrados incrementales: «coming soon». Sin tabla de tipos publicada.
- **Fivetran** — **no** tiene ClickHouse como origen. Sí como **destino** (sólo ClickHouse Cloud; código
  mantenido por ClickHouse, alojado por Fivetran).
- **ClickHouse** — para streaming de volumen propone ClickPipes, no conectores de terceros.

## 3. Lo que hay que medir (X0)

Orden de ejecución; cada bloque responde a una decisión del §1.

| # | Pregunta | Decide | Estado |
|---|---|---|---|
| A | **Driver.** Qué crates hablan ClickHouse (`clickhouse` oficial por HTTP, `klickhouse`/`clickhouse-rs` nativo), qué formato de lectura dan (RowBinary, Arrow, Native), TLS, cancelación, compresión, madurez | el transporte | medido, §3.1 |
| B | **Tipos.** Una tabla sonda con todos: `Nullable`, `LowCardinality`, `UInt64`, `Int128/256`, `Decimal(P,S)`, `DateTime64`, `Enum`, `UUID`, `IPv4/6`, `Array`, `Tuple`, `Map`, `Nested`, `Variant/JSON`, `FixedString`, geo | el mapa a tipos de OOS; qué es `Opaque`; la urgencia de `Binary` y `Map` (0075 §12) | medido, §3.1 |
| C | **Catálogo.** `system.columns` / `system.tables` / `system.parts`: clave de ordenación, de partición, motor, filas estimadas, vistas y vistas materializadas, `Distributed` | descubrimiento y cursores | medido, §3.2 |
| D | **Lectura masiva.** 10⁷–10⁸ filas: rendimiento por formato, memoria, límites del servidor (`max_execution_time`, `max_memory_usage`), partición por rangos de la clave de ordenación para paralelizar | el corte de la lectura | medido, §3.5 |
| E | **Consistencia.** Lectura durante inserciones y mutaciones; `ReplacingMergeTree`, `CollapsingMergeTree`, `FINAL`; qué es un snapshot aquí | el testigo y la garantía que se declara | medido, §3.3 |
| F | **Incremental.** Cursor sobre la clave de ordenación o una columna de fecha; borrados (`ALTER ... DELETE`, lightweight delete, `is_deleted`) | `cambia: none` o no | medido, §3.4 |
| G | **Parámetros y SQL.** Atado tipado (`{p:Type}`), traducción desde el SQL de ORE, qué funciones se empujan | el traductor | medido, §3.6 |
| H | **Seguridad y errores.** Usuario de sólo lectura, cuotas, TLS, contraseña, códigos de error, cancelación (`KILL QUERY`) | verbos y errores | medido, §3.7 |
| I | **Versiones y despliegues.** 24.8, 25.8, latest, Cloud, Altinity, chDB; formatos e interfaces que cambian entre ellas | el rango que se sirve | medido, §3.8 |

### 3.1 A + B medidos (2026-10-27): driver y tipos

Sonda en `scratchpad/chprobe` (fuera del repo), contra 25.8.33.6, sobre `ore.tipos` (**60 columnas**, 3 filas:
completa, casi vacía, extremos) y `ore.grande` (10⁷ filas, 6 columnas).

**Tres vías de lectura probadas**

| Vía | Crate | Tipos que lee (de 60) | 10⁷ filas |
|---|---|---|---|
| **HTTP + `ArrowStream`** | `clickhouse` 0.15.3 + `arrow` | 52 bien o degradados, **8 los rechaza el servidor** | 229 MB, 1,56 s de descarga, 2,38 s con decodificar |
| **Nativo + Arrow** | `clickhouse-arrow` 0.2.1 | 48 bien; **panic** en `Map` y en los geo (`not implemented`); falla `Variant`, `Dynamic`, `Time`, `Time64`, `BFloat16`, `SimpleAggregateFunction` y `AggregateFunction` | **5,18 s**, 618 lotes |
| **HTTP + `RowBinaryWithNamesAndTypes`** | `clickhouse` 0.15.3 + `clickhouse-types` 0.1.4 | **58 de 60** con cabecera parseada; el servidor las acepta todas | 432 MB, 1,72 s de descarga (sin decodificar) |

- **`clickhouse-rs` (0.1.21) se descarta**: sin mantenimiento desde 2023.
- **`clickhouse-arrow` se descarta**: es el más lento, entra en pánico con `Map` y los geo, y arrastra **arrow 57**
  (ORE usa el 58; dos versiones en el árbol). Además devuelve `String` como `Binary` y pierde los nombres de las tuplas.
- **El servidor no sabe dar Arrow** para `UUID`, `JSON`, `Variant`, `Dynamic`, `Time`, `Time64`, `BFloat16`
  y `AggregateFunction`. Y degrada: `Date` → `UInt16`, `DateTime` → `UInt32` (**sin zona**), `Enum` → entero
  (**sin los nombres**), `IPv4` → `UInt32`, `Int128/256` y `UInt128/256` → `FixedSizeBinary`.
- **RowBinary conserva el tipo exacto** (el nombre del tipo viaja en la cabecera) y es lo único que cubre todo.
  Cuesta ~2× en bytes (432 MB frente a 229 MB, sin compresión; con `compress=1` se acerca).

**Tabla de tipos (RowBinary)**: casi todo es de ancho fijo y trivial. Lo que no es un entero o un decimal:

| Tipo de ClickHouse | En RowBinary | Hacia OOS (propuesta, sin decidir) |
|---|---|---|
| `Int8…Int64`, `UInt8…UInt64`, `Float32/64`, `Bool` | ancho fijo | tipos propios; **`UInt64` no cabe en `Int64`** |
| `Int128/256`, `UInt128/256` | 16/32 bytes LE | decimal de 0 escala o `Opaque` (decidir) |
| `Decimal(P,S)`, P ≤ 76 | entero de 4/8/16/32 bytes | `Decimal`; **P > 38 supera a Arrow `Decimal128`** |
| `String`, `FixedString(N)` | longitud LEB128 + bytes | `String` (no hay garantía de UTF-8) |
| `Date`, `Date32`, `DateTime(tz)`, `DateTime64(p,tz)` | enteros desde 1970 | marca de tiempo con zona |
| `Time`, `Time64(p)` | entero con signo, **hasta 999:59:59** | no cabe en un `Time` de 24 h: decidir |
| `Enum8/16` | entero; **nombres sólo en el tipo** | texto (con los nombres del tipo) |
| `UUID`, `IPv4`, `IPv6` | 16/4/16 bytes | `Uuid`, texto |
| `Nullable(T)` | 1 byte + valor | nulo |
| `LowCardinality(T)` | como `T` en RowBinary | como `T` |
| `Array`, `Tuple` (con y sin nombres), `Map(K,V)` | recursivos | `List`, `Struct`, **`Map`** (0075 §12) |
| `Nested` | **dos columnas `Array` paralelas** (`nest.k`, `nest.v`) | listas paralelas |
| `JSON`, `Variant`, `Dynamic` | codificación propia (JSON: `output_format_binary_write_json_as_string`) | texto JSON / `Opaque` |
| `Point`, `Ring`, `Polygon`, … | tuplas y arrays de `Float64` | geometría (ver 0066/0067) |
| `AggregateFunction(f, T)` | estado opaco del agregado | `Opaque`, o rechazar |

**Lo que dice esto sobre el diseño:** nada todavía. Esta sección mide **qué entrega cada vía**; qué vía se usa y cómo se lee
es de §3.10 (X1), que contrasta estas cifras con la práctica de ClickHouse. Lo que sí se sostiene: la crate `clickhouse-types` no parsea
tuplas con nombre ni `AggregateFunction`, y `clickhouse-arrow` arrastra arrow 57 (ORE usa el 58).
- **Medido después** (§3.5–§3.8): decodificar RowBinary es barato (1,08 s para 10⁷ filas), y TLS, cancelación y versiones están cubiertos.

### 3.2 C medido (2026-10-27): el catálogo

Tablas de prueba en `ore.*`: una por motor (`MergeTree` particionada con TTL y columnas `DEFAULT`/`MATERIALIZED`/`ALIAS`/`EPHEMERAL`,
`ReplacingMergeTree`, `CollapsingMergeTree`, `SummingMergeTree`, `AggregatingMergeTree`, `Memory`, `Log`), una vista, una vista
materializada con su destino, un diccionario, una tabla con índice de salto y proyección.

- **`system.tables` lo dice casi todo**: `engine`, `sorting_key`, `primary_key` (puede ser **prefijo** de la clave de
  ordenación: `dev` frente a `dev, ts`), `partition_key`, `sampling_key`, `total_rows`, `total_bytes`, `comment`. Las vistas,
  las vistas materializadas y los diccionarios traen `total_rows = NULL`; `Memory` y `Log` dan 0 aunque tengan filas.
- **`system.columns`** añade `default_kind` (`DEFAULT`, `MATERIALIZED`, `ALIAS`, `EPHEMERAL`) y `is_in_{partition,sorting,primary}_key`.
- **`SELECT *` no devuelve** las columnas `MATERIALIZED` ni `ALIAS`, y las `EPHEMERAL` no se pueden leer: el conector
  **debe nombrar las columnas** que lee, no usar `*`.
- **Lo que se ve depende de los permisos.** Con `GRANT SELECT` sobre tres tablas, `system.tables` y `system.columns`
  muestran **sólo esas tres** (y `system.databases` sólo `ore`): no hay forma de «descubrir» lo que no se puede leer.
- **`system.parts` y `system.processes` piden su propio grant** (`SELECT ON system.parts`). Un lector sin él no ve las
  *parts*: la estimación de filas sale de `system.tables.total_rows`, lo demás es opcional.
- **El perfil `readonly` importa.** Con `readonly=1` el servidor **prohíbe cambiar cualquier setting por consulta**
  (`Cannot modify 'max_threads' setting in readonly mode`), también `output_format_binary_write_json_as_string`.
  Con `readonly=2` (o sólo con `GRANT SELECT`, sin `readonly`) sí se puede, y escribir da `ACCESS_DENIED`.
  El conector no puede depender de settings por consulta y **debe decirle al usuario que use `readonly=2`**, o
  llevar su configuración en el perfil del usuario.

### 3.3 E medido (2026-10-27): consistencia

- **Cada consulta ve una instantánea, aunque tarde.** Un lector lento (~6 s, 10⁶ filas) vio exactamente el estado del
  inicio mientras entraban 1 000 filas, se actualizaban 500 000 (`ALTER … UPDATE`) y se borraban 50 000 (`ALTER … DELETE`):
  `count = 1 000 000`, `sum = 1 000 000`. Las *parts* son inmutables y la consulta se queda con las que tenía.
- **Entre consultas no hay instantánea.** No hay transacciones (requieren Keeper). Fijar un conjunto de *parts* con
  `WHERE _part IN (...)` se rompe en cuanto hay **una** mutación o una fusión: `all_1_1_0` pasa a `all_1_1_0_3` y la
  consulta devuelve 0 filas (`old_parts_lifetime` = 480 s). Una lectura paralela por rangos de la clave **no** es
  consistente entre rangos; sólo lo es cada rango por separado.
- **Duplicados.** `ReplacingMergeTree(ver, del)` con 3 inserciones (3 M de claves; 1 M con `ver=2`; 100 k con `del=1`):

  | | filas |
  |---|---|
  | `count()` (sin `FINAL`) | **4 100 000** (duplicados y borradas incluidas) |
  | `count()` con `FINAL` | **2 900 000** (lo correcto) |
  | `uniqExact(k)` | 3 000 000 (cuenta también las borradas) |
  | tras `OPTIMIZE … FINAL`, `count()` sin `FINAL` | 3 000 000: **las filas `del=1` siguen físicamente ahí** |

  `FINAL` cuesta ~2× en lectura (0,035 s → 0,069 s, 3,1 M filas). **La corrección es del origen, no del conector**: hace falta
  `FINAL` en `Replacing`/`Collapsing` para que la lectura signifique lo que el usuario cree.
- **`DELETE` ligero** no deja rastro en los datos (`_row_exists` siempre 1 al leer). Lo único visible es el comando en
  `system.mutations` (que pide grant).

### 3.4 F medido (2026-10-27): el cursor incremental

Filas leídas por el servidor (`X-ClickHouse-Summary`, `read_rows`), sobre 10⁷ filas:

| Cursor | Filas leídas |
|---|---|
| `id > 9 900 000`, `id` es la clave de ordenación | **8 193** (13 de 1 222 gránulos, búsqueda binaria) |
| `ts > …`, `ts` **no** está en la clave ni en la partición | **10 000 000** (escaneo completo) |
| `id > …` con `ORDER BY (grp, id)` (el cursor no es prefijo) | **10 000 000** |
| `ts > …` con `PARTITION BY toDate(ts)` | **1 607 962**: la columna de la clave de partición tiene un min-max por *part* gratis (2 de 5 *parts*) |
| `n > …` con índice de salto `minmax` y datos dispersos | 10 000 000: el índice no ayuda si el valor no está agrupado |
| paginar con `LIMIT … OFFSET 5 M` | 6 083 037 (y crece con el desplazamiento) frente a 1 835 008 con `WHERE id > x ORDER BY id LIMIT` |

- **El incremental es barato sólo sobre el prefijo de la clave de ordenación**, y a medias sobre una columna de la clave de
  partición con datos que entran en orden. Sobre cualquier otra columna **es un escaneo completo**; el catálogo (C)
  lo sabe (`is_in_sorting_key`), así que el conector puede **ofrecer sólo cursores baratos** y avisar del resto.
- **Un cursor no ve los borrados** ni las actualizaciones de columnas que no avanzan. Con `ReplacingMergeTree` el cursor
  natural es `ver`, y exige `FINAL` en la lectura. Para borrados no hay nada comparable al CDC: `cambia: none` (§5).

### 3.5 D medido (2026-10-27): lectura masiva

`ore.grande`, 10⁷ filas × 6 columnas (`UInt64, UInt32, Float64, String, DateTime64(3), Nullable(Int32)`), HTTP + RowBinary, con un
decodificador **propio** (≈40 líneas: filas de ancho fijo, `String` con longitud LEB128, `Nullable` con un byte). Máquina de 16 CPU,
Docker Desktop en bucle local; las cifras sirven para comparar entre sí, no como rendimiento absoluto.

| Lectura | Tiempo | Filas/s |
|---|---|---|
| 1 flujo | 1,08 s | 9,3 M |
| **2 rangos de `id` en paralelo** | **0,69 s** | 14,4 M |
| 4 rangos | 0,76 s | 13,2 M |
| 8 rangos | 0,78 s | 12,8 M |
| 1 flujo con `lz4` | 0,89 s | 11,2 M |

- **Decodificar RowBinary es barato** (descarga y decodificación juntas, 1,08 s; Arrow costaba 2,38 s) y **es correcto**: la suma de
  `id` y los 1 428 572 nulos coinciden con los del servidor.
- **Paralelizar por rangos de la clave gana poco** (1,5×) y se satura con 2. Cómo partir (o no) una lectura se decide en §3.10.
- **Memoria del servidor por lectura en flujo: 54–62 MiB**, independiente del tamaño de la tabla (`query_log`).
- **Sin límites por defecto** (`max_execution_time`, `max_memory_usage`, `max_rows_to_read`, `max_result_*` = 0), pero un perfil
  de usuario puede ponerlos: el conector los trata como errores (§3.7).
- No se separó el efecto de `lz4` en bytes del cable (la crate descomprime antes de contar).

### 3.6 G medido (2026-10-27): parámetros

Parámetros **del servidor** (`{nombre:Tipo}` + `param_nombre=valor` en la URL; en la crate, `.param()`), no sustitución en el cliente.

- **Valores exactos**: `UInt64` máximo, `Int128`, `Decimal(38,10)`, `DateTime64(3,'Europe/Madrid')`, `UUID`, `Bool`, `IPv4`,
  `Float64` con `nan`/`-inf`, `Array(Int32)`, `Array(String)`, `Tuple`, `Map`, `Nullable(Int32)` con `\N`, y `IN {x:Array(UInt64)}`.
- **Cadenas peligrosas viajan intactas**: `' OR 1=1 --`, `\`, tabulaciones y saltos de línea, comillas, `{x:String}`, `\N`, `NULL`,
  `1; DROP TABLE …`, unicode con emoji, **`\0`** y la cadena vacía. La crate (`.param()`) las devolvió idénticas en **14 de 14**.
  El valor en la URL va **escapado como TSV** (`\\`, `\t`, `\n`); la crate lo hace sola.
- **`Identifier` es seguro** (`{t:Identifier}` entrecomilla; un `;` dentro queda en el nombre) pero **`db.tabla` es un solo
  identificador**: se pasan **dos** (`{db:Identifier}.{t:Identifier}`).
- **El cursor parametrizado usa el índice**: `WHERE id > {c:UInt64}` lee 8 193 filas, como el literal. `LIMIT {n:UInt32}` funciona.
- **Errores claros**: `BAD_QUERY_PARAMETER` (457), `UNKNOWN_QUERY_PARAMETER` (456), `UNKNOWN_TYPE` (50).
- **Trampa: los enteros desbordan en silencio.** `{x:UInt8}` con `300` devuelve **44**, sin error, en las tres versiones.
  **El conector debe comprobar el rango de los enteros antes de atar.**
- Los mismos valores funcionan igual en 24.8, 25.3 y 25.8.

### 3.7 H medido (2026-10-27): errores, cancelación, límites, TLS

**Errores antes de empezar** (llegan con estado HTTP y `X-ClickHouse-Exception-Code`, limpios):

| Caso | HTTP | Código |
|---|---|---|
| tabla inexistente / columna inexistente | 404 | 60 / 47 |
| sintaxis | 400 | 62 |
| login erróneo | 403 | 516 |
| sin permiso (`SELECT`, `INSERT`, `system.*`) | 500 | 497 |
| parámetro mal formado | 500 | 457 |
| `max_execution_time` superado (sin emitir aún) | 408 | 159 |
| `max_rows_to_read` / `max_bytes_to_read` | 500 | 158 / 307 |
| `max_result_rows` con `throw` | 500 | 396 |
| cuota agotada / demasiadas consultas del usuario | 500 | 201 / 202 |
| memoria (`max_memory_usage`) | 500 | 241 |

**Un error a mitad de flujo llega como `HTTP 200`** (la cabecera ya salió). Lo medido con un `throwIf` en la fila 5 000 000:

| Versión | Qué ve el cliente |
|---|---|
| **25.8** | las filas hasta el fallo, `__exception__\r\n<mensaje>` y la **conexión cortada** (curl: error 18, *chunked* sin cierre) |
| **25.3** | igual (curl: error 18) |
| **24.8** | las filas y el mensaje **pegado sin marca**; el *chunked* **termina limpio** (curl: exit 0) |

- **La crate `clickhouse` detectó el error en las tres versiones** y lo devolvió como `Err` con el mensaje del servidor. En 24.8, donde el HTTP
  no lo delata, lo hace mirando el texto del final: es una heurística de la crate, no del protocolo.
- **El mismo fallo en `ArrowStream` deja el flujo sin su marcador de fin** (también detectable).
- `wait_end_of_query=1` convierte el fallo en un `500` limpio, pero el servidor **bufferiza** la respuesta entera: no vale para 10⁷ filas.
- **Un límite con `*_overflow_mode=break` trunca en silencio.** Con `max_result_rows=100` y `result_overflow_mode=break`, un usuario recibió
  **65 409 de 10 000 000 filas con `HTTP 200` y sin ningún aviso**. Si el perfil del usuario lo trae, la lectura sale incompleta.
  Es detectable al conectar: **`system.settings WHERE changed` lo ve cualquier lector** (`ore_brk` → `max_result_rows=100,
  result_overflow_mode=break`). Y con `readonly=2` se puede forzar `result_overflow_mode=throw` por consulta (probado: 396).
  Con `readonly=1` no se puede cambiar nada: **hay que negarse a leer** si el perfil trae un `break`.
- `max_execution_time` con la consulta ya emitiendo da también `200` y el mensaje al final (código 159).

**Cancelación**

| Situación | Qué pasa al cerrar el cliente |
|---|---|
| la consulta **emite** sin parar | cancelada en **menos de 1 s** (falla la escritura) |
| una consulta **lenta que emite 1 fila/s** (ajuste por defecto) | **sigue viva** a los 2 s; murió antes de 10 s, al fallar una escritura |
| lo mismo con `cancel_http_readonly_queries_on_client_close=1` | **cancelada a los 2 s** |
| una consulta que **no emite nada** durante minutos (agregación pesada) | **sin medir**: sin escritura que falle, el único mecanismo sería el ajuste anterior |
| `KILL QUERY` por `query_id` | funciona con el administrador (el cliente ve el flujo cortado, error 18) |
| `KILL QUERY` por un lector con sólo `GRANT SELECT` | **`ACCESS_DENIED`**: pide `SELECT` sobre `system.processes` |

- Cancelar = **cerrar la conexión**, con `cancel_http_readonly_queries_on_client_close=1` (no válido con `readonly=1`). `KILL QUERY` es un
  extra que necesita un permiso que el lector no tiene.
- **Un consumidor parado corta el flujo.** Un cliente que leyó 17 bytes y paró 45 s recibió **sólo 31 MB de ~432** y el servidor cerró
  (`http_send_timeout` = 30 s). Poner `http_send_timeout=300` y `send_timeout=300` **en la consulta no lo evita** (cortó con 18–24 MB): son del servidor, no
  de la consulta. Quien lee tiene que vaciar el socket sin pausas largas (§3.10).

**TLS** (CA y certificado propios, `https_port 8443`, TLS ≥ 1.2):

- La crate acepta un cliente `hyper` propio (`with_http_client`): un conector **`hyper-rustls` con la CA que traiga la credencial** conectó a
  `localhost` y a `127.0.0.1` (SAN con IP), devolviendo `25.8.33.6`.
- **Cierra en falso**: sin la CA da `InvalidCertificate(UnknownIssuer)`; con un nombre que no está en el SAN, `NotValidForName`.
  HTTP plano contra el puerto TLS: respuesta vacía. (El servidor se configuró con `disableProtocols` para SSLv2/3 y TLS 1.0/1.1; no se
  probó aparte con un cliente que sólo hable TLS 1.1.)
- Sin `with_http_client`, la crate sólo confía en `webpki-roots` o en las raíces del sistema: una CA privada exige el conector propio.
- **Autenticación**: usuario y contraseña (HTTP Basic); la contraseña viaja en claro sin TLS. No se midió mTLS ni JWT.
- **Para el conector esto no cuenta:** ORE no ofrece una CA propia por fichero (el conector corre en un Job sin tus ficheros), así que el TLS es el del
  sistema (`native-tls`) y no hace falta el conector `hyper-rustls` de arriba (ver el paso 1 de §6).

### 3.8 I medido (2026-10-27): versiones

Misma sonda de tipos (RowBinary por HTTP) contra tres servidores:

| | 24.8 | 25.3 | 25.8 |
|---|---|---|---|
| columnas de la sonda creadas (de 60) | 54 (sin `bf16`, `t`, `t64`, `j`, `v`, `dy`) | 57 (sin `bf16`, `t`, `t64`) | 60 |
| cabecera parseada | 52 | 55 | **58** |
| los que fallan | `tn` y `agg` | `tn` y `agg` | `tn` y `agg` |
| `Time`, `Time64` | no existen (el setting `enable_time_time64_type` da 115) | no existen (ídem) | sí |
| `BFloat16` | no probado | no probado | sí |
| `JSON`, `Variant`, `Dynamic` | no probados | sí | sí |
| `output_format_binary_write_json_as_string` | **no existe** | sí | sí |
| error a mitad de flujo | silencioso en el HTTP | **conexión cortada** | **conexión cortada** |
| parámetros, `UInt8`←300 | igual | igual | igual |

- **Los dos fallos de la cabecera son los mismos en las tres versiones**: son de la crate `clickhouse-types` (tuplas con nombre y
  `AggregateFunction`), no del servidor.
- **Un setting que el servidor no conoce da error** (código 115), también uno inventado: **el conector no puede mandar settings por
  defecto**; sólo los que la versión del servidor declare (se consulta `system.settings`).
- En 24.8 y 25.3 se quitaron de la sonda las columnas cuyos tipos se suponían inexistentes; sólo `Time`/`Time64` se confirmó (el setting no existe).
- Sin medir: ClickHouse Cloud, Altinity, chDB, clústeres con réplicas, `Distributed`, mTLS y JWT.

### 3.9 X0 cerrado: las preguntas y sus respuestas (2026-10-27)

| # | Pregunta | Respuesta medida | Lo que deja abierto |
|---|---|---|---|
| A | driver | HTTP + RowBinary con `clickhouse` 0.15.3; el nativo y Arrow se descartan (§3.1) | qué formato leer (§3.10) |
| B | tipos | 58/60 con la crate; el resto con un parseo propio de la cadena del tipo (§3.1) | tabla de tipos: `UInt64`, `Int128/256`, `Decimal` > 38, `Time` 999 h, `Map`, `Nested` |
| C | catálogo | `system.tables`/`columns`, filtrado por permisos; `system.parts` aparte; `SELECT *` omite columnas (§3.2) | nombrar las columnas; `readonly=2`; sin `system.parts` obligatorio |
| D | lectura masiva | 9–14 M filas/s decodificadas, 54–62 MiB por flujo, el paralelismo sólo ayuda 1,5× (§3.5) | flujo único o por partes: se decide con la práctica de ClickHouse (§3.10) |
| E | consistencia | instantánea por consulta; ninguna entre consultas; `FINAL` obligatorio en `Replacing`/`Collapsing` (§3.3) | qué consistencia se declara; si `FINAL` es por defecto o una opción del usuario |
| F | incremental | barato sólo sobre el prefijo de la clave de ordenación; sin borrados (§3.4) | ofrecer sólo cursores baratos; `cambia: none` |
| G | parámetros | del servidor, seguros; **los enteros desbordan en silencio** (§3.6) | comprobar el rango antes de atar; dos `Identifier` |
| H | seguridad y errores | errores limpios antes de empezar; **a mitad de flujo, `200` + mensaje**; `break` trunca en silencio; cancelar cerrando la conexión; TLS con CA propia por `hyper-rustls` (§3.7) | detectar el corte; rechazar perfiles con `break`; no mandar settings por defecto |
| I | versiones | 24.8, 25.3 y 25.8 sirven; `Time`/`Time64` no existen en 25.3; el corte de flujo sólo es detectable por el HTTP desde 25.3 (§3.8) | el suelo de versión |

### 3.10 X1 (2026-10-27): qué hace ClickHouse y sus propios clientes

X0 midió **comportamiento**; no decía si lo que se medía era la manera correcta de leer ClickHouse. X1 lo contrasta con la documentación
oficial (leída el 2026-10-27), con cómo leen los clientes y conectores de referencia, y con tres pruebas que faltaban.

**Cómo leen los de referencia** (lo que dicen sus README y documentación; no se leyó su código fuente):

| Cliente / conector | Cómo lee | Notas |
|---|---|---|
| **`clickhouse-rs`** (el oficial de Rust) | HTTP, **`RowBinaryWithNamesAndTypes`** por defecto | validación del esquema por defecto desde 0.14 (5–10 % de coste); `Native` sobre TCP **previsto**, sin Arrow; avisa de que *«los cursores pueden devolver un error aun después de dar filas»* y recomienda `wait_end_of_query=1`; para certificados propios, `native-tls` o `rustls-tls-native-roots` |
| **`clickhouse-java` client-v2** | **`RowBinaryWithNamesAndTypes`** por defecto; también `Native` | «mejor soporte para RowBinary y Native» |
| **`clickhouse-connect`** (Python) | **`Native`** sobre HTTP, en bloques (un bloque cada vez); `ArrowStream` aparte | códec Rust experimental, sólo para `Native` |
| **Conector de Spark** | HTTP (sobre el JDBC oficial); formato de lectura `json` (por defecto) o `binary`; `lz4` por defecto | **reparte la lectura por `_partition_id`** (una partición por tarea), no por rangos de la clave; en `Distributed` lee la tabla local; **no documenta consistencia** |
| **Driver ADBC** (oficial, v0.1.0) | HTTP, resultado en lotes de Arrow | «en desarrollo activo»; sin `JSON`; sólo `Append` al escribir |
| **Servidor Arrow Flight** | gRPC | **25.8: soporte inicial; 26.4: Flight SQL**; la documentación no dice «experimental» ni «GA» |
| **Airbyte** | JDBC (ya visto en §2) | |

**Lo que dice la documentación oficial y X0 confirma o matiza**

- **Un error a mitad de flujo**: *«una respuesta 200 no garantiza que la consulta haya ido bien»* y *«la única manera de cazarlos todos es analizar el
  cuerpo antes de interpretarlo»*. `wait_end_of_query=1` ayuda, pero el resultado tiene que caber en `http_response_buffer_size`.
- **Marco del error en el cuerpo, por versión (medido):**

  | Versión | Cabecera | Cuerpo |
  |---|---|---|
  | 24.8 | — | el mensaje, sin marca |
  | 25.3, 25.8 | — | `__exception__\r\n<mensaje>`, y el *chunked* **sin cerrar** (curl: error 18) |
  | **26.4** | **`X-ClickHouse-Exception-Tag: <16 caracteres>`** | `__exception__\r\n<ETIQUETA>\r\n<mensaje>\r\n<longitud> <ETIQUETA>\r\n__exception__\r\n`, y el *chunked* **sin cerrar** |

  Con 26.4 un lector que conozca la etiqueta (la cabecera sale antes que el cuerpo) detecta el error **sin ambigüedad posible**.
- **Cerrar la conexión**: la documentación dice que *«las peticiones en curso no se detienen solas si se pierde la conexión»*. Lo medido en X0 lo
  matiza: una consulta que **emite** se cancela en < 1 s porque falla la escritura; una que no escribe, no.
- **`FINAL`**: la documentación lo describe como necesario *«a veces»* para resultados exactos, cuesta CPU y memoria, se puede activar por consulta
  o por perfil (`final = 1`), y propone, si no hace falta, escribir consultas que toleren los duplicados con agregación. No dice cuándo conviene.

**Cuatro pruebas que faltaban**

1. **`Native` (columnar) frente a `RowBinary` (filas)** sobre las mismas 10⁷ filas, con un decodificador correcto de cada uno (las sumas y los
   nulos coinciden con los del servidor):

   | | 1 flujo | 2 flujos | 4 flujos |
   |---|---|---|---|
   | `Native` | 1,03 s (9,7 M/s) | 0,84 s (11,9 M/s) | 0,78 s (12,8 M/s) |
   | `RowBinary` | 0,85–1,07 s (9,4–11,8 M/s) | 0,69–0,93 s (10,7–14,4 M/s) | — |

   **No se distinguen.** Con `max_threads` = 1, 4 o 16 el tiempo es el mismo (~1 s): **el cuello de botella no es el servidor sino una sola
   conexión** (~430 MB/s) y la decodificación en un hilo.

   **Construyendo las columnas** (lotes de 65 536 filas, como un `RecordBatch`; cada decodificador escribe en `Vec`s por columna: arreglos
   contiguos para las de ancho fijo, desplazamientos + bytes para `String`, validez para `Nullable`), dos pasadas cada uno:

   | | 1 flujo | 2 flujos |
   |---|---|---|
   | `RowBinary` → columnas | 0,88 s · 1,02 s (11,4 · 9,8 M/s) | 0,78 s · 0,80 s |
   | `Native` → columnas | 1,41 s · 1,06 s (7,1 · 9,5 M/s) | 1,19 s · 0,82 s |

   **Tampoco aquí gana `Native`**: la primera pasada de cada uno es más lenta (arranque en frío) y la segunda empata. Mi decodificador de
   `Native` copia con iteradores, no con una copia directa de memoria; con una copia directa podría ganar algo, **sin medir**. Y **la documentación
   advierte de que algunas codificaciones de `Native` dependen de la versión** (`JSON`, `Variant`, `Dynamic`), y que `Nullable(LowCardinality(T))` da error.
2. **Arrow Flight SQL en 26.4** (ADBC de Python; 26.4 sí arranca en esta CPU, `latest` no):
   - **Tipos: 58 de 60**, más fieles que HTTP+Arrow: `UUID` como extensión `arrow.uuid`, `Map` nativo, `Date` como `date32`, `Variant` como unión.
     Pero `DateTime` sale como `uint32` (sin zona), `Enum` como entero, `Time` y `BFloat16` como `binary`, y **`JSON` y `Dynamic` fallan**.
   - **Lento**: 2,3–3,0 M filas/s (frente a ~10 M por HTTP), con cliente de Python.
   - **El servidor ejecuta la consulta entera antes de enviar nada** (`GetFlightInfo`): `execute()` tardó 1,36 s y el primer lote llegó a los **6,2 s**
     (por HTTP, el primer byte a los 0,3 s). Y **retiene el resultado en memoria**: **483 MiB** para un `SELECT *` de 10⁷ filas frente a 54–62 MiB por HTTP.
     Una consulta sin fin (`system.numbers`) dio `MEMORY_LIMIT_EXCEEDED` (5,21 GiB) *en el propio `GetFlightInfo`*: **no hay lectura en flujo de
     verdad** con este cliente. El error de `throwIf` también salió ahí, antes de dar un solo dato.
   - **No medido**: cancelación (no se puede con una consulta sin fin) y parámetros (el `execute(..., parameters=…)` de ADBC fue por la vía de
     ingesta y falló en el cliente, no en el servidor). `PollFlightInfo` (resultados por bloques) no lo usa este cliente.
3. **Leer por partición (`_partition_id`, como Spark) frente a un flujo único**, en `ore.gpart` (10⁷ filas, 16 particiones, a columnas, `RowBinary`):

   | | tiempo | filas/s |
   |---|---|---|
   | flujo único (dos pasadas) | 0,94 s · 0,86 s | 10,6 · 11,7 M |
   | por partición, concurrencia 1 | 1,17 s | 8,6 M |
   | por partición, concurrencia 2 | 0,82 s | 12,2 M |
   | por partición, concurrencia 4 | 0,86 s | 11,6 M |
   | por partición, concurrencia 8 | 0,91 s | 10,9 M |

   **El reparto por partición no mejora al flujo único** (en bucle local, una sola máquina): el servidor ya paraleliza dentro de una consulta, y
   16 consultas sólo añaden coste con la concurrencia 1. *Sin medir:* una red con latencia, donde varias conexiones sí pueden rendir más que una.
4. **Un consumidor parado**: poner `http_send_timeout=300` y `send_timeout=300` **en la consulta no evita el corte** (cortó con 18–24 MB recibidos);
   son tiempos del servidor. Quien lee tiene que **vaciar el socket sin pausas largas**.

**Lo que X1 permite afirmar** (y con qué fuerza):

| Afirmación | Fuerza |
|---|---|
| `RowBinaryWithNamesAndTypes` es corriente en los clientes oficiales (Rust y Java por defecto); `Native` también (Python, Java) | **documentada** |
| `Native` y `RowBinary` rinden igual **en esta máquina y con este esquema**, también construyendo columnas | **medida**, con la salvedad del decodificador de `Native` y de que es un bucle local |
| Un flujo único rinde lo mismo que repartir por partición o por rangos de la clave; el servidor ya paraleliza dentro de la consulta | **medida** (bucle local) |
| Un flujo único conserva la instantánea de la consulta; repartir la pierde (§3.3) | **medida** |
| Arrow Flight (26.4) no sirve para lectura masiva: materializa el resultado en el servidor | **medida**, con un cliente de Python |
| `Native` tiene codificaciones que dependen de la versión; `RowBinary` no se documenta así | **documentada** |
| No hay «mejor práctica» publicada que diga cuál de los formatos usar para un conector de lectura | **ausente**: la documentación no recomienda ninguno para lectura masiva |

**Lo que X1 no cierra** (no hay medida): una red con latencia (varias conexiones podrían rendir más que una), la compresión en el cable (`lz4`/`zstd`),
un `Native` con copia directa de memoria, y la cancelación y los parámetros por Flight. Ninguno de los cuatro cambia lo que sigue.

## 4. El laboratorio

Docker (28.5.1) en la máquina de desarrollo; usuario `ore`/`ore`. Cinco contenedores, todos con la versión **fijada**:

| Contenedor | Imagen | Puertos (HTTP / nativo) | Contenido |
|---|---|---|---|
| `ore-clickhouse` | 25.8.33.6 (LTS) | 8123 / 9000 | `ore.tipos` (60 columnas), `ore.grande` (10⁷ filas), una tabla por motor, usuarios `ore_g`, `ore_r1`, `ore_r2`, `ore_brk` |
| `ore-clickhouse-253` | 25.3.14.14 | 8125 / 9002 | `ore.tipos` (57 columnas) |
| `ore-clickhouse-24` | 24.8.14.39 | 8124 / 9001 | `ore.tipos` (54 columnas) |
| `ore-clickhouse-264` | 26.4.5.143 | 8126 / 9003, **Arrow Flight 9090** | `ore.tipos`, `ore.grande` (copiados con `remote()`), `arrowflight_port` activado |
| `ore-clickhouse-tls` | 25.8 | **8443 (HTTPS)** / 9440 | CA y certificado propios (`localhost`, `127.0.0.1`), TLS ≥ 1.2 |

```sh
docker run -d --name ore-clickhouse -p 8123:8123 -p 9000:9000   -e CLICKHOUSE_USER=ore -e CLICKHOUSE_PASSWORD=ore -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1   --ulimit nofile=262144:262144 clickhouse/clickhouse-server:25.8
```

- **`latest` (publicada hace ~2 días, medido el 2026-10-27) no arranca** en esta máquina: `Illegal instruction` en `entrypoint.sh`. La CPU
  es un Xeon E5-2689 (Sandy Bridge, sin AVX2) y esa compilación exige más. **Fijar siempre la versión.**
- Montar certificados con `-v` desde Windows: si el fichero no existe, Docker **crea un directorio** con ese nombre (y el servidor no arranca).
- Git Bash reescribe rutas y `-subj /CN=…`: usar `MSYS_NO_PATHCONV=1`.
- La sonda (`chprobe`, fuera del repo, en el scratchpad de la sesión) tiene los modos `tipos-http`, `tipos-native`, `tipos-rb`, `masivo`, `rb N`,
  `mitad`, `param` y `tls`; compila nativa en ~1,5 min.

## 5. Incremental y CDC

*(pendiente)*

## 6. La hoja de ruta

**Principio: el binario existe desde el primer día y el kit es la puerta.** `ore-read-clickhouse` habla el protocolo v2 desde el primer commit; cada paso
engrosa ese mismo binario y termina con el kit en verde para los casos que le tocan. No hay paso que sea «una librería probada con fixtures».

**Ciclo de trabajo, idéntico en todos los pasos:**

```text
editar  →  cargo build -p ore-read-clickhouse   (segundos, incremental)
        →  ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos N
```

`N` son los casos del kit (1 proyección · 2 cada operador · 3 operador no declarado · 4 limit y orderBy · 5 tipos · 6 tabla vacía · 7 flujo ·
8 timeoutMs · 9 sólo lectura · 10 cancelar · 11 servir · 12 secretos · 13 capacidades · 14 estimar). Un paso se da por hecho cuando **su comando
de prueba sale en verde**, no cuando «parece que funciona».

**El orden es el del riesgo, no el de los números de los casos.** Lo que X0 y X1 dejaron sin comprobar va primero, porque si falla cambia el diseño.

| Paso | Qué se construye | Lo prueba | Por qué aquí |
|---|---|---|---|
| **0 · hecho** | `ore-clickhouse` + `ore-read-clickhouse` compilan; `capacidades` contesta el protocolo v2 (`cambia: none`, sin operadores); `leer` y `conectar` rechazan con un error claro | `ore-read-clickhouse capacidades` · `cargo test -p ore-clickhouse` | el binario existe |
| **1 · hecho** · transporte y banco | `transporte`: HTTP en flujo con `ureq` 2 + `native-tls`, parámetros del servidor con el escapado TSV, detección del error a mitad de flujo **por versión** (etiqueta en 26.4, *chunked* roto en 25.x, texto del final en 24.8). Y **el banco `clickhouse` del kit**, que carga la semilla por ese mismo transporte (`CLICKHOUSE_KIT_URL`). El vaciado del socket sin pausas largas es de quien lee (paso 6) | `--casos 13` · 33 pruebas del transporte sin red · 6 contra 24.8, 25.3, 25.8 y 26.4 · el banco carga dos veces sin romper nada | era lo único que nadie había comprobado (¿expone `ureq` el *chunked* sin cerrar?): **sí lo expone** |
| **2 · conectar** · hecho | `conectar` sobre `Cliente::abrir` (la URL ya se analiza desde el paso 1). Al abrir comprueba: la **versión** (el suelo se fija en §1), el perfil (`readonly=1` no admite ningún setting por consulta: se avisa y no se mandan), **`*_overflow_mode=break`** en `system.settings WHERE changed` (se rechaza: trunca en silencio), y que los settings que se mandan los declara esa versión (uno desconocido da el error 115) | `--casos 12,3` (el 3 pasa hoy sin nada que probar; el 12 necesita `conectar`: un usuario inexistente sale como `credencial`) · pruebas con `ore_r1`, `ore_r2` y `ore_brk` del laboratorio | de ahí cuelga todo lo demás |
| **3 · leer lo mínimo** · hecho | el `Peticion` → SQL (dialecto `CLICKHOUSE` en `ore-sql`: identificadores con backtick, `{p:Tipo}`), el flujo del `Vigia` → RowBinary → Arrow para `Integer` y `String`. Las columnas se nombran siempre (`SELECT *` omite las `MATERIALIZED` y las `ALIAS`). **Cuando el `Vigia` da un error del servidor, la fila parcial que lo precede se descarta** | `--casos 6,1,7` (vacía, proyección, flujo de 10⁶ filas) | el primer dato de extremo a extremo |
| **4 · tipos** · hecho (sin compuestos) | parser del texto del tipo (con tuplas con nombre y `AggregateFunction`, que la crate `clickhouse-types` no parseaba) y decodificador RowBinary para los 9 tipos de la semilla; los de X0 como pruebas propias (`UInt64`, `Int128/256`, `Decimal` > 38, `Time`…). Lo que no cabe **falla con error claro y no se redondea** (§1 fija el mapa) | `--casos 5` | donde se toman las decisiones de tipos de §1 |
| **5 · filtros y orden** · hecho | los operadores uno a uno (se declara cada uno cuando pasa), `limit`, `orderBy`, y el atado con `Param` (ya escapa; **aquí se comprueba el rango de los enteros**, que el servidor desborda en silencio: `UInt8` ← 300 = 44). `FINAL` en `Replacing` y `Collapsing` según lo que se decida en §1 | `--casos 2,3,4` | el kit prueba lo declarado y rechaza lo demás |
| **6 · tiempo, escritura y cancelar** · hecho (el caso 10 sólo en Linux) | `timeoutMs` → `max_execution_time` (un error con el flujo ya empezado llega por el `Vigia`); sólo lectura; cancelar **cerrando la conexión**, con `cancel_http_readonly_queries_on_client_close=1` cuando el perfil lo permita (una consulta que no emite nada sigue viva sin él). Quien lee **vacía el socket sin pausas largas**: el servidor corta a un consumidor parado ~30 s | `--casos 8,9,10` | es donde X0 y X1 más enseñaron |
| **7 · servir y estimar** · hecho | sesión por credencial en `servir` (HTTP no tiene sesión: es el agente con su *pool*); `estimar` con `system.tables.total_rows` (las vistas y `Memory` no la dan) | `--casos 11,14` | |
| **8 · verbos propios** · hecho | `catalogo` (`system.tables` y `system.columns`, filtrados por los permisos del usuario: no se descubre lo que no se puede leer), `check`, `explorar`, `testigo`, `cambio`; la consistencia que se declare (§1) | pruebas propias | no son casos del kit |
| **9 · puerta completa** · pendiente | los 14 casos en verde; los peligros de X0/X1 como casos añadidos al banco (el entero que desborda, el `break`, el error a mitad de flujo en cada versión); el grupo `clickhouse` del CI con su contenedor y el usuario `kit` sin contraseña (cabecera de `bancos/clickhouse.rs`) | `ore-kit --banco clickhouse` · CI | |
| **10 · la consola** · pendiente | la hoja de conexión (URL con `tls=true` para una contraseña; usuario sin contraseña sólo en una red de confianza) | alta de extremo a extremo | al final, como en los demás |

### Paso 1 · hecho (2026-10-27): el transporte y el banco

`ore-clickhouse` tiene `url`, `transporte` (con el vigilante) y los parámetros; el kit tiene el banco `clickhouse`. **`ore-kit --banco clickhouse --casos 13` pasa**
contra 24.8, 25.3, 25.8 y 26.4, y la carga de la semilla es idempotente (0,6 s la segunda vez, con las 10⁶ filas de `grande` ya puestas).

**Cómo se prueba** (todo verde el 2026-10-27; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse --lib                        # 33 pruebas, sin red: el vigilante y el cliente contra un servidor falso local
CLICKHOUSE_LAB_URLS=clickhouse://kit@localhost:8124/kit,clickhouse://kit@localhost:8125/kit,clickhouse://kit@localhost:8123/kit,clickhouse://kit@localhost:8126/kit \
  cargo test -p ore-clickhouse --test laboratorio         # 6 pruebas × 4 servidores; sin la variable se saltan solas (así corre el CI)
CLICKHOUSE_KIT_URL=clickhouse://kit@localhost:8123/kit \
  ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos 13
```

**Lo que construir enseñó, y que la medición no podía decir:**

1. **`ureq` 2 devuelve un *chunked* cortado como error** (`UnexpectedEof`), no como un fin normal: era la duda que decidía si el transporte síncrono
   valía. Probado con un servidor falso que cierra sin el `0\r\n\r\n`. Con eso, un corte sin marca llega al vigilante como un corte, y con marca,
   como el error del servidor.
2. **Un fixture inventado esconde el defecto que un servidor de verdad enseña.** El mensaje de 26.4 acaba en `\n`, no en `\r\n`, antes de
   `<longitud> <ETIQUETA>`; mi fixture usaba `\r\n`, las 33 pruebas pasaban y el marco se colaba en el mensaje. Lo cazó la prueba contra el
   26.4 real. El fixture es ahora el byte capturado del servidor.
3. **Los datos de antes de un error pueden terminar a mitad de una fila.** El servidor vuelca lo que tenía en el búfer cuando falla
   (34 732 179 bytes no son múltiplo de los 9 de la fila). **El decodificador tiene que descartar la fila parcial cuando llega el error.**
4. **Los parámetros se escapan en el transporte, y es un tipo** (`Param::Texto` / `Param::Nulo`): un valor sin escapar no da error, da *otro
   valor* (`\N` perdía dos bytes). `ureq` escribe el espacio como `+`, que las cuatro versiones leen como espacio.
5. **La URL sigue la regla de ORE, no la de Oracle:** `clickhouse://usuario[:clave]@host[:8123][/BASE]?tls=true&timeoutMs=…`. Un usuario **sin** contraseña
   se admite; una contraseña por HTTP en claro, no (la misma regla que Phoenix). Y **una CA propia por fichero no se ofrece** (el conector corre en
   un Job sin tus ficheros): el TLS es el del sistema. Esto **deja sin efecto** lo que §3.7 dice de montar un conector `hyper-rustls` con una CA privada:
   era una necesidad de la crate `clickhouse`, no del conector.
6. **El laboratorio necesita un usuario sin contraseña** (`kit`, con `GRANT ALL ON kit.*`, `SELECT ON system.processes` y `KILL QUERY`), por la regla
   anterior. Está en los cuatro servidores (§4) y en la cabecera de `bancos/clickhouse.rs`.

**El mapa del kit hoy** (25.8): pasan el 13, y el 2, el 3 y el 4 sin nada que probar (el conector no declara operadores, `limit` ni `orderBy`); el 9 y el
14 no aplican; **el 1, 5, 6, 7, 8, 10, 11 y 12 fallan con «todavía no sabe conectar»**, que es exactamente el paso 2.

**Entorno.** El laboratorio (§4) tiene los servidores con la versión fijada y el usuario `kit` sin contraseña. Desde el paso 1 se compila con la toolchain MSVC (`. C:\ORE\scripts\dev-windows.ps1`, que vive en `C:\ORE` y no en este worktree): el esqueleto del paso 0 compilaba con la de GNU porque aún no arrastraba nada del sistema. Docker Desktop no arranca los contenedores solo: tras un reinicio hay que levantarlos (`docker start ore-clickhouse ore-clickhouse-24 ore-clickhouse-253 ore-clickhouse-264`).

### Paso 2 · hecho (2026-10-28): conectar

`ClickHouse::conectar` abre el `Cliente` y hace **tres consultas** (la primera es también la prueba de la credencial: un usuario inexistente sale como `credencial`):
`SELECT version(), getSetting('readonly')`; los `*_overflow_mode` cambiados del perfil (`system.settings WHERE changed`); y cuáles de los ajustes que el conector
mandará (`max_execution_time`, `cancel_http_readonly_queries_on_client_close`) declara esa versión. `Sesion` lleva `cliente`, `version`, `readonly` y `ajustes`.

- **Suelo de versión: 24.8** (`lectura::SUELO`), la más antigua medida en X0; por debajo, `origen`. Una versión que no se entiende tampoco pasa. Decidido aquí; §1 lo recogerá.
- **`readonly=1`**: conecta y no manda ningún ajuste (`Sesion.ajustes` vacío). `readonly=2` los manda.
- **`*_overflow_mode = 'break'`** en el perfil: se rechaza con `operador`, nombrando el ajuste. Con él una consulta que pasa el límite acaba bien y con filas de menos.

**Cómo se prueba** (verde el 2026-10-28 en 24.8, 25.3, 25.8 y 26.4; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse --lib                          # 35 pruebas, sin red (la versión y el desborde se analizan aparte)
CLICKHOUSE_LAB_URLS=...  cargo test -p ore-clickhouse --test conectar   # 6 pruebas × 4 servidores
ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos 12,3,13
```

El laboratorio gana tres usuarios **sin contraseña** en los cuatro servidores: `ore_r1` (`readonly=1`), `ore_r2` (`readonly=2`) y `ore_brk` (`result_overflow_mode='break'`, `max_result_rows=100`), todos con `GRANT SELECT ON *.*`.

### Paso 3 · hecho (2026-10-28): leer lo mínimo

`ClickHouse::leer` manda ``SELECT `c1`, `c2` FROM `base`.`tabla` `` (las columnas siempre nombradas, en el orden de la proyección; el objeto es `base.tabla`
o `tabla` con la base de la URL; un nombre con comilla invertida, barra o control se rechaza con `operador`) en **`RowBinaryWithNamesAndTypes`** y escribe el flujo
Arrow por stdout, un lote cada 65 536 filas. `rowbinary.rs` analiza el texto del tipo y decodifica la celda.

**Decisión: los tipos salen de la cabecera de la propia respuesta, no del catálogo.** La cabecera trae nombres y tipos de *esa* consulta, también si no hay filas
(caso 6), así que no hay `system.columns` previo ni carrera entre preguntar y leer. La cabecera Arrow sale después de leerla: lo que falla antes
(tabla, columna, credencial) es un error limpio y no un flujo a medias.

**Se adelantó parte del paso 4**, porque los casos 6 y 7 piden la semilla entera y no sólo `Integer` y `String`: se leen `Int8…Int64`, `UInt8…UInt32`, `Float32/64`, `Bool`,
`String`, `Decimal(P≤38, S)`, `Date`, `Date32`, `DateTime`, `DateTime64(0…9)` (siempre como instante UTC), `Nullable` y `LowCardinality`. **Falla con error claro y código
`esquema`, nombrando el tipo:** `UInt64`, `Int128/256`, `Decimal` de más de 38 dígitos, `Time`, `Enum`, `Map`, `Array`, `Tuple`…; un `DateTime64` con más
precisión que un microsegundo se rechaza si el valor no es exacto, y un `String` que no es UTF-8 también. Nada se redondea.

**Todavía no (rechaza con `operador`):** filtros, `limit`, `orderBy` (paso 5), recorte por clave y rango incremental (§5), `timeoutMs` y cancelar con el flujo vivo (paso 6: hoy
el token sólo se mira entre lotes).

**El caso 5 falla, y es del paso 4:** `local` y `instante` son los dos `DateTime64(6, 'UTC')` en el servidor y el kit espera `local` como hora de pared (`FechaHora`). El
catálogo no puede distinguirlos: hace falta el **tipo declarado** de la petición (`tipos`) y que el banco lo mande (`completar`).

**Cómo se prueba** (verde el 2026-10-28; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse                              # 46 unitarias (tipos, celdas, cabecera cortada, error a mitad de fila) + 6 conectar + 6 laboratorio + 2 leer
CLICKHOUSE_LAB_URLS=...  cargo test -p ore-clickhouse     # leer × 4 versiones × (kit, ore_r1, ore_r2); tabla/columna inexistente
ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos 6,1,7    # en 24.8, 25.3, 25.8 y 26.4
```

Caso 7 (debug): 10⁶ filas en ~3 s, 47 MB, 16 lotes, primer byte a los ~0,45 s. **Mapa del kit completo en 25.8:** pasan 1, 2, 3, 4, 6, 7, 12 y 13; fallan el 5 (paso 4), el
8 y el 10 (paso 6) y el 11 (paso 7); el 9 y el 14 no aplican.

### Paso 4 · hecho (2026-10-28): tipos

**El caso 5 pasa** en 24.8, 25.3, 25.8 y 26.4 (los 9 tipos de la semilla con su tipo Arrow y sus valores exactos).

**Decisiones de §1 que este paso toma:**

| Tipo de ClickHouse | Sale como | Por qué |
|---|---|---|
| `UInt64` | `Decimal<20, 0>` (decimal128) | exacto: no cabe en un `int64` y redondear no es una opción |
| `Int128/256`, `UInt128/256`, `Decimal` > 38 dígitos | **error `esquema` con el nombre del tipo** | superan a `decimal128` (38 dígitos); sin un físico que los guarde exactos, se dice y no se corta |
| `Enum8/16` | texto con el **nombre** (sale del tipo) | los nombres sólo viajan en el tipo; un valor que el tipo no nombra es un error |
| `UUID`, `IPv4`, `IPv6`, `FixedString(N)` | texto | el relleno de ceros del `FixedString` se quita; que no sea UTF-8 es un error |
| `DateTime`, `DateTime('tz')`, `DateTime64(p[, 'tz'])` | instante UTC (microsegundos) | la zona sólo cambia cómo se escribe; con `p` > 6 sólo pasa si el valor es exacto |
| `LowCardinality(T)` | como `T` | en RowBinary no existe |

**El tipo declarado desempata, no manda.** `local` e `instante` son el mismo `DateTime64(6, 'UTC')` en el servidor. Con catálogo manda el catálogo
(ADR 0069 M3); lo declarado (`tipos` de la petición) sólo resuelve eso: un instante declarado `DateTime` sale como hora de reloj (`FechaHora`), con los mismos microsegundos.
El banco del kit manda ahora `tipos` (`completar`, como el de MongoDB).

**Fuera de este paso, y sigue fallando con error claro:** los compuestos (`Array`, `Tuple`, `Map`, `Nested`: piden `Forma` y `push_datos`), `Time/Time64` (hasta 999:59:59, no caben
en 24 h), `JSON/Variant/Dynamic`, `AggregateFunction` y geo. Entran cuando un cliente los pida (el mismo criterio que en los demás orígenes).

**Cómo se prueba** (verde el 2026-10-28; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse                  # 49 unitarias; tests/tipos.rs × 4 versiones: crea una tabla con UInt64, Enum, FixedString, UUID, IPv4/6,
                                              # LowCardinality, Nullable, DateTime con zona y DateTime64(3), la lee y compara con lo insertado;
                                              # Int128, Map y Array se rechazan con su nombre
ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos 5,1,6    # en las cuatro versiones
```

### Paso 5 · hecho (2026-10-28): filtros, orden y límite

Los casos 2, 3 y 4 pasan en 24.8, 25.3, 25.8 y 26.4: **30/30 pruebas de operadores** y los diez declarados, `limit` y `orderBy`. El conector declara ya los diez operadores sin residual.
El SQL sale de `traductor.rs`; `lectura.rs` sólo lo manda.

- **Los valores van atados, nunca en el texto del SQL:** cada uno es un parámetro del servidor (`{p0:Int64}`) con el tipo de la columna. Una comilla en el valor no llega al SQL.
  Los tipos de la tabla salen de `system.columns` (una consulta, sólo si hay filtros); una columna que no está es `esquema`, una tabla que no está (o que el usuario no ve) es `objeto`.
- **Un valor que no cabe en la columna es un error `operador`, no otro valor.** Enteros por su rango (`UInt8` ← 300 sería 44 en el servidor), `UInt64`, fechas fuera de 1900–2299
  (el rango de `Date32`; el servidor las recorta), decimales de más de 38 dígitos. Un decimal se ata con **la escala de su valor**, no la de la columna: `< 0.005` sobre `Decimal(12, 2)`
  no se redondea a `< 0.01`.
- **Instantes y fechas:** el instante va como microsegundos (`fromUnixTimestamp64Micro({p:Int64}, 'UTC')`) y la fecha como días (`toDate32({p:Int32})`): exactos, sin parsear texto en el servidor.
  El tipo declarado decide si un `DateTime64` se compara como instante o como hora de reloj.
- **Semántica de SQL:** `neq` y los de orden no dejan pasar el nulo; `in` vacío es `0`; `like` sólo sobre `String`/`FixedString` (distingue mayúsculas); `orderBy` lleva `NULLS LAST` también en `DESC`.
- **`readonly=1` funciona:** los parámetros del servidor no son ajustes, así que se atan con ese perfil (probado con `ore_r1`).

**`FINAL`: aquí se decidió «no por defecto», y el paso 8 lo corrige** (ver su nota): X0 §3.3 ya había medido que sin `FINAL` una `Replacing` cuenta 4 100 000 filas donde son
2 900 000, y que «la corrección es del origen» exige `FINAL`. Una decisión de rendimiento no puede cambiar lo que la lectura significa.

**Cómo se prueba** (verde el 2026-10-28; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse     # 59 unitarias (atado, rangos, decimal, instante, in vacío, like, orden, comilla en el nombre) + leer × 4 versiones × (kit, ore_r1, ore_r2):
                                 # in + like + orderBy + limit; el UInt8 con 300 se rechaza y con 44 da una fila
ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos 2,3,4    # en las cuatro versiones
```

**Mapa del kit en 25.8:** pasan 1 a 7, 12 y 13; fallan el 8 y el 10 (paso 6) y el 11 (paso 7); el 9 y el 14 no aplican.

### Paso 6 · hecho (2026-10-28): tiempo y cancelar

**El caso 8 pasa en las cuatro versiones** (`tiempo` a los ~1,2 s con `timeoutMs` 1000 sobre `kit.lenta`, nada vivo en el origen). El 9 no aplica (una lectura no escribe: no hay SQL).

- **`timeoutMs` (o el de la URL) corta por dos lados.** El servidor con `max_execution_time` en segundos con decimales (`1000` ms → `1`, `1500` → `1.5`; cero sería «sin límite» y no se manda), y el
  cliente 2 s después con un tope de toda la petición (`Cliente::con_tope`), por si el servidor no contesta o el perfil no deja mandar ajustes. Un tope vencido es `tiempo`
  también en Windows, donde llega como el error 10060 sin el texto «timed out».
- **Cancelar mata la consulta en el servidor.** Cada lectura lleva un `query_id` propio, anotado en el `Token` mientras dura; `cancelar` hace `KILL QUERY WHERE query_id = … ASYNC` con
  un cliente aparte (tope de 3 s, mejor esfuerzo) y la lectura acaba sola con «lectura cancelada». **Medido: la consulta deja el servidor en 0,5–0,8 s en las cuatro versiones**
  (el `sleep` de la vista lenta se interrumpe). Además cada consulta lleva `cancel_http_readonly_queries_on_client_close=1` cuando el perfil lo permite: si el proceso muere sin poder
  hacer el `KILL`, cerrarse la conexión cancela la consulta.
- **El límite, medido: un perfil `readonly=1`** no puede mandar ese ajuste ni (sin el permiso) hacer `KILL`. Ante un `timeoutMs` la lectura vuelve a tiempo con `tiempo`, pero **una consulta que
  no emite nada sigue en el servidor hasta que intenta escribir y encuentra la conexión cerrada**: 3–12 s en la vista lenta (un bloque). Una consulta que va dando filas se corta antes.
  El conector no lo puede arreglar desde el cliente; el que opere el servidor puede dar el ajuste en el perfil.
- **«Vaciar el socket sin pausas largas» no necesita un hilo aparte:** el bucle de lectura sólo se detiene al escribir el lote por stdout, y el lote es de 65 536 filas. Un consumidor parado más de ~30 s
  con el socket sin leer sigue siendo cortado por el servidor, y eso es un fallo del consumidor, no del conector.

**El caso 10 del kit no se puede cerrar en Windows.** Manda `kill -TERM`, y Windows no tiene `SIGTERM`: el proceso muere de golpe y nadie hace el `KILL`. Lo que queda es el cierre de la conexión:
pasa en 26.4 (1,7 s) y no en 24.8, 25.3 ni 25.8, que lo notan más tarde (la consulta lenta no emite nada). **Lo que sí está probado es el camino que SIGTERM recorre en Linux**
(`Origen::cancelar` → `KILL`), en `tests/cancelar.rs`; queda por correr el caso 10 en Linux (la VM de laboratorio o el CI), y eso es lo único que falta para darlo por cerrado.

**Cómo se prueba** (verde el 2026-10-28 salvo el caso 10; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse --test cancelar     # × 4 versiones: timeoutMs con kit y con readonly=1 (tiempo, y el tiempo que tarda en irse la consulta); cancelar desde otro hilo y medir que se va
ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos 8,10
```

El banco del kit buscaba la consulta viva por `kit.lenta` y el SQL lleva el nombre entre comillas: ahora quita las comillas antes de comparar.

### Paso 7 · hecho (2026-10-28): servir y estimar

**Los casos 11 y 14 pasan en las cuatro versiones**; el conector declara ya `servir` y `estimar`.

- **`servir`.** El bucle es el del esqueleto (`ore_driver::origen`), que guarda **una `Sesion` por URL** (por credencial): 100 peticiones comparten un `Cliente`, es decir un agente HTTP con su *pool* de
  conexiones, y otra credencial es otra sesión. Una sesión que pierde la conexión se tira y se abre otra. HTTP no tiene sesión en el servidor, así que el kit no puede contarlas (el banco no
  implementa `sesiones`): comprueba que las 100 respuestas llegan y que otra credencial sigue funcionando. 100 peticiones con un filtro tardan ~10 s en este laboratorio (Docker en Windows, binario de depuración):
  cada una hace dos idas, los tipos de la tabla y la lectura. **No se guardan los tipos entre peticiones**: una tabla que cambia de forma en mitad de una sesión larga daría un valor atado con el tipo viejo, y es
  un fallo peor que una ida de más. Si algún día importa, la caché lleva caducidad.
- **`estimar`.** `SELECT total_rows, total_bytes FROM system.tables` para el objeto: ni una fila leída, nada que facturar (caso 14: `filas 1000000, bytes 12293009` en ~0,2 s). Contesta
  `{"filas":N,"bytes":N,"fuente":"system.tables"}` con lo que el servidor sabe. **Es la tabla entera: los filtros no se descuentan** (una cota superior, que es lo que un planificador necesita). Una vista
  o un motor que no guarda su tamaño devuelve `\N` en los dos: el objeto **no estima** (error `operador`), y una tabla que no existe o que el usuario no ve es `objeto`.

**Cómo se prueba** (verde el 2026-10-28; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse --test leer    # estimar × 4 versiones: kit.grande dice 1000000, la vista lenta no estima, una tabla que no existe es objeto
ore-kit --conector target/debug/ore-read-clickhouse --banco clickhouse --casos 11,14
```

**Mapa del kit completo (2026-10-28), en Windows:** 24.8, 25.3 y 25.8 pasan 12 de 14 (falla sólo el 10); **26.4 pasa los 13 que aplican**. El caso 9 no aplica. El 10 es el de §6 paso 6: pendiente de correrlo en Linux.

### Paso 8 · hecho (2026-10-28): verbos propios, consistencia y `FINAL`

Los cinco verbos de más allá del protocolo v2, en `verbos.rs`. Todos abren con `ClickHouse::conectar` (versión, suelo y perfil incluidos) y ven **lo que el usuario puede ver**: `system.tables` y
`system.columns` no enseñan lo que no se puede leer, y las bases del servidor (`system`, `INFORMATION_SCHEMA`) no se catalogan.

| Verbo | Qué hace |
|---|---|
| `check` | abre, lista lo que ve y **lee una fila de una tabla** (el permiso de lectura); el motivo del fallo va literal y sin la credencial. Una base de la URL sin tablas visibles es `objeto` |
| `catalogo` | cada tabla y vista con los tipos de `system.columns` (el de ClickHouse en `origen`, el de OOS en `tipo`, `None` si no se sabe leer), obligatoriedad por `Nullable`, comentarios y filas estimadas. **`clave` y `unicas` van vacías**: el `PRIMARY KEY` de ClickHouse es un índice disperso que no impide repetidos, y la ausencia es una respuesta cierta; su clave de ordenación sale como aviso |
| `explorar` | las bases con objetos que se ven, y cuántos |
| `testigo` | `none`: no hay un orden de cambios que se lea de fuera, y devolver «ahora» inventaría una marca (ADR 0016) |
| `cambio` | huella de una `MergeTree` no replicada = hash de los nombres de sus *parts* activas + cuántas, con `respalda: true`. Un `INSERT`, un `UPDATE` o `DELETE` (también el ligero) y un `TRUNCATE` la cambian; una fusión también (un falso «cambió», que sólo cuesta una copia de más). **Vistas, `Memory`, réplicas y tablas distribuidas: `respalda: false`.** Sin `GRANT SELECT ON system.parts`: `respalda: false` y un aviso por stderr (la copia no se fía; lo arregla el grant) |

La cara `lee` del catálogo es la de siempre (`predicatePushdown` de la inducción, `fullScan: cheap`) y la `cambia` es `mode: none, witness: none`: **no se declara lo que no se hace**. El incremental por columna (§5) y el CDC son
otra capa; el vocabulario de OOS no tiene un campo de consistencia, así que ésta se declara aquí.

**`FINAL`: corregido.** Una tabla `Replacing` o `Collapsing` (también `Versioned`, `Replicated` y `Shared`) **se lee con `FINAL`**: cada clave sale una vez y las borradas no salen. Verificado: dos *inserts* de la clave 1 dan
2 filas con `FINAL` y 3 en una `MergeTree` con los mismos datos. Cuesta ~2× en lectura, y el catálogo lo avisa. La sesión recuerda el motor de cada tabla 60 s (la de `servir` dura, y un motor sólo cambia
recreando la tabla). `Summing` y `Aggregating` **no** llevan `FINAL`: sus filas parciales son su contrato.

**Consistencia que se declara** (X0 §3.3, y es lo que este origen garantiza):
- **Cada lectura ve una instantánea**: las *parts* son inmutables y la consulta se queda con las que tenía, aunque tarde y aunque entren, se actualicen y se borren filas mientras tanto.
- **Entre lecturas no hay instantánea** (no hay transacciones sin Keeper). Dos lecturas de la misma tabla pueden ver estados distintos, y una lectura paralela por rangos no es consistente entre rangos. Por eso el conector lee con **un solo flujo**.
- **Un `DELETE` ligero o una mutación se ven en la lectura siguiente**, no en la que ya empezó.

**Grants que el usuario necesita** (el banco del kit los documenta): `SELECT` sobre lo que se lee; `SELECT ON system.parts` para `cambio`; `KILL QUERY` para que cancelar mate la consulta; sin ellos todo funciona y se degrada como se dijo.

**Cómo se prueba** (verde el 2026-10-28; fmt y clippy limpios):

```text
cargo test -p ore-clickhouse --test verbos    # × 4 versiones: check (bien, mal usuario, base inexistente), catalogo (tipos, sin clave, sin bases del servidor), explorar, testigo, cambio
                                              # (INSERT, UPDATE, DELETE cambian la huella y sin escribir no; Memory y vista no respaldan; tabla inexistente es error), FINAL (Replacing 2 filas, MergeTree 3)
```

**Un hallazgo del laboratorio, no del conector:** el caso 10 del kit en Windows mata al conector sin que nadie haga el `KILL` y deja consultas de `kit.lenta` vivas en 24.8/25.x; las pruebas de `tests/cancelar.rs` que corrían
justo después las contaban como suyas y fallaban de vez en cuando. Ahora esperan a que se vayan antes de empezar y corren una a una.

## 7. Lo que se aceptó a cambio

*(pendiente)*

## Fuentes

- [Airbyte · ClickHouse source](https://docs.airbyte.com/integrations/sources/clickhouse)
- [Fivetran · bases de datos](https://fivetran.com/docs/connectors/databases)
- [ClickHouse · integración con Fivetran (destino)](https://clickhouse.com/docs/integrations/connectors/data-ingestion/etl-tools/fivetran)
