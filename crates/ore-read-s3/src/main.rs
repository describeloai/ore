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
//! | `leer` | las filas de una `Table` con `format`, en Arrow (`filas.rs`, E6; v2 en 0053 F2·4: diez operadores, particiones y grupos de filas descartados, `limit`, `timeoutMs`) | la petición |
//! | `estimar` | los ficheros y los bytes que leer costaría, del listado | la petición |
//! | `servir` | peticiones una tras otra, la fuente resuelta guardada entre ellas | una petición por línea |
//! | `capacidades` | lo que este lector sabe poner (`docs/federation.md` §1.4) | nada |
//! | `versiones` | lo vigente de un `ObjectTable`, con versión y huella, para la transacción de una colección (`versiones.rs`, E8·1) | la petición |
//! | `bajar` | los bytes de los ítems de una colección mantenida, fijados a su versión y cotejados, en tramas (`bajar.rs`, E8·2) | la petición |
//!
//! La URL lleva la credencial y va **siempre por stdin**, nunca por `argv`, y
//! este programa no la imprime: ni en un error (lo que dice S3 no la contiene)
//! ni en `explorar` (`fuente::publica`).

mod acceso;
mod bajar;
mod catalogo;
mod filas;
mod fuente;
mod medio;
mod origen;
mod tabular;
mod versiones;

use std::io::Read as _;
use std::process::ExitCode;

/// **Un error de este lector, con su código del contrato** (`docs/federation.md`
/// §1.3). Los errores de dentro son texto; aquí se clasifican por lo que S3
/// dijo (`403 AccessDenied`, `404 NoSuchKey`…) o por el freno de la lectura.
fn fallo(m: String) -> ore_driver::Fallo {
    use ore_driver::{Codigo, Fallo};
    let tiene = |xs: &[&str]| xs.iter().any(|x| m.contains(x));
    let codigo = if m.starts_with(filas::AGOTADO) {
        Codigo::Tiempo
    } else if m.starts_with(filas::CANCELADA) {
        Codigo::Origen
    } else if tiene(&[
        "403",
        "AccessDenied",
        "InvalidAccessKeyId",
        "SignatureDoesNotMatch",
        "ExpiredToken",
        "falta la credencial",
        "falta `secret_access_key`",
    ]) {
        Codigo::Credencial
    } else if tiene(&["NoSuchBucket", "NoSuchKey", "404"]) {
        Codigo::Objeto
    } else if tiene(&["el filtro `", "`like` sobre", "no es un operador"]) {
        Codigo::Operador
    } else if tiene(&["timed out", "Connection", "connection", "dns error", "Dns"]) {
        Codigo::Conexion
    } else {
        Codigo::Origen
    };
    Fallo::new(codigo, m)
}

/// `leer` y `estimar`, con su error ya en el código del contrato.
fn leer_o_estimar(
    verbo: &str,
    entrada: &str,
    avisos: &mut Vec<String>,
) -> Result<(), ore_driver::Fallo> {
    use std::io::Write as _;
    let p = ore_driver::leer_peticion(entrada).map_err(ore_driver::Fallo::operador)?;
    filas::CAPACIDADES.admite(&p)?;
    let f = fuente::leer(&p.url).map_err(fallo)?;
    if verbo == "estimar" {
        println!("{}", filas::estimar(&f.bucket, &p).map_err(fallo)?);
        return Ok(());
    }
    let salida = std::io::stdout();
    let mut s = std::io::BufWriter::with_capacity(1 << 20, salida.lock());
    let l = filas::leer(&f.bucket, &p, &mut s, filas::UMBRAL).map_err(fallo)?;
    s.flush()
        .map_err(|e| fallo(format!("no se pudo escribir el flujo: {e}")))?;
    let (n, b) = ore_s3::contadores();
    avisos.push(format!(
        "{} filas de {} ficheros ({} descartados por partición, {} grupos de filas por \
         estadísticas), con {n} peticiones y {} KB de cuerpo",
        l.filas,
        l.ficheros,
        l.descartados,
        l.grupos_descartados,
        b / 1024
    ));
    for (c, k) in &l.rescatados {
        avisos.push(format!(
            "{k} valores de `{c}` rescatados en `_rescued_data`"
        ));
    }
    Ok(())
}

/// **`servir`** (ADR 0053 F2·4): peticiones una tras otra. Lo que se guarda
/// entre una y otra es la fuente resuelta —y con ella la credencial temporal
/// que un rol canjea (`ore-sts`), que dura una hora: se renueva a los 50
/// minutos—.
fn servir() -> ExitCode {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    let cancelada = Arc::new(AtomicBool::new(false));
    let para_cancelar = cancelada.clone();
    let mut fuentes: ore_driver::servir::Conexiones<(fuente::Fuente, Instant)> =
        ore_driver::servir::Conexiones::new(Duration::from_secs(60));
    let stdout = std::io::stdout();
    let mut salida = std::io::BufWriter::with_capacity(1 << 20, stdout.lock());
    let hecho = ore_driver::servir::servir(
        std::io::BufReader::new(std::io::stdin()),
        &mut salida,
        Arc::new(move || para_cancelar.store(true, Ordering::SeqCst)),
        |p, r| {
            cancelada.store(false, Ordering::SeqCst);
            filas::CAPACIDADES.admite(p)?;
            let caduca = fuentes
                .tomar(&p.url, |u| fuente::leer(u).map(|f| (f, Instant::now())))
                .map_err(fallo)?
                .1
                .elapsed()
                > Duration::from_secs(50 * 60);
            if caduca {
                fuentes.quitar(&p.url);
            }
            let (f, _) = fuentes
                .tomar(&p.url, |u| fuente::leer(u).map(|f| (f, Instant::now())))
                .map_err(fallo)?;
            filas::leer_con(&f.bucket, p, r, filas::UMBRAL, Some(cancelada.clone()))
                .map(|l| l.filas)
                .map_err(fallo)
        },
    );
    match hecho {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ore-read-s3: servir: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbo = args.first().map(String::as_str).unwrap_or("catalogo");
    // Los que no leen stdin antes de empezar.
    match verbo {
        "capacidades" => {
            println!("{}", filas::CAPACIDADES.json());
            return ExitCode::SUCCESS;
        }
        "servir" => return servir(),
        _ => {}
    }

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
        // Los verbos del conector v2: el error, tipado y tapado.
        "leer" | "estimar" => {
            let url = ore_driver::leer_peticion(&entrada)
                .map(|p| p.url)
                .unwrap_or_default();
            let hecho = leer_o_estimar(verbo, &entrada, &mut avisos);
            for a in &avisos {
                eprintln!("ore-read-s3: aviso · {a}");
            }
            return match hecho {
                Ok(()) => ExitCode::SUCCESS,
                Err(f) => {
                    eprintln!("ore-read-s3: {}", f.tapado(&url).linea());
                    ExitCode::FAILURE
                }
            };
        }
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
