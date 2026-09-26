//! **Servir una vista es servir su consulta** (ADR 0040 paso 4): la de una
//! vista SQL tal cual la escribió quien la declaró, la de una estructurada su
//! traducción (`linaje::como_sql`) —una sola View, decisión A—. Lo guardado no
//! se toca; lo que se sirve lleva **cada nombre del árbol resuelto**:
//!
//! - un **dataset**, por el nombre con que lo registra quien lo lee: en un
//!   puesto, `"__ore_dataset"."<p>.<n>"` (el esquema interno donde los tres SDK
//!   ponen cada dataset); en el catálogo (`/v1`), por su nombre del catálogo;
//! - una **vista**, por su propia consulta servida, como subconsulta con el
//!   alias que tenía: quien lee no necesita tener registradas las de debajo;
//! - una **tabla** de un origen no se sirve desde lo que se tiene: se lee por
//!   un dataset que la copie.
//!
//! Y resuelto **del todo**, no dejado a DuckDB: medido
//! (`medida-servir-la-vista-como-sql.py`), un nombre de dos partes dentro de una
//! vista de DuckDB se resuelve contra el schema de esa vista, no contra `main`,
//! y `ventas.clientes` desde el schema `espana` no encuentra
//! `ventas.default.clientes`.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use sqlparser::ast::{
    Ident, ObjectName, ObjectNamePart, Query, Statement, TableAlias, TableFactor, Visit, VisitMut,
    Visitor, VisitorMut,
};
use sqlparser::dialect::DuckDbDialect;
use sqlparser::parser::Parser;

use crate::document::Kind;
use crate::link::{Loaded, Package};

/// El esquema interno donde un puesto registra cada dataset
/// (`"__ore_dataset"."<p>.<n>"`). Aparte de los nombres del árbol porque una
/// View y su dataset pueden llamarse igual (v1alpha12).
pub const ESQUEMA_DE_DATASETS: &str = "__ore_dataset";

/// Quién lee lo servido, y por tanto cómo se nombra un dataset.
#[derive(Debug, Clone, Copy)]
pub enum Para<'a> {
    /// Un puesto: `"__ore_dataset"."<p>.<n>"`.
    Puesto,
    /// El catálogo (`/v1`). Sin `base`, el namespace es la base: lo de
    /// `default` es `"p"."n"` y lo demás `"p"."s"."n"`. Con `base` (la petición
    /// traía `prefix`), el namespace es el schema: lo de esa base es
    /// `"s"."n"`, y lo de otra `"p"."s"."n"`.
    Catalogo { base: Option<&'a str> },
}

/// Una consulta servida: el texto y los datasets que lee, en el orden en que
/// aparecen, por su nombre corto (`p.n`, `p.s.n`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Servida {
    pub consulta: String,
    pub datasets: Vec<String>,
}

/// La consulta de una vista, servida. `Err` dice por qué no se sirve.
pub fn servir(pkg: &Package, v: &Loaded, para: Para) -> Result<Servida, String> {
    let mut datasets = Vec::new();
    let consulta = servir_con(pkg, v, para, &mut datasets, &mut Vec::new())?;
    Ok(Servida { consulta, datasets })
}

fn servir_con(
    pkg: &Package,
    v: &Loaded,
    para: Para,
    datasets: &mut Vec<String>,
    pila: &mut Vec<String>,
) -> Result<String, String> {
    let qn = v.qname().unwrap_or_default();
    if pila.contains(&qn) {
        return Err(format!(
            "lo que `{qn}` lee vuelve sobre ella: {} → {qn}",
            pila.join(" → ")
        ));
    }
    let sql =
        crate::linaje::como_sql(v).ok_or_else(|| format!("`{qn}` no se escribe como consulta"))?;
    let mut sentencias = Parser::parse_sql(&DuckDbDialect {}, &sql)
        .map_err(|e| format!("la consulta de `{qn}` no se analiza: {e}"))?;
    let [Statement::Query(q)] = sentencias.as_mut_slice() else {
        return Err(format!("la consulta de `{qn}` no es UNA consulta"));
    };
    let ctes = nombres_de_with(q);
    pila.push(qn);
    let mut r = Resolver {
        pkg,
        desde: v,
        para,
        ctes,
        datasets,
        pila,
        fallo: None,
    };
    let _ = VisitMut::visit(q.as_mut(), &mut r);
    let fallo = r.fallo.take();
    pila.pop();
    match fallo {
        Some(f) => Err(f),
        None => Ok(q.to_string()),
    }
}

/// Los nombres que la consulta define en sus `WITH`, a cualquier profundidad:
/// no son del árbol.
fn nombres_de_with(q: &Query) -> BTreeSet<String> {
    struct Ctes(BTreeSet<String>);
    impl Visitor for Ctes {
        type Break = ();
        fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
            if let Some(w) = &q.with {
                for c in &w.cte_tables {
                    self.0.insert(c.alias.name.value.to_lowercase());
                }
            }
            ControlFlow::Continue(())
        }
    }
    let mut c = Ctes(BTreeSet::new());
    let _ = Visit::visit(q, &mut c);
    c.0
}

fn citado(s: &str) -> ObjectNamePart {
    ObjectNamePart::Identifier(Ident::with_quote('"', s))
}

struct Resolver<'a, 'b> {
    pkg: &'a Package,
    desde: &'a Loaded,
    para: Para<'a>,
    ctes: BTreeSet<String>,
    datasets: &'b mut Vec<String>,
    pila: &'b mut Vec<String>,
    fallo: Option<String>,
}

impl Resolver<'_, '_> {
    fn nombre_del_dataset(&self, corto: &str) -> ObjectName {
        let Some((p, s, n)) = crate::punteros::partes(corto) else {
            return ObjectName(vec![citado(ESQUEMA_DE_DATASETS), citado(corto)]);
        };
        ObjectName(match self.para {
            Para::Puesto => vec![citado(ESQUEMA_DE_DATASETS), citado(corto)],
            Para::Catalogo { base: None } if s == crate::normalize::SCHEMA_POR_DEFECTO => {
                vec![citado(p), citado(n)]
            }
            Para::Catalogo { base: Some(b) } if b == p => vec![citado(s), citado(n)],
            Para::Catalogo { .. } => vec![citado(p), citado(s), citado(n)],
        })
    }
}

impl VisitorMut for Resolver<'_, '_> {
    type Break = ();

    fn post_visit_table_factor(&mut self, tf: &mut TableFactor) -> ControlFlow<()> {
        let TableFactor::Table {
            name,
            alias,
            args: None,
            ..
        } = tf
        else {
            return ControlFlow::Continue(());
        };
        let partes: Vec<String> = name
            .0
            .iter()
            .map(|p| match p {
                ObjectNamePart::Identifier(i) => i.value.clone(),
                otra => otra.to_string(),
            })
            .collect();
        let escrito = partes.join(".");
        if partes.len() == 1 && self.ctes.contains(&escrito.to_lowercase()) {
            return ControlFlow::Continue(());
        }
        let Some(d) = crate::linaje::resolver(self.pkg, self.desde, &escrito) else {
            self.fallo = Some(if escrito.starts_with('@') {
                format!(
                    "`{}` lee del objeto de un origen (`{}`), no de un dataset: una vista se \
                     sirve desde lo que se tiene",
                    self.desde.qname().unwrap_or_default(),
                    escrito.trim_start_matches('@')
                )
            } else {
                format!(
                    "`{}` lee `{escrito}`, que no está en el árbol",
                    self.desde.qname().unwrap_or_default()
                )
            });
            return ControlFlow::Break(());
        };
        let qn = d.qname().unwrap_or_default();
        match d.kind {
            Kind::Dataset => {
                if !self.datasets.contains(&qn) {
                    self.datasets.push(qn.clone());
                }
                *name = self.nombre_del_dataset(&qn);
                ControlFlow::Continue(())
            }
            Kind::View => {
                let servida = match servir_con(self.pkg, d, self.para, self.datasets, self.pila) {
                    Ok(s) => s,
                    Err(e) => {
                        self.fallo = Some(e);
                        return ControlFlow::Break(());
                    }
                };
                let Ok(mut s) = Parser::parse_sql(&DuckDbDialect {}, &servida) else {
                    self.fallo = Some(format!("la consulta servida de `{qn}` no se analiza"));
                    return ControlFlow::Break(());
                };
                let Some(Statement::Query(sub)) = s.pop() else {
                    self.fallo = Some(format!("la consulta servida de `{qn}` no es una consulta"));
                    return ControlFlow::Break(());
                };
                let alias = alias.clone().unwrap_or(TableAlias {
                    explicit: true,
                    name: Ident::with_quote('"', partes.last().cloned().unwrap_or_default()),
                    columns: vec![],
                    at: None,
                });
                *tf = TableFactor::Derived {
                    lateral: false,
                    subquery: sub,
                    alias: Some(alias),
                    sample: None,
                };
                ControlFlow::Continue(())
            }
            _ => {
                self.fallo = Some(format!(
                    "`{}` lee `{qn}`, una `Table` de un origen, y no un dataset: una vista se \
                     sirve desde lo que se tiene. Léela por un `Dataset` que la copie",
                    self.desde.qname().unwrap_or_default()
                ));
                ControlFlow::Break(())
            }
        }
    }
}
