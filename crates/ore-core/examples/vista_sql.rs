//! Prototipo del paso 2 de ADR 0040 (medida del paso 0): de una consulta SQL,
//! lo que lee, lo que proyecta y su linaje por columna (directo, derivado e
//! INDIRECT), con sqlparser y sin motor.
//!
//! Entrada por stdin: bloques separados por una línea `-- @@ <id>`.
//! Salida: una línea por bloque, `<id>\t<json>`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::ops::ControlFlow;

use sqlparser::ast::{
    Expr, FunctionArguments, GroupByExpr, Join, JoinConstraint, JoinOperator, ObjectName,
    ObjectNamePart, Query, Select, SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Statement,
    TableFactor, Visit, Visitor,
};
use sqlparser::dialect::DuckDbDialect;
use sqlparser::parser::Parser;

type Ref = (String, String);

#[derive(Clone, Default, Debug)]
struct Lin {
    dir: BTreeSet<Ref>,
    der: BTreeSet<Ref>,
    /// INDIRECT sólo hacia esta columna (las claves de grupo hacia los agregados).
    ind: BTreeSet<Ref>,
}

impl Lin {
    fn todo(&self) -> BTreeSet<Ref> {
        self.dir.union(&self.der).cloned().collect()
    }
    fn derivada(self) -> Lin {
        Lin {
            dir: BTreeSet::new(),
            der: self.todo(),
            ind: self.ind,
        }
    }
    fn mas(&mut self, o: Lin) {
        self.dir.extend(o.dir);
        self.der.extend(o.der);
        self.ind.extend(o.ind);
    }
}

#[derive(Clone, Default, Debug)]
struct Salida {
    cols: Vec<(String, Lin)>,
    ind: BTreeSet<Ref>,
    lee: BTreeSet<String>,
    avisos: Vec<String>,
    lectoras: Vec<String>,
}

#[derive(Clone, Debug)]
enum Rel {
    Arbol(String),
    Sub(Salida),
}

struct Ambito<'a> {
    rels: Vec<(String, Rel)>,
    padre: Option<&'a Ambito<'a>>,
}

const GENERADORES: &[&str] = &["range", "generate_series", "unnest"];

fn nombre(n: &ObjectName) -> Vec<String> {
    n.0.iter()
        .map(|p| match p {
            ObjectNamePart::Identifier(i) => i.value.clone(),
            other => other.to_string(),
        })
        .collect()
}

/// Referencias de columna de una expresión, sin entrar en subconsultas; las
/// subconsultas se devuelven aparte.
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
                let col = v.last().unwrap().value.clone();
                let q = v[..v.len() - 1]
                    .iter()
                    .map(|i| i.value.clone())
                    .collect::<Vec<_>>()
                    .join(".");
                self.cols.push((Some(q), col));
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

fn resolver(a: &Ambito, q: &Option<String>, col: &str, avisos: &mut Vec<String>) -> Lin {
    let mut out = Lin::default();
    let cmp = |x: &str, y: &str| x.eq_ignore_ascii_case(y);
    match q {
        Some(q) => {
            for (alias, rel) in &a.rels {
                let casa = cmp(alias, q)
                    || matches!(rel, Rel::Arbol(n) if cmp(n, q) || n.to_lowercase().ends_with(&format!(".{}", q.to_lowercase())));
                if casa {
                    return de_rel(rel, col);
                }
            }
        }
        None => {
            if a.rels.len() == 1 {
                return de_rel(&a.rels[0].1, col);
            }
            for (_, rel) in &a.rels {
                if let Rel::Sub(s) = rel
                    && s.cols.iter().any(|(n, _)| cmp(n, col))
                {
                    return de_rel(rel, col);
                }
            }
            let arboles: Vec<&Rel> = a
                .rels
                .iter()
                .map(|(_, r)| r)
                .filter(|r| matches!(r, Rel::Arbol(_)))
                .collect();
            if !arboles.is_empty() {
                if arboles.len() > 1 {
                    avisos.push(format!("ambigua:{col}"));
                }
                for r in arboles {
                    out.mas(de_rel(r, col));
                }
                return out;
            }
        }
    }
    if let Some(p) = a.padre {
        return resolver(p, q, col, avisos);
    }
    avisos.push(format!(
        "sin-fuente:{}{col}",
        q.as_ref().map(|q| format!("{q}.")).unwrap_or_default()
    ));
    out
}

fn de_rel(rel: &Rel, col: &str) -> Lin {
    match rel {
        Rel::Arbol(n) => Lin {
            dir: [(n.clone(), col.to_string())].into(),
            ..Default::default()
        },
        Rel::Sub(s) => s
            .cols
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(col))
            .map(|(_, l)| l.clone())
            .unwrap_or_default(),
    }
}

/// Linaje de una expresión, y lo que sus subconsultas miran (INDIRECT).
fn lin_de(e: &Expr, a: &Ambito, sal: &mut Salida) -> Lin {
    let r = refs_de(e);
    let mut l = Lin::default();
    for (q, c) in &r.cols {
        l.mas(resolver(a, q, c, &mut sal.avisos));
    }
    for sq in &r.subs {
        let s = consulta(sq, a, &BTreeMap::new());
        sal.lee.extend(s.lee.clone());
        sal.lectoras.extend(s.lectoras.clone());
        sal.avisos.extend(s.avisos.clone());
        for (_, cl) in &s.cols {
            l.der.extend(cl.todo());
        }
        l.der.extend(s.ind.clone());
    }
    let directa = matches!(
        e,
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Nested(_)
    );
    if directa { l } else { l.derivada() }
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

fn factor(
    t: &TableFactor,
    padre: &Ambito,
    ctes: &BTreeMap<String, Salida>,
    sal: &mut Salida,
    rels: &mut Vec<(String, Rel)>,
) {
    match t {
        TableFactor::Table {
            name, alias, args, ..
        } => {
            let partes = nombre(name);
            let n = partes.join(".");
            if args.is_some() {
                if GENERADORES.contains(&n.to_lowercase().as_str()) {
                    rels.push((
                        alias.as_ref().map(|a| a.name.value.clone()).unwrap_or(n),
                        Rel::Sub(Salida::default()),
                    ));
                } else {
                    sal.lectoras.push(n);
                }
                return;
            }
            let alias_ = alias
                .as_ref()
                .map(|a| a.name.value.clone())
                .unwrap_or_else(|| partes.last().cloned().unwrap_or_default());
            if partes.len() == 1
                && let Some(s) = ctes.get(&n.to_lowercase())
            {
                sal.ind.extend(s.ind.clone());
                rels.push((alias_, Rel::Sub(s.clone())));
                return;
            }
            sal.lee.insert(n.clone());
            rels.push((alias_, Rel::Arbol(n)));
        }
        TableFactor::Derived {
            subquery, alias, ..
        } => {
            let mut s = consulta(subquery, padre, ctes);
            if let Some(a) = alias {
                for (i, c) in a.columns.iter().enumerate() {
                    if let Some(x) = s.cols.get_mut(i) {
                        x.0 = c.name.value.clone();
                    }
                }
            }
            sal.lee.extend(s.lee.clone());
            sal.lectoras.extend(s.lectoras.clone());
            sal.avisos.extend(s.avisos.clone());
            sal.ind.extend(s.ind.clone());
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
            factor(&table_with_joins.relation, padre, ctes, sal, rels);
            for j in &table_with_joins.joins {
                union(j, padre, ctes, sal, rels);
            }
        }
        TableFactor::Function { name, .. } => sal.lectoras.push(nombre(name).join(".")),
        TableFactor::UNNEST { .. } => {}
        otro => sal.avisos.push(format!("factor:{}", otro)),
    }
}

fn union(
    j: &Join,
    padre: &Ambito,
    ctes: &BTreeMap<String, Salida>,
    sal: &mut Salida,
    rels: &mut Vec<(String, Rel)>,
) {
    factor(&j.relation, padre, ctes, sal, rels);
    if let Some(JoinConstraint::On(e)) = restriccion(&j.join_operator) {
        let a = Ambito {
            rels: rels.clone(),
            padre: Some(padre),
        };
        let l = lin_de(e, &a, sal);
        sal.ind.extend(l.todo());
    }
    if let Some(JoinConstraint::Using(cols)) = restriccion(&j.join_operator) {
        let a = Ambito {
            rels: rels.clone(),
            padre: Some(padre),
        };
        for c in cols {
            let c = nombre(c).join(".");
            for (_, r) in &a.rels {
                sal.ind.extend(de_rel(r, &c).todo());
            }
        }
    }
}

fn select(s: &Select, padre: &Ambito, ctes: &BTreeMap<String, Salida>) -> Salida {
    let mut sal = Salida::default();
    let mut rels = vec![];
    for t in &s.from {
        factor(&t.relation, padre, ctes, &mut sal, &mut rels);
        for j in &t.joins {
            union(j, padre, ctes, &mut sal, &mut rels);
        }
    }
    let a = Ambito {
        rels,
        padre: Some(padre),
    };
    for item in &s.projection {
        match item {
            SelectItem::UnnamedExpr(e) => {
                let n = match e {
                    Expr::Identifier(i) => i.value.clone(),
                    Expr::CompoundIdentifier(v) => v.last().unwrap().value.clone(),
                    otro => otro.to_string(),
                };
                let l = lin_de(e, &a, &mut sal);
                sal.cols.push((n, l));
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                let l = lin_de(expr, &a, &mut sal);
                sal.cols.push((alias.value.clone(), l));
            }
            SelectItem::Wildcard(_) => {
                for (_, r) in &a.rels {
                    estrella(r, &mut sal);
                }
            }
            SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::ObjectName(n), _) => {
                let q = nombre(n).join(".");
                if let Some((_, r)) = a.rels.iter().find(|(al, _)| al.eq_ignore_ascii_case(&q)) {
                    estrella(&r.clone(), &mut sal);
                }
            }
            otro => sal.avisos.push(format!("proyeccion:{otro}")),
        }
    }
    let mira = |e: &Expr, sal: &mut Salida| {
        let l = lin_de(e, &a, sal);
        sal.ind.extend(l.todo());
    };
    if let Some(w) = &s.selection {
        mira(w, &mut sal);
    }
    // Una clave de grupo decide qué filas salen y cuántas se juntan: deja su
    // arista INDIRECT hacia los agregados; hacia todas las salidas sólo si no
    // se proyecta (entonces decide filas que no se ven). Así lo hace el motor.
    if let GroupByExpr::Expressions(es, _) = &s.group_by {
        let mut claves: BTreeSet<Ref> = BTreeSet::new();
        for e in es {
            claves.extend(lin_de(e, &a, &mut sal).todo());
        }
        let es_clave = |l: &Lin| l.der.is_empty() && !l.dir.is_empty() && l.dir.is_subset(&claves);
        let proyectadas: BTreeSet<Ref> = sal
            .cols
            .iter()
            .filter(|(_, l)| es_clave(l))
            .flat_map(|(_, l)| l.dir.clone())
            .collect();
        for r in &claves {
            if proyectadas.contains(r) {
                for (_, l) in sal.cols.iter_mut() {
                    if !es_clave(l) {
                        l.ind.insert(r.clone());
                    }
                }
            } else {
                sal.ind.insert(r.clone());
            }
        }
    }
    // Un `HAVING` recorta por un agregado, y el agregado sale de sus grupos:
    // mira lo que nombra y además las claves, hacia todas las salidas.
    if let Some(h) = &s.having {
        mira(h, &mut sal);
        if let GroupByExpr::Expressions(es, _) = &s.group_by {
            for e in es {
                mira(e, &mut sal);
            }
        }
    }
    if let Some(q) = &s.qualify {
        mira(q, &mut sal);
    }
    sal
}

fn estrella(r: &Rel, sal: &mut Salida) {
    match r {
        Rel::Arbol(n) => sal.cols.push((
            "*".into(),
            Lin {
                dir: [(n.clone(), "*".to_string())].into(),
                ..Default::default()
            },
        )),
        Rel::Sub(s) => sal.cols.extend(s.cols.clone()),
    }
}

fn cuerpo(b: &SetExpr, padre: &Ambito, ctes: &BTreeMap<String, Salida>) -> Salida {
    match b {
        SetExpr::Select(s) => select(s, padre, ctes),
        SetExpr::Query(q) => consulta(q, padre, ctes),
        SetExpr::SetOperation { left, right, .. } => {
            let mut l = cuerpo(left, padre, ctes);
            let r = cuerpo(right, padre, ctes);
            for (i, (_, lr)) in r.cols.into_iter().enumerate() {
                if let Some((_, ll)) = l.cols.get_mut(i) {
                    ll.mas(lr);
                }
            }
            l.ind.extend(r.ind);
            l.lee.extend(r.lee);
            l.avisos.extend(r.avisos);
            l.lectoras.extend(r.lectoras);
            l
        }
        SetExpr::Values(_) => Salida::default(),
        otro => Salida {
            avisos: vec![format!("cuerpo:{otro}")],
            ..Default::default()
        },
    }
}

fn consulta(q: &Query, padre: &Ambito, ctes: &BTreeMap<String, Salida>) -> Salida {
    let mut ctes = ctes.clone();
    let mut extra = Salida::default();
    if let Some(w) = &q.with {
        for c in &w.cte_tables {
            let mut s = consulta(&c.query, padre, &ctes);
            for (i, col) in c.alias.columns.iter().enumerate() {
                if let Some(x) = s.cols.get_mut(i) {
                    x.0 = col.name.value.clone();
                }
            }
            extra.lee.extend(s.lee.clone());
            extra.lectoras.extend(s.lectoras.clone());
            extra.avisos.extend(s.avisos.clone());
            ctes.insert(c.alias.name.value.to_lowercase(), s);
        }
    }
    let mut s = cuerpo(&q.body, padre, &ctes);
    s.lee.extend(extra.lee);
    s.lectoras.extend(extra.lectoras);
    s.avisos.extend(extra.avisos);
    s
}

fn js(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn refs_js(r: &BTreeSet<Ref>) -> String {
    format!(
        "[{}]",
        r.iter()
            .map(|(t, c)| format!("[{},{}]", js(t), js(c)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn analizar(texto: &str) -> String {
    let st = match Parser::parse_sql(&DuckDbDialect {}, texto) {
        Ok(s) => s,
        Err(e) => return format!("{{\"error\":{}}}", js(&e.to_string())),
    };
    if st.len() != 1 {
        return format!(
            "{{\"error\":{}}}",
            js(&format!("OOS2038: {} sentencias", st.len()))
        );
    }
    let Statement::Query(q) = &st[0] else {
        return format!("{{\"error\":{}}}", js("OOS2038: no es un SELECT"));
    };
    let raiz = Ambito {
        rels: vec![],
        padre: None,
    };
    let s = consulta(q, &raiz, &BTreeMap::new());
    let _ = FunctionArguments::None;
    format!(
        "{{\"lee\":[{}],\"lectoras\":[{}],\"avisos\":[{}],\"ind\":{},\"cols\":[{}]}}",
        s.lee.iter().map(|x| js(x)).collect::<Vec<_>>().join(","),
        s.lectoras
            .iter()
            .map(|x| js(x))
            .collect::<Vec<_>>()
            .join(","),
        s.avisos.iter().map(|x| js(x)).collect::<Vec<_>>().join(","),
        refs_js(&s.ind),
        s.cols
            .iter()
            .map(|(n, l)| format!(
                "[{},{},{},{}]",
                js(n),
                refs_js(&l.dir),
                refs_js(&l.der),
                refs_js(&l.ind)
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let mut id: Option<String> = None;
    let mut buf = String::new();
    let volcar = |id: &Option<String>, buf: &str| {
        if let Some(i) = id {
            println!("{i}\t{}", analizar(buf));
        }
    };
    for linea in entrada.lines() {
        if let Some(r) = linea.strip_prefix("-- @@ ") {
            volcar(&id, &buf);
            id = Some(r.trim().to_string());
            buf.clear();
        } else {
            buf.push_str(linea);
            buf.push('\n');
        }
    }
    volcar(&id, &buf);
}
