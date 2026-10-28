//! El binario. Todo lo demás vive en la librería, para que ore-serve escriba en el catálogo con
//! EL MISMO código cuando llegue el corte (ADR 0079 E1a·5).
use std::process::ExitCode;

fn main() -> ExitCode {
    ore_catalogo::arrancar(std::env::args().skip(1).collect())
}
