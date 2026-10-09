# Orígenes de objetos y de ficheros · estado del arte (2026-10-09)

Investigación para [0061](../decisions/0061-origenes-de-objetos.md): qué almacenes de objetos y de
ficheros se usan como **origen** en las plataformas de datos, y qué superficie hace falta en cada
uno para tener la paridad que S3 ya tiene en ORE (listado, fijar una lectura a su versión, URL
firmada, rangos, lectura condicionada, huella sin bajar, cambios para la ingesta incremental).

⚠️ Lo marcado *(sin verificar)* sale de conocimiento previo y no de la documentación oficial:
se confirma antes de cerrar el diseño de ese origen.

## 1. Qué se usa

No hay un informe público de cuota de mercado del almacén de objetos en sí; la de la nube entera
es AWS ~29–31 %, Azure ~20–24 %, GCP ~11–13 %
([SurferCloud](https://www.surfercloud.com/blog/aws-vs-azure-vs-google-cloud-2025-comparison)). El
indicador útil es lo que soportan las plataformas como fuente:

- **Databricks Unity Catalog** (external locations): S3, ADLS Gen2, GCS y Cloudflare R2; OneLake
  en camino ([docs](https://docs.databricks.com/aws/en/connect/unity-catalog/cloud-storage)).
- **Palantir Foundry**: S3, ABFS (ADLS Gen2 / OneLake), IBM COS, GCS, FTP/FTPS, SFTP, SMB, HDFS,
  SharePoint, Google Drive, OneDrive
  ([source types](https://palantir.com/docs/foundry/data-integration/source-type-overview/)).
- **Snowflake**: S3, GCS, Azure; «compatible con S3» si pasa su suite (R2 viene activado)
  ([docs](https://docs.snowflake.com/en/user-guide/data-load-s3-compatible-storage)).
- **Airbyte**: S3, GCS, Azure Blob, SFTP, SharePoint, Drive
  ([File](https://docs.airbyte.com/integrations/sources/file)).
- Trino, DuckDB, Polaris, Fivetran: S3 con endpoint configurable, GCS y Azure nativos; el resto,
  por la API de S3 *(sin verificar)*.

**Orden resultante:** S3 → Azure Blob / ADLS Gen2 → GCS → SFTP → SharePoint/OneDrive → Cloudflare
R2 → SMB → Google Drive → HDFS → los S3 on-prem (MinIO, Ceph RGW, StorageGRID) → IBM COS, OCI,
Wasabi, Backblaze B2, DigitalOcean Spaces → Alibaba OSS. Box, Dropbox, NFS, Azure Files, EFS/FSx
casi nunca son origen directo (los de ficheros se montan y caen en SMB/NFS).

## 2. Los que hablan la API de S3

La API de S3 es un estándar de hecho: estos proveedores implementan las mismas peticiones y la
misma firma, y el driver de S3 les vale con otro `endpoint`. Lo que cambia es **lo que no
implementan**:

| proveedor | fijar la versión | URL firmada | rango | `If-Match` | huella sin bajar | credencial corta | cambios |
|---|---|---|---|---|---|---|---|
| **Cloudflare R2** | **no hay versionado** | SigV4, sólo en `*.r2.cloudflarestorage.com` | sí | sí | ETag; CRC32/SHA-256 sólo COMPOSITE | no hay STS; tokens por bucket | notificaciones a Queues |
| **MinIO** | `versionId` (con erasure coding) | SigV4 | sí | sí | ETag, checksums de S3 | STS (web identity / LDAP) | notificaciones |
| **Backblaze B2** | siempre versionado, semántica propia | SigV4 (no POST) | sí | sí *(sin verificar)* | ETag (SHA-1 en su API) | sólo application keys | *(sin verificar)* |
| **Wasabi** | `versionId` | SigV4 | sí | sí | ETag | AssumeRole (12 h), sin federación OIDC | limitado |
| **DigitalOcean Spaces** | `versionId` (sólo por API) | SigV2/V4 | sí | sin documentar | ETag | no | sin documentar |
| IBM COS, Oracle OCI, Alibaba OSS | `versionId` *(sin verificar)* | SigV4 / la suya | sí | sí | ETag/MD5/CRC64 | IAM propio | eventos propios |

Fuentes: [R2](https://developers.cloudflare.com/r2/api/s3/api/),
[B2](https://www.backblaze.com/docs/cloud-storage-s3-compatible-api),
[Wasabi](https://docs.wasabi.com/docs/iam-and-sts-support),
[Spaces](https://docs.digitalocean.com/products/spaces/reference/s3-compatibility/).

## 3. Los que tienen protocolo propio

### Google Cloud Storage (JSON API)

- **Credencial**: cuenta de servicio, Workload Identity Federation (token corto), HMAC sólo para
  la XML API.
- **Listado**: `objects.list` con `pageToken`, `prefix`, `versions=true` para las generaciones.
- **Fijar**: `generation` (int64, siempre presente) y `?generation=N`. Sin Object Versioning, la
  generación vieja desaparece al sobrescribir: guarda contra el cambio, no lectura histórica.
- **URL firmada**: V4 (`GOOG4-RSA-SHA256`), con la clave de la cuenta o `iam.signBlob`.
- **Rango** sí; **condicional**: `ifGenerationMatch`.
- **Huella**: `crc32c` siempre; `md5Hash` no en los objetos compuestos.
- **Cambios**: la generación en el listado, notificaciones a Pub/Sub, inventario.
- Su API «compatible con S3» (XML + HMAC) es parcial —no hay `ListObjectVersions`, el pin va por
  `x-goog-generation`—: el driver va por la JSON API.

### Azure Blob / ADLS Gen2

- **Credencial**: clave compartida, SAS, **SAS de delegación de usuario** (firmada con Entra ID, la
  buena), service principal, managed identity, workload identity federation.
- **Listado**: `List Blobs` con `marker`, `include=versions,snapshots`; con namespace jerárquico,
  `List Paths` (DFS).
- **Fijar**: `?versionid=` o `?snapshot=`.
- **URL firmada**: SAS, que puede fijar `versionid`/`snapshot`.
- **Rango**: `x-ms-range`; **condicional**: `If-Match`.
- **Huella**: `Content-MD5` sólo si quien subió lo puso; `x-ms-content-crc64` por rango.
- **Cambios**: Change Feed, Event Grid, inventario.
- ⚠️ **Con namespace jerárquico (ADLS Gen2) no hay versionado ni Change Feed**
  ([Microsoft](https://learn.microsoft.com/en-us/azure/storage/blobs/storage-feature-support-in-storage-accounts)):
  ahí sólo ETag + `If-Match`.

### Ficheros y SaaS

| origen | credencial | fijar | URL firmada | rango | huella | cambios |
|---|---|---|---|---|---|---|
| **SFTP** | contraseña o clave SSH; huella del host obligatoria | **nada**: tamaño + fecha | no | sí | no (salvo extensiones raras) | volver a listar |
| **SharePoint / OneDrive** (Graph) | OAuth2 client credentials (`Sites.Selected`) | `driveItemVersion` | 302 a una URL de minutos | sí | `quickXorHash` | `delta`, webhooks |
| **Google Drive** | OAuth2 / cuenta con delegación | `revisions` (los Docs se exportan) | no | sí | `md5Checksum` en binarios | `changes.list` |
| **SMB / NFS** | NTLM/Kerberos | nada | no | sí | no | volver a listar |
| **HDFS** | Kerberos | nada (snapshots de directorio) | no | sí | `getFileChecksum`, según el bloque | inotify |

Fuentes Graph: [delta](https://learn.microsoft.com/en-us/graph/api/driveitem-delta),
[versiones](https://learn.microsoft.com/en-us/graph/api/driveitem-list-versions).

## 4. Trampas por proveedor

- **R2**: sin versionado (el pin es el ETag), URL firmada sólo en el endpoint de S3, sin STS; el
  ETag de un multipart no es un MD5.
- **MinIO**: la edición community entró en mantenimiento en dic. 2025 y su repositorio se archivó
  en feb. 2026 (fuentes secundarias: [ayedo](https://content.ayedo.de/en/posts/minio-im-maintenance-mode/));
  probar también contra Ceph RGW o Garage. Sin erasure coding, sin versionado.
- **Backblaze B2**: los buckets de antes del 4-may-2020 no hablan S3; borrar deja versiones
  ocultas.
- **Wasabi**: STS de 12 h como mucho, sin federación OIDC.
- **Spaces**: lo no implementado contesta `NotImplemented`: las capacidades se miden al dar de alta.
- **Azure**: con HNS, sin versionado; `Content-MD5` no es fiable; la SAS de cuenta es una
  credencial larga (mejor la de delegación, 7 días como mucho); endpoints `blob.` y `dfs.`.
- **GCS**: XML y JSON se comportan distinto; los compuestos sin MD5; `signBlob` tiene cuota.
- **SharePoint**: la versión actual no se pide por id; throttling por tenant; los ficheros de
  Office pueden cambiar de bytes sin edición visible.
- **SFTP**: fechas al segundo y zonas raras; ficheros a medio escribir (exigir estabilidad entre
  dos listados o un `.done`); sin fijar la huella del host, MITM.
- **HDFS**: el checksum depende del tamaño de bloque; Kerberos.
