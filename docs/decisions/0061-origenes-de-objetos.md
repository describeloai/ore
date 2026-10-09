# 0061 · Orígenes de objetos — más allá de S3

**Estado:** propuesto (2026-10-09). **O0 y O1 hechos** (2026-10-09); siguiente O2 (GCS). D-O1
aplicada en O1. Investigación:
[`o-origenes-de-objetos-estado-del-arte.md`](../investigacion/o-origenes-de-objetos-estado-del-arte.md).

## Contexto

Hoy ORE lee tres orígenes de punta a punta: PostgreSQL y BigQuery (tablas) y **S3** (objetos: un
bucket da `Table` con `format` sobre sus Parquet/CSV/JSONL y `ObjectTable` sobre lo demás, de donde
salen las `MediaCollection` virtuales y mantenidas de 0046 y 0049). Lo que el mercado pide como
origen, por lo que soportan Foundry, Databricks Unity, Snowflake y Airbyte, es en este orden:
S3 → **Azure Blob / ADLS Gen2** → **GCS** → **SFTP** → **SharePoint/OneDrive** → Cloudflare R2 →
SMB → Google Drive → HDFS, más los que hablan la API de S3 (MinIO, Ceph, Wasabi, B2, IBM COS, OCI…).

**Lo que hay en el código** (mapeado el 2026-10-09):

- El contrato de un origen **no es un rasgo de Rust: es un binario `ore-read-<tipo>`** (ADR 0008):
  el verbo en `argv[1]`, la URL con su credencial por stdin, la salida por stdout, el error tipado
  por stderr (`ore-driver`). `ore-read-s3` es el único que además sabe `versiones` y `bajar`:
  **esos dos verbos son el contrato de «almacén de objetos»**.
- Dentro de `ore-read-s3`, el catálogo, las filas, el esquema tabular, las versiones y la bajada
  ya trabajan sobre un rasgo interno `Origen` (`&dyn Origen`), no sobre S3; sus tipos (`Objeto`,
  `Version`, `Abierto`, la huella CRC64NVME) son de `ore-s3`.
- **Nueve sitios sólo conocen S3**: el `Origen` de `ore-medios` (`Lago | S3`) y su firma
  (`servicio.rs`), el `tipo != "s3"` de `ore-cli/src/colecciones.rs`, el canje de credenciales de
  `ore-serve` (sólo con `role_arn`), `cola.rs::url_sin_secreto`, el asistente de
  `credenciales.rs`, los tipos por defecto de `ore-federation`, los bancos del kit de conectores,
  el `Dockerfile` y la malla (`20-driver.yaml`, `45-ore-medios.yaml`: sólo el 443 y el 5432).

## Decisión

1. **Un driver nativo por protocolo** (`ore-read-gcs`, `ore-read-azure`, `ore-read-sftp`,
   `ore-read-sharepoint`), con el mismo contrato de binario y los mismos verbos. **Los que hablan
   la API de S3 no llevan driver propio**: son perfiles del de S3 (`endpoint`, *path-style*,
   región libre), con lo que cada uno no implementa dicho en sus capacidades. La API «compatible
   con S3» de GCS no se usa: es parcial (sin listado de versiones, el pin por sus cabeceras).
2. **Lo común se escribe una vez.** Los tipos neutros, el rasgo `Origen` y la lógica que hoy vive
   en `ore-read-s3` (catálogo, filas, versiones, bajada) salen a crates compartidos; cada
   proveedor implementa el rasgo —listar, listar versiones, la huella de una versión, leer fijado
   con rango, firmar— y su comprobación de acceso.
3. **Cada origen dice lo que sabe** en `capacidades`: cómo fija una lectura (`version` | `etag` |
   `ninguna`), si firma URLs, qué huella da sin bajar (`crc64nvme`, `crc32c`, `md5`, ninguna) y si
   su credencial es corta. Quien sirve una colección lo lee de ahí y no promete lo que el origen no
   da.
4. **D-O1 · leer sin versionado** (R2, ADLS con namespace jerárquico, SFTP) *(aceptada con O1,
   2026-10-09)*: una colección **virtual** sólo sobre un origen que fije al menos por ETag (`If-Match`):
   si el objeto cambió, `412 media/cambiado`, nunca otros bytes. Uno que no fija nada (SFTP) sólo
   como colección **mantenida**: la versión es la copia en el lago, con su sha256.

5. **D-O2 · cómo se identifica ORE ante el almacén del cliente** *(aceptada, 2026-10-09; investigación
   §5)*: con **las cuentas de la celda**, que el cliente autoriza sobre su bucket —el estándar del
   sector (Snowflake, BigQuery) y lo que ORE ya hace con S3—; la segunda opción, **suplantar una cuenta
   del cliente** (credencial de una hora), para separar permisos por conexión. Sin claves largas de
   cuenta de servicio, salvo que un cliente lo exija. Nunca una identidad compartida entre celdas: en
   GCS no hay `ExternalId`, y lo que protege es que la identidad sea por celda. Un cliente con
   *Domain Restricted Sharing* necesita una federación en su proyecto: límite conocido, se construye
   cuando se pida.

## El plan

Cada hito se construye y se prueba **en local**, con el emulador de cada proveedor en Docker; la
prueba contra el proveedor de verdad queda como deuda temporal, saldada cuando haya cuenta.

| hito | qué | laboratorio |
|---|---|---|
| **O0 · generalizar** | lo de abajo: S3 sigue igual, y nada asume S3 | los tests de hoy |
| **O1 · los que hablan S3** | perfiles (R2, MinIO/Ceph, Wasabi, B2…); sin versionado, fijado por ETag | MinIO y Ceph/Garage; la conformidad de media sobre una virtual |
| **O2 · GCS** | `ore-read-gcs`: `generation`, `crc32c`, URL V4, federación por `ore-gcp` | `fake-gcs-server` |
| **O3 · Azure Blob / ADLS** | `ore-read-azure`: `versionid` o ETag según el namespace; SAS de delegación; Entra | Azurite |
| **O4 · SFTP** | fijado sintético (la copia en el lago); el puerto 22 en la malla, con su go | `atmoz/sftp` |
| **O5 · SharePoint / OneDrive** | Graph: versiones, `delta`, `quickXorHash` | un tenant de prueba |

### O0 · generalizar, sin cambiar el comportamiento

| paso | qué |
|---|---|
| **O0·1** | **`ore-objetos`** (librería ligera): los tipos neutros (`Objeto`, `Version`, `Abierto` con su `huella` y no un `crc64nvme`), el rasgo `Origen` (lo de `ore-read-s3`, más leer fijado con rango para `ore-medios`, y firmar), las `Capacidades`, las huellas y `EnMemoria` para las pruebas. `ore-s3` implementa el rasgo para su `Bucket` |
| **O0·2** | **`ore-read-objetos`** (librería): catálogo, filas, tabular, versiones, bajada y el bucle de verbos, movidos de `ore-read-s3`. `ore-read-s3` queda en lo suyo: leer la fuente, `check` (qué permiso de IAM falta), `explorar` y su `main` |
| **O0·3** | **`ore-medios`** lee y firma por el rasgo: `Origen::Objeto`, el proveedor por el esquema de la URL de la fuente; uno que no conoce, `501` que lo dice |
| **O0·4** | **El plano de control**, por tipo y no por `"s3"`: el firmante de `ore collections --servir`, el canje de credenciales de `ore-serve`, `url_sin_secreto` y el asistente de credenciales, los tipos de la pasarela (los conectores instalados), los bancos del kit, y las capacidades de objetos en el verbo `capacidades` |

Cada paso entra en verde: los tests de Rust de los crates tocados, `los_objetos_de_un_bucket.rs`,
`dependencias.rs` (las invariantes: `ore-serve` no lee orígenes, el firmante no tiene red), las
pruebas de fuego de S3 contra el S3 de mentira (`s3-*.sh`), la de la puerta y la conformidad de
media en la JVM y en Python.

### O0 · hecho (2026-10-09)

| paso | commit | qué quedó |
|---|---|---|
| O0·1 | `40256a03` | `ore-objetos` (sólo `sha2`): `Objeto`, `Version`, `Abierto` (con `huella`), el rasgo `Origen`, `Fija`, `Capacidades`, la huella (CRC64NVME y el cálculo que casa con una huella dada) y `EnMemoria`. `ore-s3` implementa el rasgo sobre `Cubo(&Bucket)` (el `Bucket` es de `ore-sigv4`) |
| O0·2 | `3c095480` | `ore-read-objetos`: catálogo, filas, tabular, versiones, bajada, medio (movidos con su historia) y `driver::main::<P: Proveedor>()`. `ore-read-s3` es su fuente, `acceso.rs` y un `Proveedor` |
| O0·3 | `f331ed0c` | el rasgo gana `leer_fijado` (con `Rechazo`: cambiado, rango, origen) y `firmar`; `ore-medios` usa `Origen::Objeto` y `origenes::de(fuente)` elige el proveedor por el esquema (otro, `501`) |
| O0·4 | `14d22cd7` | `capacidades` con el nombre de cada driver y `objetos`; el firmante de `ore collections --servir` por tipo (`ore-firmar-<tipo>`); `canjeador_de` en `ore-serve` |

**Probado sin cambio de comportamiento**: los tests de los crates tocados (598 en `ore-cli`,
`ore-serve`, `ore-driver` y `ore-read-objetos`; los 42 de `ore-medios`, que leen una virtual contra
un S3 falso por el rasgo), y **el binario de antes de O0 contra el de ahora**, con el mismo S3 de
mentira, en `catalogo`, `testigo`, `explorar`, `check`, `versiones`, `bajar` y tres errores: byte a
byte igual. Sólo `capacidades` cambia, a propósito (`objetos`). `leer`, `estimar` y `servir` van por
el mismo código movido y los cubren sus tests.

**Para añadir un proveedor** queda, en este orden:

1. su transporte y su `impl Origen` (listar, versiones, huella, `abrir_version`, `leer_fijado`,
   `firmar` si sabe) y sus `Capacidades`;
2. `ore-read-<tipo>`: su fuente (leer la URL y canjear), su `check`, su `explorar`, su
   `Proveedor` y `driver::main`;
3. su rama en `ore-medios/src/origenes.rs`, y en `canjeador_de` (`ore-serve/src/datasets.rs`) si
   su credencial se canjea; un `ore-firmar-<tipo>` sin red si sus URLs se firman fuera de
   `ore-medios` (`ore collections --servir`);
4. las ramas que ya eran por tipo: `url_sin_secreto` (`cola.rs`), el asistente
   (`credenciales.rs`), `--tipos` de la pasarela, el banco del kit;
5. el `Dockerfile` (`/opt/ore/conectores`), `ci/compilar-binarios.sh` y, si su puerto no es el
   443, la malla (`20-driver.yaml`, `45-ore-medios.yaml`), con su go;
6. las invariantes de `dependencias.rs` para lo nuevo, su banco de mentira, y la conformidad de
   media sobre una virtual suya.

### O1 · hecho (2026-10-09): los que hablan S3, como perfiles

**O1·0, medido** en Docker: **VersityGW** (con versionado, como S3) pasa el driver de entonces
entero —`versionId`, CRC64NVME, la URL firmada bajada con `curl`—; **Garage** (sin versionado, como
R2), todo salvo `versiones` y `bajar`: `ListObjectVersions` es `501 NotImplemented`. Garage además
**ignora `versionId`** (uno inventado da la actual) y **respeta `If-Match`** (`412`). MinIO ya no
publica imágenes (Docker Hub y quay): el laboratorio usa esos dos.

| paso | commit | qué |
|---|---|---|
| O1·1 | `596ca2b4` | si el origen no versiona, cada objeto es su única versión, **`etag:<su ETag>`** (`ore_objetos::version_por_etag`): cambia con los bytes, y nunca viaja como `versionId`: cada lectura (`huella_de`, `bajar`, `content`) la convierte en `If-Match`. No se firma su URL (`firmar` → `None`; `ore-firmar-s3` tampoco). `check` dice `fija: version \| etag` y, fuera de AWS, el bucket en vez del ARN |
| O1·2 | (en O1·1) | lo que sabe cada fuente sale de `check`, medido contra ella |
| O1·3 | `84377db6` | `pruebas-de-fuego/o1-los-que-hablan-s3.sh`: los dos emuladores por todos los verbos, la URL firmada con `curl`, la virtual por `ore-medios` (`laboratorio_s3.rs`) y un objeto reescrito **con el mismo tamaño** entre `versiones` y `bajar` (garage no lo copia; versity copia la versión listada): 13/13. Con `etag_de` roto a propósito, garage copia los bytes nuevos y la prueba falla |
| O1·4 | (este) | [`docs/origenes-de-objetos.md`](../origenes-de-objetos.md): la URL, y un perfil por proveedor (endpoint, región, cómo fija), probado o «de su documentación» |

**Deuda temporal**: R2, Wasabi, B2, Spaces, IBM y OCI se conectan por la misma API que los probados,
pero ninguno se ha medido contra una cuenta suya; cada uno pasa a «probado» cuando su URL pase
`o1-los-que-hablan-s3.sh`.

### O2 · GCS (en curso)

**O2·0, medido** (`fake-gcs-server` 1.52.2, `-backend memory`): bucket con versionado, `generation`,
`crc32c`, `md5Hash`, `versions=true` (la vieja con `timeDeleted`), leer los metadatos y los bytes de
una generación vieja, rango `206`, generación que no existe `404`, `x-goog-hash`. **No**: la paginación
(con `maxResults` corta sin `nextPageToken`), `ifGenerationMatch` (lo ignora), `testIamPermissions`
(404) y el token (no lo comprueba). El driver fija por `?generation=N`, que es exacto, y la paginación,
la firma V4 (contra los vectores de Google), la identidad y `testIamPermissions` se prueban fuera del
emulador; contra GCS de verdad, deuda temporal mientras la cuenta de Google siga suspendida.

**O2·1, hecho** (`a06f03a3`): `ore-gcs` sobre la API JSON. La URL `gs://bucket/prefijo` no lleva
secreto (la cuenta de la celda) o dice `?suplantar=<cuenta del cliente>` (`generateAccessToken`, una
hora, `devstorage.read_only`). El validador de cada objeto es su **generación**; listar paginado,
versiones, `crc32c` sin bajar, leer una generación con rango, `testIamPermissions`, y la URL V4
firmada por `signBlob` —su petición canónica casa con los vectores de conformidad de Google—.
`firmar` del rasgo devuelve `Result<Option<_>>` (GCS firma por red y puede fallar), y `bajar`
coteja la huella de cualquier origen con su algoritmo (`crc32c` incluido). Contra
`fake-gcs-server`: todo lo que el emulador sabe, y lo que no (firma, permisos), dicho como tal.

**O2·2, hecho**: `ore-read-gcs`, el `main` común con lo de GCS. Su `check` le **pregunta** a GCS
(`testIamPermissions`: `storage.objects.list` y `storage.objects.get`, que da
`roles/storage.objectViewer`) y aun así lista una página y mira un objeto: manda lo que pasa de
verdad, que IAM puede no ver entero (un perímetro que niega, una ACL de objeto que concede); sin
`testIamPermissions` (el emulador) prueba y dice `"como":"probando"`. Con `suplantar`, el primer
permiso es la identidad (`roles/iam.serviceAccountTokenCreator` sobre la cuenta del cliente). Un
bucket que no existe se dice así, no como un rol que falta. `fija` siempre `version`: las
generaciones viejas se leen con los mismos dos permisos. El rasgo `Origen` vale también prestado
(`&T`), y `ore-gcs` gana `pagina` (una página, para `check` y `explorar`) y sus contadores. Contra
el emulador, los verbos del binario: `capacidades`, `check`, `explorar`, `catalogo` (1 tabla y 3
conjuntos), `testigo`, `versiones` (generación y `crc32c`), `bajar` cotejado, y con la huella
cambiada o una generación que no está, no copia.

## Lo que no se hace aquí

- Escribir en un origen: un origen se lee; lo que ORE escribe va a su lago (0049 B4b).
- SMB, Google Drive, HDFS, Box, Dropbox: después de O5, si se piden.
- Una política de red por fuente (lo que propone `20-driver.yaml`): con O4, si el 22 lo pide.
