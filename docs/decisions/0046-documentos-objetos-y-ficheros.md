# 0046 · Documents, objects & files: el producto de los datos que son ficheros

**Estado:** aprobado (2026-09-28); mercado investigado; el nombre, `MediaCollection`; F0 hecho; F1 medido; F2, el texto de v1alpha16; E1 (esquemas y conformance), E2 (la gramática en ORE), E3 (la superficie) y E4 (el driver) hechos ·
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

### 3 · Los semiestructurados son tablas: una `Table` con `format`

*(Revisado al escribir la spec, v1alpha16: decía «el `Dataset` leyendo el `ObjectTable`».)* Un
Parquet, un CSV o un JSONL son **filas**, y todos los fabricantes separan registrar objetos
(*object table*, *directory table*, *volume*) de registrar las filas que hay dentro de ficheros
(*external table*, *BigLake table*). Aquí igual: **una `Table` cuya `object` es un prefijo o un
fichero, con `format`** (`parquet`/`csv`/`jsonl`, `partitions`, las opciones del CSV). Lo que
compone sobre una tabla —`View`, `Dataset` mantenido, la copia, la entidad— compone sobre ésta
**sin cambiar una regla**, y el `Dataset` no se toca. Las columnas se deducen y son una propuesta
que se confirma (F1: el código postal).

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
| **F2 · la spec** (`C:\oos`, v1alpha16) ✅ texto `860a269`; esquemas y conformance `4fa2206`/`241c894` (E1); ORE 43/43 (E2) | `ObjectTable`, la colección (con su nombre decidido) y la referencia a medio, con sus diagnósticos y su conformance | conformance verde; ORE en el submódulo |
| **F3 · el catálogo de objetos** | `ore-read-s3 catalogo`: el `ObjectTable` de cada prefijo con su listado, y `ore source induce` lo escribe en la fuente (0045) | una fuente S3 real catalogada; el `ObjectTable` en el árbol |
| **F4 · lo semiestructurado como tabla** | `Dataset` sobre un `ObjectTable` de Parquet, CSV y JSONL: esquema deducido y congelado, columna rescatada, `leer` en Arrow | una base standard de S3 con sus datasets copiados |
| **F5 · la colección** | la gestionada (copia al lago por digest y manifiesto, transaccional) y la virtual; se decide la copia de la base standard con lo medido en su iteración | una colección de PDF de S3, en el lago y en sitio |
| **F6 · referencia y servir** | la referencia a medio en una entidad; URL firmada tras Cedar; etiquetas heredadas | un objeto de la ontología con su documento, servido |
| **F7 · consola** | alta de S3 con su formulario propio; colecciones en el catálogo con vista previa por tipo | lo de F5 visto en la consola |
| **F8 · procesar** | texto de documentos, en crudo, por OCR y respetando la maquetación, a `Dataset` desde un puesto | un PDF escaneado convertido en filas |

Los borrados en origen y la copia de la base standard tienen su propia iteración de medida entre F3
y F5; sus resultados cambian F5.

### El plan de v1alpha16 en ORE (medido de cabo a rabo el 2026-09-28)

Lo que hay y se reutiliza: el firmador SigV4 de `ore-store/src/r2.rs` (hmac, sha2, ureq con
native-tls: compila aquí sin nada nuevo); el despacho por nombre (`s3://` → `ore-read-s3`), el Job
de catálogo, el cofre y el saneado de F0; el lago Iceberg con sus punteros en el árbol y su CAS; la
forma de `TrainedModel` (prefijo y huella) como molde de la colección; y la plantilla de BigQuery
para el driver (crate, verbos, catálogo en el driver, `check` por permiso, Dockerfile) y la de
`Dataset` (v1alpha12) para el compilador.

Lo que falta: el driver; listado paginado, rangos, `CRC64NVME` (sin crate en el lock: propia) y
prefirmado; un nivel «objeto» en la forma del catálogo (`ore-driver/src/catalogo.rs`, hoy sólo
tabular); v1alpha16 en `ore-core`; el puntero y el manifiesto de la colección; una ruta que sirva
binarios. Y dos hallazgos: **nada comprueba hoy las palabras de `changes.mode`/`witness`** (un
`witness` mal escrito compila y se trata como `none`, `vistas.rs:455`), y **no hay evaluador de
Cedar en tiempo de ejecución** (el acceso lo deciden las concesiones de IAM).

| paso | qué | hecho cuando |
|---|---|---|
| **E1 · la spec, completa** ✅ `oos 4fa2206` | esquemas `schemas/v1alpha16/`; `conformance/v1alpha16/` (8 aceptan, 34 rechazan, uno por regla; `OOS2040` incluido); el texto afinado al escribir los casos (abajo) | empujado en OOS |
| **E2 · la gramática** (`ore-core`) ✅ 43/43 | `V1Alpha16`, los dos kinds, sus claves y reglas de forma, `Table.format`, `listing`, `Media<x>`; en el enlazado OOS2004/2018/2040/2035 y el flujo (OOS4011/4002/4012); censo, assets, diff, `code.rs`; `borrador_de_v1alpha16`; mover el submódulo | v1alpha16 42/42, y v1alpha1–14 sin un resultado cambiado |
| **E3 · la superficie** ✅ | los kinds en `KINDS` de ore-serve, candado, `vista.rs`, carpetas de `ore init` | un árbol a mano con los tres compila y se sirve por `/documentos` |
| **E4 · el driver** (F3) ✅ | `ore-read-s3` con el firmador sacado a un crate común; `check` (qué acción falta y sobre qué ARN), `explorar`, `catalogo` (paginado, HEAD con huella, tipo por los bytes, pie de Parquet por rangos, CSV/JSONL con BOM y ceros a la izquierda, índice del zip), `testigo`; forma `objects` en `ore-driver` | pruebas con datos fijos, y una prueba de fuego de sólo lectura contra el bucket de F1 |
| **E5 · inducir** (F3) ✅ en local | `ore source induce` escribe un `ObjectTable` por conjunto y una `Table` con `format` por grupo tabular; la base, su `MediaCollection` **según su clase** (abajo); limpieza de `objects/`; la política IAM en `credenciales.rs`; el esquema de la fuente en ore-serve. Binario antes que malla | una fuente S3 real dada de alta en vivo, con sus punteros |
| **E5b · ore-serve a escala** | índices por petición en el esquema de una fuente y en `GET /paquetes` (hoy cúbico); después, no reanalizar el árbol en cada petición | el origen de 2.000 tablas por debajo de lo que tarda `ore validate` |
| **E6 · lo tabular** (F4) | `leer` de una `Table` con `format` (Parquet por rangos, CSV/JSONL con tipos congelados) a Arrow (0043) | una base standard sobre S3 con los datasets de Olist copiados y las filas cuadradas |
| **E7 · medir borrados** | qué dan el listado y las versiones (ya activadas en el bucket) ante un borrado, y qué hace con él una colección mantenida y una virtual; el coste de copiar ficheros al lago. (Si la standard copia o sirve en sitio ya no se mide: lo decide la clase, abajo) | informe aquí; decide E8 |
| **E8 · la colección** (F5) | manifiesto de ítems (huella, camino, formato, tamaño, versión), transacción = manifiesto nuevo, puntero `colecciones/*.json` con CAS, copia al lago por contenido o virtual, retención en el mantenimiento | una colección de PDF de S3, en el lago y en sitio |
| **E9 · servir y referenciar** (F6) | ruta de ítems y URL firmada y temporal; `Media<…>` resuelto en una entidad. **El acceso, en espera** (abajo) | un `Contrato` con su PDF, servido |
| **E9b · medir la federación** | el rol IAM del cliente con *external ID* que confía en la identidad de la plataforma, sin claves que guardar ni rotar | informe aquí; decide el formulario de E10 |
| **E10 · consola** (F7) | alta de S3 con su formulario (el de E9b), los `ObjectTable` en el árbol de orígenes, colecciones con vista previa por tipo | lo de E8 visto en la consola |

F8 (procesar) queda fuera de este plan.

**Decidido con el usuario (2026-09-28):**

- **El nivel del activo.** `ObjectTable` y `MediaCollection` son activos al nivel de `Table`,
  `View` y `Dataset`, en un paquete y un schema, y **comparten el espacio de nombres del schema**
  (`OOS2035`). El puntero vive en el paquete de la fuente (`<fuente>/<schema>/objects/`), una vez;
  la colección, en una base (`<base>/<schema>/collections/`). La **colección virtual** no es un
  kind aparte: es la `MediaCollection` con `virtual: true`. Lo que la separa de un `ObjectTable`
  es su historia —cada transacción fija qué ítems tenía (clave, huella, versión)— y su gobierno
  (dueño, tipo, etiquetas que suman); lo que comparte con él son los bytes, que siguen en el
  origen. Su límite: la retención de una virtual vale lo que la del origen (sin versionado, un
  borrado deja el ítem roto; con él, apunta a su `versionId`). Lo mide E7.
- **La clase de la base decide la colección** (al medir E5). Una base **estándar** copia todo lo
  que elige, así que su colección es **mantenida**: copia los ficheros al lago y compila contra
  `materialization.payload`, el conducto que ya autoriza para sus datasets. Una **foránea** es un
  espejo que no copia, así que la suya es **virtual**: se sirve desde el origen. Es la misma regla
  que da a una tabla un `Dataset` en la una y una `View` en la otra. Un conjunto copiado uno a uno
  en una foránea (`copies`), mantenido. Un contenedor (`archive`) o lo que no se sabe qué es
  (`binary`) no es una colección en ninguna: la base lo elige, la fuente escribe su puntero, y se
  dice por qué no hay colección.
- **El acceso al servir (E9), en espera.** Hay en curso, en otra sesión, el ADR que tiende el
  puente entre IAM y el plano de control de la organización y el plano de productos y de datos:
  con él, todo consumidor del plano de datos consumirá IAM de forma centralizada y estándar. E9 se
  conecta a ese puente cuando exista; hasta entonces no se construye una comprobación propia.
- **La credencial.** E4–E9 con claves de acceso (lo medido en F1); la federación sin claves se mide
  en su paso, E9b, antes de la consola.
- **Las palabras de `changes`.** Activar la comprobación de `mode`/`witness` para todas las
  versiones puede tumbar árboles vivos: se mide antes contra victor, demo y prueba (en E2).

**Lo que E2 hizo y midió.** `ore-core` habla v1alpha16: los dos kinds con sus claves y su forma,
`Table.format` (sus claves de csv son `OOS1005` fuera de csv), `listing`, `Media<x>` como variante
del tipo (`OOS3001` antes de v1alpha16; la colección se resuelve en el enlazado), el `ObjectTable`
como suelo del linaje y fuente de una consulta (sus columnas fijas y particiones), `OOS2004`,
`OOS2018`, `OOS2040`, `OOS2035` (el espacio de nombres del schema), `OOS2009`, y el flujo: la
colección hereda del `datasource` de su `ObjectTable` y suma lo suyo (`OOS4012` si rebaja), la
mantenida no virtual instancia `materialization.payload` (`OOS4011`, `OOS4002`), y `Media<c>` entra
en la propiedad como una herencia más, como el concepto. **Las palabras de `changes` se comprueban
ya** en `Table` y `ObjectTable`: medido antes contra los tres árboles vivos (victor `none/none` ×22,
demo `append/log` ×11, prueba sin tablas), todos dentro del vocabulario. Un hueco que ningún caso
cubría y E2 encontró: `from.objectTable` cruza de paquete y la regla de `exports` no lo veía; ahora
es `OOS2028`, con su caso (oos `241c894`). Queda para E3 la superficie que no es gramática: el
índice de assets, la superficie SQL del árbol (`sql_del_arbol`), las aristas, el diff de las caras
de un `ObjectTable`, `KINDS` de ore-serve, el candado y `ore init`.

**Lo que E3 midió e hizo.** Medido con un árbol a mano que junta todo v1alpha16 (la fuente con su
`ObjectTable` y su tabla de Parquet; la base con la colección, un dataset, la entidad con `Media<…>`
y una vista que pregunta el listado): compilaba limpio, y la superficie no lo veía. El índice de
assets daba 5 de 7 ítems, contaba `objects/` y `collections/` como schemas y dejaba la vista con
una relación rota; `ore view` decía que esa vista «lee datasets»; `/documentos` no servía los dos
kinds y habría escrito una colección en v1alpha12 (`OOS1003`); retirar una fuente no veía las
colecciones ni las vistas que salen de sus `ObjectTable`. Hecho: los dos kinds son ítems
(`objecttable:`, `collection:`) con detalle, columnas fijas, relaciones en los dos sentidos
(`sale_de`/`produce`, y `referencia`/`referenciada_por` por `Media<…>`), clasificación y el
conducto de la copia; `ore view` y `ore sql` dicen por dónde se lee cada uno; `/documentos` lee el
`ObjectTable` y no lo escribe (405: su escritor es `ore source induce`), y escribe la colección en
`collections/` y en v1alpha16; `quien_nombra` y `bases_que_salen_de` conocen las dependencias
nuevas. Y un fallo de flujo que el índice destapó: la raíz de un linaje solo heredaba el
`datasource` de una `Table`, así que el listado de una fuente `high` se copiaba sin clasificar;
ahora lo hereda (caso nuevo, oos `521eb11`). Pruebas: `assets::guardar_…`,
`punteros::lo_que_sale_de_una_fuente_de_objetos` y el paso 24 de `los-documentos.sh`.
**Movido** (decidido con el usuario): el diff de estos kinds, a después de E8; crear una colección
por SQL, a E10.

**Lo que E4 midió e hizo.** El driver `ore-read-s3` (verbos `check`, `explorar`, `catalogo`,
`testigo`; `leer` llega con E6) y el crate `ore-s3`, que saca la firma SigV4 de `ore-store/src/r2.rs`
para que el lago y el lector firmen con el mismo código: gana la credencial temporal y la fecha
inyectable, y pasa el ejemplo oficial de S3 (el `GET` con `Range` de la documentación de SigV4), no
solo el vector de la clave. Sin SDK ni dependencias nuevas: `Cargo.lock` solo gana los dos crates.
La forma del catálogo (`ore-driver`) gana `objects` —prefijo, patrón, medio, cuántos, cuánto pesan,
extensiones— y, en la tabla, `object` y `format`, con su ida y vuelta.

Contra el bucket de F1 (`pruebas-de-fuego/s3-real.sh`, solo lectura, la URL con la credencial por
`ORE_S3_URL` y nunca impresa): `check` dice cada acción sobre su ARN (`s3:ListBucket` sobre el
bucket, `s3:GetObject` sobre `bucket/*`, `s3:ListBucketVersions` informativa) y, con la región
equivocada, **que es la región** (`x-amz-bucket-region`), no un permiso. El catálogo da **12 tablas
y 5 conjuntos**: `ventas/pedidos/fecha=*` es una tabla Parquet con `fecha` como columna, 1500 filas
del pie y `total` `Decimal<12, 2>`; los ocho CSV de Olist y la traducción, una tabla cada uno
(esquemas distintos), con el código postal en texto y el BOM quitado; `logs/*.jsonl`, una tabla con
la unión de sus claves; contratos (document, 4), fotos (image, 3: `jpg` y `png`), los zips con su
patrón y lo suelto de la raíz. **38 peticiones y 583 KB leídos para 171 MB, en 6,6 s** desde
España: reutilizar la conexión lo bajó de 14,4 s (la primera petición TLS cuesta ~1,3 s). El
`testigo` de un prefijo es la huella de su listado (clave, ETag, tamaño): cambia si algo entra,
cambia o **desaparece**. Y por `ore`: `source add` + `check` + `catalog` dan el mismo catálogo, y la
credencial no está ni en el manifiesto ni en el catálogo. La imagen de drivers lleva
`ore-read-s3`. **Sin medir**: un listado de miles de objetos (la confirmación de tipo es de dos por
carpeta y extensión, y un CSV cuesta una lectura de 64 KB: lineal en carpetas, no en objetos) y la
federación (E9b).

**Lo que E5 midió e hizo.** Medido con el catálogo de F1 en un árbol con la fuente aparte y dos
bases: el alta y el catálogo ya iban, y la inducción no. La `Table` de un fichero salía con el
nombre del catálogo en `object` y sin `format` (tres `OOS1004` por `listing`); el alcance no
aceptaba un conjunto («el origen no tiene `nueva_carpeta.contratos`»); nadie escribía un
`ObjectTable` ni limpiaba `objects/`; y ore-serve daba el nombre del catálogo como objeto y no
enseñaba los conjuntos. Hecho:

- la `Table` de ficheros sale en v1alpha16 con dónde está (`object`: la clave o el prefijo) y cómo
  se lee (`format`, `type` primero); los nombres del schema se reparten entre tablas y conjuntos
  (`OOS2035`);
- el alcance elige un conjunto como una tabla, por su nombre del catálogo, y `s.*` los incluye;
- `ore source induce` escribe el `ObjectTable` de cada conjunto que alguna base elige
  (`<schema>/objects/`), lo exporta y lo retira cuando ya no lo elige nadie, con su carpeta;
- la base escribe su `MediaCollection` según su clase (arriba), con las extensiones vistas como
  `formats`; sin fuente aparte, escribe también el puntero (`<conjunto>_t`); la re-inducción gobierna
  `objects/` y `collections/` como las demás carpetas (solo lo marcado);
- ore-serve: en el esquema de una fuente, `object` sigue siendo el nombre que se elige y `location`
  dice dónde está; los conjuntos son filas `kind: objects` con sus columnas fijas, medio, cuántos,
  bytes, su puntero y quién los usa (por su colección o por haberlos elegido); una base sin tablas
  enseña lo que eligió, no el bucket entero; `GET /fuentes/credenciales/s3` da la política IAM de
  solo lectura (las cuatro acciones que `check` prueba).

El árbol con las dos bases compila sin un diagnóstico; el Job de copia de la estándar no ve las
colecciones (recorre `Dataset`), así que hasta E8 una colección mantenida existe y no tiene ítems,
como un dataset antes de su primera copia. Pruebas: `los_objetos_de_un_bucket` (el catálogo de F1
como fixture: clases, retirada, errata), `un_bucket_ensena_…` y
`s3_ensena_la_politica_de_solo_lectura`. **Sin hacer aquí**: el alta en vivo necesita que una
persona guarde la credencial (el custodio decide con su concesión, `secreto:emitir`), así que la
hace el usuario en la consola de victor; `drift-detect` no mira los conjuntos todavía.

**E5′ · el puntero de todo lo catalogado** (decidido con el usuario al ver el origen en la
consola, 2026-09-29; enmienda la regla de 0045 P3′). Con E5 desplegado, el origen `s3_ventas` de
victor enseñaba sus 17 activos y ninguno abría su ficha: la fuente solo escribía el puntero de lo
que **alguna base** elegía. Esa regla no tenía un argumento de fondo en 0045 («el Job de catálogo
no lo lanza: cataloga una vez y en ese momento ninguna base usa nada») y contradecía el suyo: el
puntero es un **hecho del origen** y se escribe sin revisión, así que no depende de que alguien lo
lea. Ahora:

- `ore source induce` escribe el puntero de **todo** el catálogo —tablas y conjuntos— y retira solo
  lo que **desaparece del origen**;
- `ore source catalog --out packages/<fuente>/discover.catalog.json` —lo que hace el Job de
  catálogo— induce la fuente en el mismo acto: sin tocar la malla;
- retirar una base no toca su fuente; `discover`, `review`, `model` y `copy` siguen llamando al
  escritor, idempotente, por los árboles catalogados antes de esta regla.

Medido antes, sobre copias de los árboles vivos y con binarios de release: en victor +88 punteros
(82 → 170 YAML, `ore validate` 0,2 → 0,3 s), en demo +237 (34 → 271, 0,17 → 0,35 s), todos
compilan sin un diagnóstico. Un origen sintético de 2.000 tablas y 32.000 columnas: 3 s de
inducción, 4,8 MB, `ore validate` 0,3 → 3,4 s (lineal, ~1,6 ms por puntero). **Y un coste cúbico en
ore-serve**: `GET /paquetes` 1,5 → 105 s y el esquema del origen 1 → 98 s, porque cada fila del
catálogo recorre todos los documentos, y cada puntero, otra vez todos buscando lo que lee (sin
índices: ~10¹⁰ comparaciones). Existía ya con la regla vieja —una base que eligiera 2.000 tablas
tardaba lo mismo—, y es lo siguiente: índices por petición, y después no reanalizar el árbol en
cada una. Un hallazgo más: el sufijo `_2` de dos objetos que dan el mismo identificador (`Pedidos`
y `pedidos`) se calcula sobre el catálogo entero, así que si uno sale del origen el otro cambia de
nombre y las bases que lo leían dejan de resolver. Era así antes; con todo escrito se verá más.

**Lo que E1 afinó del texto de v1alpha16** (un caso no puede dejar una regla abierta): una etiqueta
de colección por debajo de la heredada es `OOS4012` (se eleva, no se rebaja), no `OOS4002`; copiar
es un conducto —la mantenida no virtual instancia `materialization.payload` (`OOS4011`, `OOS4002`),
la virtual y la escrita no—; una consulta lee un `ObjectTable` (sus columnas fijas y particiones,
`OOS2018` las demás) y no lee una colección (`OOS2018`); `listing` en una `Table` sin `format` es
`OOS1004`.
