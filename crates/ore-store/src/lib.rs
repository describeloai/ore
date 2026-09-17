//! `ore-store` — **el almacén delegado**, fuera del compilador, en dos binarios
//! que comparten todo menos el transporte:
//!
//! | binario | dónde guarda | con qué credencial |
//! |---|---|---|
//! | `ore-store-r2` | cualquier S3 (R2 de Cloudflare, medido en el ADR 0015) | clave estática, SigV4 |
//! | `ore-store-gcs` | Google Cloud Storage por su API JSON | el token de la cuenta que corre (Workload Identity), sin clave |
//!
//! Normativo: [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md).
//! El protocolo —la cabecera por stdin, las filas una por línea, una línea de
//! vuelta— y el nombre del artefacto (su digest) son los mismos en los dos: una
//! copia sellada por uno la reconoce el otro.
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
pub mod r2;
pub mod sobre;
