//! **Oracle Database como origen** (ficha 0074, capa 1): lo que `ore-read-oracle` necesita.
//!
//! ```text
//!   Peticion ──ore-sql (dialecto ORACLE)──▶ SELECT … FETCH FIRST n ROWS ONLY     [puro, sin red]
//!   ALL_TAB_COLUMNS ──tipos──▶ Tipo (el escalar de OOS y su físico en Arrow)     [puro]
//!   celdas en texto canónico ──lote──▶ RecordBatch de Arrow                      [puro]
//!   transporte: abrir, catalogar, leer, cancelar                                 [la red]
//! ```
//!
//! # Por qué el thin
//!
//! `oracledb` habla el protocolo de Oracle directamente (sin Instant Client), así que el driver no lleva una
//! librería propietaria de glibc dentro de la imagen musl. Medido el 2026-10-25 contra Oracle 26ai Free: 10⁶ filas
//! en 3 s y 10⁷ en 33–55 s con una memoria de 8–29 MB; lee bien 22 de los 24 tipos de la sonda.
//!
//! # Lo que Oracle tiene de particular
//!
//! - **`''` es `NULL`**: una cadena vacía no existe. Lo que llega nulo, nulo se queda.
//! - **`NUMBER` sin precisión** no cabe en un decimal de Arrow (hasta 38 dígitos con escala libre): viaja como
//!   texto exacto, igual que el `decimal` de Cassandra.
//! - **`DATE` lleva hora**: es `DateTime`, no `Date`.
//! - **Los nombres son mayúsculas** salvo que se hayan creado entre comillas: el catálogo da el nombre exacto y el
//!   conector siempre lo cita.
//!
//! Este crate no abre un socket salvo en [`transporte`]; el resto se prueba sin servidor.

pub mod atado;
pub mod lectura;
pub mod lote;
pub mod tipos;
pub mod transporte;
pub mod url;
pub mod verbos;

pub use lote::{Constructor, ErrorLote};
pub use tipos::Tipo;
