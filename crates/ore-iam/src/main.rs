//! El binario. Todo lo demas vive en la biblioteca, para que `ore-cofre`
//! use EL MISMO motor de autorizacion y no una copia suya.
use std::process::ExitCode;

fn main() -> ExitCode {
    ore_iam::arrancar(std::env::args().skip(1).collect())
}
