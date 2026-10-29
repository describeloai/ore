//! **Los verbos que son de CockroachDB** y no del esqueleto: `catalogo`, `check`, `explorar`, `testigo` y `cambio`.
//! Los cuatro de lectura (`capacidades`, `leer`, `estimar`, `servir`) están en [`crate::lectura`].
//!
//! Hasta el paso 6 de la hoja de ruta (ADR 0080 §6) los cinco contestan un error claro.

use std::process::ExitCode;

/// El `main` de lo que no son los cuatro verbos v2.
pub fn otros(verbo: &str, _args: &[String]) -> ExitCode {
    match verbo {
        "catalogo" | "check" | "explorar" | "testigo" | "cambio" => {
            eprintln!("ore-read-cockroach: todavía no sabe `{verbo}` (ADR 0080 §6, paso 6)");
        }
        otro => eprintln!(
            "ore-read-cockroach: verbo desconocido `{otro}` (capacidades, leer, estimar, servir, catalogo, check, explorar, testigo, cambio)"
        ),
    }
    ExitCode::FAILURE
}
