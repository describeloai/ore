//! **ClickHouse como origen** (ADR 0077): lo que `ore-read-clickhouse` necesita.
//!
//! Este crate crece contra el kit de conformidad (`ore-kit --banco clickhouse`), caso a caso; la hoja de ruta
//! está en el ADR (§6). Lo que hay hoy es el esqueleto: el binario existe, habla el protocolo v2 y declara
//! lo que sabe (todavía nada que leer).

pub mod lectura;
pub mod rowbinary;
pub mod traductor;
pub mod transporte;
pub mod url;
pub mod verbos;

pub use lectura::ClickHouse;
