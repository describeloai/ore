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
//! | `leer` | las filas de una `Table` con `format`, en Arrow (`filas.rs`, E6) | la petición |
//! | `versiones` | lo vigente de un `ObjectTable`, con versión y huella, para la transacción de una colección (`versiones.rs`, E8·1) | la petición |
//! | `bajar` | los bytes de los ítems de una colección mantenida, fijados a su versión y cotejados, en tramas (`bajar.rs`, E8·2) | la petición |
//! | `firmar` | URLs prefirmadas de los ítems de una colección virtual, fijadas a su versión; sin abrir un socket (`firmar.rs`, E9·3) | la petición |
//!
//! La URL lleva la credencial y va **siempre por stdin**, nunca por `argv`, y
//! este programa no la imprime: ni en un error (lo que dice S3 no la contiene)
//! ni en `explorar` (`fuente::publica`).

mod acceso;
mod bajar;
mod catalogo;
mod filas;
mod firmar;
mod fuente;
mod medio;
mod origen;
mod tabular;
mod versiones;

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
        "versiones" => serde_json::from_str::<serde_json::Value>(&entrada)
            .map_err(|e| format!("la petición no es JSON: {e}"))
            .and_then(|n| {
                let url = n.get("url").and_then(|u| u.as_str()).unwrap_or("");
                let f = fuente::leer(url)?;
                versiones::versiones(&f.bucket, &entrada)
            }),
        "leer" => ore_driver::leer_peticion(&entrada).and_then(|p| {
            let f = fuente::leer(&p.url)?;
            let salida = std::io::stdout();
            let mut s = std::io::BufWriter::with_capacity(1 << 20, salida.lock());
            let l = filas::leer(&f.bucket, &p, &mut s, filas::UMBRAL)?;
            use std::io::Write as _;
            s.flush()
                .map_err(|e| format!("no se pudo escribir el flujo: {e}"))?;
            let (n, b) = ore_s3::contadores();
            avisos.push(format!(
                "{} filas de {} ficheros, con {n} peticiones y {} KB de cuerpo",
                l.filas,
                l.ficheros,
                b / 1024
            ));
            for (c, k) in &l.rescatados {
                avisos.push(format!(
                    "{k} valores de `{c}` rescatados en `_rescued_data`"
                ));
            }
            Ok(String::new())
        }),
        "bajar" => serde_json::from_str::<serde_json::Value>(&entrada)
            .map_err(|e| format!("la petición no es JSON: {e}"))
            .and_then(|n| {
                let f = fuente::leer(n.get("url").and_then(|u| u.as_str()).unwrap_or(""))?;
                let (pedidos, hilos) = bajar::pedidos(&entrada)?;
                let salida = std::sync::Mutex::new(std::io::BufWriter::with_capacity(
                    1 << 20,
                    std::io::stdout(),
                ));
                let c = bajar::en_paralelo(&f.bucket, &pedidos, hilos, &salida);
                use std::io::Write as _;
                salida
                    .into_inner()
                    .map_err(|_| "la salida quedó envenenada".to_string())?
                    .flush()
                    .map_err(|e| format!("no se pudo escribir el flujo: {e}"))?;
                let (n, _) = ore_s3::contadores();
                avisos.push(format!(
                    "{} ítems entregados ({} KB) y {} que no, con {n} peticiones",
                    c.entregados,
                    c.bytes / 1024,
                    c.fallidos
                ));
                Ok(String::new())
            }),
        "firmar" => serde_json::from_str::<serde_json::Value>(&entrada)
            .map_err(|e| format!("la petición no es JSON: {e}"))
            .and_then(|n| {
                let f = fuente::leer(n.get("url").and_then(|u| u.as_str()).unwrap_or(""))?;
                firmar::firmar(&f, &n)
            }),
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
