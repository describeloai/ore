//! **El `main` de un driver de objetos** (ADR 0061 O0·2): los verbos del
//! contrato (ADR 0008, `docs/federation.md` §1), una vez para todos los
//! proveedores. Cada uno pone lo suyo en un [`Proveedor`].
//!
//! | verbo | qué | entrada (stdin) |
//! |---|---|---|
//! | `check` | qué permiso falta y sobre qué (del proveedor) | la coordenada |
//! | `explorar` | las carpetas, con su URL sin credencial (del proveedor) | la coordenada |
//! | `catalogo <fuente>` | tablas y conjuntos de objetos (`catalogo.rs`) | la URL |
//! | `testigo` | la huella del listado de un objeto o un prefijo | la coordenada |
//! | `leer` | las filas de una `Table` con `format`, en Arrow (`filas.rs`, E6; v2 en 0053 F2·4) | la petición |
//! | `estimar` | los ficheros y los bytes que leer costaría, del listado | la petición |
//! | `servir` | peticiones una tras otra, la fuente resuelta guardada entre ellas | una petición por línea |
//! | `capacidades` | lo que este lector sabe poner (`docs/federation.md` §1.4) | nada |
//! | `versiones` | lo vigente de un `ObjectTable`, con versión y huella (`versiones.rs`, E8·1) | la petición |
//! | `bajar` | los bytes de los ítems de una colección mantenida, fijados y cotejados, en tramas (`bajar.rs`, E8·2) | la petición |
//!
//! La URL lleva la credencial y va **siempre por stdin**, nunca por `argv`, y
//! el driver no la imprime: ni en un error ni en `explorar`.

use crate::{bajar, catalogo, filas, versiones};
use ore_objetos::Origen;
use std::io::Read as _;
use std::process::ExitCode;

/// **Lo propio de un proveedor**: cómo se lee su URL, cómo se comprueba el
/// acceso y cómo se exploran sus carpetas, cómo se clasifica lo que contesta,
/// y su [`Origen`].
pub trait Proveedor {
    /// El nombre del binario, para los mensajes (`ore-read-s3`).
    const NOMBRE: &'static str;
    /// La fuente resuelta: la URL leída, con su credencial (canjeada, si es
    /// corta).
    type Fuente;
    fn leer(url: &str) -> Result<Self::Fuente, String>;
    /// El prefijo de la URL: lo que se cataloga si no se dice otro objeto.
    fn prefijo(f: &Self::Fuente) -> &str;
    /// El almacén de la fuente, como [`Origen`].
    fn origen(f: &Self::Fuente) -> Box<dyn Origen + Sync + '_>;
    /// `check`: qué permiso falta y sobre qué, en una línea de JSON.
    fn comprobar(f: &Self::Fuente) -> String;
    /// `explorar`: las carpetas, con su URL sin credencial.
    fn explorar(f: &Self::Fuente) -> Result<String, String>;
    /// Un error de dentro, con su código del contrato (`docs/federation.md`
    /// §1.3): lo de todos lo clasifica [`clasificar`], y lo del proveedor
    /// (sus códigos de credencial y de objeto que no está) lo dice él.
    fn fallo(m: String) -> ore_driver::Fallo;
    /// Cuántas peticiones y cuántos bytes de cuerpo, para los avisos.
    fn contadores() -> (usize, usize);
}

/// **El código del contrato de un error de un lector de objetos**: el freno de
/// la lectura y los filtros, que son de todos; lo que el origen contesta, por
/// las palabras que el proveedor dice (`credencial`, `objeto`).
pub fn clasificar(m: String, credencial: &[&str], objeto: &[&str]) -> ore_driver::Fallo {
    use ore_driver::{Codigo, Fallo};
    let tiene = |xs: &[&str]| xs.iter().any(|x| m.contains(x));
    let codigo = if m.starts_with(filas::AGOTADO) {
        Codigo::Tiempo
    } else if m.starts_with(filas::CANCELADA) {
        Codigo::Origen
    } else if tiene(credencial) || tiene(&["falta la credencial"]) {
        Codigo::Credencial
    } else if tiene(objeto) {
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
fn leer_o_estimar<P: Proveedor>(
    verbo: &str,
    entrada: &str,
    avisos: &mut Vec<String>,
) -> Result<(), ore_driver::Fallo> {
    use std::io::Write as _;
    let p = ore_driver::leer_peticion(entrada).map_err(ore_driver::Fallo::operador)?;
    filas::CAPACIDADES.admite(&p)?;
    let f = P::leer(&p.url).map_err(P::fallo)?;
    let o = P::origen(&f);
    if verbo == "estimar" {
        println!("{}", filas::estimar(&*o, &p).map_err(P::fallo)?);
        return Ok(());
    }
    let salida = std::io::stdout();
    let mut s = std::io::BufWriter::with_capacity(1 << 20, salida.lock());
    let l = filas::leer(&*o, &p, &mut s, filas::UMBRAL).map_err(P::fallo)?;
    s.flush()
        .map_err(|e| P::fallo(format!("no se pudo escribir el flujo: {e}")))?;
    let (n, b) = P::contadores();
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
/// que un rol canjea, que dura una hora: se renueva a los 50 minutos—.
fn servir<P: Proveedor>() -> ExitCode {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    let cancelada = Arc::new(AtomicBool::new(false));
    let para_cancelar = cancelada.clone();
    let mut fuentes: ore_driver::servir::Conexiones<(P::Fuente, Instant)> =
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
                .tomar(&p.url, |u| P::leer(u).map(|f| (f, Instant::now())))
                .map_err(P::fallo)?
                .1
                .elapsed()
                > Duration::from_secs(50 * 60);
            if caduca {
                fuentes.quitar(&p.url);
            }
            let (f, _) = fuentes
                .tomar(&p.url, |u| P::leer(u).map(|f| (f, Instant::now())))
                .map_err(P::fallo)?;
            let o = P::origen(f);
            filas::leer_con(&*o, p, r, filas::UMBRAL, Some(cancelada.clone()))
                .map(|l| l.filas)
                .map_err(P::fallo)
        },
    );
    match hecho {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: servir: {e}", P::NOMBRE);
            ExitCode::FAILURE
        }
    }
}

/// **El `main` de un driver de objetos.**
pub fn main<P: Proveedor>() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbo = args.first().map(String::as_str).unwrap_or("catalogo");
    // Los que no leen stdin antes de empezar.
    match verbo {
        "capacidades" => {
            println!("{}", filas::CAPACIDADES.json());
            return ExitCode::SUCCESS;
        }
        "servir" => return servir::<P>(),
        _ => {}
    }

    let mut entrada = String::new();
    if std::io::stdin().read_to_string(&mut entrada).is_err() {
        eprintln!("{}: no se pudo leer stdin", P::NOMBRE);
        return ExitCode::FAILURE;
    }

    let mut avisos: Vec<String> = Vec::new();
    let resultado = match verbo {
        "catalogo" => P::leer(entrada.trim()).and_then(|f| {
            let c = catalogo::leer(
                &*P::origen(&f),
                args.get(1).map(String::as_str).unwrap_or("fuente"),
                P::prefijo(&f),
                &mut avisos,
            )?;
            let (n, b) = P::contadores();
            avisos.push(format!(
                "catalogado con {n} peticiones y {} KB leídos: {} tablas y {} conjuntos de objetos",
                b / 1024,
                c.tablas.len(),
                c.objetos.len()
            ));
            Ok(ore_driver::catalogo::escribir(&c))
        }),
        "check" => ore_driver::leer_coordenada(&entrada)
            .and_then(|(url, _)| P::leer(&url))
            .map(|f| P::comprobar(&f)),
        "explorar" => ore_driver::leer_coordenada(&entrada)
            .and_then(|(url, _)| P::leer(&url))
            .and_then(|f| P::explorar(&f)),
        "testigo" => ore_driver::leer_coordenada(&entrada).and_then(|(url, objeto)| {
            let f = P::leer(&url)?;
            let objeto = if objeto.is_empty() {
                P::prefijo(&f).to_string()
            } else {
                objeto
            };
            catalogo::testigo(&*P::origen(&f), &objeto)
        }),
        "versiones" => serde_json::from_str::<serde_json::Value>(&entrada)
            .map_err(|e| format!("la petición no es JSON: {e}"))
            .and_then(|n| {
                let url = n.get("url").and_then(|u| u.as_str()).unwrap_or("");
                let f = P::leer(url)?;
                versiones::versiones(&*P::origen(&f), &entrada)
            }),
        // Los verbos del conector v2: el error, tipado y tapado.
        "leer" | "estimar" => {
            let url = ore_driver::leer_peticion(&entrada)
                .map(|p| p.url)
                .unwrap_or_default();
            let hecho = leer_o_estimar::<P>(verbo, &entrada, &mut avisos);
            for a in &avisos {
                eprintln!("{}: aviso · {a}", P::NOMBRE);
            }
            return match hecho {
                Ok(()) => ExitCode::SUCCESS,
                Err(f) => {
                    eprintln!("{}: {}", P::NOMBRE, f.tapado(&url).linea());
                    ExitCode::FAILURE
                }
            };
        }
        "bajar" => serde_json::from_str::<serde_json::Value>(&entrada)
            .map_err(|e| format!("la petición no es JSON: {e}"))
            .and_then(|n| {
                let f = P::leer(n.get("url").and_then(|u| u.as_str()).unwrap_or(""))?;
                let (pedidos, hilos) = bajar::pedidos(&entrada)?;
                let salida = std::sync::Mutex::new(std::io::BufWriter::with_capacity(
                    1 << 20,
                    std::io::stdout(),
                ));
                let c = bajar::en_paralelo(&*P::origen(&f), &pedidos, hilos, &salida);
                use std::io::Write as _;
                salida
                    .into_inner()
                    .map_err(|_| "la salida quedó envenenada".to_string())?
                    .flush()
                    .map_err(|e| format!("no se pudo escribir el flujo: {e}"))?;
                let (n, _) = P::contadores();
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
        eprintln!("{}: aviso · {a}", P::NOMBRE);
    }
    match resultado {
        Ok(salida) => {
            if !salida.is_empty() {
                println!("{salida}");
            }
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("{}: {m}", P::NOMBRE);
            ExitCode::FAILURE
        }
    }
}
