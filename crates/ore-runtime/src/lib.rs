//! # ORE Runtime
//!
//! **El suelo del motor de ORE** (ADR 0081 §7.4). Es al motor lo que el Databricks Runtime es a Databricks:
//! un runtime con nombre y versión del que nace igual cualquier ejecución de la plataforma. No es una pieza
//! que haga cosas: es la placa base. Define los zócalos donde encajan los demás frentes y garantiza que lo
//! que se enchufa en ellos corre siempre igual.
//!
//! | módulo | qué es |
//! |---|---|
//! | [`manifiesto`] | qué runtime es: sus versiones, comprobadas contra `Cargo.lock` |
//! | [`limites`] | lo que el proceso tiene: memoria, CPU y disco, del cgroup |
//! | [`opciones`] | la configuración como contrato: `ore.*`, los desvíos de DataFusion y el perfil por papel |
//! | [`motor`] | el motor del proceso: el `RuntimeEnv` y los dos runtimes de tokio |
//! | [`zocalos`] | los puntos de enchufe de los demás frentes |
//! | [`fallo`] | los errores |
//!
//! Dos objetos, nunca uno: el **motor** vive lo que el proceso; la **sesión**, lo que la consulta.

pub mod fallo;
pub mod limites;
pub mod manifiesto;
pub mod motor;
pub mod opciones;
pub mod zocalos;

pub use fallo::Fallo;
pub use limites::Limites;
pub use manifiesto::Manifiesto;
pub use motor::Motor;
pub use opciones::{OreOpciones, Papel};
pub use zocalos::Zocalos;
