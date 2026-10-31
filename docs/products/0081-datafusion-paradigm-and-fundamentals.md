# 0081 · DataFusion paradigm & fundamentals

**ORE sustituye a DuckDB, por completo y en todos los caminos, por Apache DataFusion distribuido con
Ballista**, sobre su Iceberg y el Parquet de Rust. Primero se construye un fundamento excelente, estándar y
con teoría, que no sabe nada de productos. Después, cada producto vuelve a nacer sobre él.

**Estado:** frentes 1 y 2 cerrados (2026-10-31); siguiente, el frente 3. Borrador desde el 2026-10-30. Rama `datafusion-fundamentos`, worktree `C:\ORE-datafusion`, sobre
`origin/main` 10092ad3. Este borrador recoge la visión, los principios, las
garantías y el estado del arte investigado (§6, con el informe y sus notas en
[`docs/investigacion/`](../investigacion/reports/DataFusion%20y%20Ballista%20sobre%20Iceberg.md)).
La construcción se organiza en **diez frentes** (§7): ocho áreas y dos transversales, cada una un
subproducto manejable que encaja con los demás en una sola imagen. El detalle de cada frente se escribe
cuando se trabaja, empezando por el suelo. Al cerrarlos, se escribe un ADR nuevo con la imagen fiel del
producto.

## 1. La tesis

- **Qué:** DataFusion es el motor de ORE, local y distribuido. DuckDB sale de todos los caminos: los SDK
  de Python, Node y JVM, el núcleo en Rust, la geo, las imágenes y el CI.
- **Para qué:**
  - que ORE sea **el mejor sitio del mundo para correr DataFusion distribuido a escala**;
  - aportar al motor y a su ecosistema.
- **Por qué:** los servicios que ORE va a integrar están construidos sobre DataFusion. No es un capricho:
  es una condición de supervivencia y de diferenciación en el mercado.
- **Por qué encaja:** ORE ya es Rust, arrow-rs, Parquet e Iceberg. DataFusion está construido sobre
  arrow-rs, la misma familia de crates del workspace. Arrow no queda obsoleto: pasa a ser también el
  formato en memoria del motor, además del de tránsito (0043).
- **Cómo se mide frente a la tesis del producto:** el git-lakehouse serverless. El árbol git define, el
  catálogo transaccional (0079) guarda el estado e Iceberg es el formato. El motor nuevo tiene que reforzar
  ese reparto, no cruzarlo.

## 2. El orden: el fundamento primero, los productos vuelven a nacer

**La decisión de fondo.** No se encaja lo que hoy existe en el stack nuevo. Ni Query, ni *Elastic
Sessions* (0073), ni la admisión por plan (0076), ni los SDK se adaptan tal como son. Acomodar una pieza
vieja a un motor nuevo da dos piezas a medias pegadas con cola, y no se domina ni el fundamento ni la
pieza.

1. **El fundamento.** DataFusion y Ballista sobre el lago de ORE, de punta a punta: sesión, catálogo,
   lectura y escritura Iceberg, tipos, distribución, memoria, fallos y observabilidad. Funciona por sí
   mismo, se prueba con la vara estándar (§5) y no depende de ningún producto.
2. **Los productos vuelven a nacer sobre él**, cada uno con su plan:
   - Query y su sesión. Su escalado se rediseña desde lo que el fundamento ofrezca: con un motor
     distribuido, «que la sesión crezca» es otra pregunta (¿en memoria o en ejecutores?).
   - Code Repositories, Builds y Transforms.
   - Las copias de Views.
   - La geo.
   - Node y JVM.
   - El índice de teselas.
   - El dialecto guardado y su migración.
   - Retirar DuckDB.

## 3. Los principios

1. **El fundamento no sabe de productos.** Sabe de catálogos, tablas, planes, ejecutores y commits; nada de
   hojas, puestos ni kernels. Un producto se construye encima, nunca dentro.
2. **Una sola sesión y un solo plan.** Una sola función construye la sesión de DataFusion, y la usan igual
   el proceso local, el scheduler y cada ejecutor. Lo local y lo distribuido son el mismo plan, y **el plan
   no sabe qué distribuidor lo reparte**.
3. **Iceberg por la spec**, contra el catálogo REST de ORE (`/v1`), que sigue siendo el único que mueve un
   puntero (CAS, `Idempotency-Key`, 0031 y 0079). Sin atajos propios en el formato.
4. **El catálogo de ORE es quien sabe.** La resolución de nombres, el snapshot de cada tabla, la poda de
   ficheros y la credencial salen del catálogo, que ya guarda el estado de cada activo en Postgres (0079).
   DataFusion ejecuta. Es el estado del arte (§6.7): poseer catálogo, planificación y poda, y dejar al motor
   la ejecución.
5. **Las garantías se escriben antes que el código** (§4). Si no está escrito, no se promete.
6. **Lo estándar es la prueba** (§5). Aprobar lo de la comunidad es la vara, no unas pruebas a medida.
7. **Upstream primero: extender, nunca bifurcar.** Lo que haya que cambiar va por los traits de extensión
   de DataFusion y los codecs de Ballista, o como PR. Lo que generalice se aporta.
8. **Sin DuckDB dentro.** DuckDB sólo como oráculo de las pruebas diferenciales mientras exista.
9. **Medido contra producción:** victor, nodos dedicados y las variables de prod.
10. **Al paso del upstream.** DataFusion publica una major cada dos o tres meses, con cambios de
    semántica. ORE actualiza **DataFusion, Ballista e iceberg-rust a la vez, una vez por trimestre**, con su
    propio corpus de pruebas como red.

## 4. Las garantías

| | qué se promete | lo que el estado del arte dice de ello |
|---|---|---|
| **semántica SQL** | el dialecto es **el de DataFusion**, documentado; lo que ORE añade (PIVOT, MERGE, sentencias propias) va por las extensiones del parser y del planificador | DataFusion 55 ya trae `QUALIFY`, `EXCLUDE`/`REPLACE`, `ASOF JOIN … MATCH_CONDITION`, `GROUP BY ALL`, `UNION BY NAME`, `DISTINCT ON` y el pipe `\|>`; no trae PIVOT/UNPIVOT, TABLESAMPLE, ATTACH ni MERGE (`RelationPlanner` desde 52). Entre majors cambian semánticas: en 54 la coerción cadena-número, en 55 el `AND` puede reordenarse |
| **tipos y nulos** | para calcular, los de DataFusion; para guardar, los de Iceberg, con una frontera sin pérdida al escribir; la nulabilidad en el esquema (§7.5) | cadenas y binarios en `Utf8View`/`BinaryView` por defecto; tipos de extensión como almacenamiento más `FieldMetadata`, sin casteo propio; cada campo escrito hacia Iceberg lleva `PARQUET:field_id`; decimales hasta 38 |
| **la geo** | WKB con metadatos GeoArrow y su CRS, como ya decidió 0066/0067 | ningún lector Rust trata todavía `geometry`/`geography` v3 de Iceberg (Comet vuelve a Java) |
| **aislamiento** | una sentencia lee **una sola foto del catálogo para todas sus tablas**, fijada al resolverla; una rama se lee con su puntero (0044, §7.5) | el proveedor de catálogo del upstream recarga en cada escaneo y siempre lee el snapshot actual: dos menciones de una tabla pueden ver dos snapshots. El patrón correcto es resolver una vez, de forma asíncrona, con caché de una consulta |
| **escrituras** | append y `create table as` con escritura paralela y **un solo commit** por `/v1`; overwrite, delete y update con la validación de conflictos en el servidor; MERGE después | iceberg-rust sólo hace `fast_append`; overwrite y RowDelta son PR abiertos; su commit reaplica a ciegas tras un conflicto y no envía `Idempotency-Key` |
| **un reintento nunca escribe dos veces** | la `Idempotency-Key` se fija al planificar, viaja en el plan y se repite en cada intento; el 409 es terminal; lo que no se sabe se dice (`CommitStateUnknown`) | la spec REST pide UUIDv7, que se reproduce en 2xx y en 4xx terminales (409 incluido) y no en 5xx; es lo que ORE ya implementa (0031) |
| **fallos** | una tarea se reintenta; un job sobrevive a su scheduler; un lector nunca ve un snapshot a medias | el scheduler de Ballista guarda su estado **sólo en memoria** (etcd/sled retirados en 47); el shuffle está atado al disco del ejecutor; los límites de reintento no están documentados |
| **memoria** | **un contrato público** de lo que derrama, lo que falla y con qué error; nunca un proceso muerto sin explicación | en 55.1 derraman sort, agregaciones, sort-merge join, nested loop y repartition; **fallan** hash join, ventanas, cross join, TopK con k grande, symmetric hash join, unnest y CTE recursiva. Ningún hash join derrama hoy, ni en DataFusion ni en Ballista |
| **gobierno** | el fundamento no decide accesos: ve sólo lo que `/v1` le devuelve, ya filtrado por `ore-serve` (0047) | — |
| **la credencial** | **nunca sale del ejecutor que la usa**: por el plan viaja una referencia firmada que el ejecutor canjea en `/v1` | Ballista expone la configuración de cada job (`GET /api/job/{id}/config`) y la escribe en sus registros de eventos |
| **el clúster** | interno y autenticado entre sus piezas | Ballista no autentica ni autoriza en ningún salto; su servicio de resultados se conecta a cualquier dirección que diga un ticket |

## 5. La vara

- **`sqllogictest`:** el corpus de DataFusion corrido sobre la sesión de ORE, más un corpus propio. Con
  `configMatrix` se prueba cada ajuste no por defecto que ORE active (por ejemplo `pushdown_filters`
  activado y desactivado).
- **TPC-H y TPC-DS** sobre tablas Iceberg de ORE, a varias escalas, en local y distribuido. Las 22 y las
  99 terminan y coinciden.
- **La matriz de conformidad de Iceberg:**
  - leer lo que escriben Spark, PyIceberg y DuckDB: borrados por posición, por igualdad, deletion vectors
    v3, evolución de esquema y particiones;
  - que Spark y PyIceberg lean lo que escribe ORE;
  - **la lista de retrocesos de Comet**, convertida en casos de prueba. Es el catálogo público de lo que
    el lector Rust aún no hace bien: `initial-default` v3, `geometry`/`geography`/`variant`, igualdad sobre
    campos anidados, varios buckets, y lecturas anidadas tras evolución o con name mapping.
- **Diferencial frente a DuckDB:** los datasets de victor y las mismas consultas, con 0 diferencias o con
  la diferencia explicada.
- **Fallos:** el modo de caos de Ballista (`testing.chaos_execution.*`) y la inyección en el commit, con
  0 resultados distintos.

## 6. El estado del arte (octubre de 2026)

El detalle, con cada fuente, está en el informe
[*DataFusion y Ballista sobre Iceberg*](../investigacion/reports/DataFusion%20y%20Ballista%20sobre%20Iceberg.md)
y en sus seis notas. Aquí, lo que decide.

### 6.1 Las piezas y sus versiones

| pieza | versión | lo relevante |
|---|---|---|
| DataFusion | 55.2.0 (55.0.0 el 2026-08-25; 56 en desarrollo) | 877 commits de 175 contribuidores en la 55; MSRV 1.94; los traits de sesión pasan al crate `datafusion-session`; derrame enchufable (`TempFileFactory`) |
| Ballista | 55.0.0 (2026-10-03) sobre DataFusion 55.1.0 | AQE por defecto, sort shuffle, **igualdad exacta de protocolo** entre scheduler y ejecutor |
| arrow-rs y parquet | 59.x con DataFusion 55 | **el workspace está en 58.4** |
| iceberg-rust | 0.10.1 publicada (2026-08-01) va con DataFusion 53 y Arrow 58; `main` ya va con DataFusion 55, Arrow 59.2 y MSRV 1.95 | la poda errónea por tipos promovidos se arregló el 2026-10-22 (#3483): hace falta una rev posterior |
| datafusion-iceberg | sin release; la integración salió de iceberg-rust el 2026-09-17 (#3228) | una partición, estadísticas desconocidas, recarga en cada escaneo, listado ansioso del catálogo |
| object_store | 0.13.x | el registro distingue esquema, host y puerto, nunca la ruta |
| datafusion-distributed | 2.0.0 (Datadog) | streaming, sin reintentos ni persistencia; gobernanza en discusión (#784) |
| SedonaDB | 0.4.0 (2026-06-19) | 176 funciones ST_/RS_, CRS por fila, joins espaciales mayores que la memoria; suele ir una major por detrás de DataFusion |
| Comet | 1.1.0 (2026-10-06) | el lector DataFusion+Iceberg más probado: la planificación en Java, la ejecución en iceberg-rust |

Comet y Lakekeeper fijan iceberg-rust por git rev, y Lakekeeper sobre una bifurcación: a quien construye
motores, la release publicada ya no le basta.

### 6.2 El motor

- **Rendimiento.** DataFusion 55 es el más rápido en ClickBench sobre Parquet particionado en
  c7a.metal-48xlarge y el segundo en c6a.4xlarge. No hay comparación rigurosa de 2026 en TPC-H/DS frente a
  DuckDB.
- **Optimizador.** Es sobre todo heurístico: no tiene un modelo de coste real para ordenar joins (#17718),
  y esa es su mayor debilidad frente a DuckDB en consultas tipo TPC-DS. Su fuerza son los **filtros
  dinámicos** (de TopK, hash join y MIN/MAX), que crecen en cada major.
- **Ejecución.** Pull sobre streams asíncronos, paralelismo por `target_partitions`, escaneo por morsels
  desde 54, y cancelación cooperativa (`EnsureCooperative`).
- **Dos runtimes de tokio.** La práctica documentada en producción: uno de E/S y otro de CPU, con el object
  store enrutado al de E/S (el ejemplo `thread_pools`, InfluxData). Sin eso, las sondas de salud se ahogan
  bajo carga.

### 6.3 Parquet

- **Activado por defecto:** poda por row group, page index, bloom filters en lectura, `Utf8View`, filtros
  dinámicos, caché de metadatos (50 MB), caché de estadísticas de fichero y caché del listado.
- **Desactivado por defecto: `pushdown_filters` y `reorder_filters`.** Es la palanca opcional más grande:
  con filtros dinámicos, ClickBench Q23 pasa de 32,04 s a 1,42 s en un núcleo. También es la más delicada:
  en abril de 2026 se midió desde +7,4 % hasta −13,3 % según la consulta.
- **Leer ordenado por estadísticas:** sort pushdown hasta 27–49× en `LIMIT`, poda TopK 4,2×, morsels
  ~2× en escaneos sesgados.
- **El almacén de objetos** cuesta decenas o cientos de ms por petición. object_store junta rangos a menos
  de 1 MiB y lanza 10 en paralelo.
- **Índices externos.** El camino que DataFusion recomienda para lagos con catálogo:
  1. un `TableProvider` que elige ficheros desde un índice (Postgres incluido);
  2. un `ParquetAccessPlan` por fichero, con su `RowSelection`;
  3. un `ParquetFileReaderFactory` que sirve los metadatos desde caché.

  Así no hay **ningún GET de pie de fichero**. Encaja de lleno con 0079.
- **Escribir.** DataFusion escribe por defecto en `zstd(3)`, con diccionario, estadísticas por página y
  row groups de 1.048.576 filas. Iceberg apunta a **128 MB por row group y 512 MB por fichero**. Lo que más
  premia la poda es ordenar por la clave caliente, con rangos disjuntos entre ficheros, y declararlo.
  Los bloom filters, sólo para igualdades de alta cardinalidad. Las cadenas se escriben como UTF8.

### 6.4 Iceberg

- **Lectura sólida para v1–v3** en el núcleo de iceberg-rust:
  - poda por manifiesto, partición, row group, page index y bloom;
  - borrados por posición, por igualdad y deletion vectors v3;
  - caché de manifiestos (`ObjectCache`).
- **El escaneo del upstream lo desperdicia.** `UnknownPartitioning(1)` y `Statistics::new_unknown` dejan
  al optimizador sin con qué repartir ni ordenar joins. RisingWave, agrupando tareas en particiones, sacó
  2,72× sobre su motor en TPC-H 100 GB, pero se quedó sin memoria en Q5 y Q18 por falta de estimaciones de
  cardinalidad.
- **Dos lectores distintos.** El `ArrowReader` de iceberg-rust aplica borrados. El `ParquetSource` de
  DataFusion tiene materialización tardía, filtros dinámicos y caché de pies, pero no sabe de borrados
  Iceberg. Un borrado por posición o un deletion vector se puede expresar como `RowSelection`; uno por
  igualdad, como anti-join.
- **Escritura.** Sólo `fast_append`. El commit, ante un conflicto, recarga y reaplica a ciegas: es seguro
  para un append, pero no para overwrite ni merge.
- **Credenciales.** iceberg-rust sólo trae almacenamiento OpenDAL, congela la credencial prestada al cargar
  la tabla sin renovarla, y **no guarda en caché las tablas con credencial prestada**.
- **La spec REST** ofrece más de lo que usa el cliente Rust: `loadCredentials`, planificación en servidor
  (`planTableScan`, `fetchScanTasks`) y `commitTransaction`. El cliente no implementa ninguna de las dos
  primeras (#2651 y #3011, abiertos).

### 6.5 Distribuir

- **Ballista** es un clúster por lotes. Tiene shuffle bloqueante materializado en el disco local del
  ejecutor (Arrow IPC, `lz4`) y AQE por defecto: las reglas físicas tienen que ser idempotentes. El umbral
  de broadcast es 128 MiB, y por encima de 64 MiB de lado de construcción usa sort-merge.
  - TPC-H SF1000 con 32 ejecutores: 502 s, frente a 898 s de Spark 4.1.3, 455 s de Spark+Comet y 412 s de
    Trino.
  - Extensión por cinco ganchos: config producer, runtime producer, session builder, function registry y
    codecs lógico y físico.
  - **Rechaza claves de configuración desconocidas**: lo de ORE va como `ConfigExtension`.
- **datafusion-distributed** es una librería, no un clúster: nodos de red en streaming, sin coordinador
  fijo, sin reintentos ni persistencia. Datadog la usa en producción (hasta 10× más rápido al mismo coste
  en consultas pesadas) y resume su lección como «construir un framework, no un servicio» y no distribuir
  lo ligero.
- **Los bancos miden regímenes distintos**, y ninguno es independiente:

  | escala | datafusion-distributed | Ballista | fuente |
  |---|---|---|---|
  | SF10 | 10 s | 42 s | banco propio de datafusion-distributed, quizá con un Ballista anterior a 55 |
  | SF100 | 42 s | 237 s | ídem |
  | SF1000 | 387 s | 279 s (pipelined), 517 s (bloqueante) | Andy Grove, #784 |

  El streaming gana en lo interactivo pequeño y mediano; la materialización da puntos de reintento a
  escala de lote.
- **Huecos de Ballista:**
  - sin HA y sin autenticación;
  - Flight SQL bloquea hasta acabar, no cancela y no escribe;
  - el escalador KEDA mira cada 30 s y no viene en las imágenes publicadas;
  - el hash join híbrido que derrama (#2108) se cerró sin fusionar.

### 6.6 Memoria

- **Lo que se contabiliza.** El pool sólo cuenta a los consumidores grandes: ni el escaneo ni lo
  intermedio. Hay que dejar un 10–20 % de holgura.
- **Ningún pool cambia de límite en marcha**, pero el trait `MemoryPool` es público.
- **`FairSpillPool` reparte mal en tuberías de operadores bloqueantes** (#22036). La propuesta de arriba son
  pools anidados al estilo de Velox: una raíz por consulta y un hijo por operador.
- **El pool no puede pedir a un operador que derrame** (#21422).
- **El hash join** falla en 0,2 s bajo 300 MB; con `prefer_hash_join=false` (sort-merge) termina en 3,7 s,
  unas 3,5 veces más lento cuando cabría. El mapa de un join terminado sigue vivo a través del filtro
  dinámico **sin reserva**.
- **Las ventanas** guardan toda la entrada antes de evaluar (#22946): un riesgo directo para lo
  geotemporal.
- **Ballista** da a cada ejecutor el 70 % del cgroup y a cada tarea un `FairSpillPool` de pool/vcores.

### 6.7 Lo que hacen quienes lo corren en producción

- **Cloudflare R2 SQL** es el análogo público más cercano a ORE: serverless, sobre Iceberg en
  almacenamiento de objetos.
  - Poda de arriba abajo en su propio planificador: catálogo, snapshot, manifest list, manifiestos y pies.
  - Envía a cada trabajador los row groups y los metadatos Parquet ya serializados, y para pronto en
    `ORDER BY … LIMIT`.
  - DataFusion es sólo el ejecutor de cada nodo.
- **Comet** deja la planificación de Iceberg en Java y ejecuta las tareas en iceberg-rust.
- **Datadog**: no distribuir lo ligero, ser una librería y no un servicio, y enrutar las particiones a los
  mismos trabajadores para que las cachés sigan calientes.
- **InfluxDB 3**: cachés de metadatos y de datos en el querier.
- **Spice.ai**: federación con aceleración local por `TableProvider`.
- **ParadeDB**: DataFusion dentro de Postgres.
- **Embucket**: compatible con Snowflake, sobre Lambda y S3 Tables.
- **El patrón común:** el dueño del catálogo planifica y poda, y DataFusion ejecuta. ORE es dueño de `/v1`
  y del estado en Postgres: **el catálogo es el planificador natural**.

## 7. Los frentes

La absorción y construcción del fundamento se parte en diez frentes. Cada uno es un subproducto con su
pregunta central, su previo y la prueba que lo cierra. Ninguno está desconectado: todos casan en una sola
imagen.

### 7.1 Las ocho áreas

| | frente | qué abarca | la pregunta central | decisiones |
|---|---|---|---|---|
| **1** | **el suelo** · la plataforma del motor | el trío de versiones (arrow-rs/parquet 59, DataFusion 55, iceberg-rust por rev); la sesión plantilla que se clona por consulta, construida en un solo sitio; los dos runtimes de tokio (E/S y CPU); `target_partitions` contra el cgroup; las opciones de ORE como `ConfigExtension`; la cadencia con el upstream | ¿cuál es la unidad mínima de motor de ORE, y cómo se construye siempre igual? | DF1, DF13 |
| **2** | **el contrato** · la teoría | la semántica SQL (el dialecto de DataFusion, lo que ORE añade, los cambios entre majors); los tipos y nulos de 0032/0051 (Utf8View, decimales, `timestamptz`, field ids, la geo en WKB); el aislamiento por snapshot; la idempotencia | ¿qué significa exactamente una consulta en ORE? | DF14, DF15 |
| **3** | **el catálogo como planificador** | la resolución asíncrona por consulta con el snapshot fijado; ramas de ORE como refs de Iceberg; Views como `ViewTable`; el listado por un camino propio; la caché de metadatos entre consultas; a medio plazo, la poda de ficheros en Postgres por `planTableScan` | ¿cuánto del planificar le quitamos al motor? | DF2, DF5 |
| **4** | **el acceso** · el almacén y las credenciales | un object store por tabla con registro propio; la credencial como referencia firmada canjeada en `/v1` y renovada; una sola pila de E/S (`Storage` de iceberg-rust sobre object_store); el rendimiento contra GCS; las cachés de datos; el gobierno por construcción | ¿cómo llegan los bytes rápido sin que la credencial salga de donde se usa? | DF3, DF7 |
| **5** | **la lectura** | el escaneo Iceberg → DataFusion en N particiones por bytes y con estadísticas del manifiesto; los dos lectores (`ArrowReader` como oráculo, `ParquetSource` con `ParquetAccessPlan`); borrados por posición, igualdad y deletion vectors; `pushdown_filters`, filtros dinámicos y filtros `Inexact` | ¿cómo se lee al máximo sin una sola poda errónea? | DF4 |
| **6** | **la escritura y el commit** | la disposición de los ficheros (row groups por bytes, tamaño, orden por la clave caliente, bloom, field ids); escritura en paralelo con un solo commit por `/v1`; la clave de idempotencia fijada al planificar; append y CTAS, luego copy-on-write, luego MERGE; los huérfanos y la compactación | ¿cómo se escribe en paralelo sin que un reintento escriba dos veces? | DF6 |
| **7** | **ejecutar sin romperse** · memoria y fallos de un nodo | el pool jerárquico propio; el contrato público de operadores; el reintento con sort-merge; el derrame (volumen, compresión); la cancelación | ¿qué puede prometer ORE en memoria con el DataFusion de hoy? | DF10 |
| **8** | **distribuir** | Ballista frente a datafusion-distributed con codecs y nodos que no saben quién reparte; el umbral de un solo nodo; shuffle, AQE y broadcast; la seguridad del clúster; el job persistido en Postgres; el despliegue (pools dedicados, imagen única, protocolo exacto) | ¿cuándo se distribuye, con qué, y cómo sobrevive? | DF8, DF9, DF11, DF12 |

### 7.2 Las dos transversales

| | frente | qué abarca |
|---|---|---|
| **9** | **probar y ver** | la vara de §5 desde el primer día (`sqllogictest` con `configMatrix`, TPC-H/DS, la matriz de conformidad con la lista de Comet, el diferencial frente a DuckDB, el caos); `EXPLAIN ANALYZE` y las métricas guardadas por consulta. **Cada frente se cierra pasando su parte** |
| **10** | **aportar arriba** | lo que generalice va a DataFusion, Ballista o iceberg-rust: el proveedor Iceberg con particiones y estadísticas, `Storage` sobre object_store, el hash join y las ventanas que derraman, el reclamo desde el pool |

### 7.3 El orden

```
1 suelo ─► 2 contrato (escrito) ─► 3 catálogo ─► 4 acceso ─► 5 lectura ─► 6 escritura ─► 8 distribuir
                                                               └──── 7 memoria y fallos ────┘
9 probar y ver · 10 aportar arriba: desde el primer día
```

- **Del 1 al 6, en serie:** cada uno necesita el anterior para probarse de verdad.
- **El 7, junto al 5 y el 6:** son ellos quienes lo ponen a prueba, y lo que no derrama condiciona lo que el
  contrato puede prometer.
- **El 8, al final:** se distribuye lo que ya es correcto y robusto en un nodo. Así un fallo se puede
  atribuir al motor o al reparto.

Cada frente se trabaja igual: **un previo** que despeja lo desconocido, **la construcción**, y **la prueba**
que lo cierra.

### 7.4 Frente 1 · el suelo: **ORE Runtime**

**Qué es.** El suelo es al motor de ORE lo que el Databricks Runtime es a Databricks: **un runtime con
nombre y versión**. Es un paquete cerrado de DataFusion, Ballista e iceberg-rust más las extensiones de ORE,
del que nace igual cualquier ejecución de la plataforma: una consulta local, un scheduler, un ejecutor, una
prueba o un banco. Nadie elige el runtime, lo pone la plataforma; pero cada ejecución dice cuál la corrió.

**No es una pieza que haga cosas: es la placa base.** Define los zócalos donde encajan los demás frentes y
garantiza que todo lo que se enchufa en ellos corre siempre igual.

**Sus propiedades:**

1. **Una sola fuente de verdad.** Un crate del que sale todo, con dos objetos:
   - **el motor del proceso**: el `RuntimeEnv`, los dos runtimes de tokio, las cachés, la raíz de memoria
     y el registro de almacenes;
   - **la sesión de cada consulta**: la plantilla clonada más su contexto.

   Lo local, el scheduler y el ejecutor llaman a la misma función.
2. **Zócalos, no piezas.** El suelo declara los puntos de enchufe: catálogo, acceso, lectura, escritura,
   memoria, distribución y observación. Cada frente llena el suyo, y nadie construye una sesión por su
   cuenta.
3. **Un solo Arrow en todo el árbol.** El trío de versiones queda fijado, y una guarda del CI prohíbe que el
   grafo de dependencias tenga dos arrow-rs o dos DataFusion. Un suelo con un Arrow distinto al del resto
   del workspace ya sería dos piezas pegadas.
4. **La versión se ve.** El runtime sabe qué es y lo publica: su versión, la de DataFusion, la de Ballista,
   la rev de iceberg-rust y la de arrow-rs. Esa versión viaja en el protocolo (Ballista exige igualdad
   exacta) y queda en cada ejecución.
5. **La configuración es un contrato tipado:**
   - las opciones de ORE van como `ConfigExtension`;
   - hay un perfil por papel (local, scheduler, ejecutor, prueba);
   - cada valor que se aparta del upstream lleva escrito el porqué;
   - las claves desconocidas se rechazan;
   - la configuración de una consulta es inmutable y queda registrada.
6. **La ejecución no se ahoga:**
   - un runtime de E/S y otro de CPU, y el de CPU dimensionado a la cuota del cgroup, no a los núcleos del
     nodo;
   - la salud responde aunque la CPU esté saturada;
   - una consulta se cancela de verdad, y un pánico se queda en su consulta.
7. **Conoce sus límites.** El proceso lee del cgroup su memoria, su CPU y su disco, y con eso dimensiona la
   raíz de memoria (con su holgura) y el directorio de derrame. La jerarquía de pools es del frente 7.
8. **Actualizar es una rutina, no un proyecto.** Una vez por trimestre: la guía de cambios del upstream, el
   corpus de semántica propio, los bancos comparados con la versión anterior y un canario.
9. **Se prueba desde el primer día.** El suelo trae el arnés de `sqllogictest` sobre la sesión de ORE, con el
   corpus de DataFusion en verde (las excepciones, documentadas), y el arnés de bancos. Cada frente
   posterior llega con su prueba.
10. **Ligero y sin productos.** Clonar una sesión cuesta microsegundos, el proceso arranca rápido y hay una
    sola imagen para los tres papeles. No sabe de Query ni de puestos, y no lleva DuckDB.

**Cuándo está cerrado:**

- **Versiones:** el trío compila, hay cero arrow-rs o DataFusion duplicados en el grafo, y la versión del
  runtime se ve.
- **Pruebas:** el corpus `.slt` de DataFusion pasa sobre la sesión de ORE.
- **Medido en un pod de victor:**
  - el coste de clonar una sesión;
  - que `target_partitions` sale del cgroup;
  - que la salud responde con la CPU saturada.
- **Papeles:** local, scheduler y ejecutor nacen de la misma función y corren una consulta trivial.
- **Actualización:** un simulacro de 55.1 a 55.2.

#### 7.4.1 El previo S0: lo que se sabe

Medido y leído el 2026-10-30, con el detalle y las fuentes en
[`docs/investigacion/s0-ore-runtime.md`](../investigacion/s0-ore-runtime.md):

- **El trío compila junto, con un solo Arrow:**
  - arrow-rs y parquet 59.3.2, DataFusion 55.2.0 (Ballista 55.0 lo acepta), object_store 0.13.2 e
    iceberg-rust `main` en `4dd8c946`, que lleva #3483;
  - compila en glibc y en **musl**, el target de las imágenes: 14 y 36 min en release con 16 núcleos;
    `target` de 2 GB.
- **Subir todo el workspace al trío rompe 10 errores en dos crates:** 9 en ore-store, por la API de
  iceberg-rust, y 1 en ore-read-objetos, por `Geometry` de parquet 59. El resto compila sin cambios.
  sqlparser se queda en 0.63 en ore-core (DataFusion usa 0.62): bajarlo rompe `OrderBySort`.
- **El guardián de dependencias pasa 12/12** con el trío dentro del workspace, y prohíbe que `ore-serve`
  lleve el runtime: tiene que ser un binario propio.
- **El perfil `release` del workspace lleva `panic = "abort"`, y DataFusion vuelve a lanzar los pánicos de
  sus operadores:** con ese perfil, un pánico tumba el proceso entero.
- **musl sin mimalloc cuesta el doble:** clonar la sesión, 74 µs frente a 37 µs en glibc; construirla,
  350 µs frente a 190.
- **La cuota de CPU del cgroup se respeta y se redondea hacia abajo:** 1,5 CPU → `target_partitions` 1.
- **Ballista se arranca como librería** (`start_server`, `start_executor_process`, con sus ganchos), y
  **pierde en silencio** en el scheduler y el ejecutor las claves de configuración que no tiene
  registradas.
- **`datafusion-sqllogictest` 55.x está publicado**, con un driver sobre un `SessionContext` propio. Su
  corredor hay que copiarlo, y el corpus son 504 `.slt` con tres submódulos de datos.

#### 7.4.2 Cómo se construye

Seis pasos, en este orden. Cada uno cierra con su prueba antes del siguiente.

| | paso | qué se construye | dónde | se da por hecho cuando |
|---|---|---|---|---|
| **S1** | **el suelo común del workspace** | arrow y parquet 59 e iceberg-rust por rev en todo el workspace; arreglar los 10 errores de ore-store y ore-read-objetos; la guarda «un solo arrow-rs, parquet, DataFusion y object_store», con sqlparser como excepción nombrada; la toolchain fijada (≥ 1.95) | `Cargo.toml` de la raíz y de los crates, `crates/ore-store`, `crates/ore-read-objetos`, `crates/ore-cli/tests/dependencias.rs`, `rust-toolchain.toml` | `cargo check` del workspace; `cargo test -p` de ore-store, ore-read-objetos y ore-cli; las pruebas de huella; el CI de la rama en verde. **Un PR propio**, antes de tocar el runtime |
| **S2** | **el crate `ore-runtime`** | **el motor del proceso**: `RuntimeEnv`, los dos runtimes de tokio, límites leídos del cgroup, raíz de memoria con holgura y directorio de derrame. **La sesión**: plantilla más `sesion(contexto)`. **La configuración**: `ConfigExtension` `ore.*` tipada, con un perfil por papel. **El manifiesto de versión**. **Los zócalos**: traits vacíos para catálogo, acceso, lectura, escritura, memoria, distribución y observación | `crates/ore-runtime` (librería) | pruebas unitarias de cada propiedad; el coste de clonar medido |
| **S3** | **los tres papeles** | un binario, `ore-runtime local\|scheduler\|executor`, con Ballista como librería: `override_session_builder`, los `override_config_producer` y `override_runtime_producer`, todos desde la función de S2; Ballista sin features por defecto; mimalloc con musl; **un perfil `runtime` con `panic = "unwind"`** y cada consulta en su tarea | `crates/ore-runtime/src/bin` | en Docker, scheduler + ejecutor + cliente corren una consulta; una clave `ore.*` llega intacta a los tres; un pánico provocado vuelve como error y el proceso sigue |
| **S4** | **el arnés de la vara** | el corredor de `sqllogictest` sobre la sesión de ORE, copiado y adaptado del upstream; el corpus de la versión exacta con sus submódulos; la lista de excepciones, documentada | `crates/ore-runtime/tests` y un script de corpus en `ci/` | el corpus de DataFusion 55.2 en verde sobre la sesión de ORE |
| **S5** | **al CI y a la imagen** | el binario en `compilar-binarios.sh` con su perfil propio (como ya se hace con hbase); una etapa y una imagen `runtime` en el `Dockerfile`; las huellas; un job del CI para el arnés; **la compilación en frío medida en el CI** | `ci/`, `Dockerfile`, `.github/workflows/ci.yml` | CI en verde e imagen publicada |
| **S6** | **medido en victor** | un pod en un pool dedicado: el coste de clonar la sesión con mimalloc, `target_partitions` frente a la cuota, la salud con la CPU saturada; el simulacro de actualización (subir la rev de iceberg-rust y una patch de DataFusion) | un pool dedicado de victor, nunca el nodo de sistema | las cifras en el ADR: **el suelo cerrado** |

**Estado de S1 (2026-10-30, hecho en local).** Todo el workspace va con arrow-rs y parquet 59 e
iceberg-rust en `4dd8c946`; la toolchain, fijada en 1.99.

- **Los arreglos:**
  - ore-store: `Table::manifest_reader().read(mf)`, que conserva la herencia de secuencias del manifiesto;
    `MetadataLocation::try_*_with_metadata`; `FileWrite::close` devuelve su `FileMetadata`;
  - ore-read-objetos: `LogicalType::Geometry(GeometryType)`.
- **La guarda nueva:** `el_motor_va_con_una_sola_version`, en `crates/ore-cli/tests/dependencias.rs`, con
  sqlparser como única excepción (D-S1).
- **El aviso de la lente geo (0067 G2·4·1) saltó:** la rev ya abre los v3 con `geometry`. **La lente se
  queda**, porque quitarla es decisión de la geo (0067, DF15), y la prueba registra el hecho nuevo.
- **Pasado en Docker:**
  - `cargo check --workspace` y `cargo fmt --all --check`;
  - clippy de los crates con código cambiado;
  - las pruebas de los 18 crates tocados;
  - las tres pruebas de huella.
- **Fuera de S1:** en Docker, `ore-phoenix --test trazas` falla
  `cancelar_cierra_el_statement…` igual con la base sin cambios (4/4), y la guarda de fin de línea no puede
  llamar a git dentro del contenedor.
- **El CI:** en verde, en el PR describeloai/ore#87 y en `main` (§7.4.6).

**Decidido (2026-10-30):**

- **D-S1 · sqlparser:** dos versiones en el grafo, 0.63 en ore-core y 0.62 en DataFusion, hasta que los
  productos vuelvan a nacer. La guarda lo nombra como la única excepción.
- **D-S2 · la toolchain:** fijada en `channel = "1.99"`. Obliga a recompilar en frío en el CI una vez.
- **D-S3 · el nombre:** `ore-runtime` para el crate y el binario; ORE Runtime para el producto.
- **D-S4 · la imagen:** `scratch`, como `ore` e `iam`. Se revisa en S5 si operarla pide un shell.

#### 7.4.3 S2 · el diseño del crate `ore-runtime`

**Qué es S2.** La librería del suelo: el motor del proceso, la sesión de cada consulta, la configuración, los
límites, la versión y los zócalos. **No trae** Ballista (S3), el arnés de `sqllogictest` (S4), el CI ni la
imagen (S5), ni el contenido de ningún otro frente: catálogo, almacenes y lectura llegan después por sus
zócalos. Las APIs en que se apoya están comprobadas en el código fuente de DataFusion 55.2.

**Las piezas:**

| módulo | qué es | su contrato |
|---|---|---|
| `manifiesto` | **qué runtime es** | `Manifiesto::actual()`: ORE Runtime (`CARGO_PKG_VERSION`), DataFusion (`DATAFUSION_VERSION`), arrow-rs, la rev de iceberg-rust y el commit (`option_env!("ORE_COMMIT")`, como `ore`). Una prueba compara las versiones declaradas con `Cargo.lock`, así que una subida no puede dejar el manifiesto mintiendo |
| `limites` | **lo que el proceso tiene** | `Limites::leer()`: memoria (`memory.max`), CPU (`cpu.max`) y disco del directorio de derrame, de cgroup v2 con v1 y el host como respaldo. Los analizadores son funciones puras, probadas con ficheros fijos. La CPU sale **entera y nunca 0** (como std, pero dicho) |
| `opciones` | **la configuración como contrato** | `OreOpciones`, una `ConfigExtension` con prefijo `ore` (`extensions_options!`): holgura de memoria, directorio y tope de derrame. `DESVIOS`, la tabla `(clave, valor, porqué)` de lo que se aparta de DataFusion; de entrada `target_partitions` explícito y `spill_compression = lz4_frame`, y `pushdown_filters` espera a la lectura. `config_de(papel, &limites, &opciones)`, pura: un perfil por `Papel` (local, scheduler, ejecutor, prueba). Las claves pasan **siempre** por `options_mut().set()`, que devuelve error; nunca por `set_str`, que hace pánico |
| `motor` | **el motor del proceso**, uno por proceso | `Motor::arrancar(papel, opciones, zocalos)`: lee los límites, construye el `RuntimeEnv` (raíz de memoria del zócalo, sobre el límite menos la holgura; `DiskManager` en el directorio de derrame con su tope) y **los dos runtimes de tokio**: E/S (pocos hilos, red y salud) y CPU (los hilos de `limites.cpu`, sin E/S). Construye una vez la **plantilla** de `SessionState` |
| `sesion` | **la sesión de cada consulta** | `Motor::sesion(&Contexto) -> SessionState`: clona la plantilla, que comparte el `RuntimeEnv`, y aplica sólo lo que el contexto puede cambiar (una lista cerrada de claves) más el id de la consulta. Después, inmutable |
| `correr` | **ejecutar sin ahogar ni caer** | `Motor::correr(sesion, plan) -> Consulta`: el plan corre en el runtime de CPU dentro de una `SpawnedTask`, y entrega sus lotes por un canal acotado a quien lo pide desde cualquier runtime. **Un mecanismo da tres propiedades**: la CPU nunca bloquea la E/S ni la salud; un pánico llega como `Fallo::Panico` y el proceso sigue; soltar la `Consulta` cancela la tarea |
| `zocalos` | **los puntos de enchufe** | `Zocalos { catalogo, almacenes, memoria, extensiones, observacion }`, con implementaciones por defecto que no hacen nada |
| `fallo` | los errores | `Fallo`: `Config`, `Limites`, `Ejecucion(DataFusionError)`, `Panico { mensaje }`, `Cancelada` |

**Los zócalos, y quién llena cada uno.** Son los mínimos que el suelo necesita llamar, y ninguno más. La
lectura y la escritura no son zócalos: llegan como `TableProvider` a través del catálogo, y como reglas y
planificadores a través de las extensiones.

| zócalo | forma | lo llena |
|---|---|---|
| `Catalogo` | `async fn preparar(&self, estado: &mut SessionState, referencias: &[TableReference])`: resolver antes de planificar, el patrón de catálogo remoto de DataFusion | frente 3 |
| `Almacenes` | el `ObjectStoreRegistry` del `RuntimeEnv` | frente 4 |
| `Memoria` | `fn raiz(&self, limite: usize) -> Arc<dyn MemoryPool>`; por defecto `TrackConsumersPool<GreedyMemoryPool>` | frente 7 |
| `Extensiones` | funciones, reglas del analizador y del optimizador, `QueryPlanner` | frentes 2, 5 y 6 |
| `Observacion` | `al_empezar(&Ficha)` y `al_terminar(&Ficha, &Metricas)`: la ficha lleva id, manifiesto, configuración y plan | frente 9 |

La distribución no es un zócalo de S2: es S3 (los papeles) y el frente 8.

**Las decisiones de diseño, y por qué:**

- **Dos objetos, nunca uno.** El motor vive lo que el proceso; la sesión, lo que la consulta. Mezclarlos
  es lo que hace que las cachés no sirvan (se pierden con cada consulta) o que la configuración se filtre
  entre consultas.
- **La configuración es una función pura del papel, los límites y las opciones.** Se prueba sin arrancar
  nada, y es la misma en los tres papeles: así lo exige Ballista, que pierde en silencio las claves que un
  papel no conoce.
- **Cada desvío tiene una prueba.** Una prueba comprueba que cada clave de `DESVIOS` sigue existiendo en
  DataFusion. Si una actualización la renombra, se sabe en la prueba, no en producción.
- **`correr` es la única entrada para ejecutar.** Nadie llama a `collect` sobre el runtime equivocado.
  El precio es un canal por consulta, despreciable frente a la E/S.
- **Los zócalos se definen por lo que el suelo llama, no por lo que harán los frentes.** Cada frente
  podrá ensanchar su zócalo, pero no añadir uno sin pasar por el suelo.

**El orden de construcción.** Cada paso cierra con sus pruebas antes del siguiente.

| | paso | se da por hecho cuando |
|---|---|---|
| **S2·1** | el crate (`crates/ore-runtime`, DataFusion 55 con sus features por defecto) y el **manifiesto** | el manifiesto coincide con `Cargo.lock`; el guardián sabe que ni `ore` ni `ore-serve` alcanzan DataFusion |
| **S2·2** | los **límites** | los analizadores con ficheros fijos (v2, v1, `max`, cuota fraccionaria); en Docker con `--cpus` y `-m`, lo leído coincide |
| **S2·3** | las **opciones** y el perfil por papel | cada desvío existe en DataFusion; una clave desconocida es error, no pánico; los cuatro papeles dan configuraciones distintas donde deben y la misma donde deben |
| **S2·4** | el **motor**: `RuntimeEnv`, memoria, disco y los dos runtimes | una consulta agota la raíz de memoria y falla con `ResourcesExhausted` nombrando a sus consumidores; un sort grande derrama en el directorio dado |
| **S2·5** | la **plantilla y la sesión** | dos sesiones comparten el `RuntimeEnv` y no la configuración; el contexto sólo cambia las claves permitidas; el coste de clonar, medido y escrito |
| **S2·6** | **`correr`** | con la CPU saturada por una consulta, un latido en el runtime de E/S responde en < 50 ms; el pánico de una UDF vuelve como `Fallo::Panico`; soltar la consulta la cancela |
| **S2·7** | los **zócalos y la observación** | un catálogo de prueba se llama en `preparar`; la observación recibe la ficha con manifiesto, configuración y plan |

**Estado (2026-10-30): S2·1 a S2·4, hechos en local.**

- El crate `crates/ore-runtime` tiene los módulos `manifiesto`, `limites`, `opciones`, `motor`, `zocalos` (por
  ahora sólo el de memoria) y `fallo`.
- **Pruebas:** 19 en verde y 1 ignorada, que corre a mano en un contenedor con límites. Clippy y fmt,
  limpios.
- **El guardián:** con la regla nueva `el_motor_no_entra_en_ore_ni_en_ore_serve` pasa 14/14.
- **Lo que se probó:**
  - el manifiesto coincide con `Cargo.lock`;
  - en Docker con `--cpus 2 -m 1g` y `--cpus 3 -m 2g`, los límites leídos coinciden exactamente, y los dos
    salen de cgroup v2;
  - un `array_agg` sobre 27 MB de raíz falla con `Resources exhausted` y nombra a sus consumidores;
  - un sort de 6 M filas sobre 54 MB derrama en el directorio dado, y con 1 MiB de tope el error lo dice;
  - cada desvío existe en DataFusion, y una clave desconocida es un error, nunca un pánico;
  - los tres papeles de una consulta dan la misma configuración, y `Prueba` sólo cambia las particiones.
- **Hallazgo:** DataFusion 55 lista las claves de una `ConfigExtension` **sin su prefijo** (`holgura_memoria`
  en lugar de `ore.holgura_memoria`), aunque para fijarlas haya que escribirlo. `opciones::entradas` da la
  foto canónica, con cada clave en su espacio, que se puede volver a aplicar. Es candidato a aportarse arriba
  (frente 10).

**Estado (2026-10-30): S2·5 a S2·7, hechos en local. S2 completo.**

- **Los módulos nuevos:** `sesion`, `correr` y los cinco zócalos.
- **Pruebas:** 29 en verde, tres veces seguidas (19 de la librería y 10 de punta a punta, en
  `tests/suelo.rs`), y 2 ignoradas, que son medidas manuales.
- **Lo que se probó:**
  - dos sesiones comparten el `RuntimeEnv` y no la configuración;
  - el contexto sólo cambia `batch_size` y `time_zone`;
  - `SET` y `create table` son error;
  - con la CPU llena por dos consultas pesadas, un latido de 20 ms en el runtime de E/S nunca se retrasa
    50 ms;
  - el pánico de una UDF vuelve como `Fallo::Panico`, y la consulta siguiente corre;
  - soltar la consulta la cancela, la observación lo dice y la CPU queda libre;
  - el catálogo resuelve cada consulta sin que la plantilla gane la tabla;
  - la observación recibe la ficha (id, manifiesto, foto de configuración, plan) y el resultado con sus
    métricas;
  - el registro de almacenes es el del zócalo.
- **Un ajuste de diseño:** `Catalogo::preparar` **devuelve** la lista de catálogos de la consulta en vez de
  registrar en la de la plantilla. Esa lista la comparten todas las copias de la plantilla, y registrar en
  ella filtraría tablas entre consultas.
- **Hallazgo:** `SessionStateBuilder::new_from_existing` **no conserva el id de la sesión**: le da un UUID
  nuevo. `Sesion::planificar` lo arrastra a mano. Es otro candidato a aportarse arriba.
- **El coste de abrir una sesión, medido** (release, Docker local con glibc, sin mimalloc; la cifra de prod es
  de S6):
  - **abrir una sesión: 37 µs**, que es un clon de la plantilla y nada más;
  - **abrir y planificar `select 1`: 339 µs**, con la única reconstrucción del estado (~160 µs) al instalar
    los catálogos de la consulta;
  - la primera versión reconstruía el estado dos veces (al abrir, por el id, y al planificar) y abrir
    costaba **201 µs**. Ahora el id se pone en la misma reconstrucción que los catálogos.

**Al cerrar S2:** `cargo test -p ore-runtime`, clippy, fmt, el guardián y las huellas en verde. Todavía no
hay binario.

**Lo que S2 cuesta y no se ve:** DataFusion entra en el `cargo test --workspace` del CI y alarga el job
`suite`. Se mide en S5. Desde `[profile.dev.package."*"]` las dependencias ya compilan sin depuración.

#### 7.4.4 S3 · el diseño de los tres papeles

**Qué es S3.** Ballista entra en el suelo **como librería**, y el suelo gana su binario: `ore-runtime local`,
`ore-runtime scheduler` y `ore-runtime executor`. Los tres nacen del mismo `Motor` de S2. **No trae** la salud
por HTTP ni la imagen (S5), el arnés (S4), el estado del scheduler fuera de memoria, la autenticación ni TLS
(frentes 4 y 8), KEDA ni Flight SQL. Todo lo que sigue está leído en el código publicado de Ballista 55.0.0.

**Lo que el código de Ballista obliga a decidir:**

| hecho (Ballista 55.0.0) | consecuencia para el suelo |
|---|---|
| El scheduler pide un `SessionBuilder` (`Fn(SessionConfig) -> SessionState`) y un `ConfigProducer`; el ejecutor, un `ConfigProducer`, un `RuntimeProducer` y un `BallistaFunctionRegistry` | los cuatro salen del `Motor`: la configuración de `config_de`, el estado de la plantilla con la configuración que llega, el `RuntimeEnv` del proceso y las funciones de la plantilla (`BallistaFunctionRegistry::from(&SessionState)`). **El registro por defecto sólo trae las funciones de DataFusion**: sin esto, una UDF de ORE no existe en el ejecutor |
| El scheduler y el ejecutor aplican las claves del cliente con `update_from_key_value_pair`, que **pierde en silencio** las que no conoce | el `ConfigProducer` de los tres papeles es el mismo y lleva `OreOpciones` y `BallistaConfig` registradas; una prueba lo comprueba de punta a punta |
| El ejecutor corre cada tarea en **su propio pool de CPU** (`DedicatedExecutor`, `vcores` hilos) y la envuelve en `catch_unwind` | en el ejecutor, ese pool **es** el runtime de CPU del suelo, con `vcores = limites.cpu`, y el `Motor` no levanta otro (dos pools de CPU se pisarían). `catch_unwind` sólo sirve con `panic = "unwind"` |
| El ejecutor reemplaza la memoria de cada tarea por un `FairSpillPool` del 70 % de lo que detecta, repartido por `vcores`, salvo con `memory_pool_size = Some(0)`, que deja el `RuntimeEnv` tal cual | `memory_pool_size = Some(0)`: **la raíz de memoria es la del zócalo**, con su holgura, en los tres papeles. Repartirla entre tareas es la jerarquía del frente 7, que lo hará en el zócalo, no Ballista por su cuenta |
| El cliente distribuido es un `QueryPlanner` (`upgrade_for_ballista(url)`), y el plan que corre en el cliente sólo **espera y reenvía** lotes por gRPC y Flight | `distribuir` es una propiedad de la plantilla, no un papel: con `ore.scheduler` puesto, la plantilla lleva el planificador de Ballista y `correr` funciona igual. Pero ese plan **hace red**, y el runtime de CPU no tiene E/S: en una sesión distribuida, `correr` lo lanza en el runtime de E/S |
| `start_executor_process` y `start_server` hacen `tokio::spawn` en el runtime que los llama | se llaman desde el runtime de E/S del `Motor`; el de E/S de cada papel se dimensiona por papel (el scheduler planifica ahí) |
| Las features por defecto traen clap, tracing, axum, **mimalloc** (ejecutor), la API REST y `ballista-history` (scheduler) | `default-features = false` en los tres crates, con `arrow-ipc-optimizations` a mano. mimalloc lo pone el binario, como los demás de ORE |
| Ballista escribe con el facade `log` | el binario trae un `log::Log` mínimo a stderr, sin dependencias nuevas |

**Las piezas:**

| dónde | qué es | su contrato |
|---|---|---|
| `opciones` | **`ore.scheduler`** y la configuración de Ballista | `ore.scheduler: Option<String>`. `config_de` añade `BallistaConfig` a **todos** los papeles con las restricciones de Ballista (`round_robin_repartition = false`, `schema_force_view_types = false`), cada una escrita en `DESVIOS` con su porqué |
| `motor` | **el runtime de CPU, por papel** | local y prueba: el de S2. Scheduler: ninguno. Ejecutor: ninguno, porque lo pone Ballista con `limites.cpu` hilos. `Motor::correr` en un papel sin runtime de CPU es `Fallo::Config` |
| `sesion` / `correr` | **la sesión distribuida** | con `ore.scheduler`, la plantilla nace con `upgrade_for_ballista(url)` y `correr` va al runtime de E/S. La `Ficha` lleva el scheduler |
| `papeles` (nuevo) | **el scheduler y el ejecutor como funciones** | `papeles::scheduler(&Motor, Escucha) -> Result<(), Fallo>` y `papeles::ejecutor(&Motor, Escucha, scheduler) -> Result<(), Fallo>`. Llenan los ganchos de Ballista desde el `Motor` y no aceptan otros. Terminan con SIGTERM |
| `manifiesto` | **la versión de Ballista** | `BALLISTA` y `BALLISTA_PROTOCOL_VERSION`, comparados con `Cargo.lock` como las demás |
| `src/bin/ore-runtime.rs` | **el binario, fino** | lee los argumentos (clap del workspace), pone mimalloc con musl, el log y las señales, arranca el `Motor` del papel y llama a `papeles`. `ore-runtime --version` imprime el manifiesto. **`compile_error!` si se compila con `panic = "abort"`**: el perfil equivocado no llega a dar un binario |
| `Cargo.toml` de la raíz | **el perfil `runtime`** | `inherits = "imagen"` (sin LTO, 16 unidades) y `panic = "unwind"`. `compilar-binarios.sh` lo usa en S5, como ya hace con hbase |

**Decisiones que pido** (las cuatro recomiendan lo de la tabla):

- **D-S5 · la CPU por papel:** el ejecutor usa el pool de Ballista, a `limites.cpu`; el `Motor` no levanta otro
  en el scheduler ni en el ejecutor.
- **D-S6 · la memoria del ejecutor:** la raíz del zócalo, sin el reparto por tarea de Ballista
  (`memory_pool_size = 0`), hasta el frente 7. Hasta entonces, una tarea puede quedarse la raíz entera, que es
  lo mismo que pasa en local.
- **D-S7 · distribuir es una opción, no un papel:** `ore-runtime local --scheduler <url>` es el cliente, y el
  binario sigue teniendo tres papeles.
- **D-S8 · el perfil `runtime`:** hereda de `imagen`, sin LTO. LTO con musl ya cuesta 36 min en S0; se mide en
  S5 si compensa.

**El orden de construcción.** Cada paso cierra con sus pruebas antes del siguiente.

| | paso | se da por hecho cuando |
|---|---|---|
| **S3·1** | **Ballista en el árbol**: los tres crates sin features por defecto, la versión en el manifiesto, el perfil `runtime` | un solo arrow-rs, DataFusion, tonic y prost en el grafo; ni mimalloc ni axum por Ballista; el guardián, con Ballista en `UNA_SOLA_VERSION`, en verde; el manifiesto coincide con `Cargo.lock` |
| **S3·2** | **la configuración de Ballista** en `config_de` y `ore.scheduler` | los tres papeles dan la misma configuración, con `ballista.*`; cada restricción de Ballista está en `DESVIOS`; las 29 pruebas de S2 siguen en verde |
| **S3·3** | **`papeles`**: scheduler y ejecutor desde el `Motor`, y la sesión distribuida | **un clúster en el proceso de la prueba** (scheduler, un ejecutor y un cliente, cada uno con su `Motor`): un `group by` sobre Parquet baraja entre etapas y da el resultado de local; una UDF de prueba `opcion('ore.holgura_memoria')`, que **sólo** está en las extensiones, corre en el ejecutor y devuelve el valor que puso el cliente; el pánico de una UDF vuelve como error y la consulta siguiente corre; soltar la consulta la cancela en el scheduler |
| **S3·4** | **el binario** | en Docker, tres contenedores (scheduler, ejecutor y `local --scheduler`) corren una consulta; `--version` da el manifiesto; SIGTERM los cierra limpios; compilado con `panic = "abort"`, no compila. **Con musl y mimalloc**, abrir una sesión cuesta lo que en glibc (37 µs ± 20 %) |

**Decidido (2026-10-30):** D-S5 a D-S8, como se proponen.

**Estado (2026-10-30): S3·1 y S3·2, hechos en local.**

- **Ballista en el árbol:** `ballista-core`, `-scheduler` y `-executor` 55.0.0 sin features por defecto. Ni
  mimalloc ni clap entran por Ballista. **axum sí**: es una dependencia fija del scheduler, que no quita ninguna
  feature. Es la misma versión que ya usa el workspace (0.8.9), así que el grafo no gana una copia.
- **El guardián** (14/14) cuenta ahora también `ballista`, `tonic` y `prost` entre las familias de una sola
  versión.
- **El manifiesto** dice Ballista y su protocolo (55.0.0, 3), comparados con `Cargo.lock`.
- **El perfil `runtime`**: `imagen` con `panic = "unwind"`.
- **Un ajuste de diseño:** las restricciones de Ballista (`DESVIOS_DISTRIBUIDOS`, ocho claves con su porqué)
  **sólo se aplican a lo distribuido**. El scheduler y el ejecutor las llevan siempre; un proceso local, sólo
  con `ore.scheduler`. Una consulta local conserva el hash join, los filtros dinámicos y `Utf8View`.
  `BallistaConfig` se registra en todos los papeles, así que ninguna clave `ballista.*` se pierde.
- **Lo que cazó la prueba de las restricciones:** `enable_dynamic_filter_pushdown` manda sobre las de join,
  agregado y top-k. Por eso la prueba compara el **efecto** de las restricciones escritas con el de
  `upgrade_for_ballista`, clave a clave, y no la lista de claves.
- **Pruebas:** 21 de la librería y 10 de punta a punta, en verde dos veces seguidas. Clippy, fmt y las tres
  huellas, limpios.

**Estado (2026-10-30): S3·3 y S3·4, hechos en local. S3 completo.**

- **Lo nuevo:**
  - el módulo `papeles` (`scheduler` y `ejecutor`, que llenan los ganchos de Ballista desde el `Motor`);
  - `registro`, el `log` del proceso a stderr;
  - el binario `src/bin/ore-runtime.rs`.
- **El `Motor` por papel:** levantan runtime de CPU local y prueba, y nadie más. El cliente distribuido corre
  en el de E/S. Sólo el cliente lleva el planificador de Ballista. Las extensiones se aplican al final, porque
  lo de Ballista reemplaza la lista de funciones.
- **Pruebas:** 24 de la librería, 4 del clúster (`tests/papeles.rs`) y 10 del suelo. Clippy y fmt, limpios.
  El clúster corre dentro del proceso de la prueba: un scheduler, un ejecutor y un cliente, cada uno con su
  `Motor`, y demuestra cuatro cosas:
  - un `group by` con barajado da lo mismo que en local;
  - una UDF que sólo está en las extensiones corre en el ejecutor (hilo `task_runner`) y ve el
    `ore.holgura_memoria` del cliente;
  - un pánico vuelve como error y el clúster sigue;
  - soltar la consulta cancela su trabajo en el scheduler.
- **En Docker, tres contenedores con el mismo binario** (scheduler; ejecutor con `--cpus 2 -m 2g`;
  `local --scheduler`):
  - un millón de filas en Parquet, agrupadas, dan el resultado de local con 17 tareas en el ejecutor;
  - el ejecutor lee del cgroup 2 CPU y una raíz de 1740 MiB;
  - con SIGTERM, los dos terminan limpios, con código 0, en ~2 s;
  - `--version` da el manifiesto;
  - **compilado con `panic = "abort"`, no compila**: el `compile_error!` lo explica.
- **El binario:** 152 MB en glibc con el perfil `runtime`; 27 min de compilación en frío (16 núcleos, a la vez
  que la de musl).
- **Con musl y mimalloc, abrir una sesión cuesta 19,6 µs**, y abrir y planificar `select 1`, 235 µs. Es menos
  que en glibc (37 µs, aunque con `--release` y LTO). Sin mimalloc, en S0, eran 74 µs. Compilar para musl en
  frío llevó 54 min.

**Lo que S3 encontró:**

1. **Las opciones de ORE se perdían en el ejecutor.** Ballista manda a los ejecutores lo que lista
   `entries()`, y una extensión hecha con `extensions_options!` lista sus claves **sin prefijo**: el ejecutor
   no las sabe fijar y las tira en silencio. `OreOpciones` implementa `ExtensionOptions` a mano, con el
   prefijo, como `BallistaConfig`, y una prueba hace el viaje de ida y vuelta. Es el hallazgo de S2·3 con su
   consecuencia a la vista: candidato a aportarse arriba (frente 10).
2. **Ballista 55 no cancela el trabajo al soltar el stream de `DistributedQueryExec`**, y el clúster lo
   seguiría calculando. El suelo lo cancela: la guarda de `correr` pide `cancel_job` al scheduler, y la
   observación dice qué trabajo canceló. En modo push el cliente sólo conoce el id del trabajo con el primer
   estado, que en un trabajo largo tarda. Por eso `ballista.client.pull = true` está en `DESVIOS`, con su
   porqué, aunque cuesta un sondeo cada 50 ms. Candidato a aportarse arriba.
3. **Después de cancelar, el clúster tarda en soltar la CPU (Ballista 55, medido):**
   - el aborto es cooperativo: una tarea de CPU (un sort-merge join) no lo ve hasta que su stream cede, y
     aquí fueron **39 s**;
   - el scheduler pierde el estado de la tarea cancelada («Fail to find job … in the active cache»), y el
     trabajo siguiente, de tres etapas de milisegundos, tardó **21 s**.

   Son de los frentes 7, 8 y 10. La prueba del suelo comprueba lo que el suelo garantiza: el trabajo queda
   cancelado en el scheduler.
4. **El scheduler lista los ficheros al planificar.** Sin acceso al almacenamiento, la consulta dio **un
   resultado vacío, sin error**. Lo de almacenes (frente 4) tiene que llegar también al scheduler, y que un
   escaneo sin ficheros sea un error es una garantía que falta. Queda en los riesgos (§11).
5. **Las restricciones de Ballista sólo para lo distribuido** (S3·2) y **las extensiones al final** (S3·3):
   las dos se ven en la plantilla, cada una con su prueba.

**Al cerrar S3:** `cargo test -p ore-runtime`, clippy, fmt, el guardián y las huellas en verde, y un binario
que corre en Docker. Lo que queda para S6 es medirlo en victor.

**Lo que puede salir mal, y cómo se ve:**

- **El runtime de E/S se queda corto en el ejecutor**, que sirve el barajado por Flight con dos hilos. Se ve en
  S6 como latencia de barajado; el número de hilos por papel ya es una opción.
- **Una UDF de ORE sin codec**: las funciones viajan por nombre y resuelven en el registro del ejecutor, pero
  un `TableProvider` o un nodo propio necesita su codec. Hoy no hay ninguno; el primero que llegue (frentes 3 y
  5) trae su codec y su prueba en el clúster de S3·3.

#### 7.4.5 S4 · la vara: resultados

**Hecho en local (2026-10-30). El corpus de DataFusion 55.2 pasa sobre la sesión de ORE.**

**Decidido:**
- **D-S9 · qué mide:** la configuración y el `RuntimeEnv` de un `Motor` de papel `Prueba`, sobre la
  preparación por fichero del upstream. La política de la `Sesion` (sin DDL ni `SET`) queda fuera: el corpus
  es DDL.
- **D-S10 · depender o copiar:** `datafusion-sqllogictest =55.2.0` como dev-dependencia. Del upstream sólo se
  copia el bucle por fichero.
- **D-S11 · el corpus:** `test_files/` con arrow-testing y parquet-testing.
- **D-S12 · las excepciones caducan:** cada una con su categoría y su porqué, y una que sobra hace fallar la
  vara.

**Las piezas:**

| | dónde |
|---|---|
| el corpus, en el tag que dice `Cargo.lock` (12 s, 97 MB) | `ci/corpus-slt.sh <dir>` |
| el corredor (`harness = false`); sin `ORE_CORPUS_SLT`, no corre | `crates/ore-runtime/tests/vara.rs` |
| las excepciones | `crates/ore-runtime/tests/excepciones-de-la-vara.txt` |

**El resultado:**

- **257 ficheros, 36 018 registros, en 32 s** (16 núcleos, en Docker). Corren 256 en verde y 1 con
  excepciones. Uno más lo salta el upstream: `avro.slt`, que pide una feature que no traemos.
- **Lo que no entra en la cuenta de los 504:** 244 de spark (otro dialecto), el de tpch y el de Parquet
  cifrado. Es lo que el upstream deja fuera por defecto, más spark.
- **Dos excepciones, las dos `desvio`:** los dos `SHOW ALL` de `information_schema.slt` enseñan la
  configuración de ORE:
  - `spill_compression = lz4_frame`;
  - la raíz de memoria y el directorio de derrame del motor;
  - las claves `ore.*` y `ballista.*`.
- **Ninguna divergencia.** Fuera de esos dos registros, la sesión de ORE da lo mismo que DataFusion.
- **Dos veces seguidas en verde.** Una excepción de más hace fallar la vara con `CADUCADA`.

**Lo que encontró:**

1. **`new_from_existing(..).with_config(..)` pisa el catálogo por defecto.** `new_from_existing` apaga
   `create_default_catalog_and_schema` porque el catálogo ya existe; `with_config` lo vuelve a encender, y al
   construir se crea uno vacío encima del que había.
   - **En la vara:** borraba las tablas de cada fichero.
   - **En `papeles::scheduler`:** pasaba igual, en silencio, y el catálogo pisado era el de la lista de la
     plantilla, que comparten todas las sesiones, en cada sesión que abría el scheduler.

   Los dos llaman ya a `with_create_default_catalog_and_schema(false)`. Es una trampa de la API, candidata a
   aportarse arriba.
2. **El upstream calcula `WORKSPACE_ROOT` desde donde se compiló su crate,** que aquí es el registro de
   cargo, no el corpus. Sin corregirlo, fallan 588 registros de `EXPLAIN` que sólo difieren en la ruta. La vara
   cambia la raíz del corpus en lo que sale antes de validar. Es candidato a aportarse arriba: una variable de
   entorno para la raíz.
3. **El cierre de `ore-cli` en el guardián pasa de 193 a 202.** `datafusion-sqllogictest` pide clap con
   colores, y el lock apunta esas aristas en el `clap` de todos. `cargo tree -p ore-cli` no las compila. Está
   escrito en el guardián, con su porqué.

**Lo que queda para S5:** el job del CI, que trae el corpus (con caché) y corre la vara.

#### 7.4.6 S5 · al CI y a la imagen

**Cableado (2026-10-30). Las medidas salen del primer CI.**

- **`ci/compilar-binarios.sh`:** `ore-runtime` se compila aparte, como el dinámico de hbase, con su perfil
  `runtime` en `$OBJETIVO/runtime`. cargo-chef cocina sus dependencias en ese perfil, y la puerta exige que
  `ore-runtime --version` salga bien.
- **El Dockerfile:** la imagen `ore-runtime` es `scratch` (D-S4), con el usuario 65532 y un `/tmp` en el que
  se puede escribir. Ahí derrama DataFusion y baraja el ejecutor si no se les da otro sitio; en producción, un
  volumen montado encima.
- **Las huellas:** la imagen está en el mapa del CI y el binario se declara en su `Cargo.toml`. Las tres
  pruebas de huella están en verde; `ore-runtime` es de los que no dependen de `ore-base`.
- **El CI:**
  - el job `vara` trae el corpus con caché y corre la vara, y entra en `puerta`;
  - `ore-runtime:main` se etiqueta con las demás, para S6.
- **Medido en el CI (2026-10-30), todo en verde:**

  | job | en frío (el PR #87 y la fusión) | en caliente (#89 en `main`) |
  |---|---|---|
  | `suite` | 13 min (PR) · 11,5 min (`main`) | **3,7 min**, lo mismo que antes de DataFusion |
  | `vara` | 6,5 min (la vara corre en 62 s) | **2 min** |
  | `construir` | **47,5 min**: las 37 imágenes con la toolchain y el lock nuevos, y DataFusion y Ballista | **8 min** |
  | `imagen` | — | 3,3 min, y despliega |

  El frío lo pagan una vez: el cambio de toolchain o de `Cargo.lock`. Después, la caché.
- **Lo desplegado:** las cargas de ORE de t-victor y t-demo corren `083dbbc5`, con arrow 59 dentro, y están en
  1/1.

#### 7.4.7 S6 · medido en victor: el suelo, cerrado

**Hecho el 2026-10-30.** `ore-runtime diagnostico` es el propio binario midiéndose en su pod. Corrió en dos
Jobs del pool `pesado-driver` (n2-highmem-4, no spot; nunca el de sistema), con `ore-runtime:main` fijada por su
digest (`sha256:8044f955…`). El guion es `pruebas-de-fuego/runtime/s6.sh`, y todo, en 85 s.

| | cuota de 3 CPU y 8 GiB | cuota de 1,5 CPU y 4 GiB |
|---|---|---|
| límites que lee (cgroup v2) | 3 CPU · 8192 MiB | **1 CPU** (redondeado hacia abajo) · 4096 MiB |
| `target_partitions` | **3** (el nodo tiene 4 núcleos) | **1** |
| raíz de memoria (menos el 15 %) | 6963 MiB | 3481 MiB |
| abrir una sesión | **26,4 µs** | **23,6 µs** |
| abrir y planificar `select 1` | 688 µs | 464 µs |
| latido de 20 ms en E/S, con una consulta pesada por hilo de CPU | peor **6,2 ms**, media 1,4 ms | peor **2,9 ms**, media 1,2 ms |

**Lo que dicen:**

- **La sesión, con musl y mimalloc, cuesta ~25 µs**, lo mismo que en local (19,6). Sin mimalloc, en S0, eran
  74 µs.
- **Las particiones salen de la cuota, no del nodo**, y una cuota fraccionaria se redondea hacia abajo, como
  decidió S0.
- **La salud no se ahoga:** con la CPU llena, el runtime de E/S se retrasa como mucho 6 ms.
- **Planificar `select 1` cuesta 0,5–0,7 ms** en un nodo de producción, frente a ~0,3 en local. Es la
  reconstrucción del estado, una vez por consulta (S2·5).
- **El manifiesto dice «sin sellar».** El CI no sella el commit en ningún binario: su caché va por contenido, y
  sellarlo la invalidaría en cada commit. El commit desplegado lo dice la anotación `ore.dev/commit` del pod.

**El suelo está cerrado** (§7.4, «cuándo está cerrado»):

- **Versiones:** el trío compila, el grafo no tiene duplicados del motor y la versión se ve.
- **Pruebas:** la vara pasa en el CI.
- **Medido en victor:** el coste de la sesión, las particiones que salen del cgroup y la salud con la CPU llena.
- **Papeles:** los tres nacen del mismo `Motor` y corren una consulta.
- **Queda, a propósito:** el simulacro de actualización se hace en la primera actualización real.

### 7.5 Frente 2 · el contrato: **ORE SQL**

**Qué es.** El contrato es **lo que significa una consulta en ORE**: qué acepta, qué devuelve, con qué tipos, qué
escribe y qué foto de los datos ve. Lo firma **ORE con quien pregunta**: una persona en la consola, el SDK de
Python, Java o Node, un Build o un motor de fuera por `/v1`. El motor lo cumple, pero no lo firma.

**Es un producto:** tiene nombre (*ORE SQL*), versión, una página que lee el runtime y una vara que lo demuestra.
Si la vara no pasa, el contrato no se cumple.

**La idea, en una frase:** el lenguaje y los tipos de cálculo son los de **DataFusion**, el lago es el de
**Iceberg**, y los verbos, el commit, la foto y la idempotencia son los de **ORE**. Cada pieza tiene un solo dueño,
sin pegamento entre ellas.

**Lo que no es:**
- **No es compatibilidad con DuckDB.** Lo que hoy está atado a DuckDB no se traduce: el producto vuelve a nacer
  encima de este contrato.
- **No es un dialecto propio.** ORE no reescribe SQL por texto ni con regex. Lo que añade entra por los puntos de
  extensión que DataFusion declara.

#### 7.5.1 El previo: lo que ya se prometía y lo que DataFusion declara

Leído el 2026-10-31 sobre `origin/main` 7c2e5f85 y sobre las fuentes de DataFusion 55.2.0, Ballista 55.0.0,
iceberg-rust `4dd8c946` y `apache/datafusion-iceberg`.

**Lo que ORE ya promete, en tres capas:**

1. **Propio de ORE y sin motor:**
   - **un solo escritor:** `ore-store`, sobre iceberg-rust. DuckDB no escribe ningún fichero en ningún camino;
   - **un solo commit:** `/v1` → `ore datasets --commit` → CAS en Postgres;
   - **la idempotencia**, en dos niveles: la `Idempotency-Key` UUIDv7 y la huella `ore.operacion`;
   - **los tipos del lago** de 0032, con los `field_id`;
   - **las ramas**, con un puntero por rama (0044).

   Los verbos son `write` en `overwrite`, `append` y `upsert`; `create or replace dataset … as`; `insert` e
   `insert or replace`; el DDL; y `createTable`, `updateTable` y `commitTransaction` por `/v1`. `UPDATE`, `DELETE`
   y `MERGE` se rechazan.
2. **Lo que decide DuckDB:**
   - el dialecto, guardado en cada View como `dialect: duckdb`;
   - los tipos de salida (`avg` tiene hoy tres respuestas);
   - las reglas de nulos derivados de 0051, medidas contra DuckDB 1.5.6;
   - la geo (las extensiones `spatial` y `h3`) y las Functions como macros;
   - la admisión, que pesa el `EXPLAIN` de DuckDB;
   - tipos de DuckDB que se cuelan en el SDK (`HUGEINT`, `TIMETZ`).
3. **Huecos que existen con cualquier motor:**
   - casts con `safe=true` que inventan nulos en `ore-store/src/carga.rs`;
   - tres definiciones distintas de ensanchar;
   - **cada tabla de una consulta se resuelve contra un fetch distinto, así que no hay foto común**;
   - Spark sin upsert ni idempotencia;
   - no hay idempotencia de pipeline.

**Lo que declara DataFusion 55.2:**
- **Casi todo el contrato:** el dialecto, las sentencias, 270 funciones escalares, 46 de agregado y 11 de ventana,
  la coerción, `CAST` que falla y `TRY_CAST` que da nulo, la división entera entre cero como error, la nulabilidad
  seguida por el plan y `NOT NULL` comprobado al escribir. También planifica todos los verbos (`INSERT` con
  Append, Overwrite y Replace; `DELETE`, `UPDATE`, `TRUNCATE` y `MERGE INTO`), aunque cada TableProvider decide
  cuáles cumple.
- **Lo que deja en blanco:**
  - ningún proveedor persistente implementa sus verbos;
  - no ejecuta transacciones;
  - no hay foto común entre tablas;
  - no tiene JSON, geo, UUID ni MAP;
  - no tiene `AS OF`;
  - la zona horaria por defecto es «sin zona»;
  - **los enteros desbordan sin error**.
- **La integración con Iceberg** salió de iceberg-rust el 2026-09-17 a `apache/datafusion-iceberg`:
  - solo hace append y lee en una sola partición;
  - no es idempotente ni trae codec;
  - está fijada a iceberg-rust `5b82ab99`, 82 commits por detrás de la de ORE.

**El cruce:** la capa 1 se queda como está, porque es justo lo que DataFusion deja en blanco. La capa 2 la
sustituye lo que declara DataFusion. La capa 3 no se arregla en lo de hoy: el contrato la resuelve por diseño.

#### 7.5.2 La visión

```
        quien pregunta (consola · SDK · Build · /v1)
                         │  ORE SQL vN
   ┌─────────────────────┴──────────────────────────────┐
   │ lenguaje y cálculo ........ DataFusion (adoptado)   │
   │ nombres ................... catálogo de ORE         │
   │ verbos · commit · foto .... TableProvider de ORE    │
   │ frontera con el lago ...... Iceberg (iceberg-rust)  │
   └─────────────────────────────────────────────────────┘
```

- **El lenguaje se adopta entero.** El contrato es el de DataFusion en el major que lleve el runtime, y la
  referencia es la suya, enlazada y no copiada. ORE solo fija y publica tres ajustes:
  - `execution.time_zone = UTC`;
  - `catalog.information_schema = true`;
  - el dialecto `generic`.

  Cada superficie tiene sus `SQLOptions`, y `SET` de claves que cambian la semántica no se admite.
- **Los nombres son el catálogo de DataFusion.**
  - Cada base de ORE es un `CatalogProvider`; `base.schema.nombre` es el nombre de tres partes.
  - La base de la sesión es el catálogo por defecto, así que `schema.nombre` funciona de forma nativa.
  - Las vistas son `TableProvider`s con su plan guardado (`dialect: datafusion`).
  - Las Functions son UDF con su nombre completo (`functions.<def>`): DataFusion busca el nombre compuesto tal cual.
- **Los tipos son dos sistemas con una frontera.**
  - Para calcular, los de DataFusion y Arrow; para guardar, el mapa de Iceberg de iceberg-rust.
  - Al escribir se convierte solo lo que no pierde nada; lo que pierda algo es un error con la columna y la fila.
  - Los tipos propios de ORE son tipos de extensión de Arrow con `FieldMetadata`: la geo en WKB con su CRS,
    Vector, Media y Anchor.
  - La columna del lago se empareja por `field_id`.
- **Los nulos los lleva el esquema.**
  - Iceberg `required` es no nulable, y DataFusion lo sigue por el plan.
  - `NOT NULL` se comprueba al escribir, en `DataSinkExec`.
  - Las reglas propias de 0051 se retiran.
- **Los verbos los planifica DataFusion y los cumple el TableProvider del lago de ORE:**

  | SQL | en ORE |
  |---|---|
  | `SELECT` | lee la foto |
  | `INSERT INTO` | append |
  | `INSERT OVERWRITE` · `create [or replace] dataset … as` | overwrite |
  | `MERGE INTO` | upsert y el `MERGE` general, en copy-on-write |
  | `DELETE` · `UPDATE` | copy-on-write; **son verbos nuevos** |
  | `TRUNCATE` | overwrite vacío |
  | `create dataset … (columnas)` · `drop dataset` | un commit sin datos; retirar deja la historia |
  | `create/drop` de base, schema, vista y colección | los cumple el catálogo del árbol, con su producto |

  - **El DDL de ORE habla con sus palabras.** `create dataset` entra por la extensión de `DFParser`, la misma con
    la que DataFusion añade `CREATE EXTERNAL TABLE` y `COPY`. El `select` de dentro lo planifica DataFusion.
    `CREATE TABLE` se rechaza.
  - **Cada sentencia que escribe es un commit de ORE.** No hay transacciones entre sentencias; el commit atómico
    de N tablas sigue siendo `commitTransaction`.
  - **El mismo verbo significa lo mismo** en SQL, Python, Java y Node.
- **La foto:**
  - una sentencia ve **una sola foto del catálogo** para todas sus tablas;
  - la siguiente sentencia ve lo que escribió la anterior;
  - una rama se lee con su puntero, y lo que no tocó, de `main`;
  - el viaje en el tiempo es la relación `at('b.s.t', snapshot => n)`.
- **Lo que la v1 no cubre, y lo dice:**
  - la geo y JSON, que van a sus frentes;
  - UUID y MAP como tipos SQL, que llegarán por `TypePlanner` cuando se pidan;
  - el desbordamiento de enteros, que pasa a negativo sin error, como hace DataFusion. Corregirlo es una aportación
    upstream (frente 10).
- **Cómo se demuestra:**
  - la vara de DataFusion (S4) cubre la capa adoptada;
  - **la vara de ORE**, con sus `.slt` propios, cubre lo que pone ORE;
  - una página del contrato la genera el runtime: la versión, los ajustes, las funciones desde
    `information_schema.routines` y las excepciones.
- **La versión:** `ORE SQL vN` es un major de DataFusion más la capa de ORE, y se ve en el manifiesto. Al subir de
  major, lo que cambie para el usuario sale nombrado; si nada cambia, la versión no sube.

#### 7.5.3 Las decisiones

1. **El TableProvider del lago es de ORE** (DF2): `datafusion-iceberg` queda como referencia (`predicate.rs`,
   `task_writer.rs`), no como dependencia.
2. **La foto es una sola sentencia sobre Postgres.**
   - `efectivo(ref, rutas)` de `ore-catalogo` lee los punteros de N tablas a la vez, y una sentencia de Postgres
     ve una sola foto.
   - La lee el zócalo `Catalogo`, una vez por consulta.
   - «A fecha del commit N» queda fuera: un `bigserial` no ordena la visibilidad.
3. **El viaje en el tiempo entra por `RelationPlanner`**, que ve los argumentos con nombre; una UDTF no los admite.
   El snapshot sale de los metadatos de la tabla de la foto, que guardan sus snapshots con su hora: no hace falta la
   historia de `ore-catalogo`. Solo vale para lo que la retención no haya expirado (C9).
4. **Un `TypePlanner` de ORE pone `TIMESTAMP` en `Timestamp(µs)` y `TIMESTAMPTZ` en `Timestamp(µs, "UTC")`**, en
   DDL, `CAST` y literales. Lo que llegue en `ns` (por ejemplo `now()`) se trunca con el cast de `INSERT`, que es la
   semántica adoptada.
5. **El commit tiene una sola puerta: `/v1`.**
   - El runtime commitea como cualquier motor: `updateTable` con `Idempotency-Key`.
   - Así hereda la idempotencia, las promesas, el dueño y el documento sin duplicar ninguna regla.
   - La clave se fija al planificar y se repite en cada reintento de Ballista.
6. **El dialecto guardado** (DF14) es `dialect: datafusion`. Como los productos vuelven a nacer, no hay nada que
   migrar.

Corrige dos filas de §4. En **aislamiento**, la rama se lee con su puntero, no como ref de Iceberg (0044). En
**tipos y nulos**, los tipos de cálculo son los de DataFusion y la frontera es la de Iceberg.

#### 7.5.4 Cómo se construye

**Lo que ya está:**
- **El suelo:** el zócalo `Catalogo` (la foto), el zócalo `Extensiones` (`TypePlanner`, `RelationPlanner`,
  funciones), `Sesion::planificar` (donde entra el parser), los papeles, la vara y el CI.
- **Las piezas de ORE:** `ore-store` escribe los ficheros, los manifiestos y los `requirements + updates`;
  `ore-catalogo` lee los punteros en una sentencia; `/v1` hace el commit.

Hoy solo se alcanzan desde sus binarios y son síncronas (un `block_on` propio), así que C3 las abre en asíncrono
sin cambiar lo que hacen.

**Cada paso entra con su `.slt` en la vara de ORE**, que es la definición ejecutable del contrato, y con su test.

| paso | qué | hecho cuando |
|---|---|---|
| **C1 · el lenguaje fijado** ✅ | Los tres ajustes entran en los desvíos del runtime. El `TypePlanner` pone µs. El manifiesto dice `ORE SQL v1`. Nace la vara de ORE (`tests/contrato/*.slt`), con el arnés de S4. | La vara de DataFusion sigue verde, con las excepciones ajustadas. `lenguaje.slt` pasa. |
| **C2 · el parser de ORE** ✅ | `create [or replace] dataset … as` entra por `DFParser`. `planificar` devuelve un plan de consulta o de escritura, con las `SQLOptions` de cada superficie. `CREATE TABLE` se rechaza. | `ddl.slt` pasa. |
| **C2b · el DDL de ORE** ✅ | Todos los verbos de ORE sobre el catálogo entran por la extensión de `DFParser`: bases (standard y foreign, con origen), schemas, datasets vacíos, vistas (y materializadas), colecciones de medios y retirarlos. Cada uno da su plan tipado (`CambioDeCatalogo`), con los nombres resueltos y su consulta planificada. | `ddl.slt` pasa, con cada forma y cada rechazo. |
| **C3 · `ore-store` en asíncrono** ✅ | `preparar_async` y `apilar` pasan a ser públicas y aceptan un `Stream` de lotes, con la memoria acotada. El `Almacen` va por `spawn_blocking`. | Los tests de `ore-store` siguen verdes. Escribir desde un stream da el mismo snapshot que desde un `Vec`. |
| **C4 · leer** ✅ | Un crate `ore-tabla` con el `Catalogo` de ORE, el `TableProvider` y las vistas. El `Catalogo` lee los punteros en una sentencia, con un trait `Punteros` implementado sobre Postgres y en memoria. El `TableProvider` usa el `TableScan` de iceberg-rust, en particiones por `FileScanTask`, con el `ArrowReader` sobre el `FileIO` de `ore-store`. | `nombres.slt` y `foto.slt` pasan sobre un lago en memoria. |
| **C5 · append y overwrite** ✅ | `insert_into` → `DataSinkExec(OreSink)`. `write_all` hace la primera mitad de `ore-store` y el commit por una `Puerta`: `/v1` con su `Idempotency-Key`, o memoria con CAS para los tests. Verbos: `INSERT`, `INSERT OVERWRITE`, `create dataset … as` y `TRUNCATE`; y los de C2b que son del lago (`create dataset … (columnas)`, `drop dataset`), por la misma puerta. | `verbos.slt`, `tipos.slt` y `nulos.slt` pasan. El mismo commit dos veces da un solo snapshot. |
| **C6 · DELETE y UPDATE** ✅ | `delete_from` y `update` leen la foto fijada, filtran o proyectan, y reescriben como overwrite. | `delete-update.slt` pasa. |
| **C7 · MERGE INTO** ✅ | Primero el upsert (`MATCHED → UPDATE`, `NOT MATCHED → INSERT`); el resto de las cláusulas, con su `.slt` cada una. | `merge.slt` pasa, y el resultado es igual al de `write(mode="upsert")`. |
| **C8 · distribuido** ✅ | Los codecs lógico y físico de `ore-tabla`, en los tres papeles (diseño y pasos C8·1–C8·5, abajo). | La vara de ORE pasa en el clúster de proceso. Un fallo provocado en el commit no duplica nada. |
| **C9 · el viaje en el tiempo** ✅ | `at(…)` por `RelationPlanner`, resuelto por el zócalo `Catalogo`. | `at.slt` pasa. |
| **C10 · en victor** ✅ | La vara de ORE y un banco de leer y escribir en `pesado-driver`, contra el lago y el `/v1` reales. | Todo verde en producción, y las cifras en este ADR. |

**C1, hecho (2026-10-31):**

- **Los dos ajustes** (`time_zone = UTC`, `information_schema = true`) están en `opciones::DESVIOS`, con su porqué.
  `contrato::TiposDeOre` pone en µs `TIMESTAMP`, `TIMESTAMPTZ` (UTC) y `TIME` sin precisión. Una prueba lo
  comprueba en local y como cliente del clúster, porque `upgrade_for_ballista` rehace el estado.
- **El manifiesto** dice `ORE Runtime 0.1.0 · ORE SQL v1 (…)`.
- **La vara de ORE** (`tests/contrato.rs`) corre cada `.slt` por el camino de una consulta: `Sesion::planificar` →
  `Motor::correr`. `lenguaje.slt` pasa.
- **La vara de DataFusion** sigue en verde: 257 ficheros y 36 018 registros en 43 s. Hay **24 desvíos nombrados**,
  todos de los dos ajustes:
  - con UTC, los instantes se pintan con `Z`, `now()` es `Timestamp(ns, "UTC")` y `SHOW TIME ZONE` dice UTC;
  - con `information_schema`, existe lo que el corpus esperaba que no.

  Un `RESET` de una clave del contrato vuelve al valor de DataFusion. La vara no lo cuenta como cambio del fichero,
  porque en ORE una consulta no hace `SET` ni `RESET`.
- **Lo que encontró la vara de ORE, un fallo del suelo:** la sesión nunca marcaba el inicio de la sentencia
  (`mark_start_execution`). Sin esa marca `now()` no se sustituye, y cualquier consulta con `now()` fallaba al
  ejecutarse. Arreglado en `Sesion::planificar`.
- **Las `SQLOptions` por superficie** pasan a C2: hasta que haya escrituras, la sesión solo tiene una política (sin
  DDL ni `SET`).

**C2, hecho (2026-10-31):**

- **`sentencia::analizar`** reconoce `create [or replace] dataset <nombre> as <consulta>` por sus primeras palabras,
  sobre `DFParser`. La consulta de dentro la analiza DataFusion, y lo demás es suyo.
- **`Sesion::planificar` devuelve un `Plan`**: o `Logico` (lo que planifica DataFusion, consulta o DML) o
  `CrearDataset`, con su nombre resuelto y su consulta planificada. El destino pasa por el zócalo de catálogo como
  cualquier otro nombre.
- **Lo que se admite depende de la superficie** (`Contexto::que_escribe`):
  - una superficie que solo lee no admite el DML de DataFusion ni `create dataset`;
  - ninguna superficie admite `SET` ni el DDL de DataFusion;
  - `CREATE TABLE` se rechaza con la forma de ORE;
  - `COPY` se rechaza, porque escribe fuera del lago.
- **Sin `or replace`, el dataset no puede existir.** Hoy, sin él, se niega. Lo comprueba el catálogo en C5.
- **`ddl.slt`** pasa: los errores de la consulta interna son los de DataFusion, más los rechazos de ORE.
  `create dataset … (columnas)` aún no está en ORE SQL.

**C2b, hecho (2026-10-31):** C2 solo traía `create dataset … as`. ORE declara más verbos sobre el catálogo
(`sql_del_arbol/guion.rs`, 0039), y hoy los analiza cambiando `dataset` por `table  ` en el texto antes de pasarlo
al parser. C2b los lleva todos a la extensión de `DFParser`, sin tocar el texto.

- **`sentencia::analizar`** reconoce los verbos por sus primeras palabras (las de ORE, como `standard`, `media`,
  `formats` u `origin`, por su texto):
  - `create [standard|foreign] database … [from origin … include (…)]`;
  - `create schema`;
  - `create dataset … (columnas)` y `… as <consulta>`;
  - `create [or replace] [materialized] view … [(col comment …)] [comment …] [with schema evolution] as <consulta>`;
  - `create [or replace] media collection … media … formats (…) [as <consulta>]`;
  - `drop database|schema|dataset|view|media collection [if exists] … [cascade]`.

  Las reglas de 0039 son errores con nombre. `or replace` e `if not exists` no van juntas. Una base o un schema no
  se reemplazan. Una foreign database exige su origen. `cascade` solo vale para schema y base. `create table` y
  `drop table` dicen la forma de ORE.
- **`Plan::Catalogo(CambioDeCatalogo)`** (`ddl.rs`) es el plan:
  - los nombres, resueltos en la base y el schema de la sesión;
  - las consultas de dentro, planificadas por DataFusion sobre la foto, y que solo leen;
  - las columnas de un dataset vacío, tipadas por DataFusion con el `TypePlanner` de ORE (`timestamp` es µs, y
    `not null`, no nulable);
  - una vista que nombra más o menos columnas de las que da su consulta es un error.

  Su `Display` es la forma canónica, que enseñan la vara y la observación.
- **Los verbos de datos no necesitan parser**: `INSERT`, `INSERT OVERWRITE`, `DELETE`, `UPDATE`, `MERGE INTO` y
  `TRUNCATE` son de DataFusion. `insert or replace`, el de DuckDB, es `MERGE INTO`.
- **Quién cumple cada uno:**
  - los de los datasets (crear vacío, retirar), el lago por `/v1`, con C5;
  - las bases, los schemas y las vistas, el catálogo del árbol (el documento en git), con su producto;
  - los de otros productos (la foreign database y su origen, la copia de la vista materializada, la colección de
    medios), su producto cuando vuelva a nacer.

  El contrato fija su forma y su plan.
- **`ddl.slt`** pasa, con cada forma y cada rechazo: la vara enseña lo planificado en una fila hasta que C5
  ejecute.

**C3, hecho (2026-10-31):**

- **`Lago::preparar_async` es pública** y recibe los lotes como un `Stream`. Cada lote se escribe y se suelta, en el
  runtime de quien llama, sin el `block_on` de `ore-store`. Lo mismo hace `Lago::abrir_async`.
- **Las versiones síncronas** (`preparar`, `instantanea_flujo`, `abrir`) son la misma función envuelta, así que su
  comportamiento no cambia. La geo ordena por Hilbert con `hilbert_por_tramos_flujo`, que da lo mismo que la
  versión síncrona.
- **Probado:**
  - preparar desde un flujo da las mismas filas confirmadas que desde un `Vec`;
  - abrir y preparar corren dentro de una tarea de otro runtime de tokio. El futuro es `Send` y no entra en pánico.

  Los 95 tests de `ore-store` siguen verdes.
- **El riesgo de §7.5.4 desaparece:** el camino asíncrono no hace `block_on`. El almacén síncrono sigue por
  `spawn_blocking`, en el `Storage` de `Lago`, como antes.

**C4, hecho (2026-10-31):** el crate `ore-tabla`, con tres piezas.

- **La foto** (`foto`) son los punteros leídos con `ore_core::punteros::Punteros::todos`: sobre Postgres es una
  sentencia (`vista_entera`), y sobre ficheros, un directorio. Cada puntero con `metadata_location` es un dataset.
  Las vistas son un `Objeto` más de la foto. De dónde salen en producción lo deciden los productos al volver a
  nacer; hoy solo las pone el lago en memoria.
- **`CatalogoOre`** es el zócalo `Catalogo` del runtime. Lee la foto una vez por consulta, en un hilo de
  bloqueo, y da un catálogo de DataFusion por base.
  - **Un dataset** se abre en su `metadata_location` (`Lago::abrir_async`).
  - **Una vista** se planifica al nombrarla, sobre la misma foto, con su base y su schema por defecto
    (`ViewTable`). Una vista que se nombra a sí misma es un error.
  - **Listar** (`information_schema`) no abre nada.
- **`TablaOre`** lee Iceberg con el `TableScan` y el `ArrowReader` de iceberg-rust. Reparte un `FileScanTask`
  por partición, hasta `target_partitions`. Todavía sin poda: esa es del frente 5.
- **La base de la sesión es su catálogo por defecto** (`Contexto::en_base`), con el schema `default` de ORE. Una y
  dos partes se resuelven como en DataFusion, dentro de la base.
- **La vara de ORE se muda a `ore-tabla`**, porque necesita el lago. Corre sobre un `LagoEnMemoria`, escrito con el
  mismo `Lago` de producción, con dos bases, tres datasets y tres vistas.
  - `nombres.slt` pasa: tres, dos y una parte, otra base, lo que no está, los tipos del lago, las vistas,
    `functions.<def>` y `information_schema`.
- **La foto, probada en Rust** (`tests/foto.rs`), porque un `.slt` no puede escribir entre dos sentencias:
  - una consulta con dos tablas y una vista lee la foto una sola vez;
  - lo escrito entre planificar y correr no se ve;
  - la siguiente consulta ve lo nuevo en todas sus tablas;
  - `DePunteros` lee los ficheros de punteros.
- **El guardián del árbol** (`ore-cli/tests/dependencias.rs`) admite `ore-tabla` en `ore-core`, por la misma razón
  que `ore-catalogo`: lee los punteros con la puerta del árbol, no con una copia.

**C5, hecho (2026-10-31):** ORE escribe por SQL con su contrato.

- **`escritura::OreSink`** es el `DataSink` de una tabla del lago. `write_all` hace la primera mitad de `ore-store`
  (`preparar_async`, en flujo, sobre la tabla de la foto o su esbozo) y el commit por la **`Puerta`**, una vez.
  `DataSinkExec` comprueba `NOT NULL` contra el esquema del sink, que es el de la tabla. Lo que cuenta un `INSERT`
  son las filas añadidas (`Preparado::anadidas`, nuevo), no el total.
- **`TablaOre` escribe:**
  - `insert_into`: Append anexa, Overwrite sobrescribe, y Replace se rechaza porque es `MERGE INTO`;
  - `truncate` sobrescribe con nada.

  **`ejecutable`** convierte lo de ORE en un plan de DataFusion:
  - `create [or replace] dataset … as` es un `INSERT OVERWRITE` sobre la tabla que hay o su esbozo, con el
    esquema de la consulta llevado a los tipos del lago (`al_lago`: cadenas de vista a cadenas, el tiempo en µs y
    en UTC) por `CAST`;
  - `create dataset … (columnas)` y `drop dataset` son un commit sin datos;
  - las bases, los schemas, las vistas y las colecciones dicen que los cumple el catálogo del árbol.
- **La clave de idempotencia** es un UUIDv7 fijado al planificar el sink, y va en el plan. La misma tarea ejecutada
  dos veces es un solo commit, con las filas sin duplicar. Si alguien escribió entre la foto y el commit, el CAS no
  entra y no se publica nada.
- **Las dos puertas:**
  - **`PuertaV1`** (`v1.rs`) es la de producción, igual que `write()` del SDK. `stage-create` para una tabla que
    nace, `updateTable` con `Idempotency-Key` y reintentos de un 5xx con la misma clave (0,5, 1 y 2 s); un 409 es
    conflicto; retirar es `dropTable`. Está probada contra un servidor falso: las rutas, el cuerpo, la clave
    repetida, el 409, el esbozo y el retiro. Contra `ore-serve`, en C10.
  - **`PuertaEnMemoria`** cumple el mismo contrato (CAS y clave) sobre la foto: es la de la vara.
- **La vara de ORE** pasa, con cada fichero sobre un lago recién sembrado:
  - `verbos.slt`: `INSERT`, `INSERT OVERWRITE`, CTAS en otro schema y otra base, `or replace` con el esquema
    nuevo, `if not exists` y `TRUNCATE`;
  - `tipos.slt`: los tipos del lago al releer, ns truncado a µs y `CAST` que falla en vez de dar nulo;
  - `nulos.slt`: `NOT NULL` al escribir, y la nulabilidad del plan (`count`, literales, `left join`);
  - `ddl.slt`: los datasets ejecutados.

  `tests/escritura.rs` prueba el reintento, el conflicto y que una superficie que solo lee no escribe.
- **Lo que encontró:**
  - una vista que no se puede planificar tumbaba `information_schema.columns` de todo el catálogo. Ahora sigue
    listada, sin columnas, y da su error al usarla (`VistaRota`);
  - el DDL sin consulta no fijaba la foto (`Sesion::fijar_foto`);
  - un CTAS de literales da columnas no nulables, así que un `INSERT` parcial falla por `NOT NULL`. Es el contrato.
- **Queda para otros frentes:**
  - el frente 4 pone la credencial con que se escriben los bytes (la que `/v1` presta en `config`); aquí escribe
    el `Lago` del catálogo;
  - releer da la zona de `timestamptz` como `+00:00` (el mapa de iceberg-rust), que es UTC.

**C6, hecho (2026-10-31):** `DELETE` y `UPDATE`, verbos nuevos que hoy ORE rechaza, sobre las filas de un dataset.
`drop` sigue siendo el verbo del activo entero.

- **En copy-on-write sobre la foto, en flujo:** `TablaOre::delete_from` y `update` (`escritura::cambiar`) escanean
  la tabla entera y añaden una proyección:
  - una columna de marca, `condición IS TRUE`: una fila cuya condición da `NULL` no se toca, como en SQL;
  - en `UPDATE`, cada columna asignada como `CASE WHEN marca THEN CAST(valor) ELSE columna END`.

  El mismo `OreSink`, con la marca (`con_marca`), cuenta lo que toca, quita lo borrado, suelta la marca y
  sobrescribe en un commit. Devuelve cuántas filas tocó, no cuántas reescribió. **Lo que no toca ninguna fila no
  hace commit.**
- **Un DML sobre una vista** dice que la vista no tiene filas propias y que se cambia su definición.
- **`delete-update.slt`** pasa:
  - borrar y cambiar con y sin condición;
  - el `NULL` en la condición;
  - el `CAST` que falla;
  - otro schema;
  - las vistas.

  `tests/escritura.rs` prueba que lo que no toca nada no hace commit y que lo que toca hace uno.
- **El coste:** reescribe la tabla entera, como el upsert de ORE hoy. Los borrados por posición y los deletion
  vectors son del frente 6.

**C7, hecho (2026-10-31):** `MERGE INTO`, el verbo estándar en lugar del `insert or replace` de DuckDB y el
upsert de ORE.

- **`TablaOre::merge_into` (`escritura::fusionar`) en copy-on-write sobre la foto.** Construye un plan lógico que
  DataFusion planifica: un `FULL JOIN` por la condición `on` entre el destino (con el número de cada fila,
  `row_number`) y la fuente (el plan que da DataFusion, vuelto a poner como tabla).
  - Cada fila casada, solo del destino o solo de la fuente, toma la **primera** cláusula de su clase cuya condición
    se cumple: `WHEN MATCHED`, `WHEN NOT MATCHED [BY TARGET]` o `WHEN NOT MATCHED BY SOURCE`, con `UPDATE`,
    `DELETE` o `INSERT [(columnas)] VALUES`. Los valores se llevan al tipo de la columna con `CAST`.
  - Lo que no toma ninguna cláusula se queda como estaba. Una fila solo de la fuente sin `INSERT` no entra.
- **El sink tiene ahora tres marcas** (`toca`, `quita` y `fila`), las mismas para `DELETE` y `UPDATE`. Cuenta lo
  tocado, quita lo borrado y sobrescribe en un commit.
  - **Una fila del destino que casa con dos de la fuente es un error**, como pide SQL, y no publica nada.
  - **Lo que no toca nada no hace commit.**
- **`merge.slt`** pasa:
  - el upsert;
  - el alias;
  - `insert` sin columnas;
  - la condición en la cláusula;
  - que gane la primera cláusula que se cumple;
  - `not matched by source`;
  - lo que no casa;
  - la fila que casa dos veces;
  - otro schema;
  - la vista.

  `tests/escritura.rs` comprueba que hace un commit o ninguno.
- **Frente a `write(mode="upsert")` del SDK:** la semántica es la misma (lo que casa por clave se sustituye y lo
  nuevo se inserta), y la prueba el primer bloque de `merge.slt`. El cotejo contra el SDK de verdad es de C10.

**C9, hecho (2026-10-31):** el viaje en el tiempo.

- **La forma:** `at('base.schema.nombre', snapshot => <id>)` o `at('…', timestamp => '<instante>')`, en un `FROM`.
  Hace falta así porque el dialecto `generic` de DataFusion no analiza `VERSION AS OF` ni `FOR SYSTEM_TIME AS OF`.
- **Cómo entra:**
  - por un `RelationPlanner` (`viaje::ViajeEnElTiempo`), que ve los argumentos con nombre;
  - es síncrono, así que la sesión deja hecha la E/S: recoge los destinos de `at(…)` del AST, los resuelve en la foto
    de la sentencia junto a los demás nombres, los abre e instala el planificador con ellos;
  - el zócalo `Catalogo` tiene un método nuevo, `en_el_tiempo`, sin E/S. En ORE (`TablaOre::en`) elige el snapshot de
    los metadatos de la tabla de la foto: por su id, o el último a esa hora. Lo lee con su esquema de entonces y solo
    para leer.
- **Lo que encontró:** DataFusion compara dos `TableScan` por su nombre y no por su tabla. Fundía `a` y `at('a', …)` de
  la misma sentencia en un solo escaneo y devolvía lo de hoy en los dos. El escaneo de entonces lleva ahora su propio
  nombre (`a@snapshot=…`) bajo el alias `a`. Es un hallazgo para aportar arriba (frente 10).
- **`at.slt`** pasa:
  - lo de hoy con un instante futuro;
  - el nombre entero, otra base, con alias y en un `join`;
  - antes de que la tabla existiera;
  - un snapshot que no está;
  - una vista;
  - lo que no existe;
  - la forma;
  - que no se escribe.

  **`tests/viaje.rs`** lee cada snapshot por su id y por su instante, y lo de hoy junto a lo de antes en la misma
  sentencia.
- **Fuera de la v1:** `commit => n` (la foto entera del catálogo en un commit) no está. Un `bigserial` no ordena la
  visibilidad (§7.5.3).

**C8, diseño (2026-10-31):** el plan, consistente al repartirlo.

- **Qué viaja y por dónde** (Ballista 55):
  - el **cliente** planifica en lógico, con la foto ya fijada, y manda el plan lógico al scheduler (codec lógico);
  - el **scheduler** lo planifica en físico: es ahí donde corren `scan` (que lista los ficheros), `insert_into`,
    `delete_from`, `update` y `merge_into`;
  - después parte el plan en etapas y manda cada una a los ejecutores (codec físico).

  DataFusion ya serializa sus nodos y todos los DML, `MERGE` incluido (`DmlNode`). Lo que no sabe serializar son los
  nodos de ORE: eso es C8.
- **La regla:** cada nodo viaja con todo lo que el contrato fijó al planificar, y decodificarlo no hace E/S ni vuelve
  a resolver nada. Si un papel resolviera por su cuenta, podría ver otra foto o fijar otra clave. Las piezas vivas
  (el `Lago` y la `Puerta`) no viajan: las pone el codec de cada proceso.

  | nodo | dónde se codifica | qué lleva |
  |---|---|---|
  | `TablaOre` (fuente de `TableScan` y destino de `Dml`) | lógico | el `metadata_location` y los metadatos de la foto, el snapshot de `at(…)`, el esquema de escritura y, si escribe, su nombre |
  | `EscaneoOre` | físico | el esquema y las `FileScanTask` de cada partición, que iceberg-rust ya serializa, más lo que el `FileIO` necesita. Los metadatos no viajan |
  | `DataSinkExec` con `OreSink` | físico | la tabla de la foto, el esquema de Iceberg, la operación, si lleva marcas y **la clave de idempotencia** |

  Las vistas no viajan: DataFusion las despliega antes de que el plan salga del cliente. `FuenteDeMerge` tampoco: se
  planifica dentro del scheduler y desaparece en el plan físico.
- **El commit, una vez:** `DataSinkExec` pide una sola partición, así que el sink corre en una tarea de un ejecutor y
  confirma con la clave que fijó el scheduler. Si esa tarea se repite (Ballista la reintenta, o la puerta reintenta
  la red), la clave es la misma y `/v1` contesta lo que ya hizo. Lo escrito dos veces queda huérfano, y
  `recoger-huerfanas` lo barre.
- **El enchufe en el suelo:** el zócalo de `Extensiones` da los dos codecs, y por defecto ninguno. Los papeles los
  pasan a los tres:
  - el cliente, con `with_ballista_{logical,physical}_extension_codec`;
  - el scheduler, con `override_{logical,physical}_codec`;
  - el ejecutor, con `override_{logical,physical}_codec`.

  El codec de ORE envuelve el de Ballista. Pone un prefijo a lo suyo y pasa lo demás al de Ballista: sin eso, los
  nodos de Ballista (el barajado) dejarían de viajar.

**C8, los pasos:**

| paso | qué | la prueba |
|---|---|---|
| **C8·1 · el enchufe** | Los codecs en `Extensiones` y en los tres papeles, envolviendo los de Ballista. | `papeles.rs` sigue verde, y un nodo de prueba hace el viaje de ida y vuelta por el clúster de proceso. |
| **C8·2 · el codec lógico** | `TablaOre`: la foto, el snapshot y el destino de escritura. | Ida y vuelta sin E/S (`LagoEnMemoria::lecturas` no se mueve). La tabla decodificada lee lo mismo y en el mismo snapshot. |
| **C8·3 · el codec físico** | `EscaneoOre` y `DataSinkExec` con `OreSink`. | Ida y vuelta: las mismas tareas, la misma operación, las mismas marcas y **la misma clave**. |
| **C8·4 · la vara distribuida** | La vara de ORE (`tests/contrato.rs`), entera, en el clúster de proceso: cliente, scheduler y ejecutor, cada uno con su `Motor` y su codec. | Todos los `.slt` pasan. Lo que no pase queda con su razón. |
| **C8·5 · el fallo provocado** | Una puerta que confirma y pierde la respuesta, y un sink que cae después de confirmar: se reintenta la tarea. | Un solo snapshot por sentencia (`LagoEnMemoria::commits`). Lo de más queda huérfano, y nada sale duplicado. |

**Lo que C8 deja para C10:**
- **La credencial de la puerta en el ejecutor.** Ahora es el ejecutor, no el cliente, quien confirma, así que necesita
  la credencial de máquina de `/v1`. Es la misma decisión que C10 ya tenía pendiente.
- **Lo que pesa una tarea.** Cada tarea de escaneo lleva las `FileScanTask` de su etapa, y el sink lleva los metadatos.
  Se mide en `pesado-driver` con tablas de miles de ficheros. Si pesa, el plan B es partir las tareas por partición
  al codificar.

**C8, hecho (2026-10-31):** el plan, consistente al repartirlo.

- **El enchufe (C8·1):**
  - `ore_runtime::codecs` envuelve el codec del frente con el de Ballista, con el prefijo `ORE\x01`.
  - El zócalo de `Extensiones` da los codecs (`codecs()`), el motor los guarda, y los tres papeles los pasan a
    Ballista.
  - **La prueba:** en `papeles.rs`, un nodo propio (`numeros`) viaja del cliente al scheduler y al ejecutor, y el
    barajado de Ballista sigue viajando con su codec.
- **Los codecs de `ore-tabla` (C8·2 y C8·3, `codec::CodecOre`):**
  - `TablaOre` viaja con los metadatos de la foto y el snapshot. Al decodificarse se reconstruye con
    `Lago::de_metadatos`, sin leer nada.
  - `EscaneoOre` ya no lleva la tabla: lleva su `FileIO`, el esquema y las `FileScanTask`, y lee con
    `ore_store::lago::lector`.
  - `DataSinkExec` viaja entero con su `OreSink`, porque `datafusion-proto` solo serializa los sinks de fichero.
  - La vista rota viaja con su error.
  - **La prueba (`tests/codec.rs`):** ida y vuelta sin leer del almacén (`LagoEnMemoria::objetos_leidos`). Lo escrito
    después de planificar no se ve al otro lado. `at(…)` conserva su snapshot. El sink conserva su clave: el plan de
    ida y el de vuelta dan un solo commit en `INSERT`, `INSERT OVERWRITE`, `DELETE` y `UPDATE`.
- **La vara repartida (C8·4):** `el_contrato_se_cumple_repartido` corre los nueve `.slt` en un clúster de proceso
  (scheduler, ejecutor y cliente), y pasan todos.
  - Las 82 sentencias que leen o escriben el lago van todas por `DistributedQueryExec`.
  - Las demás, Ballista las resuelve en el cliente, y así debe ser: los planes vacíos y `information_schema`, que es
    del catálogo del cliente.
- **El commit sin respuesta (C8·5):**
  - Ballista solo reintenta una tarea si su error es de E/S. Por eso la puerta distingue
    `FalloCommit::SinRespuesta`: la red no contestó, o `/v1` dio 5xx tras sus reintentos, y no se sabe si aplicó.
  - El sink lo devuelve como `IoError`. Ballista reintenta la tarea, que vuelve a escribir sus ficheros (quedan
    huérfanos) y confirma con la misma clave.
  - **La prueba:** con una puerta que confirma y pierde la respuesta, `INSERT`, `DELETE` y `MERGE` repartidos
    terminan bien, con un commit cada uno y sin filas de más.
  - En el proceso no hay quien reintente la tarea: la sentencia falla diciendo que no se sabe si se publicó, y el
    commit queda hecho una vez.
- **Lo que encontró, para aportar arriba (frente 10):**
  - Ballista devuelve los errores de una tarea con su nombre de Debug: `DivideByZero` y no "Divide by zero". El `.slt`
    acepta las dos formas.
  - `datafusion-proto` no tiene un camino para los sinks propios: `DataSinkExec` entero va por el codec de extensión.
- **Pendiente para C10:**
  - la credencial de `/v1` en el ejecutor, que ahora es quien confirma;
  - lo que pesa una tarea con miles de ficheros.

**C10, hecho (2026-10-31):** el contrato, en victor, en un nodo.

- **Cómo se corre** (`pruebas-de-fuego/runtime/c10.sh`):
  - Cada sentencia va en un Job del pool `pesado-driver` (normal, no spot), con `ore-runtime local --v1`, la imagen
    fijada por su digest y 3 CPU y 8 GiB.
  - El Job usa la cuenta `puesto` y el token proyectado de la apertura de un puesto abierto. `ore-serve` lo trata como
    ese puesto: lee y escribe a nombre de su persona, con la credencial que `/v1` presta para cada tabla (§7.6, §7.7).
  - El dataset de prueba es `sandbox.default.c10_ids`, de 10 millones de filas.
  - Es un nodo, como la sesión ligera de un puesto. No corre dentro del pod de la sesión: lo que tarda en una cuota
    de sesión se mide cuando el producto vuelva a nacer encima.
- **Lo medido** (imagen `9cd59268`). Cada tiempo es el del contenedor, con el arranque y la ida y vuelta a `/v1`:

| sentencia | da | tiempo |
|---|---|---|
| `create or replace dataset … as select … from generate_series(1, 10M)` | 10.000.000 filas | 23 s |
| `select count(*), sum(id)` | 10.000.000 | 8 s |
| `select k, count(*) … group by k … limit 3` | 100.000 por grupo | 6 s |
| `insert` de una fila | 1 | 7 s |
| `update … where id = 1` | 1 | 10 s |
| `delete … where k = 7` | 100.000 | 10 s |
| `merge into … using (values …)`: una coincide, una nace | 2 | 15 s |
| `select count(*)`, lo de hoy | 9.900.002 | 4 s |
| `at(…, timestamp => <antes de escribir>)` | 10.000.000 | 4 s |
| `information_schema.tables` | `c10_ids` | 6 s |
| `drop dataset` | «no existe» ✗ | — |

- **El `drop dataset` que faltaba.** El DDL de ORE no pasa por el planificador, así que el `CatalogoV1` no traía su
  nombre en la foto. Lo arregla #98: `ListaOre::objeto_resuelto` resuelve por `/v1` lo que la foto no tiene, y lo usan
  `drop dataset`, `create dataset` y el CTAS. Lo prueba `drop_dataset_resuelve_su_nombre_por_v1`, contra un
  `ore-serve` de mentira. La pasada en victor con la imagen `05eacb5f` no llegó a hacerse, porque el puesto se cerró
  antes; `c10_ids` sigue en el lago, y la próxima pasada lo retira.
- **Lo que encontró en producción, y lo arreglado:**
  - **El esbozo sin credencial.** `PuertaV1` no pedía `X-Iceberg-Access-Delegation` en el `stage-create`, así que la
    tabla que nacía no traía credencial. Ahora va en todas las peticiones (#96).
  - **El prefijo con barra.** La credencial prestada se busca por el prefijo más largo, y `ore-serve` lo da con barra
    final. Se compara con la ubicación más su barra.
  - **La credencial en el log.** El error de «sin credencial» imprimía la respuesta entera de `/v1`, con el token de
    GCS dentro. Ahora solo dice los prefijos. El token que salió era de vida corta y acotado a la tabla.
  - **Las raíces de TLS.** La imagen `scratch` no traía certificados de CA. El Dockerfile los copia de alpine y fija
    `SSL_CERT_FILE` (#97).
  - **`at` como tabla.** El catálogo pedía a `/v1` una tabla llamada `at`. Ahora la descarta al buscar referencias
    (#97).
  - **El puesto que se cierra.** Una apertura cerrada a mitad de la pasada da 403 («puesto no vivo en esta
    apertura»). El guion lee la apertura del Job del puesto en cada pasada.
- **El CI, de paso.** `main` tardó 1 h 17 min tras #94, porque cambió la receta de cargo-chef y la caché de
  dependencias se quedó fría. Tras #98 tardó unos 50 minutos compilando los binarios, con las cachés acertadas, y la
  causa no se ha medido. Dos pruebas inestables: el pánico del ejecutor de `papeles.rs`, que roza su límite de
  60 s, y el segundo `cuando` de `la_puerta_heredada.rs`, ya arreglado (#97).
- **Las decisiones pendientes, cerradas:**
  - **El token de máquina** es el token del pod (`x-ore-pod`), con la identidad de la persona del puesto.
  - **La credencial en el ejecutor:** la canjea él mismo con `loadCredentials` (§7.7). Desplegar los ejecutores es del
    frente 8 (§7.8).
  - **Lo que pesa una tarea con miles de ficheros** pasa al frente 8, que es quien reparte.

**El contrato está cerrado.** La vara de ORE pasa en el proceso y repartida (C8), y en victor pasa en un nodo contra
el lago y el `/v1` reales, salvo la retirada, que está arreglada y espera su pasada. Las piezas de los frentes 3 y 4
que C10 adelantó son su punto de partida (§7.6, §7.7).

**El orden:**
- C1 primero; C2 y C3 a la vez, porque no se tocan; C2b, que completa C2; luego C4 y C5; C6, C7 y C9 en cualquier orden; C8; y C10.
- **C1–C5 son el corte mínimo:** con ellos ORE ya lee y escribe por SQL con su contrato.

**Lo que se deja a otros frentes, a propósito:**
- **Frente 5:** leer al máximo (particiones por bytes, estadísticas, `pushdown`, borrados por igualdad).
- **Frente 4:** el acceso (`object_store` por tabla, la credencial prestada).
- **Frente 6:** la escritura fina (disposición, orden, huérfanos, compactación).
- **Los productos**, que vuelven a nacer encima después.

**Riesgos que ya se ven:**
- **El `block_on` de `ore-store` dentro de `spawn_blocking`** es lo primero que se comprueba en C3.
- **El `FileIO` de `ore-store` es síncrono por debajo:** sirve para leer bien, no rápido.
- **El token de máquina para `/v1` desde el runtime:** decidido en C10, es el token del pod (`x-ore-pod`).

### 7.6 Frente 3 · el catálogo: lo que ya existe

C10 adelantó una pieza de este frente, porque el motor en un puesto no podía leer de otra manera. Es su punto de
partida, no su diseño: el previo del frente 3 lo revisa.

- **`CatalogoV1`** (`crates/ore-tabla/src/v1.rs`). El zócalo de catálogo de un puesto resuelve por `/v1` lo que
  nombra cada sentencia, y lo que nombran sus vistas: `loadTable`, o `loadView` si es una vista, en paralelo. Es
  el mismo camino que Spark, DuckDB y PyIceberg, así que `ore-serve` decide el acceso, la rama del puesto y lo que
  el puesto lee.
- **Una mención, un snapshot.** Un mismo nombre se resuelve una vez por sentencia. Dos tablas distintas se leen
  cada una en su `loadTable`: el REST de Iceberg todavía no tiene una foto de varias tablas, y la propuesta del
  número de commit del catálogo sigue abierta.
- **El listado.** `information_schema` y `SHOW TABLES` listan la base con `listNamespaces`, `listTables` y
  `listViews`, sin cargar nada, así que nada cuenta como leído. Lo listado se abre al pedirlo.
- **Lo que el frente decide:**
  - la caché de metadatos entre consultas;
  - una foto de varias tablas (la propuesta de Iceberg, o una lectura de Postgres por lotes, como resuelve Unity
    Catalog);
  - las ramas de ORE como refs de Iceberg;
  - a medio plazo, `planTableScan` con la poda en Postgres (DF2, DF5).

#### 7.6.1 El previo: lo que se sabe (2026-10-31)

**Lo que hace hoy un escaneo** (`TablaOre::scan`, `crates/ore-tabla/src/tabla.rs`):
- **No poda al planificar.** Los filtros de la consulta no llegan a iceberg-rust (`with_filter`): se abren el
  manifest-list y todos los manifests del snapshot, y se planifican todos sus ficheros.
- **No hay caché entre consultas.** La `ObjectCache` de iceberg-rust (32 MB, en `moka`) va atada a un objeto `Table`,
  y el catálogo rehace la tabla en cada sentencia con su `loadTable`. Así que dura una sentencia.
- **Cada tabla, su viaje.** Una sentencia con N tablas hace N `loadTable` y, para cada una, el `metadata.json`, el
  manifest-list y sus manifests desde GCS.
- **Las tareas se reparten por cuenta**, no por bytes, y las estadísticas salen desconocidas. Eso es del frente 5.

**El lago de victor** (los 160 datasets de `main`, leídos del catálogo de Postgres y de su `metadata.json`):

| | |
|---|---|
| ficheros por tabla | mediana 1, p99 8, máximo 8; 165 en total |
| particionadas · con borrados | 0 · 0 |
| snapshots por tabla | 10 como mucho |
| `metadata.json` | 27 KB el mayor |

Hoy el coste de planificar son los viajes a `/v1` y a GCS, no los ficheros. Para medir lo que el frente cambia hace
falta una tabla sembrada con muchos ficheros y manifests: el lago de hoy no la tiene.

**El estado del arte:**
- **La planificación en el servidor ya está en la spec REST.** Son `planTableScan`, `fetchPlanningResult`,
  `cancelPlanning` y `fetchScanTasks`.
  - La petición lleva el snapshot o un rango incremental, `select`, `filter` y `stats-fields`.
  - La respuesta trae las tareas en línea (`completed`) o un `plan-id` que se sondea. Las tareas llevan sus borrados,
    el filtro residual y `storage-credentials`.
  - El servidor la anuncia en `endpoints` de `/v1/config`.
  - **Quién la tiene:**
    - en los clientes: Java ≥ 1.11 y PyIceberg sí; iceberg-rust no (#3011, abierto y sin conectar a `TableScan`);
    - en los servidores: Unity, para aplicar filtros de filas y máscaras, y Snowflake Horizon, en preview; Gravitino,
      solo síncrona; Polaris, todavía no (#966).
- **La foto de varias tablas no existe en REST.** Para escribir está `commitTransaction`. Para leer, en junio de 2026
  hay dos propuestas sin votar: un `LoadTables` por lotes, consistente por sentencia, y el número de secuencia de
  commit del catálogo.
- **Las cachés:**
  - **El cliente REST de iceberg-rust** guarda 100 tablas durante 5 minutos, con ETag. Nunca guarda las que traen una
    credencial prestada, porque un 304 no la renueva.
  - **Trino** guarda los metadatos en el coordinador.
  - **Spark** usa `CachingCatalog`, que caduca a los 30 s.
  - **DataFusion 55** guarda estadísticas, listados y pies de Parquet.
- **Planificar fuera del motor** (R2 SQL). Lo hace un coordinador:
  - poda en tres pasos: el manifest-list por partición, los manifests por min/max y los pies por row group;
  - emite el trabajo en streaming y en el orden del `ORDER BY`;
  - manda a cada worker los metadatos Parquet ya leídos.
- **`datafusion-iceberg`** hace `load_table` en cada `scan()` y planifica al ejecutar. Lo demás es como lo nuestro hoy:
  una partición y estadísticas desconocidas. La diferencia es que pasa los filtros como `Inexact` a `with_filter`.
- **Las podas erróneas.** #3483 (en nuestra rev) arregló la poda con tipos promovidos. Quedan #3501 y #3503 (de
  `date` a `timestamp`). Nadie documenta una guarda de ficheros planificados frente a `total-data-files`: la
  proponemos nosotros (§11).

**Lo que se deduce:**
- **La caché puede hacerse mejor que la de iceberg-rust**, porque en ORE la credencial ya no va con la tabla (§7.7,
  `LagosV1`). Un `metadata_location` es inmutable, así que la tabla, su manifest-list y sus manifests se pueden
  guardar por esa clave entre consultas. Solo el puntero (`loadTable`) y la credencial se renuevan.
- **La foto de varias tablas la tiene Postgres en una sentencia** (la foto de C4, `Punteros`). Servirla por `/v1` como
  un `loadTables` por lotes sigue la propuesta de junio y no cierra la puerta al número de secuencia.
- **`planTableScan` en `ore-serve` es de medio plazo:** compensa cuando los manifests pesan más que el viaje, y en
  victor todavía no pesan. Se decide con el banco.

**Cómo se construye (propuesta):**

| paso | qué | hecho cuando |
|---|---|---|
| **K1 · el banco** | Una tabla sembrada en `sandbox` con 1, 100, 1.000 y 10.000 ficheros y manifests, con y sin partición. `ore-runtime` mide las fases de planificar: `loadTable`, `metadata.json`, manifest-list, manifests y tareas, en tiempo y en lecturas. | Las cifras de hoy en victor, en este ADR. |
| **K2 · la poda y la guarda** | Los filtros, como `Inexact`, a `with_filter`. Sin filtro, los ficheros planificados tienen que ser `total-data-files`: un escaneo vacío de un snapshot con ficheros es un error. | La vara de ORE sigue verde, con y sin poda da lo mismo, y la guarda salta en una prueba. |
| **K3 · la caché** | La tabla, el manifest-list y los manifests, por `metadata_location`, en el `Motor`, entre consultas y acotados por memoria. El puntero y la credencial se siguen pidiendo. | La segunda consulta no lee nada de GCS para planificar (K1 lo mide). |
| **K4 · la foto de varias tablas** | Un `loadTables` por lotes en `/v1`, servido desde Postgres en una sentencia, y `CatalogoV1` resolviendo una sentencia en un viaje. | Dos tablas de una sentencia salen del mismo estado del catálogo, y en un viaje. |
| **K5 · las ramas** | Leer una rama de ORE con `at(…)`, como ref de Iceberg. | `at.slt` con una rama. |
| **K6 · `planTableScan`** | Solo si K1 dice que los manifests pesan: `ore-serve` poda en Postgres y devuelve las tareas. | Se decide con las cifras de K1. |

### 7.7 Frente 4 · el acceso: lo que ya existe

También adelantado en C10. Es *credential vending*, como hacen Unity Catalog, Polaris y Lake Formation: solo
`ore-serve` tiene la credencial del bucket, y el motor lee y escribe cada tabla con la que `/v1` presta para ella, a
la persona del puesto.

- **El zócalo `Lagos`** (`crates/ore-tabla/src/lagos.rs`) da el lago de una tabla por su nombre y su ubicación,
  sin leer nada. En las pruebas, un solo lago.
- **`LagosV1`** (`v1.rs`) da un almacén por tabla con su credencial prestada:
  - la siembra `loadTable`, o el esbozo si la tabla nace;
  - si no está sembrada, la canjea `loadCredentials`, y así un ejecutor canjea la suya: la credencial no viaja en
    el plan (DF7);
  - se renueva cinco minutos antes de caducar.
- **Una tabla que nace.** `loadCredentials` de un nombre esbozado presta la credencial de su sitio nuevo, con lo
  mismo que comprueba el esbozo.
- **La identidad.** `PuertaV1` y `LagosV1` se presentan con el token del pod (`x-ore-pod`). `ore-serve` lo trata
  como el puesto y escribe a nombre de la persona que lo abrió (*identity propagation*).
- **Provisional, a propósito.** El almacén prestado va sobre el `Almacen` síncrono de `ore-store`. Lo que el frente
  4 pide sigue pendiente: una sola pila de E/S (`Storage` de iceberg-rust sobre object_store), renovar sin
  rehacer el almacén, el rendimiento contra GCS y las cachés de datos (DF3).

### 7.8 Frente 8 · distribuir: el esquema de partida

El reparto ya existe y se prueba:
- **El suelo** arranca los tres papeles de Ballista, el cliente, el scheduler y el ejecutor (§7.4.4).
- **El contrato** viaja entero por el cable con los codecs de ORE, envueltos con los de Ballista (C8).
- **La vara de ORE** pasa entera en un clúster de proceso, y un commit sin respuesta no duplica nada.

Falta desplegarlo. El esquema que sigue se pensó en C10 contra el estado del arte y se aparcó para este frente: se
retomará con todo el roadmap hecho, para repasarlo, definirlo o cambiarlo.

```
pod del puesto ─────────────────────────────┐          Job del motor (a demanda)
│ cliente (ore-runtime local --v1 …)        │          │ N × ore-runtime executor
│ scheduler (:50050)                       ◄┼──────────┤   --scheduler <IP del puesto>:50050
│ agente: latido con su IP                  │  flight  │   cuenta `puesto`, token de su apertura
└───────────────────────────────────────────┘ ◄──────► └ cola-sesiones (spot, mínimo 0)
        │ x-ore-pod                                          │ x-ore-pod
        └────────────► ore-serve /v1 (como la persona) ◄─────┘
```

- **Cómputo de una sola identidad.** Es la forma de Spark en Kubernetes, EMR Serverless y los clústeres dedicados
  de Databricks: un driver por sesión, con sus ejecutores. Los ejecutores son pods del puesto, así que heredan la
  identidad de la persona sin código nuevo. Además, un clúster por sesión evita compartir un Ballista sin
  autenticación entre usuarios.
- **Los ejecutores, a demanda**, como el *dynamic allocation* de Spark. Nacen cuando un plan lo merece (el umbral
  de DF8; por debajo, un solo nodo), y se retiran tras un tiempo sin uso y siempre al cerrar el puesto.
- **El scheduler, en el pod del puesto**, junto al cliente. El estado de Ballista vive en memoria, así que muere
  con la sesión.
- **Lo que pide la malla.** Flux solo puede crear Jobs en la cola (Role `encolar`), así que no hay un Service por
  puesto: el agente le dice su IP a `ore-serve`, y `ore-serve` la escribe en el Job de los ejecutores. La red se
  abre con una NetworkPolicy estática por etiquetas, como la de Spark en el Build elástico.
- **Abierto:**
  - la red es por etiquetas y no por puesto, así que DF9 (mTLS) cierra de verdad;
  - cuánto tarda en llegar un ejecutor por Flux, Kueue y un nodo que escala; si es mucho, se crean por la API;
  - el job persistido;
  - AQE y broadcast;
  - datafusion-distributed para lo interactivo.

## 8. Las decisiones abiertas

Cada una se cierra con lo que se mida, y su respuesta da forma al fundamento. Las cerradas lo dicen en su fila.

| | decisión | lo que recomienda el estado del arte |
|---|---|---|
| **DF1** · versiones | qué trío fija el fundamento | DataFusion 55.x, arrow-rs y parquet 59.x (el workspace sube desde 58.4), iceberg-rust por una git rev posterior a #3483, object_store 0.13.x, Rust ≥ 1.95 |
| **DF2** · el proveedor Iceberg | adoptar `datafusion-iceberg` o uno propio | **Cerrada (§7.5.3): propio.** Lo recomendado era propio, sobre el núcleo de iceberg-rust: catálogo asíncrono, escaneo en N particiones con estadísticas, snapshot fijado y commit por `/v1`. Las mejoras genéricas son justo las PR que el upstream necesita |
| **DF3** · la E/S | object_store, OpenDAL o los dos | **object_store para todo**, implementando el trait `Storage` de iceberg-rust sobre él: una pila y una renovación de credencial |
| **DF4** · el lector | `ArrowReader` de iceberg-rust o `ParquetSource` de DataFusion | escalonado: el de iceberg-rust como base correcta y oráculo; `ParquetSource` con `ParquetAccessPlan` para lo que no tiene borrados, o los tiene por posición o deletion vector; la igualdad, como anti-join |
| **DF5** · el catálogo planificador | quién poda ficheros | a medio plazo, **`planTableScan` y `loadCredentials?planId` en `/v1`**, con la poda en Postgres y los metadatos Parquet servidos al ejecutor |
| **DF6** · el alcance de las escrituras | qué se escribe y cuándo | append y `create table as` primero; overwrite, delete y update por copy-on-write con validación en `/v1`; MERGE cuando haya RowDelta o ORE lo aporte |
| **DF7** · la credencial en distribuido | cómo llega al ejecutor | una referencia firmada (celda, sesión, tabla, caducidad), canjeada en `/v1` por el ejecutor; nunca en la configuración del job |
| **DF8** · el distribuidor | Ballista, datafusion-distributed o los dos | Ballista para los lotes, con el plan y los codecs independientes del distribuidor; datafusion-distributed medido para lo interactivo; un solo nodo por debajo de un umbral |
| **DF9** · la seguridad del clúster | cómo se cierra | NetworkPolicies, mTLS o malla entre piezas, la API REST de Ballista fuera de cualquier camino de inquilino y los tickets de resultados validados |
| **DF10** · la memoria | qué pool | **un pool jerárquico propio**: raíz de proceso con holgura e hijos por consulta y tarea, de límite atómico; un hash join que agota la memoria se reintenta con sort-merge; derrame a un volumen dedicado con `lz4_frame` |
| **DF11** · los fallos | cómo sobrevive un job a su scheduler | el job (plan, referencias, clave de commit) se guarda en Postgres antes de enviarse y se reenvía de forma idempotente; ejecutores fuera de spot |
| **DF12** · los clientes | cómo se habla con el fundamento | Flight SQL para apuntarle consultas mientras se construye; los productos, por la API de Ballista y su servicio de resultados |
| **DF13** · la cadencia | cómo se sigue al upstream | una actualización por trimestre, DataFusion, Ballista e iceberg-rust a la vez, con el corpus `.slt` propio como red |
| **DF14** · el dialecto guardado | qué pasa con las Views (`dialect: duckdb`), los `.sql` y `queries/` | **Cerrada (§7.5.3):** `dialect: datafusion`; los productos vuelven a nacer y no hay nada que migrar |
| **DF15** · la geo | SedonaDB o funciones propias sobre WKB | WKB con GeoArrow y CRS ya; SedonaDB candidata para las funciones, con su retraso de una major |
| **DF16** · el nombre | cómo se llama el producto | en inglés |

## 9. Lo que hoy hace DuckDB

Sobre `origin/main` 2267edff. Es lo que los productos, al volver a nacer, tendrán que cubrir sobre el
fundamento. No es un requisito del fundamento.

| área | dónde | qué usa de DuckDB |
|---|---|---|
| SDK Python | `puesto/python/ore/__init__.py`, `lsp_sql.py`, `sql_functions.py`, `calcular.py`, `geo.py`, `agente.py`, `ore_motor.py` | `iceberg_scan` con secretos por raíz, `attach ':memory:'` (un catálogo por base), `register` de Arrow, `memory_limit`/`threads`/`temp_directory` antes de cada sentencia, `explain (format json)`, `duckdb_memory()`, las Functions como UDF, `tokenize` y `describe` en el servidor de lenguaje |
| SDK Node | `puesto/node/ore/index.mjs`, `reparto.mjs`, `vigilante.mjs` | lo mismo, más el escritor Parquet/GeoParquet (`COPY`) y `nanoarrow`; los tipos de valor de DuckDB son parte de su API |
| SDK JVM | `puesto/jvm/ore/Ore.java` | JDBC, `arrowExportStream`/`registerArrowStream` |
| núcleo Rust | `ore-core`: `sql_del_arbol`, `guion`, `reparto`, `servir`, `vista_sql`, `sistema` (~24 usos de `DuckDbDialect`); `admision.rs` lee su `EXPLAIN`; `enriquecer.rs` genera su dialecto; `validate.rs` sólo acepta `dialect: duckdb`; `ore-view::a_sql` | su gramática, sus operadores, sus reglas de nulos y decimales |
| geo | 29 funciones `ST_*` y `h3_*`, el tipo `GEOMETRY('CRS')`, `geometry_always_xy`, `ST_Hilbert`, el `SPATIAL_JOIN`; `ore-teselas/indice_sql.rs`; `ore-piramide` lanza su binario | 0066–0068 y 0072 |
| escrituras | `ore-store` y `catalogo.rs` aceptan a DuckDB como escritor Iceberg | |
| imágenes y CI | extensiones en `/opt/ore/duckdb`; el binario en `piramide`; `duckdb` fijado en tres jobs | |
| lo guardado | Views con `dialect: duckdb`; `.sql` de 0039 y `queries/`, en DuckDB implícito; Functions con tipos de DuckDB | PIVOT, ASOF, `COLUMNS()`, `exclude`, `FROM t`, `::` |
| decisiones | 0031, 0032, 0038, 0039, 0040, 0051, 0053, 0066–0068, 0072, 0073, 0076 | «el dialecto es el del motor que corre la sentencia» (0039) |

## 10. Lo que falta por saber

- La versión exacta de object_store que fija DataFusion 55 (0.13.2 o 0.14.x), y la de arrow-rs en su
  `Cargo.toml`.
- Si `target_partitions = 0` respeta la cuota de CPU del cgroup o cuenta los núcleos del nodo.
- El coste de clonar una sesión plantilla con más de 300 funciones.
- Los límites de reintento de tarea y de etapa de Ballista, y la expiración del latido. Hay que leerlos
  en el código.
- Si el modo pipelined, Flight SQL y el history server de Ballista son de 55.0.0 o posteriores.
- Si 55.2 ya reserva en el pool el mapa del hash join que retiene el filtro dinámico.
- `timestamp_ns` y los valores por defecto v3 en iceberg-rust.
- HTTP/1.1 con un pool grande frente a `http2_only` contra GCS, con muchos GET de rango en paralelo.
- Qué versión de DataFusion fija SedonaDB y cómo se embebe desde Rust.

## 11. Riesgos

- **La memoria.** Lo que no derrama limita qué se puede prometer. Se dice por su nombre y se aporta arriba:
  el hash join (#24768), las ventanas (#22946), el reclamo (#21422) y el pool por consulta (#22036).
- **Lo inmaduro alrededor del motor.** iceberg-rust aún no es 1.0, datafusion-iceberg no tiene release, y
  Ballista no tiene HA ni autenticación. El fundamento lo rodea, lo construye por su cuenta donde haga
  falta, y lo aporta.
- **Errores silenciosos de poda.** Ya hubo una clase entera (#3483). Por eso los filtros se declaran
  `Inexact` y el lector de iceberg-rust queda como oráculo.
- **Un escaneo sin ficheros es un resultado vacío, no un error.** Medido en S3·4: un scheduler sin acceso
  al almacenamiento planificó cero ficheros y la consulta devolvió vacío, con éxito. El catálogo (frente 3)
  sabe cuántos ficheros tiene el snapshot y lo tiene que comprobar.
- **Cancelar en Ballista es lento.** El aborto es cooperativo y, tras cancelar, el scheduler pierde el estado
  de la tarea (S3, medido: 39 s y 21 s). Hasta que se arregle arriba, cancelar libera el clúster tarde.
- **La semántica cambia entre majors.** El corpus propio la fija y la actualización trimestral la vigila.
- **Las versiones.** arrow-rs, parquet, iceberg-rust y DataFusion tienen que ir al paso en todo el
  workspace, no sólo en el fundamento.
- **El dialecto es de las personas.** Una traducción que se equivoca en silencio es peor que un error
  (medido con sqlglot en `docs/query.md` §2.5).
