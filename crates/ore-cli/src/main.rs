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

    let (nombre, fase) = match cli.command {
        Command::Validate { .. } => ("validate", "0"),
        Command::Compile { .. } => ("compile", "0"),
        Command::Init => ("init", "1"),
        Command::Source => ("source", "1"),
        Command::Discover => ("discover", "1"),
        Command::Review => ("review", "1"),
        Command::Dev { .. } => ("dev", "3"),
        Command::Lint { .. } => ("lint", "posterior"),
        Command::Test { .. } => ("test", "posterior"),
        Command::Diff { .. } => ("diff", "posterior"),
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
