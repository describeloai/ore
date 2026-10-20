# 0071 · Origen Apache Phoenix

**Leer Apache Phoenix** —SQL sobre tablas de HBase— como origen de ORE, por el **Phoenix Query Server
(PQS)**, que habla el protocolo **Avatica** (Apache Calcite) sobre HTTP. Phoenix pone tipos, catálogo y
SQL sobre un almacén de claves ordenadas; lo que no pone es un cliente que no sea Java.

**Estado:** X0 medido (2026-10-20) contra Phoenix 5.1.3 + HBase 2.4.18 + PQS 6.0.0 en Docker, con el
cliente Python `phoenixdb` como sonda y los bytes del protocolo grabados (§3, `pruebas-de-fuego/phoenix/trazas/`).
**M1, M2 y M3 hechos:** el cliente Avatica (`ore-avatica`), el conector `ore-read-phoenix` y su catálogo (§6). **Sin construir:** los filtros y el kit (M4). **Sin medir:** autenticación, TLS, permisos,
Phoenix ≥ 5.2, varios nodos (§3.11). **Hoja de ruta mientras se desarrolla; al cerrarse, la imagen** de lo
que es el origen Phoenix en ORE.

Sigue a [`0008`](0008-el-protocolo-del-driver.md), [`0016`](0016-el-testigo-y-el-rango.md),
[`0032`](0032-el-contrato-de-tipos.md), [`0069`](0069-origin-mongodb.md) y
[`0070`](0070-origin-cassandra.md) (el patrón: medir primero, construir después; el esqueleto
`ore_driver::origen`; el vocabulario cerrado de operadores).

## 1. Qué se decide

- **v1: Phoenix por PQS, solo lectura.** Nada de HBase crudo, ni del cliente «grueso» (JDBC directo a
  ZooKeeper y a los RegionServers: solo existe en Java), ni de Kerberos (§5).
- **Un binario `ore-read-phoenix`** sobre el esqueleto `Origen`, y **un cliente Avatica propio en Rust**
  (`ore-avatica`, sin saber nada de Phoenix): no existe ninguno, y el protocolo que hace falta son ocho
  mensajes protobuf (§3.2). Se prueba contra los bytes grabados de un PQS de verdad.
- **Catálogo real**, de Avatica (`getTables`, `getColumns`) y de `SYSTEM.CATALOG`; no se infiere nada.
- **La clave primaria es el único camino rápido.** Un filtro por rango de la clave es un `RANGE SCAN`; los
  demás son un barrido completo (§3.6). El conector empuja lo que traduce exacto y evalúa el resto con
  semántica SQL, como en Cassandra.
- **Cuatro invariantes del protocolo** (§3.3) que el cliente cumple siempre: pide el tamaño del primer
  frame, lee `missing_statement`, no confía en el `offset`, y trata «0 filas y no terminado» como error.

## 2. Lo que hacen los demás

Leído el 2026-10-20. **Ni Airbyte ni Fivetran tienen un conector de Phoenix.** Para HBase, Fivetran dijo
en su foro que no hay plan («no planificado») y su *Connector SDK* trae un ejemplo `apache_hbase` con
`happybase` y Thrift; Airbyte no tiene HBase en lo que se pudo comprobar. No hay estándar de industria que
copiar.

| | qué hace |
|---|---|
| Clientes Avatica existentes | JDBC (el de referencia), Python (`phoenixdb`), Go (`calcite-avatica-go`), .NET. **Ninguno en Rust** (búsqueda en la web, sin mirar crates.io) |
| Apache SeaTunnel | source de HBase propio |
| CData | HBase por su driver comercial, «acceso en vivo» |

## 3. Lo medido

Laboratorio: `pruebas-de-fuego/phoenix/` (`Dockerfile`, `arranque.sh`, `hbase-site.xml`, `semilla.py`,
`trazas.py`). Sonda: `phoenixdb` 1.2.2 (con un doble de `gssapi` en Windows, que no tiene Kerberos).

### 3.1 El laboratorio, y por qué costó

No hay imagen oficial con PQS; se arma desde los binarios de Apache. **Lo que falló, en orden:**

1. `archive.apache.org` va a ~170 KB/s: el tarball de Phoenix (400 MB) tardaba una hora. HBase y el PQS
   salen de un espejo y los jars de Phoenix de Maven Central (la imagen se construye en ~1 minuto).
2. **Los scripts del PQS 6.0.0 son Python 2** (`print "x"`): no corren con `python3`. Se lanza la clase
   (`org.apache.phoenix.queryserver.server.QueryServer`) con `java` directamente.
3. **El PQS 6.0.0 publicado solo funciona con el cliente de Phoenix 5.1** (PHOENIX-6861): con 5.3 arranca
   y muere con `Servlet class …JMXJsonServlet is not a javax.servlet.Servlet`; con el `javax.servlet`
   suelto, al revés. Con 5.2+ hay que **compilar el PQS sin el perfil `shade-javax-servlet`**: no se ha
   hecho. **El laboratorio es 5.1.3 + HBase 2.4.18.**
4. El cliente `embedded` no trae Hadoop ni el registro: el PQS necesita `$HBASE_HOME/lib/*` y
   `lib/client-facing-thirdparty/*` en el *classpath* (`NoClassDefFoundError` de `commons-collections`,
   luego de `log4j.AppenderSkeleton`).
5. La primera conexión crea las tablas `SYSTEM.*` y tarda ~15 s; el primer intento puede dar un 500.
6. Con `phoenix.schema.isNamespaceMappingEnabled=false` **un esquema es solo un prefijo**: `CREATE SCHEMA`
   da `725 / 43M08`. El conector no debe depender de que existan.

**Reproducible desde cero:** la imagen quedó una vez con el `arranque.sh` viejo porque el arreglo se había
copiado a mano a un contenedor; se detectó al levantar un segundo contenedor. Se prueba siempre con
`docker build` + `docker run` limpios.

### 3.2 El protocolo en el cable

Todo en un `POST` al puerto del PQS (8765), `Content-Type: application/x-google-protobuf`. El cuerpo es un
**`WireMessage`** con `name` = `org.apache.calcite.avatica.proto.Requests$<Nombre>` y `wrapped_message` = el
mensaje serializado; la respuesta es otro `WireMessage` con `…Responses$<Nombre>`.

| petición | respuesta | nota |
|---|---|---|
| `OpenConnectionRequest{connection_id, info}` | `OpenConnectionResponse` | el id lo inventa el cliente; `info` lleva propiedades de Phoenix (`TenantId`, `CurrentSCN`, `phoenix.query.timeoutMs`) |
| `CreateStatementRequest` | `CreateStatementResponse{statement_id}` | |
| `PrepareAndExecuteRequest{sql, first_frame_max_size, max_rows_total}` | **`ExecuteResponse`** `{results[] ResultSetResponse}` | la referencia no nombra la respuesta; es `ExecuteResponse`, con una **lista** de resultados |
| `FetchRequest{statement_id, offset, frame_max_size}` | `FetchResponse{frame, missing_statement, missing_results}` | |
| `CloseStatementRequest`, `CloseConnectionRequest` | `…Response` | |
| `SchemasRequest`, `TablesRequest`, `ColumnsRequest` | `ResultSetResponse` | metadatos JDBC |
| `ErrorResponse` | | `error_message`, `error_code`, `sql_state`, `severity` |

### 3.3 Cuatro cosas que el protocolo hace y no dice

1. **Sin `first_frame_max_size` el primer frame llega vacío** (0 filas, no terminado): hay que pedirlo.
2. **El servidor ignora el `offset` del `Fetch`.** Es un cursor hacia delante: pedir el offset 100 devolvió
   las filas 6-8, con `offset: 100` en el frame. **Un reintento de un `Fetch` perdido no relee**: el trozo
   está consumido. Una lectura interrumpida se reinicia entera.
3. **Un `Fetch` tras cerrar el *statement* devuelve 0 filas sin error** en el cliente `phoenixdb` (que
   esconde el campo). En crudo viene **`missing_statement=true` y `missing_results=true`**; un statement que
   no existe, igual. Sin leer esas banderas, un statement caducado sería un **truncado silencioso**.
4. **Los ids de statement no son de la conexión:** un `Fetch` desde *otra* conexión leyó el statement de la
   primera. El id es un entero pequeño: quien llegue al PQS puede leer resultados ajenos. No es nuestro
   fallo, pero el conector no debe dejar statements abiertos y no debe confiar en el aislamiento.

### 3.4 Tipos (`tipos` con 23 columnas)

`ColumnMetaData.type` trae `name` (el tipo de Phoenix) y `rep` (cómo viaja). **Lo que distingue un tipo sin
signo es `name`**, no `rep`.

| Phoenix | `rep` | valor en el cable | a Arrow |
|---|---|---|---|
| `TINYINT`/`SMALLINT`/`INTEGER`/`BIGINT` | BYTE/SHORT/INTEGER/LONG | `number_value` | Int8/16/32/64 |
| `UNSIGNED_*` | igual que el de signo | `number_value` | **el de signo** (Phoenix los limita al máximo con signo: `UNSIGNED_INT` rechaza 4 000 000 000 con `203 / 22005`) |
| `FLOAT`, `DOUBLE` | `FLOAT`/`DOUBLE` | `double_value` | Float32/64 |
| `DECIMAL(p,s)` | BIG_DECIMAL | **`string_value` exacto** (`'12345.678'`, `'3.14159265358979323846'`) | Decimal128 con precisión y escala; **sin precisión declarada** (`DECIMAL` a secas) → texto exacto, como el `decimal` de Cassandra |
| `BOOLEAN` | BOOLEAN | `bool_value` | Boolean |
| `VARCHAR`, `CHAR(n)` | STRING | `string_value` (UTF-8 correcto: `héllo ñ 日本`) | Utf8 |
| `BINARY(n)`, `VARBINARY` | BYTE_STRING | `bytes_value` (y `string_value` en base64) | Binary |
| `DATE` | JAVA_SQL_DATE | **días enteros** (20746): **pierde la hora** (Phoenix `DATE` lleva fecha y hora) | `CAST(col AS TIMESTAMP)` en el SELECT |
| `TIME` | JAVA_SQL_TIME | milisegundos del día (49 559 000) | Time32 ms |
| `TIMESTAMP`, `UNSIGNED_TIMESTAMP` | JAVA_SQL_TIMESTAMP | **milisegundos** (1 792 503 959 123): **se pierden los nanosegundos** (`…123456789` llega como `…123`) | Timestamp ms; **precisión declarada** |
| `T ARRAY` | ARRAY | `array_value[]` de `TypedValue`; el tipo del elemento va en `type.component` | List |

**Zona horaria:** el mismo dato dio **idénticos** valores con el JVM del PQS en UTC y en `Asia/Tokyo`
(`TZ` del contenedor): no dependen de la zona del servidor con la configuración por defecto.
`CAST(DATE AS TIMESTAMP)` recupera la hora; `TO_CHAR(ts, 'yyyy-MM-dd HH:mm:ss.SSS')` solo da milisegundos.
**Nanosegundos: no hay camino medido.**

### 3.5 Catálogo

- `getSchemas` devuelve pares (esquema, catálogo): `''`, `ORE_X0`, `ORE_X0_AJENO`, `SYSTEM`, y **uno por cada
  vista de inquilino** (`ORE_X0` / `t1`, `ORE_X0` / `t2`).
- `getTables` da `TABLE_TYPE` `TABLE`/`VIEW` (las tablas de `SYSTEM` son `s` en `SYSTEM.CATALOG`), y columnas útiles: `SALT_BUCKETS`,
  `MULTI_TENANT`, `VIEW_TYPE`, `IMMUTABLE_ROWS`, `GUIDE_POSTS_WIDTH`, `INDEX_STATE`. Los índices secundarios
  son tablas aparte.
- `getColumns` da `TYPE_NAME`, `DATA_TYPE` (código JDBC), `COLUMN_SIZE`, `DECIMAL_DIGITS`, `NULLABLE`,
  `ORDINAL_POSITION`, **`KEY_SEQ`** (la posición en la clave primaria) y `ARRAY_SIZE`.
- `SYSTEM.CATALOG` directo confirma `SALT_BUCKETS` (4), `MULTI_TENANT`, `VIEW_TYPE` y `TENANT_ID`.
- **Una tabla con sal no enseña la columna de sal** en `SELECT *` ni en `getColumns`.
- **Las vistas de inquilino salen en el catálogo pero no se pueden leer sin `TenantId`** (`1012 / 42M03`,
  tabla indefinida); con `TenantId=t1` la vista y la tabla base se ven filtradas a ese inquilino, y la base
  sin él devuelve **todos** los inquilinos. El catálogo tiene que decidirlo (§6, M3).
- **Permisos:** el laboratorio no tiene ACL de HBase; **no se midió** el filtrado por permisos (§3.11).

### 3.6 Qué se puede empujar

Todo esto funciona: `BETWEEN`, `IN`, `LIKE`, `IS NULL`, `<>`/`!=`, `NOT (…)`, `LIMIT` y `OFFSET`,
identificadores entre comillas dobles (los sin comillas se pasan a mayúsculas), `COUNT(*)`. No hay
`ALLOW FILTERING` que prohibir: **el coste es de barrido, no de rechazo.**

| consulta (262 144 filas) | plan (`EXPLAIN`) | tiempo |
|---|---|---|
| `id BETWEEN 10 AND 200` | `RANGE SCAN OVER … [10] - [200]` | 0,1 s |
| `id IN (…)` / clave exacta | `POINT LOOKUP ON 1 KEY` | 0,1 s |
| `importe = 1.25` (columna sin clave) | `FULL SCAN` + `SERVER FILTER` | 2,25 s |
| `COUNT(*)` | `FULL SCAN` | 1,0 s |

Los parámetros `?` no valen en `PrepareAndExecute` (`2003 / INT04`, «binds size: 0»): se usa
`ExecuteRequest` con parámetros o, como en los demás conectores, **literales tipados escapados por el
conector**.

### 3.7 Lectura masiva

Un barrido de 262 144 filas (5 columnas) con un solo cliente Python:

| `frame_max_size` | frames | tiempo | filas/s |
|---|---|---|---|
| 1 000 | 264 | 18,2 s | 14 400 |
| 10 000 | 28 | 6,5 s | 40 600 |
| 50 000 | 7 | 6,3 s | 41 600 |

→ frames de **≥ 10 000** filas; el coste fijo por frame es el cuello de los pequeños.

**Partición por rangos de la clave** (`id >= a AND id < b`, 4 conexiones): 10,0 s → 6,4 s (**×1,6**), en un
nodo, con un cliente Python limitado por el GIL. En Rust y con varias regiones debería ser mejor; **no se
midió**. **Las estadísticas no dan una partición automática en el laboratorio:** `UPDATE STATISTICS` corrió y
`SYSTEM.STATS` quedó vacía (la guía de partición es de 100 MB por defecto y la tabla es pequeña). El
particionado se hará por la clave (§4).

### 3.8 Errores y tiempo límite

**Corregido en M1 con los bytes reales.** El PQS entrega **todo** error SQL en un HTTP 500 con un `ErrorResponse`
cuyo `error_code` es **−1 (4294967295)** y `sql_state` **`00000`**: Phoenix no los pasa. El código de Phoenix va
**dentro del texto** del mensaje (`TableNotFoundException: ERROR 1012 (42M03): Table undefined. tableName=NO_EXISTE`,
encadenado con ` -> `) y en la pila de Java del campo `exceptions`. Los números de la tabla son los que `phoenixdb`
extrae de ese texto; ORE los extrae igual (`ERROR <n> (<estado>)`, en `ore-phoenix`, M2). `ore-avatica` entrega el
error tal como llegó (`Error::Servidor`) y no interpreta Phoenix.

| caso | `error_code` (de Phoenix, en el texto) | `sql_state` | código ORE |
|---|---|---|---|
| tabla que no existe | 1012 | 42M03 | `objeto` |
| columna que no existe | 504 | 42703 | `esquema` |
| error de sintaxis | 601 | 42P00 | `origen` (un fallo nuestro) |
| tipos incompatibles (`id = 'x'`) | 203 | 22005 | `operador` |
| división por cero | 202 | 22012 | `origen` |
| **tiempo límite** | **−1 (4294967295)** | **00000** | `tiempo` |

**El tiempo límite no trae código propio** y hay que reconocerlo por el texto, que tiene **dos formas**:
`SQLTimeoutException: … allotted time: 1500 ms` en una consulta simple, y `TimeoutException` a secas
(`Encountered exception in sub plan [0] execution → java.util.concurrent.TimeoutException`) dentro de una unión.
Se activa con la propiedad **`phoenix.query.timeoutMs` de `OpenConnection`** (1 500 ms cortó a los 2,3 s; 300 ms en
una unión de 262 144², a los 0,7 s). **Tras el error el servidor deja de trabajar** (CPU del PQS de vuelta al 1,7 %
en cuanto el cliente recibe el error): no hay un `killOp` que llamar, pero tampoco una consulta zombi. El cliente
cierra el *statement*.

### 3.9 `estimar`

`EXPLAIN` da el plan en una columna de texto y **solo trae filas y bytes estimados con estadísticas** o en una
búsqueda puntual (`1 ROWS 115 BYTES`). Sin estadísticas (el caso por defecto de una tabla pequeña o recién
cargada) **no hay estimación**. `COUNT(*)` es exacto pero **cuesta un barrido** (1 s por 262 144 filas).
Consecuencia: `estimar` devuelve lo que `EXPLAIN` sepa y, si no, **dice que no lo sabe**; no se inventa.

### 3.12 Lo que Phoenix hace con los filtros del kit (medido para M4, 2026-10-20)

Se cargó la semilla `tipos` del kit en una tabla de Phoenix y se pasó cada filtro del caso 2 escrito como lo
emitiría el conector. **Lo que coincide con SQL:** `=`, `<>`, `<`, `<=`, `>`, `>=`, `IN`, `IS [NOT] NULL` y `LIKE`
(sensible a mayúsculas, `%` y `_`) sobre enteros (también `-9223372036854775808` como literal), `DECIMAL`, `DOUBLE`,
`BOOLEAN`, `DATE` y `TIMESTAMP`; un nulo no casa con `<>` ni con `NOT IN`. `ORDER BY … NULLS LAST` vale en los dos
sentidos (sin él, Phoenix pone los nulos **primero** en `ASC` y en `DESC`) y `LIMIT` se combina con él.

**Lo que no coincide, y qué hace el conector:**

| hallazgo | consecuencia |
|---|---|
| **La barra invertida es un escape dentro de un literal de texto.** `'a'` guarda `a` + retroceso (``); `'x'` deja el literal **sin cerrar**. | El escape duplica **las comillas y las barras**. Con solo las comillas (como en Cassandra) el valor se corrompe y un valor con `'` rompe la sentencia: es el mismo defecto que una inyección. Test y sabotaje propios. |
| **`LIKE` trata `\_` y `\%` como comodín escapado** (no hay `ESCAPE`: error de sintaxis); DuckDB trata la barra como un carácter. | Un patrón con barra **se rechaza** (`operador`), no se aproxima. |
| **La cadena vacía es nulo** (`''` se guarda como `NULL`), y el literal `''` en un filtro también: `<> ''`, `> ''` y `>= ''` no devuelven nada. | El literal vacío se normaliza: `eq ''` y `lt/le ''` → nada, `neq/gt/ge ''` → `IS NOT NULL`, `in` descarta el `''`. El banco del kit declara «el vacío es nulo». |
| **`TIMESTAMP` guarda nanosegundos** y los compara exactos (`= …59.999999` casa, `= …59.999` no), pero Avatica entrega milisegundos. | La precisión sigue declarada en milisegundos (§3.4). El banco carga milisegundos, como el de Mongo. |
| `TO_TIMESTAMP(…, 'SSSSSS')` interpreta los microsegundos como **milisegundos** (año 2100). | Los literales de instante se escriben `TIMESTAMP '…'`; los de fecha, `TO_DATE(…, 'yyyy-MM-dd', 'GMT')`. |
| `IN ()` es un error de sintaxis; `local` (una columna de la semilla) es palabra **reservada**. | `in` vacío → `1 = 0`; los identificadores van siempre entre comillas (ya lo hace M2). |
| `NaN` e `Infinity` no se pueden escribir como literal. | Un filtro de `DOUBLE` con `NaN` se rechaza. |
| `TO_CHAR(<entero>)` mete separadores de miles (`fila-1,001`). | La carga del banco usa `CAST(… AS VARCHAR)`. |

**`estimar` no se puede hacer honesto con `EXPLAIN`.** Sus columnas son estructuradas (`EST_ROWS_READ`,
`EST_BYTES_READ`), pero con estadísticas el número es de la tabla **y no del filtro**: `PEDIDOS` da 933 450 filas (real
262 144) **también** con `id BETWEEN 10 AND 200` (real 191), y `SALADA` da 2 735 415 (real 40). Sin estadísticas no hay
número (`kit_tipos`, `kit_grande`). Solo es exacto en un `POINT LOOKUP` (1 fila) y en un `LIMIT n`. Decisión: **M4 no
declara `estimar`** (el caso 14 sale «no aplica», como en Cassandra y MongoDB) y el catálogo sigue sin `filas_estimadas`.

**Orden y coste.** `ORDER BY` por la clave: 0,03 s para las 10 primeras de 262 144; por una columna que no es clave:
3,2 s (`SERVER TOP 10 ROWS SORTED BY`). Ordenar 1,02 M filas por una no clave **sin límite** funciona, en flujo, 45,8 s.

**Cargar 10⁶ filas dentro de Phoenix:** `UPSERT INTO t SELECT id + n, … FROM t WHERE id <= n` duplicando; 1 000 → 1 024 000
filas en 72 s, sin sacar un dato del servidor (el `ejecutar` de `ore-avatica` basta: no hace falta `ExecuteBatch`).

**Un objeto lento para los casos 8 y 10:** Phoenix no tiene `sleep` ni vistas con uniones, pero una vista con un
`REGEXP_SUBSTR` sin coincidencias sobre 1 M filas tarda **23 s** sin dar ninguna fila. Sirve.

### 3.10 `ROW_TIMESTAMP` y lecturas en el tiempo

- Una columna `ROW_TIMESTAMP` en la clave convierte el filtro en un recorte de ficheros de HBase:
  `EXPLAIN` muestra `ROW TIMESTAMP FILTER [1792195200000, 9223372036854775807)` y un `RANGE SCAN`. Es **un
  cursor real** para el incremental.
- **`PHOENIX_ROW_TIMESTAMP()`** funciona en cualquier tabla pero **llega como `DATE` (días enteros)** y en un
  agregado dio 1970: **no es fiable tal cual**; habría que `CAST` a `TIMESTAMP`, y no se midió.
- **`CurrentSCN`** (propiedad de la conexión) lee la tabla **como era en ese instante**: con un instante
  anterior a la creación, la tabla «no existe» (`1012`). Da **lecturas consistentes** (una instantánea) sin
  coste, si el origen la soporta; **sin medir** como mecanismo de testigo.
- **No hay borrados** en un cursor de tiempo (una fila borrada no tiene marca): como en Cassandra.

### 3.11 Sin medir

Autenticación (BASIC, DIGEST, SPNEGO del PQS), **TLS**, **permisos** (ACL de HBase: `GRANT`), varios nodos y
regiones, **Phoenix ≥ 5.2** (PQS compilado a mano) y **HBase ≥ 2.5**, transacciones, índices secundarios
(globales y locales: el optimizador los elige solo), la memoria del PQS con frames grandes (el laboratorio
usa `-Xmx512m`), la caducidad de conexiones y statements ociosos, un PQS detrás de un balanceador (¿hace
falta afinidad? los ids no son de la conexión, §3.3), y el límite de conexiones.

## 4. Cómo estará hecho

Tres capas, para iterar cada una sola:

1. **`ore-avatica`** (sin saber de Phoenix): `prost` + HTTP (los dos ya están en el `Cargo.lock`; se
   comprueba contra `dependencias.rs` al compilar). Abrir/cerrar conexión, crear/cerrar statement,
   `PrepareAndExecute`, `Fetch`, metadatos, `ErrorResponse`. **Los cuatro invariantes de §3.3.** Se prueba
   contra un **servidor falso en el proceso** que reproduce los bytes de `trazas/` (corre en el CI sin
   laboratorio) y contra el PQS real.
2. **`ore-phoenix`**: tipos (§3.4), catálogo (§3.5), traducción de filtros por exactitud (§3.6), evaluación
   residual con semántica SQL (reutiliza lo que ya hay en `ore-sql`), partición por la clave, `estimar` (§3.9).
3. **`ore-read-phoenix`**: el `main` de una línea. Dirección `phoenix://usuario:clave@host[:8765]/[esquema]?tls=…&tenant=…&timeoutMs=…`
   (sigue la forma de `cassandra://`); rechaza cualquier opción que apague la verificación.

**Decisiones que ya se pueden tomar de lo medido:**

- Se envía **siempre** `first_frame_max_size` y `frame_max_size ≥ 10 000`; se lee `missing_statement`; una
  respuesta con 0 filas, no terminada, **es un error** (`conexion`, no reintentable por el mismo statement).
- `DATE` se lee como `CAST(col AS TIMESTAMP)`; los `UNSIGNED_*` como el de signo; la precisión de
  `TIMESTAMP` se **declara en milisegundos** en el esquema.
- El **tiempo límite** (`timeoutMs`) viaja en `OpenConnection` y el texto del error se reconoce.
- Los literales se **escapan y tipan** en el conector (no hay `?`).
- **Lectura de solo-lectura:** el conector solo emite `SELECT` y `EXPLAIN`; nada más sale de la caja de
  herramientas del cliente.

## 5. Incremental

**M7, opcional y último.** Con una columna `ROW_TIMESTAMP` declarada en la clave: `WHERE ts > cursor` es un
recorte de ficheros (§3.10). Sin ella, **no hay incremental** (no se hace un barrido y se llama «incremental»).
**Borrados: no.** `CurrentSCN` queda como posible testigo de instantánea, sin medir.

## 6. La hoja de ruta

Cada hito acaba con algo ejecutable y verde, y se firma con **sabotajes** (como en 0070).

| hito | qué entrega |
|---|---|
| **X0 · medir** | **Hecho (2026-10-20):** laboratorio reproducible, semilla, trazas grabadas, este documento. |
| **M1 · el cliente Avatica** | **Hecho (2026-10-20)**, ver abajo. `ore-avatica`: abrir, ejecutar, paginar, cerrar, metadatos, errores; los cuatro invariantes; servidor falso con las trazas; contra el PQS real. |
| **M2 · leer** | **Hecho (2026-10-20)**, ver abajo. `ore-phoenix` + `ore-read-phoenix`: tipos a Arrow, `leer` en streaming con memoria acotada, `check`, `servir`; `-p`/`TODOS`/`COPY` (la guarda `lista_de_origenes`); pasarela. |
| **M3 · catálogo** | **Hecho (2026-10-20)**, ver abajo. `catalogo`/`explorar`: tablas, vistas, clave primaria (`KEY_SEQ`), sal, inquilinos (se saltan con aviso; `tenant=` las abre). Permisos: sin medir. |
| **M4 · filtros y estimar** | Traducción exacta, residual, vocabulario cerrado, `estimar` honesto; el kit (14 casos); `ci.yml` con el laboratorio. |
| **M5 · rendimiento y robustez** | Partición por rangos de la clave en paralelo; tiempos límite; cierre de statements; PQS caído, statement perdido, reintentos. |
| **M6 · seguridad** | BASIC/DIGEST solo sobre HTTPS; TLS con el diseño de 0070; secretos fuera de los errores; el mínimo de TLS 1.2. |
| **M7 · incremental** | `ROW_TIMESTAMP` (§5). |

### M1 · hecho (2026-10-20)

**`crates/ore-avatica`**, el cliente del protocolo, sin saber nada de Phoenix: `Conexion::abrir` (con las propiedades
de `OpenConnection`), `ejecutar` → `Lectura` (frame a frame, cierra el statement al soltarla o al acabar),
`esquemas`/`tablas`/`columnas` (metadatos leídos enteros, con paginación), `Valor` (nulo, bool, entero, real, texto,
bytes, decimal **exacto como texto**, lista) y `Columna` (nombre, tipo de Phoenix, `rep`, precisión, escala,
componente de un `ARRAY`), y `Error` (red, tiempo agotado, HTTP ajeno, servidor, protocolo, statement perdido,
truncado) con `reintentable()` y `es_tiempo_limite()`.

- **Sin un crate nuevo en el árbol:** `prost` (solo `derive`) y `ureq` ya estaban; `Cargo.lock` gana únicamente la
  entrada de `ore-avatica` (+10 líneas) y `dependencias.rs` sigue en 193. Los mensajes **no se escriben a mano**:
  `pruebas-de-fuego/phoenix/generar_proto.py` los genera de los descriptores de Avatica (los de `phoenixdb`), con
  los números de campo del propio protocolo, y `proto.rs` se regenera idéntico.
- **Los cuatro invariantes (§3.3), cada uno con su prueba:** el primer frame se pide con tamaño (y `filas_por_frame ≤ 0`
  se rechaza al abrir); `missing_statement` y `missing_results` son `StatementPerdido`; el `Fetch` lleva las filas
  leídas y **nunca se reintenta**; un frame sin filas ni `done` es `Truncado` (y cierra el statement).
- **`reintentable()` no es «repite esta petición»:** es «se puede empezar de nuevo la operación entera»: red, tiempo
  agotado, un 502/503/504 de un balanceador y un statement perdido sí; un error SQL, el tiempo límite de Phoenix y
  lo que el protocolo no prevé, no (volverían a pasar).
- **Pruebas (34, corren en el CI sin laboratorio):** 4 unitarias; **6 con los bytes grabados de un PQS real**
  (lectura paginada, los 23 tipos —incluido el decimal exacto, `DATE` en días y `TIMESTAMP` en milisegundos—,
  catálogo, error de tabla, statement perdido, tiempo límite); **18** de protocolo con mensajes fabricados (los cuatro
  invariantes, un `done` sin filas, la sesión compartida entre conexión y lectura, un `ExecuteResponse` sin firma o
  con dos resultados, otra respuesta, una página de balanceador, un puerto sin servidor, un servidor mudo, la
  credencial *Basic*, un metadato en varios frames, identificadores distintos). Más **6 contra el laboratorio**
  (`PHOENIX_LAB_URL`): un barrido de 262 144 filas sin una de más ni de menos y en orden, cuatro lecturas en
  paralelo por rangos de la clave, los tipos, el catálogo (sal, clave primaria), los errores reales y el tiempo límite.
- **Se probó a sí mismo: diez sabotajes**, uno a uno (no pedir el tamaño del primer frame; ignorar `missing_*`; mandar
  siempre `offset 0`; no tratar «sin filas ni done» como error; que el `Debug` enseñe la contraseña; soltar una
  lectura sin cerrar el statement; reintentar un error del servidor; aceptar credenciales en la URL; no recortar un
  cuerpo ajeno; no mandar la credencial), y cada uno pone rojo la prueba que debía. Con cargo en `--no-fail-fast`
  porque, si no, el primer binario rojo oculta los demás.
- **Rendimiento medido** (262 144 filas, frames de 10 000 y 20 000, una conexión): **~35 000 filas/s** en *release*
  (22 800 en *debug*: el *debug* de `prost` es lento, y no es lo que se despliega) y **×2,2** con 4 conexiones en
  paralelo por rangos de la clave (3,4 s). Es del orden de lo que da `phoenixdb` (40 600 filas/s con frames de 10 000): el cuello es el PQS, no el cliente.
- **Lo que costó.** (1) La documentación de Avatica no dice que la respuesta de `PrepareAndExecute` es un
  `ExecuteResponse` con una lista; (2) el error SQL no trae el código de Phoenix (§3.8); (3) la traza de «tiempo
  límite» de X0 no era determinista —esa vez la consulta terminó antes—, y se regrabó con una unión pesada y un límite
  de 300 ms; (4) el `TIMESTAMP` solo llega en milisegundos, y no se arregla en el cliente.
- **Sin explicar:** dos veces, justo tras varias consultas pesadas (la unión de 262 144² con el *heap* del PQS en
  512 MB), la **apertura** de 3 de 4 conexiones simultáneas devolvió un reset de TCP (`os error 10054`). No se ha
  vuelto a reproducir (0 de 75 aperturas simultáneas, 0 de 3 repeticiones del test, desde Python o desde Rust). Lo
  coherente es un PQS agobiado por el *heap*, pero **no está demostrado**; por eso un reset al abrir es
  `Error::Red` y reintentable (con un identificador nuevo), y el laboratorio sigue con `-Xmx512m`.
- **No hecho:** TLS propio (se usa el de `native-tls` del sistema para `https://`), autenticación distinta de
  *Basic*, un `ExecuteRequest` con parámetros (los literales los escapa `ore-phoenix`, §3.6), y contra
  Phoenix ≥ 5.2.

### M2 · hecho (2026-10-20)

**`crates/ore-phoenix`** (el conector) y **`crates/ore-read-phoenix`** (el `main` de una línea), sobre `ore-avatica`:

- **`url.rs`:** `phoenix://usuario:clave@host[:8765][/ESQUEMA]?tls=&tenant=&timeoutMs=&frame=`. Lo que apaga la
  verificación de TLS no se ofrece (cuatro formas), una CA por fichero tampoco, y **una contraseña por HTTP en claro
  se rechaza** (HTTP *Basic* iría legible): exige `tls=true`. Ni el `Debug` ni los errores llevan la contraseña.
- **`tipos.rs` + `lote.rs`:** los tipos de §3.4 a Arrow. Los `UNSIGNED_*` son el entero con signo; `DECIMAL(p,s)` es
  `decimal128(p,s)` **exacto** (más cifras que `s` o más de `p` dígitos **no cabe y no se redondea**, también la
  notación `1E+3` de `BigDecimal`), y sin precisión es texto exacto; `TIMESTAMP` en milisegundos a microsegundos de
  UTC; `DATE` se pide con `CAST(… AS TIMESTAMP)` para no perder la hora (declarado `Date`, se lee tal cual en días);
  `TIME`, un array (JSON) y un binario (base64, `Opaque`). Un tipo declarado por la tabla manda si el dato lo lleva;
  un `TIMESTAMP` declarado `Date` no (tiraría la hora a escondidas).
- **`esquema.rs`:** la tabla de `getColumns` con su clave primaria (`KEY_SEQ`); se filtra por **igualdad exacta**
  porque `_` es un comodín en los patrones de JDBC. Los nombres se citan siempre (y la comilla se dobla).
- **`transporte.rs`:** los errores del PQS en los siete códigos. El código de Phoenix se saca del texto (§3.8): tabla o
  esquema indefinido → `objeto`, columna indefinida → `esquema`, tipos que no se comparan → `operador`, permisos →
  `credencial`, clúster caído → `conexion` (reintentable), el tiempo límite (en sus dos formas) → `tiempo`, un
  statement perdido → `conexion` reintentable. La pila de Java no sale.
- **`lectura.rs`:** el `Origen`. Empuja **la proyección y el `LIMIT`**, nada más: ningún filtro, clave, cursor ni orden
  (`operadores` vacío): lo que se pida se **rechaza con `operador` antes de hablar con el servidor**, nunca se
  ignora. Dos propiedades de la misma columna la piden una vez. La cabecera Arrow se escribe **después** de ejecutar:
  lo que falla sale como un error limpio y no como un flujo a medias. Lotes de 8 192 filas mientras llegan frames de
  20 000: **la memoria es un frame y un lote, no la tabla**. Un `timeoutMs` de la petición abre **su propia
  conexión** con `phoenix.query.timeoutMs` (Phoenix lo lee al abrirla) sin tocar la sesión que `servir` reutiliza.
  La cancelación deja de pedir frames (el PQS no tiene `killOp`, §3.8). Se resuelve `ore_x0.salada` como lo haría
  Phoenix (`ORE_X0.SALADA`) si el nombre exacto no existe.
- **`verbos.rs`:** `check` (abre, pregunta los esquemas y, con un esquema en la URL, lista sus tablas y **lee una
  fila de una**: el permiso), `testigo` (`none`) y `cambio` (`respalda: false`; una tabla que no está es un error).
  `catalogo` y `explorar` contestan que son de M3, y no inventan un catálogo.
- **Empaquetado:** `ci/compilar-binarios.sh` (`-p` y `TODOS`), el `COPY` del `Dockerfile`, el brazo `phoenix` de
  `credenciales.rs` (con lo que no se admite y su porqué: TLS inseguro, contraseña sin TLS, Kerberos, CA por fichero)
  y las pruebas de `fuente.rs` y `rutas.rs`. La guarda `lista_de_origenes` pasa; `dependencias.rs` sigue en 193 y
  `Cargo.lock` solo gana `ore-avatica`, `ore-phoenix` y `ore-read-phoenix`: **ningún crate nuevo**. El binario pesa 1,7 MB.

**Pruebas.** 41 unitarias; **10 de punta a punta con los bytes grabados de un PQS real** (el conector entero, el
cliente Avatica real y el servidor falso: los 23 tipos hasta Arrow, un error de tabla, el tiempo límite, un statement
perdido, el tiempo de la petición, un filtro rechazado sin hablar con el servidor, la contraseña); y **8 contra el
laboratorio** (`PHOENIX_LAB_URL`): 262 144 filas sin una de más ni de menos y en lotes acotados, `LIMIT`, los tipos,
los errores, un `timeoutMs` de 1 ms que corta en el servidor, **cancelar a mitad** (0,7 s, y la sesión sigue
sirviendo), `check`, `testigo` y `cambio`. Y **`pruebas-de-fuego/phoenix/pasarela.py`: 34 comprobaciones** del
binario y de la pasarela de verdad, abriendo el flujo con `pyarrow` y sin fiarse del conector: la pasarela
**descubre** el conector de su carpeta sin que nadie lo nombre; su presupuesto por defecto (100 000 filas) **corta**
una lectura de 262 144 y lo dice (`cortado`, motivo `filas`); cuatro lecturas a la vez salen enteras; `DELETE
/v1/read/{id}` corta una a mitad y la pasarela sigue sirviendo; y **la contraseña no está** en ninguna respuesta, en
`/v1/origins`, en el registro de lecturas ni en lo que escribió la pasarela.

**Se probó a sí mismo: quince sabotajes**, uno a uno (redondear un decimal; no pedir la hora de un `DATE`; que un
comodín de `getColumns` cuele otra tabla; no rechazar lo no declarado; ignorar el recorte por clave; admitir una
contraseña por HTTP en claro; admitir `verify=false`; que una tabla indefinida no sea `objeto`; que el tiempo límite
no sea `tiempo`; un identificador con comillas sin doblarlas; que un statement perdido no sea `conexion`; no empujar
el `LIMIT`; que el tiempo de la petición no llegue a la conexión; no mirar la cancelación; pedir una columna repetida
dos veces), y cada uno pone rojo la prueba que debía (el de la cancelación, solo la del laboratorio). El primer
intento del sabotaje del tiempo de la petición **no compilaba**, y se rehízo con otra mutación.

**Rendimiento medido** (262 144 filas, *release*): **~33 000 filas/s** por el binario y 7,9 s por la pasarela; cuatro
a la vez, de 9 a 15 s (la **variación** es del PQS: 37, 19, 11,6 y 9 s en cuatro repeticiones seguidas, con el *heap* en
512 MB). Una vez, de cuatro lecturas simultáneas, dos recibieron un `502` de la pasarela tras 30 s con el PQS lento;
no se ha repetido en las cinco repeticiones posteriores. Es el mismo síntoma que el reset de TCP de M1 (§M1, «sin
explicar») y se atribuye a la capacidad del laboratorio: **el laboratorio sube a `-Xmx1g`** (`PQS_XMX`), sin
reproducirlo todavía con esa memoria.

**Lo que M2 no hace, dicho:** filtros, ordenación, `estimar` y el recorte por clave (M4); `catalogo` y `explorar`
(M3); partir una lectura grande por rangos (M5). **Cancelar mata el proceso sin cerrar el statement** (el esqueleto
sale con 143 tras avisar): el PQS lo retiene hasta que caduque; cerrar el statement al cancelar es de M5. Y el
conector **no ha corrido en el CI** todavía: la parte sin laboratorio (unitarias, las trazas, la guarda) sí; el
laboratorio y `pasarela.py`, no.

### M3 · hecho (2026-10-20)

`catalogo` y `explorar` en `ore-phoenix` (`verbos.rs`), sin crates nuevos.

- **Cómo recorre.** Un `getTables` (`TABLE` y `VIEW`) da los objetos; después **un `getColumns` por tabla**, porque
  el de un esquema entero falla con `1012` si hay una sola vista de inquilino (§3.5). Sin `SYSTEM`, sin índices
  (`TABLE_TYPE = INDEX`), ordenado: las tablas sin esquema primero, luego por esquema y nombre. Si la URL nombra un
  esquema, solo ese (igualdad exacta: el patrón trata `_` como comodín).
- **Lo que trae cada tabla.** Nombre `ESQUEMA.TABLA` (con las minúsculas citadas tal cual), tipo de OOS
  (`type`), el de Phoenix (`sourceType`: `UNSIGNED_INT`, `INTEGER ARRAY`…), clave primaria por `KEY_SEQ`, obligatoria
  si la clave o `NOT NULL`, `kind` `table`/`view`, `reads` (`predicatePushdown: []`, `fullScan: expensive`) y
  `changes: none/none`. **No lleva `filas_estimadas`**: el `EXPLAIN` se equivocó ×3,6 (§3.9); es de `estimar` (M4).
- **Las vistas de inquilino** se saltan con un aviso en stderr que nombra la tabla, el motivo literal de Phoenix y la
  salida (`tenant=`). Con `tenant=` entran. Una tabla **multi-tenant** se cataloga con un aviso: sin `tenant=` lee los
  inquilinos de todos. Otros avisos: sal (cubos) y columnas `DATE` (se leen como `DateTimeTz`).
- **Una tabla que no se deja catalogar no tumba el catálogo**; solo falla si no se cataloga ninguna.
- **`explorar`** lista los esquemas con tablas o vistas y dice si hay tablas sin esquema.
- **Permisos: sin medir.** El laboratorio no tiene ACL de HBase; `catalogo` lista lo que `getTables` enseña.

**Pruebas.** Unitarias (`objetos_desde`); 7 con la traza real `catalogo_completo` (18 intercambios, con los dos `500` de
las vistas de inquilino; la reproducción casa `getColumns` por tabla, no por orden); 3 contra el laboratorio (que
lo catalogado se lee; `tenant=`; esquema en la URL); la pasarela (6 comprobaciones, 39 en total). El laboratorio gana
una tabla sin esquema, una en minúsculas citada y una tabla inmutable con índice (el índice mutable pide el códec WAL
en `hbase-site.xml`). **Cuatro sabotajes, los cuatro muerden:** sin el filtro `SYSTEM`, sin el filtro de esquema exacto,
una tabla mala que tumba el catálogo, y `obligatoria` sin mirar `NOT NULL`.

**Lo que M3 no hace, dicho:** filtros y `estimar` (M4); permisos; metadatos de `ROW_TIMESTAMP` (M7).

## 7. Fuera de este producto

HBase crudo (Thrift, REST); el cliente «grueso» de Phoenix; Kerberos (SPNEGO); escritura; Phoenix ≥ 5.2
hasta que se compile el PQS (o se decida lo contrario); la consola (`rubix-platform`: la tarjeta ya existe
como «próximamente»).

## Fuentes

- [Avatica: referencia de protobuf](https://calcite.apache.org/avatica/docs/protobuf_reference.html)
- [Phoenix: tipos de datos](https://phoenix.apache.org/language/datatypes.html)
- [Phoenix: ROW_TIMESTAMP](https://phoenix.apache.org/rowtimestamp.html)
- [Phoenix Query Server](https://phoenix.apache.org/server.html)
- [phoenix-queryserver: BUILDING.md (PHOENIX-6861)](https://apache.googlesource.com/phoenix-queryserver/+/refs/heads/master/BUILDING.md)
- [Fivetran: petición de un conector HBase](https://support.fivetran.com/hc/en-us/community/posts/1500001268222-New-Connector-Request-for-new-Apache-HBase-Connector)
- [Fivetran Connector SDK: ejemplos](https://fivetran.com/docs/connector-sdk/examples)
