//! v1alpha26 — **la función propia** (ORE 0056 V2, OOS v1alpha26 `01`).
//!
//! Una `Function` de v1alpha26 no es de ningún paquete: vive en
//! `functions/<nombre>.yaml` de la raíz, se llama `functions.<nombre>`, lleva
//! su versión (`metadata.version`, que calcula quien deriva el documento) y la
//! huella de lo que corre (`spec.codeDigest`). Aquí, lo que la gramática pide
//! de ella antes del enlazado:
//!
//! - **la forma** (`OOS1004`): `metadata.version` en semver, `spec.owner` y
//!   `spec.codeDigest` (`sha256:` y 64 hexadecimales). `namespace` y `schema`
//!   no existen en esta versión: los rechaza el control de claves (`OOS1005`);
//! - **el sitio** (`OOS2036`): `functions/<nombre>.yaml` de la raíz, y en
//!   ningún otro;
//! - **el nombre** (`OOS2035`): único en el espacio de trabajo **sin
//!   distinguir mayúsculas**, porque en SQL `Total` y `total` son lo mismo;
//! - **el espacio** (`OOS2048`): ningún paquete se llama `functions`.
//!
//! La huella se compara con el código en [`crate::promover`], junto a la firma:
//! las dos son lo que el código da.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{ApiVersion, Kind};
use crate::link::{Loaded, Package};
use crate::parse::Node;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// El espacio de las funciones propias: `functions.<nombre>`.
pub const ESPACIO: &str = "functions";

/// La versión con que nace una función (v1alpha26 `01` §5).
pub const VERSION_INICIAL: &str = "0.1.0";

/// Si el documento es una función propia: `Function` de v1alpha26 o posterior.
pub fn es_propia(d: &Loaded) -> bool {
    d.kind == Kind::Function && d.version().is_some_and(|v| v >= ApiVersion::V1Alpha26)
}

/// Dónde vive el documento de una función propia, desde la raíz.
pub fn ruta_del_documento(nombre: &str) -> String {
    format!("{ESPACIO}/{nombre}.yaml")
}

/// `x.y.z` con tres enteros sin ceros a la izquierda.
pub fn es_semver(v: &str) -> bool {
    let partes: Vec<&str> = v.split('.').collect();
    partes.len() == 3
        && partes.iter().all(|p| {
            !p.is_empty()
                && p.chars().all(|c| c.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}

fn es_huella(v: &str) -> bool {
    v.strip_prefix("sha256:")
        .is_some_and(|h| h.len() == 64 && h.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')))
}

/// Los manifiestos de entorno y sus bloqueos, por runtime (v1alpha26 `01` §6).
fn entorno(runtime: &str) -> (&'static str, &'static [&'static str]) {
    if runtime == "node" {
        ("package.json", &["package-lock.json"])
    } else {
        ("pyproject.toml", &["pylock.toml", "uv.lock", "poetry.lock"])
    }
}

/// **`codeDigest`** (v1alpha26 `01` §6): `sha256:` del fichero del
/// `entrypoint`, del manifiesto de entorno más cercano subiendo desde él y de
/// su bloqueo al lado, cada uno precedido por su ruta desde la raíz y un `\n`,
/// con los finales de línea en `\n` (un checkout de Windows no es otro código).
/// `None` si el fichero no se puede leer.
pub fn huella(raiz: &Path, fichero: &str, runtime: &str) -> Option<String> {
    let leer = |rel: &str| -> Option<String> {
        std::fs::read_to_string(raiz.join(rel))
            .ok()
            .map(|t| t.replace("\r\n", "\n"))
    };
    let mut h = Sha256::new();
    let mut sumar = |rel: &str, texto: &str| {
        h.update(rel.as_bytes());
        h.update(b"\n");
        h.update(texto.as_bytes());
    };
    sumar(fichero, &leer(fichero)?);
    let (manifiesto, bloqueos) = entorno(runtime);
    // Subiendo desde la carpeta del fichero hasta la raíz, incluida.
    let mut carpeta = fichero.rsplit_once('/').map(|(c, _)| c.to_string());
    loop {
        let en = |n: &str| match &carpeta {
            Some(c) => format!("{c}/{n}"),
            None => n.to_string(),
        };
        if let Some(t) = leer(&en(manifiesto)) {
            sumar(&en(manifiesto), &t);
            if let Some((b, t)) = bloqueos.iter().find_map(|b| Some((en(b), leer(&en(b))?))) {
                sumar(&b, &t);
            }
            break;
        }
        match carpeta.take() {
            Some(c) => carpeta = c.rsplit_once('/').map(|(p, _)| p.to_string()),
            None => break,
        }
    }
    let dig: [u8; 32] = h.finalize().into();
    Some(format!(
        "sha256:{}",
        dig.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// La ruta de un fichero desde la raíz, con `/`.
pub fn relativa(raiz: &Path, p: &Path) -> String {
    p.strip_prefix(raiz)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    // ── OOS2048 · el espacio de las funciones no es un paquete ──────────────
    for p in pkg.of(Kind::Package) {
        if p.meta("name").and_then(Node::as_str) == Some(ESPACIO) {
            out.push(
                Diagnostic::new(
                    Code::Oos2048,
                    &p.path,
                    format!("un paquete no puede llamarse `{ESPACIO}`"),
                )
                .at(p
                    .meta("name")
                    .map(Node::pos)
                    .unwrap_or_else(|| p.root.pos()))
                .help(
                    "`functions` es el espacio de las funciones propias (v1alpha26): con un \
                     paquete así, `functions.x` sería a la vez una función y un dato. Dale otro \
                     nombre al paquete",
                ),
            );
        }
    }

    let mut nombres: BTreeMap<String, Vec<&Loaded>> = BTreeMap::new();
    for f in pkg.of(Kind::Function).filter(|f| es_propia(f)) {
        let nombre = f.meta("name").and_then(Node::as_str).unwrap_or_default();
        nombres.entry(nombre.to_lowercase()).or_default().push(f);

        // ── OOS1004 · lo que una función propia lleva ───────────────────────
        let falta = |que: &str, ayuda: &str| {
            Diagnostic::new(
                Code::Oos1004,
                &f.path,
                format!("la función propia `{nombre}` no dice `{que}`"),
            )
            .at(f.root.pos())
            .help(ayuda.to_string())
        };
        match f.meta("version").and_then(Node::as_str) {
            None => out.push(falta(
                "metadata.version",
                "la escribe quien deriva el documento (`ore functions generate`, o el commit): \
                 sin versión no hay con qué comparar el siguiente cambio",
            )),
            Some(v) if !es_semver(v) => out.push(
                Diagnostic::new(
                    Code::Oos1004,
                    &f.path,
                    format!("`version: {v}` no es `<mayor>.<menor>.<parche>`"),
                )
                .at(f
                    .meta("version")
                    .map(Node::pos)
                    .unwrap_or_else(|| f.root.pos()))
                .help("tres enteros sin ceros a la izquierda, como `0.1.0`"),
            ),
            Some(_) => {}
        }
        if f.section("owner").is_none() {
            out.push(falta(
                "spec.owner",
                "fuera de un paquete no hay de quién heredarlo: `user:<handle>` o \
                 `team:<handle>`, quien la crea",
            ));
        }
        match f.section("codeDigest").and_then(Node::as_str) {
            None if f.section("runtime").and_then(Node::as_str) != Some("wasm")
                && f.section("runtime").and_then(Node::as_str) != Some("model") =>
            {
                out.push(falta(
                    "spec.codeDigest",
                    "la huella de lo que corre: la escribe quien deriva el documento",
                ))
            }
            Some(v) if !es_huella(v) => out.push(
                Diagnostic::new(
                    Code::Oos1004,
                    &f.path,
                    format!("`codeDigest: {v}` no es `sha256:` y 64 hexadecimales"),
                )
                .at(f
                    .section("codeDigest")
                    .map(Node::pos)
                    .unwrap_or_else(|| f.root.pos())),
            ),
            _ => {}
        }

        // ── OOS2036 · su sitio es `functions/` de la raíz ───────────────────
        let suyo = pkg.root.join(ESPACIO).join(format!("{nombre}.yaml"));
        if f.path != suyo {
            out.push(
                Diagnostic::new(
                    Code::Oos2036,
                    &f.path,
                    format!(
                        "la función propia `{nombre}` vive en `{}`, y no aquí",
                        ruta_del_documento(nombre)
                    ),
                )
                .at(f
                    .meta("name")
                    .map(Node::pos)
                    .unwrap_or_else(|| f.root.pos()))
                .help(
                    "una función de v1alpha26 no es de ningún paquete: su documento está en \
                     `functions/` de la raíz y se llama como ella. Dentro de un paquete \
                     parecería suya",
                ),
            );
        }
    }

    // ── OOS2035 · un nombre, una función, sin mirar mayúsculas ──────────────
    for (_, fs) in nombres {
        if fs.len() < 2 {
            continue;
        }
        let donde: Vec<String> = fs.iter().map(|f| relativa(&pkg.root, &f.path)).collect();
        for f in &fs[1..] {
            out.push(
                Diagnostic::new(
                    Code::Oos2035,
                    &f.path,
                    format!(
                        "`{ESPACIO}.{}` está declarada {} veces: {}",
                        f.meta("name").and_then(Node::as_str).unwrap_or_default(),
                        fs.len(),
                        donde.join(" · ")
                    ),
                )
                .at(f
                    .meta("name")
                    .map(Node::pos)
                    .unwrap_or_else(|| f.root.pos()))
                .help(
                    "el nombre de una función es único en el espacio de trabajo, y en SQL las \
                     mayúsculas no distinguen: renombra uno de los dos `def`",
                ),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver() {
        assert!(es_semver("0.1.0"));
        assert!(es_semver("12.0.3"));
        assert!(!es_semver("01.0.0"));
        assert!(!es_semver("1.0"));
        assert!(!es_semver("1.0.0-rc1"));
    }

    #[test]
    fn la_huella_es_la_de_la_spec() {
        // El caso `a-function-of-its-own` de la suite, calculado a mano con
        // la regla de §6.
        let r = &std::env::temp_dir().join(format!("ore-huella-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(r);
        let py = "packages/ventas/facturacion/funciones/total.py";
        let mani = "packages/ventas/facturacion/pyproject.toml";
        std::fs::create_dir_all(r.join("packages/ventas/facturacion/funciones")).unwrap();
        std::fs::write(r.join(py), "def total(): pass\n").unwrap();
        std::fs::write(r.join(mani), "[project]\n").unwrap();
        let mut h = Sha256::new();
        for (ruta, t) in [(py, "def total(): pass\n"), (mani, "[project]\n")] {
            h.update(ruta.as_bytes());
            h.update(b"\n");
            h.update(t.as_bytes());
        }
        let esperado: [u8; 32] = h.finalize().into();
        let esperado = format!(
            "sha256:{}",
            esperado
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(huella(r, py, "python").as_deref(), Some(esperado.as_str()));
        // Un CRLF no es otro código.
        std::fs::write(r.join(py), "def total(): pass\r\n").unwrap();
        assert_eq!(huella(r, py, "python").as_deref(), Some(esperado.as_str()));
        // Otro cuerpo, otra huella.
        std::fs::write(r.join(py), "def total(): return 1\n").unwrap();
        assert_ne!(huella(r, py, "python").as_deref(), Some(esperado.as_str()));
        let _ = std::fs::remove_dir_all(r);
    }
}
