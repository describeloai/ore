//! **`ore sql <fichero>`** — lo que un `.sql` del árbol declara: qué lee, qué
//! escribe y en qué modo, o por qué no es una unidad. Un guion de varias
//! sentencias, cada una con lo suyo y en orden (`sentencias` en `--json`). Lo analiza
//! `ore_core::sql_del_arbol`; si el fichero está dentro de un árbol (hay un
//! `ontology.config.yaml` subiendo, o `--arbol`), además lo coteja con él.
//!
//! Es lo que ore-serve llama antes de correr un `.sql` como trabajo, y lo que
//! el editor pinta: `--json` da los nombres con su línea y su columna. Lo que
//! no hay (una posición, una escritura, una ayuda) es `false`, como en el
//! resto de lo que ore-serve devuelve.
use ore_core::json::Json;
use ore_core::sql_del_arbol::guion::{Sentencia, Trozo, cotejar_guion, guion};
use ore_core::sql_del_arbol::{Fallo, Nombre, Unidad};
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
        ("codigo", f.codigo.map(Json::s).unwrap_or(Json::Bool(false))),
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
    let (trozos, fallos) = match guion(&texto) {
        Ok(t) => {
            let fallos = match &arbol {
                Some(a) => {
                    let (pkg, _) = ore_core::validate::cargar_paquete(a);
                    cotejar_guion(&pkg, &t)
                }
                None => Vec::new(),
            };
            (t, fallos)
        }
        Err(f) => (Vec::new(), f),
    };
    // Una sola unidad: lo de siempre (`lee`, `escribe`, `consulta`).
    let unidad: Option<&Unidad> = match trozos.as_slice() {
        [
            Trozo {
                sentencia: Sentencia::Unidad(u),
                ..
            },
        ] => Some(u),
        _ => None,
    };
    let avisos: Vec<&Fallo> = trozos.iter().flat_map(|t| &t.avisos).collect();

    if op.json {
        let escribe = unidad
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
            .map(|u| u.lee.iter().map(nombre_json).collect())
            .unwrap_or_default();
        let consulta = unidad
            .map(|u| Json::s(&u.consulta))
            .unwrap_or(Json::Bool(false));
        let j = Json::obj([
            ("fichero", Json::s(fichero.to_string_lossy())),
            ("consulta", consulta),
            ("cotejado", Json::Bool(arbol.is_some())),
            ("lee", Json::Arr(lee)),
            ("escribe", escribe),
            ("fallos", Json::Arr(fallos.iter().map(fallo_json).collect())),
            // 0038: lo que no para la frase —hoy, `ORE-SQL-2P`—
            (
                "avisos",
                Json::Arr(avisos.iter().map(|a| fallo_json(a)).collect()),
            ),
            // el guion, sentencia a sentencia y en orden
            (
                "sentencias",
                Json::Arr(
                    trozos
                        .iter()
                        .map(|t| {
                            let [l, c] = posicion(t.pos);
                            Json::obj([
                                ("que", Json::s(t.sentencia.que())),
                                l,
                                c,
                                ("texto", Json::s(&t.texto)),
                                ("dice", Json::s(dice(&t.sentencia))),
                            ])
                        })
                        .collect(),
                ),
            ),
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
        for a in &avisos {
            let donde = a.pos.map(|p| format!(":{p}")).unwrap_or_default();
            eprintln!(
                "{}{donde}: aviso[{}]: {}",
                fichero.display(),
                a.codigo.unwrap_or(""),
                a.mensaje
            );
            if let Some(ay) = &a.ayuda {
                eprintln!("  ayuda: {ay}");
            }
        }
        if fallos.is_empty() && !trozos.is_empty() {
            if let Some(u) = unidad {
                println!("{}", dice_de_unidad(u));
            } else {
                for (i, t) in trozos.iter().enumerate() {
                    let donde = t.pos.map(|p| format!("{p}")).unwrap_or_default();
                    println!("{} · {donde} · {}", i + 1, dice(&t.sentencia));
                }
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

fn dice_de_unidad(u: &Unidad) -> String {
    let lee: Vec<String> = u.lee.iter().map(Nombre::referencia).collect();
    let lee = if lee.is_empty() {
        "nada del árbol".to_string()
    } else {
        lee.join(", ")
    };
    match &u.escribe {
        None => format!("análisis · lee {lee}"),
        Some(e) => format!(
            "transform · lee {lee} → escribe {} ({})",
            e.destino.referencia(),
            e.modo.como_en_write()
        ),
    }
}

/// Lo que una sentencia del guion hace, en una línea.
fn dice(s: &Sentencia) -> String {
    let si = |b: bool| if b { " · si no existe" } else { "" };
    match s {
        Sentencia::Unidad(u) => dice_de_unidad(u),
        Sentencia::CrearBase {
            nombre,
            clase,
            origen,
            si_no_existe,
            ..
        } => format!(
            "crea la {} database `{nombre}`{}{}",
            clase.como_en_el_alta(),
            origen
                .as_ref()
                .map(|o| format!(" del origen `{}` ({})", o.nombre, o.incluye.join(", ")))
                .unwrap_or_default(),
            si(*si_no_existe)
        ),
        Sentencia::CrearSchema {
            base,
            schema,
            si_no_existe,
            ..
        } => format!("crea el schema `{base}.{schema}`{}", si(*si_no_existe)),
        Sentencia::CrearDataset {
            destino,
            columnas,
            clave,
            si_no_existe,
        } => format!(
            "crea el dataset `{}` vacío ({}){}{}",
            destino.referencia(),
            columnas
                .iter()
                .map(|c| format!("{} {}", c.nombre, c.tipo))
                .collect::<Vec<_>>()
                .join(", "),
            if clave.is_empty() {
                String::new()
            } else {
                format!(" · clave ({})", clave.join(", "))
            },
            si(*si_no_existe)
        ),
    }
}
