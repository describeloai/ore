//! # ore-exec — el ejecutor L2
//!
//! Aquí vive lo que `ore` no puede llevar dentro. La costura es la misma que la
//! del driver, y el motivo es distinto:
//!
//! > **El driver está vetado por saber hablar por la red. El evaluador está
//! > vetado por saber qué hora es.**
//!
//! `cedar-policy-core` depende de `chrono` porque Cedar 4 tiene una extensión
//! `datetime` y una política puede decir *«antes del 1 de enero»*. Pero el reloj
//! es la evidencia, no el argumento: el compilador contesta *«¿qué dice este
//! documento?»* y el evaluador *«¿puede **este** principal?»*, y la segunda
//! pregunta necesita una **petición**, que `ore validate` no tiene.
//!
//! Medición y razonamiento completos en
//! `docs/decisions/0007-enlazar-el-evaluador-de-cedar.md`.
//!
//! # La regla que gobierna este crate
//!
//! El esquema que carga el evaluador es **el que emite `ore export --format
//! cedarschema`**, obtenido de `ore_core::cedar_schema`. No uno equivalente: el
//! mismo. Si divergieran, la política se habría validado contra uno y se
//! evaluaría contra otro, y ninguna prueba lo vería — el fallo no tiene aspecto
//! de fallo.

pub mod autorizar;
pub mod motor;
pub mod plan;
pub mod topologia;

pub use autorizar::{Denegacion, Identidad, Peticion, Veredicto};
pub use plan::{Consulta, Filtro, Lectura, Plan, Rechazo};
pub use plan::Travesia;
pub use topologia::{Arista, Topologia};
pub use motor::{Carga, Motor};
