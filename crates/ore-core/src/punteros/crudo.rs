//! ⛔ **LA RUTA DE CADA PUNTERO** (0038 P2; 0079 §19.1): el catálogo guarda cada puntero por la ruta
//! que tuvo en el árbol, y aquí está cómo se forma y qué nombre dice. **No** es la forma de leer o
//! escribir un puntero: eso es [`super::Punteros`].
//!
//! Quien lo use sin razón no pasa la guarda `la_puerta_de_los_punteros` (ore-core/tests).

use std::path::{Path, PathBuf};

use super::partes;
use crate::normalize::SCHEMA_POR_DEFECTO;
use crate::parse::Node;

/// La carpeta de los punteros, bajo la raíz del árbol.
pub const CARPETA: &str = "datasets";

/// Las carpetas de los punteros: los de hoy y los de antes (`copias/`).
pub const CARPETAS_AL_DIA: [&str; 2] = [CARPETA, "copias"];

/// El puntero de `corto` en `dir`:
/// `<dir>/<base>/<schema>/<n>.json`.
pub fn ruta_en(dir: &Path, corto: &str) -> Option<PathBuf> {
    let (b, s, n) = partes(corto)?;
    Some(dir.join(b).join(s).join(format!("{n}.json")))
}

/// El puntero de antes (`<dir>/<p>_<n>.json`): sólo lo de `default` lo tuvo.
pub fn legado_en(dir: &Path, corto: &str) -> Option<PathBuf> {
    match partes(corto)? {
        (b, SCHEMA_POR_DEFECTO, n) => Some(dir.join(format!("{b}_{n}.json"))),
        _ => None,
    }
}

/// La ruta relativa a la raíz del árbol, con `/`: `datasets/ventas/default/pedidos.json`.
pub fn ruta(corto: &str) -> Option<String> {
    let (b, s, n) = partes(corto)?;
    Some(format!("{CARPETA}/{b}/{s}/{n}.json"))
}

/// **Qué nombre dice un fichero de punteros**, relativo a su carpeta:
/// `<b>/<s>/<n>.json` es `corto(b, s, n)`; uno de antes en la raíz de la
/// carpeta, el campo `nombre`, `tabla` o `vista` del puntero o, si no lo trae,
/// su nombre de fichero partido por la primera `_` (el paquete no lleva).
pub fn clave_de(rel: &Path, nodo: Option<&Node>) -> Option<String> {
    let segs: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let sin_json = |f: &str| f.strip_suffix(".json").map(String::from);
    match segs.as_slice() {
        [b, s, f] => Some(crate::normalize::corto(b, s, &sin_json(f)?)),
        [f] => {
            let del_campo = nodo.and_then(|n| {
                ["nombre", "tabla", "vista"].into_iter().find_map(|k| {
                    n.get(k)
                        .and_then(|(_, v)| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| crate::normalize::a_corto(s).into_owned())
                })
            });
            del_campo.or_else(|| {
                let stem = sin_json(f)?;
                Some(match stem.split_once('_') {
                    Some((p, x)) => format!("{p}.{x}"),
                    None => stem,
                })
            })
        }
        _ => None,
    }
}

/// Los ficheros `.json` de `dir`, a cualquier profundidad, ordenados.
pub fn ficheros(dir: &Path) -> Vec<PathBuf> {
    fn andar(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(es) = std::fs::read_dir(d) else {
            return;
        };
        for e in es.flatten() {
            let p = e.path();
            if p.is_dir() {
                andar(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    andar(dir, &mut out);
    out.sort();
    out
}
