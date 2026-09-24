//! **`ore sql <fichero>`** — lo que un `.sql` del árbol declara: qué lee, qué
//! escribe y en qué modo, o por qué no es una unidad. Lo analiza
//! `ore_core::sql_del_arbol`; si el fichero está dentro de un árbol (hay un
//! `ontology.config.yaml` subiendo, o `--arbol`), además lo coteja con él.
//!
//! Es lo que ore-serve llama antes de correr un `.sql` como trabajo, y lo que
//! el editor pinta: `--json` da los nombres con su línea y su columna. Lo que
//! no hay (una posición, una escritura, una ayuda) es `false`, como en el
//! resto de lo que ore-serve devuelve.
use ore_core::json::Json;
use ore_core::sql_del_arbol::{Fallo, Nombre, analizar, cotejar};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub struct Opciones {
    pub arbol: Option<PathBuf>,
    pub json: bool,
}

/// El árbol que contiene el fichero: el primer directorio subiendo con un
/// `ontology.config.yaml`.
fn arbol_de(fichero: &Path) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(fichero).ok()?;
    abs.ancestors()
        .skip(1)
        .find(|d| d.join("ontology.config.yaml").is_file())
        .map(Path::to_path_buf)
}

fn posicion(pos: Option<ore_core::diag::Pos>) -> [(&'static str, Json); 2] {
    match pos {
        Some(p) => [
            ("linea", Json::Int(p.line as i64)),
            ("columna", Json::Int(p.col as i64)),
        ],
        None => [("linea", Json::Bool(false)), ("columna", Json::Bool(false))],
    }
}

fn nombre_json(n: &Nombre) -> Json {
    let [l, c] = posicion(n.pos);
    Json::obj([("ref", Json::s(n.referencia())), l, c])
}

fn fallo_json(f: &Fallo) -> Json {
    let [l, c] = posicion(f.pos);
    Json::obj([
        ("mensaje", Json::s(&f.mensaje)),
        l,
        c,
        (
            "ayuda",
            f.ayuda.as_deref().map(Json::s).unwrap_or(Json::Bool(false)),
        ),
    ])
}

pub fn sql(fichero: &Path, op: &Opciones) -> ExitCode {
    let texto = match std::fs::read_to_string(fichero) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: no se pudo leer `{}`: {e}", fichero.display());
            return ExitCode::from(66);
        }
    };
    let arbol = op.arbol.clone().or_else(|| arbol_de(fichero));
    let (unidad, fallos) = match analizar(&texto) {
        Ok(u) => {
            let fallos = match &arbol {
                Some(a) => {
                    let (pkg, _) = ore_core::validate::cargar_paquete(a);
                    cotejar(&pkg, &u)
                }
                None => Vec::new(),
            };
            (Some(u), fallos)
        }
        Err(f) => (None, f),
    };

    if op.json {
        let escribe = unidad
            .as_ref()
            .and_then(|u| u.escribe.as_ref())
            .map(|e| {
                let [l, c] = posicion(e.destino.pos);
                Json::obj([
                    ("ref", Json::s(e.destino.referencia())),
                    ("modo", Json::s(e.modo.como_en_write())),
                    l,
                    c,
                ])
            })
            .unwrap_or(Json::Bool(false));
        let lee = unidad
            .as_ref()
            .map(|u| u.lee.iter().map(nombre_json).collect())
            .unwrap_or_default();
        let consulta = unidad
            .as_ref()
            .map(|u| Json::s(&u.consulta))
            .unwrap_or(Json::Bool(false));
        let j = Json::obj([
            ("fichero", Json::s(fichero.to_string_lossy())),
            ("consulta", consulta),
            ("cotejado", Json::Bool(arbol.is_some())),
            ("lee", Json::Arr(lee)),
            ("escribe", escribe),
            ("fallos", Json::Arr(fallos.iter().map(fallo_json).collect())),
        ]);
        println!("{}", j.jcs());
    } else {
        for f in &fallos {
            let donde = f.pos.map(|p| format!(":{p}")).unwrap_or_default();
            eprintln!("{}{donde}: {}", fichero.display(), f.mensaje);
            if let Some(a) = &f.ayuda {
                eprintln!("  ayuda: {a}");
            }
        }
        if let Some(u) = unidad.as_ref().filter(|_| fallos.is_empty()) {
            let lee: Vec<String> = u.lee.iter().map(Nombre::referencia).collect();
            let lee = if lee.is_empty() {
                "nada del árbol".to_string()
            } else {
                lee.join(", ")
            };
            match &u.escribe {
                None => println!("análisis · lee {lee}"),
                Some(e) => println!(
                    "transform · lee {lee} → escribe {} ({})",
                    e.destino.referencia(),
                    e.modo.como_en_write()
                ),
            }
            if arbol.is_none() {
                println!("  (sin árbol alrededor: los nombres no se cotejaron)");
            }
        }
    }
    if fallos.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
