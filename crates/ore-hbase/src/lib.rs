//! **Apache HBase como origen** (ficha 0072, N3): lo que `ore-read-hbase` necesita.
//!
//! ```text
//!   hbase:meta ──tablas──────▶ qué tablas hay (sin pasar por el maestro)                  [ore-hbase-rpc]
//!   unas filas por región ──muestreo──▶ Tabla (columnas `familia:cualificador`, su tipo)    [transporte]
//!   Peticion ─preparar─▶ intervalos de clave + residuales + qué celdas pedir               [puro]
//!   filas de HBase ─lote─▶ RecordBatch de Arrow (texto: String u Opaque en base64)          [puro]
//! ```
//!
//! # Lo que hace distinto a HBase
//!
//! Medido en X0 y N1–N2 (`docs/products/0072-origin-hbase.md`):
//!
//! - **No hay esquema.** Una tabla son bytes bajo una clave: el catálogo sale de **muestrear**, y lo que no sale en la
//!   muestra no figura. Nada se infiere en silencio: una columna es `String` u `Opaque`, nunca un número (N4).
//! - **El único camino rápido es el rango de la clave de fila.** Lo demás se evalúa aquí, con la semántica de SQL.
//! - **El protocolo es el nativo** (ZooKeeper + RPC protobuf): lo pone `ore-hbase-rpc`, que aguanta regiones que se
//!   mueven o se parten, reinicios y scanners caducados sin perder ni repetir una fila.
//! - **La autenticación es SIMPLE**: afirma un usuario, sin secreto y sin cifrar. La URL tiene que reconocerlo.
//!
//! Este crate no abre un socket: lo hace `ore-hbase-rpc`; el resto se prueba con filas y tipos escritos a mano.

pub mod codificacion;
pub mod esquema;
pub mod filtros;
pub mod lectura;
pub mod lote;
pub mod transporte;
pub mod url;
pub mod verbos;

pub use codificacion::Codificacion;
pub use esquema::{Columna, Tabla, Tipo};
