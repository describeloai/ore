//! ⛔ **LOS PUNTEROS COMO FICHEROS DEL ÁRBOL** (0038 P2; 0079 §19.1): dónde
//! vive cada uno en un directorio, cómo se lee el de antes y qué nombre dice
//! cada fichero. Es el detalle de [`super::Ficheros`] y del mundo de git (la
//! superposición de una rama, las migraciones), **no** la forma de leer o
//! escribir un puntero: eso es [`super::Punteros`].
//!
//! Quien lo use fuera de esa lista no pasa la guarda
//! `la_puerta_de_los_punteros` (ore-core/tests): cada sitio que sigue aquí
//! lleva su razón en ella, y el corte (E1a·5) los retira.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::partes;
use crate::json::Json;
use crate::normalize::SCHEMA_POR_DEFECTO;
use crate::parse::Node;

/// La carpeta de los punteros, bajo la raíz del árbol.
pub const CARPETA: &str = "datasets";

/// El fichero, en la raíz de un árbol de rama superpuesto, que dice qué
/// punteros se leen de `main` al día (0044 C.2 ③). No se confirma nunca. El
/// mismo nombre en `ore-serve` (`git::AL_DIA`) y en `ore overlay`.
pub const AL_DIA: &str = ".ore-al-dia.json";

/// Las carpetas con punteros que una rama lee de `main` al día (0044 C.2 ③):
/// los de hoy y los de antes (`copias/`).
pub const CARPETAS_AL_DIA: [&str; 2] = [CARPETA, "copias"];

/// El puntero de `corto` en `dir` (la carpeta de punteros o un `--informe`):
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

/// El puntero de `corto` en `dir`, si está: el de su sitio o, si no, el de
/// antes. Devuelve dónde se encontró.
pub fn leer_en(dir: &Path, corto: &str) -> Option<(PathBuf, Node)> {
    [ruta_en(dir, corto), legado_en(dir, corto)]
        .into_iter()
        .flatten()
        .find_map(|r| {
            let t = std::fs::read_to_string(&r).ok()?;
            let n = crate::parse::parse(&t).ok()?;
            Some((r, n))
        })
}

/// Retira el puntero de antes de `corto` en `dir`, si está y no es `aqui`.
pub fn retirar_legado(dir: &Path, corto: &str, aqui: &Path) {
    if let Some(l) = legado_en(dir, corto)
        && l != aqui
        && l.is_file()
    {
        let _ = std::fs::remove_file(l);
    }
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

/// **Todos los punteros de `dir`**, por su forma corta: `(ruta, nodo)`. Si un
/// nombre tiene los dos —el de su sitio y el de antes—, manda el de su sitio.
pub fn todos_en(dir: &Path) -> BTreeMap<String, (PathBuf, Node)> {
    let mut out: BTreeMap<String, (PathBuf, Node)> = BTreeMap::new();
    for f in ficheros(dir) {
        let Some(n) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|t| crate::parse::parse(&t).ok())
        else {
            continue;
        };
        let Ok(rel) = f.strip_prefix(dir) else {
            continue;
        };
        let Some(clave) = clave_de(rel, Some(&n)) else {
            continue;
        };
        let de_antes = rel.components().count() == 1;
        if de_antes && out.contains_key(&clave) {
            continue;
        }
        out.insert(clave, (f, n));
    }
    out
}

/// Los punteros del árbol como JSON, por su forma corta: lo que lee el índice
/// de assets ([`crate::assets::indice`]). Fieles: un `null` (la hora que no se
/// sabe, 0063 S6b) sigue nulo y no la cadena `"null"`, que la salud tomaba por
/// una hora.
pub fn del_arbol(raiz: &Path) -> BTreeMap<String, Json> {
    todos_en(&raiz.join(CARPETA))
        .into_iter()
        .map(|(k, (_, n))| (k, Json::de_node_fiel(&n)))
        .collect()
}
