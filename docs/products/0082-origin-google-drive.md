# 0082 · Origen Google Drive

**Leer Google Drive** —«Mi unidad», carpetas compartidas y unidades compartidas— como origen de objetos de ORE.
Es el caso de [`0061`](0061-origenes-de-objetos.md) que quedaba en la lista (tras SMB): un driver
`ore-read-gdrive` con el contrato de binario y los verbos de siempre (`catalogo`, `versiones`, `bajar`,
`check`, `explorar`), implementando el rasgo `Origen` de `ore-objetos`, con lo que salen las `ObjectTable` y
las `MediaCollection` virtuales y mantenidas.

**Estado:** **borrador, X0 en curso** (2026-10-31). Rama `origen-google-drive`, worktree `C:\ORE-gdrive`.
§2 es lo publicado por Airbyte y Fivetran; §3 es lo que se mide y §3.1 lo medido. Nada decidido todavía.

Sigue el método de [`0070`](0070-origin-cassandra.md), [`0075`](0075-origen-oracle.md),
[`0077`](0077-origin-clickhouse.md) y [`0080`](0080-origin-cockroachdb.md): medir primero, construir después.
El modelo más cercano es SharePoint (0061 D-O5): versiones por API, huella propia, sin URLs firmadas.

## 1. Qué se decide

Nada aún. Las preguntas que X0 tiene que cerrar, con la inclinación de partida:

1. **Identidad.** Inclinación: la cuenta de la celda, con la carpeta o unidad compartida con ella (D-O2/D-O5);
   sin clave JSON ni OAuth de usuario salvo que un cliente lo exija. Depende de §3 C.
2. **Fijar una lectura.** Inclinación: `revisions` + `md5Checksum` para los ficheros binarios; D-O1 si la
   revisión actual no se puede pedir por id (como en SharePoint). Depende de §3 B.
3. **Docs, Sheets y Slides nativos.** No tienen bytes: sólo `files.export`. Inclinación: objetos exportados
   (docx/pdf, csv por hoja), sólo colección **mantenida**, versión = revisión + formato. Depende de §3 D.
4. **Rutas.** Drive no tiene prefijos: las carpetas son padres por ID. Hay que construir el árbol.
5. **Cambios.** `changes.list` con `startPageToken` frente a listar entero. Depende de §3 E.

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

## 3. Lo que hay que medir (X0)

Contra la API real de Drive v3, como se hizo con GCS y Azure.

- **A. Identidad y cuota:** qué puede hacer una cuenta de servicio; alcance mínimo (`drive.readonly`,
  `drive.file`, `drive.metadata.readonly`); acceso a una sola carpeta.
- **B. Binarios:** `md5Checksum`, `size`, `revisions` (¿la actual se pide por id?), lectura con rango y
  `If-Match`, `headRevisionId`, `keepForever`, límite de revisiones que Drive conserva.
- **C. Árbol:** `parents`, nombres duplicados, ficheros en varias carpetas, accesos directos (`shortcut`),
  papelera, `corpora`/`supportsAllDrives`, «Compartido conmigo».
- **D. Nativos:** `export` (formatos, límite de 10 MB), su determinismo (¿mismos bytes dos veces?), si
  `revisions` da `md5Checksum` (no), `exportLinks`.
- **E. Cambios:** `changes.getStartPageToken` / `changes.list`, borrados, qué ve una cuenta de servicio.
- **F. Ritmo:** cuotas por usuario y por proyecto, `403 rateLimitExceeded` / `429`, `Retry-After`.

### 3.1 Resultados (2026-10-31)

Laboratorio: proyecto `project-8853a180-450d-47be-b83`, API de Drive activada, cuenta de servicio
`ore-gdrive-x0` **sin clave** (se suplanta con `gcloud --impersonate-service-account`, token con alcance
`drive`).

| # | qué | Drive de verdad | lo que supone |
|---|---|---|---|
| A1 | `about` con la cuenta de servicio | `200`; la cuenta tiene su propio Drive vacío, **`storageQuota.limit = 0`** | la identidad funciona; no es dueña de nada |
| A2 | crear una carpeta | `200`, con `parents` = la raíz de su Drive | una cuenta de servicio sí crea carpetas |
| A3 | subir un binario de 12 bytes | **`403 storageQuotaExceeded`**: «Service Accounts do not have storage quota» | **una cuenta de servicio no puede ser dueña de ficheros**: el contenido siempre es de un usuario o de una unidad compartida; ORE sólo lee |
| A4 | crear un Doc y una Hoja vacíos | **`403 storageQuotaExceeded`**, igual | tampoco los nativos: sembrar el laboratorio exige un usuario |
| D1 | `alt=media` sobre un nativo | `400 Unsupported alt type "media" for non byte stream request` | los nativos sólo se leen por `export` |
| D2 | `about.exportFormats` | Docs: rtf, odt, html, pdf, markdown, epub, zip, docx, txt; Hojas: ods, tsv, pdf, xlsx, csv… | hay formato tabular (csv/tsv/xlsx) y de texto (markdown/txt) |
| D3 | `about.maxImportSizes` | doc 10 MiB, hoja 100 MiB, presentación 100 MiB, dibujo 2 MiB | tope de importación; el de **export** se mide en D |

**Pendiente de medir** (necesita ficheros de un usuario compartidos con la cuenta de servicio): B, C, D
(export real, determinismo), E y F.

## 4. El laboratorio

- Cuenta de servicio: `ore-gdrive-x0@project-8853a180-450d-47be-b83.iam.gserviceaccount.com`, sin clave;
  `serviceAccountTokenCreator` sólo para `lunacorunaspain@gmail.com`.
- Carpeta de la cuenta: `x0-raiz` (`15Z2Tpc5x5e2FdVQUhcxiEp_PeWWd_PfD`), vacía.
- Siembra: un usuario con alcance `drive` crea una carpeta de pruebas con binarios, nativos, subcarpetas,
  duplicados, un acceso directo y la comparte con la cuenta de servicio como lector.
- **Al cerrar X0: borrar la cuenta de servicio y desactivar la API si ya no hace falta.**

## 5. Fuentes

- [Airbyte · Google Drive](https://docs.airbyte.com/integrations/sources/google-drive)
- [Fivetran · Google Drive](https://fivetran.com/docs/connectors/files/google-drive)
