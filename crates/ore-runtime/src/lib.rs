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
//! | [`motor`] | el motor del proceso: el `RuntimeEnv`, los dos runtimes de tokio y la plantilla de sesión |
//! | [`sesion`] | la sesión de cada consulta: la plantilla clonada, su contexto, planificar |
//! | [`correr`] | ejecutar sin ahogar ni caer: la CPU aparte, los pánicos como fallo, soltar es cancelar |
//! | [`diagnostico`] | el runtime se describe y se mide en su pod: `ore-runtime diagnostico` |
//! | [`codecs`] | cómo viajan por Ballista los nodos de los frentes, envueltos con los de Ballista |
//! | [`papeles`] | el scheduler y el ejecutor de Ballista, arrancados desde el motor |
//! | [`registro`] | lo que Ballista y DataFusion escriben, a stderr |
//! | [`zocalos`] | los puntos de enchufe de los demás frentes |
//! | [`fallo`] | los errores |
//!
//! Dos objetos, nunca uno: el **motor** vive lo que el proceso; la **sesión**, lo que la consulta.

pub mod codecs;
pub mod contrato;
pub mod correr;
pub mod ddl;
pub mod diagnostico;
pub mod fallo;
pub mod limites;
pub mod manifiesto;
pub mod motor;
pub mod opciones;
pub mod papeles;
pub mod registro;
pub mod sentencia;
pub mod sesion;
pub mod viaje;
pub mod zocalos;

pub use correr::Consulta;
pub use fallo::Fallo;
pub use limites::Limites;
pub use manifiesto::Manifiesto;
pub use motor::Motor;
pub use opciones::{OreOpciones, Papel};
pub use sesion::{Contexto, Plan, Sesion};
pub use zocalos::Zocalos;
