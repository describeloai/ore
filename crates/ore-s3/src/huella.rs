//! **La huella de contenido de S3**: el CRC64NVME (`FULL_OBJECT`) que S3
//! calcula de cada objeto desde 2025 y devuelve con `x-amz-checksum-mode`. Vive
//! en `ore-objetos` (ADR 0061 O0·1), con las de los demás orígenes; aquí se
//! sigue llamando como antes.

pub use ore_objetos::huella::{Crc64Nvme, de};
