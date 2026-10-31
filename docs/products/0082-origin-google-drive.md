# 0082 · Origen Google Drive

**Leer Google Drive** —«Mi unidad» y las carpetas que un cliente comparte— como origen de objetos de ORE. Es el
caso de [`0061`](0061-origenes-de-objetos.md) que quedaba en la lista (tras SMB): un driver `ore-read-gdrive`
con el contrato de binario y los verbos de siempre (`catalogo`, `versiones`, `bajar`, `check`, `explorar`),
implementando el rasgo `Origen` de `ore-objetos`, con lo que salen las `ObjectTable` y las `MediaCollection`
virtuales y mantenidas.

**Estado:** **X0 medido** con una cuenta personal de Gmail y **pasos 1 a 4 hechos** (2026-10-31; del 3, lo que no pide go). Rama `origen-google-drive`, worktree
`C:\ORE-gdrive`. §2 es lo publicado por Airbyte y Fivetran; §3.1 son nuestras mediciones y §1 lo que de ellas
se propone. **Sin medir:** unidades compartidas y delegación de dominio (exigen Workspace), el tope de `export`
(10 MB, sólo de la documentación), el `429` (120 peticiones seguidas no lo provocaron) y los borrados de verdad
en `changes`. Nada de esto es código todavía.

Sigue el método de [`0070`](0070-origin-cassandra.md), [`0075`](0075-origen-oracle.md),
[`0077`](0077-origin-clickhouse.md) y [`0080`](0080-origin-cockroachdb.md): medir primero, construir después. El
modelo más cercano es SharePoint (0061 D-O5): versiones por API, huella propia, sin URLs firmadas.

## 1. Qué se propone

Cada punto cita la medición de §3.1 que lo sostiene. Se confirman al empezar el paso 1.

1. **Identidad: la cuenta de la celda, como lectora de lo que el cliente comparte** (D-O2/D-O5), con el alcance
   **`drive.readonly`**: `drive.metadata.readonly` no lee bytes y `drive.file` no ve lo compartido (F1). Sin
   clave JSON ni OAuth de usuario. Una cuenta de servicio no puede ser dueña de nada (A3), cosa que ORE no
   necesita. El cliente comparte una carpeta con el correo de la cuenta y le aparece en `sharedWithMe` (C1).
2. **Un lector sólo ve la revisión actual** (B2): una antigua es `404`, aunque el dueño vea las cinco. Pedir lo
   histórico exigiría ser editor, lo que ORE no pide. **La versión de un binario es su `headRevisionId` con el
   `md5Checksum`** (B1), y se lee por esa revisión (`revisions/{id}?alt=media`): si ya no es la actual, `404`, no
   otros bytes. Es el equivalente a `412`, pero con `404`. La actual sí se pide así siendo lector (B2: `200`).
3. **No hay lectura condicional.** `If-Match` e `If-None-Match` se ignoran y no hay `ETag` (B4). Lo que fija es
   la revisión por id (punto 2); lo demás es D-O1: se vigila `headRevisionId`/`md5` al abrir y al terminar,
   como el `cTag` de SharePoint. Colección **virtual** con esa vigilancia y **mantenida** con la copia en el
   lago y su sha256. El rango funciona, también por el final (B3), sin el truco del `HEAD` de Azure.
4. **Huella: `md5:`** (la que ya tiene `ore-objetos`), sin bajar el fichero. **Sin URLs firmadas**
   (`firma: false`): Drive no firma; `ore-medios` sirve los bytes por `leer_fijado`.
5. **Docs, Hojas y Presentaciones nativos: sólo por `export`, sólo colección mantenida** (D1, D2). No tienen
   `md5` ni `size` real (dicen 1 o 1024, D4) ni revisiones para un lector (D5). Tres formatos son
   **deterministas** (`txt`, `pdf`, `csv`: el mismo sha256 dos veces) y dos **no** (`docx`, `xlsx`: el zip lleva
   marcas de tiempo, D3). Propuesta: Docs → `txt` (o `markdown`), Hojas → `csv`/`tsv` por hoja, Presentaciones
   → `pdf`; `docx`/`xlsx` sólo a petición. La versión de un nativo es **`modifiedTime` + `version`** de Drive
   (no hay `md5`), la huella se calcula tras bajar y la dedup por contenido sólo vale en los deterministas.
6. **Rutas: se construyen por `parents`** (C2). El mismo nombre se repite en una carpeta (dos `a.csv`, C3), así
   que **la clave de la fila es el ID de Drive**, no la ruta. Los accesos directos (`shortcut`, C4) se listan
   como tales y no se siguen, como el `remoteItem` de SharePoint. Lo que está en la papelera no lo ve un lector
   (C5), y `trashed=false` se pide de todos modos.
7. **Cambios: `changes.list` sirve y es barato** (E1): un `startPageToken` y luego sólo lo que cambió, con
   `md5`, `trashed` y `removed`. Propuesta: adoptarlo en lugar de listar entero, guardando el token y con un
   recorrido completo como respaldo (los tokens caducan; no medido). Resuelve lo que en SharePoint quedó
   pendiente (`delta`).
8. **Ritmo:** 120 peticiones seguidas no provocaron ni un `403`/`429` (F2). El cliente respeta igualmente
   `Retry-After`, reintenta con espera y se identifica en el `User-Agent`.
9. **Binario propio** `ore-read-gdrive` + librería `ore-gdrive`, sobre `ore_driver::origen`, como GCS, Azure y
   SharePoint; las ramas de «para añadir un proveedor» de 0061, y el puerto es el 443.

## 2. Lo que hacen los demás (publicado, sin medir)

Leído el 2026-10-31 en [Airbyte](https://docs.airbyte.com/integrations/sources/google-drive) y
[Fivetran](https://fivetran.com/docs/connectors/files/google-drive).

| Tema | Airbyte | Fivetran |
|---|---|---|
| Autenticación | cuenta de servicio (JSON) u OAuth (`drive.readonly`) | usuario o cuenta de servicio; etiquetas y grupos exigen usuario (y admin) |
| Qué se sincroniza | una carpeta y sus subcarpetas, glob por ruta | *Magic Folder* (un fichero = una tabla) y *Merge Mode* (patrón → tabla) |
| Cambios | `start_date` por fecha de modificación; no replica borrados | fecha de modificación; Forms e IMPORTRANGE no la actualizan |
| Nativos | Docs → docx, Sheets/Slides/dibujos → PDF | Sheets sí; Docs no dice |
| Permisos | streams de ACL e identidades | tablas de usuarios, grupos y acceso público |
| Límites | Parquet particionado no; copia sin parsear ≤ 1 GB y sólo a S3 | excluye accesos directos, tablas dinámicas y subidas desde la app; no mezcla Sheets y Excel |

## 3. Lo que había que medir (X0)

Contra la API real de Drive v3, como se hizo con GCS y Azure:

- **A. Identidad y cuota:** qué puede hacer una cuenta de servicio; alcance mínimo.
- **B. Binarios:** `md5Checksum`, `revisions`, lectura con rango y `If-Match`, `headRevisionId`.
- **C. Árbol:** `parents`, nombres duplicados, accesos directos, papelera, «Compartido conmigo».
- **D. Nativos:** `export` (formatos, determinismo), `size`, `revisions`.
- **E. Cambios:** `changes.getStartPageToken` / `changes.list`.
- **F. Alcances y ritmo.**

### 3.1 Resultados (2026-10-31)

Laboratorio: proyecto `project-8853a180-450d-47be-b83`, API de Drive activada, cuenta de servicio
`ore-gdrive-x0` **sin clave** (se suplanta con `gcloud --impersonate-service-account`) como lector, y una
carpeta `x0-ore` en el Drive personal de `lunacorunaspain@gmail.com` (cuenta de Gmail, no Workspace),
compartida con ella como *reader*.

| # | qué | Drive de verdad | lo que supone |
|---|---|---|---|
| A1 | `about` con la cuenta de servicio | `200`; su Drive propio vacío, **`storageQuota.limit = 0`** | funciona; no es dueña de nada |
| A2 | crear una carpeta | `200` | una cuenta de servicio sí crea carpetas |
| A3 | subir un binario, o crear un Doc o Hoja vacíos | **`403 storageQuotaExceeded`** («Service Accounts do not have storage quota») | no puede ser dueña de ficheros: el contenido es de un usuario; ORE sólo lee |
| B1 | binario subido por un usuario | `md5Checksum`, `size` y `headRevisionId` en el listado y en `files.get` | huella y versión sin bajar nada |
| B2 | `revisions` de un fichero con 5 revisiones | **el dueño ve las 5; el lector (la cuenta de servicio) sólo la actual**; una antigua por id es `404` | un lector no lee lo histórico |
| B3 | `Range: bytes=0-3` / `bytes=-4` | `206` / `206` (por el final, bien) | rango completo, sin `HEAD` previo |
| B4 | `If-Match: "zzz"` / `If-None-Match: "zzz"` | `200` / `200`, y no hay `ETag` | **no hay lectura condicional** |
| B5 | `version` tras 3 actualizaciones de contenido | 1 → 4 → 6 → 7 → 8 → 10: sube también con cambios de metadatos | `version` no es una versión del contenido |
| B6 | `keepRevisionForever` | el dueño la ve marcada; otras revisiones se conservan sólo un tiempo | la retención de Drive no la controlamos |
| B7 | revisión inventada | `404` | |
| C1 | `sharedWithMe` de la cuenta | la carpeta compartida | así se descubre lo que el cliente comparte |
| C2 | listar los hijos de una carpeta, con `pageSize=2` | `nextPageToken` largo; sin ruta en la respuesta | el árbol se arma por `parents` |
| C3 | dos ficheros `a.csv` en la misma carpeta | los dos, con IDs distintos | el nombre no es una clave |
| C4 | acceso directo | `application/vnd.google-apps.shortcut`, `shortcutDetails.targetId`, **`canDownload: false`** | se lista y no se sigue |
| C5 | fichero en la papelera | `changes` lo da con `trashed: true`; **el lector no lo ve en un listado** | `trashed=false` siempre |
| D1 | `alt=media` sobre un nativo | `400 Unsupported alt type "media"` | sólo `export` |
| D2 | `about.exportFormats` | Docs: txt, pdf, html, markdown, docx, odt, rtf, epub; Hojas: csv, tsv, xlsx, ods, pdf | hay formatos tabulares y de texto |
| D3 | `export` dos veces, 2 s entre medias | `txt`, `pdf`, `csv`: **mismo sha256**; `docx`, `xlsx`: **distinto** | sólo los tres primeros son deterministas |
| D4 | `size` de un nativo | 1 o 1024 | no es el tamaño de nada |
| D5 | `revisions` de un nativo | el dueño ve una (`id` «1»); el lector, ninguna | un nativo no tiene versión que fijar |
| E1 | `changes.getStartPageToken` + `changes.list` | los 3 cambios hechos (atajo, papelera, actualización), con `md5`, `trashed`, `removed` y `time`, más `newStartPageToken` | el incremental barato |
| F1 | alcances | `drive.readonly`: lista, bytes y export bien; `drive.metadata.readonly`: lista bien, bytes y export `403`; `drive.file`: lista bien, bytes `404`, export `403` | el mínimo para leer es `drive.readonly` |
| F2 | 120 `files.get` seguidos | 120 × `200` | la cuota no se nota a este ritmo |

### Lo que X0 no midió

- **Unidades compartidas** (`supportsAllDrives`, `corpora=drive`) y la **delegación de dominio**: exigen
  Workspace y la cuenta de pruebas es de Gmail. Se miden con el primer cliente o con una cuenta de Workspace.
- El tope de **`export`** (10 MB, por la documentación) y los ficheros grandes.
- El **`429`**/`rateLimitExceeded`: no se alcanzó. Los topes por usuario y por proyecto son de la documentación.
- `changes` con ficheros **borrados de verdad** (`removed: true`) y la caducidad de un `pageToken`.
- Una **carrera** de verdad: que el fichero cambie justo entre `versiones` y `bajar` (la revisión pedida
  pasa de actual a antigua y debe dar `404`). Se mide en el paso 1 con la prueba de fuego, como `o1`–`o3`.
- Más de un padre: la documentación dice que ya no se admite; no se intentó añadir uno.

## 4. El laboratorio

- Cuenta de servicio: `ore-gdrive-x0@project-8853a180-450d-47be-b83.iam.gserviceaccount.com`, sin clave;
  `serviceAccountTokenCreator` sólo para `lunacorunaspain@gmail.com`.
- Carpeta de la cuenta de servicio: `x0-raiz`, vacía (se borra con la cuenta).
- Siembra: `x0-ore` en el Drive de `lunacorunaspain@gmail.com` (`1ZcGAslw7gRV4B0-E-aHsPNASBM44YM42`) con dos
  `a.csv` (uno con 5 revisiones), `sub/c.csv`, un Doc y una Hoja nativos, un acceso directo y un fichero en la
  papelera; compartida con la cuenta de servicio como lector. Se sembró tras `gcloud auth login
  --enable-gdrive-access`, que añade el alcance `drive` a la sesión de gcloud de esa cuenta.
- **Al cerrar el laboratorio: borrar la cuenta de servicio, la carpeta `x0-ore` y, si no se usa más,
  desactivar la API de Drive.**

## 5. La hoja de ruta

| paso | qué |
|---|---|
| **1 · hecho** | `ore-gdrive` y `ore-read-gdrive`: URL, identidad, árbol, listar, leer fijado, huella; `check` y `explorar`; el banco es el Drive de verdad |
| **2 · hecho** | los nativos por `export` (Docs → `txt`, Hojas → `csv`, Presentaciones → `pdf`), su versión y su vigilancia |
| **3 · hecho en el código** | cablear el resto de ORE: `ore-medios` (`gdrive://`), `ore-serve` (alta, `credenciales`), la pasarela; falta, con go, la malla (§ «Paso 3») y el banco del kit (paso 5) |
| **4 · hecho** | `changes.list` en lugar de listar entero, con el token guardado (en el proceso) |
| 5 | la prueba de fuego (`o6-gdrive.sh`) con un Drive de mentira para el CI, incluida la carrera entre `versiones` y `bajar` |
| 6 | la hoja de la consola |

### Paso 1 · hecho (2026-10-31): el origen y su lector

- **`ore-gdrive`** (librería, 5 dependencias, `ureq` como los demás): la URL
  `gdrive://<id de la carpeta>[/<prefijo>][?endpoint=…]`; el árbol por `parents`, con los repetidos como
  `a~<id>.csv` y las barras de un nombre como `%2F`; `Item` con su clase (carpeta, fichero, nativo, acceso
  directo, otro); el listado paginado; leer una revisión con rango; `md5Checksum` → `md5:<base64>`; las esperas
  por `429`, `5xx` y el `403` de ritmo (no el de cuota de almacenamiento, que no se arregla esperando); el rasgo
  `Origen` entero (`listar`, `rango`, `abrir`, `rango_de`, `listar_versiones`, `huella_de`, `abrir_version`,
  `leer_fijado`). El validador de un fichero es **`<id>@<revisión>`**, y también lo que el listado da como
  versión y como `etag`.
- **`ore-read-gdrive`**: `driver::main` de `ore-read-objetos` con `fija: version`, `firma: false`,
  `huella: md5` y `credencial_corta`. Su `check` prueba cinco cosas en cadena (`identidad`, `carpeta`, `listar`,
  `leer`, `versiones`) y dice cada fallo con su arreglo; en particular, que Drive contesta `404` igual a una
  carpeta que no existe y a una que no se ha compartido, y que una API de Drive apagada no es un permiso.
- ⭐ **Lo que X0 no había medido, y el paso 1 sí:** el token del metadata server no trae el alcance de Drive
  (sólo `cloud-platform`), así que la cuenta que corre **se suplanta a sí misma** con `drive.readonly`
  (`generateAccessToken`, una hora). Necesita `roles/iam.serviceAccountTokenCreator` sobre sí misma, que
  `ore-medios` ya tiene (GC0). Probado en el laboratorio por el mismo camino, con `ORE_GCP_TOKEN` (el token de
  quien suplanta) y `ORE_GDRIVE_CUENTA`. **Sin probar en una celda**: el metadata server y la autosuplantación
  de `ore-driver-<celda>`.
- **Los nativos** se listaban pero no se leían: el paso 2 los lee.
- **Medido contra el Drive de verdad** (carpeta `x0-ore`, cuenta de servicio como lectora):
  - `check`: las cinco comprobaciones `ok`, `fija: version`, el prefijo `sub/`.
  - `explorar`: `sub/` con su URL y 4 objetos sueltos; el acceso directo, contado como saltado.
  - `catalogo`: 3 tablas CSV con su esquema inferido leyendo por rango (8 peticiones, 2 KB), la de `sub/c.csv`
    como `sub.c`.
  - `versiones`: la versión `<id>@<revisión>` y la huella `md5:…` de cada fichero, sin bajarlo.
  - `tests/laboratorio.rs` (con `ORE_LAB_GDRIVE`): el árbol (5 ficheros, 2 nativos, 1 acceso directo saltado,
    la papelera sin ver), los dos `a.csv` distintos, leer fijado, el md5 de Drive = el de los bytes, rango y
    rango por el final, una revisión que no está → `Rechazo::Cambiado`, un nativo → error claro, y una carpeta
    que no está → error y no un listado vacío.
- **Tests sin red:** 12 en el paso 1 (la URL y su guarda de `endpoint`, la paginación, la huella hex → base64, los nombres
  repetidos y los nativos, la clasificación de los fallos, las capacidades).

### Paso 2 · hecho (2026-10-31): los nativos, por `export`

**Medido antes de escribirlo** (Drive de verdad, `export` de un Doc y de una Hoja):

| qué | resultado | lo que supone |
|---|---|---|
| `Range: bytes=0-3` sobre un `export` | **`200` con el cuerpo entero**: Drive lo ignora | no se puede pedir un trozo: se exporta entero y se corta |
| `Content-Length` de un `export` | lo da (13) | el tamaño se conoce al exportar, no al listar |
| `txt` de un Doc | empieza por un **BOM UTF-8** (`EF BB BF`) que el Doc no tiene, y pierde el salto de línea final | se quita el BOM; lo que sale es lo que se sirve y lo que se hashea |
| `csv` de una Hoja | CRLF entre filas y **sin salto al final** | tal cual; el lector de CSV ya lo admite |

**Qué hace `ore-gdrive`:**
- `Gdrive::exportar(item)`: exporta entero en memoria (tope de 10 MB, el de Drive; más es
  `403 exportSizeLimitExceeded`), quita el BOM del `txt`, y **vigila**: al terminar mira el `version` del
  fichero, y si cambió la exportación no es la del validador y falla con `404` (el idioma de «cambió»), nunca
  da otros bytes. Guarda las últimas 8 exportaciones por validador, para que el rango, la huella y la bajada
  salgan de una sola.
- El validador de un nativo es **`<id>#v<version>`** (el `version` de Drive sube con cualquier cambio, también
  de metadatos: más conservador que una revisión, y una copia de más es lo peor que pasa). La huella es el
  `md5` **de lo exportado**: no hay otra. Un nativo es siempre una versión única y vigente (X0 D5).
- El rasgo `Origen` hace con un nativo lo mismo que con un binario: `listar` (con su extensión de exportación),
  `rango` y `rango_de` (cortados de lo exportado), `abrir`, `abrir_version` (con tamaño, tipo y huella),
  `huella_de`, `listar_versiones` y `leer_fijado` (`200`/`206` con `content-range`, `416` si no cabe, y
  `Cambiado` si el validador ya no es el vigente).
- `check` prueba un nativo exportándolo, y `explorar` los cuenta como ficheros.

**Medido contra el Drive de verdad:** el test de `tests/laboratorio.rs` ahora cubre los dos nativos (rango del
Doc `Hola`/`Drive`, la Hoja entera con CRLF, la huella = el md5 de lo exportado, `206` con
`content-range: bytes 0-2/13`, `416`, un validador viejo → `Cambiado`, 5 versiones en la carpeta). Por el
binario: el catálogo da 4 tablas y 1 conjunto de objetos (la **Hoja es una tabla** `default.hoja_nativa` con
`a` y `b` enteras y 2 filas; el Doc, un conjunto de objetos) y `versiones` da la huella de cada nativo.
**15 tests sin red** y el de laboratorio.

**Lo que no cubre, dicho:**
- El `tamano` de un nativo en el listado es **0** hasta que se exporta (X0 D4): exportar todos al listar
  costaría una exportación por fichero. Si algo del catálogo dependiera del tamaño, se verá en el paso 3.
- Una Hoja con **varias pestañas** exporta sólo la primera a `csv`: es lo que Google documenta, y no se midió
  con una Hoja de dos pestañas. Una por pestaña pide la API de Sheets (otro alcance): queda para cuando se pida.
- El tope de 10 MB y las Presentaciones (`pdf`) no se ejercitaron; el `pdf` de un Doc era determinista (X0 D3).

### Paso 3 · hecho (2026-10-31): el resto de ORE lo conoce

Lo que 0061 lista para «añadir un proveedor», punto por punto:

| dónde | qué |
|---|---|
| `ore-medios` (`origenes.rs`) | la rama `gdrive://`: el cliente se guarda por URL entre peticiones (no lleva secreto), con su token de Drive y lo ya listado. Sin URLs firmadas: los bytes pasan por `leer_fijado`, y la galería en vivo dice que esta fuente no da URL |
| `ore-serve` (`cola.rs`, `url_sin_secreto`) | `gdrive://<id>[/<prefijo>]` viaja en claro por la cola, **sin ningún parámetro** (ni `endpoint`: daría el token al portador a otro servidor) ni usuario ni `#` |
| `ore-serve` (`datasets.rs`, `canjeador_de`) | `gdrive` no canjea nada: quien lee obtiene su token |
| `ore-serve` (`credenciales.rs`) | `GET /fuentes/credenciales/gdrive`: las dos cuentas de la celda (`--cuenta-driver`, `--cuenta-medios`) con los pasos para compartir la carpeta como *Lector* y de dónde sacar su ID; **no admitidos**: la clave JSON y el OAuth de un usuario (la ingesta no puede depender de una sesión); sin una cuenta, se dice cuál falta |
| `ore-conector-kit` (`pasarela.rs`) | `gdrive` es una familia de almacén de objetos: tiene colecciones por la pasarela |
| `ore-cli` | el esquema `gdrive://` ya daba el tipo `gdrive` (y el lector `ore-read-gdrive`) sin tocar nada; queda con su test |

**Sin cambios, y por qué:** `ore-federation` y la pasarela no listan los tipos (los toman de los lectores
instalados); `huella_fiable` no cambia aún (ver abajo); `SIN_VERSIONES` no incluye `gdrive`, que sí fija por
revisión; ni hay `ore-firmar-gdrive` (Drive no firma, `firma: false`).

**Una cosa que corregí al cablear:** el error de `endpoint` de `ore-gdrive` repetía el valor recibido; una URL
con credencial dentro (`http://u:p@host`) la habría devuelto por la API. Ya no lo repite, y `ore-medios` lo
comprueba (`el_esquema_dice_el_proveedor_y_la_url_no_se_repite`).

**Probado:** `ore-gdrive` (11 sin red + el de laboratorio), `ore-read-gdrive` (3), `ore-medios` (`gdrive://` se
acepta sin secreto y un `endpoint` ajeno da `502` sin repetir nada), `ore-serve` (URL sin secreto, canje,
asistente: 3 tests nuevos), `ore-cli` (`esquema`, y las 14 invariantes de `dependencias.rs`) y `ore-conector-kit`
compila. Clippy y `fmt` limpios en lo tocado. **La suite de `ore-serve` tiene 11 fallos que ya tiene
`origin/main`** (`arbol::transforms` y `dos_fases`, git y rutas en Windows): 363 pasan allí, 364 aquí.

**Queda, y pide go (toca la malla y el aprovisionador):**
- ⭐ **La autosuplantación en una celda.** `ore-driver-<celda>` y `ore-medios-<celda>` tienen que poder pedir un
  token de Drive suplantándose a sí mismas: `roles/iam.serviceAccountTokenCreator` sobre sí mismas. `ore-medios`
  ya lo tiene (GC0); **el driver, sin comprobar**. Es el único cambio de infraestructura que Drive pide, y no se
  ha hecho ni probado.
- La API de Drive activada en el proyecto de la celda (`gcloud services enable drive.googleapis.com`; en el
  laboratorio ya está). Si no, `check` lo dice (`accessNotConfigured`).
- El 443 de salida ya está abierto en la malla (`20-driver.yaml`, `45-ore-medios.yaml`); `www.googleapis.com` no
  añade puerto.

**Queda, de código:**
- El **banco del kit** (`bancos/gdrive.rs`) y su prueba de fuego necesitan un Drive de mentira: es el paso 5,
  como Graph tiene el suyo. Hasta entonces el CI no ejercita `ore-read-gdrive`.
- `huella_fiable("gdrive")`: Drive da MD5, como Azure, que sí está en la lista. Se añade cuando la colección
  mantenida de Drive se pruebe de punta a punta (paso 5): una huella que distingue duplicados es una promesa.

### Paso 4 · hecho (2026-10-31): el árbol sigue los cambios

Listar una carpeta de Drive es una petición por carpeta, y cada `listar` de un proceso que vive repetía todas.
Ahora `Gdrive::recorrer` arma el árbol **una vez** y luego lo pone al día con `changes.list` (X0 E1).

**Cómo:**
- **La primera vez:** se pide el `startPageToken` **antes** de recorrer (lo que cambie mientras se recorre se
  repite después, y repetir un cambio no estropea nada) y se recorre el ámbito de la fuente (la carpeta de su
  prefijo) con las mismas peticiones de siempre, más una.
- **Después, una petición** (`changes.list` desde el token guardado, paginada) y se aplica al índice:
  un fichero nuevo, cambiado o renombrado se anota por su ID; uno borrado, en la papelera, o **movido fuera del
  ámbito** se quita **con todo lo que colgaba de él**; una carpeta que **entra** (creada, o movida desde fuera) se
  recorre entera, porque sus hijos no cambiaron y el *feed* no los menciona.
- **Respaldo:** si el *feed* falla (un token caducado, un `4xx`), se tira el índice y se vuelve al recorrido
  completo: nunca se sirve un árbol dudoso. Lo que cae fuera del ámbito (un `listar` por encima del prefijo de la
  URL) se recorre sin índice, como antes.
- Las rutas se rearman del índice entero cada vez, con los repetidos como `a~<id>.csv`, y una carpeta que no está
  sigue siendo un error (`is not in the Drive folder`), no una lista vacía.
- `ore-gdrive::recorridos()` cuenta árboles armados enteros y puestas al día; `Gdrive::token_de_seguimiento()`
  deja ver el token.

**Lo que esto es, y lo que no:**
- **Vive en el proceso.** Un driver de una sola pasada (`catalogo`, `versiones`, `check`) paga **una petición de más**
  (el token) y nada más; lo gana quien se queda: `ore-medios`, que lista una y otra vez para servir una colección
  virtual, y `servir`. Persistir el token entre pasadas pediría estado del lado de ORE (en qué punto del *feed*
  quedó la última copia): no se hace, y el testigo de la copia (`listing`) sigue siendo el hash del listado.
- **El *feed* es el de todo lo que la cuenta ve**, no el de la carpeta: un cambio en otra carpeta compartida se
  mira y se ignora (cuesta la página, no el recorrido).
- **El `saltados` es el de todo el ámbito**, no el de la ruta pedida.

**Medido contra el Drive de verdad** (`tests/laboratorio.rs`, escribiendo como el usuario, con
`ORE_LAB_GDRIVE_ESCRIBE`): crear una carpeta y un fichero dentro, cambiar su contenido (otra revisión, otro
validador), renombrarlo, mover **desde fuera** una carpeta con un fichero (entra con su contenido), mandar un
fichero y luego una carpeta entera a la papelera: en cada paso el árbol del lector lo refleja y el token avanza, y
al final es el de antes. 41 s los dos tests (el *feed* tarda unos segundos; se consulta cada 3 s, hasta 2
minutos). Dejan el Drive como estaba. **7 tests sin red** de `cambios.rs` (el índice, el borrado en cascada, la
carpeta que entra, los repetidos y la paginación del *feed*).

**Sin medir:**
- El **ahorro**: la carpeta del laboratorio tiene 3 carpetas, y una petición frente a tres no enseña nada.
  Importa con árboles de cientos de carpetas, y hay que medirlo con uno.
- El **respaldo** (un token que Drive rechaza) está escrito pero sin ejercitar contra Drive: no se sabe qué `4xx`
  da un token caducado (la documentación dice que no caducan mientras haya cambios que ver, y que se invalidan
  tras unos días sin usarse).
- Un árbol con **unidades compartidas** (`spaces=drive` y `includeItemsFromAllDrives` están puestos; Workspace no).

**Lo que queda feo y se arregla después:** el nombre de una tabla de un fichero repetido sale de su ID
(`default.a_15_5fy8…`); es correcto y ilegible, y lo resuelve quien dé de alta (la consola).

**Para quien quiera repetirlo:** `ORE_GCP_TOKEN=$(gcloud auth print-access-token)`,
`ORE_GDRIVE_CUENTA=<la cuenta>`, y `echo '{"url":"gdrive://<id>"}' | ore-read-gdrive check`.

## 6. Fuentes

- [Airbyte · Google Drive](https://docs.airbyte.com/integrations/sources/google-drive)
- [Fivetran · Google Drive](https://fivetran.com/docs/connectors/files/google-drive)
