//! **Salesforce como origen tabular** (ADR 0083): lo que `ore-read-salesforce`
//! necesita y que se puede probar sin una org.
//!
//! ```text
//!   Peticion ──traductor──▶ SOQL (una o varias consultas)          [puro, sin red]
//!   describe (JSON) ──esquema──▶ Objeto (campos y qué se puede hacer con ellos)   [puro]
//!   (usuario, clave) ──identidad──▶ JWT firmado ──▶ token          [la firma, pura]
//!   transporte: canjear, llamar, reintentar, contar la cuota       [la red]
//! ```
//!
//! # Lo que hace distinto a Salesforce
//!
//! Medido en X0 (`docs/products/0083-origin-salesforce.md` §3), contra una
//! Developer Edition:
//!
//! - **SOQL empuja la proyección** (B1): lo que se pide es lo que sale. Es lo
//!   que permite que una columna enmascarada no esté en la petición.
//! - **`!=` incluye los nulos** (B3), **`eq` no distingue mayúsculas** (B2), el
//!   orden por defecto pone **los nulos primero en los dos sentidos** (B9) y
//!   `NOT` sin paréntesis no compila (B8). El [`traductor`] escribe cada
//!   operador para que dé las filas que daría SQL, y lo que no puede escribir
//!   exacto lo empuja de más y lo marca como **residual** para reevaluarlo.
//! - **El texto largo no se filtra ni se ordena** (B7), **los compuestos no
//!   filtran** (B6) y una consulta por GET no pasa de ~8 KB (B14): se rechaza o
//!   se parte, nunca se ignora.
//!
//! Este crate no abre un socket salvo en [`transporte`].

pub mod bulk;
pub mod csv;
pub mod esquema;
pub mod identidad;
pub mod lectura;
pub mod residual;
pub mod salida;
pub mod traductor;
pub mod transporte;
pub mod url;
pub mod valores;
pub mod verbos;

pub use esquema::{Campo, Objeto};
pub use traductor::{OPERADORES, Traduccion, traducir};
pub use url::Fuente;
