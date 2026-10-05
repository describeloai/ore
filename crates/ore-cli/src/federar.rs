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
//!
//! ⭐ **Desde F5·2 decide el reparto** (`ore_core::reparto`): la petición se
//!   escribe como la sentencia que es —`SELECT columnas FROM tabla WHERE
//!   filtros`— y el coste, el interruptor y el gobierno son los mismos que
//!   para una consulta del puesto. Lo que el reparto dejaría para el motor,
//!   aquí se niega (`422 empuje`): quien pide por esta ruta no tiene motor.

use std::path::Path;
use std::process::ExitCode;

use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::link::Package;
use ore_core::parse::Node;
use ore_core::reparto::{self, Opciones};

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

/// Un identificador entre comillas dobles, como lo escribe DuckDB.
fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn cadena(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Un filtro de la petición como condición SQL, para el reparto.
fn condicion(col: &str, op: &str, valor: Option<&Node>) -> Result<String, No> {
    let uno = || {
        valor.and_then(Node::as_str).map(cadena).ok_or_else(|| {
            no(
                400,
                "operador",
                format!("`{op}` sobre `{col}` necesita un `valor`"),
            )
        })
    };
    let c = ident(col);
    Ok(match op {
        "eq" => format!("{c} = {}", uno()?),
        "neq" => format!("{c} <> {}", uno()?),
        "lt" => format!("{c} < {}", uno()?),
        "le" => format!("{c} <= {}", uno()?),
        "gt" => format!("{c} > {}", uno()?),
        "ge" => format!("{c} >= {}", uno()?),
        "like" => format!("{c} LIKE {}", uno()?),
        "isNull" => format!("{c} IS NULL"),
        "isNotNull" => format!("{c} IS NOT NULL"),
        "in" => {
            let vs: Vec<String> = valor
                .map(|v| {
                    v.items()
                        .iter()
                        .filter_map(Node::as_str)
                        .map(cadena)
                        .collect()
                })
                .unwrap_or_default();
            if vs.is_empty() {
                return Err(no(
                    400,
                    "operador",
                    format!("`in` sobre `{col}` necesita una lista"),
                ));
            }
            format!("{c} IN ({})", vs.join(", "))
        }
        _ => {
            return Err(no(
                400,
                "operador",
                format!("`{op}` no es un operador de la petición"),
            ));
        }
    })
}

fn intentar(p: &Pedido) -> Result<Json, No> {
    let (mut pkg, _) = ore_core::validate::cargar_paquete(p.raiz);
    if let Some(dir) = p.politica {
        politica_de_main(&mut pkg, dir).map_err(|(http, codigo, mensaje)| No {
            http,
            codigo,
            mensaje,
        })?;
    }

    // ① La tabla y sus columnas, en la rama: lo que el reparto daría por
    //   «del lago» aquí es un 404, porque se pidió una tabla.
    let t = pkg.table(p.tabla).ok_or_else(|| {
        no(
            404,
            "objeto",
            format!("no hay una `Table` `{}` en esta rama", p.tabla),
        )
    })?;
    let qn = t.qname().unwrap_or_default();
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

    // La petición, escrita como sentencia.
    let mut condiciones = Vec::new();
    let mut pedidos = 0usize;
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
            condiciones.push(condicion(&col, &op, f.get("valor").map(|(_, v)| v))?);
            pedidos += 1;
        }
    }
    let tabla_sql: Vec<String> = qn.split('.').map(ident).collect();
    let sql = format!(
        "SELECT {} FROM {}{}",
        columnas
            .iter()
            .map(|c| ident(c))
            .collect::<Vec<_>>()
            .join(", "),
        tabla_sql.join("."),
        if condiciones.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", condiciones.join(" AND "))
        }
    );

    // ⓪②④ El reparto: interruptor (de main), coste y conducto.
    let o = Opciones {
        desde_puesto: p.desde_puesto,
        exigir_interruptor: true,
        conectores: None,
        copia: false,
    };
    let r = reparto::repartir(&sql, &pkg, &o).map_err(|n| no(n.http, &n.codigo, n.mensaje))?;
    let l = r
        .lecturas
        .into_iter()
        .find(|l| l.tabla == qn)
        .ok_or_else(|| no(500, "objeto", format!("el reparto no lee `{qn}`")))?;

    // Lo que quedaría en el motor: esta ruta no tiene motor.
    if l.empujados.len() < pedidos || !l.en_el_motor.is_empty() {
        let admitidas: Vec<String> = t
            .section("reads")
            .and_then(|r| r.get("predicatePushdown"))
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|o| o.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        return Err(no(
            422,
            "empuje",
            format!(
                "{}: `{qn}` no deja empujarlo (`reads.predicatePushdown`: {}); en el SQL del puesto lo evalúa el motor",
                l.en_el_motor.join(", "),
                if admitidas.is_empty() {
                    "ninguno".to_string()
                } else {
                    admitidas.join(", ")
                }
            ),
        ));
    }

    // La petición para el conector: proyección, y el `fichero` de una tabla de
    // ficheros (sus tipos congelados, como la copia).
    let mut o = vec![
        ("ok", Json::Bool(true)),
        ("tabla", Json::s(qn.as_str())),
        ("fuente", Json::s(l.fuente.as_str())),
        ("tipo", Json::s(l.tipo.as_str())),
        ("env", Json::s(l.env.as_str())),
        ("objeto", Json::s(l.objeto.as_str())),
        (
            "proyeccion",
            Json::Obj(
                columnas
                    .iter()
                    .map(|c| (c.clone(), Json::s(c.as_str())))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s(l.full_scan.as_str())),
        (
            "empujados",
            Json::Arr(
                l.empujados
                    .iter()
                    .map(|f| {
                        Json::obj([
                            ("columna", Json::s(f.columna.as_str())),
                            ("operador", Json::s(f.operador.as_str())),
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
/// `Err((http, código, mensaje))`.
pub(crate) fn politica_de_main(pkg: &mut Package, dir: &Path) -> Result<(), (u16, String, String)> {
    let (main, _) = ore_core::validate::cargar_paquete(dir);
    let de_main: Vec<_> = main
        .docs
        .into_iter()
        .filter(|d| matches!(d.kind, Kind::OntologyConfig | Kind::ConduitPolicy))
        .collect();
    if !de_main.iter().any(|d| d.kind == Kind::OntologyConfig) {
        return Err((
            500,
            "politica".into(),
            "la política de main no trae `ontology.config.yaml`".into(),
        ));
    }
    pkg.docs
        .retain(|d| !matches!(d.kind, Kind::OntologyConfig | Kind::ConduitPolicy));
    pkg.docs.extend(de_main);
    Ok(())
}
