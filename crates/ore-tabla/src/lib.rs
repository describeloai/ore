//! **ORE SQL sobre el lago** (0081 §7.5, C4). Lo que el contrato pone encima del ORE Runtime para leer:
//!
//! | módulo | qué |
//! |---|---|
//! | [`foto`] | **la foto**: todos los nombres de una ref en una lectura, de los punteros |
//! | [`catalogo`] | el zócalo de catálogo de ORE: los catálogos de **una** consulta, sobre su foto |
//! | [`tabla`] | una tabla Iceberg del lago como `TableProvider`, en el snapshot de la foto |
//! | [`memoria`] | un lago en memoria, para las pruebas y la vara |
//!
//! Cada base de ORE es un catálogo de DataFusion, cada schema un schema y cada dataset o vista una tabla:
//! `base.schema.nombre` es el nombre de tres partes, sin reescribir nada.

pub mod catalogo;
pub mod foto;
pub mod memoria;
pub mod tabla;

pub use catalogo::CatalogoOre;
pub use foto::{Foto, Fuente, Nombre};
