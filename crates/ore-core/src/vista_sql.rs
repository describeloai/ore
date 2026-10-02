//! **La vista es SQL** (OOS v1alpha14, ADR 0040): lo que se gobierna de una
//! vista se deriva de su consulta, y esto es lo que lo deriva.
//!
//! De un texto SQL en el dialecto de DuckDB, sin motor y sin leer una fila:
//!
//! - **lo que lee**: los nombres del árbol tras un `FROM` o un `JOIN`, dentro de
//!   un `WITH`, de una subconsulta o de un lado de un `UNION` (§3). Un nombre
//!   del `WITH` de la propia consulta no es del árbol. Leer por función
//!   —`read_parquet`, `iceberg_scan`— es [`Fallo::LeePorFuncion`] (`OOS2038`);
//!   las que generan filas sin leer nada (`range`, `generate_series`,
//!   `unnest`) se admiten;
//! - **lo que proyecta**: cada columna de salida con su nombre, y un `*` por las
//!   columnas que su fuente expone —que las dice quien llama, porque son del
//!   árbol y no de la consulta— (§4);
//! - **el linaje por columna** (§5): de qué columnas de sus fuentes sale cada
//!   una, **directa** (tal cual, con otro nombre o sin él) o **derivada** (una
//!   expresión o un agregado sobre ellas); y lo que la consulta **mira sin
//!   devolver** —`WHERE`, la condición de un `JOIN`, `QUALIFY`, una subconsulta
//!   que decide filas— como arista INDIRECT hacia todas. La agrupación, como el
//!   motor desde v1alpha8: una clave proyectada deja su arista hacia los
//!   agregados, una no proyectada hacia todas, y un `HAVING` mira además las
//!   claves;
//! - **los predicados**, clasificados para el canal lateral (§6): los que sólo
//!   revelan pertenencia a una clase —igualdad a un valor, pertenencia a una
//!   lista, ausencia— y los que **ordenan** —un rango, un patrón, una función
//!   aplicada a la columna—. Si una columna lleva etiqueta lo sabe el árbol, no
//!   la consulta: aquí sólo se dice qué mira cada predicado y de qué clase es.
//!
//! Las referencias salen **a un nivel**: a los nombres del árbol que la
//! consulta lee, no a sus raíces. Componer la cadena —una vista que lee otra—
//! es del árbol. Una columna cuyo origen no se puede decidir —sin calificar, con
//! dos fuentes que no dicen sus columnas— se toma de todas (§5: conservador,
//! nunca deja una etiqueta sin llevar), y se dice en [`Consulta::ambiguas`].

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

use sqlparser::ast::{
    BinaryOperator, CastKind, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArguments,
    GroupByExpr, Join, JoinConstraint, JoinOperator, ObjectName, ObjectNamePart, Query, Select,
    SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor, UnaryOperator,
    Value, Visit, Visitor,
};
use sqlparser::dialect::DuckDbDialect;
use sqlparser::parser::Parser;

/// Una columna de una fuente del árbol: el nombre del árbol tal como la
/// consulta lo escribe (sus partes unidas por `.`) y la columna.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ref {
    pub fuente: String,
    pub columna: String,
}

impl Ref {
    fn new(fuente: &str, columna: &str) -> Self {
        Self {
            fuente: fuente.to_string(),
            columna: columna.to_string(),
        }
    }
}

/// Una columna de salida y de dónde sale.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Columna {
    pub nombre: String,
    /// Tal cual, con otro nombre o sin él.
    pub directas: BTreeSet<Ref>,
    /// Leídas por una expresión o un agregado.
    pub derivadas: BTreeSet<Ref>,
    /// INDIRECT sólo hacia esta columna: las claves de grupo hacia un agregado.
    /// Las que van hacia todas están en [`Consulta::indirectas`].
    pub indirectas: BTreeSet<Ref>,
    /// ORE 0051 · **cuándo nunca es nula**. `Some(refs)`: nunca lo es si no lo
    /// es ninguna de `refs` en su fuente —vacío, pase lo que pase: un literal,
    /// un `COUNT`—. `None`: puede serlo. Qué garantiza cada fuente lo sabe el
    /// árbol ([`crate::vistas::nulabilidad_de_vista`]). Conservador: ante la
    /// duda, `None` (v1alpha22 `01-nunca-nula` §4).
    pub exige: Option<BTreeSet<Ref>>,
}

impl Columna {
    fn todas(&self) -> BTreeSet<Ref> {
        self.directas.union(&self.derivadas).cloned().collect()
    }

    fn mas(&mut self, o: Columna) {
        self.directas.extend(o.directas);
        self.derivadas.extend(o.derivadas);
        self.indirectas.extend(o.indirectas);
        // Dos orígenes para la misma columna (un `UNION`, una ambigua): nunca
        // nula sólo si ninguno lo es.
        self.exige = match (self.exige.take(), o.exige) {
            (Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            _ => None,
        };
    }

    /// Lo que era directo pasa a leerse por una expresión.
    fn derivada(self) -> Columna {
        let derivadas = self.todas();
        Columna {
            nombre: self.nombre,
            directas: BTreeSet::new(),
            derivadas,
            indirectas: self.indirectas,
            exige: self.exige,
        }
    }

    /// Una columna de una fuente, tal cual.
    fn de(r: Ref) -> Columna {
        Columna {
            exige: Some([r.clone()].into()),
            directas: [r].into(),
            ..Default::default()
        }
    }
}

/// Qué revela un predicado de lo que mira (§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    /// Igualdad a un valor, pertenencia a una lista de valores o ausencia, o
    /// igualdad entre columnas: sólo pertenencia a una clase.
    Revela,
    /// Un rango, `BETWEEN`, un patrón, o una función aplicada a la columna
    /// antes de compararla: ordena en vez de particionar.
    Ordena,
}

/// Dónde está un predicado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lugar {
    Where,
    Join,
    Qualify,
    Having,
}

/// Un predicado atómico: lo que mira, de qué clase es y dónde está.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicado {
    pub mira: BTreeSet<Ref>,
    pub clase: Clase,
    pub lugar: Lugar,
    /// El texto del predicado, para decirlo en un diagnóstico.
    pub texto: String,
}

/// Lo que se deriva de una consulta.
#[derive(Debug, Clone, Default)]
pub struct Consulta {
    /// Los nombres del árbol que lee, como los escribe.
    pub lee: BTreeSet<String>,
    /// Lo que proyecta, en orden.
    pub columnas: Vec<Columna>,
    /// INDIRECT hacia todas las columnas de salida.
    pub indirectas: BTreeSet<Ref>,
    pub predicados: Vec<Predicado>,
    /// Columnas sin calificar que podían ser de más de una fuente: se toman
    /// de todas.
    pub ambiguas: BTreeSet<String>,
    /// Columnas que no resuelven a ninguna fuente de la consulta.
    pub sin_fuente: BTreeSet<String>,
    /// Fuentes cuyo `*` no se pudo expandir: el árbol no dice sus columnas.
    pub estrellas_sin_expandir: BTreeSet<String>,
}

/// Por qué una consulta no es una vista (§2, §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallo {
    /// No se analiza en el dialecto.
    NoSeAnaliza(String),
    /// No es UNA consulta `SELECT`: varias sentencias, o una que escribe.
    NoEsUnaConsulta(String),
    /// Lee bytes que el árbol no nombra, por una función.
    LeePorFuncion(String),
}

impl Fallo {
    pub fn como_texto(&self) -> String {
        match self {
            Fallo::NoSeAnaliza(m) => {
                format!("la consulta no se analiza en el dialecto duckdb: {m}")
            }
            Fallo::NoEsUnaConsulta(m) => m.clone(),
            Fallo::LeePorFuncion(f) => format!(
                "lee por función (`{f}`), no por nombre: esos bytes no los nombra el árbol, \
                 y no tienen linaje ni conducto"
            ),
        }
    }
}

/// Las funciones que generan filas sin leer nada.
const GENERADORES: &[&str] = &["range", "generate_series", "unnest"];

/// Los agregados: un `HAVING` sobre uno de ellos admite rangos (§6).
const AGREGADOS: &[&str] = &[
    "count",
    "count_star",
    "sum",
    "min",
    "max",
    "avg",
    "mean",
    "median",
    "any_value",
    "arg_max",
    "arg_min",
    "approx_count_distinct",
    "string_agg",
    "list",
    "array_agg",
    "stddev",
    "stddev_pop",
    "stddev_samp",
    "var_pop",
    "var_samp",
    "variance",
    "bool_and",
    "bool_or",
    "first",
    "last",
    "mode",
    "quantile",
    "quantile_cont",
    "quantile_disc",
];

/// Analiza una consulta. `columnas_de` dice las columnas de un nombre del
/// árbol, para expandir un `*`; `None` si no las sabe.
pub fn analizar(
    sql: &str,
    columnas_de: &dyn Fn(&str) -> Option<Vec<String>>,
) -> Result<Consulta, Fallo> {
    let sentencias =
        Parser::parse_sql(&DuckDbDialect {}, sql).map_err(|e| Fallo::NoSeAnaliza(e.to_string()))?;
    let [sentencia] = sentencias.as_slice() else {
        return Err(Fallo::NoEsUnaConsulta(format!(
            "una vista es UNA consulta, y esto son {} sentencias",
            sentencias.len()
        )));
    };
    let Statement::Query(q) = sentencia else {
        return Err(Fallo::NoEsUnaConsulta(
            "una vista es una consulta `SELECT`, y esto no lo es: nada que escriba".into(),
        ));
    };
    let a = Analizador { columnas_de };
    let raiz = Ambito {
        rels: vec![],
        nulas: BTreeSet::new(),
        padre: None,
    };
    let s = a.consulta(q, &raiz, &BTreeMap::new());
    if let Some(f) = s.lectoras.first() {
        return Err(Fallo::LeePorFuncion(f.clone()));
    }
    Ok(Consulta {
        lee: s.lee,
        columnas: s.cols,
        indirectas: s.ind,
        predicados: s.predicados,
        ambiguas: s.ambiguas,
        sin_fuente: s.sin_fuente,
        estrellas_sin_expandir: s.estrellas,
    })
}

/// **Qué filas salen**, sin decir qué columnas: la consulta sin su proyección,
/// escrita de nuevo por el analizador. Dos consultas que dan lo mismo aquí
/// devuelven las mismas filas aunque proyecten otra cosa o estén escritas con
/// otros espacios; si difieren, las filas pueden ser otras (v1alpha14 §9). En
/// un `UNION` la proyección decide las filas, y se compara entera. `None` si
/// no es una consulta.
pub fn filas(sql: &str) -> Option<String> {
    let mut sentencias = Parser::parse_sql(&DuckDbDialect {}, sql).ok()?;
    let [Statement::Query(q)] = sentencias.as_mut_slice() else {
        return None;
    };
    if let SetExpr::Select(s) = q.body.as_mut() {
        s.projection.clear();
    }
    Some(q.to_string())
}

// ─── el análisis ──────────────────────────────────────────────────────────

/// Lo que sale de una consulta o subconsulta mientras se analiza.
#[derive(Debug, Clone, Default)]
struct Salida {
    cols: Vec<Columna>,
    ind: BTreeSet<Ref>,
    lee: BTreeSet<String>,
    lectoras: Vec<String>,
    predicados: Vec<Predicado>,
    ambiguas: BTreeSet<String>,
    sin_fuente: BTreeSet<String>,
    estrellas: BTreeSet<String>,
}

impl Salida {
    /// Lo que una subconsulta aporta a la de fuera, salvo sus columnas.
    fn absorber(&mut self, o: &Salida) {
        self.lee.extend(o.lee.iter().cloned());
        self.lectoras.extend(o.lectoras.iter().cloned());
        self.predicados.extend(o.predicados.iter().cloned());
        self.ambiguas.extend(o.ambiguas.iter().cloned());
        self.sin_fuente.extend(o.sin_fuente.iter().cloned());
        self.estrellas.extend(o.estrellas.iter().cloned());
    }
}

/// Una relación en el `FROM`: un nombre del árbol, o una subconsulta (un
/// `WITH`, una derivada) con su linaje ya hecho.
#[derive(Debug, Clone)]
enum Rel {
    Arbol(String),
    Sub(Salida),
}

struct Ambito<'a> {
    rels: Vec<(String, Rel)>,
    /// ORE 0051: las relaciones (por su posición en `rels`) del lado que genera
    /// nulos de un `LEFT`, `RIGHT` o `FULL JOIN`.
    nulas: BTreeSet<usize>,
    padre: Option<&'a Ambito<'a>>,
}

impl Ambito<'_> {
    /// Lo que sale de una relación del lado que genera nulos puede ser nulo,
    /// aunque su fuente lo garantice.
    fn nula(&self, i: usize, mut c: Columna) -> Columna {
        if self.nulas.contains(&i) {
            c.exige = None;
        }
        c
    }
}

struct Analizador<'f> {
    columnas_de: &'f dyn Fn(&str) -> Option<Vec<String>>,
}

fn nombre(n: &ObjectName) -> Vec<String> {
    n.0.iter()
        .map(|p| match p {
            ObjectNamePart::Identifier(i) => i.value.clone(),
            otra => otra.to_string(),
        })
        .collect()
}

fn igual(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Las columnas que una expresión nombra, sin entrar en sus subconsultas, y
/// las subconsultas aparte.
struct Refs {
    cols: Vec<(Option<String>, String)>,
    subs: Vec<Query>,
    prof: usize,
}

impl Visitor for Refs {
    type Break = ();

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        if self.prof == 0 {
            self.subs.push(q.clone());
        }
        self.prof += 1;
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _q: &Query) -> ControlFlow<()> {
        self.prof -= 1;
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        if self.prof > 0 {
            return ControlFlow::Continue(());
        }
        match e {
            Expr::Identifier(i) => self.cols.push((None, i.value.clone())),
            Expr::CompoundIdentifier(v) if v.len() >= 2 => {
                let calif = v[..v.len() - 1]
                    .iter()
                    .map(|i| i.value.clone())
                    .collect::<Vec<_>>()
                    .join(".");
                self.cols.push((Some(calif), v[v.len() - 1].value.clone()));
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

fn refs_de(e: &Expr) -> Refs {
    let mut r = Refs {
        cols: vec![],
        subs: vec![],
        prof: 0,
    };
    let _ = e.visit(&mut r);
    r
}

/// ORE 0051 · El nombre de una columna tal cual, para cotejar lo que un
/// `WHERE` afirma con lo que se proyecta: `(calificación, columna)`, sin
/// mayúsculas.
fn clave_de(e: &Expr) -> Option<(Option<String>, String)> {
    match e {
        Expr::Nested(i) => clave_de(i),
        Expr::Identifier(i) => Some((None, i.value.to_lowercase())),
        Expr::CompoundIdentifier(v) if v.len() >= 2 => Some((
            Some(
                v[..v.len() - 1]
                    .iter()
                    .map(|i| i.value.to_lowercase())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            v[v.len() - 1].value.to_lowercase(),
        )),
        _ => None,
    }
}

/// ORE 0051 · Las funciones que dan nulo sólo si algún argumento lo es. Lo que
/// no está aquí puede dar nulo por su cuenta —`nullif`, `sqrt(-1)`, un
/// agregado sobre un grupo vacío— y es nulable.
const PROPAGAN: &[&str] = &[
    "upper",
    "lower",
    "ucase",
    "lcase",
    "length",
    "len",
    "char_length",
    "character_length",
    "strlen",
    "trim",
    "ltrim",
    "rtrim",
    "abs",
    "round",
    "floor",
    "ceil",
    "ceiling",
    "sign",
    "md5",
    "sha256",
    "substr",
    "substring",
    "replace",
    "left",
    "right",
    "lpad",
    "rpad",
    "reverse",
    "repeat",
    "concat",
    "date_trunc",
    "date_part",
    "datepart",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "strftime",
];

/// ORE 0051 · Las funciones de ventana que numeran: nunca dan nulo.
const RANGOS: &[&str] = &[
    "row_number",
    "rank",
    "dense_rank",
    "ntile",
    "percent_rank",
    "cume_dist",
];

/// ¿Es la expresión una columna, tal cual?
fn es_columna(e: &Expr) -> bool {
    match e {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) => true,
        Expr::Nested(i) => es_columna(i),
        _ => false,
    }
}

/// ¿Es la expresión un valor escrito, sin columnas?
fn es_valor(e: &Expr) -> bool {
    match e {
        Expr::Value(_) | Expr::TypedString { .. } => true,
        Expr::UnaryOp {
            op: UnaryOperator::Minus | UnaryOperator::Plus,
            expr,
        } => es_valor(expr),
        Expr::Nested(i) => es_valor(i),
        Expr::Cast { expr, .. } => es_valor(expr),
        _ => false,
    }
}

/// ¿Lleva la expresión un agregado?
fn lleva_agregado(e: &Expr) -> bool {
    struct Busca(bool);
    impl Visitor for Busca {
        type Break = ();
        fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
            if let Expr::Function(f) = e
                && let Some(n) = nombre(&f.name).last()
                && AGREGADOS.iter().any(|a| igual(a, n))
                && f.over.is_none()
            {
                self.0 = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
    }
    let mut b = Busca(false);
    let _ = e.visit(&mut b);
    b.0
}

fn restriccion(op: &JoinOperator) -> Option<&JoinConstraint> {
    use JoinOperator::*;
    match op {
        Join(c) | Inner(c) | Left(c) | LeftOuter(c) | Right(c) | RightOuter(c) | FullOuter(c)
        | CrossJoin(c) | Semi(c) | LeftSemi(c) | RightSemi(c) | Anti(c) | LeftAnti(c)
        | RightAnti(c) => Some(c),
        AsOf { constraint, .. } => Some(constraint),
        _ => None,
    }
}

impl Analizador<'_> {
    /// Una columna, calificada o no, en el ámbito: su linaje.
    fn resolver(&self, a: &Ambito, calif: &Option<String>, col: &str, sal: &mut Salida) -> Columna {
        match calif {
            Some(q) => {
                for (i, (alias, rel)) in a.rels.iter().enumerate() {
                    let casa = igual(alias, q)
                        || matches!(rel, Rel::Arbol(n) if igual(n, q)
                            || n.to_lowercase().ends_with(&format!(".{}", q.to_lowercase())));
                    if casa {
                        return a.nula(i, self.de_rel(rel, col));
                    }
                }
            }
            None => {
                if let [(_, rel)] = a.rels.as_slice() {
                    return a.nula(0, self.de_rel(rel, col));
                }
                // Si una subconsulta la tiene, es suya.
                for (i, (_, rel)) in a.rels.iter().enumerate() {
                    if let Rel::Sub(s) = rel
                        && s.cols.iter().any(|c| igual(&c.nombre, col))
                    {
                        return a.nula(i, self.de_rel(rel, col));
                    }
                }
                // Si el árbol dice las columnas de sus fuentes, de la que la tenga.
                let arboles: Vec<(usize, &String)> = a
                    .rels
                    .iter()
                    .enumerate()
                    .filter_map(|(i, (_, r))| match r {
                        Rel::Arbol(n) => Some((i, n)),
                        Rel::Sub(_) => None,
                    })
                    .collect();
                let con: Vec<&(usize, &String)> = arboles
                    .iter()
                    .filter(|(_, n)| {
                        (self.columnas_de)(n).is_some_and(|cs| cs.iter().any(|c| igual(c, col)))
                    })
                    .collect();
                if let [(i, n)] = con.as_slice() {
                    return a.nula(*i, Columna::de(Ref::new(n, col)));
                }
                if !arboles.is_empty() {
                    if arboles.len() > 1 {
                        sal.ambiguas.insert(col.to_string());
                    }
                    // De cuál, no se sabe: puede ser nula (`exige` queda `None`).
                    let mut out = Columna::default();
                    for (_, n) in arboles {
                        out.mas(Columna::de(Ref::new(n, col)));
                    }
                    return out;
                }
            }
        }
        match a.padre {
            Some(p) => self.resolver(p, calif, col, sal),
            None => {
                sal.sin_fuente.insert(match calif {
                    Some(q) => format!("{q}.{col}"),
                    None => col.to_string(),
                });
                Columna::default()
            }
        }
    }

    fn de_rel(&self, rel: &Rel, col: &str) -> Columna {
        match rel {
            Rel::Arbol(n) => Columna::de(Ref::new(n, col)),
            Rel::Sub(s) => s
                .cols
                .iter()
                .find(|c| igual(&c.nombre, col))
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// El linaje de una expresión. Lo que sus subconsultas leen o miran entra
    /// como derivado: el valor depende de ello.
    fn linaje(&self, e: &Expr, a: &Ambito, sal: &mut Salida) -> Columna {
        let r = refs_de(e);
        let mut l = Columna::default();
        for (q, c) in &r.cols {
            l.mas(self.resolver(a, q, c, sal));
        }
        for sq in &r.subs {
            let s = self.consulta(sq, a, &BTreeMap::new());
            sal.absorber(&s);
            for c in &s.cols {
                l.derivadas.extend(c.todas());
                l.derivadas.extend(c.indirectas.iter().cloned());
            }
            l.derivadas.extend(s.ind.iter().cloned());
        }
        if es_columna(e) { l } else { l.derivada() }
    }

    /// Un predicado que decide filas: sus columnas, INDIRECT hacia todas; y sus
    /// átomos, clasificados.
    fn mira(&self, e: &Expr, lugar: Lugar, a: &Ambito, sal: &mut Salida) {
        let l = self.linaje(e, a, sal);
        sal.ind.extend(l.todas());
        sal.ind.extend(l.indirectas);
        self.atomos(e, lugar, a, sal);
    }

    fn atomos(&self, e: &Expr, lugar: Lugar, a: &Ambito, sal: &mut Salida) {
        use BinaryOperator::*;
        let anota = |this: &Self, lados: &[&Expr], clase: Clase, sal: &mut Salida| {
            let mut mira = BTreeSet::new();
            let mut clase = clase;
            for lado in lados {
                let l = this.linaje(lado, a, sal);
                // Una columna que llega derivada —de una expresión en un `WITH`
                // o una subconsulta— es una función aplicada a la de abajo.
                if !l.derivadas.is_empty() {
                    clase = Clase::Ordena;
                }
                mira.extend(l.todas());
            }
            if !mira.is_empty() {
                sal.predicados.push(Predicado {
                    mira,
                    clase,
                    lugar,
                    texto: e.to_string(),
                });
            }
        };
        match e {
            Expr::Nested(i) => self.atomos(i, lugar, a, sal),
            Expr::BinaryOp {
                left,
                op: And | Or,
                right,
            } => {
                self.atomos(left, lugar, a, sal);
                self.atomos(right, lugar, a, sal);
            }
            Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr,
            } => self.atomos(expr, lugar, a, sal),
            _ if lugar == Lugar::Having && lleva_agregado(e) => {
                // Un umbral sobre un agregado no es un canal lateral (§6).
            }
            Expr::BinaryOp {
                left,
                op: Eq | NotEq | Spaceship,
                right,
            } => {
                let (l, r) = (left.as_ref(), right.as_ref());
                if es_columna(l) && (es_valor(r) || es_columna(r)) {
                    anota(self, &[l, r], Clase::Revela, sal);
                } else if es_columna(r) && es_valor(l) {
                    anota(self, &[r], Clase::Revela, sal);
                } else {
                    anota(self, &[l, r], Clase::Ordena, sal);
                }
            }
            Expr::InList { expr, list, .. } if es_columna(expr) && list.iter().all(es_valor) => {
                anota(self, &[expr], Clase::Revela, sal)
            }
            Expr::IsNull(x)
            | Expr::IsNotNull(x)
            | Expr::IsTrue(x)
            | Expr::IsNotTrue(x)
            | Expr::IsFalse(x)
            | Expr::IsNotFalse(x)
                if es_columna(x) =>
            {
                anota(self, &[x], Clase::Revela, sal)
            }
            otro => anota(self, &[otro], Clase::Ordena, sal),
        }
    }

    fn factor(
        &self,
        t: &TableFactor,
        padre: &Ambito,
        ctes: &BTreeMap<String, Salida>,
        sal: &mut Salida,
        rels: &mut Vec<(String, Rel)>,
        nulas: &mut BTreeSet<usize>,
    ) {
        match t {
            TableFactor::Table {
                name, alias, args, ..
            } => {
                let partes = nombre(name);
                let n = partes.join(".");
                let alias_ = alias
                    .as_ref()
                    .map(|a| a.name.value.clone())
                    .unwrap_or_else(|| partes.last().cloned().unwrap_or_default());
                if args.is_some() {
                    if GENERADORES.iter().any(|g| igual(g, &n)) {
                        let cols = alias
                            .iter()
                            .flat_map(|a| &a.columns)
                            .map(|c| Columna {
                                nombre: c.name.value.clone(),
                                ..Default::default()
                            })
                            .collect();
                        rels.push((
                            alias_,
                            Rel::Sub(Salida {
                                cols,
                                ..Default::default()
                            }),
                        ));
                    } else {
                        sal.lectoras.push(n);
                    }
                    return;
                }
                if partes.len() == 1
                    && let Some(s) = ctes.get(&n.to_lowercase())
                {
                    sal.ind.extend(s.ind.iter().cloned());
                    rels.push((alias_, Rel::Sub(s.clone())));
                    return;
                }
                sal.lee.insert(n.clone());
                rels.push((alias_, Rel::Arbol(n)));
            }
            TableFactor::Derived {
                subquery, alias, ..
            } => {
                let mut s = self.consulta(subquery, padre, ctes);
                if let Some(a) = alias {
                    for (c, col) in s.cols.iter_mut().zip(&a.columns) {
                        c.nombre = col.name.value.clone();
                    }
                }
                sal.absorber(&s);
                sal.ind.extend(s.ind.iter().cloned());
                rels.push((
                    alias
                        .as_ref()
                        .map(|a| a.name.value.clone())
                        .unwrap_or_default(),
                    Rel::Sub(s),
                ));
            }
            TableFactor::NestedJoin {
                table_with_joins, ..
            } => {
                self.factor(&table_with_joins.relation, padre, ctes, sal, rels, nulas);
                for j in &table_with_joins.joins {
                    self.union(j, padre, ctes, sal, rels, nulas);
                }
            }
            TableFactor::Function { name, .. } => sal.lectoras.push(nombre(name).join(".")),
            TableFactor::TableFunction { expr, .. } => sal.lectoras.push(expr.to_string()),
            TableFactor::UNNEST { alias, .. } => rels.push((
                alias
                    .as_ref()
                    .map(|a| a.name.value.clone())
                    .unwrap_or_default(),
                Rel::Sub(Salida::default()),
            )),
            otro => sal.lectoras.push(otro.to_string()),
        }
    }

    fn union(
        &self,
        j: &Join,
        padre: &Ambito,
        ctes: &BTreeMap<String, Salida>,
        sal: &mut Salida,
        rels: &mut Vec<(String, Rel)>,
        nulas: &mut BTreeSet<usize>,
    ) {
        let antes = rels.len();
        self.factor(&j.relation, padre, ctes, sal, rels, nulas);
        // ORE 0051: el lado que genera nulos. `ASOF` y `OUTER APPLY`, como un
        // `LEFT`: lo conservador.
        {
            use JoinOperator::*;
            match &j.join_operator {
                Left(_) | LeftOuter(_) | OuterApply | AsOf { .. } => {
                    nulas.extend(antes..rels.len())
                }
                Right(_) | RightOuter(_) => nulas.extend(0..antes),
                FullOuter(_) => nulas.extend(0..rels.len()),
                _ => {}
            }
        }
        let a = Ambito {
            rels: rels.clone(),
            nulas: nulas.clone(),
            padre: Some(padre),
        };
        match restriccion(&j.join_operator) {
            Some(JoinConstraint::On(e)) => self.mira(e, Lugar::Join, &a, sal),
            Some(JoinConstraint::Using(cols)) => {
                for c in cols {
                    let c = nombre(c).join(".");
                    for (_, r) in &a.rels {
                        sal.ind.extend(self.de_rel(r, &c).todas());
                    }
                }
            }
            _ => {}
        }
    }

    /// `nula`: la relación está del lado que genera nulos de un `OUTER JOIN`.
    fn estrella(&self, r: &Rel, nula: bool, sal: &mut Salida) {
        let quiza = |mut c: Columna| {
            if nula {
                c.exige = None;
            }
            c
        };
        match r {
            Rel::Arbol(n) => match (self.columnas_de)(n) {
                Some(cs) => {
                    for c in cs {
                        sal.cols.push(quiza(Columna {
                            nombre: c.clone(),
                            ..Columna::de(Ref::new(n, &c))
                        }));
                    }
                }
                None => {
                    sal.estrellas.insert(n.clone());
                }
            },
            Rel::Sub(s) => sal.cols.extend(s.cols.iter().cloned().map(quiza)),
        }
    }

    /// ORE 0051 · **Cuándo nunca es nula una expresión** (v1alpha22
    /// `01-nunca-nula` §4): el conjunto de columnas de origen que no pueden ser
    /// nulas para que ella no lo sea, o `None` si puede serlo de todas formas.
    /// Lo que no se conoce, `None`: una regla de más miente; una de menos, no.
    fn exige(&self, e: &Expr, a: &Ambito, sal: &mut Salida) -> Option<BTreeSet<Ref>> {
        use BinaryOperator::*;
        let nunca = || Some(BTreeSet::new());
        match e {
            Expr::Identifier(i) => self.resolver(a, &None, &i.value, sal).exige,
            Expr::CompoundIdentifier(v) if v.len() >= 2 => {
                let calif = v[..v.len() - 1]
                    .iter()
                    .map(|i| i.value.clone())
                    .collect::<Vec<_>>()
                    .join(".");
                self.resolver(a, &Some(calif), &v[v.len() - 1].value, sal)
                    .exige
            }
            Expr::Nested(x) => self.exige(x, a, sal),
            Expr::Value(v) => (!matches!(v.value, Value::Null)).then(BTreeSet::new),
            Expr::TypedString(_) => nunca(),
            // Un `CAST` que no cabe falla; un `TRY_CAST`, da nulo.
            Expr::Cast {
                kind: CastKind::Cast | CastKind::DoubleColon,
                expr,
                ..
            } => self.exige(expr, a, sal),
            Expr::UnaryOp {
                op: UnaryOperator::Minus | UnaryOperator::Plus | UnaryOperator::Not,
                expr,
            } => self.exige(expr, a, sal),
            Expr::IsNull(_)
            | Expr::IsNotNull(_)
            | Expr::IsTrue(_)
            | Expr::IsNotTrue(_)
            | Expr::IsFalse(_)
            | Expr::IsNotFalse(_)
            | Expr::IsUnknown(_)
            | Expr::IsNotUnknown(_)
            | Expr::IsDistinctFrom(..)
            | Expr::IsNotDistinctFrom(..)
            | Expr::Exists { .. } => nunca(),
            Expr::BinaryOp { op: Spaceship, .. } => nunca(),
            // Lo que propaga el nulo y nada más. Dividir no: por cero, DuckDB da
            // nulo con `//` y `%` (medido en 1.5.6; con `/` da `inf`, y otras
            // versiones y motores, nulo). Ni lo demás (un `->` de JSON sin la
            // clave).
            Expr::BinaryOp {
                left,
                op:
                    Plus | Minus | Multiply | StringConcat | Gt | Lt | GtEq | LtEq | Eq | NotEq | And
                    | Or | BitwiseOr | BitwiseAnd | BitwiseXor,
                right,
            } => self.todas(&[left, right], a, sal),
            Expr::Between {
                expr, low, high, ..
            } => self.todas(&[expr, low, high], a, sal),
            Expr::InList { expr, list, .. } => {
                let mut es: Vec<&Expr> = vec![expr];
                es.extend(list.iter());
                self.todas(&es, a, sal)
            }
            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                self.todas(&[expr, pattern], a, sal)
            }
            Expr::Extract { expr, .. } => self.exige(expr, a, sal),
            // Sin `ELSE`, lo que no casa es nulo. Con él, nunca nula si no lo es
            // ninguna rama.
            Expr::Case {
                conditions,
                else_result: Some(otro),
                ..
            } => {
                let mut es: Vec<&Expr> = conditions.iter().map(|c| &c.result).collect();
                es.push(otro);
                self.todas(&es, a, sal)
            }
            Expr::Function(f) => self.exige_funcion(f, a, sal),
            _ => None,
        }
    }

    /// Nunca nula si no lo es ninguna de `es`.
    fn todas(&self, es: &[&Expr], a: &Ambito, sal: &mut Salida) -> Option<BTreeSet<Ref>> {
        let mut out = BTreeSet::new();
        for x in es {
            out.extend(self.exige(x, a, sal)?);
        }
        Some(out)
    }

    fn exige_funcion(&self, f: &Function, a: &Ambito, sal: &mut Salida) -> Option<BTreeSet<Ref>> {
        let n = nombre(&f.name).last()?.to_lowercase();
        // `COUNT` cuenta: un grupo vacío es 0. Y el número de una fila, una fila.
        if n == "count" || (f.over.is_some() && RANGOS.contains(&n.as_str())) {
            return Some(BTreeSet::new());
        }
        let args: Vec<&Expr> = match &f.args {
            FunctionArguments::None => vec![],
            FunctionArguments::List(l) => l
                .args
                .iter()
                .map(|x| match x {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Some(e),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?,
            FunctionArguments::Subquery(_) => return None,
        };
        // `COALESCE`: nunca nula si no lo es alguno. Se toma el que menos exige:
        // un literal al final la hace nunca nula, pase lo que pase.
        if matches!(n.as_str(), "coalesce" | "ifnull") {
            return args
                .iter()
                .filter_map(|x| self.exige(x, a, sal))
                .min_by_key(BTreeSet::len);
        }
        if f.over.is_none() && f.filter.is_none() && PROPAGAN.contains(&n.as_str()) {
            return self.todas(&args, a, sal);
        }
        None
    }

    fn select(&self, s: &Select, padre: &Ambito, ctes: &BTreeMap<String, Salida>) -> Salida {
        let mut sal = Salida::default();
        let mut rels = vec![];
        let mut nulas = BTreeSet::new();
        for t in &s.from {
            self.factor(&t.relation, padre, ctes, &mut sal, &mut rels, &mut nulas);
            for j in &t.joins {
                self.union(j, padre, ctes, &mut sal, &mut rels, &mut nulas);
            }
        }
        let a = Ambito {
            rels,
            nulas,
            padre: Some(padre),
        };
        // ORE 0051. Un `ROLLUP`, un `CUBE` o unos `GROUPING SETS` añaden filas
        // de subtotal con la clave nula: ahí nada de la clave se garantiza.
        let subtotales = match &s.group_by {
            GroupByExpr::Expressions(es, m) => {
                !m.is_empty()
                    || es.iter().any(|e| {
                        matches!(e, Expr::Rollup(_) | Expr::Cube(_) | Expr::GroupingSets(_))
                    })
            }
            GroupByExpr::All(m) => !m.is_empty(),
        };
        // Y lo que el `WHERE` afirma no nulo —un `IS NOT NULL`, una comparación
        // que tiene que ser verdad—, por su nombre y por su origen.
        let (afirmadas, de_origen) = match (&s.selection, subtotales) {
            (Some(w), false) => self.afirmadas(w, &a, &mut sal),
            _ => Default::default(),
        };
        let nunca_nula = |this: &Self, e: &Expr, sal: &mut Salida| {
            if clave_de(e).is_some_and(|k| afirmadas.contains(&k)) {
                return Some(BTreeSet::new());
            }
            this.exige(e, &a, sal)
        };
        for item in &s.projection {
            match item {
                SelectItem::UnnamedExpr(e) => {
                    let n = match e {
                        Expr::Identifier(i) => i.value.clone(),
                        Expr::CompoundIdentifier(v) => v[v.len() - 1].value.clone(),
                        otra => otra.to_string(),
                    };
                    let l = self.linaje(e, &a, &mut sal);
                    let exige = nunca_nula(self, e, &mut sal);
                    sal.cols.push(Columna {
                        nombre: n,
                        exige,
                        ..l
                    });
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let l = self.linaje(expr, &a, &mut sal);
                    let exige = nunca_nula(self, expr, &mut sal);
                    sal.cols.push(Columna {
                        nombre: alias.value.clone(),
                        exige,
                        ..l
                    });
                }
                SelectItem::Wildcard(_) => {
                    for (i, (_, r)) in a.rels.iter().enumerate() {
                        self.estrella(r, a.nulas.contains(&i), &mut sal);
                    }
                }
                SelectItem::QualifiedWildcard(
                    SelectItemQualifiedWildcardKind::ObjectName(n),
                    _,
                ) => {
                    let q = nombre(n).join(".");
                    if let Some((i, (_, r))) = a.rels.iter().enumerate().find(|(_, (al, r))| {
                        igual(al, &q) || matches!(r, Rel::Arbol(t) if igual(t, &q))
                    }) {
                        self.estrella(r, a.nulas.contains(&i), &mut sal);
                    }
                }
                otro => sal.cols.push(Columna {
                    nombre: otro.to_string(),
                    ..Default::default()
                }),
            }
        }
        for c in sal.cols.iter_mut() {
            match &mut c.exige {
                // Subtotales: la clave puede salir nula; lo que no depende de
                // ninguna columna (un literal, un `COUNT`) no.
                Some(r) if subtotales && !r.is_empty() => c.exige = None,
                Some(r) => r.retain(|x| !de_origen.contains(x)),
                None => {}
            }
        }
        if let Some(w) = &s.selection {
            self.mira(w, Lugar::Where, &a, &mut sal);
        }
        // Una clave de grupo decide cuántas filas se juntan: su arista va hacia
        // los agregados; hacia todas sólo si no se proyecta, porque entonces
        // decide filas que no se ven. Como el motor desde v1alpha8.
        let claves: Vec<&Expr> = match &s.group_by {
            GroupByExpr::Expressions(es, _) => es.iter().collect(),
            GroupByExpr::All(_) => vec![],
        };
        if !claves.is_empty() {
            let mut refs: BTreeSet<Ref> = BTreeSet::new();
            for e in &claves {
                refs.extend(self.linaje(e, &a, &mut sal).todas());
            }
            let es_clave = |c: &Columna| {
                c.derivadas.is_empty() && !c.directas.is_empty() && c.directas.is_subset(&refs)
            };
            let proyectadas: BTreeSet<Ref> = sal
                .cols
                .iter()
                .filter(|c| es_clave(c))
                .flat_map(|c| c.directas.iter().cloned())
                .collect();
            for r in &refs {
                if proyectadas.contains(r) {
                    for c in sal.cols.iter_mut() {
                        if !es_clave(c) {
                            c.indirectas.insert(r.clone());
                        }
                    }
                } else {
                    sal.ind.insert(r.clone());
                }
            }
        }
        // Un `HAVING` recorta por un agregado, y el agregado sale de sus grupos.
        if let Some(h) = &s.having {
            self.mira(h, Lugar::Having, &a, &mut sal);
            for e in &claves {
                let l = self.linaje(e, &a, &mut sal);
                sal.ind.extend(l.todas());
            }
        }
        if let Some(q) = &s.qualify {
            self.mira(q, Lugar::Qualify, &a, &mut sal);
        }
        sal
    }

    /// ORE 0051 · **Lo que un `WHERE` afirma no nulo**: en cada conjunción, la
    /// columna de un `IS NOT NULL`, y las de una comparación, un `IN`, un
    /// `BETWEEN` o un `LIKE`, que no pueden ser verdad con un nulo. Por su
    /// nombre en la consulta, y por su columna de origen si se sabe.
    fn afirmadas(
        &self,
        w: &Expr,
        a: &Ambito,
        sal: &mut Salida,
    ) -> (BTreeSet<(Option<String>, String)>, BTreeSet<Ref>) {
        use BinaryOperator::*;
        let mut nombres = BTreeSet::new();
        let mut origen = BTreeSet::new();
        let mut pila = vec![w];
        let mut cols: Vec<&Expr> = vec![];
        while let Some(e) = pila.pop() {
            match e {
                Expr::Nested(x) => pila.push(x),
                Expr::BinaryOp {
                    left,
                    op: And,
                    right,
                } => {
                    pila.push(left);
                    pila.push(right);
                }
                Expr::IsNotNull(x) => cols.push(x),
                Expr::BinaryOp {
                    left,
                    op: Eq | NotEq | Lt | Gt | LtEq | GtEq,
                    right,
                } => {
                    cols.push(left);
                    cols.push(right);
                }
                Expr::InList { expr, .. }
                | Expr::Between { expr, .. }
                | Expr::Like { expr, .. }
                | Expr::ILike { expr, .. } => cols.push(expr),
                _ => {}
            }
        }
        for c in cols {
            let Some(k) = clave_de(c) else { continue };
            if let Some(r) = self.exige(c, a, sal)
                && r.len() == 1
            {
                origen.extend(r);
            }
            nombres.insert(k);
        }
        (nombres, origen)
    }

    fn cuerpo(&self, b: &SetExpr, padre: &Ambito, ctes: &BTreeMap<String, Salida>) -> Salida {
        match b {
            SetExpr::Select(s) => self.select(s, padre, ctes),
            SetExpr::Query(q) => self.consulta(q, padre, ctes),
            SetExpr::SetOperation { left, right, .. } => {
                let mut l = self.cuerpo(left, padre, ctes);
                let r = self.cuerpo(right, padre, ctes);
                // Las columnas de un `UNION` se emparejan por posición y se
                // llaman como las del primer lado.
                for (cl, cr) in l.cols.iter_mut().zip(r.cols.iter().cloned()) {
                    let nombre = std::mem::take(&mut cl.nombre);
                    cl.mas(cr);
                    cl.nombre = nombre;
                }
                l.absorber(&r);
                l.ind.extend(r.ind);
                l
            }
            SetExpr::Values(_) => Salida::default(),
            otro => Salida {
                lectoras: vec![otro.to_string()],
                ..Default::default()
            },
        }
    }

    fn consulta(&self, q: &Query, padre: &Ambito, ctes: &BTreeMap<String, Salida>) -> Salida {
        let mut ctes = ctes.clone();
        let mut de_with = Salida::default();
        if let Some(w) = &q.with {
            for c in &w.cte_tables {
                let mut s = self.consulta(&c.query, padre, &ctes);
                for (col, alias) in s.cols.iter_mut().zip(&c.alias.columns) {
                    col.nombre = alias.name.value.clone();
                }
                de_with.absorber(&s);
                ctes.insert(c.alias.name.value.to_lowercase(), s);
            }
        }
        let mut s = self.cuerpo(&q.body, padre, &ctes);
        s.absorber(&de_with);
        // Un `LIMIT` decide qué filas salen por el orden: lo que ordena, mira.
        if q.limit_clause.is_some()
            && let Some(ob) = &q.order_by
            && let sqlparser::ast::OrderByKind::Expressions(es) = &ob.kind
        {
            for e in es {
                // El orden nombra columnas de la salida o de la fuente.
                let r = refs_de(&e.expr);
                for (_, col) in &r.cols {
                    if let Some(c) = s.cols.iter().find(|c| igual(&c.nombre, col)) {
                        let (t, i) = (c.todas(), c.indirectas.clone());
                        s.ind.extend(t);
                        s.ind.extend(i);
                    }
                }
            }
        }
        s
    }
}

/// v1alpha17 (`04-la-coleccion-como-listado` §4). **Si la consulta ordena o
/// agrupa por la referencia** —`_item`, a secas o calificada—: una referencia no
/// tiene orden. `_item.path` sí se ordena: es un campo, no la referencia. Mira
/// la consulta, sus `WITH`, sus subconsultas de `FROM` y los dos lados de un
/// `UNION`.
pub fn ordena_o_agrupa_por_la_referencia(sql: &str) -> bool {
    let Ok(sentencias) = Parser::parse_sql(&DuckDbDialect {}, sql) else {
        return false;
    };
    fn es_la_referencia(e: &Expr) -> bool {
        match e {
            Expr::Identifier(i) => i.value.eq_ignore_ascii_case("_item"),
            Expr::CompoundIdentifier(p) => p
                .last()
                .is_some_and(|i| i.value.eq_ignore_ascii_case("_item")),
            Expr::Nested(x) => es_la_referencia(x),
            _ => false,
        }
    }
    fn consulta(q: &Query) -> bool {
        if let Some(w) = &q.with
            && w.cte_tables.iter().any(|c| consulta(&c.query))
        {
            return true;
        }
        if let Some(ob) = &q.order_by
            && let sqlparser::ast::OrderByKind::Expressions(es) = &ob.kind
            && es.iter().any(|e| es_la_referencia(&e.expr))
        {
            return true;
        }
        cuerpo(&q.body)
    }
    fn cuerpo(b: &SetExpr) -> bool {
        match b {
            SetExpr::Select(s) => {
                let agrupa = matches!(
                    &s.group_by,
                    GroupByExpr::Expressions(es, _) if es.iter().any(es_la_referencia)
                );
                agrupa
                    || s.from.iter().any(|t| {
                        std::iter::once(&t.relation)
                            .chain(t.joins.iter().map(|j| &j.relation))
                            .any(|r| matches!(r, TableFactor::Derived { subquery, .. } if consulta(subquery)))
                    })
            }
            SetExpr::Query(q) => consulta(q),
            SetExpr::SetOperation { left, right, .. } => cuerpo(left) || cuerpo(right),
            _ => false,
        }
    }
    sentencias
        .iter()
        .any(|s| matches!(s, Statement::Query(q) if consulta(q)))
}

#[cfg(test)]
mod pruebas_de_la_referencia {
    use super::ordena_o_agrupa_por_la_referencia as ordena;

    /// v1alpha17 (`04` §4): la referencia no tiene orden; sus campos sí.
    #[test]
    fn no_se_ordena_ni_se_agrupa_por_la_referencia() {
        assert!(ordena("SELECT _item FROM c ORDER BY _item"));
        assert!(ordena("SELECT c._item FROM c ORDER BY c._item DESC"));
        assert!(ordena("SELECT _item, count(*) FROM c GROUP BY _item"));
        assert!(ordena(
            "WITH x AS (SELECT _item FROM c ORDER BY _item) SELECT * FROM x"
        ));
        assert!(ordena(
            "SELECT * FROM (SELECT _item FROM c GROUP BY _item) t"
        ));
        assert!(ordena(
            "SELECT path FROM a UNION ALL SELECT path FROM b GROUP BY _item"
        ));
        assert!(!ordena("SELECT _item, path FROM c ORDER BY path"));
        assert!(!ordena("SELECT _item FROM c ORDER BY _item.path"));
        assert!(!ordena(
            "SELECT _item.digest, count(*) FROM c GROUP BY _item.digest"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(f: &str, c: &str) -> Ref {
        Ref::new(f, c)
    }

    fn sin_arbol(_: &str) -> Option<Vec<String>> {
        None
    }

    fn ok(sql: &str) -> Consulta {
        analizar(sql, &sin_arbol).unwrap_or_else(|e| panic!("{sql}: {e:?}"))
    }

    fn col<'a>(c: &'a Consulta, n: &str) -> &'a Columna {
        c.columnas
            .iter()
            .find(|x| x.nombre == n)
            .unwrap_or_else(|| panic!("sin columna {n}: {c:?}"))
    }

    fn nombres(c: &Consulta) -> Vec<&str> {
        c.columnas.iter().map(|x| x.nombre.as_str()).collect()
    }

    const P: &str = "ventas.s.pedidos";
    const C: &str = "ventas.s.clientes";

    #[test]
    fn renombrar_es_directo() {
        let c = ok("select id as pedido, total as importe from ventas.s.pedidos");
        assert_eq!(nombres(&c), ["pedido", "importe"]);
        assert_eq!(col(&c, "importe").directas, [r(P, "total")].into());
        assert!(c.indirectas.is_empty());
        assert_eq!(c.lee, [P.to_string()].into());
    }

    #[test]
    fn una_expresion_es_derivada() {
        let c = ok("select id, total * 1.21 as con_iva, upper(estado) from ventas.s.pedidos");
        assert!(col(&c, "con_iva").directas.is_empty());
        assert_eq!(col(&c, "con_iva").derivadas, [r(P, "total")].into());
        assert_eq!(nombres(&c)[2], "upper(estado)");
    }

    #[test]
    fn el_where_deja_indirect_hacia_todas() {
        let c = ok("select id from ventas.s.pedidos where pais = 'ES' and estado in ('a', 'b')");
        assert_eq!(c.indirectas, [r(P, "pais"), r(P, "estado")].into());
        assert!(c.predicados.iter().all(|p| p.clase == Clase::Revela));
        assert_eq!(c.predicados.len(), 2);
    }

    #[test]
    fn la_agrupacion_como_el_motor() {
        let c =
            ok("select pais, borrado, count(*) as n from ventas.s.pedidos group by pais, borrado");
        assert!(col(&c, "pais").indirectas.is_empty());
        assert!(col(&c, "borrado").indirectas.is_empty());
        assert_eq!(
            col(&c, "n").indirectas,
            [r(P, "pais"), r(P, "borrado")].into()
        );
        assert!(c.indirectas.is_empty());
        // Una clave que no se proyecta decide filas que no se ven.
        let c = ok("select pais from ventas.s.pedidos group by pais, borrado");
        assert_eq!(c.indirectas, [r(P, "borrado")].into());
    }

    #[test]
    fn el_having_mira_las_claves_y_admite_un_umbral() {
        let c = ok(
            "select pais, count(*) as n from ventas.s.pedidos group by pais having count(*) >= 8",
        );
        assert_eq!(c.indirectas, [r(P, "pais")].into());
        assert!(c.predicados.is_empty(), "{:?}", c.predicados);
    }

    #[test]
    fn el_canal_lateral_se_clasifica() {
        let clase = |w: &str| {
            let c = ok(&format!("select id from ventas.s.pedidos where {w}"));
            c.predicados.iter().map(|p| p.clase).collect::<Vec<_>>()
        };
        assert_eq!(clase("dni = 'x'"), [Clase::Revela]);
        assert_eq!(clase("'x' = dni"), [Clase::Revela]);
        assert_eq!(clase("dni in ('a', 'b')"), [Clase::Revela]);
        assert_eq!(clase("dni not in ('a')"), [Clase::Revela]);
        assert_eq!(clase("dni is not null"), [Clase::Revela]);
        assert_eq!(clase("not (dni is null)"), [Clase::Revela]);
        assert_eq!(clase("dni > 'M'"), [Clase::Ordena]);
        assert_eq!(clase("dni between 'a' and 'b'"), [Clase::Ordena]);
        assert_eq!(clase("dni like '1%'"), [Clase::Ordena]);
        assert_eq!(clase("substr(dni, 1, 1) = '1'"), [Clase::Ordena]);
        assert_eq!(
            clase("pais = 'ES' or total > 5"),
            [Clase::Revela, Clase::Ordena]
        );
    }

    #[test]
    fn una_columna_derivada_en_un_with_ordena() {
        let c =
            ok("with x as (select upper(dni) as d from ventas.s.p) select d from x where d = 'A'");
        let p = &c.predicados[0];
        assert_eq!(p.clase, Clase::Ordena);
        assert_eq!(p.mira, [r("ventas.s.p", "dni")].into());
    }

    #[test]
    fn el_join_une_por_igualdad_entre_columnas() {
        let c = ok(
            "select p.id, c.nombre from ventas.s.pedidos p join ventas.s.clientes c on c.id = p.cliente_id",
        );
        assert_eq!(col(&c, "nombre").directas, [r(C, "nombre")].into());
        assert_eq!(c.indirectas, [r(C, "id"), r(P, "cliente_id")].into());
        assert_eq!(c.predicados[0].clase, Clase::Revela);
        assert_eq!(c.predicados[0].lugar, Lugar::Join);
    }

    #[test]
    fn un_with_no_es_del_arbol() {
        let c = ok(
            "with grandes as (select * from ventas.s.pedidos where total > 100) select pais, count(*) as n from grandes group by pais",
        );
        assert_eq!(c.lee, [P.to_string()].into());
        assert_eq!(c.estrellas_sin_expandir, [P.to_string()].into());
    }

    #[test]
    fn la_estrella_se_expande_con_el_arbol() {
        let arbol = |n: &str| (n == C).then(|| vec!["id".to_string(), "segmento".to_string()]);
        let c = analizar(
            "select * from ventas.s.clientes where segmento = 'pyme'",
            &arbol,
        )
        .unwrap();
        assert_eq!(nombres(&c), ["id", "segmento"]);
        assert_eq!(col(&c, "segmento").directas, [r(C, "segmento")].into());
    }

    #[test]
    fn sin_calificar_con_dos_fuentes_decide_el_arbol_o_se_toma_de_todas() {
        let sql =
            "select nombre from ventas.s.pedidos p join ventas.s.clientes c on c.id = p.cliente_id";
        let c = ok(sql);
        assert_eq!(
            col(&c, "nombre").directas,
            [r(P, "nombre"), r(C, "nombre")].into()
        );
        assert_eq!(c.ambiguas, ["nombre".to_string()].into());
        let arbol = |n: &str| (n == C).then(|| vec!["id".to_string(), "nombre".to_string()]);
        let c = analizar(sql, &arbol).unwrap();
        assert_eq!(col(&c, "nombre").directas, [r(C, "nombre")].into());
        assert!(c.ambiguas.is_empty());
    }

    #[test]
    fn una_subconsulta_en_el_where_mira_lo_suyo() {
        let c = ok(
            "select id from ventas.s.clientes where id in (select cliente_id from ventas.s.pedidos where total > 500)",
        );
        assert_eq!(c.lee, [C.to_string(), P.to_string()].into());
        assert!(c.indirectas.contains(&r(P, "cliente_id")));
        assert!(c.indirectas.contains(&r(P, "total")));
    }

    #[test]
    fn la_union_empareja_por_posicion() {
        let c = ok(
            "select id, pais from ventas.s.pedidos union all select id, region from ventas.s.clientes",
        );
        assert_eq!(nombres(&c), ["id", "pais"]);
        assert_eq!(
            col(&c, "pais").directas,
            [r(P, "pais"), r(C, "region")].into()
        );
    }

    #[test]
    fn la_ventana_y_el_qualify() {
        let c = ok(
            "select id, rank() over (partition by pais order by total desc) as puesto from ventas.s.pedidos qualify puesto = 1",
        );
        assert_eq!(
            col(&c, "puesto").derivadas,
            [r(P, "pais"), r(P, "total")].into()
        );
    }

    #[test]
    fn el_limit_mira_por_lo_que_ordena() {
        let c = ok("select id, total from ventas.s.pedidos order by total desc limit 10");
        assert_eq!(c.indirectas, [r(P, "total")].into());
        let c = ok("select id, total from ventas.s.pedidos order by total desc");
        assert!(c.indirectas.is_empty());
    }

    #[test]
    fn se_lee_por_nombre_nunca_por_funcion() {
        assert!(matches!(
            analizar("select * from read_parquet('s3://x/*.parquet')", &sin_arbol),
            Err(Fallo::LeePorFuncion(f)) if f == "read_parquet"
        ));
        assert!(matches!(
            analizar(
                "select * from ventas.s.p join iceberg_scan('x') i on true",
                &sin_arbol
            ),
            Err(Fallo::LeePorFuncion(_))
        ));
        let c = ok("select i as dia from range(7) as t(i)");
        assert!(c.lee.is_empty());
        assert_eq!(nombres(&c), ["dia"]);
    }

    #[test]
    fn una_vista_es_una_consulta() {
        assert!(matches!(
            analizar("select 1; select 2", &sin_arbol),
            Err(Fallo::NoEsUnaConsulta(_))
        ));
        assert!(matches!(
            analizar("insert into a.b select 1", &sin_arbol),
            Err(Fallo::NoEsUnaConsulta(_))
        ));
        assert!(matches!(
            analizar("select from where", &sin_arbol),
            Err(Fallo::NoSeAnaliza(_))
        ));
    }

    #[test]
    fn una_columna_sin_fuente_se_dice() {
        let c = ok("select x.id from ventas.s.pedidos p");
        assert_eq!(c.sin_fuente, ["x.id".to_string()].into());
    }

    #[test]
    fn las_filas_no_dependen_de_la_proyeccion_ni_de_los_espacios() {
        let a = filas("select id, pais from v.s.p where pais in ('ES', 'PT')").unwrap();
        let b = filas(
            "SELECT  id
FROM v.s.p WHERE pais IN ('ES', 'PT')",
        )
        .unwrap();
        assert_eq!(a, b);
        let c = filas("select id from v.s.p where pais in ('ES')").unwrap();
        assert_ne!(a, c);
        // En un UNION la proyección decide qué filas hay.
        assert_ne!(
            filas("select id from v.s.a union select id from v.s.b"),
            filas("select pais from v.s.a union select id from v.s.b")
        );
        assert!(filas("select 1; select 2").is_none());
    }

    /// El corpus de `medida-la-vista-sql.py`: lo que la gente escribe en un
    /// `CREATE VIEW`. Todas se analizan salvo `PIVOT`, que no es un `SELECT`.
    #[test]
    fn el_corpus_se_analiza() {
        let corpus = [
            "select id, pais, total from ventas.s.pedidos",
            "select distinct pais from ventas.s.clientes",
            "select id, total from ventas.s.pedidos where fecha >= date '2026-01-01'",
            "select id, email from ventas.s.clientes where email like '%@empresa.com'",
            "select id, case when total > 100 then 'grande' else 'normal' end as tamano from ventas.s.pedidos",
            "select id, upper(nombre) as nombre, date_trunc('month', alta) as mes from ventas.s.clientes",
            "select id, cast(total as double) as total from ventas.s.pedidos",
            "select c.id, c.nombre, count(p.id) as pedidos, coalesce(sum(p.total), 0) as gastado from ventas.s.clientes c left join ventas.s.pedidos p on p.cliente_id = c.id group by c.id, c.nombre",
            "select p.id, c.segmento, sum(l.cantidad * l.precio) as bruto from ventas.s.pedidos p join ventas.s.clientes c on c.id = p.cliente_id join ventas.s.lineas l on l.pedido_id = p.id group by p.id, c.segmento",
            "select * from ventas.s.pedidos qualify row_number() over (partition by cliente_id order by fecha desc) = 1",
        ];
        for sql in corpus {
            let c = ok(sql);
            assert!(c.sin_fuente.is_empty(), "{sql}: {:?}", c.sin_fuente);
        }
        let c = ok(corpus[8]);
        assert_eq!(
            col(&c, "bruto").derivadas,
            [
                r("ventas.s.lineas", "cantidad"),
                r("ventas.s.lineas", "precio")
            ]
            .into()
        );
        assert_eq!(
            col(&c, "bruto").indirectas,
            [r(P, "id"), r(C, "segmento")].into()
        );
        assert!(
            analizar(
                "pivot ventas.s.pedidos on pais using sum(total) group by estado",
                &sin_arbol
            )
            .is_err()
        );
    }

    // ── ORE 0051 · cuándo nunca es nula ─────────────────────────────────────

    /// `Some(refs)` de la columna `n`, o `None`.
    fn exige(c: &Consulta, n: &str) -> Option<BTreeSet<Ref>> {
        col(c, n).exige.clone()
    }

    fn de(refs: &[(&str, &str)]) -> Option<BTreeSet<Ref>> {
        Some(refs.iter().map(|(f, c)| r(f, c)).collect())
    }

    #[test]
    fn leer_tal_cual_o_renombrar_exige_su_columna() {
        let c = ok("select id, total as importe from ventas.s.pedidos");
        assert_eq!(exige(&c, "id"), de(&[(P, "id")]));
        assert_eq!(exige(&c, "importe"), de(&[(P, "total")]));
    }

    #[test]
    fn un_literal_nunca_es_nulo_y_null_si() {
        let c = ok("select 1 as uno, 'a' as letra, null as nada from ventas.s.pedidos");
        assert_eq!(exige(&c, "uno"), de(&[]));
        assert_eq!(exige(&c, "letra"), de(&[]));
        assert_eq!(exige(&c, "nada"), None);
    }

    #[test]
    fn el_lado_que_genera_nulos_de_un_outer_join_es_nulable() {
        let izq = "select p.id, c.nombre from ventas.s.pedidos p \
                   left join ventas.s.clientes c on p.cliente = c.id";
        let c = ok(izq);
        assert_eq!(exige(&c, "id"), de(&[(P, "id")]), "el lado que se conserva");
        assert_eq!(exige(&c, "nombre"), None);
        let c = ok(&izq.replace("left join", "right join"));
        assert_eq!(exige(&c, "id"), None);
        assert_eq!(exige(&c, "nombre"), de(&[(C, "nombre")]));
        let c = ok(&izq.replace("left join", "full join"));
        assert_eq!((exige(&c, "id"), exige(&c, "nombre")), (None, None));
        let c = ok(&izq.replace("left join", "join"));
        assert_eq!(exige(&c, "id"), de(&[(P, "id")]));
        assert_eq!(exige(&c, "nombre"), de(&[(C, "nombre")]));
    }

    #[test]
    fn un_case_sin_else_es_nulable_y_con_else_exige_sus_ramas() {
        let c = ok("select case when total > 10 then 'alto' end as sin, \
                    case when total > 10 then estado else 'bajo' end as con \
                    from ventas.s.pedidos");
        assert_eq!(exige(&c, "sin"), None);
        assert_eq!(exige(&c, "con"), de(&[(P, "estado")]));
    }

    #[test]
    fn coalesce_basta_con_uno() {
        let c = ok(
            "select coalesce(nota, 'sin nota') as a, coalesce(nota, estado) as b \
                    from ventas.s.pedidos",
        );
        assert_eq!(exige(&c, "a"), de(&[]));
        assert_eq!(exige(&c, "b"), de(&[(P, "nota")]));
    }

    #[test]
    fn count_nunca_es_nulo_y_los_demas_agregados_si() {
        let c = ok(
            "select estado, count(*) as n, count(nota) as m, sum(total) as s, \
                    max(total) as x from ventas.s.pedidos group by estado",
        );
        assert_eq!(exige(&c, "estado"), de(&[(P, "estado")]));
        assert_eq!(exige(&c, "n"), de(&[]));
        assert_eq!(exige(&c, "m"), de(&[]));
        assert_eq!((exige(&c, "s"), exige(&c, "x")), (None, None));
    }

    #[test]
    fn lo_que_el_where_afirma_no_es_nulo() {
        let c = ok("select nota, total from ventas.s.pedidos where nota is not null and total > 0");
        assert_eq!(exige(&c, "nota"), de(&[]));
        assert_eq!(exige(&c, "total"), de(&[]));
        // Incluso del lado que genera nulos: la fila sin pareja no pasa.
        let c = ok("select c.nombre from ventas.s.pedidos p \
                    left join ventas.s.clientes c on p.cliente = c.id \
                    where c.nombre is not null");
        assert_eq!(exige(&c, "nombre"), de(&[]));
        // Un `OR` no afirma ninguno de los dos.
        let c = ok("select nota from ventas.s.pedidos where nota is not null or total > 0");
        assert_eq!(exige(&c, "nota"), de(&[(P, "nota")]));
    }

    #[test]
    fn una_funcion_propaga_o_no_se_sabe() {
        let c = ok(
            "select upper(estado) as u, total / 2 as mitad, total * 2 as doble, \
                    nullif(estado, 'x') as n, rara(estado) as r, try_cast(total as int) as t, \
                    cast(total as int) as ci, total is null as es from ventas.s.pedidos",
        );
        assert_eq!(exige(&c, "u"), de(&[(P, "estado")]));
        assert_eq!(exige(&c, "mitad"), None, "dividir por cero puede dar nulo");
        assert_eq!(exige(&c, "doble"), de(&[(P, "total")]));
        assert_eq!((exige(&c, "n"), exige(&c, "r")), (None, None));
        assert_eq!(exige(&c, "t"), None);
        assert_eq!(exige(&c, "ci"), de(&[(P, "total")]));
        assert_eq!(exige(&c, "es"), de(&[]));
    }

    #[test]
    fn un_union_exige_los_dos_lados() {
        let c = ok("select id from ventas.s.pedidos union all select id from ventas.s.clientes");
        assert_eq!(exige(&c, "id"), de(&[(P, "id"), (C, "id")]));
        let c = ok("select id from ventas.s.pedidos union all select null from ventas.s.clientes");
        assert_eq!(exige(&c, "id"), None);
    }

    #[test]
    fn un_rollup_deja_la_clave_nulable() {
        let c = ok("select estado, count(*) as n from ventas.s.pedidos \
                    where estado is not null group by rollup (estado)");
        assert_eq!(exige(&c, "estado"), None);
        assert_eq!(exige(&c, "n"), de(&[]));
    }

    #[test]
    fn una_subconsulta_y_un_with_llevan_lo_suyo() {
        let c = ok("with t as (select id, null as x from ventas.s.pedidos) \
                    select t.id, t.x, d.n from t \
                    join (select count(*) as n from ventas.s.clientes) d on true");
        assert_eq!(exige(&c, "id"), de(&[(P, "id")]));
        assert_eq!(exige(&c, "x"), None);
        assert_eq!(exige(&c, "n"), de(&[]));
    }
}
