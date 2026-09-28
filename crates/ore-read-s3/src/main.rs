//! `ore-read-s3` — el lector de un bucket de S3, **la familia de los objetos**
//! (ADR 0046 E4).
//!
//! Las otras familias leen filas de algo que ya las tiene: una base de datos,
//! un fichero NDJSON. Un bucket guarda **ficheros**, y de un fichero sale una de
//! dos cosas: filas —si es un Parquet, un CSV, un JSONL— o un objeto —un PDF,
//! una foto, un zip—. El catálogo de este lector dice las dos, y `ore source
//! induce` las escribe como `Table` con `format` y como `ObjectTable`
//! (spec v1alpha16).
//!
//! # Los verbos
//!
//! | verbo | qué | entrada (stdin) |
//! |---|---|---|
//! | `check` | qué acción falta y sobre qué ARN (`acceso.rs`) | la coordenada |
//! | `explorar` | las carpetas del bucket, con su URL sin credencial | la coordenada |
//! | `catalogo <fuente>` | tablas y conjuntos de objetos (`catalogo.rs`) | la URL |
//! | `testigo` | la huella del listado de un objeto o un prefijo | la coordenada |
//! | `leer` | **todavía no**: llega con E6 (lo tabular) | — |
//!
//! La URL lleva la credencial y va **siempre por stdin**, nunca por `argv`, y
//! este programa no la imprime: ni en un error (lo que dice S3 no la contiene)
//! ni en `explorar` (`fuente::publica`).

mod acceso;
mod catalogo;
mod fuente;
mod medio;
mod origen;
mod tabular;

use std::io::Read as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbo = args.first().map(String::as_str).unwrap_or("catalogo");

    let mut entrada = String::new();
    if std::io::stdin().read_to_string(&mut entrada).is_err() {
        eprintln!("ore-read-s3: no se pudo leer stdin");
        return ExitCode::FAILURE;
    }

    let mut avisos: Vec<String> = Vec::new();
    let resultado = match verbo {
        "catalogo" => fuente::leer(entrada.trim()).and_then(|f| {
            let c = catalogo::leer(
                &f.bucket,
                args.get(1).map(String::as_str).unwrap_or("fuente"),
                &f.prefijo,
                &mut avisos,
            )?;
            let (n, b) = ore_s3::contadores();
            avisos.push(format!(
                "catalogado con {n} peticiones y {} KB leídos: {} tablas y {} conjuntos de objetos",
                b / 1024,
                c.tablas.len(),
                c.objetos.len()
            ));
            Ok(ore_driver::catalogo::escribir(&c))
        }),
        "check" => ore_driver::leer_coordenada(&entrada)
            .and_then(|(url, _)| fuente::leer(&url))
            .map(|f| acceso::comprobar(&f)),
        "explorar" => ore_driver::leer_coordenada(&entrada)
            .and_then(|(url, _)| fuente::leer(&url))
            .and_then(|f| acceso::explorar(&f)),
        "testigo" => ore_driver::leer_coordenada(&entrada).and_then(|(url, objeto)| {
            let f = fuente::leer(&url)?;
            let objeto = if objeto.is_empty() {
                f.prefijo.clone()
            } else {
                objeto
            };
            catalogo::testigo(&f.bucket, &objeto)
        }),
        "leer" => Err(
            "leer las filas de un bucket llega con 0046 E6 (lo tabular); hoy este lector \
             cataloga, comprueba y fecha"
                .to_string(),
        ),
        otro => Err(format!("`{otro}` no es un verbo de este lector")),
    };
    for a in &avisos {
        eprintln!("ore-read-s3: aviso · {a}");
    }
    match resultado {
        Ok(salida) => {
            if !salida.is_empty() {
                println!("{salida}");
            }
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-read-s3: {m}");
            ExitCode::FAILURE
        }
    }
}
