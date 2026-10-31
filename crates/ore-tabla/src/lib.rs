//! **ORE SQL sobre el lago** (0081 §7.5, C4). Lo que el contrato pone encima del ORE Runtime para leer:
//!
//! | módulo | qué |
//! |---|---|
//! | [`foto`] | **la foto**: todos los nombres de una ref en una lectura, de los punteros |
//! | [`catalogo`] | el zócalo de catálogo de ORE: los catálogos de **una** consulta, sobre su foto |
//! | [`tabla`] | una tabla Iceberg del lago como `TableProvider`, en el snapshot de la foto |
//! | `poda` | los filtros como predicado de Iceberg al planificar, y la guarda de lo planificado (K2) |
//! | [`escritura`] | escribir: el sink de ORE, la puerta del commit y los verbos que cumple el lago (C5) |
//! | [`codec`] | los nodos de ORE en el plan distribuido: viajan con lo que el contrato fijó al planificar (C8) |
//! | [`v1`] | la puerta de producción: `/v1` de `ore-serve`, como el SDK |
//! | [`lagos`] | el lago de cada tabla: uno para todo, o el que `/v1` presta por tabla (C10·2) |
//! | [`memoria`] | un lago en memoria y su puerta, para las pruebas y la vara |
//! | [`medida`] | lo que cuesta planificar: las peticiones a `/v1` y las lecturas del lago prestado (K1) |
//!
//! Cada base de ORE es un catálogo de DataFusion, cada schema un schema y cada dataset o vista una tabla:
//! `base.schema.nombre` es el nombre de tres partes, sin reescribir nada.

pub mod catalogo;
pub mod codec;
pub mod escritura;
pub mod foto;
pub mod lagos;
pub mod medida;
pub mod memoria;
mod poda;
pub mod tabla;
pub mod v1;

pub use catalogo::CatalogoOre;
pub use codec::CodecOre;
pub use escritura::{Puerta, ejecutable};
pub use foto::{Foto, Fuente, Nombre};
