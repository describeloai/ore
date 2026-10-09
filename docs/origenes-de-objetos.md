# Orígenes de objetos · cómo se conecta un bucket

**Estado:** S3 en vivo (0046); los que hablan la API de S3, probados en el laboratorio (ADR
[0061](decisions/0061-origenes-de-objetos.md) O1, 2026-10-09). GCS, Azure, SFTP y SharePoint, por
construir (O2–O5).

Un bucket es una **fuente** (`ontology.config.yaml`, `type: s3`): de él salen `Table` con `format`
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

## Por qué no un driver por proveedor

La API de S3 es la forma principal de hablar con todos estos: un driver propio repetiría las mismas
peticiones con otra firma que mantener. Un driver nativo se hará cuando un cliente choque con algo
que sólo da su API (los buckets B2 de antes de 2020, el versionado completo de OCI, el IAM de IBM
sin claves HMAC). GCS, Azure, SFTP y SharePoint sí llevan driver propio: no hablan S3, o su capa S3
es peor que su API (ADR 0061).
