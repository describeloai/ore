# Orígenes de objetos · cómo se conecta un bucket

**Estado:** S3 en vivo (0046); los que hablan la API de S3, probados en el laboratorio (ADR
[0061](decisions/0061-origenes-de-objetos.md) O1, 2026-10-09); GCS, con su driver propio y probado
en el laboratorio (O2, 2026-10-09), sin probar todavía contra GCS de verdad. Azure, SFTP y
SharePoint, por construir (O3–O5).

Un bucket es una **fuente** (`ontology.config.yaml`, `type: s3` o `type: gcs`): de él salen `Table` con `format`
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

## Por qué no un driver por proveedor

La API de S3 es la forma principal de hablar con todos estos: un driver propio repetiría las mismas
peticiones con otra firma que mantener. Un driver nativo se hará cuando un cliente choque con algo
que sólo da su API (los buckets B2 de antes de 2020, el versionado completo de OCI, el IAM de IBM
sin claves HMAC). GCS, Azure, SFTP y SharePoint sí llevan driver propio: no hablan S3, o su capa S3
es peor que su API (ADR 0061).
