# E11 · Media en código: estado del arte y base común (2026-09-30)

Investigación previa al ADR 0049 (media en los code repositories: SQL, Python, JVM, Node). Cinco
líneas en paralelo —una por paradigma y una de estándares—, documentación oficial y fuentes
primarias. Foundry y Databricks entran como un dato más, no como referencia. Lo que se afirma sobre
ORE está medido en el código de hoy.

## Lo que dicen las cinco a la vez

Cinco paradigmas, un mismo diseño. Donde el diseño es bueno, se repite:

1. **Tres capas: referencia → handle → bytes.** Listar no lee contenido; la referencia viaja por
   la tabla; el handle se abre cuando se pide; los bytes, al final. Beam lo hace explícito
   (`match → readMatches → ReadableFile`), Daft (`daft.File`), Lance (`take_blobs → BlobFile`),
   HF datasets (`decode=False`), Snowflake (`FILE`), BigQuery (`ObjectRef`, bytes en la
   pseudocolumna oculta `data`). Quien lo aplana lo paga: Spark `binaryFile` y `read_files` meten
   `content` en la fila, techo de ~2 GiB y todo a memoria.
2. **La referencia es un struct tipado, no una cadena.** Arrow no tiene tipo canónico para
   ficheros, así que cada motor define el suyo… hasta ahora: **Parquet ya tiene el tipo lógico
   `FILE`** (`uri`, `offset`, `size`, `content_type`, `checksum`, `inline`; comprobado en
   `LogicalTypes.md`) e **Iceberg v4 lo propone como tipo primitivo** con la semántica de Parquet
   (apache/iceberg#17919, abierto el 2026-09-01). Es la forma de nuestro ítem.
3. **Primero el listado, luego los bytes de las filas elegidas.** Se filtra sobre lo ligero. El
   listado es una tabla derivada con su frescura explícita (Snowflake `DIRECTORY`, object tables
   de BigQuery con obsolescencia máxima).
4. **La autorización la decide el motor; la URL solo transporta.** Privilegio del catálogo (stage,
   conexión, RLS por fila sobre el listado); la URL firmada es derivada, corta (30 min–6 h) y **al
   portador**. Nunca se guarda en una columna.
5. **La identidad es del contenido, y la da la plataforma.** Ningún motor garantiza que su
   referencia siga al origen (Snowflake documenta la desincronización). El ETag de un multiparte
   no es un MD5, y el checksum de S3 puede cambiar al copiar: ni ETag ni CRC son identidad.
   Lo son `sha256` (OCI, RFC 9530) y la versión fijada (`VersionId`, `generation`).
6. **Leer fijado o fallar.** `If-Match`/`VersionId`/`generation`, `fs.openAsBlob` en Node: mejor
   un error que otro contenido. ORE ya lo hace (`abrir(clave, etag)`, el 412 de `motivo_fijado`).
7. **Streaming acotado, y el ciclo de vida del stream es API.** Rangos y `seek` para lo grande;
   límites explícitos (píxeles en sharp, `maximumSize` en JavaCV, `maxLength` en Spark). En la JVM
   `close()` drena el cuerpo y `abort()` lo corta: un lector parcial de un vídeo debe abortar.
8. **El MIME se detecta, no se cree.** Declarado y detectado por firma, por separado (WHATWG MIME
   Sniffing, Tika, file-type); la extensión es una pista; nunca se asciende a un tipo ejecutable.
9. **Los errores de media son por ítem**, no del job (`on_error` en Daft y Pixeltable).
10. **Escribir es subir, hashear en el mismo pase y confirmar**: multiparte, checksum verificado por
    el servidor, transacción o commit explícito (Foundry, fsspec, Flink `FileSink`). Un ítem de
    salida existe cuando tiene su hash. Ninguna librería direcciona por contenido: lo hace la
    plataforma.
11. **La reproducibilidad vive en el catálogo versionado** (Lance, Pixeltable, Foundry), no en el
    motor: un transform fija la versión de la colección que leyó.

## Lo que SQL no hace, a propósito

No ordena, agrupa ni hace cluster por la referencia (Snowflake); las object tables son de solo
lectura (BigQuery); no hay ACL por fichero, se filtra el listado por fila; no transcodifica ni
sirve rangos: eso va a la URL o al handle. Las funciones de IA reciben **la referencia**, no los
bytes, y el motor comprueba el permiso sobre ella (Databricks, que pasa `BINARY`, es la excepción).

## Lo que cambia de verdad por lenguaje

| | Python | JVM | Node/TS | SQL |
|---|---|---|---|---|
| el handle | file-like con `seek` (`io.RawIOBase`) | `InputStream`/`SeekableByteChannel`, try-with-resources | `Blob` (`slice`, `stream`) y `ReadableStream` web | no hay: la referencia y la URL |
| lo grande | lotes y contrapresión del motor (Ray, Daft) | back-pressure (Reactive Streams, Flink); arrays ≤2 GiB | `pipeline` + `AbortSignal`; CPU fuera del bucle (libvips, ffmpeg en proceso hijo) | fuera de SQL |
| el tipo | por convención (struct Arrow) | nominal (`Dataset<MediaItem>` con `Encoder`) | un mismo tipo en Node, navegador y Workers | un tipo propio (`FILE`, `ObjectRef`) |
| lo que chirría | cada librería trae su file-like | JNI (JavaCV, Lance Java inmaduro) | sin `seek` estándar (tokenizadores); mucho ESM; `fluent-ffmpeg` archivado (2025) | — |

La base es común; el **handle** es idiomático de cada lenguaje. Esa es la frontera.

## Lo que ORE ya tiene (y dónde se separa)

- Ítem de colección: `clave`, huella, medio, tamaño, versión, estado; servir por huella con URL
  firmada (1 h, clamp a la sesión STS); `Media<c>` en la gramática y la ontología.
- Lectura fijada con `If-Match` y rangos en `ore-read-s3` (`abrir`, `rango_de`).
- **La huella de un ítem es el CRC64NVME del origen** (0046, l. 577); en la colección
  **mantenida** el blob del lago es `ore/v2/blobs/sha256/<hex>`, con un índice huella → sha256.
  Es decir: la identidad fuerte (sha256) existe **solo** donde se copia. En una **virtual** hoy
  solo hay CRC64NVME, que el estándar no acepta como identidad frente a un adversario (RFC 9530:
  `crc32c`/`md5` obsoletos para eso) — sí como validador y para deduplicar.
- Falta: servir por **ruta**; un handle perezoso con rangos sobre la URL; fijar la versión de la
  colección leída en un transform; nada de esto en SQL ni en Java.

## La base común propuesta (para el ADR 0049)

**Una referencia, `MediaRef`**, con la forma de Parquet `FILE` (y de su propuesta en Iceberg) y
los campos de ORE; en Arrow, extensión propia `ore.media_ref` sobre un struct, que degrada a un
struct normal en Parquet/Iceberg:

| campo | estándar | regla |
|---|---|---|
| `collection` (`b.s.c`) | propio | parte del localizador |
| `path` | propio | nombre lógico, no identidad |
| `version` | S3 `VersionId` / GCS generation | fija los bytes; sin ella, «la última» y lo leído dice cuál fue |
| `digest` | OCI descriptor, RFC 9530 | identidad del contenido; `alg:hex` (`sha256:`), abierto a otros |
| `size` | OCI, Parquet `size` | obligatorio |
| `media_type` / `media_type_detected` | RFC 6838 / WHATWG sniffing | declarado y detectado por separado |
| `checksum` | Parquet `checksum`, S3 `x-amz-checksum-*` | CRC64NVME: validador y deduplicación, **no** identidad |
| `selector` | RFC 9110 Range, Media Fragments, RFC 8118 (PDF), IIIF | una parte del medio; ausente = entero |
| `annotations` | OCI `annotations`, Dublin Core, Exif/XMP | técnicos baratos (ancho, alto, duración, páginas); GPS no por defecto |

**Nunca en una columna**: la URL firmada (caduca y es al portador) ni quién autoriza (lo decide
la concesión al servir).

**Siete operaciones, las mismas en los cuatro paradigmas** (el nombre, idiomático):

| operación | qué | SQL | código |
|---|---|---|---|
| `list(c, prefijo?, as_of?)` | el listado como filas de `MediaRef`, sin bytes | la colección como tabla | DataFrame / Dataset |
| `stat(ref)` | metadatos frescos | función escalar | método |
| `open(ref)` | el flujo, fijado; verifica tamaño y digest al terminar | — | handle idiomático |
| `read_range(ref, off, len)` | 206 con validador fuerte | — | `seek`/`slice` sobre el handle |
| `url(ref, ttl)` | derivada, corta, al portador | función escalar | método |
| `put(bytes\|stream, path)` | sube, hashea en el mismo pase, detecta el tipo; idempotente por digest | `CREATE COLLECTION … AS` (más tarde) | salida de colección |
| `verify(ref)` | recalcula el digest | — | método |

**Reglas**: identidad = `digest`; localizador = (`collection`, `path`, `version`); un transform
lee una colección **en una transacción** (`as_of`) y lo registra en su linaje; los errores de un
ítem son del ítem.

## Decisiones que el ADR tiene que tomar (abiertas)

1. **Identidad en las virtuales**: aceptar CRC64NVME como huella (como hoy, rápido, sin leer) o
   calcular sha256 al catalogar (leer cada byte del origen una vez). Afecta a `digest` obligatorio.
2. **Alinear con Parquet `FILE`** al escribir un listado a Dataset (`uri` = localizador `ore://`,
   `checksum`, `content_type`) o quedarnos en struct propio y mapear.
3. **El esquema textual**: `ore://b.s.c/ruta?v=<versión>#<selector>` (propio, sin registrar).
4. **TTL por defecto** de la URL (hoy 1 h; la investigación sugiere ≤15 min para lo interactivo).
5. **Por dónde empezar**: SQL (listado + `url`) es lo más acotado; Python es donde está el
   provecho (transforms). Java y Node, después, sobre la misma base.

## Fuentes principales

- Parquet `FILE`: https://github.com/apache/parquet-format/blob/master/LogicalTypes.md
- Iceberg v4 `file`: https://github.com/apache/iceberg/issues/17919
- Snowflake `FILE` y directory tables: https://docs.snowflake.com/en/sql-reference/data-types-unstructured ·
  https://docs.snowflake.com/en/user-guide/data-load-dirtables · https://docs.snowflake.com/en/sql-reference/functions/build_scoped_file_url
- BigQuery ObjectRef y object tables: https://docs.cloud.google.com/bigquery/docs/work-with-objectref ·
  https://docs.cloud.google.com/bigquery/docs/object-table-introduction
- DuckDB `read_blob`: https://duckdb.org/docs/current/guides/file_formats/read_file.html
- Beam FileIO: https://beam.apache.org/releases/javadoc/current/org/apache/beam/sdk/io/FileIO.ReadableFile.html
- Spark `binaryFile`: https://spark.apache.org/docs/latest/sql-data-sources-binaryFile.html
- AWS SDK v2 `ResponseInputStream`: https://docs.aws.amazon.com/java/api/latest/software/amazon/awssdk/core/ResponseInputStream.html
- Tika: https://tika.apache.org/3.2.0/detection.html
- Daft files: https://docs.daft.ai/en/stable/modalities/files/
- Lance blob: https://lance.org/guide/blob/
- Pixeltable: https://docs.pixeltable.com/platform/external-files
- HF datasets: https://huggingface.co/docs/datasets/image_load
- Ray Data: https://docs.ray.io/en/latest/data/data-internals.html
- Foundry media sets (dato): https://www.palantir.com/docs/foundry/transforms-python/media-set-transforms-api
- Node web streams / Blob: https://nodejs.org/api/webstreams.html · https://nodejs.org/api/buffer.html
- file-type: https://github.com/sindresorhus/file-type · sharp: https://sharp.pixelplumbing.com/api-constructor
- OCI descriptor: https://github.com/opencontainers/image-spec/blob/main/descriptor.md
- RFC 9530 (Digest Fields), RFC 9110 (Range), RFC 6838, RFC 8118; WHATWG MIME Sniffing;
  W3C Media Fragments; IIIF Image 3.0; C2PA 2.2; OpenLineage facets.

Sin verificar: API Java de OpenDAL; si `binaryFile` poda `content`; Lance Java #5167; si Flink trae
un formato de fichero binario entero.

---

# Segunda ronda (2026-09-30): Foundry, Databricks, IA sobre media, y ORE sin anestesia

Objetivo fijado: que los code repositories sean **el mejor, más cómodo y flexible lugar para
trabajar con media** —convertirla en activos tabulares: inferencia en lote, OCR, extracción de
texto, RAG, clasificación, detección, segmentación, transcripción, eventos, entidades—, de
vanguardia desde el primer momento. Base común primero; Python y SQL encima; Java y Node al final.

## Foundry: lo que copiar y lo que no

**Copiar.**
- La *media reference* como tipo de columna de primera clase: la misma en tabla, ontología, modelo
  y UI, con miniatura en la tabla.
- **Operaciones por tipo de medio** (`media_input.transform()`): imagen → OCR, teselas, embeddings;
  documento → texto con maquetación, páginas a imagen, campos de formulario; vídeo → escenas,
  audio; audio → `transcribe` con segmentos. Con **columnas de salida fijas** (`item, path,
  reference, resultado`) y `suppress_errors` (el fallo, dato de la fila).
- Lectura incremental `added/previous/current` con `batch_limit`; sobrescribir una ruta no rompe
  el incremental.
- *Access patterns* (miniaturas, ondas, teselas) bajo demanda con política de persistencia.
  `fast_copy` por referencia al blob.
- **AIP Document Intelligence**: comparar estrategias (OCR, OCR con maquetación, VLM) sobre la
  colección, medir calidad, tiempo y tokens, y **desplegar la ganadora como transform
  versionado**. El mejor bucle de iteración a producción visto. `Use LLM` con caché de filas por
  hash del prompt.
- https://www.palantir.com/docs/foundry/transforms-python/media-set-transforms-api ·
  https://www.palantir.com/docs/foundry/document-intelligence/overview

**Evitar.**
- Resultados como **JSON en un string**: no hay tipo bbox, segmento ni vector.
- Dos políticas de transacción mal separadas: 10 000 ítems por transacción frente a sin snapshot
  (semanas para 4 M de PDF).
- Permiso **por media set entero**; el token del build solo lee una referencia si su media set es
  entrada declarada (acceso y linaje mezclados).
- Streams sin acceso aleatorio; sin preview de media ni GPU en la vista previa; fotogramas en un
  TAR.
- Virtual media sets sin STS ni borrados (ORE ya va por delante: E9b, E8).

## Databricks

**Copiar.**
- Ya adoptó el `FILE` de Parquet (Beta, DBR 18: `uri, offset, size, content_type, checksum`;
  `MANAGED`/`EXTERNAL`), y lo usa para aplicar controles de tabla a ficheros: admite que el
  permiso por volume no basta.
- Salidas de IA versionadas con `bbox`, página, confianza y citas (`ai_parse_document` →
  `ai_extract` → `ai_prep_search`).
- El error por fila como columna. Eventos de fichero gestionados.
- https://docs.databricks.com/aws/en/sql/language-manual/data-types/file-type

**Evitar.**
- Auto Loader garantiza que el fichero se **escribió**, no que la **inferencia** salió bien: un
  fichero cuyo modelo falló queda «visto». Identidad por **ruta**. Cambiar el modelo o el prompt
  no invalida nada. Checkpoint opaco.
- Tres formas distintas de error; lo que no es del endpoint tumba la consulta.
- Las funciones de IA, no deterministas, **rompen el refresco incremental**.
- Límites fijos (1800 s, 100 MB, 500 páginas).
- El acceso por ruta o dentro de una UDF **se sale del linaje**.

## IA sobre media: lo que hace cómodo un entorno

Ray Data, Daft, Pixeltable y Lance Geneva coinciden:
- **El modelo se carga una vez por actor** (clase con `__init__`/`__call__`).
- **Recursos declarados** en el decorador (`gpus`, lote, reintentos, `on_error`).
- **Esquema de salida del tipo** (Pydantic).
- **Un ítem da N filas** (páginas, regiones, segmentos, chunks).
- **El error es dato**, y se reintentan solo los fallidos (`recompute_columns(errors_only=True)`).
- **Checkpoint y relleno incremental**; límite de ritmo por modelo; coste visible por ejecución.
- https://docs.ray.io/en/latest/data/batch_inference.html · https://docs.daft.ai/en/stable/api/udf/ ·
  https://docs.pixeltable.com/

**Resultados anclados.**
- Cada herramienta ancla con su geometría: Docling `prov.bbox`, Unstructured polígono antihorario,
  marker horario, WhisperX tiempos por palabra. Se normaliza al entrar, con un sistema de
  coordenadas declarado.
- El estándar neutral de «un resultado sobre una parte de un medio» es **W3C Web Annotation**
  (target = fuente + selector + estado de versión). https://www.w3.org/TR/annotation-model/
- COCO, WebVTT y ALTO/hOCR/PAGE van como exportación, no como almacenamiento.

**Memoización.** La clave que nadie cierra bien es `(identidad del contenido, función, versión,
modelo, parámetros)`, con estado `ok | error | pendiente`. Cambiar cualquiera de ellos recalcula;
mover la ruta sin cambiar el contenido, no.

## ORE sin anestesia (auditoría del código, verificada)

**Bloqueantes: hoy ORE no sostiene inferencia en lote sobre miles de ficheros.**

1. **Una colección virtual no se puede leer desde código.**
   - La URL firmada apunta al S3 del cliente, y el puesto solo sale a Google
     (`malla/21-el-puesto.yaml`, `salida-del-puesto`: DNS, metadatos, `199.36.153.8/30`), a
     propósito (`51-el-puesto.yaml:32-38`).
   - E9·4 funcionó con una **mantenida** (lago).
   - El diseño de acceso a media no es el correcto: el código del usuario no debería necesitar
     alcanzar el origen.
2. **Servir cuesta procesos, un `git fetch` y el manifiesto entero por llamada.**
   - ore-serve lanza `ore` → `ore-store pagina` y `blob-firmar`.
   - `pagina` carga todas las filas antes de filtrar (`ore-store/src/ciclo.rs:1802`); recorrer N
     ítems es N²/1000.
   - Medido: 1,4–2,1 s por llamada. ore-serve tiene 1 réplica y 500m de CPU. No escala a batch.
3. **El token del agente dura 300 s** (`61-realms.yaml`) y solo se renueva **entre** celdas
   (`agente.py:403,431`): una celda de inferencia larga pierde el acceso a mitad.
4. **Sin GPU.**
   - No la hay ni en la malla ni en la cuota (0 en la región, ADR 0027).
   - Los recursos del puesto son fijos (1–2 CPU, 2–4 Gi), y corre una celda cada vez, en un hilo.
5. **Sin tipos para resultados.**
   - No hay struct, vector ni bbox; `write()` rechaza listas, structs y binario
     (`ore/__init__.py:637-663`).
   - Cualquier OCR o embedding hoy sería JSON en un string: el error de Foundry, peor.

**Débiles.**

6. `@transform` no acepta una colección como entrada (`over()` → 404), el SDK no tiene listado de
   ítems, y `media()` **no pasa por el linaje** (`_lee`): lo leído no consta. Es el error de
   Databricks.
7. No hay incremental, ni error por ítem (una excepción tumba la celda), ni memoización, ni
   reintentos salvo el del commit. `write()` rechaza una tabla vacía: una pasada sin nada nuevo
   falla.
8. `over()` materializa la tabla entera en pandas; sin lotes ni streaming.
9. Identidad: la huella es CRC64NVME (o `etag:`); sha256 solo en las mantenidas.
10. La imagen del puesto no fija versiones. Sin internet, los pesos de un modelo solo llegan por
    ruedas o por el bucket.

**Bien, y se mantiene.**
- La cola con cuota por inquilino (Kueue).
- ore-serve no pasa bytes: URL firmada, con rangos en el lago.
- En las mantenidas, blobs por sha256, subidos con 32 hilos.
- Lectura fijada con `If-Match` (412).
- La procedencia de lo escrito.
- DuckDB con tope de memoria y derrame a disco.
- S3 por rol (E9b), por delante de Foundry.

## Lo que cambia de enfoque

1. **La media se lee por el lago, no por el origen.** Para código, una virtual necesita un camino
   que el puesto alcance: un proxy de lectura en la celda (rangos, fijado a versión) o
   materializar bajo demanda en el lago (la caché de *access patterns* de Foundry). Es la decisión
   de arquitectura nº 1, y cambia qué significa «virtual» para el cómputo.
2. **Servir es un servicio, no un proceso por petición**: un índice de ítems consultable sin leer
   el manifiesto entero, firma en proceso, lotes grandes y paginación por cursor.
3. **La credencial del código se renueva sola** durante la celda: el SDK pide token cuando caduca.
4. **Tipos de resultado nativos antes que cualquier OCR**: struct, `list<struct>`, vector de
   dimensión fija, y los tipos de ancla (página, bbox con sistema de coordenadas, intervalo de
   tiempo, rango de texto). Son el `selector` de la `MediaRef`, alineado con W3C Web Annotation.
5. **Un solo modelo incremental por ítem**, con la clave de memoización y estado
   `ok|error|pendiente`, en vez de checkpoint opaco (Databricks) o dos políticas de transacción
   (Foundry).
6. **El acceso a media desde código pasa por el catálogo**: la colección es entrada declarada del
   transform (linaje) y el permiso se comprueba ahí, sin que acceso y linaje sean lo mismo.
7. **Cómputo para IA**: GPU (nodos, cuota, recursos por trabajo) y pesos de modelo como activo del
   lago. Sin esto, «batch inference» es una promesa.

## La base común, revisada

La `MediaRef` de la primera ronda se mantiene (forma de Parquet/Databricks `FILE`), con dos
cambios: el `selector` pasa a ser **tipado**, no una cadena, y se añade la **tabla anclada** como
forma canónica de todo resultado sobre media:

| grupo | columnas |
|---|---|
| referencia | `item: MediaRef` |
| ancla | `ancla_id` (determinista), `ancla_padre`, `tipo` (item, página, región, texto, intervalo, fotograma), `pagina`, `bbox` + `sistema_coord`, `poligono`, `t_ini`/`t_fin`, `char_ini`/`char_fin` |
| carga | `etiqueta`, `texto`, `valor` (struct tipado por el esquema), `vector`, `confianza` |
| procedencia | `fn`, `fn_version`, `modelo`, `modelo_rev`, `params_hash`, `ejecucion`, `creado` |
| estado | `estado` (ok, error), `error_tipo`, `error_msg`, `intentos` |

**Python, primero.**
- La colección como entrada de `@transform`, e `items()` perezoso por lotes.
- `ItemHandle` con `open()`, `seek` y rangos, que pide su acceso al leer y lo renueva.
- Un decorador de modelo con estado por actor, recursos, lote, reintentos y `on_error`.
- `aplicar()` generador (1 → N), incremental por la clave de memoización.
- Operaciones por tipo de medio listas (texto con maquetación, páginas a imagen, transcripción,
  embeddings) que escriben la tabla anclada.
- `estimar()` en seco antes de gastar.

**SQL, acotado.**
- La colección como tabla de su listado (`MediaRef` + metadatos).
- Escalares `stat` y `url`; funciones de tabla con `LATERAL` para lo que da N filas.
- `{valor, error}` siempre.
- Materializar como vista mantenida incremental con la misma clave; `reintentar_errores`.

**Java y Node**: la misma `MediaRef`, la misma tabla anclada y el handle idiomático; al final.
