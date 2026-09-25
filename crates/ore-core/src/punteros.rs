//! **Los punteros de los datasets en el árbol** (0038 P2): dónde vive el de
//! cada nombre, cómo se lee el de antes, y qué nombre dice cada fichero.
//!
//! ```text
//! datasets/<base>/<schema>/<nombre>.json      ventas/default/pedidos.json
//! ```
//!
//! Un separador que ningún nombre lleva: `a_b.c` y `a.b_c` eran el mismo
//! `a_b_c.json` (medido, `medida-los-punteros.sh` M2), y borrar `ventas` se
//! llevaba los de `ventas_eu` (M4). **El de antes** (`datasets/<p>_<n>.json`,
//! sólo `default`) se sigue leyendo hasta que `ore migrate` lo mueva, y quien
//! escribe un puntero lo deja en su sitio y retira el de antes.
//!
//! **En el lago**, un dataset nuevo se llama `catalogo/<base>/<schema>/<n>`
//! ([`dataset_nuevo`]) y no `datasets/<base>/…`: `datasets/ventas_x` —la tabla
//! `x` de `ventas`, de antes— es prefijo de `datasets/ventas_x/default/n`, y la
//! credencial prestada a la primera (acotada a su prefijo) alcanzaría la
//! segunda. Lo que ya está no se mueve: el puntero guarda su `dataset`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::json::Json;
use crate::normalize::SCHEMA_POR_DEFECTO;
use crate::parse::Node;

/// La carpeta de los punteros, bajo la raíz del árbol.
pub const CARPETA: &str = "datasets";

/// `(base, schema, nombre)` de una forma corta (`p.n` o `p.s.n`).
pub fn partes(corto: &str) -> Option<(&str, &str, &str)> {
    let v: Vec<&str> = corto.split('.').collect();
    match v.as_slice() {
        [p, n] if !p.is_empty() && !n.is_empty() => Some((p, SCHEMA_POR_DEFECTO, n)),
        [p, s, n] if !p.is_empty() && !s.is_empty() && !n.is_empty() => Some((p, s, n)),
        _ => None,
    }
}

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

/// El nombre en el lago de un dataset que nace: `catalogo/<base>/<schema>/<n>`.
pub fn dataset_nuevo(corto: &str) -> String {
    match partes(corto) {
        Some((b, s, n)) => format!("catalogo/{b}/{s}/{n}"),
        None => format!("catalogo/{}", corto.replace('.', "/")),
    }
}

/// **El nombre en el lago de una tabla, por su `metadata_location`**: lo que
/// hay entre `ore/v2/` y su `/metadata/` (`s3://b/ore/v2/datasets/ventas_x/
/// metadata/00001-….metadata.json` → `datasets/ventas_x`). Es lo cierto —donde
/// están los bytes—, y lo que un puntero tiene que reclamar: un nombre que no
/// fuera el suyo dejaría la tabla huérfana para `recoger-huerfanas`.
pub fn dataset_de_ubicacion(ml: &str) -> Option<String> {
    let (_, resto) = ml.split_once("/ore/v2/")?;
    let (d, _) = resto.rsplit_once("/metadata/")?;
    (!d.is_empty()).then(|| d.to_string())
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
/// de assets ([`crate::assets::indice`]).
pub fn del_arbol(raiz: &Path) -> BTreeMap<String, Json> {
    todos_en(&raiz.join(CARPETA))
        .into_iter()
        .map(|(k, (_, n))| (k, Json::de_node(&n)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_ruta_el_legado_y_el_nombre_en_el_lago() {
        let d = Path::new("datasets");
        assert_eq!(
            ruta_en(d, "ventas.pedidos").unwrap(),
            d.join("ventas").join("default").join("pedidos.json")
        );
        assert_eq!(
            ruta_en(d, "ventas.espana.pedidos").unwrap(),
            d.join("ventas").join("espana").join("pedidos.json")
        );
        assert_eq!(
            legado_en(d, "ventas.pedidos").unwrap(),
            d.join("ventas_pedidos.json")
        );
        assert_eq!(legado_en(d, "ventas.espana.pedidos"), None);
        assert_eq!(ruta("a_b.c").unwrap(), "datasets/a_b/default/c.json");
        assert_eq!(ruta("a.b_c").unwrap(), "datasets/a/default/b_c.json");
        assert_eq!(
            dataset_nuevo("ventas.pedidos"),
            "catalogo/ventas/default/pedidos"
        );
        assert_eq!(partes("x"), None);
        assert_eq!(
            dataset_de_ubicacion(
                "s3://copia/ore/v2/datasets/ventas_x/metadata/00001-a.metadata.json"
            )
            .as_deref(),
            Some("datasets/ventas_x")
        );
        assert_eq!(
            dataset_de_ubicacion("s3://c/ore/v2/catalogo/v/default/p/metadata/1.metadata.json")
                .as_deref(),
            Some("catalogo/v/default/p")
        );
        assert_eq!(dataset_de_ubicacion("gs://b/otra/cosa.json"), None);
    }

    #[test]
    fn la_clave_de_un_fichero_nuevo_y_de_uno_de_antes() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            clave_de(&p("ventas/default/pedidos.json"), None).unwrap(),
            "ventas.pedidos"
        );
        assert_eq!(
            clave_de(&p("ventas/espana/pedidos.json"), None).unwrap(),
            "ventas.espana.pedidos"
        );
        assert_eq!(
            clave_de(&p("ventas_pedidos.json"), None).unwrap(),
            "ventas.pedidos"
        );
        let n = crate::parse::parse(r#"{"tabla":"a_b.c"}"#).unwrap();
        assert_eq!(clave_de(&p("a_b_c.json"), Some(&n)).unwrap(), "a_b.c");
        let n = crate::parse::parse(r#"{"vista":"ventas.default.v"}"#).unwrap();
        assert_eq!(clave_de(&p("ventas_v.json"), Some(&n)).unwrap(), "ventas.v");
        assert_eq!(clave_de(&p("a/b.json"), None), None);
    }

    #[test]
    fn se_lee_el_de_su_sitio_y_si_no_el_de_antes() {
        let d = std::env::temp_dir().join(format!("ore-punteros-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("ventas/default")).unwrap();
        std::fs::write(
            d.join("ventas_viejo.json"),
            r#"{"dataset":"datasets/ventas_viejo"}"#,
        )
        .unwrap();
        std::fs::write(d.join("ventas_nuevo.json"), r#"{"dataset":"antes"}"#).unwrap();
        std::fs::write(
            d.join("ventas/default/nuevo.json"),
            r#"{"dataset":"ahora"}"#,
        )
        .unwrap();
        let (r, _) = leer_en(&d, "ventas.viejo").unwrap();
        assert_eq!(r, d.join("ventas_viejo.json"));
        let t = todos_en(&d);
        assert_eq!(t.len(), 2, "{:?}", t.keys());
        assert_eq!(
            t["ventas.nuevo"].1.get("dataset").unwrap().1.as_str(),
            Some("ahora")
        );
        retirar_legado(&d, "ventas.nuevo", &d.join("ventas/default/nuevo.json"));
        assert!(!d.join("ventas_nuevo.json").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
