//! El binario. Todo lo demás vive en la biblioteca, que es lo que se prueba.
use std::process::ExitCode;

fn main() -> ExitCode {
    ore_postgres::arrancar(std::env::args().skip(1).collect())
}
