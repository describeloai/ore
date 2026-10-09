//! **Lo que el catálogo le pide a un bucket**, detrás de un rasgo: listar y
//! leer un rango. El rasgo y el almacén en memoria de las pruebas viven en
//! `ore-objetos`, y el bucket lo implementa en `ore-s3` (ADR 0061 O0·1): así
//! lo mismo vale para cualquier origen de objetos.

pub use ore_objetos::Origen;
#[cfg(test)]
pub use ore_objetos::memoria::EnMemoria;
