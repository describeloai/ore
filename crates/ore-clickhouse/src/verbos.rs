//! **Los verbos que no son los cuatro del protocolo v2** (`catalogo`, `check`, `explorar`, `testigo`, `cambio`).
//! Ninguno existe todavía: cada uno entra con su paso de la hoja de ruta (ADR 0077 §6).

use std::process::ExitCode;

pub fn otros(verbo: &str, _args: &[String]) -> ExitCode {
    eprintln!(
        "ore-read-clickhouse: el verbo `{verbo}` todavía no existe (ADR 0077, esqueleto); hay: capacidades"
    );
    ExitCode::FAILURE
}
