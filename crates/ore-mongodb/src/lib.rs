//! **MongoDB como origen** (ficha 0069, M1): lo que `ore-read-mongodb` necesita
//! y que se puede probar sin servidor.
//!
//! ```text
//!   Peticion ──traductor──▶ Plan (find o aggregate, en BSON)     [puro, sin red]
//!   documentos ──esquema──▶ Esquema (tipos de OOS por muestreo)  [puro]
//!   documentos ──lote─────▶ RecordBatch de Arrow, con su Tipo    [puro]
//!   transporte: abrir, muestrear, leer, cancelar, testigo        [la red]
//! ```
//!
//! # Lo que hace distinto a Mongo
//!
//! Medido en X0 (`docs/decisions/0069-origin-mongodb.md` §3), contra un Atlas M0
//! y un `mongo:7`:
//!
//! - **No hay catálogo que preguntar**: el esquema se infiere de una muestra
//!   ([`esquema`]), y lo que no se deja tipar se queda `String` y se dice.
//! - **`null`, ausente y «dentro de un array» no son lo que SQL espera**:
//!   `{v: null}` casa el `null`, el campo ausente y un array que contiene
//!   `null`; `{v: 1}` casa un array que contiene `1`. El [`traductor`] escribe
//!   cada operador para que signifique lo que significa en SQL.
//! - **No hay `NULLS LAST`**: se emula con un pipeline ([`traductor::Plan`]).
//!
//! Este crate no abre un socket salvo en [`transporte`]; el resto se prueba con
//! documentos BSON escritos a mano.

pub mod esquema;
pub mod lectura;
pub mod lote;
pub mod traductor;
pub mod transporte;
pub mod url;
pub mod verbos;

mod decimal;

pub use esquema::{Aviso, Clase, Columna, Esquema, Observador, Tipo};
pub use lote::{Constructor, ErrorLote};
pub use traductor::{Plan, Tipos, Traduccion, traducir};
