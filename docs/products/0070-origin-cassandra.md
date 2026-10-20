# 0070 · Origen Cassandra

**Leer Apache Cassandra** —un clúster propio (4.x, 5.x) y **DataStax Astra DB**— como origen de ORE.
Es el primer origen de **columna ancha**: tiene catálogo real y un lenguaje parecido a SQL (CQL), pero
casi todo lo que SQL daría por hecho —filtrar por cualquier columna, ordenar, `!=`, `IS NULL`— el
servidor lo niega o lo cobra.

**Estado:** X0 medido (2026-10-20) contra `cassandra:5` (5.0.9) en Docker, con autenticación y un rol
de solo lectura, y contra el crate `scylla` 1.9; **M1 y M2 hechos** (el crate `ore-cassandra` y el binario `ore-read-cassandra`, §6). **Sin medir:** Astra de verdad (cuenta, *secure connect
bundle*), TLS, varios nodos, Cassandra 4.x. **Hoja de ruta mientras se desarrolla; al cerrarse, la
imagen** de lo que es el origen Cassandra en ORE.

Sigue a [`0008`](0008-el-protocolo-del-driver.md), [`0016`](0016-el-testigo-y-el-rango.md),
[`0032`](0032-el-contrato-de-tipos.md) y [`0069`](0069-origin-mongodb.md) (el patrón: medir primero,
construir después; el esqueleto `ore_driver::origen`).

## 1. Qué se decide

- **v1: Cassandra OSS y Astra DB.** DSE/HCD, ScyllaDB y Amazon Keyspaces se declaran en `capacidades` y
  quedan «de su documentación».
- **Un binario `ore-read-cassandra`** sobre el esqueleto `Origen`, con el crate `scylla` como
  transporte (async, enrutado por token, paginación; compatible con Cassandra según sus mantenedores
  y **medido** aquí contra 5.0.9).
- **Catálogo real**, de `system_schema`; no se infiere nada.
- **Lectura masiva por rangos de token**, en paralelo. **Nunca `ALLOW FILTERING`** en el servidor.
- **Incremental v1 por cursor** (`witness: field`) sobre una columna `timestamp`/`timeuuid` que la tabla
  designa, **solo si hay un SAI** y el cursor es selectivo; borrados **no** (§5).
- **El conector filtra lo que el servidor no sabe** y devuelve exactamente lo pedido (§4).

## 2. Lo que hacen los demás

Leído el 2026-10-20. **Ni Fivetran ni Airbyte tienen un conector de Cassandra como origen** que se haya
podido verificar: Fivetran dijo en su foro que no hay plazo (el post no tiene fecha visible); Airbyte
tiene un destino Cassandra archivado. No hay estándar de industria que copiar; las referencias son:

| | qué hace |
|---|---|
| Spark Cassandra Connector | parte la tabla en **rangos de token** (~64 MB por partición), pagina con un tamaño fijo de filas por ida |
| Kafka Connect (Lenses) | incremental por `TIMESTAMP`, `TIMEUUID` o `TOKEN` |
| Debezium Cassandra | **CDC del commitlog, en cada nodo**, con retraso, posible desorden y duplicados entre réplicas; puede bloquear escrituras si `cdc_raw` se llena. Necesita acceso a los nodos |

## 3. Lo medido

Laboratorio: dos contenedores `cassandra:5` (5.0.9, `MAX_HEAP_SIZE=1G`, ~1,7 GB cada uno; Docker de 5,8 GB),
uno sin autenticación y otro con `PasswordAuthenticator` + `CassandraAuthorizer`; un rol con
`GRANT SELECT ON KEYSPACE`. Cliente: `scylla` 1.9.0 con `openssl-010` (**112 crates**; Postgres 114,
Mongo 223), compilado en 1 min en frío. Las cifras de rendimiento son de **un nodo en Docker y un cliente
en depuración**: valen como orden de magnitud.

### 3.1 Catálogo y tipos

| qué | medido | lo que supone |
|---|---|---|
| `system_schema.tables/columns` | `kind` = `partition_key`/`clustering`/`static`/`regular`, `position`, `clustering_order` y el tipo CQL **en texto** (`frozen<list<int>>`) | el catálogo se pregunta; `primaryKey` = partition key + clustering, real |
| metadatos del resultado (driver) | `frozen<list<int>>` llega como `frozen: false` | el tipo exacto sale de `system_schema`, no del resultado |
| `system_schema.indexes` | `kind: CUSTOM` + `class_name: StorageAttachedIndex` (SAI) o `COMPOSITES` (índice clásico) | se sondea qué columnas filtran en el servidor |
| vistas materializadas | **deshabilitadas por defecto** en 5.0 (`materialized_views_enabled: false`); SASI también | no se catalogan salvo que existan (`system_schema.views`) |
| contadores | tabla con `flags: {counter}`; se leen como `bigint` | `Integer`, solo lectura |
| tipos leídos con `scylla` | `ascii text varchar tinyint smallint int bigint varint float double decimal boolean blob inet timestamp date time duration timeuuid uuid list set map tuple` y **UDT**, anidados | todos llegan; `decimal` = bytes del entero + escala; `varint` = bytes; `duration` = meses, días y nanosegundos; `date` = días + 2³¹; `time` = ns; `timestamp` = ms |
| `vector<float, 3>` (5.0) | llega como `Vector([Float…])` | `list<Float>` |
| cadena vacía y `null` | distintos (`''` vuelve como cadena vacía) | se conservan |
| fila con solo la clave | existe, columnas `null` (hay *row marker*) | es una fila |
| partición **solo con columnas estáticas** | sale **una fila con la clave de clustering a `null`** | rompe «la clave nunca es nula» (§4.4) |
| `writetime(col)` | `null` si la columna es `null`; **no se puede usar en `WHERE`** (error de sintaxis) | no sirve de cursor |

### 3.2 Qué filtros acepta el servidor

| petición | resultado | consecuencia |
|---|---|---|
| `col = x` sobre columna sin índice | `Invalid`: «might involve data filtering… use ALLOW FILTERING» | no se empuja |
| `col = x ALLOW FILTERING` | funciona (300 000 filas, 2,5 s) | existe, pero lee todo en el servidor |
| `col = x` con índice clásico | funciona | `eq` |
| `col > x` con índice clásico | `Invalid` (pide `ALLOW FILTERING`) | solo `eq` |
| `col > x`, `<=`, rangos con **SAI** | funciona sin `ALLOW FILTERING` | `range` empujable |
| `!=` | **`Unsupported "!=" relation`**, también con `ALLOW FILTERING` | nunca se empuja |
| `IS NULL` | error de sintaxis (solo existe `IS NOT NULL` en vistas) | nunca se empuja |
| `LIKE` | solo sobre columnas indexadas | no se empuja |
| `id IN (1,3)` sobre partition key | funciona | `in` |
| `id > 1` sobre partition key | pide `ALLOW FILTERING` | no |
| `token(id) > a AND token(id) <= b` | funciona | **el recorrido por rangos** |
| `ORDER BY n LIMIT 2` sin restringir la partición | `ORDER BY is only supported when the partition key is restricted by an EQ or an IN` | `orderBy: false` |
| `LIMIT n` | funciona, pero sin orden devuelve «las primeras n» del anillo | `limit` solo si no hay nada que quite filas |
| `USING TIMEOUT` | error de sintaxis (es de ScyllaDB) | **no hay tiempo límite por consulta en el servidor** |
| timeouts del servidor | `read_request_timeout: 5000ms`, `range_request_timeout: 10000ms`, globales | `timeoutMs` es del **cliente** |

### 3.3 Lectura masiva (300 000 filas, ~120 B cada una, un nodo, `num_tokens=16`)

| estrategia | filas/s |
|---|---|
| un flujo, página de 1 000 | 44 800 |
| un flujo, página de 5 000 | 60 800 |
| 16 rangos de token, **1** a la vez | 47 700 |
| 16 rangos de token, **4** a la vez | **108 800** |
| 16 rangos de token, 8 a la vez | 116 800 |
| 16 rangos de token, 16 a la vez | 120 300 |

Con un nodo se satura hacia 4–8; con varios nodos los rangos se reparten y el límite sube con el
clúster (**sin medir**).

### 3.4 Cursor sobre SAI

| consulta | filas | tiempo |
|---|---|---|
| `ts >` (último 1 %) | 2 999 | **0,43 s** |
| `ts >` (último 50 %) | 149 999 | **17,6 s** (≈ 8 500 filas/s) |
| escaneo completo, 16 rangos | 300 000 | 2,5 s |

Un SAI con un rango **ancho** es ~14 veces más lento por fila que un escaneo. **El cursor solo vale si es
selectivo**; pasado cierto umbral conviene escanear por rangos y filtrar en el conector.

### 3.5 Permisos, errores y cancelación

| qué | medido | código del conector |
|---|---|---|
| rol con `SELECT` en un keyspace | lee, pagina, usa SAI, lee `system.local` (versión, `tokens`); **no escribe** («has no MODIFY permission») | solo lectura la da el **rol** |
| `system_schema` | **lista keyspaces, tablas y columnas de otros keyspaces a los que el rol no tiene permiso** (`otra.secreta`) | el catálogo **filtra** por permiso: `LIST ALL PERMISSIONS OF <rol>` funciona sobre el propio rol |
| contraseña mala | `AuthenticationError`, 147 ms | `credencial` |
| sin credencial con auth activa | `MissingAuthentication` | `credencial` |
| nadie escucha / IP muda | `ConnectTimeout` (a los 2 s configurados) | `conexion` |
| tabla que no existe | `Invalid: table … does not exist` | `objeto` |
| sin `SELECT` sobre la tabla | `Unauthorized: has no SELECT permission` | `credencial` |
| columna inexistente | `Invalid: Undefined column name` | `esquema` |
| filtro que pide `ALLOW FILTERING` | `Invalid` | `operador` |
| tiempo del cliente | `RequestTimeout(1ms)` | `tiempo` |
| soltar el cursor a las 100 filas | 107 ms; **no hay `killOp`** | cancelar = dejar de pedir páginas |

### 3.6 Lo que el laboratorio enseñó, y no estaba en la documentación leída

- **El driver descubre los nodos y usa SU dirección**, no la del punto de contacto. Con Docker, NAT, un
  balanceador o Kubernetes, esa dirección no se alcanza: *«ConnectTimeout»* tras conectar bien. El driver
  hereda además **el puerto del punto de contacto** para los nodos descubiertos. Hace falta un
  `AddressTranslator`, y la URL tiene que poder pedir «solo este punto de contacto».
- Un `INSERT` con solo la clave y una partición con solo estáticas no son lo mismo: la segunda tiene la
  clave de clustering nula (§3.1).
- Los timeouts de lectura son **del servidor y globales**: un recorrido que tarde más de 10 s por rango
  falla con timeout de servidor, y la única defensa es paginar y partir en rangos pequeños.

## 4. Cómo estará hecho

1. **Un binario `ore-read-cassandra`**, URL `cassandra://usuario:clave@host:9042/keyspace`; sin
   credencial en `argv`. Los sabores se declaran en `capacidades`; lo que cambia entre ellos (SAI,
   vector, MV) se sondea.
2. **Catálogo** de `system_schema` filtrado por los permisos del rol (§3.5). `primaryKey` = partition key
   + clustering. Cada tabla emite `reads`: `predicatePushdown` según lo que el servidor admite,
   `fullScan: expensive` si no hay índice, y **`requiredFilters`** (lo ofrece la spec OOS) para tablas
   que de verdad lo exijan.
3. **Tipos** CQL → OOS (0032): `varint`/`decimal` exactos (`Decimal<p,s>` o `String` si no caben),
   `duration` como `String` con la forma de CQL (`14mo3d14400000000000ns`: OOS no tiene intervalo),
   `timeuuid`/`uuid` e `inet` como `String`, `counter` como `Integer`, colecciones, tuplas, UDT y `vector`
   como `String` con JSON (un mapa es un objeto con la clave en texto), `blob` como `Opaque` en base64.
   Sin declaración, `varint` y `decimal` viajan como **texto exacto**; la tabla puede declararlos
   `Integer` o `Decimal<p, s>` y entonces un valor que pierde cifras es un error que nombra columna y fila.
   `time` tiene nanosegundos en CQL y microsegundos en OOS: si trae nanosegundos sueltos **no cabe**.
4. **La petición.** El servidor admite poco, así que el conector **empuja lo que puede** (igualdad e `IN`
   sobre la partition key; rangos de clustering; rangos y `eq` sobre columnas con SAI; `token()` para
   partir) y **evalúa el resto él mismo sobre lo que lee**, sin `ALLOW FILTERING`. Lo que no sabe hacer ni
   así se rechaza con `operador`, nunca se ignora. `orderBy: false`; `limit` solo cuando nada lo estropea.
   - **Clave nula.** Una fila con la clave de clustering `null` (partición solo-estática) **se salta** y se
     dice en el informe del catálogo; la tabla no puede prometer una clave que no se cumple.
5. **Lectura masiva** por rangos de token, a 4–8 a la vez, con páginas de ~5 000 filas, desde la lista
   de `tokens` de `system.local` y `system.peers_v2`; Arrow en flujo, en orden. La consistencia por
   defecto es `LOCAL_QUORUM`, configurable. **No es una instantánea**: se dice.
6. **Credencial y red.** Usuario y contraseña al cofre como `fuente-<n>`; rol con `SELECT` sobre el
   keyspace. TLS siempre que se pida; sin `AddressTranslator` se rompe tras NAT (§3.6), así que la URL
   declara el modo «solo el punto de contacto». **Astra** usa un *secure connect bundle* (un fichero):
   el crate lo soporta tras la feature `unstable-cloud`; cómo viaja hasta el driver es lo que hay que
   resolver, como la CA propia de MongoDB (0069 M5).
7. **Cancelar y tiempo.** No hay `killOp` ni tiempo por consulta en el servidor: `timeoutMs` es del
   cliente (`RequestTimeout`) y cancelar es dejar de pedir páginas y soltar el cursor.

## 5. Incremental

El contrato ya tiene el modo (`witness: field`, 0016). Para Cassandra:

| qué | v1 |
|---|---|
| altas y actualizaciones | **cursor** sobre una columna `timestamp`/`timeuuid` **designada por la tabla**, con **SAI**; el conector lee `WHERE col > cursor` si hay SAI y el cursor es selectivo, y si no, recorre y filtra |
| sin SAI (4.x) | `changes: none`: copia entera |
| **borrados** | **no se ven** con un cursor. Se reconcilian con un barrido de **solo las claves** (partition key + clustering), que es una lectura barata, y se declara |
| CDC del commitlog | **fuera de v1**: es por nodo, con duplicados, y Astra no da acceso a los nodos |

El riesgo es el que Airbyte documenta para su *cursor field*: si algo modifica filas **sin** tocar la
columna del cursor, el cambio no se ve. Va al informe del catálogo.

## 6. La hoja de ruta

| hito | qué | estado |
|---|---|---|
| **M0 · medir** | este documento | **hecho** salvo lo de abajo |
| **M1 · crate y traductor** | `ore-cassandra`: tipos CQL→Arrow, traductor petición→CQL (puro) con el filtrado en cliente | **hecho** (2026-10-20) |
| **M2 · `ore-read-cassandra`** | `catalogo`, `leer` por rangos, `check`, `explorar`, `capacidades`, `testigo`; el kit | **hecho** (2026-10-20) |
| **M3 · cableado** | `fuente.rs`, `credenciales.rs`, `rutas.rs`, banco del kit, imagen, CI con `cassandra:5` | |
| **M4 · cambios** | cursor sobre SAI, barrido de claves | |
| **M5 · consola** | hoja de Cassandra y de Astra en el asistente | |
| **M6 · prueba de fuego** | contra un Astra real y un Cassandra con autenticación | |

### M1 · hecho (2026-10-20)

El crate **`ore-cassandra`** (`crates/ore-cassandra`), fuera del cierre de `ore-cli`. **58 pruebas
unitarias sin servidor y 9 de laboratorio** contra el Cassandra con autenticación y el rol de solo
lectura de X0: **67/67**. Sin `ALLOW FILTERING` en ninguna parte (hay un aserto).

| módulo | qué hace |
|---|---|
| `tipos` | el texto de `system_schema` (`frozen<list<int>>`, `vector<float, 3>`, UDT con comillas) a un árbol `Cql`, y de ahí al `Tipo` de OOS, su físico de Arrow y si se puede empujar |
| `esquema` | `Tabla` desde las filas de `system_schema.columns` e `indexes`: el rol de cada columna, su orden y el mejor índice (SAI sobre clásico); lo que no se usa se dice. Todo nombre sale entre comillas |
| `traductor` | `Peticion` → `Plan`, **función pura**. Tres modos: `Particion`, `Indice`, `Recorrido` (§3.2 de este documento). Rechaza `orderBy`, bytes, colecciones y duraciones; lo demás que el servidor no sabe va al conector |
| `residual` | lo que el servidor no sabe, con la semántica de SQL: un nulo no casa con nada, `NaN` es el mayor, `like` sin escape, `claves` por pareja |
| `lote` | celdas del driver → `Valor` de OOS → `RecordBatch`: enteros y decimales de **cualquier tamaño** exactos, `varint`→texto, `time` sin perder nanosegundos en silencio, JSON de lo compuesto |
| `url` | `cassandra://…`: la credencial se decodifica, el TLS no se puede apagar, no hay CA por fichero, `contacto=solo` |
| `transporte` | abrir (con la traducción de direcciones), `system_schema`, el anillo de tokens y `leer` un plan; los errores del driver a los siete códigos |

**El traductor, por modo:**

| modo | cuándo | qué empuja |
|---|---|---|
| `Particion` | toda la clave de partición con `=`/`IN` (o `claves` que la cubren) | una ejecución por combinación (hasta 1 000); sobre el clustering, el prefijo con `=` y **un rango** (contigüidad de CQL) |
| `Indice` | sin partición y un predicado que un índice cubre | todos los `=`/rangos sobre columnas SAI; o **un** `=` sobre un índice clásico |
| `Recorrido` | el resto, o `preferir_recorrido` | `token(clave) > ? AND token(clave) <= ?`, los rangos los pone el transporte |

`preferir_recorrido` existe porque el SAI con un rango ancho es ~14 veces más lento que un escaneo
(§3.4): quien conoce la selectividad del cursor (M4) elige.

**Lo que se decidió construyendo, y no estaba en X0:**

- **Un valor que no cabe en el ancho de la columna no se empuja y no falla.** `id = 99999999999` sobre
  un `int` no devuelve nada: el servidor lo rechazaría, el conector lo evalúa y da cero filas. Lo mismo
  con un instante con microsegundos sueltos (CQL tiene milisegundos: un `eq` no casaría y un `<=` casaría
  de más).
- **Una fila con la clave de clustering `null` se salta.** La primera columna de clustering se pide
  siempre para poder hacerlo; una partición solo-estática no es una fila de la tabla.
- **El `limit` solo va al servidor si no queda nada por filtrar aquí**, es una sola ejecución y la tabla
  no tiene estáticas; si no, cada rango cortaría por su cuenta y lo corta quien lee.
- **`float`/`double`, `decimal`, `varint` e `inet` no se empujan** (el servidor los compara con otra
  anchura, escala u orden que el texto de OOS); `uuid` solo `=`/`IN`.
- **El plan dice a qué columna corresponde cada enlace** (`enlaces`), porque un `int` no admite un
  `bigint`: el transporte escribe cada uno con el tipo exacto de su columna (`timeuuid` incluido).

**Medido en el laboratorio:** el recorrido de `grande` (300 000 filas, 4 rangos a la vez) lo lee
entero en **1,9 s (~155 000 filas/s)** pasando por todo el crate, sin una repetida ni una perdida entre
rangos; una igualdad sobre una columna sin índice la filtra el conector sobre las 300 000 y devuelve la
fila exacta; el tiempo del cliente (`timeoutMs: 1`) es `tiempo`. Las siete causas de error de X0 §3.5
salen con su código y la contraseña mala no aparece en el mensaje. El `time` de la semilla (con
nanosegundos) **falla con columna y fila**, como manda 0032 §4.

**El árbol de dependencias.** `scylla` añade 11 paquetes al lock y **ninguno entra en el árbol real**
de `ore-cli` (217 crates), del firmante (17), de `ore-serve` (240) ni de `ore-federation` (262): se
midió con `cargo tree`. Pero `dependencias.rs` cuenta desde el lock, donde `lz4_flex` y `hashbrown`
pasan a tener dos versiones y sus nombres se desambiguan: el cierre de `ore-cli` sube de 192 a 193 y el
del firmante de 27 a 28, igual que con MongoDB (190 → 192). Se sube el tope con su porqué.

**Todavía no hay**: el binario `ore-read-cassandra` y el flujo Arrow IPC enmarcado (M2), el catálogo
filtrado por los permisos del rol (el transporte lista `system_schema` tal cual), `estimar`, `servir`,
el cursor del incremental (M4), TLS contra un servidor (se construye el contexto, sin medir) y Astra.

### M2 · hecho (2026-10-20)

El binario **`ore-read-cassandra`** (`crates/ore-read-cassandra`, un `main` de una línea sobre
`ore_driver::origen`) con los verbos del protocolo, y su **banco del kit**. **El kit de conformidad lo
pasa por las dos puertas**, contra el Cassandra con autenticación de X0:

| puerta | pasan | fallan | no aplican |
|---|---|---|---|
| el conector directo (14 casos) | **11** | 0 | 3 (9, 10, 14) |
| por la pasarela de verdad (10 casos) | **9** | 0 | 1 (el de las colecciones de objetos), estable en tres ejecuciones seguidas |

Los «no aplica» son honestos: el 9 (una lectura no puede escribir: no hay SQL), el 10 (**no hay nada
lento que cancelar**: ni `killOp` ni una consulta que dure más que una página) y el 14 (`estimar` no se
declara). Lo medido: el caso 7 lee **10⁶ filas en 10 s** (~99 000 filas/s con todo el proceso, Arrow por
stdout y 47 MB), en 123 lotes y con el primer byte a los 654 ms; el 8, `tiempo` a los 415 ms con
`timeoutMs: 1`; el 11, 100 peticiones en 1,9 s por **una** sesión (otra credencial, otra); el 12, ninguna
credencial en 44 salidas y la mala sale como `credencial`.

| verbo | qué hace |
|---|---|
| `capacidades` | los diez operadores (lo que el servidor no sabe lo filtra el conector), `limit` sí, `orderBy` no, `estimar` no, `servir` sí |
| `leer` | Arrow IPC en flujo, un lote cada 8 192 filas **mientras los rangos siguen leyendo**; la cabecera se escribe *después* de preparar la sentencia y el anillo, así que lo que falla ahí sale como un error limpio |
| `catalogo` | las tablas de los keyspaces que el rol puede leer: tipos de OOS de `system_schema` con el tipo CQL citado (`sourceType`), `primaryKey`, `fullScan: expensive`, `changes: none`. **Filtra por permisos** (`LIST ALL PERMISSIONS`): `otra.secreta` no sale. Las filas estimadas son las particiones de `system.size_estimates`, solo si la tabla no tiene clustering |
| `check` | abre, pregunta versión y partidor, calcula el anillo y, si la URL nombra un keyspace, lee **una fila de una tabla**; el motivo del fallo va literal y sin la contraseña |
| `explorar` | los keyspaces que el rol ve |
| `testigo` | `none`: Cassandra no sabe fecharse y no se inventa «ahora» (ADR 0016) |
| `cambio` | `respalda: false`; si la tabla no está, es un error |

**Lo que se aprendió construyéndolo, y no estaba en X0:**

- **La salida tiene que ser determinista.** Con 4 rangos leyendo a la vez, la primera versión entregaba
  las filas en el orden en que contestaban, y el caso 1 de la pasarela (el mismo flujo que el conector
  directo) fallaba **a veces**: había pasado por suerte. Ahora cada rango tiene su canal acotado y se
  entregan **un rango tras otro** en el orden del anillo (o los juegos en el de la petición): leen a la
  vez como mucho `paralelo`, los que van por delante se paran cuando su canal se llena, y la memoria no
  pasa de `paralelo × canal × trozo` filas. El rendimiento no cambió (300 000 filas en ~1,8 s).
- **Una sesión no son sus conexiones.** El driver abre varias por nodo. El conector se presenta en
  `system_views.clients` con `APPLICATION_NAME` y un `CLIENT_ID` distinto por sesión, y el banco las
  cuenta por él.
- **El banco del kit también era una sesión.** Usaba el mismo `Conexion` y se presentaba con el nombre
  del conector: el caso «el ocioso se cierra» veía siempre una sesión viva (la suya) y fallaba. Ahora
  `Conexion::abrir_como` le da su propio nombre.
- **Un falso positivo del caso 12 que enseña:** con la contraseña `cassandra` el kit decía que la
  credencial salía en 3 salidas. Era el nombre del conector en sus mensajes. Con una contraseña única,
  ninguna. El laboratorio usa un rol con contraseña propia.
- **Cancelar es dejar de pedir páginas.** Sin `killOp`, soltar los cursores y abortar las tareas es lo
  que hay, y el servidor acaba solo la página que tuviera en curso (107 ms medidos en X0 §3.5).
- **`DateTime` declarado.** La semilla del kit distingue `DateTime` (sin zona) de `DateTimeTz`, y en
  Cassandra los dos son un `timestamp`: lo que los separa es lo que la tabla declara en la petición,
  como en MongoDB. `Tipo::FechaHora` solo existe por esa declaración.
- **La guarda de la lista de orígenes funcionó a la primera:** con el crate `ore-read-cassandra` creado
  y sin cablear, `lista_de_origenes` avisó de que faltaban el `-p`, `TODOS` y el `COPY` del `Dockerfile`.
  Quedan cableados. Como la pasarela descubre los conectores de su carpeta, **no hay que tocar
  `--tipos`**: con la imagen nueva carga `cassandra` sola.
- El kit no encontraba el conector en Windows: exige que se llame `ore-read-<tipo>` y es `….exe`.
  Compara ya sin la extensión (solo en Windows).

**Todavía no hay**: el servicio de CI con `cassandra:5` y los pasos del kit (`ci.yml`), el cableado de
`fuente.rs`, `credenciales.rs` y `rutas.rs` (M3), `estimar`, el cursor del incremental (M4), TLS contra
un servidor (se construye el contexto, sin medir) y Astra. **La imagen con `ore-read-cassandra` está
sin construir**: compila nativo en Windows (MSVC); la compilación en Alpine/musl, como la de
`ore-read-mongodb`, queda por ver en el CI.

**Sin medir todavía:** Astra de verdad (bundle, TLS, límites del plan gratuito), TLS propio,
Cassandra 4.x (sin SAI: su `ALLOW FILTERING` y el modo `none`), varios nodos y varios centros de datos
(consistencia, `peers_v2`), particiones enormes y con muchos *tombstones*, SAI sobre texto (`LIKE`,
analizadores), `USING TIMEOUT`/*speculative retry*, y los compatibles (DSE, HCD, Keyspaces, ScyllaDB).

## 7. Fuera de este producto

- Escribir en Cassandra (un origen se lee).
- CDC del commitlog y Debezium.
- Búsqueda vectorial (`ORDER BY … ANN OF`): el tipo `vector` se lee, no se consulta por similitud.
- `ALLOW FILTERING` en el servidor.

## Fuentes

[DataStax · CQL SELECT](https://docs.datastax.com/en/cql/3.3/cql/cql_reference/cqlSelect.html) ·
[scylla-rust-driver](https://lib.rs/crates/scylla) ·
[Spark Cassandra Connector](https://docs.datastax.com/en/dse-spark-connector-api/6.8/com/datastax/spark/connector/rdd/package.html) ·
[Debezium · Cassandra](https://debezium.io/documentation/reference/connectors/cassandra.html) ·
[Lenses · Cassandra source](https://docs.legacy.lenses.io/5.4/connectors/sources/cassandrasourceconnector) ·
[Fivetran · petición de conector](https://support.fivetran.com/hc/en-us/community/posts/4415310753559-New-Connector-Support-Cassandra).
Medidas: `cassandra:5` (5.0.9) en Docker, 2026-10-20.
