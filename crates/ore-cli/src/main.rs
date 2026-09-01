//! La CLI de ORE.
//!
//! Tres caras bajo un solo binario, con fronteras de confianza distintas. La
//! columna que importa no es qué hace cada comando, sino **qué toca**: nueve de
//! catorce no abren un socket.

mod candado;
mod empaquetar;
mod fuente;
mod inductor;
mod inicio;
mod lector;
mod mcp;
mod revision;
mod vocabulario;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// La identidad del motor, y es mas que un numero de version.
///
/// Un bundle lleva `sha256:...` y **G1** promete que el mismo commit produce el
/// mismo digest. Quien audite ese bundle tiene que poder contestar *cual motor
/// lo produjo*, y `ore 0.1.0` contesta *alguna compilacion de la 0.1.0* — que
/// para una garantia de determinismo no vale.
///
/// El commit entra por **variable de entorno al compilar**, no por un `build.rs`
/// que invoque a git. La diferencia no es de comodidad: asi una compilacion
/// local dice honestamente que **no viene de un commit conocido**, en vez de
/// sellar el hash de un arbol que puede estar sucio. Un binario que miente sobre
/// su procedencia tiene exactamente el mismo aspecto que uno que no.
///
/// Y las versiones de OOS **se derivan** de `ApiVersion::ALL` (P2): una lista
/// escrita a mano aqui envejeceria en silencio la primera vez que el motor
/// aprendiera una version nueva.
fn version() -> String {
    let commit = option_env!("ORE_COMMIT").unwrap_or("sin sellar");
    let oos: Vec<&str> = ore_core::document::ApiVersion::ALL
        .iter()
        .map(|v| v.as_str())
        .collect();
    format!(
        "{} ({commit})
OOS: {}",
        env!("CARGO_PKG_VERSION"),
        oos.join(" · ")
    )
}

#[derive(Parser)]
#[command(
    name = "ore",
    about = "Ontology Runtime Engine — compila, coteja y sirve un repositorio ontológico",
    long_about = None,
    version = version()
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Lo que se puede hacer con una fuente. Hoy solo darla de alta; `list` y
/// `remove` esperan a tener más de una cosa que decir que la que ya dice el
/// manifiesto, que se lee.
#[derive(Subcommand)]
enum AccionFuente {
    /// Da de alta una fuente: el secreto va a `.env.local` y el manifiesto solo
    /// declara de qué variable sale.
    Add {
        /// Nombre con el que los bindings la referenciarán (`datasourceRef`).
        #[arg(long)]
        name: String,
        /// Cadena de conexión completa. **No** se escribe en ningún documento OOS.
        url: String,
        /// Driver. Por defecto se deriva del esquema de la URL.
        #[arg(long = "type", value_name = "DRIVER")]
        tipo: Option<String>,
        /// Variable de entorno. Por defecto, del manifiesto más el nombre.
        #[arg(long, value_name = "VAR")]
        env: Option<String>,
        /// Etiqueta que hereda todo lo enlazado a esta fuente. Repetible.
        #[arg(long = "label", value_name = "CLAVE=VALOR")]
        label: Vec<String>,
        /// Para qué es esta fuente.
        #[arg(long, value_name = "TEXTO")]
        description: Option<String>,
        /// Raíz del repositorio ontológico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum Command {
    // ── Scaffolder ──────── autoría · toca metadatos de producción y, si se pide, un LLM
    /// Crea el esqueleto de un repositorio ontológico.
    ///
    /// No escribe un retículo ni un `ConduitPolicy`: son decisiones de gobierno
    /// y este comando no las tiene. Omitir el conducto además YA significa algo
    /// —autorización ⊥, denegación por defecto—, así que escribirlo no lo haría
    /// más cierto.
    Init {
        /// Nombre del repositorio. Por defecto, el del directorio.
        #[arg(long)]
        name: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Registra una fuente física, separando la credencial de la conexión.
    #[command(name = "source", subcommand)]
    Source(AccionFuente),
    /// Introspecciona una fuente y propone entidades y bindings en DRAFT.
    ///
    /// Son dos actos: **leer** un catálogo y **proponer** una ontología, y se
    /// piden por separado porque fallan por separado. `--source` lee de una
    /// fuente declarada; `--from` toma un catálogo ya leído, venga de donde
    /// venga. Lo que produce el primero es exactamente lo que acepta el segundo.
    Discover {
        /// Un catálogo en JSON: columnas, tipos y claves de un origen.
        #[arg(long, conflicts_with = "source", required_unless_present = "source")]
        from: Option<PathBuf>,
        /// El nombre de una fuente declarada en `ontology.config.yaml`.
        #[arg(long)]
        source: Option<String>,
        /// Dónde se escribe el paquete inducido.
        #[arg(long)]
        out: PathBuf,
        /// Nombre y espacio de nombres del paquete. Por defecto, el del directorio.
        #[arg(long)]
        name: Option<String>,
    },
    /// Escribe el paquete publicable: un `.oob`.
    ///
    /// No es un archivo comprimido, y esa es la decisión: uno lleva marcas de
    /// tiempo y orden de entradas, así que el mismo paquete daría bytes
    /// distintos. Un `.oob` es **la forma canónica escrita en un fichero**, y su
    /// digest es el del paquete — el contenedor no cambia la identidad.
    Pack {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Dónde se escribe. Sin esto, el `.oob` sale por stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Resuelve `dependencies` y escribe `ontology.lock`.
    ///
    /// Contra **el árbol**, no contra un registro: `ore` no sabe hablar por la
    /// red. Un paquete se resuelve si está vendorizado como miembro del
    /// workspace, y si no, esto falla en vez de inventar una entrada.
    Lock {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Comprueba sin escribir. En CI hace falta saber que el lock quedó
        /// atrás **sin** tocar el árbol: uno que se arregla solo al mirarlo no
        /// se distingue de uno al día.
        #[arg(long)]
        check: bool,
    },
    /// Cola interactiva de decisiones para lo que el descubrimiento no supo clasificar.
    ///
    /// No edita lo inducido: **vuelve a inducir** el catálogo que `discover` dejó
    /// al lado, esta vez con las decisiones tomadas. Por eso es puro igual que el
    /// inductor —sin red, sin credenciales, sin driver— y por eso contestar dos
    /// veces lo mismo produce el mismo paquete byte a byte.
    Review {
        /// El paquete inducido: donde `discover` escribió.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Contesta en diferido, sin terminal. Es lo que permite probar esto en
        /// CI, y una cola que solo se contesta a mano no se prueba.
        #[arg(long, value_name = "FICHERO")]
        answers: Option<PathBuf>,
    },
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
    /// El registro de qué gobierna qué, y quién responde.
    ///
    /// No es una lista de incumplimientos y no puede serlo: una propiedad sin
    /// la clase que exige su clasificación no compila. La pregunta que contesta
    /// es la otra — **quién responde, y por qué vía**.
    Report {
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
    /// Emite a ODCS o a esquema Cedar (`cedar` en JSON, `cedarschema` nativo).
    ///
    /// `oos` y `json` dan la forma canónica, con y sin interpretar. Apache Ossie
    /// está declarado y no implementado: falla explicando por qué exige binding.
    Export {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        format: String,
    },

    // ── Runtime ─────────── producción · custodia credenciales vivas
    /// Sirve el contrato por MCP sobre stdio. Nivel L1: no toca un dato.
    ///
    /// La frontera con `serve` no es el nivel, es qué custodian: `dev` es un
    /// proceso hijo que muere con su cliente y no abre un puerto; `serve` es un
    /// servicio que sobrevive a sus clientes y por eso les debe autenticación
    /// (ADR 0005).
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
        Command::Report { path } => return informar(path),
        Command::Diff { before, after } => return diferir(before, after),
        Command::Compile { path } => return compilar(path),
        Command::Export { path, format } => return exportar(path, format),
        Command::Dev { path } => return desarrollo(path),
        Command::Init { name, path } => return inicio::init(path, name.as_deref()),
        Command::Discover {
            from,
            source,
            out,
            name,
        } => return descubrir(from.as_deref(), source.as_ref(), out, name.as_deref()),
        Command::Review { path, answers } => return revision::review(path, answers.as_deref()),
        Command::Lock { path, check } => return candado::lock(path, *check),
        Command::Pack { path, out } => return empaquetar::pack(path, out.as_deref()),
        Command::Source(AccionFuente::Add {
            name,
            url,
            tipo,
            env,
            label,
            description,
            path,
        }) => {
            return fuente::add(&fuente::Alta {
                raiz: path,
                nombre: name,
                url,
                tipo: tipo.as_deref(),
                env: env.as_deref(),
                etiquetas: label,
                descripcion: description.as_deref(),
            });
        }
        _ => {}
    }

    let (nombre, fase) = match cli.command {
        Command::Validate { .. }
        | Command::Diff { .. }
        | Command::Compile { .. }
        | Command::Export { .. }
        | Command::Dev { .. }
        | Command::Init { .. }
        | Command::Discover { .. }
        | Command::Report { .. }
        | Command::Review { .. }
        | Command::Lock { .. }
        | Command::Pack { .. }
        | Command::Source(_) => unreachable!(),
        Command::Lint { .. } => ("lint", "posterior"),
        Command::Test { .. } => ("test", "posterior"),
        Command::Plan { .. } => ("plan", "posterior"),
        Command::Promote { .. } => ("promote", "posterior"),
        Command::DriftDetect => ("drift-detect", "posterior"),
        Command::Serve { .. } => ("serve", "posterior"),
    };

    eprintln!("ore {nombre}: no implementado todavía (fase {fase})");
    eprintln!();
    eprintln!("  Hoy existen: {}.", implementados().join(", "));
    eprintln!("  Marcador:    cargo test -p ore-cli --test conformance -- --nocapture");

    std::process::ExitCode::from(70) // EX_SOFTWARE
}

/// Los comandos que hoy hacen algo.
///
/// **Se deriva de `clap`**, que es quien sabe qué hay. La lista anterior estaba
/// escrita a mano, era la tercera copia de la misma cosa y le faltaba `report` —
/// que es exactamente lo que le pasa a una cuenta escrita a mano en cuanto la
/// realidad avanza sin ella.
fn implementados() -> Vec<String> {
    use clap::CommandFactory as _;
    Cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .filter(|n| !SIN_IMPLEMENTAR.contains(&n.as_str()))
        .collect()
}

/// Lo que está declarado y todavía no hace nada. Es la lista corta, y es la que
/// encoge: un comando desaparece de aquí el día que existe.
const SIN_IMPLEMENTAR: [&str; 6] = ["lint", "test", "plan", "promote", "drift-detect", "serve"];

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
        // El acuse de recibo. Escribir Cedar es hoy un acto a ciegas: se declara
        // una politica y nada dice si alcanza lo que uno creia. Que una politica
        // no alcance nada NO es un error —`Property in [Label, EntityType]`
        // existe para que una entidad quede gobernada el dia que se etiqueta, sin
        // tocar la politica—, pero verlo es la diferencia entre saberlo y
        // suponerlo.
        let alcance = if path.is_dir() {
            ore_core::politica::alcance(&ore_core::validate::cargar_paquete(path).0)
        } else {
            Default::default()
        };
        if !alcance.is_empty() {
            println!();
            for (id, props) in &alcance {
                if props.is_empty() {
                    println!("  · {id} — no alcanza ninguna propiedad todavia");
                } else {
                    println!("  · {id} — {}", resumir(props));
                }
            }
        }
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

/// `ore discover` — el inductor.
///
/// Escribe lo que es un hecho y **reporta lo que es una conjetura**. Lo inducido
/// entra en `DRAFT` y probablemente no compile: una entidad sin clave falla con
/// `OOS2010`, y está bien que falle — inventar la clave sería lo único peor.
/// Las primeras propiedades y cuantas quedan. Una politica sobre `critical`
/// puede alcanzar doscientas, y doscientas lineas no informan: abruman.
fn resumir(props: &[String]) -> String {
    const MUESTRA: usize = 4;
    if props.len() <= MUESTRA {
        return props.join(", ");
    }
    format!(
        "{}, y {} mas",
        props[..MUESTRA].join(", "),
        props.len() - MUESTRA
    )
}

fn descubrir(
    origen: Option<&std::path::Path>,
    fuente: Option<&String>,
    destino: &std::path::Path,
    nombre: Option<&str>,
) -> std::process::ExitCode {
    // El catálogo se lee de un fichero o de una fuente viva, y a partir de aquí
    // el resto del comando no distingue cuál: es el mismo texto.
    let texto = match (origen, fuente) {
        (Some(o), _) => match std::fs::read_to_string(o) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: no se pudo leer `{}`: {e}", o.display());
                return std::process::ExitCode::from(66); // EX_NOINPUT
            }
        },
        (None, Some(f)) => match lector::catalogo(std::path::Path::new("."), f) {
            Ok(t) => t,
            Err(fallo) => {
                eprintln!("error: {}", fallo.mensaje);
                for l in &fallo.ayuda {
                    eprintln!("{l}");
                }
                return std::process::ExitCode::from(fallo.codigo);
            }
        },
        (None, None) => unreachable!("clap exige --from o --source"),
    };
    let catalogo = match inductor::Catalogo::leer(&texto) {
        Ok(c) => c,
        Err(m) => {
            eprintln!("error: {m}");
            return std::process::ExitCode::from(65); // EX_DATAERR
        }
    };

    let paquete = nombre.map(String::from).unwrap_or_else(|| {
        destino
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "inducido".into())
    });

    // El vocabulario que el repositorio ya publica. Sin esto la séptima pregunta
    // solo sabe ofrecer «acuña uno», que es la respuesta cara y la que produce
    // cuatro mil conceptos.
    let voc = match raiz_del_repositorio(destino) {
        Some(r) => vocabulario::Vocabulario::leer(&r),
        None => vocabulario::Vocabulario::default(),
    };
    let ind = inductor::inducir_con(&catalogo, &paquete, &inductor::Decisiones::default(), &voc);
    if let Err((codigo, mensaje)) = escribir_paquete(&ind, destino) {
        eprintln!("error: {mensaje}");
        return std::process::ExitCode::from(codigo);
    }

    // El catálogo, al lado de lo que produjo. No es un caché: es lo que hace que
    // `--source` sea reproducible como `--from`, y lo que permite que `review`
    // vuelva a inducir sin hablar con la fuente ni custodiar una credencial.
    let catalogo_json = destino.join("discover.catalog.json");
    if let Err(e) = std::fs::write(&catalogo_json, &texto) {
        eprintln!(
            "error: no se pudo escribir `{}`: {e}",
            catalogo_json.display()
        );
        return std::process::ExitCode::from(73);
    }
    let _ = std::fs::write(revision::ruta_cola(destino), revision::cola(&ind));

    print!("{}", inductor::informe(&ind, destino));
    for l in costura(destino, &catalogo) {
        eprintln!("{l}");
    }
    std::process::ExitCode::SUCCESS
}

/// Escribe los ficheros de una inducción bajo `destino`.
///
/// Lo comparten `discover` y `review` porque escriben lo mismo: el inductor dice
/// QUÉ, y quien llama dice DÓNDE. Dos copias de este bucle serían dos sitios
/// donde arreglar el mismo permiso denegado.
fn escribir_paquete(
    ind: &inductor::Induccion,
    destino: &std::path::Path,
) -> Result<(), (u8, String)> {
    for (rel, contenido) in &ind.ficheros {
        let ruta = destino.join(rel);
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d)
                .map_err(|e| (73, format!("no se pudo crear `{}`: {e}", d.display())))?;
        }
        std::fs::write(&ruta, contenido)
            .map_err(|e| (73, format!("no se pudo escribir `{}`: {e}", ruta.display())))?;
    }
    Ok(())
}

/// El directorio que manda sobre un paquete: el primero, subiendo, que tiene un
/// manifiesto.
///
/// Lo necesitan dos cosas por motivos distintos y es el mismo directorio: ahí
/// está la fuente declarada, y ahí están los conceptos publicados que `discover`
/// puede ofrecer. Buscarlo dos veces sería tener dos ideas de dónde empieza el
/// repositorio.
pub fn raiz_del_repositorio(desde: &std::path::Path) -> Option<std::path::PathBuf> {
    // `canonicalize` exige que la ruta exista, y `--out` normalmente **no existe
    // todavía**: se resuelve desde el ancestro más cercano que sí exista. Sin
    // esto, subir desde una ruta relativa acaba en el directorio vacío y el
    // repositorio parece no estar donde está.
    let absoluto = desde
        .ancestors()
        .find_map(|d| std::fs::canonicalize(d).ok())
        .or_else(|| std::env::current_dir().ok())?;
    absoluto
        .ancestors()
        .find(|d| d.join("ontology.config.yaml").is_file())
        .map(std::path::Path::to_path_buf)
}

/// El aviso de la costura: un binding referencia una fuente, y esa fuente la
/// declara **el manifiesto del repositorio**.
///
/// Salió midiendo, no leyendo. `discover --out <dir fuera de un repo>` escribe
/// bindings con `datasourceRef: crm_prod` y nada declara ese datasource, así que
/// `ore validate` responde `OOS2004` por cada uno. Es coherente —el inductor no
/// inventa un manifiesto— pero significa que **el camino de verdad es dentro de
/// un repositorio**, y eso no lo decía nadie: el comando terminaba en verde y el
/// error aparecía un paso después, lejos de su causa.
fn costura(destino: &std::path::Path, cat: &inductor::Catalogo) -> Vec<String> {
    let fuente = cat.fuente();
    let manifiesto = raiz_del_repositorio(destino).map(|r| r.join("ontology.config.yaml"));

    let Some(m) = manifiesto else {
        return vec![
            format!("aviso: nada declara la fuente `{fuente}`, y los bindings la referencian."),
            format!(
                "  `{}` no está dentro de un repositorio ontológico: no hay",
                destino.display()
            ),
            "  `ontology.config.yaml` en ningún directorio por encima, así que".to_string(),
            "  `ore validate` dirá OOS2004 una vez por binding.".to_string(),
            "  `ore init` crea uno, y `ore discover --out packages/<nombre>` induce dentro."
                .to_string(),
        ];
    };
    let declarada = std::fs::read_to_string(&m)
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok())
        .map(|a| {
            a.get("datasources")
                .map(|(_, v)| v.items())
                .unwrap_or(&[])
                .iter()
                .any(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(fuente))
        })
        .unwrap_or(false);
    if declarada {
        return Vec::new();
    }
    vec![
        format!("aviso: `{}` no declara la fuente `{fuente}`.", m.display()),
        "  Los bindings la referencian, así que `ore validate` dirá OOS2004 por cada uno."
            .to_string(),
        format!("  `ore source add --name {fuente} <url>` la declara sin escribir el secreto."),
    ]
}

/// `ore dev` — el servidor de contexto.
///
/// Compila el repositorio y sirve **el contrato** por MCP sobre stdio. No abre
/// un puerto, no lee una credencial y no toca un dato: es L1, y la mitad que el
/// criterio de la fase 3 daba por supuesta sin nombrarla.
fn desarrollo(path: &std::path::Path) -> std::process::ExitCode {
    let pkg = match cargar_valido(path, false) {
        Ok(p) => p,
        Err(c) => return c,
    };
    match mcp::servir(&pkg) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(motivo) => {
            eprintln!("error: {motivo}");
            std::process::ExitCode::from(70) // EX_SOFTWARE
        }
    }
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

/// Carga un paquete y lo rechaza si no valida. Compilar algo que no valida
/// produciría un digest de un artefacto que no existe.
fn cargar_valido(
    path: &std::path::Path,
    ignorar_generados: bool,
) -> Result<ore_core::link::Package, std::process::ExitCode> {
    if !path.is_dir() {
        eprintln!("error: `{}` no es un directorio de paquete", path.display());
        return Err(std::process::ExitCode::from(66)); // EX_NOINPUT
    }
    let diags: Vec<_> = ore_core::validate_package(path)
        .into_iter()
        .filter(|d| !(ignorar_generados && d.code == ore_core::Code::Oos2013))
        .collect();
    if let Some(d) = diags.first() {
        eprintln!("{}", d.render(path));
        return Err(std::process::ExitCode::from(65)); // EX_DATAERR
    }
    Ok(ore_core::validate::cargar_paquete(path).0)
}

/// `ore compile` — la forma canónica y los digests.
///
/// Puro por invariante III: sin red, sin credenciales, sin reloj, sin
/// aleatoriedad. Ejecutarlo dos veces sobre el mismo árbol de ficheros produce
/// byte a byte la misma salida, y eso es lo que `digest/deterministic-across-runs`
/// certifica.
fn compilar(path: &std::path::Path) -> std::process::ExitCode {
    let pkg = match cargar_valido(path, false) {
        Ok(p) => p,
        Err(c) => return c,
    };

    let canonica = ore_core::normalize::package(&pkg);
    let salida = ore_core::json::Json::obj([
        (
            "canonical",
            ore_core::json::Json::Obj(canonica.into_iter().collect()),
        ),
        (
            "digest",
            ore_core::json::Json::obj([
                (
                    "package",
                    ore_core::json::Json::s(ore_core::digest::package(&pkg)),
                ),
                (
                    "bundle",
                    ore_core::json::Json::s(ore_core::digest::bundle(&pkg)),
                ),
                (
                    "documents",
                    ore_core::json::Json::Obj(
                        ore_core::digest::documents(&pkg)
                            .into_iter()
                            .map(|(k, v)| (k, ore_core::json::Json::s(v)))
                            .collect(),
                    ),
                ),
            ]),
        ),
        (
            "oosVersion",
            ore_core::json::Json::s(ore_core::digest::OOS_VERSION),
        ),
    ]);
    println!("{}", salida.pretty());
    std::process::ExitCode::SUCCESS
}

/// `ore export` — traducción a un formato externo.
///
/// El argumento puede ser un **directorio de paquete OOS** o un **fichero de
/// otro formato**, y `--format` dice a qué se traduce. Que ambas direcciones
/// vivan en el mismo comando no es economía de subcomandos: es la afirmación de
/// que la traducción es reversible, y ahí es donde un perfil deja de ser una
/// limitación y pasa a ser interoperabilidad.
///
/// La ida y vuelta se compone desde fuera —`export a odcs`, luego `export ese
/// odcs a oos`— y se compara. ORE no se examina a sí mismo.
fn exportar(path: &std::path::Path, formato: &str) -> std::process::ExitCode {
    use ore_core::json::Json;

    // Un fichero suelto no es un paquete OOS: es un documento de otro formato
    // que entra. Se lee sin validarlo — §4.3 prohíbe interpretar lo ajeno.
    if path.is_file() {
        let texto = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: no se pudo leer `{}`: {e}", path.display());
                return std::process::ExitCode::from(66);
            }
        };
        let arbol = match ore_core::parse::parse(&texto) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("error: `{}` no analiza: {}", path.display(), e.message);
                return std::process::ExitCode::from(65);
            }
        };
        let entrada = ore_core::normalize::foreign(&arbol);
        let salida = match formato {
            // Sin traducir: el documento tal cual, en forma canónica. Es la
            // referencia contra la que se mide la fidelidad de la ida y vuelta.
            "json" => entrada,
            "oos" => Json::Obj(ore_core::odcs::import(&entrada).into_iter().collect()),
            "odcs" => ore_core::odcs::reemit(&entrada),
            otro => {
                eprintln!("error: `--format {otro}` no se puede producir desde un fichero suelto");
                return std::process::ExitCode::from(64); // EX_USAGE
            }
        };
        println!("{}", salida.pretty());
        return std::process::ExitCode::SUCCESS;
    }

    // `cedarschema` regenera el artefacto, así que no puede exigir que el
    // artefacto esté al día: sería pedirle a alguien que arregle un fichero
    // usando un comando que ese mismo fichero bloquea.
    let regenera = formato == "cedarschema";
    let pkg = match cargar_valido(path, regenera) {
        Ok(p) => p,
        Err(c) => return c,
    };

    let salida = match formato {
        "odcs" => ore_core::odcs::emit(&pkg),
        "cedar" => ore_core::cedar_schema::emit(&pkg),
        // La sintaxis nativa es la que se compromete al repositorio y la que
        // consume el tooling de Cedar; el JSON es la misma proyeccion en el
        // formato de esquema de Cedar. Emitir las dos desde el mismo sitio es
        // lo que impide que diverjan.
        "cedarschema" => {
            print!("{}", ore_core::cedar_schema::emit_text(&pkg));
            return std::process::ExitCode::SUCCESS;
        }
        "oos" => Json::Obj(ore_core::normalize::package(&pkg).into_iter().collect()),
        // La cuarta superficie. El SDL es texto, no JSON: sale por `stdout` sin
        // pasar por `Json`, igual que `cedarschema`.
        "graphql" => match ore_core::graphql::emit(&pkg) {
            Ok(sdl) => {
                print!("{sdl}");
                return std::process::ExitCode::SUCCESS;
            }
            Err(motivo) => {
                eprintln!("error: no se puede emitir a GraphQL: {motivo}");
                return std::process::ExitCode::from(65); // EX_DATAERR
            }
        },
        // Ossie no es anfitrión de `Entity`: un `Dataset` exige `source` y cada
        // `Field` exige `expression`, y ninguno de los dos está en la entidad —
        // están en el binding. Emitir sin él obligaría a INVENTAR los valores
        // obligatorios, y produciría un documento que valida contra el esquema
        // de Ossie y miente sobre dónde vive el dato.
        "ossie" => {
            let huerfanas: Vec<String> = ore_core::normalize::sin_binding(&pkg);
            if !huerfanas.is_empty() {
                eprintln!(
                    "error: no se puede emitir a Ossie: {} sin binding",
                    huerfanas.join(", ")
                );
                eprintln!();
                eprintln!("  Un `Dataset` de Ossie exige `source`; cada `Field`, `expression`.");
                eprintln!("  Ninguno de los dos está en la entidad: están en el binding.");
                eprintln!("  Emitir de todos modos exigiría inventarlos, y el documento");
                eprintln!("  resultante validaría contra Ossie y mentiría sobre dónde vive");
                eprintln!("  el dato. Por eso `Entity` es gramática propia y no perfil.");
                return std::process::ExitCode::from(65); // EX_DATAERR
            }
            eprintln!("ore export --format ossie: emisión no implementada todavía (fase 2)");
            return std::process::ExitCode::from(70);
        }
        otro => {
            eprintln!("error: formato `{otro}` desconocido");
            eprintln!("  formatos: odcs, cedar, cedarschema, graphql, ossie, oos, json");
            return std::process::ExitCode::from(64);
        }
    };
    println!("{}", salida.pretty());
    std::process::ExitCode::SUCCESS
}
/// `ore report` — **el registro de qué gobierna qué, y quién responde**.
///
/// # Lo que NO es, y por qué eso lo define
///
/// No es una lista de incumplimientos. **Aquí no puede haber filas rojas**:
/// una propiedad sin la clase de gobierno que exige su clasificación no
/// compila (`OOS8001`), así que un paquete que llega hasta aquí ya está
/// cubierto entero.
///
/// Eso lo separa del *compliance status report* de GitLab, que es fila por
/// (proyecto, control) con su estado, y existe porque **allí el gobierno se
/// evalúa cada doce horas sobre un objetivo que ya está desplegado**. Aquí se
/// evalúa al compilar, así que la pregunta interesante deja de ser *¿está
/// gobernado?* y pasa a ser:
///
/// > **¿Quién responde, y por qué vía?**
///
/// # Por qué no lista todas las propiedades
///
/// Porque la mayoría no exige nada. Medido sobre la ontología de referencia:
/// **40 propiedades clasificadas, 29 sin ninguna exigencia**. Un informe que
/// las listara sería el 72% de filas diciendo *«nada que gobernar»*, y el ruido
/// esconde exactamente lo que se viene a mirar.
///
/// Lo que exige gobierno lo decide `requiresGovernance`, **no** la
/// clasificación: una propiedad `low` está clasificada y no exige nada.
///
/// # Y lo ámbar no son filas
///
/// Una regla que **existe y no cuenta** —una aserción `severity: warning`, una
/// `type: text` que se transporta sin interpretar— no corresponde a ninguna
/// pareja (propiedad, clase): corresponde a una regla. Va al margen, y va,
/// porque *«lo vimos y no paramos nada»* tiene el mismo aspecto que no haberlo
/// visto.
fn informar(path: &std::path::Path) -> std::process::ExitCode {
    let pkg = match cargar_valido(path, true) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let lat = ore_core::flow::lattices(&pkg);
    let props = ore_core::flow::efectivas(&pkg, &lat);
    let cubierto = ore_core::governance::cobertura_atribuida(&pkg);
    let alcance = ore_core::politica::alcance(&pkg);

    let mut filas = 0usize;
    for (prop, etiquetas) in &props {
        // Lo que EXIGE cada clasificación que alcanza. Es la misma lectura que
        // hace `OOS8001`, en la otra dirección: allí para señalar lo que falta,
        // aquí para nombrar lo que hay.
        let mut exigidas: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for (ret, nivel) in etiquetas {
            let Some(l) = lat.get(ret) else { continue };
            for (piso, naturalezas) in &l.requires_governance {
                if l.ge(nivel, piso) == Some(true) {
                    exigidas.extend(naturalezas.iter().map(String::as_str));
                }
            }
        }
        if exigidas.is_empty() {
            continue;
        }
        if filas == 0 {
            println!("{:<34} {:<15} QUIÉN RESPONDE", "PROPIEDAD", "EXIGE");
        }
        filas += 1;
        for clase in &exigidas {
            let quienes = cubierto
                .get(prop)
                .and_then(|m| m.get(clase))
                .map(|v| {
                    v.iter()
                        .map(|d| match &d.owner {
                            Some(o) => format!("{} ({o})", d.regla),
                            // Solo pasa con VARIOS `ConduitPolicy`: no habría
                            // forma de saber de cuál hereda, y adivinar el dueño
                            // de una decisión de seguridad es peor que no
                            // tenerlo. La salida es `@oosOwner` en la política.
                            None => format!("{} (varios ConduitPolicy: sin herencia)", d.regla),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("{prop:<34} {clase:<15} {quienes}");
        }
    }

    if filas == 0 {
        println!("Ninguna propiedad de este paquete exige gobierno.");
        println!("No es un vacío: `requiresGovernance` es lo que exige, y ningún retículo");
        println!("declarado lo pide en los niveles que este modelo alcanza.");
        return std::process::ExitCode::SUCCESS;
    }

    // ── El margen ───────────────────────────────────────────────────────────
    let mut margen: Vec<String> = Vec::new();
    for (id, alcanzadas) in &alcance {
        if alcanzadas.is_empty() {
            margen.push(format!(
                "la política `{id}` no alcanza ninguna propiedad — no es un defecto: \
                 `Property in [Label, …]` existe para que una entidad quede gobernada el día \
                 que se etiqueta"
            ));
        }
    }
    for r in pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Ruleset)
    {
        let q = r.qname().unwrap_or_default();
        for a in r.section("assertions").map(|n| n.items()).unwrap_or(&[]) {
            let id = a.get("id").and_then(|(_, v)| v.as_str()).unwrap_or("?");
            let tipo = a
                .get("type")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("library");
            let sev = a
                .get("severity")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("error");
            if sev == "warning" {
                margen.push(format!(
                    "`{q}#{id}` es `severity: warning` y **no cuenta**: un aviso es, por \
                     definición, «lo vimos y no paramos nada»"
                ));
            } else if tipo == "text" || tipo == "custom" {
                margen.push(format!(
                    "`{q}#{id}` es `type: {tipo}` y **no cuenta**: se transporta sin \
                     interpretar, así que el compilador no sabe qué afirma"
                ));
            }
        }
    }
    if !margen.is_empty() {
        println!("\nAl margen — existen y no descargan nada:");
        for m in &margen {
            println!("  · {m}");
        }
    }

    println!(
        "\n{filas} propiedad(es) exigen gobierno, y las {filas} lo tienen: si alguna no lo \
         tuviera, esto no habría compilado."
    );
    std::process::ExitCode::SUCCESS
}
