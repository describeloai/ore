//! `ore validate` — el nivel **L0**.
//!
//! Completamente hermético: sin red, sin credenciales, sin reloj, sin
//! aleatoriedad. Nada de este módulo puede leer ninguna de esas cosas, y esa
//! restricción es la que hace verdad la frase que vende el producto: *el paso
//! que decide qué significan las cosas es el único que no puede filtrar nada.*
//!
//! # Precedencia
//!
//! `99-errors.md` §2.1 es normativo y aquí se materializa como el **orden** de
//! las comprobaciones: no se puede fallar la validación contra un esquema que
//! no se ha podido seleccionar.
//!
//! ```text
//! OOS1001  analizar          ─┐
//! OOS1002  apiVersion         │ despacho: elegir contra qué validar
//! OOS1003  kind              ─┘
//! OOS1005  claves            ─┐
//! OOS1004  forma             ─┘ validación
//! ```

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{self, Kind};
use crate::parse::{self, Node};
use std::path::{Path, PathBuf};

/// Valida un único documento. Se detiene en el primer fallo: continuar tras un
/// error de despacho produciría ruido, no información.
pub fn validate_document(file: &Path, text: &str) -> Vec<Diagnostic> {
    // ── OOS1001 · análisis ──────────────────────────────────────────────────
    let root = match parse::parse(text) {
        Ok(n) => n,
        Err(e) => {
            return vec![Diagnostic::new(Code::Oos1001, file, e.message).at(e.pos)];
        }
    };

    let Node::Mapping { .. } = root else {
        return vec![
            Diagnostic::new(Code::Oos1004, file, "el documento raíz debe ser un mapa")
                .at(root.pos()),
        ];
    };

    // ── OOS1002 · apiVersion ────────────────────────────────────────────────
    let Some((k, v)) = root.get("apiVersion") else {
        return vec![
            Diagnostic::new(Code::Oos1002, file, "falta `apiVersion`")
                .at(root.pos())
                .help(format!("añade `apiVersion: {}`", document::API_VERSION)),
        ];
    };
    match v.as_str() {
        Some(document::API_VERSION) => {}
        Some(other) => {
            return vec![
                Diagnostic::new(
                    Code::Oos1002,
                    file,
                    format!("`apiVersion: {other}` no está soportada"),
                )
                .at(k.pos())
                .help(format!(
                    "esta implementación entiende {}. Hay un esquema por apiVersion: \
                 sin resolver la versión no hay contra qué validar",
                    document::API_VERSION
                )),
            ];
        }
        None => {
            return vec![
                Diagnostic::new(Code::Oos1002, file, "`apiVersion` debe ser una cadena")
                    .at(v.pos()),
            ];
        }
    }

    // ── OOS1003 · kind ──────────────────────────────────────────────────────
    let Some((kk, kv)) = root.get("kind") else {
        return vec![Diagnostic::new(Code::Oos1003, file, "falta `kind`").at(root.pos())];
    };
    let Some(kind) = kv.as_str().and_then(Kind::parse) else {
        let nombre = kv.as_str().unwrap_or("<no es una cadena>");
        return vec![
            Diagnostic::new(
                Code::Oos1003,
                file,
                format!("`kind: {nombre}` no es un documento de v1alpha1"),
            )
            .at(kk.pos())
            .help(format!(
                "los documentos son: {}",
                Kind::ALL
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        ];
    };

    let mut diags = Vec::new();

    // ── OOS1005 · claves desconocidas ───────────────────────────────────────
    //
    // Se comprueba antes que la forma porque una errata es más probable que un
    // incumplimiento estructural, y decirlo con el nombre del campo ayuda más.
    // En `OntologyConfig` las secciones cuelgan de la raíz, así que el conjunto
    // admitido allí es la unión: comprobarlas por separado rechazaría
    // `datasources` antes de llegar a mirarla.
    let mut raiz_ok: Vec<&str> = kind.root_keys().to_vec();
    if kind.sections_at_root() {
        raiz_ok.extend_from_slice(kind.spec_keys());
    }
    check_keys(file, &root, &raiz_ok, "", &mut diags);

    if let Some((_, meta)) = root.get("metadata") {
        check_keys(file, meta, kind.metadata_keys(), "metadata.", &mut diags);
    }
    if !kind.sections_at_root()
        && let Some((_, spec)) = root.get("spec")
    {
        check_keys(file, spec, kind.spec_keys(), "spec.", &mut diags);
    }
    if !diags.is_empty() {
        return diags;
    }

    // ── OOS1004 · forma ─────────────────────────────────────────────────────
    for regla in document::shape_rules()
        .into_iter()
        .filter(|r| r.kind == kind)
    {
        let mut nodo = &root;
        let mut encontrado = true;
        for seg in regla.path {
            match nodo.get(seg) {
                Some((_, v)) => nodo = v,
                None => {
                    encontrado = false;
                    break;
                }
            }
        }
        if encontrado && let Some((msg, help)) = (regla.check)(nodo) {
            let mut d = Diagnostic::new(Code::Oos1004, file, msg).at(nodo.pos());
            if let Some(h) = help {
                d = d.help(h);
            }
            diags.push(d);
        }
    }

    diags
}

fn check_keys(
    file: &Path,
    map: &Node,
    permitidas: &[&str],
    prefijo: &str,
    out: &mut Vec<Diagnostic>,
) {
    for (k, _) in map.entries() {
        let Some(name) = k.as_str() else { continue };
        if permitidas.contains(&name) || document::is_extension(name) {
            continue;
        }
        // `apiVersion` y `kind` conviven con las secciones cuando estas cuelgan
        // de la raíz; no son desconocidas.
        if prefijo.is_empty() && matches!(name, "apiVersion" | "kind" | "metadata" | "spec") {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos1005,
                file,
                format!("clave desconocida `{prefijo}{name}`"),
            )
            .at(k.pos())
            .help(format!(
                "si es una extensión de proveedor, declárala como `x-<proveedor>-{name}`; \
                 si no, revisa si es una errata"
            )),
        );
    }
}

/// Valida un paquete entero.
///
/// Dos fases, y el orden es normativo: **no se puede enlazar un paquete que no
/// analiza**. Si la fase de documento encuentra algo, la de enlazado no llega a
/// correr — resolver referencias sobre un árbol que no se pudo construir
/// produciría cascadas de errores derivados en lugar de la causa.
pub fn validate_package(root: &Path) -> Vec<Diagnostic> {
    let mut ficheros = Vec::new();
    recolectar(root, &mut ficheros);
    ficheros.sort();

    let mut diags = Vec::new();
    let mut cargados = Vec::new();
    let mut cedar = Vec::new();

    for f in ficheros {
        if f.extension().is_some_and(|x| x == "cedar") {
            if let Ok(t) = std::fs::read_to_string(&f) {
                cedar.push((f.clone(), t));
            }
            continue;
        }

        let text = match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => {
                diags.push(Diagnostic::new(
                    Code::Oos1001,
                    &f,
                    format!("no se pudo leer: {e}"),
                ));
                continue;
            }
        };
        // `ontology.lock` no es un documento de la ontología: es un artefacto
        // generado, versionado por su propio formato y sin `apiVersion` ni
        // `kind` a propósito. No pasa por la fase de documento, pero sí entra en
        // la de enlazado — el grafo de dependencias vive ahí.
        if f.file_name().is_some_and(|n| n == "ontology.lock") {
            if let Some(l) = cargar(&f, &text) {
                cargados.push(l);
            }
            continue;
        }

        let d = validate_document(&f, &text);
        if d.is_empty()
            && let Some(l) = cargar(&f, &text)
        {
            cargados.push(l);
        }
        diags.extend(d);
    }

    if !diags.is_empty() {
        return diags;
    }
    let pkg = crate::link::Package {
        root: root.to_path_buf(),
        docs: cargados,
        cedar,
    };

    // Enlazado antes que tipos: no se puede comprobar el tipo de una referencia
    // que no resuelve. Es la misma disciplina de fases que impide enlazar un
    // paquete que no analiza.
    let refs = crate::link::link(&pkg);
    if !refs.is_empty() {
        return refs;
    }
    // Tipos antes que flujo: la propagación transporta etiquetas por el mismo
    // grafo de derivación que los tipos acaban de validar.
    let tipos = crate::types::check(&pkg);
    if !tipos.is_empty() {
        return tipos;
    }
    crate::flow::check(&pkg)
}

/// Reanaliza un documento ya validado para la fase de enlazado. `ontology.lock`
/// no lleva `kind` —es un artefacto generado, no un documento de la ontología—
/// y entra como `OntologyConfig` para que el grafo de dependencias sea legible.
fn cargar(path: &Path, text: &str) -> Option<crate::link::Loaded> {
    let root = parse::parse(text).ok()?;
    let kind = match root
        .get("kind")
        .and_then(|(_, v)| v.as_str())
        .and_then(Kind::parse)
    {
        Some(k) => k,
        None if path.file_name().is_some_and(|n| n == "ontology.lock") => Kind::OntologyConfig,
        None => return None,
    };
    Some(crate::link::Loaded {
        path: path.to_path_buf(),
        kind,
        root,
    })
}

fn recolectar(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entradas.flatten() {
        let p = e.path();
        if p.is_dir() {
            // `.ore/` es caché derivada y no forma parte del paquete.
            if p.file_name().is_some_and(|n| n == ".ore") {
                continue;
            }
            recolectar(&p, out);
        } else if p.extension().is_some_and(|x| x == "yaml" || x == "yml")
            || p.extension().is_some_and(|x| x == "cedar")
            || p.file_name().is_some_and(|n| n == "ontology.lock")
        {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codigos(text: &str) -> Vec<Code> {
        validate_document(Path::new("t.yaml"), text)
            .iter()
            .map(|d| d.code)
            .collect()
    }

    const ENTIDAD_OK: &str = "apiVersion: oos.dev/v1alpha1\nkind: Entity\n\
        metadata: { name: Employee, namespace: hr }\n\
        spec:\n  nature: entity\n  primaryKey: [id]\n  properties:\n    id: { type: String }\n";

    #[test]
    fn un_documento_correcto_no_produce_diagnosticos() {
        assert!(codigos(ENTIDAD_OK).is_empty());
    }

    #[test]
    fn el_despacho_precede_a_la_validacion() {
        // kind desconocido Y clave desconocida: debe ganar el despacho, porque
        // sin kind no hay contra qué validar las claves.
        let t = "apiVersion: oos.dev/v1alpha1\nkind: Ontology\nzzz: 1\n";
        assert_eq!(codigos(t), vec![Code::Oos1003]);
    }

    #[test]
    fn extension_de_proveedor_admitida() {
        let t = ENTIDAD_OK.replace(
            "  nature: entity",
            "  x-acme-owner: plataforma\n  nature: entity",
        );
        assert!(codigos(&t).is_empty(), "{:?}", codigos(&t));
    }
}
