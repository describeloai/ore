//! `ore-store` — **el almacén delegado**, fuera del compilador, en dos binarios
//! que comparten todo menos el transporte:
//!
//! | binario | dónde guarda | con qué credencial |
//! |---|---|---|
//! | `ore-store-r2` | cualquier S3 (R2 de Cloudflare, medido en el ADR 0015) | clave estática, SigV4 |
//! | `ore-store-gcs` | Google Cloud Storage por su API JSON | el token de la cuenta que corre (Workload Identity), sin clave |
//!
//! Normativo: [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md),
//! revisado por [0031 §10](../../../docs/decisions/0031-el-puesto.md) (W3.6a,
//! 2026-09-20): **la copia es un dataset**, una tabla Iceberg en el bucket que
//! este programa escribe (`lago.rs`) y cuyo puntero vive en el árbol. El
//! protocolo —la petición por stdin, las filas una por línea, una línea de
//! vuelta— es el mismo en los dos binarios, y el almacén de cada uno es también
//! el suelo que Iceberg pisa: un transporte, no dos.
//!
//! # Por qué dos, y por qué el segundo (2026-09-17)
//!
//! La celda vive en GCP y **no puede tener una clave estática**: la política de
//! la organización lo prohíbe (`iam.disableServiceAccountKeyCreation`, y las
//! claves HMAC de la API S3 de GCS cuentan como tales). Y mandar la copia a R2
//! la sacaría de la VPC, que es lo contrario de lo que la copia es (0027, 0028).
//! Lo que un Job de la celda sí tiene es un token del metadata server con la
//! cuenta `driver` — el mismo con el que ya lee Secret Manager. `ore-store-gcs`
//! habla con ese token y con nada más.
pub mod almacen;
pub mod carga;
pub mod ciclo;
pub mod gcs;
pub mod lago;
pub mod r2;
pub mod sobre;
