//! La CLI de ORE.
//!
//! Tres caras bajo un solo binario, con fronteras de confianza distintas. La
//! columna que importa no es qué hace cada comando, sino **qué toca**: nueve de
//! catorce no abren un socket.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "ore",
    about = "Ontology Runtime Engine — compila, coteja y sirve un repositorio ontológico",
    long_about = None,
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    // ── Scaffolder ──────── autoría · toca metadatos de producción y, si se pide, un LLM
    /// Crea el esqueleto de un repositorio ontológico.
    Init,
    /// Registra una fuente física, separando la credencial de la conexión.
    #[command(name = "source")]
    Source,
    /// Introspecciona una fuente y propone entidades y bindings en DRAFT.
    Discover,
    /// Cola interactiva de decisiones para lo que el descubrimiento no supo clasificar.
    Review,
    /// Compara la declaración con el esquema físico real y abre un pull request.
    #[command(name = "drift-detect")]
    DriftDetect,

    // ── Compilador ──────── CI · hermético: sin red, sin credenciales, sin reloj
    /// Comprueba consistencia de reglas, tipados y políticas.
    Lint {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Valida contra OOS: esquema, integridad referencial y flujo de información.
    Validate {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Ejecuta los casos de prueba semánticos del paquete.
    Test {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Compara dos versiones y clasifica los cambios por eje.
    ///
    /// Es el único comando que toma DOS entradas: la clasificación de un cambio
    /// no es una propiedad de un paquete, es una relación entre dos.
    Diff { before: PathBuf, after: PathBuf },
    /// Muestra el delta semántico antes de aplicarlo.
    Plan {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Compila el repositorio a un Ontology Bundle firmado.
    Compile {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Eleva el estado de madurez de una entidad.
    Promote { entity: String },
    /// Emite a ODCS, Apache Ossie, OWL/RDF o esquema Cedar.
    Export {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        format: String,
    },

    // ── Runtime ─────────── producción · custodia credenciales vivas
    /// Sirve la ontología en local contra fuentes de desarrollo.
    Dev {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Sirve la ontología en producción.
    Serve {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Command::Validate { path } => return validar(path),
        Command::Diff { before, after } => return diferir(before, after),
        _ => {}
    }

    let (nombre, fase) = match cli.command {
        Command::Validate { .. } | Command::Diff { .. } => unreachable!(),
        Command::Compile { .. } => ("compile", "0"),
        Command::Init => ("init", "1"),
        Command::Source => ("source", "1"),
        Command::Discover => ("discover", "1"),
        Command::Review => ("review", "1"),
        Command::Dev { .. } => ("dev", "3"),
        Command::Lint { .. } => ("lint", "posterior"),
        Command::Test { .. } => ("test", "posterior"),
        Command::Plan { .. } => ("plan", "posterior"),
        Command::Promote { .. } => ("promote", "posterior"),
        Command::Export { .. } => ("export", "posterior"),
        Command::DriftDetect => ("drift-detect", "posterior"),
        Command::Serve { .. } => ("serve", "posterior"),
    };

    eprintln!("ore {nombre}: no implementado todavía (fase {fase})");
    eprintln!();
    eprintln!("  ORE arranca con 73 casos de conformidad en rojo, y ese es el plan.");
    eprintln!("  Estado actual:  cargo test -p ore-cli -- --nocapture");

    std::process::ExitCode::from(70) // EX_SOFTWARE
}

/// `ore validate` — nivel L0. Hermético: no abre un socket ni lee una credencial.
fn validar(path: &std::path::Path) -> std::process::ExitCode {
    if !path.exists() {
        eprintln!("error: no existe `{}`", path.display());
        return std::process::ExitCode::from(66); // EX_NOINPUT
    }

    let diags = if path.is_dir() {
        ore_core::validate_package(path)
    } else {
        match std::fs::read_to_string(path) {
            Ok(t) => ore_core::validate_document(path, &t),
            Err(e) => {
                eprintln!("error: no se pudo leer `{}`: {e}", path.display());
                return std::process::ExitCode::from(66);
            }
        }
    };

    if diags.is_empty() {
        println!("ok · sin errores");
        return std::process::ExitCode::SUCCESS;
    }

    let raiz = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    for d in &diags {
        eprintln!("{}", d.render(raiz));
        eprintln!();
    }
    let n = diags.len();
    eprintln!("{n} error{}", if n == 1 { "" } else { "es" });
    std::process::ExitCode::FAILURE
}

/// `ore diff` — la familia `OOS5xxx`.
///
/// Igual de hermético que `validate`: compara dos árboles de ficheros. Que el
/// carácter rompedor de un cambio **se compute** en lugar de afirmarse es
/// exactamente lo que hace que la versión sea una comprobación y no una
/// promesa.
///
/// El código de salida distingue las dos cosas que a un CI le importan por
/// separado: `0` compatible, `1` hay cambios rompedores.
fn diferir(antes: &std::path::Path, despues: &std::path::Path) -> std::process::ExitCode {
    for p in [antes, despues] {
        if !p.is_dir() {
            eprintln!("error: `{}` no es un directorio de paquete", p.display());
            return std::process::ExitCode::from(66); // EX_NOINPUT
        }
    }

    // Un paquete que no valida no se puede comparar: la diferencia entre dos
    // formas mal construidas no significa nada.
    for p in [antes, despues] {
        let (_, diags) = ore_core::validate::cargar_paquete(p);
        if let Some(d) = diags.first() {
            eprintln!("{}", d.render(p));
            eprintln!(
                "error: `{}` no analiza; no hay nada que comparar",
                p.display()
            );
            return std::process::ExitCode::from(65); // EX_DATAERR
        }
    }

    let (a, _) = ore_core::validate::cargar_paquete(antes);
    let (b, _) = ore_core::validate::cargar_paquete(despues);
    let informe = ore_core::diff::diff(&a, &b);
    println!("{}", informe.json().pretty());

    if informe.changes.is_empty() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
