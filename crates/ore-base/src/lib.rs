//! **El vocabulario de ORE** (CI C4, `docs/ci-en-minutos.md` §4.3): cómo se lee y se
//! escribe un dato. El analizador de YAML/JSON con posiciones (`parse`), los
//! diagnósticos y sus códigos (`diag`, `code`), el JSON canónico (`json`), las firmas y
//! el registro de transparencia (`firma`, `transparencia`) y los nombres de medios
//! (`medios`).
//!
//! Lo usan todos los binarios; el árbol (la ontología y sus reglas) vive en `ore-core`,
//! que re-exporta esto tal cual. ⛔ Este crate no depende de `ore-core` ni de `ore-code`:
//! si algo de aquí necesita el árbol, no es vocabulario.
pub mod code;
pub mod diag;
pub mod firma;
pub mod json;
pub mod medios;
pub mod parse;
pub mod transparencia;

pub use code::{Code, Family};
pub use diag::{Diagnostic, Pos};
