//! **Cassandra como origen** (ficha 0070, M1): lo que `ore-read-cassandra`
//! necesita y que se puede probar sin servidor.
//!
//! ```text
//!   system_schema ──esquema───▶ Tabla (roles, índices, tipos de OOS)   [puro]
//!   Peticion + Tabla ─traductor▶ Plan (CQL + enlaces + lo que queda)   [puro]
//!   fila decodificada ─residual▶ ¿cumple lo que el servidor no sabe?   [puro]
//!   filas del driver ──lote────▶ RecordBatch de Arrow, con su Tipo     [puro]
//!   transporte: abrir, esquema, leer por rangos de token               [la red]
//! ```
//!
//! # Lo que hace distinto a Cassandra
//!
//! Medido en X0 (`docs/products/0070-origin-cassandra.md` §3):
//!
//! - **El catálogo es real** (`system_schema`) y trae el rol de cada columna:
//!   clave de partición, de clustering, estática o normal. No se infiere nada.
//! - **El servidor admite casi ningún filtro**: sin `ALLOW FILTERING` solo la clave
//!   de partición (`=`, `IN`), los rangos de clustering y lo que tenga un índice.
//!   `!=` e `IS NULL` no existen. El [`traductor`] empuja lo que se puede y deja el
//!   resto para el [`residual`]: el conector devuelve **exactamente** lo pedido.
//! - **No hay orden global ni `killOp` ni tiempo por consulta en el servidor.**
//!
//! Este crate no abre un socket salvo en [`transporte`]; el resto se prueba con
//! filas y tipos escritos a mano.

pub mod esquema;
pub mod lote;
pub mod residual;
pub mod tipos;
pub mod traductor;
pub mod transporte;
pub mod url;

pub use esquema::{Columna, Indice, Rol, Tabla};
pub use tipos::{Cql, Tipo};
