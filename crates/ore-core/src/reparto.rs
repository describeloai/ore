//! **El reparto** (ADR 0053 F5, OOS v1alpha24 `01` §3–§5): de una sentencia SQL,
//! por cada `Table` de un origen que lee, **qué se le pide al origen** —columnas,
//! filtros, `limit`— y qué queda para el motor; y si el coste que la tabla
//! declara y el gobierno la dejan leer. No abre nada: lee el árbol.
//!
//! # Las reglas
//!
//! 1. **Columnas**: las que la sentencia usa de la tabla en cualquier sitio
//!    —proyección, `WHERE`, `ON`, `GROUP BY`, `HAVING`, `ORDER BY`—; `*`, todas.
//! 2. **Filtros**: cada conjunción del `WHERE` (y de un `ON`) que toca **una**
//!    tabla, con la forma `columna op literal` (`=`, `<>`, `<`, `<=`, `>`, `>=`,
//!    `IN (…)`, `IS [NOT] NULL`, `LIKE`, `BETWEEN`) y cuya familia admiten la
//!    tabla (`reads.predicatePushdown`) **y** el conector. Un `OR`, no.
//! 3. **Lo empujado se vuelve a evaluar** en el motor, siempre (decisión C): el
//!    reparto sólo quita filas que no llegarían al resultado, nunca lo decide. Por
//!    eso un filtro del `WHERE` que rechaza el nulo se empuja también al lado de
//!    un `LEFT JOIN` que puede quedar a nulo: la fila que falta vuelve a nulo, y
//!    el filtro, evaluado otra vez, la quita igual. `IS NULL` ahí, no.
//! 4. **`limit`**: sólo si nada de lo que queda en el motor puede quitar filas
//!    antes de él (spec §3): una sola relación, sin agregar, `DISTINCT`, ventanas
//!    ni `GROUP BY`, y todo el `WHERE` empujado. Con `OFFSET k`, `n + k`. Con
//!    `ORDER BY`, sólo si sus columnas son de la tabla y el conector ordena.
//! 5. **Vistas, `WITH` y subconsultas**: lo de fuera las atraviesa si son una
//!    **proyección limpia** —columnas de una sola relación, renombradas o no, sin
//!    agregar, `DISTINCT` ni `LIMIT`—; lo suyo, se suma. Es el caso
//!    `a-view-carries-the-filter-to-its-table` de la spec.
//! 6. **Coste y gobierno**, por tabla y con lo empujado: `OOS2044`, `OOS2045`,
//!    el presupuesto de `expensive`, y [`crate::flow::lectura_del_origen`].
//!
//! # Lo decidido (F5, con el usuario)
//!
//! - **A · Una lectura por tabla y sentencia.** El puesto registra cada nombre
//!   como una vista y no reescribe la sentencia: si la tabla sale dos veces, se
//!   lee una, con la unión de columnas y **sólo los filtros comunes** a todas
//!   sus apariciones; `limit`, sólo con una.
//! - **B · Lo que no se entiende se lee sin empujar.** sqlparser no analiza 7 de
//!   52 sentencias de DuckDB (`PIVOT`, `ASOF`…): sus tablas —las que el
//!   tokenizador ve, también dentro de las vistas— se leen enteras, con las
//!   mismas reglas de coste (`forbidden` → `OOS2044`) y el tope de siempre.
//! - **C · Lo empujado se vuelve a evaluar** (regla 3).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

use sqlparser::ast::{
    BinaryOperator, Expr, GroupByExpr, Ident, JoinConstraint, JoinOperator, LimitClause,
    ObjectName, ObjectNamePart, OrderByKind, OrderBySort, Query, Select, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement, TableAlias, TableFactor, TableWithJoins,
    UnaryOperator, Value, Visit, Visitor, WildcardAdditionalOptions,
};
use sqlparser::dialect::DuckDbDialect;
use sqlparser::parser::Parser;

use crate::document::Kind;
use crate::json::Json;
use crate::link::{Loaded, Package};
use crate::parse::Node;

// ── lo que entra y lo que sale ───────────────────────────────────────────────

/// Lo que un conector declara (`ore-driver` `Capacidades`), por tipo de fuente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conector {
    /// Los operadores de la petición v2 que sabe poner.
    pub operadores: BTreeSet<String>,
    pub limit: bool,
    pub order_by: bool,
}

pub struct Opciones<'a> {
    /// La lectura sale de un puesto: también su conducto (F4).
    pub desde_puesto: bool,
    /// Exigir el interruptor `federation` de la fuente (ORE, F4). La spec no lo
    /// tiene: sus casos `plan/` se corren sin él.
    pub exigir_interruptor: bool,
    /// Los conectores por tipo de fuente. `None`: no se sabe, y manda la tabla
    /// (la pasarela vuelve a comprobarlo con `admite`).
    pub conectores: Option<&'a BTreeMap<String, Conector>>,
}

/// Lo que va a la derecha de un filtro, como en la petición v2.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Valor {
    Uno(String),
    Lista(Vec<String>),
    Ninguno,
}

/// Un filtro empujado: `columna operador valor`, con los operadores de la
/// petición v2 (`eq`, `neq`, `in`, `lt`, `le`, `gt`, `ge`, `like`, `isNull`,
/// `isNotNull`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Filtro {
    pub columna: String,
    pub operador: String,
    pub valor: Valor,
}

/// La lectura de una tabla: lo que se pide al origen y lo que queda.
#[derive(Debug, Clone, PartialEq)]
pub struct Lectura {
    pub tabla: String,
    pub fuente: String,
    pub tipo: String,
    pub env: String,
    pub objeto: String,
    pub columnas: Vec<String>,
    pub empujados: Vec<Filtro>,
    /// Las condiciones sobre esta tabla que evalúa el motor, como se escribieron.
    pub en_el_motor: Vec<String>,
    pub limit: Option<u64>,
    /// `(columna, descendente)`.
    pub orden: Vec<(String, bool)>,
    pub full_scan: String,
    /// `fullScan: expensive`: se lee con presupuesto.
    pub presupuesto: bool,
    pub apariciones: usize,
    pub avisos: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reparto {
    pub lecturas: Vec<Lectura>,
    /// `false`: la sentencia no se analizó y se lee sin empujar (B).
    pub entendida: bool,
    pub avisos: Vec<String>,
}

/// Un no al planificar, con el estado HTTP que le toca y su código.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negado {
    pub http: u16,
    pub codigo: String,
    pub tabla: Option<String>,
    pub mensaje: String,
}

fn negado(http: u16, codigo: &str, tabla: &str, mensaje: impl Into<String>) -> Negado {
    Negado {
        http,
        codigo: codigo.to_string(),
        tabla: Some(tabla.to_string()),
        mensaje: mensaje.into(),
    }
}

/// La familia de `reads.predicatePushdown` de un operador de la petición
/// (v1alpha24 `01` §3: `range` es `lt/le/gt/ge`; `isNull`, las dos formas).
pub fn familia(op: &str) -> Option<&'static str> {
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

// ── la entrada ───────────────────────────────────────────────────────────────

/// **El reparto de `sql`** sobre el árbol `pkg`. `Err` es el primer no, por tabla
/// en el orden en que la sentencia las lee.
pub fn repartir(sql: &str, pkg: &Package, o: &Opciones) -> Result<Reparto, Negado> {
    let mut a = Analisis::new(pkg, o);
    let entendida = match Parser::parse_sql(&DuckDbDialect {}, sql) {
        Ok(sts) if sts.len() == 1 => match &sts[0] {
            // Lo que devuelve la consulta de fuera lo usa quien la pide: entero.
            Statement::Query(q) => match a.query(q, &[]) {
                Ok(f) => {
                    a.pedir_todo(&f);
                    true
                }
                Err(_) => false,
            },
            _ => false,
        },
        _ => false,
    };
    let mut avisos = Vec::new();
    if !entendida {
        // B: lo que el tokenizador ve, también dentro de las vistas, sin empujar.
        a = Analisis::new(pkg, o);
        let mut vistas = BTreeSet::new();
        for t in tablas_por_tokens(sql, pkg, &mut vistas, 0) {
            a.aparicion_entera(&t);
        }
        avisos.push(
            "el analizador no entiende esta sentencia: sus tablas se leen sin empujar nada \
             (todas las columnas, ningún filtro; el motor filtra después)"
                .to_string(),
        );
    }
    let lecturas = a.cerrar(entendida)?;
    Ok(Reparto {
        lecturas,
        entendida,
        avisos,
    })
}

/// Las `Table` que lee un texto, por el tokenizador: las suyas y las de las
/// vistas que nombra, hasta una profundidad.
fn tablas_por_tokens(
    sql: &str,
    pkg: &Package,
    vistas: &mut BTreeSet<String>,
    nivel: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    if nivel > 16 {
        return out;
    }
    for n in crate::sql_del_arbol::nombres_a_resolver(sql, pkg) {
        if let Some(t) = pkg.table(&n) {
            let qn = t.qname().unwrap_or_default();
            if !out.contains(&qn) {
                out.push(qn);
            }
        } else if let Some(v) = pkg.view(&n) {
            let qn = v.qname().unwrap_or_default();
            if vistas.insert(qn) {
                let texto = sql_de_vista(v).unwrap_or_default();
                for t in tablas_por_tokens(&texto, pkg, vistas, nivel + 1) {
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
            }
        }
    }
    out
}

/// **El SQL de una vista**: el suyo (`spec.sql`, v1alpha14+), o el de una
/// vista de antes escrita con `from` y `fields` —`SELECT "c" AS "n", … FROM
/// t`—, como las que dejó la inducción de una base foránea antes de v1alpha14.
/// `None` si no es ninguna de las dos (otra forma que el reparto no lee).
pub fn sql_de_vista(v: &Loaded) -> Option<String> {
    if let Some(s) = v.section("sql").and_then(Node::as_str) {
        return Some(s.to_string());
    }
    let desde = v.section("from")?;
    let origen = desde
        .get("table")
        .or_else(|| desde.get("view"))
        .and_then(|(_, n)| n.as_str())?;
    let campos: Vec<String> = v
        .section("fields")?
        .entries()
        .iter()
        .filter_map(|(k, c)| {
            let n = k.as_str()?;
            let c = c.as_str()?;
            Some(format!(
                "\"{}\" AS \"{}\"",
                c.replace('"', "\"\""),
                n.replace('"', "\"\"")
            ))
        })
        .collect();
    if campos.is_empty() {
        return None;
    }
    let destino = crate::link::cualificar(origen, v);
    Some(format!("SELECT {} FROM {destino}", campos.join(", ")))
}

/// Las `Table` a las que llega una vista (por su SQL, con el tokenizador, y
/// por las vistas que nombra). Vacío: no lee ningún origen.
pub fn tablas_de_la_vista(pkg: &Package, v: &Loaded) -> Vec<String> {
    let mut vistas = BTreeSet::new();
    vistas.insert(v.qname().unwrap_or_default());
    sql_de_vista(v)
        .map(|t| tablas_por_tokens(&t, pkg, &mut vistas, 0))
        .unwrap_or_default()
}

// ── el análisis ──────────────────────────────────────────────────────────────

/// No se entiende: se cae a B.
struct NoEntiendo;

type Res<T> = Result<T, NoEntiendo>;

/// Una aparición de una tabla en la sentencia.
#[derive(Debug, Default)]
struct Aparicion {
    tabla: String,
    declaradas: Vec<String>,
    /// Las familias que se pueden empujar: la tabla ∩ el conector.
    familias: BTreeSet<String>,
    /// Los operadores que el conector sabe (vacío y `conector_conocido` falso:
    /// todos).
    operadores: Option<BTreeSet<String>>,
    sabe_limit: bool,
    sabe_orden: bool,
    columnas: BTreeSet<String>,
    todas: bool,
    empujados: Vec<Filtro>,
    resto: Vec<String>,
    limit: Option<u64>,
    orden: Vec<(String, bool)>,
}

/// Qué es una relación del `FROM`, para lo que la lee desde fuera.
#[derive(Debug, Clone)]
enum Fuente {
    /// Una `Table`: la aparición.
    Tabla(usize),
    /// Una proyección limpia que llega a una tabla: columna de salida →
    /// (aparición, columna de la tabla).
    Limpia(Vec<(String, usize, String)>),
    /// Otra cosa (el lago, una función, una consulta que agrega): sus columnas de
    /// salida, si se saben.
    Opaca(Option<Vec<String>>),
}

#[derive(Debug, Clone)]
struct Rel {
    /// Cómo se la puede calificar, en minúsculas.
    nombres: Vec<String>,
    fuente: Fuente,
    /// Puede quedar a nulo (el lado de fuera de un `LEFT`/`RIGHT`/`FULL JOIN`).
    nula: bool,
}

#[derive(Debug, Default, Clone)]
struct Ambito {
    rels: Vec<Rel>,
}

/// A qué relación va una columna.
enum Resol {
    /// En el ámbito de dentro: la relación y su nombre de columna.
    Una(usize, String),
    /// En varias del de dentro (sin calificar): todas.
    Varias(Vec<(usize, String)>),
    /// En un ámbito de fuera: (nivel desde fuera, relación, columna).
    Fuera(usize, usize, String),
    /// No se sabe (un alias de la proyección, un parámetro de una lambda, una
    /// relación sin columnas conocidas).
    Nada,
}

struct Analisis<'p, 'o> {
    pkg: &'p Package,
    o: &'o Opciones<'o>,
    aps: Vec<Aparicion>,
    /// Los `WITH` visibles, del más de fuera al más de dentro.
    ctes: Vec<BTreeMap<String, (Query, Option<TableAlias>)>>,
    en_curso: Vec<String>,
    nivel: usize,
}

impl<'p, 'o> Analisis<'p, 'o> {
    fn new(pkg: &'p Package, o: &'o Opciones<'o>) -> Self {
        Analisis {
            pkg,
            o,
            aps: Vec::new(),
            ctes: Vec::new(),
            en_curso: Vec::new(),
            nivel: 0,
        }
    }

    fn conector_de(&self, t: &Loaded) -> Option<&Conector> {
        let fuente = t.section("datasource").and_then(Node::as_str)?;
        let tipo = self.datasource(fuente)?.0;
        self.o.conectores?.get(&tipo)
    }

    /// `(type, connectionEnv, federation)` de una fuente del manifiesto.
    fn datasource(&self, nombre: &str) -> Option<(String, Option<String>, bool)> {
        let ds = self
            .pkg
            .docs
            .iter()
            .filter(|d| d.kind == Kind::OntologyConfig)
            .flat_map(|c| {
                c.section("datasources")
                    .map(|n| n.items().to_vec())
                    .unwrap_or_default()
            })
            .find(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(nombre))?;
        let campo = |k: &str| ds.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        Some((
            campo("type").unwrap_or_default(),
            campo("connectionEnv"),
            campo("federation").as_deref() == Some("true"),
        ))
    }

    /// Una aparición nueva de la tabla `t`.
    fn aparicion(&mut self, t: &Loaded) -> usize {
        let declaradas: Vec<String> = t
            .section("columns")
            .map(|c| {
                c.entries()
                    .iter()
                    .filter_map(|(k, _)| k.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let de_la_tabla: BTreeSet<String> = t
            .section("reads")
            .and_then(|r| r.get("predicatePushdown"))
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|o| o.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let conector = self.conector_de(t).cloned();
        let ap = Aparicion {
            tabla: t.qname().unwrap_or_default(),
            declaradas,
            familias: de_la_tabla,
            operadores: conector.as_ref().map(|c| c.operadores.clone()),
            sabe_limit: conector.as_ref().is_none_or(|c| c.limit),
            sabe_orden: conector.as_ref().is_none_or(|c| c.order_by),
            ..Default::default()
        };
        self.aps.push(ap);
        self.aps.len() - 1
    }

    /// B: la tabla entera, sin empujar.
    fn aparicion_entera(&mut self, qn: &str) {
        if let Some(t) = self.pkg.table(qn) {
            let i = self.aparicion(t);
            self.aps[i].todas = true;
        }
    }

    // ── consultas ──

    fn query(&mut self, q: &Query, fuera: &[Ambito]) -> Res<Fuente> {
        self.nivel += 1;
        if self.nivel > 64 {
            return Err(NoEntiendo);
        }
        let mut puestos = false;
        if let Some(w) = &q.with {
            if w.recursive {
                return Err(NoEntiendo);
            }
            let mut m = BTreeMap::new();
            for c in &w.cte_tables {
                m.insert(
                    c.alias.name.value.to_lowercase(),
                    (
                        (*c.query).clone(),
                        Some(c.alias.clone()).filter(|a| !a.columns.is_empty()),
                    ),
                );
            }
            self.ctes.push(m);
            puestos = true;
        }
        let r = match q.body.as_ref() {
            SetExpr::Select(s) => self.select(s, Some(q), fuera),
            SetExpr::Query(dentro) => {
                let f = self.query(dentro, fuera)?;
                if q.limit_clause.is_some() || q.order_by.is_some() || q.fetch.is_some() {
                    self.pedir_todo(&f);
                    Ok(Fuente::Opaca(self.salida_de(&f)))
                } else {
                    Ok(f)
                }
            }
            SetExpr::SetOperation { left, right, .. } => {
                let i = self.set_expr(left, fuera)?;
                self.set_expr(right, fuera)?;
                Ok(Fuente::Opaca(i))
            }
            SetExpr::Values(_) => Ok(Fuente::Opaca(None)),
            _ => Err(NoEntiendo),
        };
        if puestos {
            self.ctes.pop();
        }
        self.nivel -= 1;
        r
    }

    /// Un lado de un `UNION`: lo que devuelve se usa entero.
    fn set_expr(&mut self, s: &SetExpr, fuera: &[Ambito]) -> Res<Option<Vec<String>>> {
        let f = match s {
            SetExpr::Select(sel) => self.select(sel, None, fuera)?,
            SetExpr::Query(q) => self.query(q, fuera)?,
            SetExpr::SetOperation { left, right, .. } => {
                let i = self.set_expr(left, fuera)?;
                self.set_expr(right, fuera)?;
                Fuente::Opaca(i)
            }
            SetExpr::Values(_) => Fuente::Opaca(None),
            _ => return Err(NoEntiendo),
        };
        self.pedir_todo(&f);
        Ok(self.salida_de(&f))
    }

    fn salida_de(&self, f: &Fuente) -> Option<Vec<String>> {
        match f {
            Fuente::Tabla(i) => Some(self.aps[*i].declaradas.clone()),
            Fuente::Limpia(m) => Some(m.iter().map(|(s, _, _)| s.clone()).collect()),
            Fuente::Opaca(c) => c.clone(),
        }
    }

    /// Todo lo que una relación devuelve, pedido (lo usa algo opaco).
    fn pedir_todo(&mut self, f: &Fuente) {
        match f {
            Fuente::Tabla(i) => self.aps[*i].todas = true,
            Fuente::Limpia(m) => {
                for (_, ap, c) in m.clone() {
                    self.aps[ap].columnas.insert(c);
                }
            }
            Fuente::Opaca(_) => {}
        }
    }

    fn select(&mut self, s: &Select, q: Option<&Query>, fuera: &[Ambito]) -> Res<Fuente> {
        if !s.lateral_views.is_empty() || s.prewhere.is_some() || !s.connect_by.is_empty() {
            return Err(NoEntiendo);
        }
        // ① El FROM, con sus juntas: el ámbito y lo que dicen los `ON`.
        let mut amb = Ambito::default();
        let mut ons: Vec<(Expr, Empuje)> = Vec::new();
        for twj in &s.from {
            self.from(twj, &mut amb, &mut ons, fuera)?;
        }

        // ② Lo que filtra: las conjunciones del WHERE y de los ON.
        let mut todo_empujado = true;
        if let Some(w) = &s.selection {
            for c in conjunciones(w) {
                if !self.empujar(&c, &amb, Empuje::Donde) {
                    todo_empujado = false;
                    self.al_motor(&c, &amb);
                }
            }
        }
        for (c, modo) in &ons {
            if !self.empujar(c, &amb, modo.clone()) {
                self.al_motor(c, &amb);
            }
        }

        // ③ ¿Proyección limpia? Entonces lo de la proyección lo pide quien la lee.
        let limpia = self.limpia(s, q, &amb);

        // ④ Las columnas que se usan. `FROM t` sin SELECT (DuckDB) es `SELECT *`,
        //   y `COLUMNS(…)` elige columnas por patrón: las dos, todas.
        if s.projection.is_empty() || s.projection.iter().any(elige_columnas) {
            for r in amb.rels.clone() {
                self.pedir_todo(&r.fuente);
            }
        }
        let mut exprs: Vec<&Expr> = Vec::new();
        if limpia.is_none() {
            for it in &s.projection {
                match it {
                    SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => {
                        exprs.push(e)
                    }
                    SelectItem::Wildcard(_) => {
                        for r in amb.rels.clone() {
                            self.pedir_todo(&r.fuente);
                        }
                    }
                    SelectItem::QualifiedWildcard(
                        SelectItemQualifiedWildcardKind::ObjectName(n),
                        _,
                    ) => {
                        let partes = partes(n);
                        match amb.rels.iter().position(|r| califica(r, &partes)) {
                            Some(i) => {
                                let f = amb.rels[i].fuente.clone();
                                self.pedir_todo(&f);
                            }
                            None => return Err(NoEntiendo),
                        }
                    }
                    _ => return Err(NoEntiendo),
                }
            }
        }
        if let Some(w) = &s.selection {
            exprs.push(w);
        }
        for (c, _) in &ons {
            exprs.push(c);
        }
        if let GroupByExpr::Expressions(es, _) = &s.group_by {
            exprs.extend(es.iter());
        }
        if let Some(h) = &s.having {
            exprs.push(h);
        }
        if let Some(qu) = &s.qualify {
            exprs.push(qu);
        }
        exprs.extend(s.sort_by.iter().map(|o| &o.expr));
        exprs.extend(s.cluster_by.iter());
        exprs.extend(s.distribute_by.iter());
        if let Some(q) = q
            && let Some(ob) = &q.order_by
            && let OrderByKind::Expressions(os) = &ob.kind
        {
            exprs.extend(os.iter().map(|o| &o.expr));
        }
        for e in exprs {
            self.usar(e, &amb, fuera)?;
        }
        // Las ventanas con nombre (`WINDOW w AS (…)`) miran columnas: sin
        // analizarlas, todas (conservador).
        if !s.named_window.is_empty() {
            for r in amb.rels.clone() {
                self.pedir_todo(&r.fuente);
            }
            todo_empujado = false;
        }

        // ⑤ El límite, si se puede.
        if let Some(q) = q {
            self.limite(s, q, &amb, todo_empujado)?;
        }

        Ok(match limpia {
            Some(m) => Fuente::Limpia(m),
            None => Fuente::Opaca(salida_de_select(s, &amb, self)),
        })
    }

    /// Una relación del FROM y sus juntas, al ámbito.
    fn from(
        &mut self,
        twj: &TableWithJoins,
        amb: &mut Ambito,
        ons: &mut Vec<(Expr, Empuje)>,
        fuera: &[Ambito],
    ) -> Res<()> {
        let r = self.factor(&twj.relation, fuera)?;
        amb.rels.push(r);
        for j in &twj.joins {
            let antes = amb.rels.len();
            let r = self.factor(&j.relation, fuera)?;
            amb.rels.push(r);
            let nueva = antes;
            let (c, modo) = match &j.join_operator {
                JoinOperator::Join(c)
                | JoinOperator::Inner(c)
                | JoinOperator::StraightJoin(c)
                | JoinOperator::CrossJoin(c) => (c, Empuje::Dentro),
                JoinOperator::Left(c) | JoinOperator::LeftOuter(c) => {
                    amb.rels[nueva].nula = true;
                    (c, Empuje::SoloA(vec![nueva]))
                }
                JoinOperator::Right(c) | JoinOperator::RightOuter(c) => {
                    let izq: Vec<usize> = (0..nueva).collect();
                    for i in &izq {
                        amb.rels[*i].nula = true;
                    }
                    (c, Empuje::SoloA(izq))
                }
                JoinOperator::FullOuter(c) => {
                    for r in amb.rels.iter_mut() {
                        r.nula = true;
                    }
                    (c, Empuje::Nada)
                }
                // Semi y anti: el lado derecho sólo filtra; lo suyo en el ON se
                // puede filtrar antes, como el de un LEFT JOIN.
                JoinOperator::Semi(c)
                | JoinOperator::LeftSemi(c)
                | JoinOperator::Anti(c)
                | JoinOperator::LeftAnti(c) => (c, Empuje::SoloA(vec![nueva])),
                _ => return Err(NoEntiendo),
            };
            match c {
                JoinConstraint::On(e) => {
                    for x in conjunciones(e) {
                        ons.push((x, modo.clone()));
                    }
                }
                JoinConstraint::Using(cols) => {
                    // Las columnas del USING, de los dos lados.
                    for n in cols {
                        let col = partes(n).join(".");
                        for i in 0..amb.rels.len() {
                            let f = amb.rels[i].fuente.clone();
                            self.pedir_columna(&f, &col);
                        }
                    }
                }
                JoinConstraint::Natural => {
                    for r in amb.rels.clone() {
                        self.pedir_todo(&r.fuente);
                    }
                }
                JoinConstraint::None => {}
            }
        }
        Ok(())
    }

    fn factor(&mut self, tf: &TableFactor, fuera: &[Ambito]) -> Res<Rel> {
        match tf {
            TableFactor::Table {
                name,
                alias,
                args,
                sample,
                ..
            } => {
                // `TABLESAMPLE`: un `LIMIT` empujado antes de la muestra cambiaría
                // las filas. Sin analizar (B).
                if sample.is_some() {
                    return Err(NoEntiendo);
                }
                if args.is_some() {
                    // Una función de tabla (`read_parquet(…)`, `range(…)`): no es del árbol.
                    return Ok(rel_de(alias.as_ref(), &partes(name), Fuente::Opaca(None)));
                }
                let p = partes(name);
                let f = self.por_nombre(name)?;
                Ok(rel_de(alias.as_ref(), &p, f))
            }
            TableFactor::Derived {
                lateral,
                subquery,
                alias,
                ..
            } => {
                if *lateral {
                    return Err(NoEntiendo);
                }
                let f = self.query(subquery, fuera)?;
                let f = renombrar(f, alias.as_ref(), self);
                Ok(rel_de(alias.as_ref(), &[], f))
            }
            TableFactor::NestedJoin { .. }
            | TableFactor::Pivot { .. }
            | TableFactor::Unpivot { .. } => Err(NoEntiendo),
            TableFactor::MatchRecognize { .. } => Err(NoEntiendo),
            // UNNEST, funciones de tabla, JSON_TABLE…: no leen el árbol.
            other => {
                if lee_del_arbol(other, self.pkg) {
                    return Err(NoEntiendo);
                }
                Ok(Rel {
                    nombres: Vec::new(),
                    fuente: Fuente::Opaca(None),
                    nula: false,
                })
            }
        }
    }

    /// Un nombre del FROM: un `WITH`, una tabla, una vista, o algo del lago.
    fn por_nombre(&mut self, nombre: &ObjectName) -> Res<Fuente> {
        let p = partes(nombre);
        if p.len() == 1 {
            let k = p[0].to_lowercase();
            let cte = self.ctes.iter().rev().find_map(|m| m.get(&k)).cloned();
            if let Some((q, cols)) = cte {
                if self.en_curso.contains(&k) {
                    return Err(NoEntiendo);
                }
                self.en_curso.push(k);
                // Un `WITH` sólo ve los `WITH` de fuera, no el ámbito del SELECT.
                let f = self.query(&q, &[]);
                self.en_curso.pop();
                let f = f?;
                return Ok(renombrar(f, cols.as_ref(), self));
            }
        }
        let qn = p.join(".");
        if let Some(t) = self.pkg.table(&qn) {
            let i = self.aparicion(t);
            return Ok(Fuente::Tabla(i));
        }
        if let Some(v) = self.pkg.view(&qn) {
            let texto = sql_de_vista(v).ok_or(NoEntiendo)?;
            let vq = match Parser::parse_sql(&DuckDbDialect {}, &texto) {
                Ok(mut sts) if sts.len() == 1 => match sts.pop() {
                    Some(Statement::Query(q)) => q,
                    _ => return Err(NoEntiendo),
                },
                _ => return Err(NoEntiendo),
            };
            let clave = format!("vista:{}", v.qname().unwrap_or_default());
            if self.en_curso.contains(&clave) {
                return Err(NoEntiendo);
            }
            self.en_curso.push(clave);
            // La vista ve sólo el árbol: ni los WITH ni el ámbito de quien la lee.
            let ctes = std::mem::take(&mut self.ctes);
            let f = self.query(&vq, &[]);
            self.ctes = ctes;
            self.en_curso.pop();
            return f;
        }
        // Del lago (un Dataset, una colección) o de fuera del árbol.
        let corto = crate::normalize::a_corto(&qn).into_owned();
        let cols = self
            .pkg
            .docs
            .iter()
            .find(|d| d.qname().as_deref() == Some(corto.as_str()))
            .and_then(|d| d.section("columns"))
            .map(|c| {
                c.entries()
                    .iter()
                    .filter_map(|(k, _)| k.as_str().map(String::from))
                    .collect()
            });
        Ok(Fuente::Opaca(cols))
    }

    // ── columnas ──

    fn resolver(&self, partes: &[String], amb: &Ambito, fuera: &[Ambito]) -> Resol {
        let (calif, col) = match partes.split_last() {
            Some((c, q)) => (q, c.clone()),
            None => return Resol::Nada,
        };
        let en = |a: &Ambito| -> Vec<(usize, String)> {
            if calif.is_empty() {
                a.rels
                    .iter()
                    .enumerate()
                    .filter_map(|(i, r)| self.columna_de(&r.fuente, &col).map(|c| (i, c)))
                    .collect()
            } else {
                a.rels
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| califica(r, calif))
                    .map(|(i, r)| (i, self.columna_de(&r.fuente, &col).unwrap_or(col.clone())))
                    .collect()
            }
        };
        let dentro = en(amb);
        match dentro.len() {
            1 => return Resol::Una(dentro[0].0, dentro[0].1.clone()),
            0 => {}
            _ => return Resol::Varias(dentro),
        }
        for (k, a) in fuera.iter().rev().enumerate() {
            let v = en(a);
            if let Some((i, c)) = v.into_iter().next() {
                return Resol::Fuera(k, i, c);
            }
        }
        // `s.campo` de un struct de DuckDB: la columna es la primera parte.
        if !calif.is_empty() {
            return self.resolver(&partes[..1], amb, fuera);
        }
        Resol::Nada
    }

    /// El nombre de `col` en la relación, si lo tiene (sin distinguir mayúsculas).
    fn columna_de(&self, f: &Fuente, col: &str) -> Option<String> {
        let igual = |a: &str| a.eq_ignore_ascii_case(col);
        match f {
            Fuente::Tabla(i) => self.aps[*i].declaradas.iter().find(|c| igual(c)).cloned(),
            Fuente::Limpia(m) => m
                .iter()
                .find(|(s, _, _)| igual(s))
                .map(|(s, _, _)| s.clone()),
            Fuente::Opaca(Some(cs)) => cs.iter().find(|c| igual(c)).cloned(),
            Fuente::Opaca(None) => None,
        }
    }

    fn pedir_columna(&mut self, f: &Fuente, col: &str) {
        match f {
            Fuente::Tabla(i) => {
                if let Some(c) = self.columna_de(f, col) {
                    self.aps[*i].columnas.insert(c);
                }
            }
            Fuente::Limpia(m) => {
                if let Some((_, ap, c)) = m.iter().find(|(s, _, _)| s.eq_ignore_ascii_case(col)) {
                    self.aps[*ap].columnas.insert(c.clone());
                }
            }
            Fuente::Opaca(_) => {}
        }
    }

    /// Las columnas que mira `e`, pedidas; sus subconsultas, analizadas con este
    /// ámbito por fuera.
    fn usar(&mut self, e: &Expr, amb: &Ambito, fuera: &[Ambito]) -> Res<()> {
        let mut v = Mira::default();
        let _ = e.visit(&mut v);
        for p in v.columnas {
            match self.resolver(&p, amb, fuera) {
                Resol::Una(i, c) => {
                    let f = amb.rels[i].fuente.clone();
                    self.pedir_columna(&f, &c);
                }
                Resol::Varias(vs) => {
                    for (i, c) in vs {
                        let f = amb.rels[i].fuente.clone();
                        self.pedir_columna(&f, &c);
                    }
                }
                Resol::Fuera(k, i, c) => {
                    let a = &fuera[fuera.len() - 1 - k];
                    let f = a.rels[i].fuente.clone();
                    self.pedir_columna(&f, &c);
                }
                Resol::Nada => {}
            }
        }
        if !v.subconsultas.is_empty() {
            let mut fuera2: Vec<Ambito> = fuera.to_vec();
            fuera2.push(amb.clone());
            for q in v.subconsultas {
                let f = self.query(&q, &fuera2)?;
                self.pedir_todo(&f);
            }
        }
        Ok(())
    }

    // ── filtros ──

    /// Empuja la conjunción `c` si puede. `true`: empujada (y se volverá a
    /// evaluar en el motor igualmente).
    fn empujar(&mut self, c: &Expr, amb: &Ambito, modo: Empuje) -> bool {
        let Some((col, filtros)) = forma(c) else {
            return false;
        };
        let (i, nombre) = match self.resolver(&col, amb, &[]) {
            Resol::Una(i, n) => (i, n),
            _ => return false,
        };
        let rel = &amb.rels[i];
        let rechaza_nulo = filtros.iter().all(|f| f.operador != "isNull");
        let vale = match &modo {
            Empuje::Donde => !rel.nula || rechaza_nulo,
            Empuje::Dentro => !rel.nula || rechaza_nulo,
            Empuje::SoloA(lado) => lado.contains(&i),
            Empuje::Nada => false,
        };
        if !vale {
            return false;
        }
        let (ap, columna) = match &rel.fuente {
            Fuente::Tabla(ap) => match self.columna_de(&rel.fuente, &nombre) {
                Some(c) => (*ap, c),
                None => return false,
            },
            Fuente::Limpia(m) => match m.iter().find(|(s, _, _)| s.eq_ignore_ascii_case(&nombre)) {
                Some((_, ap, c)) => (*ap, c.clone()),
                None => return false,
            },
            Fuente::Opaca(_) => return false,
        };
        let a = &self.aps[ap];
        let admite = |op: &str| {
            familia(op).is_some_and(|f| a.familias.contains(f))
                && a.operadores.as_ref().is_none_or(|ops| ops.contains(op))
        };
        if !filtros.iter().all(|f| admite(&f.operador)) {
            return false;
        }
        for mut f in filtros {
            f.columna = columna.clone();
            if !self.aps[ap].empujados.contains(&f) {
                self.aps[ap].empujados.push(f);
            }
        }
        true
    }

    /// Lo que no se empujó, dicho en cada tabla que toca.
    fn al_motor(&mut self, c: &Expr, amb: &Ambito) {
        let mut v = Mira::default();
        let _ = c.visit(&mut v);
        let mut aps = BTreeSet::new();
        for p in &v.columnas {
            let rels: Vec<usize> = match self.resolver(p, amb, &[]) {
                Resol::Una(i, _) => vec![i],
                Resol::Varias(vs) => vs.into_iter().map(|(i, _)| i).collect(),
                _ => vec![],
            };
            for i in rels {
                match &amb.rels[i].fuente {
                    Fuente::Tabla(ap) => {
                        aps.insert(*ap);
                    }
                    Fuente::Limpia(m) => {
                        aps.extend(m.iter().map(|(_, ap, _)| *ap));
                    }
                    Fuente::Opaca(_) => {}
                }
            }
        }
        let texto = c.to_string();
        for ap in aps {
            if !self.aps[ap].resto.contains(&texto) {
                self.aps[ap].resto.push(texto.clone());
            }
        }
    }

    // ── proyección limpia y límite ──

    fn limpia(
        &self,
        s: &Select,
        q: Option<&Query>,
        amb: &Ambito,
    ) -> Option<Vec<(String, usize, String)>> {
        if amb.rels.len() != 1
            || s.distinct.is_some()
            || s.top.is_some()
            || s.having.is_some()
            || s.qualify.is_some()
            || !s.named_window.is_empty()
            || !matches!(&s.group_by, GroupByExpr::Expressions(es, ms) if es.is_empty() && ms.is_empty())
        {
            return None;
        }
        if let Some(q) = q
            && (q.limit_clause.is_some() || q.fetch.is_some())
        {
            return None;
        }
        if s.projection.is_empty() || s.projection.iter().any(elige_columnas) {
            return None;
        }
        let rel = &amb.rels[0];
        let mapa_rel: Vec<(String, usize, String)> = match &rel.fuente {
            Fuente::Tabla(ap) => self.aps[*ap]
                .declaradas
                .iter()
                .map(|c| (c.clone(), *ap, c.clone()))
                .collect(),
            Fuente::Limpia(m) => m.clone(),
            Fuente::Opaca(_) => return None,
        };
        let mut out = Vec::new();
        for it in &s.projection {
            match it {
                SelectItem::Wildcard(w) if simple(w) => out.extend(mapa_rel.iter().cloned()),
                SelectItem::QualifiedWildcard(_, w) if simple(w) => {
                    out.extend(mapa_rel.iter().cloned())
                }
                SelectItem::UnnamedExpr(e) => {
                    let p = columna_simple(e)?;
                    let col = p.last()?;
                    let (_, ap, c) = mapa_rel
                        .iter()
                        .find(|(s, _, _)| s.eq_ignore_ascii_case(col))?;
                    out.push((col.clone(), *ap, c.clone()));
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let p = columna_simple(expr)?;
                    let col = p.last()?;
                    let (_, ap, c) = mapa_rel
                        .iter()
                        .find(|(s, _, _)| s.eq_ignore_ascii_case(col))?;
                    out.push((alias.value.clone(), *ap, c.clone()));
                }
                _ => return None,
            }
        }
        Some(out)
    }

    fn limite(&mut self, s: &Select, q: &Query, amb: &Ambito, todo_empujado: bool) -> Res<()> {
        let Some(lc) = &q.limit_clause else {
            return Ok(());
        };
        let (n, k) = match lc {
            LimitClause::LimitOffset {
                limit,
                offset,
                limit_by,
            } => {
                if !limit_by.is_empty() {
                    return Ok(());
                }
                let Some(n) = limit.as_ref().and_then(entero) else {
                    return Ok(());
                };
                let k = match offset {
                    Some(o) => match entero(&o.value) {
                        Some(k) => k,
                        None => return Ok(()),
                    },
                    None => 0,
                };
                (n, k)
            }
            LimitClause::OffsetCommaLimit { offset, limit } => {
                match (entero(limit), entero(offset)) {
                    (Some(n), Some(k)) => (n, k),
                    _ => return Ok(()),
                }
            }
        };
        if amb.rels.len() != 1
            || !todo_empujado
            || q.fetch.is_some()
            || s.distinct.is_some()
            || s.having.is_some()
            || s.qualify.is_some()
            || !matches!(&s.group_by, GroupByExpr::Expressions(es, ms) if es.is_empty() && ms.is_empty())
            || s.projection.iter().any(agrega)
        {
            return Ok(());
        }
        let ap_de = |col: &str, f: &Fuente| -> Option<(usize, String)> {
            match f {
                Fuente::Tabla(ap) => self.aps[*ap]
                    .declaradas
                    .iter()
                    .find(|c| c.eq_ignore_ascii_case(col))
                    .map(|c| (*ap, c.clone())),
                Fuente::Limpia(m) => m
                    .iter()
                    .find(|(s, _, _)| s.eq_ignore_ascii_case(col))
                    .map(|(_, ap, c)| (*ap, c.clone())),
                Fuente::Opaca(_) => None,
            }
        };
        let fuente = amb.rels[0].fuente.clone();
        let ap = match &fuente {
            Fuente::Tabla(ap) => *ap,
            Fuente::Limpia(m) => {
                let aps: BTreeSet<usize> = m.iter().map(|(_, a, _)| *a).collect();
                match aps.into_iter().collect::<Vec<_>>()[..] {
                    [a] => a,
                    _ => return Ok(()),
                }
            }
            Fuente::Opaca(_) => return Ok(()),
        };
        // Lo que quedó en el motor de esta tabla, por dentro (una vista que
        // filtra algo que no se empujó), también quita filas.
        if !self.aps[ap].resto.is_empty() {
            return Ok(());
        }
        let mut orden = Vec::new();
        if let Some(ob) = &q.order_by {
            let OrderByKind::Expressions(os) = &ob.kind else {
                return Ok(());
            };
            for o in os {
                let Some(p) = columna_simple(&o.expr) else {
                    return Ok(());
                };
                let Some((a, c)) = ap_de(p.last().map(String::as_str).unwrap_or(""), &fuente)
                else {
                    return Ok(());
                };
                let desc = match &o.options.sort {
                    None | Some(OrderBySort::Asc) => false,
                    Some(OrderBySort::Desc) => true,
                    Some(_) => return Ok(()),
                };
                if a != ap || o.options.nulls_first == Some(true) {
                    return Ok(());
                }
                orden.push((c, desc));
            }
            if !self.aps[ap].sabe_orden {
                return Ok(());
            }
        }
        if !self.aps[ap].sabe_limit {
            return Ok(());
        }
        self.aps[ap].limit = Some(n.saturating_add(k));
        self.aps[ap].orden = orden;
        Ok(())
    }

    // ── el cierre: una lectura por tabla, y lo que la tabla exige ──

    fn cerrar(self, entendida: bool) -> Result<Vec<Lectura>, Negado> {
        let mut orden: Vec<String> = Vec::new();
        let mut grupos: BTreeMap<String, Vec<Aparicion>> = BTreeMap::new();
        for a in self.aps {
            if !orden.contains(&a.tabla) {
                orden.push(a.tabla.clone());
            }
            grupos.entry(a.tabla.clone()).or_default().push(a);
        }
        let mut out = Vec::new();
        for qn in orden {
            let aps = grupos.remove(&qn).unwrap_or_default();
            out.push(lectura(self.pkg, self.o, &qn, aps, entendida)?);
        }
        Ok(out)
    }
}

/// Cómo se puede empujar una conjunción según de dónde sale.
#[derive(Debug, Clone)]
enum Empuje {
    /// El WHERE.
    Donde,
    /// El ON de una junta interna.
    Dentro,
    /// El ON de una junta externa: sólo a estas relaciones (el lado que puede
    /// quedar a nulo, que es filtrarlo antes).
    SoloA(Vec<usize>),
    /// El ON de un FULL JOIN: nada.
    Nada,
}

/// La lectura de una tabla, fundidas sus apariciones (A), y lo que exige.
fn lectura(
    pkg: &Package,
    o: &Opciones,
    qn: &str,
    aps: Vec<Aparicion>,
    entendida: bool,
) -> Result<Lectura, Negado> {
    let t = pkg
        .table(qn)
        .ok_or_else(|| negado(404, "objeto", qn, format!("no hay una `Table` `{qn}`")))?;
    let declaradas = aps
        .first()
        .map(|a| a.declaradas.clone())
        .unwrap_or_default();
    let mut avisos = Vec::new();

    // Columnas: la unión.
    let todas = aps.iter().any(|a| a.todas);
    let mut usadas: BTreeSet<String> = BTreeSet::new();
    for a in &aps {
        usadas.extend(a.columnas.iter().cloned());
    }
    let mut columnas: Vec<String> = if todas {
        declaradas.clone()
    } else {
        declaradas
            .iter()
            .filter(|c| usadas.contains(*c))
            .cloned()
            .collect()
    };
    if columnas.is_empty() {
        // `count(*)`: hace falta que lleguen filas, no columnas.
        if let Some(c) = declaradas.first() {
            columnas.push(c.clone());
            avisos.push(format!(
                "la sentencia no usa columnas de `{qn}`: se pide `{c}` para contar filas"
            ));
        }
    }

    // Filtros: los comunes a todas las apariciones.
    let mut empujados: Vec<Filtro> = aps.first().map(|a| a.empujados.clone()).unwrap_or_default();
    for a in aps.iter().skip(1) {
        empujados.retain(|f| a.empujados.contains(f));
    }
    let mut en_el_motor: Vec<String> = Vec::new();
    for a in &aps {
        for r in &a.resto {
            if !en_el_motor.contains(r) {
                en_el_motor.push(r.clone());
            }
        }
        for f in &a.empujados {
            if !empujados.contains(f) {
                let t = describir(f);
                if !en_el_motor.contains(&t) {
                    en_el_motor.push(t);
                }
            }
        }
    }
    let (limit, orden) = if aps.len() == 1 {
        (aps[0].limit, aps[0].orden.clone())
    } else {
        (None, Vec::new())
    };
    if aps.len() > 1 {
        avisos.push(format!(
            "`{qn}` aparece {} veces: se lee una, con los filtros que tienen en común",
            aps.len()
        ));
    }

    // ⓪ La fuente y su interruptor.
    let fuente = t
        .section("datasource")
        .and_then(Node::as_str)
        .map(String::from)
        .ok_or_else(|| {
            negado(
                422,
                "OOS2020",
                qn,
                format!("`{qn}` no declara `datasource`"),
            )
        })?;
    if t.section("reads").and_then(Node::as_str) == Some("none") {
        return Err(negado(
            422,
            "OOS2020",
            qn,
            format!("`{qn}` declara `reads: none`: no se lee"),
        ));
    }
    let a = Analisis::new(pkg, o);
    let (tipo, env, encendida) = a.datasource(&fuente).ok_or_else(|| {
        negado(
            404,
            "objeto",
            qn,
            format!("la fuente `{fuente}` no está declarada"),
        )
    })?;
    if o.exigir_interruptor && !encendida {
        return Err(negado(
            403,
            "federacion",
            qn,
            format!(
                "la fuente `{fuente}` no tiene la lectura en vivo encendida: se enciende en la fuente \
                 (`ore source federation {fuente} on`, o en la consola) y vale para todas las ramas"
            ),
        ));
    }
    let objeto = t
        .section("object")
        .and_then(Node::as_str)
        .unwrap_or_default()
        .to_string();

    // ④ El coste que la tabla declara (v1alpha24 `01` §4), con lo empujado.
    let reads = t.section("reads");
    let full_scan = reads
        .and_then(|r| r.get("fullScan"))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("cheap")
        .to_string();
    let cual = if entendida {
        String::new()
    } else {
        " (la sentencia no se analizó: no se empuja nada)".to_string()
    };
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
        if !empujados
            .iter()
            .any(|f| &f.columna == c && (f.operador == "eq" || f.operador == "in"))
        {
            return Err(negado(
                422,
                "OOS2045",
                qn,
                format!(
                    "`{qn}` exige un filtro `=` o `IN` empujado sobre `{c}` (`requiredFilters`){cual}"
                ),
            ));
        }
    }
    // `requiredFilters` antes que `forbidden`: es lo más concreto (los casos `plan/`
    // de la spec lo fijan: sin nada empujado y con los dos, `OOS2045`).
    if full_scan == "forbidden" && empujados.is_empty() {
        let motor = if en_el_motor.is_empty() {
            String::new()
        } else {
            format!(
                "; lo que se evalúa en el motor no protege al origen: {}",
                en_el_motor.join(", ")
            )
        };
        return Err(negado(
            422,
            "OOS2044",
            qn,
            format!(
                "`{qn}` declara `fullScan: forbidden` y ningún filtro empujado acota la lectura{cual}{motor}. \
                 Léela con un filtro que admita (`reads.predicatePushdown`: {}), o desde una copia",
                lista(&aps_familias(t))
            ),
        ));
    }
    let presupuesto = full_scan == "expensive";
    if presupuesto && empujados.is_empty() && limit.is_none() {
        avisos.push(format!(
            "`{qn}` declara `fullScan: expensive` y esta lectura la recorre entera: se corta en su presupuesto"
        ));
    }

    // ② El gobierno: las columnas pedidas y las filtradas.
    let mut tocadas = columnas.clone();
    for f in &empujados {
        if !tocadas.contains(&f.columna) {
            tocadas.push(f.columna.clone());
        }
    }
    if let Err(n) = crate::flow::lectura_del_origen(pkg, qn, &tocadas, o.desde_puesto) {
        return Err(negado(403, n.codigo, qn, n.mensaje));
    }

    Ok(Lectura {
        tabla: qn.to_string(),
        fuente,
        tipo,
        env: env.unwrap_or_default(),
        objeto,
        columnas,
        empujados,
        en_el_motor,
        limit,
        orden,
        full_scan,
        presupuesto,
        apariciones: aps.len(),
        avisos,
    })
}

fn aps_familias(t: &Loaded) -> Vec<String> {
    t.section("reads")
        .and_then(|r| r.get("predicatePushdown"))
        .map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|o| o.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn lista(v: &[String]) -> String {
    if v.is_empty() {
        "ninguno".to_string()
    } else {
        v.join(", ")
    }
}

/// Un filtro, como SQL legible.
pub fn describir(f: &Filtro) -> String {
    let op = match f.operador.as_str() {
        "eq" => "=",
        "neq" => "<>",
        "lt" => "<",
        "le" => "<=",
        "gt" => ">",
        "ge" => ">=",
        "like" => "LIKE",
        "in" => "IN",
        "isNull" => "IS NULL",
        "isNotNull" => "IS NOT NULL",
        o => o,
    };
    match &f.valor {
        Valor::Uno(v) => format!("{} {op} '{}'", f.columna, v.replace('\'', "''")),
        Valor::Lista(vs) => format!(
            "{} {op} ({})",
            f.columna,
            vs.iter()
                .map(|v| format!("'{}'", v.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Valor::Ninguno => format!("{} {op}", f.columna),
    }
}

// ── piezas sueltas ───────────────────────────────────────────────────────────

fn partes(n: &ObjectName) -> Vec<String> {
    n.0.iter()
        .map(|p| match p {
            ObjectNamePart::Identifier(i) => i.value.clone(),
            other => other.to_string(),
        })
        .collect()
}

fn rel_de(alias: Option<&TableAlias>, p: &[String], fuente: Fuente) -> Rel {
    let nombres = match alias {
        Some(a) => vec![a.name.value.to_lowercase()],
        None if !p.is_empty() => {
            let mut v = vec![p.join(".").to_lowercase()];
            if let Some(u) = p.last() {
                v.push(u.to_lowercase());
            }
            v
        }
        None => Vec::new(),
    };
    Rel {
        nombres,
        fuente,
        nula: false,
    }
}

fn califica(r: &Rel, calif: &[String]) -> bool {
    let q = calif.join(".").to_lowercase();
    r.nombres.contains(&q)
}

/// `AS x(a, b)`: las columnas de salida, renombradas por posición.
fn renombrar(f: Fuente, alias: Option<&TableAlias>, a: &Analisis) -> Fuente {
    let Some(al) = alias.filter(|al| !al.columns.is_empty()) else {
        return f;
    };
    let nuevos: Vec<String> = al.columns.iter().map(|c| c.name.value.clone()).collect();
    match f {
        Fuente::Tabla(ap) => Fuente::Limpia(
            a.aps[ap]
                .declaradas
                .iter()
                .enumerate()
                .map(|(i, c)| (nuevos.get(i).cloned().unwrap_or(c.clone()), ap, c.clone()))
                .collect(),
        ),
        Fuente::Limpia(m) => Fuente::Limpia(
            m.into_iter()
                .enumerate()
                .map(|(i, (s, ap, c))| (nuevos.get(i).cloned().unwrap_or(s), ap, c))
                .collect(),
        ),
        Fuente::Opaca(Some(cs)) => Fuente::Opaca(Some(
            cs.into_iter()
                .enumerate()
                .map(|(i, c)| nuevos.get(i).cloned().unwrap_or(c))
                .collect(),
        )),
        Fuente::Opaca(None) => Fuente::Opaca(Some(nuevos)),
    }
}

fn simple(w: &WildcardAdditionalOptions) -> bool {
    w.opt_ilike.is_none()
        && w.opt_exclude.is_none()
        && w.opt_except.is_none()
        && w.opt_replace.is_none()
        && w.opt_rename.is_none()
}

/// Las columnas de salida de un SELECT opaco, si se saben.
fn salida_de_select(s: &Select, amb: &Ambito, a: &Analisis) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for it in &s.projection {
        match it {
            SelectItem::ExprWithAlias { alias, .. } => out.push(alias.value.clone()),
            SelectItem::UnnamedExpr(e) => out.push(columna_simple(e)?.last()?.clone()),
            SelectItem::Wildcard(w) if simple(w) => {
                for r in &amb.rels {
                    out.extend(a.salida_de(&r.fuente)?);
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// `a`, `t.a`: las partes, si `e` es una columna sin más.
fn columna_simple(e: &Expr) -> Option<Vec<String>> {
    match e {
        Expr::Identifier(i) => Some(vec![i.value.clone()]),
        Expr::CompoundIdentifier(is) => Some(is.iter().map(|i: &Ident| i.value.clone()).collect()),
        Expr::Nested(e) => columna_simple(e),
        _ => None,
    }
}

/// Las conjunciones de una expresión (`a AND b AND c`).
fn conjunciones(e: &Expr) -> Vec<Expr> {
    match e {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let mut v = conjunciones(left);
            v.extend(conjunciones(right));
            v
        }
        Expr::Nested(x) => match x.as_ref() {
            Expr::BinaryOp {
                op: BinaryOperator::And,
                ..
            } => conjunciones(x),
            _ => vec![e.clone()],
        },
        _ => vec![e.clone()],
    }
}

/// Un literal, como texto de la petición.
fn literal(e: &Expr) -> Option<String> {
    match e {
        Expr::Value(v) => match &v.value {
            Value::Number(n, _) => Some(n.clone()),
            Value::SingleQuotedString(s)
            | Value::EscapedStringLiteral(s)
            | Value::UnicodeStringLiteral(s)
            | Value::TripleSingleQuotedString(s) => Some(s.clone()),
            Value::Boolean(b) => Some(b.to_string()),
            _ => None,
        },
        Expr::TypedString(t) => literal(&Expr::Value(t.value.clone())),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => match expr.as_ref() {
            Expr::Value(v) => match &v.value {
                Value::Number(n, _) => Some(format!("-{n}")),
                _ => None,
            },
            _ => None,
        },
        Expr::Cast { expr, .. } => literal(expr),
        Expr::Nested(e) => literal(e),
        _ => None,
    }
}

fn entero(e: &Expr) -> Option<u64> {
    match e {
        Expr::Value(v) => match &v.value {
            Value::Number(n, _) => n.parse().ok(),
            _ => None,
        },
        _ => None,
    }
}

/// `columna op literal`, en filtros de la petición: la columna (sus partes) y
/// uno o dos filtros (`BETWEEN`). `None`: no tiene esa forma.
fn forma(e: &Expr) -> Option<(Vec<String>, Vec<Filtro>)> {
    let f = |op: &str, v: Valor| Filtro {
        columna: String::new(),
        operador: op.to_string(),
        valor: v,
    };
    match e {
        Expr::Nested(x) => forma(x),
        Expr::BinaryOp { left, op, right } => {
            let (col, lit, al_reves) = match (columna_simple(left), columna_simple(right)) {
                (Some(c), None) => (c, literal(right)?, false),
                (None, Some(c)) => (c, literal(left)?, true),
                _ => return None,
            };
            let op = match (op, al_reves) {
                (BinaryOperator::Eq, _) => "eq",
                (BinaryOperator::NotEq, _) => "neq",
                (BinaryOperator::Lt, false) | (BinaryOperator::Gt, true) => "lt",
                (BinaryOperator::LtEq, false) | (BinaryOperator::GtEq, true) => "le",
                (BinaryOperator::Gt, false) | (BinaryOperator::Lt, true) => "gt",
                (BinaryOperator::GtEq, false) | (BinaryOperator::LtEq, true) => "ge",
                _ => return None,
            };
            Some((col, vec![f(op, Valor::Uno(lit))]))
        }
        Expr::InList {
            expr,
            list,
            negated: false,
        } => {
            let col = columna_simple(expr)?;
            let vs: Option<Vec<String>> = list.iter().map(literal).collect();
            Some((col, vec![f("in", Valor::Lista(vs?))]))
        }
        Expr::IsNull(x) => Some((columna_simple(x)?, vec![f("isNull", Valor::Ninguno)])),
        Expr::IsNotNull(x) => Some((columna_simple(x)?, vec![f("isNotNull", Valor::Ninguno)])),
        Expr::Like {
            negated: false,
            any: false,
            expr,
            pattern,
            escape_char: None,
        } => Some((
            columna_simple(expr)?,
            vec![f("like", Valor::Uno(literal(pattern)?))],
        )),
        Expr::Between {
            expr,
            negated: false,
            low,
            high,
        } => Some((
            columna_simple(expr)?,
            vec![
                f("ge", Valor::Uno(literal(low)?)),
                f("le", Valor::Uno(literal(high)?)),
            ],
        )),
        _ => None,
    }
}

/// Las funciones que agregan filas (DuckDB): con una de éstas en la proyección,
/// un `LIMIT` no se empuja.
const AGREGADOS: &[&str] = &[
    "count",
    "count_star",
    "sum",
    "avg",
    "mean",
    "min",
    "max",
    "any_value",
    "arbitrary",
    "first",
    "last",
    "arg_max",
    "arg_min",
    "argmax",
    "argmin",
    "max_by",
    "min_by",
    "array_agg",
    "list",
    "string_agg",
    "group_concat",
    "listagg",
    "bool_and",
    "bool_or",
    "bit_and",
    "bit_or",
    "bit_xor",
    "median",
    "mode",
    "quantile",
    "quantile_cont",
    "quantile_disc",
    "percentile_cont",
    "percentile_disc",
    "stddev",
    "stddev_pop",
    "stddev_samp",
    "variance",
    "var_pop",
    "var_samp",
    "product",
    "histogram",
    "approx_count_distinct",
    "approx_quantile",
    "entropy",
    "kurtosis",
    "skewness",
    "corr",
    "covar_pop",
    "covar_samp",
    "regr_slope",
    "fsum",
    "sumkahan",
    "favg",
    "bitstring_agg",
    "geomean",
    "every",
    "some",
];

/// `COLUMNS(…)` (DuckDB): elige columnas por patrón o expresión.
fn elige_columnas(it: &SelectItem) -> bool {
    struct V(bool);
    impl Visitor for V {
        type Break = ();
        fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
            if let Expr::Function(f) = e
                && f.name.to_string().eq_ignore_ascii_case("columns")
            {
                self.0 = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
    }
    let mut v = V(false);
    let _ = it.visit(&mut v);
    v.0
}

fn agrega(it: &SelectItem) -> bool {
    struct V(bool);
    impl Visitor for V {
        type Break = ();
        fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
            if let Expr::Function(f) = e {
                let n = f.name.to_string().to_lowercase();
                let n = n.rsplit('.').next().unwrap_or(&n).to_string();
                if f.over.is_some() || AGREGADOS.contains(&n.as_str()) {
                    self.0 = true;
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        }
    }
    let mut v = V(false);
    let _ = it.visit(&mut v);
    v.0
}

/// Las columnas que una expresión mira, a su nivel, y sus subconsultas (que se
/// analizan aparte, con su ámbito).
#[derive(Default)]
struct Mira {
    columnas: Vec<Vec<String>>,
    subconsultas: Vec<Query>,
    hondo: usize,
}

impl Visitor for Mira {
    type Break = ();
    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        if self.hondo == 0 {
            self.subconsultas.push(q.clone());
        }
        self.hondo += 1;
        ControlFlow::Continue(())
    }
    fn post_visit_query(&mut self, _: &Query) -> ControlFlow<()> {
        self.hondo -= 1;
        ControlFlow::Continue(())
    }
    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        if self.hondo == 0 {
            match e {
                Expr::Identifier(i) => self.columnas.push(vec![i.value.clone()]),
                Expr::CompoundIdentifier(is) => self
                    .columnas
                    .push(is.iter().map(|i| i.value.clone()).collect()),
                _ => {}
            }
        }
        ControlFlow::Continue(())
    }
}

/// ¿Nombra un factor que no se analiza alguna `Table` o `View` del árbol?
fn lee_del_arbol(tf: &TableFactor, pkg: &Package) -> bool {
    struct V<'a>(&'a Package, bool);
    impl Visitor for V<'_> {
        type Break = ();
        fn pre_visit_relation(&mut self, n: &ObjectName) -> ControlFlow<()> {
            let qn = partes(n).join(".");
            if self.0.table(&qn).is_some() || self.0.view(&qn).is_some() {
                self.1 = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
    }
    let mut v = V(pkg, false);
    let _ = tf.visit(&mut v);
    v.1
}

// ── la salida en JSON ────────────────────────────────────────────────────────

impl Filtro {
    pub fn json(&self) -> Json {
        let mut m = vec![
            ("columna", Json::s(self.columna.as_str())),
            ("operador", Json::s(self.operador.as_str())),
        ];
        match &self.valor {
            Valor::Uno(v) => m.push(("valor", Json::s(v.as_str()))),
            Valor::Lista(vs) => m.push((
                "valor",
                Json::Arr(vs.iter().map(|v| Json::s(v.as_str())).collect()),
            )),
            Valor::Ninguno => {}
        }
        Json::obj(m)
    }
}

impl Lectura {
    pub fn json(&self) -> Json {
        let mut m = vec![
            ("tabla", Json::s(self.tabla.as_str())),
            ("fuente", Json::s(self.fuente.as_str())),
            ("tipo", Json::s(self.tipo.as_str())),
            ("env", Json::s(self.env.as_str())),
            ("objeto", Json::s(self.objeto.as_str())),
            (
                "columnas",
                Json::Arr(self.columnas.iter().map(|c| Json::s(c.as_str())).collect()),
            ),
            (
                "empujados",
                Json::Arr(self.empujados.iter().map(Filtro::json).collect()),
            ),
            (
                "enElMotor",
                Json::Arr(
                    self.en_el_motor
                        .iter()
                        .map(|c| Json::s(c.as_str()))
                        .collect(),
                ),
            ),
            ("fullScan", Json::s(self.full_scan.as_str())),
            ("presupuesto", Json::Bool(self.presupuesto)),
            ("apariciones", Json::Int(self.apariciones as i64)),
            (
                "avisos",
                Json::Arr(self.avisos.iter().map(|c| Json::s(c.as_str())).collect()),
            ),
        ];
        if let Some(l) = self.limit {
            m.push(("limit", Json::Int(l as i64)));
        }
        if !self.orden.is_empty() {
            m.push((
                "orderBy",
                Json::Arr(
                    self.orden
                        .iter()
                        .map(|(c, d)| {
                            Json::obj([("columna", Json::s(c.as_str())), ("desc", Json::Bool(*d))])
                        })
                        .collect(),
                ),
            ));
        }
        Json::obj(m)
    }
}

impl Reparto {
    pub fn json(&self) -> Json {
        Json::obj([
            ("ok", Json::Bool(true)),
            ("entendida", Json::Bool(self.entendida)),
            (
                "lecturas",
                Json::Arr(self.lecturas.iter().map(Lectura::json).collect()),
            ),
            (
                "avisos",
                Json::Arr(self.avisos.iter().map(|c| Json::s(c.as_str())).collect()),
            ),
        ])
    }
}

#[cfg(test)]
#[path = "reparto_pruebas.rs"]
mod pruebas;
