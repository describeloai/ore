//! **CockroachDB como origen** (ADR 0080): lo que `ore-read-cockroach` necesita.
//!
//! Este crate crece contra el kit de conformidad (`ore-kit --banco cockroach`), paso a paso; la hoja de ruta está en el ADR (§6).
//! Lo que hay hoy es el esqueleto: el binario existe, habla el protocolo v2, declara lo que sabe (todavía nada que leer) y
//! `conectar` abre la sesión por TLS, comprueba que es CockroachDB y que su versión sirve.

pub mod lectura;
pub mod tls;
pub mod url;
pub mod verbos;

pub use lectura::Cockroach;
