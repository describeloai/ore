//! **El cliente nativo de HBase** (ficha 0072, N1): lo que habla un cliente Java con HBase, sin la JVM.
//!
//! ZooKeeper dice donde esta `hbase:meta`; `hbase:meta` dice donde esta cada region; el RegionServer de cada
//! region contesta `Scan` y `Get` por un RPC protobuf. Este crate lo hace con la biblioteca estandar y `prost`,
//! de forma sincrona y con una conexion por RegionServer, y no sabe nada de ORE: el mapeo de los bytes a
//! tipos, el incremental y el conector son de `ore-hbase`.
//!
//! Lo que X0 midio contra un HBase 2.4.18 de verdad y la documentacion no dice esta en la cabecera de cada
//! modulo (`meta`, `scan`) y en la ficha, §3.2. Hasta N6 la autenticacion es **SIMPLE**: el cliente afirma un
//! usuario, sin secreto y sin cifrar; un HBase con Kerberos lo rechaza.

mod cancelacion;
mod error;
mod filas;
#[cfg(feature = "kerberos")]
mod gss;
mod lector;
mod meta;
#[rustfmt::skip]
pub mod proto;
mod rpc;
mod sasl;
mod scan;
mod zk;

pub use cancelacion::Cancelacion;
pub use error::{Clase, Error, Resultado};
pub use filas::{claves_del_filtro, filtro_de_filas};
pub use lector::Lector;
pub use meta::{Cliente, REGION_DE_META, Region, TablaEnMeta};
pub use rpc::{Conexion, Opciones, Politica, Servidor};
pub use sasl::{Capa, Contexto, Kerberos};
pub use scan::{Celda, Consulta, Escaner, Fila};
pub use zk::Direccion;
