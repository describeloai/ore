# 0046 · Documents, objects & files: el producto de los datos que son ficheros

**Estado:** aprobado (2026-09-28); mercado investigado; el nombre, `MediaCollection`; F0 hecho; F1 medido ·
**Decide:** cómo guarda, nombra, gobierna y sirve la plataforma los datos que **no son tablas**:
documentos, imágenes, audio, vídeo (no estructurados) y ficheros CSV, Parquet, JSONL o logs
(semiestructurados), vengan de un almacén de objetos (S3, GCS, Azure Blob), de un SFTP o de
SharePoint. Abre lo que la spec dejó cerrado a propósito —*«un dataset de ficheros… se abre
entonces, y no estirando éste»* (`oos/spec/v1alpha12/00-scope.md:95-98`)— y lo abre sobre la base
de [`0045`](0045-el-puntero-es-de-la-fuente.md): **el puntero es de la fuente**.

## Por qué ahora, y por qué no es «añadir S3»

El siguiente origen es Amazon S3, y hacer un `ore-read-s3` que presente cada fichero como una tabla
sería la vía corta —el despacho por nombre (`ore-read-<tipo>`) no pide más—. Pero S3 es sólo **el
primer origen que sirve datos en forma de ficheros**; detrás vienen GCS, Azure, SFTP, SharePoint, y
la subida directa de un usuario. Lo que se decida aquí es la base del producto de almacenamiento y
gestión de ficheros de la plataforma, y todo lo que venga después cuelga de ello: la copia, la
ontología (un objeto con su foto o su contrato), el gobierno y el procesado (texto, OCR, embeddings).

## Lo que hay hoy (medido en el código)

- **La spec lo cierra a propósito.** Los tipos escalares son un conjunto cerrado; un binario es
  `Opaque`, que *«retira el gobierno»* (`v1alpha1/02-entity.md:247`); `Blob` es `OOS3001`. El
  `Dataset` es tabular (`v1alpha12/01-dataset.md:13-16`). El árbol sólo tiene por documentos
  `.yaml`, `.cedar`, `.oob` y `ontology.lock`.
- **El único concepto que ya nombra ficheros es `TrainedModel`** (v1alpha11): un prefijo del lago y
  el digest de su manifiesto. 0033 y 0034 ya dicen que es «un dataset de ficheros».
- **ORE no tiene ruta binaria**: `/arbol` es texto (404 si no es UTF-8, 2 MiB), no hay subida ni
  adjuntos, y el lago (`ore-store`) sólo escribe Iceberg.
- **Orígenes de ficheros**: sólo `ore-read-jsonl`, un directorio local de `.jsonl` (cada fichero una
  tabla, testigo = sha256 del contenido). `ore-store-r2` ya firma SigV4 contra cualquier S3, pero sin
  paginar (1000 claves), sin lectura por rangos y sin credenciales temporales.
- **La consola ofrece «Amazon S3» abierto** en el alta de fuentes, con un campo URI genérico: hoy un
  alta de S3 se registra y el Job de catálogo falla (*«no hay lector para `s3`»*). «Volumes» y
  «Unstructured» del catálogo son datos de mentira.
- ⚠️ **Dos fugas de credenciales para URLs con claves en la query**: `sin_credencial`
  (`rutas.rs`) no reconoce `secret_access_key=`, y el saneado del log del Job de catálogo
  (`44-el-catalogo.yaml`) sólo tacha `user:pass@`, así que una clave así podría acabar en `.fallos/`.

## Lo que hace el mercado (investigado, documentación oficial)

| concepto | Foundry | Databricks | Snowflake | BigQuery | AWS | Fabric |
|---|---|---|---|---|---|---|
| credencial y ubicación | Source | storage credential + external location | storage integration | connection | rol registrado en Lake Formation | cloud connection |
| colección gobernada de ficheros | dataset sin esquema; **media set** (tipado) | **volume** (managed / external) | stage (interno / externo) | *(bucket)* | *(prefijo)* | carpeta `Files` |
| tabla de metadatos por fichero | media set → filas | `read_files` + `_metadata` | **directory table** | **object table** | S3 Metadata | — |
| referencia a un fichero en una fila | **media reference** | *(ruta)* | tipo **FILE** | **ObjectRef** | *(URI)* | — |
| tabla deducida de ficheros | esquema aplicado al dataset | Auto Loader | `INFER_SCHEMA` / external table | BigLake | crawler | `Tables` |
| en sitio frente a copia | virtual media set / virtual table frente a sync | external frente a managed | stage externo frente a interno | todo en sitio | todo en sitio | shortcut |
| documentos a texto | PDF raw / OCR / layout | `ai_parse_document` | `AI_PARSE_DOCUMENT` | `ML.PROCESS_DOCUMENT` | *(Textract)* | — |

**Coinciden en seis cosas**, que son el estado del arte:

1. La credencial va **aparte** del permiso, y nadie da acceso al bucket.
2. Un fichero se consulta como **una fila de metadatos** (ruta, tamaño, ETag, fecha, tipo, bytes).
3. Una fila puede **apuntar** a un fichero.
4. Se sirve con **URLs temporales** tras comprobar el catálogo.
5. El esquema de un CSV o Parquet se **deduce y se congela**, con un sitio para lo que no encaja.
6. Los documentos pasan a texto con una función del motor.

**Se separan en dos, y las dos importan aquí:**

- **Sólo Foundry tipa la colección.** El dataset es el sustrato genérico (ficheros con transacciones
  y esquema opcional; ahí recomienda dejar CSV y JSON crudos). El **media set** es otra cosa: un solo
  tipo de medio con formato primario garantizado. Eso le da:
  - un identificador estable por ítem (lo que la ontología referencia);
  - transformaciones específicas del tipo;
  - retención, y un modo transaccional o sin transacciones.
- **Borrados y cambios en origen: el punto débil de todo el sector.** No los propagan Auto Loader,
  Snowpipe (que ni recarga un fichero que cambió de ETag), el sync APPEND de Foundry ni su virtual
  media set. Sí los reflejan las directory tables con autorefresco, S3 Metadata y la caché de las
  object tables.

Fuentes: docs de Palantir (datasets, file-based syncs, media sets, virtual media sets, media
reference, markings), Databricks (volumes, external locations, Auto Loader, `ai_parse_document`),
Snowflake (stages, directory tables, FILE, Snowpipe, `AI_PARSE_DOCUMENT`), Google (object tables,
ObjectRef, `ML.PROCESS_DOCUMENT`), AWS (Glue crawlers, Lake Formation, S3 Metadata, S3 Tables) y
Microsoft (OneLake shortcuts). La lista con URLs, en el informe de la sesión del 2026-09-28.

## Lo decidido

### 1 · El puntero en la fuente: `ObjectTable`

Como en 0045, la fuente guarda el puntero **una vez**, en su paquete. Para un origen de objetos, el
puntero no es una tabla del origen, sino **un conjunto de objetos del origen**: fuente, prefijo,
filtro (patrón de claves) y formato esperado.

- **Su catálogo** es la tabla de metadatos por objeto: clave, tamaño, ETag o generación, fecha de
  modificación y tipo de contenido. Es la directory table de Snowflake o la object table de BigQuery.
- **Se llama `ObjectTable`**, el nombre que el mercado ya reconoce para esto.
- **Lo que lo lee:**
  - una base foránea lo lee como vistas;
  - una base standard lo copia como `Dataset` (§3) o como colección (§2).

### 2 · La colección de ficheros, **tipada desde el primer día**

Un kind nuevo para una colección gobernada de ficheros de **un solo tipo de medio** (documento,
imagen, audio, vídeo, hoja de cálculo…) con formato primario garantizado:

- cada ítem, direccionable por su digest, con un manifiesto (lo que `TrainedModel` ya hace, generalizado);
- transaccional: una ingesta entra entera o no entra;
- retención declarable.

Dos formas:

- **Gestionada**: los bytes están en el lago.
- **Virtual**: en el origen, a través de su `ObjectTable`. Es el equivalente al virtual media set o al
  external volume.

Los ficheros de tipo mixto no son una colección: se separan por tipo al ingerirlos, o se quedan en su
`ObjectTable`. Con el tipo desde el primer día la colección puede ofrecer lo que sólo el tipo
permite: vista previa, extracción de texto, transcripción.

**El nombre.** La propuesta es *Media Repository*, con *Virtual Media Repository* para la virtual, y
la alternativa *File Collection*. Mi opinión, para decidir:

- *media* es el término correcto: Foundry lo usa para documentos, hojas y correos además de imagen y
  audio, y una colección de PDF es un «media» en ese sentido;
- *repository* choca con un concepto nuestro que ya existe: el **repositorio** de código (0035, 0036:
  `ore-serve /repositorios`, «Code repositories» en la consola, `repositorios.rs` en ore-core). Dos
  cosas distintas llamadas igual en el mismo producto se confunden en la interfaz, en la API y en la
  spec.

Por eso **`MediaCollection`**, con «Media collection» y «Virtual media collection» en la consola
(decidido por el usuario el 2026-09-28).

### 3 · Los semiestructurados son tablas: el `Dataset` de siempre

Un CSV, Parquet o JSONL se copia como el `Dataset` (Iceberg) que ya existe, leyendo el `ObjectTable`:

- el esquema se deduce al catalogar, se congela en el puntero, y lo que no encaja va a una columna
  rescatada;
- el `Dataset` no se estira: sigue siendo tabular.

Sólo cambia de dónde lee: un conjunto de objetos en vez de una tabla del origen.

### 4 · La referencia en la ontología: un tipo de propiedad nuevo

Una **referencia a un medio**, el equivalente a la media reference, el ObjectRef o el tipo FILE.
Apunta a un ítem de una colección, y sustituye a `Opaque` para los ficheros. Con ella, un `Contrato`
tiene su PDF y un `Empleado` su foto sin copiarlos en la fila. Es spec nueva (`C:\oos`).

### 5 · Servir y gobernar

- **Nadie accede al bucket.** ore-serve comprueba la política (Cedar) y emite una **URL firmada y
  temporal** para el ítem.
- **Las etiquetas** de una colección, y las de su `ObjectTable`, se heredan por linaje a lo que
  deriva de ellas, como hoy desde una Table.
- **La credencial** del origen vive en el cofre como la de cualquier fuente. La federación sin claves
  (identidad de GCP → AWS) se mide aparte.

### 6 · Procesar, después

Extraer texto (en crudo, por OCR o respetando la maquetación), transcribir y trocear para embeddings.
Serán transformaciones desde un puesto que producen `Dataset`s tabulares. No entran en la base; la
base tiene que permitirlas, y lo hace porque la colección está tipada.

## Lo que se mide en profundidad en iteraciones propias

- **Borrados y cambios en origen.** Es donde el sector falla, y la decisión de si se propagan, cómo
  (un diario del listado, eventos del almacén, inventario) y qué pasa con una copia que ya los tenía
  se mide antes de fijarla. Lo único que queda fijado aquí es que **el testigo del `ObjectTable` es
  su listado**: tiene que poder decir qué desapareció.
- **Si una base standard copia la colección o la referencia en sitio.** Coste, frescura, gobierno
  (una URL firmada del origen no pasa por nuestro almacén) y qué pasa cuando el origen cambia. Se
  mide antes de decidir.

## Lo que no se hace

- **No se estira el `Dataset`** para que sea «ficheros a secas»: lo dijo la spec y sigue valiendo.
  Un `Dataset` es tabular; una colección, ficheros.
- **No se presenta cada fichero como una tabla** en el driver (la vía corta): ata el producto a
  «todo es tabla» y deja fuera lo no estructurado.
- **No se lee un fichero desde una vista con una función** (`read_parquet('s3://…')`): sigue siendo
  `OOS2038`. Se lee por su `ObjectTable`, que es lo gobernado.

### F0 · lo medido (2026-09-28)

- **La guarda del alta** (`rutas.rs::sin_credencial`) buscaba `secret=` y `token=` a la letra. Con
  `s3://…?access_key_id=…&secret_access_key=…` decía «sin credencial»: sin custodio no se negaba
  (la clave acababa en el `.env.local` de un clon que se tira), y si el custodio fallaba no salía el
  502 «credencial NO guardada». Ahora mira el **nombre** de cada parámetro de la consulta con la
  regla con la que la CLI ya los tapa (`pass`, `pwd`, `secret`, `token`, `key`, `credential`), más
  `sig` (la SAS de Azure).
- **El log del Job de catálogo** que se deja en `.fallos/` sólo tachaba `usuario:clave@`. Medido con
  una URL de S3: `secret_access_key` salía entera. Ahora tapa también esos parámetros; probado con el
  `sed` de GNU y con el de busybox, que es el de la imagen `ore-drivers`.
- `ore` no imprime nunca la URL (va al driver por stdin, y el informe de `source add` la tapa); el
  riesgo era lo que un driver dijera al fallar.

**El flujo del cliente.** Un usuario IAM con claves de acceso y una política de sólo lectura sobre
el bucket es el camino más corto, y el que se ha usado para medir. No es el que AWS recomienda para
un tercero: Snowflake y Databricks piden al cliente **un rol IAM** que confía en su identidad, con un
*external ID*, sin claves que rotar. Aquí sería federar la identidad de GCP del driver con ese rol.
Se mide en F1; las claves quedan como la vía sencilla.

### F1 · lo medido (2026-09-28, contra un bucket real)

`s3://amazon-demo-bucket12313121122` (`eu-north-1`), con un usuario IAM de sólo lectura y boto3
desde fuera de AWS (España → Estocolmo). Dentro: el dataset público de Olist (9 CSV, 61 MB el
mayor), su `archive.zip` (44,7 MB), y un kit hecho a propósito: Parquet con partición Hive, CSV,
JSONL, PDF de texto y escaneado, imágenes. 26 objetos, 171 MB. Scripts `f1_barrido.py`,
`f1_medida.py` y `f1_kit.py` en el scratchpad de la sesión.

**Permisos: el primer fallo es el de cualquier cliente.** La política inicial dejaba leer la
configuración del bucket y **no listar** (`s3:ListBucket` va sobre el ARN del bucket, no sobre
`bucket/*`). Sin listar, S3 contesta 403 **también** a una clave que no existe, así que ni siquiera
se sabe si `GetObject` está concedido. ⇒ El `check` de `ore-read-s3` tiene que decir **qué acción
falta y sobre qué ARN**, como el de BigQuery dice qué rol. La política mínima que funciona:
`ListBucket` (+ `ListBucketVersions`, `GetBucketLocation`) sobre `arn:aws:s3:::<bucket>` y
`GetObject` (+ `GetObjectVersion`) sobre `arn:aws:s3:::<bucket>/*`.

| qué | medido |
|---|---|
| latencia | ~1,3 s la primera llamada (TLS), **~95–110 ms** las siguientes |
| listado | 26 objetos en una página, ~1,1 s en frío; `ListObjectsV2` pagina de 1000 en 1000 |
| catalogar todo | **74 llamadas y 668 KB leídos para 171 MB** (0,4 %): HEAD + 16 bytes por objeto, y lo que cada formato necesita |
| lectura por rangos | funciona, sufijo incluido (`bytes=-64`) |
| el índice de un zip | **940 bytes de 44,7 MB, 3 lecturas, 359 ms**: 9 CSV y sus tamaños, sin descargarlo |
| Parquet | esquema exacto (`decimal128(12,2)`, `timestamp[us, tz=UTC]`, nulos) y filas del pie. Los del kit (12 KB) caben en una lectura: **el ahorro con un Parquet grande no está medido** |
| CSV | tipos deducidos de los primeros 64 KB en ~100–200 ms por fichero |
| JSONL | la unión de claves por fichero: `usuario` aparece el segundo día |
| PDF | 3 con texto, 1 escaneado (0 caracteres): se distingue al catalogar |
| imágenes | formato y píxeles; el tipo de S3 cuadra |

**Lo que cambia el diseño:**

1. **`Content-Type` no es de fiar.** La consola de AWS subió los Parquet y los JSONL como
   `application/x-www-form-urlencoded`. El tipo se decide por **los bytes** (`PAR1`, `%PDF`, `\x89PNG`,
   `\xff\xd8\xff`, `PK\x03\x04`) y la extensión; el de S3 es una pista.
2. **El ETag no identifica el contenido**: un fichero subido por partes (más de ~16 MB desde la
   consola; 3 de 26 aquí) lleva `"<md5 de md5s>-N"`, que depende de cómo se partió. Todos los
   objetos traen, en cambio, **`ChecksumCRC64NVME` de tipo `FULL_OBJECT`** (S3 lo calcula por
   defecto desde 2025): ése es la identidad del contenido y el testigo por objeto. Con él, los dos
   `kit-s3.zip` (en la raíz y en una carpeta) se reconocen **el mismo** sin descargarlos.
3. **«Un prefijo es una tabla» no aguanta un bucket real.** En `Nueva carpeta/` hay diez CSV
   sueltos con esquemas distintos, y cada uno es una tabla. La regla que sale:
   - un prefijo es **una** tabla si sus ficheros comparten formato y esquema, con particiones Hive
     (`fecha=…`) como columnas: así `ventas/pedidos/fecha=*`;
   - si no, **cada fichero es una tabla**;
   - lo no tabular (PDF, imágenes) va a colecciones, por tipo.
4. **Deducir tipos de una muestra miente en los códigos.** `customer_zip_code_prefix` sale
   `Integer`, y un código postal brasileño empieza por cero (`01037`): como entero lo pierde. Un
   número con ceros a la izquierda en la muestra, o una columna `*_zip*`/`*_code*`, queda `String`,
   y el tipo deducido es una **propuesta que se contesta** (la decisión `tipo/*` de siempre), no un
   hecho.
5. **El BOM**: `product_category_name_translation.csv` empieza por `\ufeff` y su primera columna
   sale `\ufeffproduct_category_name`. Se quita al leer.
6. **Nombres reales**: una carpeta `Nueva carpeta` (espacio), `Foto Portada 2026.JPG`
   (mayúsculas), `bloc anucios.txt`. El `object` guarda la clave tal cual; el nombre del puntero es
   su identificador (0045 P1.5), y la colisión, `_2`.
7. **Un zip es un contenedor**, y los clientes los dejan en el bucket (aquí, el mismo dataset
   suelto y comprimido). Su índice se lee barato; **expandirlo es una transformación**, no el
   catálogo. Queda anotado para F3–F5.
8. **Sin versionado** (el bucket nunca lo tuvo): un borrado no deja rastro consultable. Lo único que
   hay es comparar dos listados. Es el punto de partida de la iteración de borrados.

**Sin medir en F1:** la federación sin claves (el rol IAM del cliente con *external ID* confiando en
la identidad de GCP del driver), eventos y S3 Metadata (el bucket no los tiene), un Parquet grande,
y un listado de miles de objetos.

## La iteración (por pasos; cada uno se mide antes de escribirse)

| paso | qué | criterio de hecho |
|---|---|---|
| **F0 · cerrar las fugas** ✅ | `sin_credencial` y el saneado del log del Job de catálogo reconocen las claves de S3 en la URL (`access_key_id`, `secret_access_key`, `session_token`) | tests de ore-serve; un fallo de catálogo con claves no las escribe en `.fallos/` |
| **F1 · medir contra un bucket real** ✅ (sin federación) | listado y paginación, lectura por rangos (el pie de un Parquet), formatos, tamaños, latencias, credenciales (claves frente a federación), qué da S3 para saber qué cambió (ETag, versiones, S3 Metadata, eventos) | informe en este ADR |
| **F2 · la spec** (`C:\oos`, v1alpha16) | `ObjectTable`, la colección (con su nombre decidido) y la referencia a medio, con sus diagnósticos y su conformance | conformance verde; ORE en el submódulo |
| **F3 · el catálogo de objetos** | `ore-read-s3 catalogo`: el `ObjectTable` de cada prefijo con su listado, y `ore source induce` lo escribe en la fuente (0045) | una fuente S3 real catalogada; el `ObjectTable` en el árbol |
| **F4 · lo semiestructurado como tabla** | `Dataset` sobre un `ObjectTable` de Parquet, CSV y JSONL: esquema deducido y congelado, columna rescatada, `leer` en Arrow | una base standard de S3 con sus datasets copiados |
| **F5 · la colección** | la gestionada (copia al lago por digest y manifiesto, transaccional) y la virtual; se decide la copia de la base standard con lo medido en su iteración | una colección de PDF de S3, en el lago y en sitio |
| **F6 · referencia y servir** | la referencia a medio en una entidad; URL firmada tras Cedar; etiquetas heredadas | un objeto de la ontología con su documento, servido |
| **F7 · consola** | alta de S3 con su formulario propio; colecciones en el catálogo con vista previa por tipo | lo de F5 visto en la consola |
| **F8 · procesar** | texto de documentos, en crudo, por OCR y respetando la maquetación, a `Dataset` desde un puesto | un PDF escaneado convertido en filas |

Los borrados en origen y la copia de la base standard tienen su propia iteración de medida entre F3
y F5; sus resultados cambian F5.
