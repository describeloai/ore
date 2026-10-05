//! **`ore federate`**: el plan de una lectura en vivo de una `Table` (ADR 0053
//! F4, `docs/federation.md` §4). No abre nada: lee el árbol y dice qué pedir
//! al origen, o por qué no.
//!
//! ```text
//! ore federate --table b.s.n [--columns a,b] [--filters '[{…}]'] [--policy DIR]
//!              [--from-workspace] [--path .]
//! ```
//!
//! Escribe UNA línea JSON: `{"ok": true, "fuente", "tipo", "env", "objeto",
//! "proyeccion", "fichero"?, "fullScan", "empujados"}` o `{"ok": false,
//! "http", "codigo", "mensaje"}`. Sale 0 en los dos casos: el que llama
//! (`ore-serve`) decide con la línea, no con el código.
//!
//! ⭐ **La política que manda es la de `main`** (decisión de F4): el
//!   interruptor de la fuente (`federation` en `ontology.config.yaml`) y los
//!   conductos (`conduits.yaml`) se leen de `--policy`, un directorio con esos
//!   dos ficheros tal como están en `main`; la tabla, sus columnas y su `reads`,
//!   de la rama. Así una fuente encendida vale en todas las ramas, y apagarla
//!   la apaga en todas.

use std::path::Path;
use std::process::ExitCode;

use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::link::Package;
use ore_core::parse::Node;

pub struct Pedido<'a> {
    pub raiz: &'a Path,
    pub tabla: &'a str,
    pub columnas: Vec<String>,
    pub filtros: Option<&'a str>,
    pub politica: Option<&'a Path>,
    pub desde_puesto: bool,
}

/// Un no, con el estado HTTP que le toca y su código.
struct No {
    http: u16,
    codigo: String,
    mensaje: String,
}

fn no(http: u16, codigo: &str, mensaje: impl Into<String>) -> No {
    No {
        http,
        codigo: codigo.to_string(),
        mensaje: mensaje.into(),
    }
}

pub fn planear(p: &Pedido) -> ExitCode {
    let linea = match intentar(p) {
        Ok(j) => j,
        Err(n) => Json::obj([
            ("ok", Json::Bool(false)),
            ("http", Json::Int(i64::from(n.http))),
            ("codigo", Json::s(n.codigo)),
            ("mensaje", Json::s(n.mensaje)),
        ]),
    };
    println!("{}", linea.jcs());
    ExitCode::SUCCESS
}

/// La familia de `reads.predicatePushdown` a la que pertenece un operador de
/// la petición (v1alpha24 `01` §3: `range` es `lt/le/gt/ge`, `isNull` las dos
/// formas del nulo).
fn familia(op: &str) -> Option<&'static str> {
    Some(match op {
        "eq" => "eq",
        "neq" => "neq",
        "in" => "in",
        "lt" | "le" | "gt" | "ge" => "range",
        "like" => "like",
        "isNull" | "isNotNull" => "isNull",
        _ => return None,
    })
}

fn intentar(p: &Pedido) -> Result<Json, No> {
    let (mut pkg, _) = ore_core::validate::cargar_paquete(p.raiz);
    if let Some(dir) = p.politica {
        politica_de_main(&mut pkg, dir)?;
    }

    // ① La tabla, en la rama.
    let t = pkg.table(p.tabla).ok_or_else(|| {
        no(
            404,
            "objeto",
            format!("no hay una `Table` `{}` en esta rama", p.tabla),
        )
    })?;
    let qn = t.qname().unwrap_or_default();
    if t.section("reads").and_then(Node::as_str) == Some("none") {
        return Err(no(
            422,
            "OOS2020",
            format!("`{qn}` declara `reads: none`: no se lee"),
        ));
    }
    let fuente = t
        .section("datasource")
        .and_then(Node::as_str)
        .map(String::from)
        .ok_or_else(|| no(422, "OOS2020", format!("`{qn}` no declara `datasource`")))?;

    // ⓪ El interruptor de la fuente, de la política (main).
    let ds = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::OntologyConfig)
        .flat_map(|c| {
            c.section("datasources")
                .map(|n| n.items().to_vec())
                .unwrap_or_default()
        })
        .find(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(fuente.as_str()))
        .ok_or_else(|| {
            no(
                404,
                "objeto",
                format!("la fuente `{fuente}` no está declarada en `main`"),
            )
        })?;
    let campo = |k: &str| ds.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    if campo("federation").as_deref() != Some("true") {
        return Err(no(
            403,
            "federacion",
            format!(
                "la fuente `{fuente}` no tiene la lectura en vivo encendida: se enciende en la fuente (`ore source federation {fuente} on`, o en la consola) y vale para todas las ramas"
            ),
        ));
    }
    let tipo = campo("type").ok_or_else(|| {
        no(
            422,
            "OOS2020",
            format!("la fuente `{fuente}` no declara `type`"),
        )
    })?;
    let env = campo("connectionEnv").ok_or_else(|| {
        no(
            422,
            "OOS2020",
            format!("la fuente `{fuente}` no declara `connectionEnv`"),
        )
    })?;
    let objeto = t
        .section("object")
        .and_then(Node::as_str)
        .map(String::from)
        .ok_or_else(|| no(422, "OOS2020", format!("`{qn}` no declara `object`")))?;

    // Las columnas: las pedidas, o todas.
    let cols_tabla: Vec<String> = t
        .section("columns")
        .map(|c| {
            c.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let columnas = if p.columnas.is_empty() {
        cols_tabla.clone()
    } else {
        p.columnas.clone()
    };
    for c in &columnas {
        if !cols_tabla.contains(c) {
            return Err(no(
                422,
                "objeto",
                format!("`{qn}` no tiene la columna `{c}`"),
            ));
        }
    }

    // Los filtros: sólo lo que la tabla deja empujar. Lo demás lo evaluará el
    // motor (F5); hasta entonces se dice, no se descarta en silencio.
    let reads = t.section("reads");
    let admitidas: Vec<String> = reads
        .and_then(|r| r.get("predicatePushdown"))
        .map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|o| o.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut filtros: Vec<(String, String)> = Vec::new();
    if let Some(f) = p.filtros {
        let n =
            ore_core::parse::parse(f).map_err(|_| no(400, "operador", "`--filters` no es JSON"))?;
        for f in n.items() {
            let col = f
                .get("columna")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or_default()
                .to_string();
            let op = f
                .get("operador")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("eq")
                .to_string();
            if !cols_tabla.contains(&col) {
                return Err(no(
                    422,
                    "objeto",
                    format!("un filtro sobre `{col}`, que `{qn}` no tiene"),
                ));
            }
            let Some(fam) = familia(&op) else {
                return Err(no(
                    400,
                    "operador",
                    format!("`{op}` no es un operador de la petición"),
                ));
            };
            if !admitidas.iter().any(|a| a == fam) {
                return Err(no(
                    422,
                    "empuje",
                    format!(
                        "`{op}` sobre `{col}`: `{qn}` no deja empujarlo (`reads.predicatePushdown`: {}); evaluarlo en el motor llega con F5",
                        if admitidas.is_empty() {
                            "ninguno".to_string()
                        } else {
                            admitidas.join(", ")
                        }
                    ),
                ));
            }
            filtros.push((col, op));
        }
    }

    // ④ El coste que la tabla declara (v1alpha24 `01` §4).
    let full_scan = reads
        .and_then(|r| r.get("fullScan"))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("cheap")
        .to_string();
    if full_scan == "forbidden" && filtros.is_empty() {
        return Err(no(
            422,
            "OOS2044",
            format!(
                "`{qn}` declara `fullScan: forbidden` y ningún filtro empujado acota la lectura"
            ),
        ));
    }
    let requeridos: Vec<String> = reads
        .and_then(|r| r.get("requiredFilters"))
        .map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|o| o.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for c in &requeridos {
        if !filtros
            .iter()
            .any(|(col, op)| col == c && (op == "eq" || op == "in"))
        {
            return Err(no(
                422,
                "OOS2045",
                format!(
                    "`{qn}` exige un filtro `eq` o `in` empujado sobre `{c}` (`requiredFilters`)"
                ),
            ));
        }
    }

    // ② El conducto, con la política de main: lo pedido y lo filtrado.
    let mut tocadas = columnas.clone();
    for (c, _) in &filtros {
        if !tocadas.contains(c) {
            tocadas.push(c.clone());
        }
    }
    if let Err(n) = ore_core::flow::lectura_del_origen(&pkg, &qn, &tocadas, p.desde_puesto) {
        return Err(no(403, n.codigo, n.mensaje));
    }

    // La petición para el conector: proyección, y el `fichero` de una tabla de
    // ficheros (sus tipos congelados, como la copia).
    let mut o = vec![
        ("ok", Json::Bool(true)),
        ("tabla", Json::s(qn.as_str())),
        ("fuente", Json::s(fuente.as_str())),
        ("tipo", Json::s(tipo)),
        ("env", Json::s(env)),
        ("objeto", Json::s(objeto)),
        (
            "proyeccion",
            Json::Obj(
                columnas
                    .iter()
                    .map(|c| (c.clone(), Json::s(c.as_str())))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s(full_scan)),
        (
            "empujados",
            Json::Arr(
                filtros
                    .iter()
                    .map(|(c, op)| {
                        Json::obj([
                            ("columna", Json::s(c.as_str())),
                            ("operador", Json::s(op.as_str())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    if let Some(formato) = t.section("format") {
        let tipos = t
            .section("columns")
            .map(|c| {
                c.entries()
                    .iter()
                    .filter_map(|(k, v)| {
                        let tipo = v.get("type").and_then(|(_, t)| t.as_str())?;
                        Some(Json::Arr(vec![Json::s(k.as_str()?), Json::s(tipo)]))
                    })
                    .collect()
            })
            .unwrap_or_default();
        o.push((
            "fichero",
            Json::obj([
                ("format", Json::de_node(formato)),
                ("tipos", Json::Arr(tipos)),
            ]),
        ));
    }
    Ok(Json::obj(o))
}

/// **La política de `main`** sobre el paquete de la rama: su manifiesto (las
/// fuentes y su interruptor) y sus conductos sustituyen a los de la rama.
fn politica_de_main(pkg: &mut Package, dir: &Path) -> Result<(), No> {
    let (main, _) = ore_core::validate::cargar_paquete(dir);
    let de_main: Vec<_> = main
        .docs
        .into_iter()
        .filter(|d| matches!(d.kind, Kind::OntologyConfig | Kind::ConduitPolicy))
        .collect();
    if !de_main.iter().any(|d| d.kind == Kind::OntologyConfig) {
        return Err(no(
            500,
            "politica",
            "la política de main no trae `ontology.config.yaml`",
        ));
    }
    pkg.docs
        .retain(|d| !matches!(d.kind, Kind::OntologyConfig | Kind::ConduitPolicy));
    pkg.docs.extend(de_main);
    Ok(())
}
