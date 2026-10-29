//! **CockroachDB como origen** (ADR 0080): lo que `ore-read-cockroach` necesita.
//!
//! Este crate crece contra el kit de conformidad (`ore-kit --banco cockroach`), paso a paso; la hoja de ruta está en el ADR (§6).
//! Lo que hay hoy: el binario habla el protocolo v2, `conectar` abre la sesión por TLS y comprueba que es CockroachDB y que su
//! versión sirve, y `leer` lee la proyección de una tabla entera (paso 2); filtros, orden y límite llegan en los pasos siguientes.

pub mod lectura;
pub mod leer;
pub mod tipos;
pub mod tls;
pub mod url;
pub mod verbos;

pub use lectura::Cockroach;
