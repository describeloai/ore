//! **Un cliente del protocolo Avatica** (Apache Calcite), el que habla el Phoenix Query Server
//! (ficha 0071, M1).
//!
//! No existe uno en Rust. El protocolo son unos ocho mensajes protobuf en un `POST` de HTTP, y este crate
//! los habla sin saber nada de Phoenix: abre una conexión, ejecuta una consulta, la pagina, la cierra y lee
//! los metadatos (`esquemas`, `tablas`, `columnas`). Los tipos de Phoenix y el SQL son de `ore-phoenix`.
//!
//! Lo que X0 midió contra un PQS de verdad y la documentación no dice (§3.3 de la ficha) es lo que este
//! crate cumple siempre: el primer frame se pide con tamaño, `missing_statement` se lee, el `offset` del
//! `Fetch` no se honra y un frame sin filas ni `done` es un error. Los bytes de un PQS real están en
//! `pruebas-de-fuego/phoenix/trazas/` y son los fixtures de `tests/`.

mod cliente;
mod error;
#[rustfmt::skip]
pub mod proto;
mod transporte;
mod valor;

pub use cliente::{Conexion, Lectura, Tabla};
pub use error::Error;
pub use transporte::Opciones;
pub use valor::{Anulable, Columna, Componente, Fila, Valor};
