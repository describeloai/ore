# 0061 · Orígenes de objetos — más allá de S3

**Estado:** propuesto (2026-10-09). **O0, O1, O2 y O3 hechos** (2026-10-09; O2 y O3 en el
laboratorio, la prueba contra GCS y Azure de verdad es deuda temporal); siguiente O4 (SFTP). D-O1
aplicada en O1, D-O2 en O2, D-O3 en O3. Investigación:
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

6. **D-O3 · cómo entra ORE en el Azure del cliente** *(aceptada, 2026-10-09; investigación §6)*:
   **las cuentas de la celda, federadas** —lo que hacen BigQuery Omni y Storage Transfer Service, y
   lo que ORE ya hace con AWS—. El cliente crea en su tenant una app registration (o una managed
   identity) con dos *federated identity credentials* (issuer `https://accounts.google.com`, subject
   = el ID único de `ore-driver-<celda>` y el de `ore-medios-<celda>`, audiencia
   `api://AzureADTokenExchange`) y le da `Storage Blob Data Reader` sobre el contenedor, y
   `Storage Blob Delegator` sobre la cuenta para que `ore-medios` firme. La URL nombra la app, no un
   secreto: `az://<cuenta>/<contenedor>[/<prefijo>]?tenant=<id>&cliente=<id de la app>`. Quien lee
   (el driver, `ore-medios`) canjea él mismo su ID token de Google por uno de Entra; `ore-serve` no
   canjea nada. **No se admiten** la clave de la cuenta (lo abre todo, y las organizaciones la apagan),
   una SAS que traiga el cliente (un secreto al portador que caduca) ni el secreto de una app. Cada
   blob se fija por `versionId` **y** su ETag a la vez (`If-Match`, lo de O3·0); sin versionado —y en
   ADLS Gen2, que no lo tiene— por ETag, con D-O1. Las URLs, SAS de delegación de usuario fijadas a
   la versión (`sr=bv`); por ETag, ninguna (D-O1). La huella, `md5:` si el blob la tiene.

7. **D-O4 · cómo entra ORE en el SFTP del cliente** *(aceptada, 2026-10-09; investigación §7)*:
   - **Identidad: una clave Ed25519 que genera ORE** —por celda, la privada en el cofre— y de la que
     el cliente sólo ve la pública, que pega en `authorized_keys` (lo que hace Fivetran; nada del
     cliente que guardar). De recurso, y dicho como tal: una privada que traiga el cliente o una
     contraseña, en el cofre como la clave de S3.
   - **La huella del host, siempre fijada** (SHA-256): la da el cliente, o la prueba del alta la
     captura y el usuario la confirma. Si cambia, se niega —nada se copia— y el error enseña la nueva;
     volver a fijarla es un acto explícito. Ningún «aceptar cualquiera» (Airbyte no verifica: no es
     el estándar).
   - **Sólo colección mantenida** (D-O1): la versión es la copia en el lago con su sha256. Un fichero
     se copia si lleva más de una **edad mínima** sin cambiar (60 s por defecto, configurable) y se
     vuelve a mirar al terminar de leerlo: si cambió el tamaño o el `mtime`, la copia se descarta
     (medido en O4·0: reescrito en sitio, la lectura mezcla los dos contenidos). Un reescrito del
     mismo tamaño en el mismo segundo no se ve: lo cubre la edad mínima.
   - **Enlaces simbólicos, no se siguen** (se listan como tales y se saltan): un enlace puede salir
     del directorio o hacer un ciclo.
   - **Red**: el puerto 22 (o el que diga la URL) de salida desde los drivers, y la **IP de salida
     fija de la celda** (Cloud NAT), que el cliente pone en su lista de permitidos. Ambas, con go.
   - **Biblioteca: `ssh2`** (libssh2 1.11.1, con *strict KEX*): síncrona como el resto de los drivers,
     21 crates frente a 212, y el mismo resultado que `russh` en O4·0. El precio es código C; se fija
     la versión y se sigue a libssh2. `russh` queda como alternativa si un día hace falta Rust puro.
     Algoritmos modernos por defecto; `ssh-rsa` con SHA-1 sólo con un parámetro explícito por fuente.

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

### O2 · hecho en el laboratorio (2026-10-09): GCS

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

**O2·3, hecho**: GCS cableado al resto de ORE.

- `ore-medios` lee `gs://` (`origenes.rs`) y guarda el cliente por URL —que no lleva secreto—
  entre peticiones, con su token (y el suplantado, una hora): sin eso sería una ida a IAM por ítem.
  El rasgo vale también por `Arc`. Lee con su cuenta, `ore-medios-<n>`, que ya firma como sí misma
  desde B2: GCS no pide IAM nuevo en la celda.
- ⭐ **El token es al portador.** Una URL con `?endpoint=` que no sea `https://…googleapis.com` se lo
  daría a otro servidor: `ore-gcs` la rechaza salvo con `ORE_GCS_LABORATORIO=1` (el laboratorio y el
  CI), y la comprobación antes del alta (`cola.rs`) sólo deja pasar `suplantar`.
- `ore-serve`: `gs://` se comprueba antes del alta como BigQuery (sin secreto, tipo `gcs`); no se
  canjea nada (`canjeador_de`: quien lee, el driver o `ore-medios`, obtiene su token él mismo); y
  `GET /fuentes/credenciales/gcs` enseña las dos cuentas —la del driver y la de medios, nueva
  `--cuenta-medios`— con `roles/storage.objectViewer` sobre el bucket, el modo `suplantar`
  (`roles/iam.serviceAccountTokenCreator` sobre la cuenta del cliente) y la clave JSON como no
  admitida.
- La ruta vieja de los datasets (`ore collections --servir`, que firma con `ore-firmar-<tipo>` en la
  imagen de `ore-serve`, sin red) **no** sirve GCS: firmar en GCS es una petición a IAM, y esa imagen
  no habla por la red. Dice 66 (no hay firmante); lo de GCS se sirve por `/media` (`ore-medios`).
- `ore-federation` carga `gcs` por defecto; `ore-read-gcs` va en la imagen de drivers
  (`/opt/ore/conectores`) y en `ci/compilar-binarios.sh`.
- El kit del conector gana el banco `gcs` (la misma semilla que S3, compartida) y el CI lo corre
  contra `fake-gcs-server`. En local: **12/12 directo** (el flujo de 10⁶ filas en 1,7 s) y **9/9 por
  la pasarela**, colecciones incluidas.

Falta, con go: `--cuenta-medios` en `malla/40-ore-serve.yaml` (binario antes que malla).

**O2·4, hecho**: `pruebas-de-fuego/o2-gcs.sh`, contra `fake-gcs-server` con dos buckets —`cubo` con
Object Versioning y `llano` sin él—, por el binario y por `ore-medios` (`tests/laboratorio_gcs.rs`):
`check` (probando, y un bucket que no existe), la guarda del `endpoint` fuera del laboratorio,
`explorar` y el catálogo, `versiones` por generación con `crc32c`, `bajar` cotejado (y con la huella
cambiada no copia), y `docs/a.pdf` reescrito **con el mismo tamaño** entre `versiones` y `bajar`:
`cubo` copia la generación listada y `llano` no copia. **14/14.** Con la fijación por generación
rota a propósito (`Gcs::bajar` sin `generation`), 4 fallos —y ni así se copian otros bytes: el
cotejo de la huella los para—. El perfil, en [`origenes-de-objetos.md`](../origenes-de-objetos.md).

**Deuda temporal de O2**, contra GCS de verdad (bloqueada mientras la cuenta de Google siga
suspendida): `testIamPermissions`, la URL firmada por `signBlob` bajada con `curl`, la suplantación
(`generateAccessToken`, y su 403 sin `TokenCreator`), la paginación, y la concesión entre proyectos
(un bucket de otro proyecto con `objectViewer` a las dos cuentas de la celda). Se salda corriendo
`o2-gcs.sh` contra un bucket real y la consola dando de alta uno.

### O3 · hecho en el laboratorio (2026-10-09): Azure Blob / ADLS Gen2

**O3·0, medido** (Azurite 3.37.0, `mcr.microsoft.com/azure-storage/azurite@sha256:830430c1…`): lista
con paginación de verdad (`maxresults`/`NextMarker`) y `delimiter`; rango `206`; `If-Match` viejo →
`412`; el `Content-MD5` lo calcula en Put Blob (y sale en el listado y en cada lectura), no en Put
Block List; `BlobNotFound`/`ContainerNotFound`; sin firma, `403`. Por HTTPS con `--oauth basic`: un
Bearer, la clave de delegación y una **SAS de delegación** que se baja con su `rsct`/`rscd`
(manipulada, `403`). **No**: el versionado (no da `x-ms-version-id`, ignora `include=versions`, y
**un `versionid` que no existe le devuelve los bytes vigentes**: por eso el driver fija por versión
*y* `If-Match`), la firma del token (sólo mira emisor, audiencia y fechas), ADLS (`dfs`, `400`) y el
CRC64 de transporte. Contra Azure de verdad, el versionado, `sr=bv`, ADLS Gen2 y el canje con Entra:
deuda temporal, sin cuenta de Azure.

**O3·1, hecho**: `ore-azure` sobre la API REST de Blob. La URL
`az://<cuenta>/<contenedor>[/<prefijo>]?tenant=…&cliente=…` no lleva secreto: el token de Google de
la cuenta que corre (`ore-gcp`, audiencia `api://AzureADTokenExchange`) se canjea en Entra por uno de
Storage (*client credentials* con aserción federada), guardado 45 minutos; los `AADSTS…` de la
federación se dicen en palabras de qué falta (`70021`: la credencial federada no casa, o aún se
propaga). Sólo se habla con `https://<cuenta>.blob.core.windows.net` —el token es al portador—, y
otro servidor, sólo con `ORE_AZURE_LABORATORIO=1`. Listar paginado (con versiones), `HEAD`, leer con
`versionid` **y** `If-Match` a la vez (la guarda de O3·0), la clave de delegación (guardada mientras
vale, hasta 6 días) y la SAS de delegación (`sr=b`, o `sr=bv` con la versión firmada en el hueco de la
instantánea, como el SDK de Azure; 24 campos de `sv=2023-11-03`). `ore-objetos` gana la huella `md5:`.
Contra Azurite (HTTPS, `--oauth basic`): listar, la versión por ETag, el `md5` (y un blob por bloques
sin él), entero, rango, un ETag viejo → `cambiado`, **un `versionid` ignorado con un ETag que no casa
tampoco da bytes**, la SAS del vigente bajada con su tipo y su disposición (manipulada, `403`) y un
contenedor que no está.

**O3·2, hecho**: `ore-read-azure`, el `main` común con lo de Azure. Su `check` **prueba** —Azure no
tiene `testIamPermissions` en el plano de datos— y dice qué falta y dónde: la identidad (el canje en
Entra; si falla, la credencial federada que la app necesita), `listar` y `leer` (`Storage Blob Data
Reader` sobre el contenedor), `versiones` (no decide: dice si se fija por `version` o por `etag`, como
en ADLS Gen2) y `firmar` (la clave de delegación: `Storage Blob Delegator` sobre **la cuenta**; no
decide, sin ella los ítems se abren por `content`). Un contenedor o una cuenta que no existe, y el
firewall de red de la cuenta (`AuthorizationFailure`), se dicen como lo que son y no como un rol que
falta. Contra Azurite, los verbos del binario: `capacidades`, `check` (y un contenedor que no está),
`explorar`, `catalogo` (1 tabla y 4 conjuntos), `testigo`, `versiones` (`etag:` y `md5:`), `bajar`
cotejado, y con la huella cambiada o un ETag que ya no es, no copia.

**O3·3, hecho**: Azure cableado al resto de ORE, como GCS en O2·3.

- `ore-medios` lee `az://`; la caché de clientes por URL de O2·3 vale ahora para cualquier proveedor
  cuya URL no lleve secreto (GCS y Azure: token de Entra y clave de delegación guardados entre
  peticiones; una `s3://` no se guarda).
- `ore-serve`: `az://` se comprueba antes del alta (sólo `tenant` y `cliente`, tipo `azure`), no se
  canjea nada, y `GET /fuentes/credenciales/azure` da los comandos `az` con los **IDs únicos de las
  dos cuentas ya puestos** —un subject mal pegado se crea sin error y falla en silencio—: la app, las
  dos credenciales federadas, `Storage Blob Data Reader` sobre el contenedor y `Storage Blob Delegator`
  sobre la cuenta; no admite la clave de la cuenta, una SAS ni el secreto de una app. El ID de la
  cuenta de medios sale de `ORE_ID_MEDIOS`, que el aprovisionador **todavía no publica** (sin él, los
  comandos salen con su hueco y se dice).
- ⭐ **Medido aquí**: Blob no entiende un rango por el final (`bytes=-8`, el pie de un Parquet; Azurite
  contesta `500`). `ore-azure` lo resuelve con el tamaño del blob (un `HEAD`; con el ETag, si cambió
  entretanto, `412`), también en `leer_fijado` (lo que pida un visor). Lo cazó el caso 9 del kit por la
  pasarela (el catálogo de los Parquet).
- `ore-federation` carga `azure` por defecto; `ore-read-azure` en la imagen de drivers y en
  `ci/compilar-binarios.sh`; el kit gana el banco `azure` (la misma semilla) y el CI lo corre contra
  Azurite por HTTPS con un certificado de un día. En local: **12/12 directo y 9/9 por la pasarela**.

Falta, con go: publicar `ORE_ID_MEDIOS` (el ID único de `ore-medios-<celda>`) junto a los otros dos
en `ids-de-la-celda` (`malla/aprovisionar-inquilino.sh`).

**O3·4, hecho**: `pruebas-de-fuego/o3-azure.sh`, contra Azurite por HTTPS (`--oauth basic`, un
certificado de un día), por el binario y por `ore-medios` (`tests/laboratorio_azure.rs`): `check`
(los cinco pasos, fija por ETag; un contenedor que no existe), la guarda del `endpoint` fuera del
laboratorio, `explorar` y el catálogo, `versiones` por ETag con `md5`, `bajar` cotejado (con la huella
cambiada no copia) y `docs/a.pdf` reescrito **con el mismo tamaño** entre `versiones` y `bajar`: no se
copia (412). **11/11.** Con el `If-Match` quitado a propósito de `Azure::bajar`, 3 fallos —y ni así
se copian otros bytes: el cotejo del `md5` los para—. El perfil, en
[`origenes-de-objetos.md`](../origenes-de-objetos.md).

**Deuda temporal de O3**, contra Azure de verdad (no hay cuenta): el canje con Entra (y sus
`AADSTS…`), el versionado (`include=versions`, leer una versión vieja, `sr=bv` —construida como el
SDK, sin validar—), ADLS Gen2 (namespace jerárquico y sus ACL), el firewall de red, y la concesión de
los dos roles. Se salda corriendo `o3-azure.sh` contra una cuenta real y la consola dando de alta un
contenedor.

### O4 · SFTP (en curso)

**O4·0, medido** (`atmoz/sftp`, OpenSSH 8.4p1, `atmoz/sftp@sha256:09603904…`): clave de host
Ed25519 y su huella SHA-256; la clave autorizada entra, otra no, y sin contraseña tampoco; el
usuario empieza en su chroot (`/`); listar recursivo (7 ficheros, 17 ms); `stat` da tamaño y `mtime`
**en segundos enteros**; leer desde una posición (el pie de un Parquet) sí; sin permiso, `errno 13`;
no está, `errno 2`; **sin `exec`** (`internal-sftp`: no hay huella calculada en el servidor); un
fichero **reescrito en sitio mientras se lee da una mezcla de los dos contenidos**, y uno
**renombrado encima** deja leer entero el de antes; **10 canales SFTP por conexión** (el undécimo,
`Connect failed`) y 31 conexiones a la vez sin problema; 64 MiB en ~0,6 s. Los dos *spikes* de Rust
(`russh` 0.64 + `russh-sftp` 3, y `ssh2` 0.9 con libssh2 1.11.1) hacen lo mismo contra él: la huella
casa con la de `ssh-keygen` en el servidor, con una equivocada se niegan, listan, leen el pie y bajan
a ~110 MiB/s. `russh` trae 212 crates y un runtime asíncrono; `ssh2`, 21 y síncrono.

**O4·1, hecho**: `ore-sftp` sobre `ssh2`. La URL `sftp://<usuario>@<host>[:<puerto>]/<ruta>?huella=…`
lleva la huella del host, que se compara **antes de autenticar** (a un servidor que no es no le llega
ni la clave); sin ella, el error dice cuál es la vista para confirmarla. La clave de la celda se lee
de `ORE_SFTP_CLAVE`; de recurso, una contraseña en la URL (que nunca sale en `publica` ni en los
mensajes). Algoritmos modernos; `ssh-rsa` y los KEX viejos con `legado=1`. Listar recursivo por
prefijo (los enlaces, marcados y sin seguir), `lstat`, leer entero o un rango (también por el final)
**vigilado**: el validador `<mtime>-<tamaño>` se compara al abrir y al terminar. La versión es ese
validador (`etag:…`), `listar_versiones` deja fuera lo que lleva menos de `edad` segundos sin cambiar,
y `leer_fijado` se niega con D-O1. Contra `atmoz/sftp`: la huella (fijada, sin fijar, otra), listar
sin enlaces, versiones, la edad mínima, rangos, un fichero sin permiso, D-O1, y **un fichero
reescrito en sitio a mitad de lectura hace fallar la lectura** en vez de dar la mezcla. Compila en
Alpine (musl, el OpenSSL estático de la imagen).

## Lo que no se hace aquí

- Escribir en un origen: un origen se lee; lo que ORE escribe va a su lago (0049 B4b).
- SMB, Google Drive, HDFS, Box, Dropbox: después de O5, si se piden.
- Una política de red por fuente (lo que propone `20-driver.yaml`): con O4, si el 22 lo pide.
