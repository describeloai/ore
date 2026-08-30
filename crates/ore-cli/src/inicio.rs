//! `ore init` — el esqueleto de un repositorio ontológico.
//!
//! # Lo que NO crea, y es la mitad del diseño
//!
//! No escribe un retículo ni un `ConduitPolicy`. Los dos son **decisiones de
//! gobierno**, y este comando no las tiene: inventarlas sería exactamente lo que
//! `01-package` §5 prohíbe —*la decisión pendiente se marca; **NO DEBE**
//! inventarse*— con el agravante de que aquí saldrían con aspecto de valor por
//! defecto sensato.
//!
//! Y con el `ConduitPolicy` hay una razón más fina: **omitirlo ya significa
//! algo.** `conduit-policy.schema.json` lo dice —*«un conducto NO listado tiene
//! autorización ⊥ y no admite nada: denegación por defecto (P4). Omitir un
//! conducto no es dejarlo abierto, es cerrarlo»*—. Un repositorio recién creado
//! **no sirve nada por ningún conducto**, que es la postura correcta, y
//! escribirla no la haría más cierta.
//!
//! # Lo que sí deriva
//!
//! El nombre, del directorio. Es la única cosa que un `init` puede saber sin
//! preguntar, y `--name` la corrige. La versión arranca en `0.1.0` porque
//! `91-versioning` reserva `0.x` para lo que aún no promete nada.

use std::path::Path;
use std::process::ExitCode;

const CONFIG: &str = "ontology.config.yaml";
const IGNORAR: &str = ".gitignore";
const PAQUETES: &str = "packages";

pub fn init(raiz: &Path, nombre: Option<&str>) -> ExitCode {
    match intentar(raiz, nombre) {
        Ok(informe) => {
            print!("{informe}");
            ExitCode::SUCCESS
        }
        Err((codigo, lineas)) => {
            for (i, l) in lineas.iter().enumerate() {
                if i == 0 {
                    eprintln!("error: {l}");
                } else {
                    eprintln!("{l}");
                }
            }
            ExitCode::from(codigo)
        }
    }
}

fn intentar(raiz: &Path, nombre: Option<&str>) -> Result<String, (u8, Vec<String>)> {
    let config = raiz.join(CONFIG);
    if config.exists() {
        return Err((
            73, // EX_CANTCREAT
            vec![
                format!("ya existe `{}`", config.display()),
                "  Este directorio ya es un repositorio ontológico. `init` no lo toca:".into(),
                "  sobrescribir un manifiesto perdería lo que declara.".into(),
            ],
        ));
    }

    let nombre = match nombre {
        Some(n) => n.to_string(),
        None => derivar_nombre(raiz).ok_or_else(|| {
            (
                65, // EX_DATAERR
                vec![
                    "no se puede derivar un nombre de paquete del directorio".into(),
                    "  Un nombre es minúscula, luego minúsculas, dígitos o `-`.".into(),
                    "  Dilo con `--name`.".into(),
                ],
            )
        })?,
    };
    if !nombre_valido(&nombre) {
        return Err((
            65,
            vec![
                format!("`{nombre}` no sirve como nombre de paquete"),
                "  Minúscula, luego minúsculas, dígitos o `-`. Sin puntos ni mayúsculas.".into(),
            ],
        ));
    }

    let escribir = |ruta: &Path, texto: &str| -> Result<(), (u8, Vec<String>)> {
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d)
                .map_err(|e| (73, vec![format!("no se pudo crear `{}`: {e}", d.display())]))?;
        }
        std::fs::write(ruta, texto).map_err(|e| {
            (
                73,
                vec![format!("no se pudo escribir `{}`: {e}", ruta.display())],
            )
        })
    };

    escribir(&config, &manifiesto(&nombre))?;

    // El directorio de paquetes existe aunque esté vacío: `workspace.members`
    // vale `packages/*` por convención y un directorio ausente convierte esa
    // convención en una sorpresa.
    std::fs::create_dir_all(raiz.join(PAQUETES))
        .map_err(|e| (73, vec![format!("no se pudo crear `{PAQUETES}/`: {e}")]))?;
    escribir(&raiz.join(PAQUETES).join(".gitkeep"), "")?;

    let ignorar = raiz.join(IGNORAR);
    let previo = std::fs::read_to_string(&ignorar).unwrap_or_default();
    if !previo.lines().any(|l| l.trim() == ".env.local") {
        let mut s = previo;
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(".env.local\n");
        escribir(&ignorar, &s)?;
    }

    Ok(informe(&nombre))
}

fn manifiesto(nombre: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha1\n\
         kind: OntologyConfig\n\
         metadata: {{ name: {nombre}, version: 0.1.0 }}\n\
         \n\
         # `workspace.members` vale `packages/*` por convención, así que no se\n\
         # declara: lo derivable no se declara (P2). Solo hace falta escribirlo\n\
         # con una disposición no estándar.\n\
         #\n\
         # `datasources` lo escribe `ore source add`, que separa el secreto de la\n\
         # conexión para que este fichero siga siendo publicable.\n"
    )
}

fn informe(nombre: &str) -> String {
    format!(
        "  ✓ {CONFIG} · {nombre} 0.1.0\n\
         \x20 ✓ {PAQUETES}/ · donde el compilador busca los paquetes\n\
         \x20 ✓ {IGNORAR} · .env.local\n\
         \n\
         \x20 Tres decisiones te esperan, y **ninguna se puede inventar**:\n\
         \n\
         \x20   un retículo      sin una escala de clasificación no hay nada que gobernar\n\
         \x20   un ConduitPolicy omitirlo NO lo deja abierto: lo cierra (P4). Hoy este\n\
         \x20                    repositorio no sirve nada por ningún conducto, que es\n\
         \x20                    la postura correcta para uno recién creado\n\
         \x20   el primer paquete   ore source add --name <fuente> <url>\n\
         \n\
         \x20 ore validate .\n"
    )
}

/// El nombre de un paquete es `^[a-z][a-z0-9-]*$`. Del directorio se toma lo que
/// encaje: `Pedidos 2024` da `pedidos-2024`.
fn derivar_nombre(raiz: &Path) -> Option<String> {
    let bruto = raiz
        .canonicalize()
        .ok()
        .as_deref()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or_else(|| raiz.file_name().map(|n| n.to_string_lossy().into_owned()))?;
    let mut out = String::new();
    let mut guion = false;
    for c in bruto.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            guion = false;
        } else if !out.is_empty() && !guion {
            out.push('-');
            guion = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    // Un nombre que empieza por dígito no es un nombre de paquete, y anteponerle
    // una letra sería inventárselo.
    out.starts_with(|c: char| c.is_ascii_lowercase())
        .then_some(out)
}

fn nombre_valido(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn en(nombre: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ore-init-{nombre}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn el_nombre_se_deriva_del_directorio() {
        assert_eq!(
            derivar_nombre(Path::new("/tmp/pedidos")).as_deref(),
            Some("pedidos")
        );
        assert_eq!(
            derivar_nombre(Path::new("/tmp/Pedidos 2024")).as_deref(),
            Some("pedidos-2024")
        );
        // Empezar por dígito no se arregla inventando una letra delante.
        assert_eq!(derivar_nombre(Path::new("/tmp/2024")), None);
    }

    #[test]
    fn lo_que_escribe_valida() {
        let d = en("valida");
        assert!(intentar(&d, Some("pedidos")).is_ok());
        let texto = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        let diags = ore_core::validate_document(&d.join(CONFIG), &texto);
        assert!(
            diags.is_empty(),
            "el manifiesto recién creado no valida: {diags:?}"
        );
    }

    /// Y lo que NO escribe importa igual: un retículo o un `ConduitPolicy`
    /// inventados saldrían con aspecto de valor por defecto sensato, que es la
    /// peor forma de inventar una decisión de gobierno.
    #[test]
    fn no_inventa_gobierno() {
        let d = en("gobierno");
        intentar(&d, Some("pedidos")).unwrap();
        let ficheros: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !ficheros.iter().any(|f| f.contains("lattice")),
            "{ficheros:?}"
        );
        assert!(
            !ficheros.iter().any(|f| f.contains("conduit")),
            "{ficheros:?}"
        );
    }

    #[test]
    fn no_sobrescribe_un_repositorio_existente() {
        let d = en("existente");
        intentar(&d, Some("pedidos")).unwrap();
        let err = intentar(&d, Some("otro")).unwrap_err();
        assert_eq!(err.0, 73);
        // Y el manifiesto sigue diciendo lo que decía.
        let texto = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        assert!(texto.contains("name: pedidos"), "{texto}");
    }
}
