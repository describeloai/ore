//! # ore-core
//!
//! El núcleo L0 de ORE: analizar, normalizar, validar, comprobar el flujo de
//! información y emitir el digest.
//!
//! ## Por qué L0 primero
//!
//! De los cuatro niveles de conformidad de OOS, **L0 es el único decidible por
//! una suite de ficheros**: es completamente hermético — sin red, sin
//! credenciales, sin reloj, sin tocar un solo dato. L1 exige un proceso vivo;
//! L2 y L3, fuentes reales.
//!
//! Y es precisamente el nivel que carga las dos garantías que definen el
//! producto:
//!
//! - **G1** · identidad determinista: el mismo commit produce el mismo digest.
//! - **G2** · gobernanza demostrada: si compila, ningún dato clasificado
//!   alcanza un conducto no autorizado. No es una alerta — es que no compila.
//!
//! ## Invariante que este crate no puede romper
//!
//! La compilación es **pura**: `bundle = f(fuente@commit, versión OOS, lock)`.
//! Sin red, sin credenciales, sin reloj, sin aleatoriedad, sin variables de
//! entorno. Nada de este crate debe leer ninguna de esas cosas.
//!
//! Es lo que hace verdad la frase que vende el producto: *el paso que decide
//! qué significan las cosas es el único que no puede filtrar nada.*

pub mod code;
pub mod diag;
pub mod document;
pub mod parse;
pub mod validate;

pub use code::{Code, Family};
pub use diag::{Diagnostic, Pos};
pub use validate::{validate_document, validate_package};

// ── Pendiente. Fase 0 ───────────────────────────────────────────────────────
//
// pub mod normalize;  N1–N8 de 90-canonical-form
// pub mod canonical;  serialización JCS (RFC 8785)
// pub mod digest;     doc · paquete · bundle
// pub mod link;       integridad referencial   → OOS2xxx
// pub mod types;      sistema de tipos         → OOS3xxx
//
// ── Fase 2 ──────────────────────────────────────────────────────────────────
//
// pub mod lattice;    retículos y orden parcial
// pub mod flow;       propagación y regla de flujo → OOS4xxx
// pub mod cedar;      proyección a esquema Cedar
//
// ── Fase posterior ──────────────────────────────────────────────────────────
//
// pub mod diff;       taxonomía de cambio rompedor → OOS5xxx
// pub mod emit;       ODCS · Ossie · Cedar
