# 0081 · DataFusion paradigm & fundamentals

**ORE sustituye a DuckDB, por completo y en todos los caminos, por Apache DataFusion distribuido con
Ballista**, sobre su Iceberg y el Parquet de Rust. Primero se construye un fundamento excelente, estándar y
con teoría, que no sabe nada de productos. Después, cada producto vuelve a nacer sobre él.

**Estado:** borrador (2026-10-30). Rama `datafusion-fundamentos`, worktree `C:\ORE-datafusion`, sobre
`origin/main` 10092ad3. **Sin construir y sin medir.** Este borrador recoge la visión, los principios, las
garantías y el estado del arte investigado (§6, con el informe y sus notas en
[`docs/investigacion/`](../investigacion/reports/DataFusion%20y%20Ballista%20sobre%20Iceberg.md)).
**No trae todavía la forma constructiva**: las piezas del fundamento y su orden se remodelan por completo a
partir del estado del arte, y al cerrarlas se escribe un ADR nuevo con la imagen fiel del producto.

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
| **tipos y nulos** | el contrato de 0032 y el de nulos de 0051, en Arrow, con ida y vuelta exacta tipo a tipo | cadenas y binarios en `Utf8View`/`BinaryView` por defecto; tipos de extensión como almacenamiento más `FieldMetadata`, sin casteo propio; cada campo escrito hacia Iceberg lleva `PARQUET:field_id`; decimales hasta 38 |
| **la geo** | WKB con metadatos GeoArrow y su CRS, como ya decidió 0066/0067 | ningún lector Rust trata todavía `geometry`/`geography` v3 de Iceberg (Comet vuelve a Java) |
| **aislamiento** | una consulta lee **un snapshot por tabla, fijado al resolverla**; una rama de ORE es la ref correspondiente | el proveedor de catálogo del upstream recarga en cada escaneo y siempre lee el snapshot actual: dos menciones de una tabla pueden ver dos snapshots. El patrón correcto es resolver una vez, de forma asíncrona, con caché de una consulta |
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

## 7. Las decisiones abiertas

Ninguna está tomada. Cada una se cierra con lo que se mida, y su respuesta da forma al fundamento.

| | decisión | lo que recomienda el estado del arte |
|---|---|---|
| **DF1** · versiones | qué trío fija el fundamento | DataFusion 55.x, arrow-rs y parquet 59.x (el workspace sube desde 58.4), iceberg-rust por una git rev posterior a #3483, object_store 0.13.x, Rust ≥ 1.95 |
| **DF2** · el proveedor Iceberg | adoptar `datafusion-iceberg` o uno propio | **propio**, sobre el núcleo de iceberg-rust: catálogo asíncrono, escaneo en N particiones con estadísticas, snapshot fijado y commit por `/v1`. Las mejoras genéricas son justo las PR que el upstream necesita |
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
| **DF14** · el dialecto guardado | qué pasa con las Views (`dialect: duckdb`), los `.sql` y `queries/` | `dialect: datafusion` y una migración que falla en voz alta, nunca en silencio. Se decide cuando los productos vuelvan a nacer |
| **DF15** · la geo | SedonaDB o funciones propias sobre WKB | WKB con GeoArrow y CRS ya; SedonaDB candidata para las funciones, con su retraso de una major |
| **DF16** · el nombre | cómo se llama el producto | en inglés |

## 8. Lo que hoy hace DuckDB

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

## 9. Lo que falta por saber

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

## 10. Riesgos

- **La memoria.** Lo que no derrama limita qué se puede prometer. Se dice por su nombre y se aporta arriba:
  el hash join (#24768), las ventanas (#22946), el reclamo (#21422) y el pool por consulta (#22036).
- **Lo inmaduro alrededor del motor.** iceberg-rust aún no es 1.0, datafusion-iceberg no tiene release, y
  Ballista no tiene HA ni autenticación. El fundamento lo rodea, lo construye por su cuenta donde haga
  falta, y lo aporta.
- **Errores silenciosos de poda.** Ya hubo una clase entera (#3483). Por eso los filtros se declaran
  `Inexact` y el lector de iceberg-rust queda como oráculo.
- **La semántica cambia entre majors.** El corpus propio la fija y la actualización trimestral la vigila.
- **Las versiones.** arrow-rs, parquet, iceberg-rust y DataFusion tienen que ir al paso en todo el
  workspace, no sólo en el fundamento.
- **El dialecto es de las personas.** Una traducción que se equivoca en silencio es peor que un error
  (medido con sqlglot en `docs/query.md` §2.5).
