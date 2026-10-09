# 0061 · Orígenes de objetos — más allá de S3

**Estado:** propuesto (2026-10-09). O0 en curso. Investigación:
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
4. **D-O1 · leer sin versionado** (R2, ADLS con namespace jerárquico, SFTP) *(propuesta, por
   decidir)*: una colección **virtual** sólo sobre un origen que fije al menos por ETag (`If-Match`):
   si el objeto cambió, `412 media/cambiado`, nunca otros bytes. Uno que no fija nada (SFTP) sólo
   como colección **mantenida**: la versión es la copia en el lago, con su sha256.

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

## Lo que no se hace aquí

- Escribir en un origen: un origen se lee; lo que ORE escribe va a su lago (0049 B4b).
- SMB, Google Drive, HDFS, Box, Dropbox: después de O5, si se piden.
- Una política de red por fuente (lo que propone `20-driver.yaml`): con O4, si el 22 lo pide.
