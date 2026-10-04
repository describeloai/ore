//! `ore-packages` — la ficha o la búsqueda de una librería en su registro
//! (0050 L6·2·1), por la entrada estándar.
//!
//! ```text
//!   echo '{"entorno":"node","nombre":"lodash"}' | ore-packages
//!   → {"descripcion":"Lodash modular utilities.","licencia":"MIT","nombre":"lodash","ultima":"4.17.21",…}
//!   echo '{"entorno":"node","q":"date"}' | ore-packages
//!   → {"resultados":[{"nombre":"date-fns","ultima":"4.1.0",…},…]}
//! ```
//!
//! Lo lanza `ore-serve`, que no habla TLS (`ore-cli/tests/dependencias.rs`).
//! Salidas: 0 con el JSON; 64 lo pedido no vale (422); 65 no existe (404); 69 el
//! registro no contestó bien (502). El porqué, en una línea por stderr.
use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut entrada = String::new();
    if std::io::stdin().read_to_string(&mut entrada).is_err() || entrada.trim().is_empty() {
        eprintln!(
            "✗ la petición va por la entrada estándar: {{\"entorno\", \"nombre\"}} o {{\"entorno\", \"q\"}}"
        );
        return ExitCode::from(64);
    }
    let r = ore_packages::peticion(&entrada).and_then(|p| ore_packages::responder(&p));
    match r {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(f) => {
            eprintln!("✗ {}", f.mensaje);
            ExitCode::from(match f.codigo {
                422 => 64,
                404 => 65,
                _ => 69,
            })
        }
    }
}
