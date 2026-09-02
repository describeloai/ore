//! `ore-maintain` — el mantenedor delegado, invocado como programa.
//!
//! ```text
//! ore-maintain mantener
//! ```
//!
//! La sesión entera va por stdin: la primera línea la abre, cada línea
//! siguiente es una orden y cada orden tiene una respuesta. Al cerrarse stdin
//! sale el informe. El razonamiento está en [`ore_maintain`] y la decisión de
//! forma en `docs/decisions/0013-el-protocolo-del-mantenedor.md`.
//!
//! # Por qué un bucle y no una llamada por orden
//!
//! Porque **esta pieza recuerda**, y es la única del motor de vistas que lo
//! hace. Un proceso por orden tendría que recibir el estado entero en cada
//! llamada y devolverlo — que para una junta es el integrador de los dos lados,
//! y entonces el transporte costaría más que el mantenimiento. La sesión es el
//! estado, y cerrarla es tirarlo.
//!
//! Sin `clap`, como `ore-exec`: un verbo y ninguna bandera.

use std::io::BufRead as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mantener") => mantener(),
        Some(otro) => {
            eprintln!("ore-maintain: `{otro}` no es un verbo. El único es `mantener`");
            ExitCode::from(64) // EX_USAGE
        }
        None => {
            eprintln!("uso: ore-maintain mantener  (la sesión entera por stdin)");
            ExitCode::from(64)
        }
    }
}

fn mantener() -> ExitCode {
    let entrada = std::io::stdin().lock();
    let mut lineas = entrada.lines().map_while(Result::ok);

    // La primera línea abre la sesión. Que abrir sea una línea y no una bandera
    // es lo que permite que el plan viaje por stdin: un plan en `argv` sería un
    // plan con un límite de longitud que depende del sistema operativo.
    let Some(primera) = lineas.next() else {
        eprintln!("ore-maintain: stdin vacío — la primera línea abre la sesión");
        return ExitCode::from(65); // EX_DATAERR
    };
    let sesion = match ore_core::parse::parse(&primera) {
        Ok(n) => ore_maintain::Sesion::abrir(&n),
        Err(e) => Err(format!("la sesión no analiza: {e:?}")),
    };
    let mut sesion = match sesion {
        Ok(s) => s,
        Err(m) => {
            eprintln!("ore-maintain: {m}");
            return ExitCode::from(65);
        }
    };

    for linea in lineas {
        if linea.trim().is_empty() {
            continue;
        }
        // Una línea que no analiza **no cierra la sesión**: se contesta con un
        // error y se sigue. Cerrarla tiraría el estado de todas las anteriores
        // por una coma mal puesta.
        let respuesta = match ore_core::parse::parse(&linea) {
            Ok(n) => sesion.atender(&n),
            Err(e) => ore_core::json::Json::obj([
                ("op", ore_core::json::Json::s("")),
                (
                    "error",
                    ore_core::json::Json::s(format!("la orden no analiza: {e:?}")),
                ),
            ]),
        };
        println!("{}", respuesta.jcs());
    }

    println!("{}", sesion.fin().jcs());
    ExitCode::SUCCESS
}
