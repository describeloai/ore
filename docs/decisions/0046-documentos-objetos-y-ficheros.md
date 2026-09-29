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
| **E5b · ore-serve a escala** · 1 ✅ · 2 ✅ | **1**, índices por petición en el esquema de una fuente y en `GET /paquetes` (era cúbico); **2**, no clonar ni reanalizar el árbol en cada petición (un clon vivo y el árbol en memoria por commit), medido antes en el clúster | 1: el origen de 2.000 tablas por debajo de lo que tarda `ore validate`; 2: una petición de victor cerca de su red |
| **E6 · lo tabular** (F4) ✅ | `leer` de una `Table` con `format` (Parquet por rangos, CSV/JSONL con tipos congelados) a Arrow (0043) | una base standard sobre S3 con los datasets de Olist copiados y las filas cuadradas |
| **E7 · medir borrados** ✅ | qué dan el listado y las versiones (ya activadas en el bucket) ante un borrado, y qué hace con él una colección mantenida y una virtual; el coste de copiar ficheros al lago. (Si la standard copia o sirve en sitio ya no se mide: lo decide la clase, abajo) | informe aquí; decide E8 |
| **E8 · la colección** (F5) ✅ en vivo: la virtual (1), la mantenida (2), su vida (3) | manifiesto de ítems (huella, camino, formato, tamaño, versión), transacción = manifiesto nuevo, puntero `colecciones/*.json` con CAS, copia al lago por contenido o virtual, retención en el mantenimiento | una colección de PDF de S3, en el lago y en sitio |
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

**Lo que E5b·1 midió e hizo.** Cronometrado por tramos (instrumentación temporal) sobre la
copia de victor con el origen de 2.000 tablas, en release y en caliente: cargar el árbol 1,4 s
(2.195 documentos), su catálogo 0,3 s, buscar el puntero de cada fila 3,5 s, y **85–91 s un solo
bucle**: por cada fila, todos los documentos, y en cada uno otra vez lo que lee —en una vista SQL,
su consulta analizada de nuevo—, ~30 ms por fila. Ya dolía pequeño (una fuente Postgres de 48
tablas: 2,1 s en ese bucle). `GET /paquetes` lo repetía por fuente y analizaba cada catálogo tres
veces. Hecho: `punteros::Indice`, el árbol recorrido **una vez por petición** —el paquete de cada
documento, lo que lee cada uno (su consulta, una vez), los punteros por (paquete, objeto) y por
(paquete, prefijo, patrón), quién lee cada puntero, y los catálogos y alcances leídos una vez—, y el
esquema de una fuente, las filas de una base, `GET /paquetes` y el nombre físico de una entidad le
preguntan a él. Dice lo mismo que antes (`el_indice_dice_lo_mismo`, documento a documento, contra
las funciones a las que sustituye). Medido después: el origen de 2.000 tablas, esquema **98 →
1,8 s** y `GET /paquetes` **105 → 1,9 s** (por debajo de los 3,4 s de `ore validate`); victor con
todos sus punteros 0,40 → 0,17 s, demo 0,60 → 0,21 s. Y una guarda que no depende de la máquina:
`el_esquema_de_una_fuente_crece_lineal` —100 y 400 tablas con una vista SQL cada una, lo mejor de
tres; ×4 objetos debe costar menos de ×12—: con el código de antes daba ×46 (160 s) y falla; con el
índice da ×6–8. **No ×4**: resolver cada nombre de una consulta (`linaje::resolver` →
`Package::table`, `view`…) sigue siendo una búsqueda lineal en ore-core que construye el nombre
cualificado de cada documento; con N vistas SQL es N×D. Con 2.000 tablas no se nota (1,8 s), con una
base de miles de vistas SQL sí. Arreglarlo es un índice por nombre en `Package` —25 constructores en
tres crates, y también el compilador—: un paso aparte, medido.

**Lo que queda para E5b·2** (medido desde fuera): en victor, con 82 YAML, `/salud` responde en
0,15 s y `/paquetes` o cualquier esquema en 1,6–1,8 s; cargar ese árbol son ~0,1 s, así que casi
todo es que ore-serve, en modo forja, **clona el árbol en cada petición**. Crece con el árbol (el de
2.000 tablas, clonado en local en Windows, 8 s; sin medir en el clúster).

**Lo que E5b·2 midió e hizo.** Medido dentro del pod de ore-serve de victor (0,5 CPU), con el
árbol de victor (168 ficheros): clonarlo 1,5–1,6 s (`--depth 1`, 0,56 s); sobre un clon vivo, un
`fetch` 0,07–0,10 s, un `worktree` del commit 0,08 s y `ore validate` 0,12 s. Con el de 2.000 tablas
(2.260 ficheros, por un *bundle*, sin red): clonarlo 1,1 s, el `worktree` 0,77 s, `ore validate`
5,2–5,5 s. Hecho (`git.rs`, «El espejo»): un espejo `--bare` por forja en el proceso, puesto al día
con un `fetch` de todas las ramas **en cada petición** —se lee siempre lo último, como antes—; para
leer, el `worktree` del commit, compartido por las peticiones de ese commit mientras alguna lo use
(se guardan ocho; uno ensuciado se rehace); para escribir, un clon local del espejo (enlaces duros,
sin red) con `origin` en la forja, así que publicar, la carrera y el `409` no cambian; y si el espejo
falla, el clon de siempre. La forja sigue siendo el sistema de registro: el espejo es una caché que
se reconstruye sola. Prueba: `el_espejo_lee_lo_ultimo_y_no_se_ensucia` (lo que otro empuja se ve en
la lectura siguiente, dos lecturas comparten árbol, uno ensuciado se rehace, `SinRama`, y lo escrito
llega a la forja). **Sin hacer**: guardar el árbol ya cargado en memoria por commit —con 2.000
tablas cargarlo sigue siendo lo que cuesta—; `Package` no es `Clone` y cambiaría ~29 sitios, así
que va aparte, si hace falta.

**Lo que E6 midió y decidió.** Medido contra el bucket de F1 antes de escribir: lo tabular son 12
tablas, 126 MB y 1,23 M filas (geolocation, 61 MB y 1.000.163; reseñas, 99.224 con 5.495 saltos de
línea dentro de comillas; 146 mil campos vacíos); bajarlo todo, 17 s desde fuera de AWS (5–9 MB/s),
y pasarlo a Arrow en Rust, 0,1–0,5 s por fichero: **lo que cuesta es la red**. Los tipos que el
catálogo dedujo de 64 KB encajan en **todas** las filas de Olist. Hueco hallado: la petición `leer`
no llevaba el `format` de la tabla, así que el driver habría tenido que volver a adivinarlo. Las
cinco decisiones, investigadas en la documentación de cada fabricante:

1. **El formato viaja del árbol al driver** (`Peticion.fichero`: `format` y los tipos congelados,
   en el orden de la tabla). Como Glue/Hive, Trino, el `FILE FORMAT` de Snowflake o Airbyte: nadie
   vuelve a deducir al leer.
2. **Lo que no encaja se rescata**, si la tabla declara `_rescued_data` (enmienda de v1alpha16 `03`
   §1.1, oos `58991c4`): nulo en su columna y su texto, con el fichero, en la rescatada —el *rescued
   data column* de Databricks—. Sin declararla, la lectura **para** con fichero, fila, columna y
   valor (lo de BigQuery y Snowflake por defecto). Nunca un nulo callado. El catálogo de S3 la
   ofrece en todo CSV y JSONL; un Parquet no rescata (su tipo es del fichero; `OOS1004`).
3. **Lo vacío, la regla de `COPY` de PostgreSQL** (`03` §1.2): vacío sin comillas es nulo y `""` la
   cadena vacía; es la que no pierde nada (Spark y DuckDB juntan las dos). Exige un analizador de CSV
   propio, porque `arrow-csv` no ve las comillas; en Olist da igual (0 campos `""`).
4. **Cada fichero se lee fijado a lo que el listado dijo** (`If-Match`): S3 es consistente por clave
   y no entre claves; si uno cambia a mitad, `412` y la copia para en vez de mezclar versiones.
5. **Parquet por rangos, con umbral** (16 MB): por debajo, un GET; por encima, el pie por sufijo y,
   grupo de filas a grupo, sólo los trozos de las columnas pedidas, juntando los que distan menos de
   1 MB. Probado con un Parquet sintético (el bucket es de sólo lectura): 5 peticiones y menos de una
   décima de los bytes.

Hecho (`ore-read-s3/src/filas.rs`, `ore_s3::abrir`/`rango_de`, `ore_driver::Fichero`): `leer`
contesta sólo en Arrow —la copia es la única que pide filas a un origen—, con el tipo de cada valor
analizado por `Fisico::analizar`, la misma forma canónica con la que el almacén estrecha; filtros de
igualdad en su tipo; particiones Hive del camino; `match` como *glob*. **La prueba**
(`pruebas-de-fuego/s3-leer.sh`, contra el bucket real): una base standard de las 12 tablas, copiada
en 28 s con **exactamente** las filas contadas con pyarrow, ninguna columna sin estrechar, los tipos
de Iceberg los congelados (`timestamp`, `decimal(12, 2)`, el código postal `string`), los 87.656
títulos vacíos nulos, 0 rescatadas, y la segunda pasada sin leer nada (0,8 s).

**Lo que E7 midió (2026-09-29).** El bucket de F1 con el versionado ya activo, y un experimento
del usuario en su raíz (subir, sobrescribir, borrar, renombrar, duplicar), leído con la clave de
sólo lectura. Lo que dice S3:

| hecho | lo que se ve |
|---|---|
| lo subido antes de activar el versionado | versión `null`, legible por versión como cualquier otra |
| sobrescribir `a.pdf` | una versión nueva; la vieja sigue legible por su `VersionId` (206); el ETag y el CRC64NVME cambian |
| borrar `b.pdf` | una **marca de borrado**: `ListObjectsV2` ya no la lista, `HEAD` sin versión da `404` con `x-amz-delete-marker: true`, y la versión anterior se sigue leyendo por su `VersionId` |
| renombrar `c.jpg` → `c2.jpg` | una marca en `c.jpg` y una versión nueva en `c2.jpg` **con el mismo CRC64NVME** |
| `d.pdf`, copia de otro | otra clave con **el mismo CRC64NVME**: tres claves (`Receipt…`, `a.pdf`, `d.pdf`) son un contenido |
| listar | `ListObjectsV2` trae clave, ETag, tamaño y fecha, **no** el checksum; `ListObjectVersions` trae además `VersionId`, `IsLatest` y las marcas, en la misma petición (98 ms para 37) |
| el checksum | `HEAD`/`GetObjectAttributes` con `ChecksumMode`: una petición por objeto, sólo para lo nuevo o lo cambiado |

El catálogo de ORE (E4) ve el **ahora** y nada más: tras el experimento salen dos conjuntos
nuevos en la raíz (`raiz_pdf`, 4; `raiz_jpg`, 2) y `b.pdf`/`c.jpg` simplemente no están. **La
historia —qué se retiró, qué se reemplazó, qué es el mismo contenido— sólo existe si alguien
compara dos listados**, y eso es la colección.

**El sector** (documentación oficial de cada fabricante): ninguna colección *virtual* se entera
de un borrado (Foundry: el ítem queda y no se lee; el `FILE` de Snowflake no se actualiza;
BigQuery fija la *generation* y falla si ya no está); las *copiadas* conservan y purgan con su
retención (Foundry: lo sobrescrito o borrado vive N días en la historia y una referencia guardada
sigue mostrando el original; lakeFS y DVC: por contenido, con recolección de basura). Una
sincronización APPEND no retira nada; SNAPSHOT y el `REFRESH` de Snowflake reflejan el origen. Los
eventos de S3 llegan «al menos una vez», duplicados y desordenados, y los configura el cliente:
no son la fuente de verdad. **El coste de copiar**: salir de S3 (`eu-north-1`) $0,09/GB (100 GB
al mes gratis), `GET` $0,0004 por mil; entrar en GCS, gratis; guardar, ~$0,02/GB-mes. 100.000
PDF de 500 KB son 50 GB: ~$4,5 la primera vez, y después sólo lo que cambia. Bajar, 11 MB/s por
flujo desde fuera de AWS.

**Lo que decide para E8:**

1. **Una transacción es la diferencia de dos listados de versiones** (`ListObjectVersions`): lo
   nuevo entra; lo que desaparece se **retira** (sale de la vista actual y vive `retention`); lo
   sobrescrito es retirar y entrar; el checksum se pide sólo para lo nuevo. La vista actual es el
   origen (SNAPSHOT) y la historia no pierde nada (Foundry): las dos cosas a la vez, que nadie da.
2. **La copiada guarda por contenido**: un blob por contenido en el lago del inquilino; un
   renombrado o un duplicado no se vuelve a bajar (en el experimento, la segunda transacción no
   bajaría **ni un byte**: `a.pdf` nuevo, `d.pdf` y `c2.jpg` son contenidos que ya estaban). La
   huella del ítem es el CRC64NVME del origen (la `checksum` del `ObjectTable`, spec `02` §5); el
   blob se nombra por sha256, calculado al copiar (64 bits no bastan como dirección en todo un
   inquilino).
3. **La virtual fija la versión de cada ítem**: si el origen versiona, un ítem retirado se sigue
   sirviendo por su `VersionId` mientras exista (lo decide el ciclo de vida del cliente, no ORE);
   si no versiona, un ítem cuyo objeto desaparece queda **perdido**, dicho, y no se finge.
4. **Manifiesto primero, puntero después, con CAS**, como la copia de un dataset.
5. **La recolección de basura, en el mantenimiento**: un blob se borra cuando ningún ítem vivo lo
   nombra y su retención venció (el criterio de lakeFS).

**E8·1a · la forma del manifiesto (medida, 2026-09-29).** Cuatro formas contra `ore-store-r2`
(release) y el S3 de mentira, con 100 / 10.000 / 100.000 ítems y transacciones de tres cambios:
Iceberg con una fila por ítem y `upsert` —0,64 s y 9,7 MB por transacción con 100.000; leer lo
actual, 0,18 s—; Iceberg como registro de eventos —4 KB por transacción, pero leer lo actual 2 s
y creciendo con la historia—; JSON entero (6,5 MB, sin snapshots ni CAS); JSON de cambios (una
cadena que rehacer). **Decidido: el manifiesto es una tabla Iceberg de sus ítems**, una fila por
(camino, versión) con su estado (`actual`, `retirado`, `perdido`), la transacción en que entró y en
la que salió. Lo frecuente —leer lo actual— es barato; escribir sólo pasa cuando el listado cambió;
la historia son las filas retiradas (los snapshots caducan como los de un dataset); y hereda el
puntero con CAS, `volcar`, `/v1` y SQL. **Límite**: el `upsert` del lago es *copy-on-write* y
crece lineal (~6 s y ~100 MB por transacción con un millón de ítems, extrapolado); si una colección
se acerca, *merge-on-read*.

**E8·1b y 1c · el listado de versiones y la transacción (hecho, 2026-09-29).**
`ore-read-s3 versiones` (`versiones.rs`; `ore_s3::listar_versiones` y `cabeza_de`): lo vigente de un
`ObjectTable` —con su `match` y el de la colección— con versión, ETag, tamaño y huella
(`crc64nvme:…`, un `HEAD` sólo para lo que no se conocía), qué versiones conocidas siguen
existiendo, y el testigo del listado. Contra el bucket: 4 PDF de la raíz, 4 huellas y 2,4 s el
primer pase; el segundo, **0 huellas** y 0,6 s. `ore materialize` hace la transacción de cada
colección virtual (`ore-cli/src/coleccion.rs`): diferencia pura (`transaccion`) entre las filas
de antes y lo vigente —entra, se retira, se pierde, vuelve—, filtrando por `formats`; se sella con
`sellar` fundiendo por (clave, versión) en `colecciones/<base>/<schema>/<n>`; el puntero va con los
de los datasets (`datasets/<base>/<schema>/<n>.json`, `kind: MediaCollection`): comparten el
espacio de nombres del schema, `recoger-huerfanas` lo reclama igual y el Job ya lo empuja, sin
tocar la malla. Si el testigo no cambió, «al día» sin leer el manifiesto. La que copia bytes dice
«pendiente: E8·2». Prueba de fuego `pruebas-de-fuego/s3-coleccion.sh` contra el bucket del
experimento: transacción 1 con `a.pdf` en su versión nueva y tres claves con una huella; al día;
un `match` estrecho retira tres que quedan `retirado` (sus versiones siguen); sin él vuelven, sin
pedir una huella.

**E8·1d · la colección, activo de primera clase (hecho, 2026-09-29).** Al nivel del dataset:
`ore collections` (lista con forma —virtual, mantenida, escrita—, origen, medio y estado del
puntero; `--ficha` con la historia de sus transacciones; `--items` por estado y paginados) y en
ore-serve `GET /colecciones`, `/colecciones/{b}/{s}/{n}` y `…/items?estado=&desde=&limite=` (los
tres parámetros, admitidos en `ore-entrada` y validados); el índice de activos lleva su
`puntero`; `ore datasets` ya no la cuenta aunque su puntero viva con los suyos; `ore view` da su
línea `raíz`, que es la que el Job de la copia lee para abrir su fuente; y la cola encola las
colecciones mantenidas —una base foránea con sólo virtuales también—, sin pedir el conducto a lo
que no copia bytes. **Medido** con un manifiesto de 100.000 ítems: lista 0,03 s, ficha 0,07 s;
los ítems, leídos enteros en texto y filtrados en `ore`, eran **4,1 s** → verbo nuevo
`ore-store pagina` (filtro de igualdad, orden, desde y límite, y el total): **0,6 s**, igual por
HTTP.

**E8·2 y E8·3 · la colección mantenida, y su vida (hecho y en vivo, 2026-09-29).** Antes de
construir se midió **el activo, no el origen** (S3 sólo entrega bytes y una huella): en el GCS de
producción (el bucket de prueba) y desde un pod del clúster con la identidad del Job de la copia.

| medido | dato | decide |
|---|---|---|
| subir con el hash en `x-goog-hash` (`uploadType=media`, lo de `gcs.rs`) | **GCS lo ignora**: un crc32c falso, 200 | multiparte (lo pequeño) o reanudable (lo grande) con el crc32c en los metadatos: 400 y el objeto no existe |
| subir lo que ya estaba (`ifGenerationMatch=0`) | 412 **después** de subirlo entero (32 MiB, 3,2 s); un `HEAD`, 0,05–0,1 s | preguntar antes, por la huella del origen |
| el lago de cada inquilino | cifrado con **su** clave KMS, borrado suave de 7 días, sin versionado, acceso público bloqueado | un blob por contenido **y por inquilino**, nunca entre inquilinos |
| en el clúster (EPYC con SHA-NI) | sha256 1.545 MB/s; subir 64 MiB a 130 MB/s; 20 KiB a 589/s con 64 hilos, sin un 429; rango 206 en ~0,1 s | hashear en flujo no cuesta; paralelo con conexión viva |
| el manifiesto con `blob` y `tipo` | +46 % de bytes (un sha256 no comprime); 100.000 filas: transacción 2,8–3,7 s, página 1,1 s, leerlo entero 2 s | el techo, abajo |
| la recogida, con 1 M de blobs | listar 12.300/s (~80 s), borrar 614/s; el borrado suave se deshace (200) | lo vivo (las filas) contra el listado, con gracia |
| la independencia, con el experimento real | 16 versiones → 10 blobs; la segunda pasada, 0 bytes; `b.pdf` y `c.jpg`: 404 en el origen, 200 en el lago con su sha256 | la mantenida vive sin el origen |
| firmar una URL con la identidad del pod | `signBlob` 403: ninguna cuenta de inquilino tiene `TokenCreator` sobre sí misma | servir es de E9, y necesita IAM |

**El activo.** Una colección mantenida es **un manifiesto** —la tabla Iceberg de sus ítems de E8·1a,
con `blob` (sha256) y `tipo`— **sobre el almacén de blobs del inquilino**: `ore/v2/blobs/sha256/<hex>`,
inmutable, con su `Content-Type`, cotejado por el servidor antes de existir. El origen entrega bytes
por un contrato común (`ore_driver::tramas`: cabecera, bytes, cierre); `ore-read-s3 bajar` baja cada
ítem **fijado a su versión** y coteja su CRC64NVME mientras llega (`ore_s3::huella`); `ore-store
blobs` los guarda (multiparte por debajo de 8 MiB, reanudable desde un temporal por encima, 64 a la
vez). Un índice `ore/v2/blobs/huellas/<sha256(huella, tamaño)>/<sha256>` dice, por la huella del
origen, qué blob es ya —*«dos ítems con la misma huella son el mismo contenido»*, spec `02` §5—.

**La transacción** (E8·2c) busca el blob de cada ítem que entra en el propio manifiesto, después en
el índice (`blobs-hay`) y sólo después en el origen, lo repetido una vez; **sella cuando todos sus
blobs están**; lo que no se pudo copiar no entra, se dice (`no_copiados`) y el testigo no avanza. En
la mantenida nada es `perdido`. `ore collections --cotejar [--muestra N]` (2d) dice que cada blob
nombrado está y mide lo suyo, y vuelve a hashear una muestra; lo roto, con sus ítems, y sale con 1.

**Su vida** (E8·3). Cada fila guarda **cuándo** salió de la vista (`retirado_ms`: la transacción es un
número y los snapshots caducan); el mantenimiento quita lo retirado más viejo que `retention` (3a).
La recogida (3b) se lleva los blobs que **ninguna fila de ningún manifiesto** del inquilino nombra y
nadie tocó en la gracia (2 h, más que un Job), con su entrada del índice; si un manifiesto no se lee,
no se recoge nada. La carrera con un Job que reutiliza un blob que no subió él (por el índice, o de una
fila retirada) se cierra **tocándolo** antes de sellar (un metadato en GCS; una copia sobre sí mismo en
S3). Por encima de **500.000 filas** (3c) el puntero y `ore collections` lo avisan: cada transacción
reescribe el manifiesto (copy-on-write) y toca *merge-on-read*, que no se construye aquí. Las tres
cosas corren en el CronJob de mantenimiento de cada inquilino (`ore collections . --recoger`).

**Un fallo que se vio a tiempo.** `recoger-huerfanas` —el mantenimiento de los datasets— lista todo
`ore/v2/` y borra lo que no está bajo un dataset reclamado: se habría llevado **todos los blobs** la
primera noche. Arreglado (`4c2bb5b`) antes de que ningún lago tuviera uno.

**En vivo, en victor.** La copia de `s3_standard`: contratos (4) y fotos (3) en transacción 1, 7
blobs, **7 de 7 con el sha256 de sus bytes**, sus huellas las de E7 y su tipo; y el mantenimiento a
mano: 7 vivos, 0 recogidos, sin `retention` nada caduca. Ese mantenimiento quitó 7 entradas del
índice que la imagen de antes de E8·3b había escrito con el formato viejo (`huellas/<h>`, sin el
sha256 en el nombre): el índice es una pista, los blobs no se tocaron. (El mensaje de `98bcdca` dice
que ningún lago tenía índice todavía; victor sí lo tenía.) Pruebas de fuego:
`pruebas-de-fuego/s3-coleccion-mantenida.sh` (ocho pasos, del catálogo a la recogida) y
`s3-coleccion.sh` sigue verde.

**Sin hacer, y dónde va.** Servir un ítem (URL firmada o ore-serve pasando los bytes) es E9, con el
acceso en espera y el permiso de firmar por dar. La recogida mira `main`: cuando una rama copie
(sesión paralela), los vivos tienen que ser los de todas. `ore-serve` no enseña todavía el cotejo ni
lanza el mantenimiento de una colección; la consola, E10. *Merge-on-read* si alguna colección pasa el
techo.

**Lo que E1 afinó del texto de v1alpha16** (un caso no puede dejar una regla abierta): una etiqueta
de colección por debajo de la heredada es `OOS4012` (se eleva, no se rebaja), no `OOS4002`; copiar
es un conducto —la mantenida no virtual instancia `materialization.payload` (`OOS4011`, `OOS4002`),
la virtual y la escrita no—; una consulta lee un `ObjectTable` (sus columnas fijas y particiones,
`OOS2018` las demás) y no lee una colección (`OOS2018`); `listing` en una `Table` sin `format` es
`OOS1004`.
