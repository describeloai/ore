//! **El linaje por columna hasta la raíz** (OOS v1alpha14 §5, ADR 0040): de
//! cada columna que un documento expone, de qué columnas **raíz** sale —las de
//! una `Table`, las de un dataset escrito, o las del objeto de una vista de
//! v1alpha7—, y por qué arista: **directa** (tal cual o calculada desde ella)
//! o **INDIRECT** (qué filas salen depende de ella).
//!
//! Hay una sola manera de derivarlo, y es la consulta. Una vista SQL la
//! escribe; una vista estructurada o un dataset mantenido **se traducen a la
//! suya** ([`como_sql`], la tabla de §7 —la misma que usa la migración—) y se
//! analizan igual. Medido antes de escribirlo (`medida-el-linaje-de-la-vista-sql.py`):
//! esa traducción da, en las 115 vistas válidas del repositorio, exactamente
//! el linaje del motor.
//!
//! Lo que esto contesta es lo que el flujo necesita cuando una cadena pasa por
//! una vista SQL, que puede leer varias fuentes y ya no tiene «una raíz»: qué
//! etiquetas lleva cada columna (`flow::carga_de`), de qué datasources sale una
//! entidad (`vistas::datasources_de`) y qué mira un predicado (`OOS4016`).

use std::collections::{BTreeMap, BTreeSet};

use crate::document::{ApiVersion, Kind};
use crate::link::{Loaded, Package};
use crate::vista_sql::{Consulta, Ref, analizar};
use crate::vistas::{self, Fuente};

/// Una columna raíz: la de un documento que es suelo —una `Table` o un
/// dataset escrito, por su nombre cualificado— o la del objeto de una vista de
/// v1alpha7, que no es un documento y se nombra `datasource·objeto`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Raiz {
    pub doc: String,
    pub columna: String,
}

/// Por dónde llega una columna raíz a una columna de salida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arista {
    /// Su valor sale de ella: tal cual, renombrada o calculada.
    Directa,
    /// Qué filas salen depende de ella: un filtro, un join, una agrupación.
    Indirecta,
}

/// Columna de salida → sus raíces.
pub type Linaje = BTreeMap<String, BTreeSet<(Raiz, Arista)>>;

/// El nombre con que la traducción de una vista de v1alpha7 nombra su objeto:
/// no es un nombre del árbol, y el `@` lo dice.
const OBJETO: char = '@';

fn q(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn nombre_sql(qn: &str) -> String {
    qn.split('.').map(q).collect::<Vec<_>>().join(".")
}

fn literal(n: &crate::parse::Node) -> Option<String> {
    let s = n.as_str()?;
    if s.parse::<f64>().is_ok() || s == "true" || s == "false" {
        Some(s.to_string())
    } else {
        Some(format!("'{}'", s.replace('\'', "''")))
    }
}

/// **La forma estructurada como consulta** (v1alpha14 `01-la-vista-es-sql` §7):
/// `from` es el `FROM`, `fields` el `SELECT`, `where` igualdad, pertenencia o
/// ausencia, `groupBy` y `having` los suyos. Vale para una vista de v1alpha7 a
/// v1alpha13 y para un dataset mantenido; `None` para lo demás, o si el
/// documento no tiene la forma que su esquema exige.
pub fn como_sql(v: &Loaded) -> Option<String> {
    if vistas::es_sql(v) {
        return v.section("sql")?.as_str().map(str::to_string);
    }
    let desde = match vistas::fuente(v)? {
        Fuente::Tabla(n) | Fuente::Vista(n) | Fuente::Dataset(n) => nombre_sql(&n),
        Fuente::Datasource { datasource, objeto } => {
            format!("{}.{}", q(&format!("{OBJETO}{datasource}")), q(&objeto))
        }
    };
    let agregados = vistas::agregados(v);
    let mut items: Vec<String> = Vec::new();
    let mut agregado_de: BTreeMap<String, String> = BTreeMap::new();
    if let Some(fs) = v.section("fields") {
        let campos = vistas::campos(v);
        for (k, _) in fs.entries() {
            let campo = k.as_str()?;
            if let Some(a) = agregados.get(campo) {
                let e = match &a.sobre {
                    None => "count(*)".to_string(),
                    Some(c) => format!("{}({})", a.funcion, q(c)),
                };
                agregado_de.insert(campo.to_string(), e.clone());
                items.push(format!("{e} AS {}", q(campo)));
            } else {
                let col = campos.get(campo)?;
                items.push(if col == campo {
                    q(col)
                } else {
                    format!("{} AS {}", q(col), q(campo))
                });
            }
        }
    } else if vistas::es_mantenido(v) {
        // La copia identidad: todo lo que su fuente expone, con sus nombres.
        items.push("*".into());
    } else {
        return None;
    }
    let mut sql = format!("SELECT {}\nFROM {desde}", items.join(", "));
    let filtros: Vec<String> = v
        .section("where")
        .map(|w| {
            w.entries()
                .iter()
                .filter_map(|(k, val)| {
                    let col = q(k.as_str()?);
                    Some(match val {
                        crate::parse::Node::Sequence { items, .. } => {
                            let vals: Vec<String> = items.iter().filter_map(literal).collect();
                            let nulo = items.iter().any(|i| i.as_str().is_none());
                            match (vals.is_empty(), nulo) {
                                (false, false) => format!("{col} IN ({})", vals.join(", ")),
                                (false, true) => {
                                    format!("({col} IN ({}) OR {col} IS NULL)", vals.join(", "))
                                }
                                _ => format!("{col} IS NULL"),
                            }
                        }
                        otro => match literal(otro) {
                            Some(l) => format!("{col} = {l}"),
                            None => format!("{col} IS NULL"),
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if !filtros.is_empty() {
        sql.push_str(&format!("\nWHERE {}", filtros.join(" AND ")));
    }
    let por = vistas::agrupacion(v);
    if !por.is_empty() {
        sql.push_str(&format!(
            "\nGROUP BY {}",
            por.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
        ));
    }
    let teniendo: Vec<String> = vistas::teniendo(v)
        .into_iter()
        .filter_map(|(campo, cond)| {
            let (op, valor) = vistas::condicion(&cond)?;
            let op = if op == "==" { "=" } else { op };
            Some(format!("{} {op} {valor}", agregado_de.get(&campo)?))
        })
        .collect();
    if !teniendo.is_empty() {
        sql.push_str(&format!("\nHAVING {}", teniendo.join(" AND ")));
    }
    Some(sql)
}

/// ¿Se gobierna por su linaje? **Toda vista, de cualquier versión, y todo
/// dataset mantenido**: una sola View dentro de ORE (ADR 0040, decisión A). Un
/// dataset escrito no: es suelo, y lo que lleva lo dice lo que leyó.
///
/// Medido antes de quitar la puerta que dejaba las de v1alpha7 a v1alpha13 en
/// `vistas::raiz()` (`medida-todas-las-vistas-por-el-linaje.py`): `ore
/// validate` da lo mismo en los 410 árboles del repositorio y `ore diff` en los
/// 25 casos de diff.
pub fn por_el_linaje(d: &Loaded) -> bool {
    d.kind == Kind::View || vistas::es_mantenido(d)
}

/// Lo que un nombre de la consulta de `desde` nombra. La de una vista SQL se
/// resuelve por el nombre; la de una forma estructurada, **por su clave**:
/// `from: { table }` dice que es la tabla, aunque una vista se llame igual —y
/// hasta v1alpha13 pueden, y la traducción no puede perderlo—.
fn resolver<'a>(pkg: &'a Package, desde: &Loaded, nombre: &str) -> Option<&'a Loaded> {
    if vistas::es_sql(desde) {
        return vistas::fuente_sql(pkg, nombre, desde);
    }
    match vistas::fuente(desde)? {
        Fuente::Tabla(q) => pkg.table(&q),
        Fuente::Vista(q) => pkg.view(&q),
        Fuente::Dataset(q) => pkg.dataset(&q),
        Fuente::Datasource { .. } => None,
    }
}

/// La consulta de un documento contra el árbol: la suya, o la de su forma.
fn consulta_de(pkg: &Package, d: &Loaded) -> Option<Consulta> {
    let sql = como_sql(d)?;
    let columnas_de = |n: &str| {
        resolver(pkg, d, n).map(|f| vistas::columnas_que_expone(pkg, f).into_iter().collect())
    };
    analizar(&sql, &columnas_de).ok()
}

/// **El linaje de un documento**, compuesto hasta la raíz. `None` si no se
/// puede derivar —la consulta no se analiza, o la cadena vuelve sobre sí—:
/// quien pregunta decide qué hacer con eso, y el enlazado ya lo ha dicho.
pub fn linaje(pkg: &Package, d: &Loaded) -> Option<Linaje> {
    linaje_con(pkg, d, &mut Vec::new())
}

fn linaje_con(pkg: &Package, d: &Loaded, pila: &mut Vec<(Kind, String)>) -> Option<Linaje> {
    let qn = d.qname().unwrap_or_default();
    // El suelo: una tabla, o un dataset que lo llena código.
    if d.kind == Kind::Table || vistas::es_escrito(d) {
        return Some(
            vistas::columnas(d)
                .into_iter()
                .map(|c| {
                    let r = Raiz {
                        doc: qn.clone(),
                        columna: c.clone(),
                    };
                    (c, [(r, Arista::Directa)].into())
                })
                .collect(),
        );
    }
    let clave = (d.kind, qn);
    if pila.contains(&clave) {
        return None;
    }
    pila.push(clave);
    let c = consulta_de(pkg, d);
    let out = c.map(|c| componer(pkg, d, &c, pila));
    pila.pop();
    out
}

/// Las raíces de una columna de una fuente de la consulta de `desde`.
fn raices_con(
    pkg: &Package,
    desde: &Loaded,
    r: &Ref,
    pila: &mut Vec<(Kind, String)>,
) -> BTreeSet<(Raiz, Arista)> {
    if let Some(objeto) = r.fuente.strip_prefix(OBJETO) {
        let (ds, obj) = objeto.split_once('.').unwrap_or((objeto, ""));
        return [(
            Raiz {
                doc: format!("{ds}·{obj}"),
                columna: r.columna.clone(),
            },
            Arista::Directa,
        )]
        .into();
    }
    let Some(f) = resolver(pkg, desde, &r.fuente) else {
        return BTreeSet::new();
    };
    let Some(l) = linaje_con(pkg, f, pila) else {
        return BTreeSet::new();
    };
    l.iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(&r.columna))
        .map(|(_, rs)| rs.clone())
        .unwrap_or_default()
}

/// Las raíces de una columna que la consulta de `desde` nombra.
pub fn raices_de(pkg: &Package, desde: &Loaded, r: &Ref) -> BTreeSet<(Raiz, Arista)> {
    raices_con(pkg, desde, r, &mut Vec::new())
}

fn componer(pkg: &Package, d: &Loaded, c: &Consulta, pila: &mut Vec<(Kind, String)>) -> Linaje {
    let indirectas = |refs: &mut dyn Iterator<Item = &Ref>, pila: &mut Vec<(Kind, String)>| {
        let mut out: BTreeSet<(Raiz, Arista)> = BTreeSet::new();
        for r in refs {
            for (raiz, _) in raices_con(pkg, d, r, pila) {
                out.insert((raiz, Arista::Indirecta));
            }
        }
        out
    };
    let de_todas = indirectas(&mut c.indirectas.iter(), pila);
    let mut out: Linaje = BTreeMap::new();
    for col in &c.columnas {
        let mut rs: BTreeSet<(Raiz, Arista)> = BTreeSet::new();
        for r in col.directas.iter().chain(col.derivadas.iter()) {
            rs.extend(raices_con(pkg, d, r, pila));
        }
        rs.extend(indirectas(&mut col.indirectas.iter(), pila));
        rs.extend(de_todas.iter().cloned());
        out.entry(col.nombre.clone()).or_default().extend(rs);
    }
    out
}

/// ¿Es un documento de v1alpha14 o posterior?
pub fn es_v14(d: &Loaded) -> bool {
    d.version().is_some_and(|v| v >= ApiVersion::V1Alpha14)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use std::path::PathBuf;

    fn doc(kind: Kind, texto: &str) -> Loaded {
        Loaded {
            path: PathBuf::from(format!("{}.yaml", kind.as_str())),
            kind,
            root: parse(texto).expect("yaml"),
        }
    }

    #[test]
    fn la_forma_como_consulta() {
        let v = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: grandes, namespace: hr }\n\
             spec:\n  owner: team:hr\n  from: { dataset: hr.resumen }\n  fields: { empleados: n, pais: pais }\n\
             \x20 where: { pais: [ES, PT], baja: [] }\n",
        );
        assert_eq!(
            como_sql(&v).unwrap(),
            "SELECT \"n\" AS \"empleados\", \"pais\"\nFROM \"hr\".\"resumen\"\n\
             WHERE \"pais\" IN ('ES', 'PT') AND \"baja\" IS NULL"
        );
        let g = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: por_pais, namespace: hr }\n\
             spec:\n  owner: team:hr\n  from: { table: employees }\n  fields: { pais: country, n: \"count()\" }\n\
             \x20 groupBy: [country]\n  having: { n: \">= 8\" }\n",
        );
        assert_eq!(
            como_sql(&g).unwrap(),
            "SELECT \"country\" AS \"pais\", count(*) AS \"n\"\nFROM \"hr\".\"employees\"\n\
             GROUP BY \"country\"\nHAVING count(*) >= 8"
        );
    }
}
