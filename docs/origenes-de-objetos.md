# Orígenes de objetos · cómo se conecta un bucket

**Estado:** S3 en vivo (0046); los que hablan la API de S3, probados en el laboratorio (ADR
[0061](decisions/0061-origenes-de-objetos.md) O1, 2026-10-09); GCS, con su driver propio y probado
en el laboratorio (O2, 2026-10-09), sin probar todavía contra GCS de verdad; Azure Blob y ADLS Gen2,
igual (O3, 2026-10-09), sin probar contra Azure de verdad; SFTP, en el laboratorio (O4, 2026-10-09), sin
probar contra un servidor de un cliente. SharePoint, por construir (O5).

Un bucket es una **fuente** (`ontology.config.yaml`, `type: s3`, `gcs`, `azure` o `sftp`): de él salen `Table` con `format`
sobre sus Parquet, CSV y JSONL, y `ObjectTable` sobre lo demás, de donde salen las colecciones de
media ([`media.md`](media.md)). La credencial no se escribe en el árbol: vive en el custodio y el
árbol sólo nombra su variable (`connectionEnv`).

## La URL

```
s3://<bucket>[/<prefijo>]?region=<región>&access_key_id=<clave>&secret_access_key=<secreto>[&session_token=…][&endpoint=<https://…>]
s3://<bucket>[/<prefijo>]?region=<región>&role_arn=<arn:aws:iam::…:role/…>          # sólo AWS: un rol, canjeado por una credencial de una hora
```

- **Sin `endpoint`**, es AWS (el host virtual de la región).
- **Con `endpoint`**, es cualquiera que hable la API de S3: el bucket va en la ruta (*path-style*)
  y la región es la que ese proveedor pida.
- Un origen **que no versiona** se fija por el ETag de cada objeto: lo que cambió entre listar y
  leer es un `412 media/cambiado`, nunca otros bytes, y una colección **virtual** suya no da URLs
  firmadas (una URL no lleva `If-Match`): sus ítems se abren por `content`. Una **mantenida** copia
  los bytes al lago y desde ahí sí. `check` dice cómo fija cada fuente (`"fija": "version" | "etag"`).

## Los perfiles

| proveedor | `endpoint` | `region` | fija por | probado |
|---|---|---|---|---|
| **AWS S3** | — | la del bucket | versión (`versionId`; `null` en un bucket sin versionar) | en vivo (0046) |
| **VersityGW** | `http(s)://<host>:7070` | `us-east-1` | versión | laboratorio (O1) |
| **Garage** | `http(s)://<host>:3900` | la de su `s3_region` | **ETag** (ignora `versionId`) | laboratorio (O1) |
| **Cloudflare R2** | `https://<cuenta>.r2.cloudflarestorage.com` | `auto` | **ETag** (no hay versionado) | de su documentación |
| **MinIO** | `http(s)://<host>:9000` | `us-east-1` | versión con erasure coding; si no, ETag | de su documentación¹ |
| **Ceph RGW** | `http(s)://<host>` | la de su *zonegroup* | versión | de su documentación |
| **Wasabi** | `https://s3.<región>.wasabisys.com` | la del bucket | versión | de su documentación |
| **Backblaze B2** | `https://s3.<región>.backblazeb2.com` | la del bucket | versión | de su documentación² |
| **DigitalOcean Spaces** | `https://<región>.digitaloceanspaces.com` | la del bucket | versión (activada por API) | de su documentación |
| **IBM COS** | `https://s3.<región>.cloud-object-storage.appdomain.cloud` | la del bucket | versión | de su documentación |
| **Oracle OCI** | `https://<namespace>.compat.objectstorage.<región>.oraclecloud.com` | la del bucket | versión (parcial en su capa S3) | de su documentación |

¹ MinIO dejó de publicar imágenes y su edición community está en mantenimiento (medido el
2026-10-09): el laboratorio usa VersityGW y Garage. ² Los buckets B2 de antes del 4-may-2020 no
hablan S3.

Lo que ninguno de los compatibles tiene es el **rol** (`role_arn`): se conectan con una clave fija
(`access_key_id` + `secret_access_key`), que vive en el custodio como cualquier otra.

**«De su documentación»** quiere decir que el driver habla con ellos por la misma API que con los
probados, pero nadie lo ha medido contra una cuenta suya: si algo no cuadra, `check` lo dice al
dar de alta la fuente, con lo que el proveedor contestó. Cada uno pasa a «probado» cuando una cuenta
real pase `pruebas-de-fuego/o1-los-que-hablan-s3.sh` con su URL.

## GCS

```
gs://<bucket>[/<prefijo>]                                     # con las cuentas de esta celda
gs://<bucket>[/<prefijo>]?suplantar=<cuenta>@<proyecto>.iam.gserviceaccount.com   # como una cuenta del cliente
```

La URL **no lleva secreto** (D-O2): no hay clave que guardar, y las claves JSON de cuenta de servicio
no se admiten (Google las prohíbe por defecto en las organizaciones nuevas).

- **Con las cuentas de la celda** (recomendado): el cliente concede `roles/storage.objectViewer`
  sobre su bucket a las dos cuentas que leen, la del driver (`ore-driver-<celda>@…`, que cataloga y
  copia) y la de medios (`ore-medios-<celda>@…`, que sirve los ítems y firma sus URLs). La consola
  las enseña con el comando (`GET /fuentes/credenciales/gcs`). Ninguna cuenta se comparte entre
  celdas.
- **Suplantando una cuenta del cliente**: el cliente da lectura a una cuenta suya y deja que las
  dos de la celda pidan su token (`roles/iam.serviceAccountTokenCreator` sobre ella). Cada lectura
  usa un token de una hora y sólo de lectura; quitando ese permiso, deja de leer.
- Si la organización del cliente sólo deja conceder a sus propios dominios (*Domain Restricted
  Sharing*), ninguno de los dos modos cabe: haría falta federación de identidad, que no está.

Cada objeto se fija por su **generación**, que GCS da siempre: leer `?generation=N` devuelve esa o
un `404`, que es `media/cambiado`. La huella es su `crc32c`, que GCS da sin bajar el objeto. Con
*Object Versioning* activado, las generaciones de antes se siguen leyendo (una colección mantenida
copia la que listó); sin él, la generación reescrita desaparece y el ítem no se copia —nunca otros
bytes—. `check` le pregunta a GCS qué permisos tiene la identidad (`testIamPermissions`) y lo
confirma listando y mirando un objeto; con `suplantar`, dice antes si la suplantación funciona.

Las URLs firmadas son V4, firmadas por IAM (`signBlob`) como la cuenta que lee, y fijadas a la
generación; se dan por `/media` (`ore-medios`). La ruta vieja de los datasets (`ore collections
--servir`) no sirve GCS, porque firmar es una petición a IAM y la imagen de `ore-serve` no sale a la
red.

`endpoint` sólo admite `https://…googleapis.com` (`private.`, `restricted.`, los regionales): el
token se manda en cada petición y quien lo recibe lee como la celda. Otro servidor sólo con
`ORE_GCS_LABORATORIO=1`, que pone el laboratorio (`pruebas-de-fuego/o2-gcs.sh`, contra
`fake-gcs-server`) y nunca una celda.

## Azure Blob y ADLS Gen2

```
az://<cuenta>/<contenedor>[/<prefijo>]?tenant=<id del tenant>&cliente=<id de la app>
```

La URL **no lleva secreto** (D-O3): nombra la app de Entra del cliente. Como BigQuery Omni y Storage
Transfer Service, el cliente crea en su tenant una app (o una managed identity) con dos *federated
identity credentials*, una por cuenta de Google de la celda (issuer `https://accounts.google.com`,
subject = su ID único, audiencia `api://AzureADTokenExchange`), y le da:

- `Storage Blob Data Reader` sobre el contenedor: listar y leer, versiones incluidas;
- `Storage Blob Delegator` sobre **la cuenta**: la clave de delegación con la que se firman las URLs
  de los ítems es de la cuenta. Sin él se cataloga y se copia igual, y los ítems se abren por
  `content`.

La consola da los comandos `az` con los IDs ya puestos (`GET /fuentes/credenciales/azure`): un ID mal
pegado se crea sin error y falla después, en silencio. Quien lee cambia su token de Google por uno de
Entra él mismo; si la credencial federada no casa —o aún se propaga, que tarda minutos—, `check` lo
dice. No se admiten la clave de la cuenta (lo abre todo, y Azure recomienda apagarla), una SAS del
cliente (un secreto al portador que caduca) ni el secreto de una app.

Cada blob se fija por su **`versionId`** si la cuenta tiene versionado, y **siempre además por su
ETag** (`If-Match`): un servidor que ignore la versión —Azurite lo hace— no puede dar otros bytes. Sin
versionado, y en **ADLS Gen2**, que no lo tiene, la versión es el ETag (D-O1): lo que cambió entre
listar y leer es un `412`, y esos ítems no dan URL firmada. Las URLs son SAS de delegación de usuario,
fijadas a la versión. La huella es el `Content-MD5`, que Azure guarda de un blob subido de una vez y
no de uno subido por bloques (los grandes, casi siempre): sin él, se coteja el tamaño. Blob no
entiende un rango por el final (`bytes=-N`): el driver lo resuelve con el tamaño del blob. `check`
prueba cada paso —Azure no tiene a quién preguntar los permisos— y dice como lo que son un contenedor
que no existe y el firewall de red de la cuenta.

`endpoint` no se admite: sólo se habla con `https://<cuenta>.blob.core.windows.net` (el token de
Storage vale para cualquier cuenta que la app pueda leer). Otro servidor sólo con
`ORE_AZURE_LABORATORIO=1`, que pone el laboratorio (`pruebas-de-fuego/o3-azure.sh`, contra Azurite) y
nunca una celda. Las nubes soberanas (Azure Government, China) no están.

## SFTP

```
sftp://<usuario>@<host>[:<puerto>]/<ruta>?huella=SHA256:<huella del host>[&edad=<segundos>][&legado=1]
```

Un servidor SFTP no versiona nada, así que **sus colecciones sólo pueden ser mantenidas** (D-O1): la
versión es la copia en el lago, con su sha256. `discover` las escribe mantenidas también en una base
foránea, y `ore` niega una virtual con su porqué.

- **Identidad** (D-O4): una clave SSH Ed25519 **de la celda**, que genera ORE; el cliente sólo pega
  su parte pública en el `authorized_keys` del usuario con el que se lee (mejor uno que sólo lea esa
  ruta). La consola da el comando con la clave puesta (`GET /fuentes/credenciales/sftp`). De recurso,
  una contraseña en la URL (`usuario:contraseña@`), que va al custodio.
- **La huella del host, siempre fijada**: se compara antes de autenticar, así que a un servidor que
  se hace pasar por el del cliente no le llega ni la clave. La prueba del alta enseña la huella vista
  para confirmarla (en el servidor: `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`); si un día
  cambia, la celda se niega a conectar hasta que se vuelva a confirmar. No hay «aceptar cualquiera».
- **Cada fichero se vigila** por su tamaño y su fecha de modificación (en segundos: SFTP no da más),
  al abrirlo y otra vez al terminar de leerlo: uno reescrito mientras se lee no se copia (daría una
  mezcla de los dos). Un fichero no se copia hasta que lleva `edad` segundos sin cambiar (60 por
  defecto): puede estar a medio escribir.
- **Los enlaces simbólicos no se siguen**, y un fichero sin permiso de lectura es un aviso (se
  cataloga por su nombre y no se copia), no un fallo.
- **Red**: la celda sale por el puerto SSH del servidor desde su **IP de salida fija**, que el
  cliente pone en su lista de permitidos (la consola la enseña). Algoritmos modernos; un servidor
  viejo que sólo hable `ssh-rsa` con SHA-1 necesita `legado=1`.

`check` va paso a paso —conexión, huella, identidad, listar, leer— y de lo que falla dice qué hacer.
Probado en el laboratorio con `pruebas-de-fuego/o4-sftp.sh` (OpenSSH, `atmoz/sftp`).

## Por qué no un driver por proveedor

La API de S3 es la forma principal de hablar con todos estos: un driver propio repetiría las mismas
peticiones con otra firma que mantener. Un driver nativo se hará cuando un cliente choque con algo
que sólo da su API (los buckets B2 de antes de 2020, el versionado completo de OCI, el IAM de IBM
sin claves HMAC). GCS, Azure, SFTP y SharePoint llevan driver propio: no hablan S3, o su capa S3
es peor que su API (ADR 0061).
