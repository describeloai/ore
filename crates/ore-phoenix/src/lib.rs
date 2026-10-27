//! **Apache Phoenix como origen** (ficha 0071, M2): lo que `ore-read-phoenix` necesita.
//!
//! ```text
//!   getColumns ───esquema────▶ Tabla (tipos, clave primaria)            [puro, de los metadatos]
//!   Peticion + Tabla ─consulta▶ SELECT de la proyección + LIMIT          [puro]
//!   filas de Avatica ─lote───▶ RecordBatch de Arrow, con su Tipo         [puro]
//!   transporte: abrir, tabla, consulta, errores → los siete códigos      [la red, por ore-avatica]
//! ```
//!
//! # Lo que hace distinto a Phoenix
//!
//! Medido en X0 y en M1 (`docs/decisions/0071-origin-phoenix.md` §3):
//!
//! - **El protocolo es Avatica por el Phoenix Query Server**, y no hay cliente en Rust: lo pone `ore-avatica`.
//! - **El catálogo es real** (`getColumns`): tipos y clave primaria no se infieren.
//! - **Los errores llegan todos con código −1**: el de Phoenix va en el texto (`ERROR 1012 (42M03)`).
//! - **`TIMESTAMP` llega en milisegundos y `DATE` en días**: lo que no llega no se inventa.
//! - **El único camino rápido es el rango de la clave primaria**; M2 no empuja filtros (M4).
//!
//! Este crate no abre un socket: lo hace `ore-avatica`; el resto se prueba con filas y tipos escritos a mano.

pub mod esquema;
pub mod incremental;
pub mod lectura;
pub mod lote;
pub mod tipos;
pub mod traductor;
pub mod transporte;
pub mod url;
pub mod verbos;

pub use esquema::{Columna, Tabla};
pub use tipos::Tipo;
