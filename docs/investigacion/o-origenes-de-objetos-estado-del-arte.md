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

## 5. Cómo se identifica la plataforma ante el almacén del cliente (2026-10-09)

Investigado para D-O2. **El patrón dominante** (Snowflake, Databricks, las conexiones de BigQuery,
Fivetran; Foundry por OIDC): la identidad es **de la plataforma** y se le enseña al cliente; el
cliente le concede lo mínimo con **su** IAM; sin claves largas (quedan de recurso: Airbyte, Foundry);
el grano es **por tenant** (Snowflake: una cuenta de servicio, un usuario IAM, una app de Entra por
cuenta) **o por conexión** (BigQuery, Databricks, Foundry con `sub` = el origen); y cuando el
principal se comparte entre clientes, un **ExternalId** que genera el proveedor (Databricks, Fivetran)
contra el *confused deputy* (que el cliente A registre el rol de B). Y un paso de **validación** con
resultado por acción (`SYSTEM$VALIDATE_STORAGE_INTEGRATION`, el «Validate Configuration» de
Databricks).

| plataforma | GCS | S3 | Azure |
|---|---|---|---|
| Snowflake | una cuenta de servicio por cuenta de Snowflake, que el cliente autoriza (`DESC STORAGE INTEGRATION`) | un usuario IAM por cuenta + `ExternalId` | una app multi-tenant por cuenta, consentida por el cliente |
| Databricks | una cuenta generada por credencial | rol del cliente que confía en su rol maestro + `ExternalId` por credencial | Access Connector (managed identity) en la suscripción del cliente |
| BigQuery (conexión) | una cuenta de sistema por conexión | — | — |
| Fivetran | la suya, en el formulario | cuenta compartida + `ExternalId` por cuenta | — |
| Foundry | clave JSON, o Workload Identity Federation (OIDC, `sub` = el origen) | clave, rol con `ExternalId`, u OIDC | OIDC |

Fuentes: [Snowflake GCS](https://docs.snowflake.com/en/user-guide/data-load-gcs-config),
[Snowflake S3](https://docs.snowflake.com/en/user-guide/data-load-s3-config-storage-integration),
[Databricks GCP](https://docs.databricks.com/gcp/en/connect/unity-catalog/cloud-storage/storage-credentials),
[BigQuery](https://docs.cloud.google.com/bigquery/docs/create-cloud-resource-connection),
[Fivetran GCS](https://fivetran.com/docs/connectors/files/google-cloud-storage/setup-guide),
[Foundry OIDC](https://www.palantir.com/docs/foundry/data-connection/oidc),
[AWS: confused deputy](https://docs.aws.amazon.com/IAM/latest/UserGuide/confused-deputy.html),
[AWS: claves de Google](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_iam-condition-keys.html).

**El S3 de ORE ya sigue el estándar** (cotejado en el código): cada celda tiene sus cuentas de
Google, y la confianza que se le da al cliente condiciona sobre su **ID único** (`sub`, no el
email, que AWS desaconseja) y la audiencia (`oaud`). `AssumeRoleWithWebIdentity` no admite
`ExternalId`: lo que aísla es la identidad por celda, como el usuario IAM por cuenta de Snowflake.

**Trampas**: la política *Domain Restricted Sharing* (`iam.allowedPolicyMemberDomains`) de una
organización de Google impide conceder nada a cuentas de fuera de su dominio —la salida del sector
es una federación de identidad en el proyecto del cliente que confíe en la plataforma, como Foundry—;
SFTP: fijar la huella del host en la primera prueba, con confirmación (Fivetran la acepta así);
SharePoint: `Sites.Selected` con consentimiento por sitio, nunca `Sites.Read.All`.

## 6. Azure: cómo entra la plataforma en el Blob Storage del cliente (2026-10-09)

Investigado para D-O3.

| producto | cómo | fuente |
|---|---|---|
| **BigQuery Omni** | app registration de un tenant del cliente con una *federated identity credential* «Other issuer»: issuer `https://accounts.google.com`, subject = la identidad de Google de la conexión, audiencia `api://AzureADTokenExchange`; `Storage Blob Data Reader`. Sin secretos | [docs](https://docs.cloud.google.com/bigquery/docs/omni-azure-create-connection) |
| **Storage Transfer Service** | SAS, Shared Key, o `federatedIdentityConfig{clientId, tenantId}` con subject = el ID único de la cuenta de Google; recomienda un rol a medida con sólo `blobs/read` sobre el contenedor | [docs](https://docs.cloud.google.com/storage-transfer/docs/source-microsoft-azure) |
| **Snowflake** | app multi-tenant suya; el cliente abre `AZURE_CONSENT_URL` y da `Storage Blob Data Reader` al service principal | [docs](https://docs.snowflake.com/en/user-guide/data-load-azure-config) |
| **Databricks UC** | Access Connector (managed identity) «strongly recommended»; el service principal con secreto, «legacy» | [docs](https://learn.microsoft.com/en-us/azure/databricks/connect/unity-catalog/cloud-storage/storage-credentials) |
| **Foundry (ABFS)** | client credentials, SAS (desaconsejado), Shared Key (no en producción), WIF/OIDC (`tenantId` + `clientId`) | [docs](https://www.palantir.com/docs/foundry/available-connectors/onelake-and-azure-blob-filesystem/) |
| **Fivetran / Airbyte** | service principal con secreto, SAS, account key | [Fivetran](https://fivetran.com/docs/connectors/files/azure-blob-storage/setup-guide), [Airbyte](https://docs.airbyte.com/integrations/sources/azure-blob-storage) |

**Los dos de Google (Omni, STS) hacen exactamente lo que ORE ya hace con AWS**: la cuenta de Google
de la plataforma, federada en una app del cliente por su ID único. Microsoft desaconseja Shared Key
(la política integrada *Storage accounts should prevent shared key access*); con
`AllowSharedKeyAccess=false` mueren la SAS de cuenta y la de servicio, y sólo vale la **SAS de
delegación de usuario** ([docs](https://learn.microsoft.com/en-us/azure/storage/common/shared-key-authorization-prevent)).

- **Federación (WIF) con Google** ([tutorial](https://learn.microsoft.com/en-us/entra/workload-id/workload-identity-federation-google-cloud),
  [límites](https://learn.microsoft.com/en-us/entra/workload-id/workload-identity-federation-considerations)):
  en una app registration **o** en una managed identity asignada por el usuario; subject = el ID
  único numérico de la cuenta (el `sub` del ID token); 20 credenciales por app; coincidencia exacta;
  minutos de propagación (`AADSTS70021`); un subject mal escrito se crea sin error y falla después.
  El canje es el *client credentials* con aserción federada
  ([doc](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow#third-case-access-token-request-with-a-federated-credential)).
  El ID token sale del metadata server de GKE (`/identity?audience=…`), como ya lo pide `ore-gcp`
  para AWS (medido desde pods el 2026-09-30): sólo cambia la audiencia.
- **Sin `ExternalId`**: lo que aísla es la identidad por celda, como en S3 y GCS.
- **RBAC** ([roles](https://learn.microsoft.com/en-us/azure/role-based-access-control/built-in-roles/storage)):
  `Storage Blob Data Reader` lista y lee, versiones incluidas, y trae `generateUserDelegationKey`;
  pero la clave de delegación es **de la cuenta**: con el lector sólo sobre el contenedor, hace falta
  además `Storage Blob Delegator` sobre la cuenta ([doc](https://learn.microsoft.com/en-us/rest/api/storageservices/create-user-delegation-sas)).
- **SAS de delegación**: la clave vale hasta 7 días y firma cuantas URLs se quiera; `sr=bv` fija una
  versión; `rsct`/`rscd` el tipo y la disposición.
- **ADLS Gen2** (namespace jerárquico): **sin versionado de blobs**
  ([versioning](https://learn.microsoft.com/en-us/azure/storage/blobs/versioning-overview)) — se fija
  por ETag (D-O1) —; las ACL POSIX se suman a RBAC; el endpoint Blob funciona sobre esas cuentas.
- **Huella**: `Content-MD5`, que Put Blob calcula siempre y Put Block List sólo guarda si el cliente
  lo da (las subidas grandes suelen quedar sin él); ningún CRC64 de objeto entero
  ([Put Block List](https://learn.microsoft.com/en-us/rest/api/storageservices/put-block-list)).

## 7. SFTP: cómo entra la plataforma en el servidor del cliente (2026-10-09)

Investigado para D-O4.

| producto | identidad | huella del host | fuente |
|---|---|---|---|
| **Fivetran** | contraseña, **un par de claves que genera Fivetran** (el cliente pega la pública en `authorized_keys`), o una privada que sube el cliente | TOFU: el usuario confirma la clave en la prueba | [docs](https://fivetran.com/docs/connectors/files/sftp/setup-guide) |
| **Databricks** (SFTP, preview) | privada PEM (recomendada) o contraseña, en una conexión de Unity Catalog | «Enforce host key fingerprint» SHA-256; la huella sale del error de una prueba | [docs](https://docs.databricks.com/aws/en/ingestion/sftp) |
| **Foundry** | contraseña o privada que sube el cliente | **obligatoria**; «Accept any host key» existe, apagado y llamado inseguro | [docs](https://www.palantir.com/docs/foundry/available-connectors/sftp/) |
| **Azure Data Factory** | contraseña, clave del cliente, o ambas | `hostKeyFingerprint` obligatoria salvo que se salte | [docs](https://learn.microsoft.com/en-us/azure/data-factory/connector-sftp) |
| **AWS Transfer Family** (conectores) | privada, contraseña o ambas, en Secrets Manager | `TrustedHostKeys`; vacío, la prueba devuelve la vista y se fija | [docs](https://docs.aws.amazon.com/transfer/latest/userguide/configure-sftp-connector.html) |
| **Airbyte** | contraseña o privada | **no verifica** (`hostkey=None` en su código) | [código](https://raw.githubusercontent.com/airbytehq/airbyte/master/airbyte-integrations/connectors/source-sftp-bulk/source_sftp_bulk/client.py) |

- **Red**: IPs de salida fijas publicadas (Fivetran por región, AWS 3 por conector, Databricks por NAT);
  túneles SSH o agentes, aparte; puertos distintos del 22.
- **Trampas** ([OpenSSH 8.8](https://www.openssh.com/txt/release-8.8)): servidores viejos que sólo
  tienen `ssh-rsa` con SHA-1; el `mtime` de SFTP v3 son segundos enteros
  ([libssh](https://api.libssh.org/master/structsftp__attributes__struct.html)), así que no basta
  como versión; `MaxSessions` 10 por conexión en OpenSSH; los ficheros a medio escribir se evitan
  con una **edad mínima** (NiFi «Minimum File Age»), renombrado al terminar, o un fichero marcador.
- **Bibliotecas de Rust**: `russh` + `russh-sftp` (Rust puro, tokio, muy activa, corrigió Terrapin en
  0.40.2; la investigación cita además una serie de avisos de seguridad recientes, sobre todo de
  denegación de servicio, **sin verificar uno a uno**) y `ssh2` (libssh2 en C, síncrona, de ritmo
  lento; la que empaqueta `libssh2-sys` 0.3.3 es la 1.11.1, con *strict KEX*: Terrapin corregido,
  comprobado en su código). `openssh` llama al binario `ssh` y obliga a llevarlo en la imagen.

## 8. SharePoint / OneDrive: cómo entra la plataforma en el tenant del cliente (2026-10-09)

Investigado para D-O5. Sin emulador ni tenant: todo lo de aquí es de la documentación; lo que no
está escrito en una oficial va marcado **sin verificar** y se mide cuando haya tenant.

| producto | identidad | permiso | por sitio | fuente |
|---|---|---|---|---|
| **Foundry** | app-only con secreto, o un usuario delegado | `Sites.Read.All` o `Sites.Selected` (`read` basta) | sí | [docs](https://palantir.com/docs/foundry/available-connectors/sharepoint-online/) |
| **Databricks Lakeflow** | U2M, o M2M con secreto | `Sites.Selected` recomendado, `read` | sí | [docs](https://docs.databricks.com/aws/en/ingestion/lakeflow-connect/sharepoint) |
| **Fivetran** | delegado con su app, o app del cliente con secreto o certificado | `Sites.Selected` `read`, por sitio o por carpeta | sí | [docs](https://fivetran.com/docs/connectors/files/share-point/setup-guide) |
| **Snowflake Openflow** | secreto, o certificado | `Sites.Selected` (+ `Files.SelectedOperations.Selected`) | sí | [docs](https://docs.snowflake.com/en/user-guide/data-integration/openflow/connectors/sharepoint/setup) |
| **Google (Gemini Enterprise)** | **credencial federada, issuer `https://accounts.google.com`**, o secreto | `Sites.Selected` + `fullcontrol` (lee ACL) | sí | [docs](https://docs.cloud.google.com/gemini/enterprise/docs/connectors/ms-sharepoint/third-party-config) |
| **AWS (Q Business)** | app-only con certificado (recomendado) | `Sites.Selected` `fullcontrol` | sí | [docs](https://docs.aws.amazon.com/amazonq/latest/qbusiness-ug/sharepoint-cloud-prereqs.html) |
| **Airbyte** | delegado, o secreto | `Files.Read.All` | no | [docs](https://docs.airbyte.com/integrations/sources/microsoft-sharepoint) |
| **ADF / Fabric** | certificado (lista) o secreto por HTTP | `Sites.Read.All` | no | [docs](https://learn.microsoft.com/en-us/azure/data-factory/connector-sharepoint-online-list) |

**El estándar es una app del tenant del cliente, app-only, con `Sites.Selected` concedido sitio a
sitio**; quien sólo ingiere pide `read` (`fullcontrol` es para leer ACL). Google es el único que
documenta la federación con sus cuentas, que es lo que ORE ya hace con Azure (§6).

- **`Sites.Selected`** ([overview](https://learn.microsoft.com/en-us/graph/permissions-selected-overview)):
  hacen falta las tres cosas —el consentimiento de la app en Entra, la concesión en el sitio
  (`POST /sites/{id}/permissions` con `roles: ["read"]` y la app en `grantedToIdentities`) y el
  ámbito en el token—. Concede quien tenga `Sites.FullControl.All` (en delegado, además rol de
  SharePoint Administrator) o con PnP (`Grant-PnPEntraIDAppSitePermission -Permissions Read`). Hay
  variantes por lista, por carpeta o por fichero (`*.SelectedOperations.Selected`, rompen la
  herencia; su estado GA, **sin verificar**). Las páginas de `children`, `content`, `versions` y
  `delta` sólo nombran `Files.Read.All`/`Sites.Read.All`: que funcionen con `Sites.Selected` lo
  dicen los conectores que ingieren con él, no una página de Graph. **Que `delta` funcione con
  `Sites.Selected`: sin verificar.**
- **Federación**: da tokens app-only para cualquier recurso de Entra, Graph incluido
  ([doc](https://learn.microsoft.com/en-us/entra/workload-id/workload-identity-federation)).
  El `/_api` (REST/CSOM) de SharePoint app-only **sólo acepta certificado**
  ([doc](https://learn.microsoft.com/en-us/sharepoint/dev/solution-guidance/security-apponly-azuread)):
  con federación, sólo Graph.
- **Direcciones**: el sitio por ruta (`/sites/{host}:/{ruta}`, id `host,guid,guid`), sus
  bibliotecas (`/sites/{id}/drives`), un elemento por ruta (`/drives/{d}/root:/{ruta}:`),
  `children` en páginas de 200 con `@odata.nextLink`. Un `driveItem` lleva `id`, `eTag` (cambia con
  el contenido y los metadatos), `cTag` (sólo con el contenido; en carpetas, con el de cualquier
  descendiente), `size`, `lastModifiedDateTime`, `file.hashes`, y lo que no es un fichero normal:
  `folder`, `package` (cuadernos de OneNote), `remoteItem` (un acceso directo a otro drive),
  `deleted`.
- **Huella**: sólo `quickXorHash` está garantizado en SharePoint y OneDrive
  ([hashes](https://learn.microsoft.com/en-us/graph/api/resources/hashes?view=graph-rest-1.0)):
  `sha256Hash` «no se usa», `sha1Hash`/`crc32Hash` «si hay». Es un XOR desplazado sobre 160 bits
  con la longitud al final, en base64 ([algoritmo](https://learn.microsoft.com/en-us/onedrive/developer/code-snippets/quickxorhash)).
  **Las versiones antiguas no traen huella.**
- **Leer**: `/content` contesta `302` a una URL pre-autenticada (la misma que
  `@microsoft.graph.downloadUrl`; «puede caducar en minutos», el recurso dice una hora) que es un
  secreto al portador; el `Range` va en esa URL (`206`, o `200` entero si no puede)
  ([doc](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0)).
  `if-none-match` con el eTag/cTag da `304`; **`If-Match`, sin documentar**: no hay forma escrita
  de fijar una descarga a un eTag.
- **Versiones** ([doc](https://learn.microsoft.com/en-us/graph/api/driveitem-list-versions?view=graph-rest-1.0)):
  ids `"3.0"`, `"2.0"`…, de la más nueva a la más vieja; una antigua se baja por
  `/versions/{id}/content` (302, con `Range`), **la actual no**: sólo por `/content`
  ([doc](https://learn.microsoft.com/en-us/graph/api/driveitemversion-get-contents?view=graph-rest-1.0)).
  Las bibliotecas versionan con un límite (por número o por edad, o «automático»: las viejas se
  recortan) ([doc](https://learn.microsoft.com/en-us/sharepoint/document-library-version-history-limits)):
  **una versión fijada puede desaparecer**, como en S3 con un ciclo de vida.
- **`delta`** ([doc](https://learn.microsoft.com/en-us/graph/api/driveitem-delta?view=graph-rest-1.0)):
  el listado completo garantizado y luego sólo los cambios (`deltaLink`, `token=latest`, los
  borrados con `deleted`, `410` para empezar de cero); sin rutas (`parentReference.path` vacío) y,
  en SharePoint, sin `cTag` en los cambios.
- **Ritmo** ([doc](https://learn.microsoft.com/en-us/sharepoint/dev/general-development/how-to-avoid-getting-throttled-or-blocked-in-sharepoint-online)):
  `429`/`503` con `Retry-After`, que hay que respetar; un presupuesto en *resource units* por app y
  por tenant (1.250 RU/min en un tenant de hasta 1.000 licencias; listar 2 RU, leer 1); el
  `User-Agent` `ISV|<empresa>|<app>/<versión>`.
- **Errores** ([doc](https://learn.microsoft.com/en-us/graph/errors)): `{"error":{"code",
  "message"}}`, programar contra `code`; `403`, `404`, `410`, `412`, `416`, `429`, `503`.
- **Trampas**: un fichero de Office puede cambiar de bytes sin una edición visible (SharePoint
  escribe sus propiedades dentro: documentado en 2010, en Online **sin verificar**); los accesos
  directos (`remoteItem`) apuntan fuera de la biblioteca; nubes soberanas (GCC High, 21Vianet) con
  otros hosts.
- **El cliente HTTP**: `ureq` 2.12.1 quita `Authorization` al seguir una redirección a otro host
  (`redirect_auth_headers` por defecto `Never`, leído en su código): el token de Graph no llega al
  host de la descarga. Aun así, el driver no sigue la redirección a ciegas (D-O5).
