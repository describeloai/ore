//! **El viaje en el tiempo** (0081 §7.5.2 y §7.5.3, decisión 3; C9): `at('b.s.t', snapshot => n)` o
//! `at('b.s.t', timestamp => '2026-10-31T10:00:00Z')`, en un `FROM`, es la tabla `b.s.t` como estaba en ese snapshot o
//! en ese momento. DataFusion no tiene `AS OF` en su dialecto, y una relación de ORE entra por el `RelationPlanner`
//! que DataFusion declara, que ve los argumentos con nombre.
//!
//! El `RelationPlanner` es síncrono, así que la sesión deja hecho lo que es E/S: recoge los destinos de `at(…)` del
//! AST ([`destinos`]), los resuelve en la foto de la sentencia junto a los demás nombres y los abre; el planificador
//! ([`ViajeEnElTiempo`]) sólo elige el snapshot, con el zócalo de catálogo ([`Catalogo::en_el_tiempo`]).

use crate::zocalos::Catalogo;
use datafusion::catalog::TableProvider;
use datafusion::common::{Result, TableReference, plan_datafusion_err, plan_err};
use datafusion::datasource::provider_as_source;
use datafusion::logical_expr::LogicalPlanBuilder;
use datafusion::logical_expr::planner::{
    PlannedRelation, RelationPlanner, RelationPlannerContext, RelationPlanning,
};
use datafusion::sql::parser::Statement;
use datafusion::sql::sqlparser::ast::{
    self, Expr, FunctionArg, FunctionArgExpr, ObjectName, TableFactor, Value, Visit, Visitor,
};
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::Arc;

/// La relación de ORE.
pub const AT: &str = "at";

/// **A qué punto de su historia.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punto {
    /// Un snapshot por su id.
    Snapshot(i64),
    /// El último snapshot a esa hora (milisegundos desde 1970, en UTC).
    Momento(i64),
}

fn es_at(nombre: &ObjectName) -> bool {
    nombre.0.len() == 1 && nombre.to_string().eq_ignore_ascii_case(AT)
}

fn texto(e: &Expr) -> Option<String> {
    match e {
        Expr::Value(v) => match &v.value {
            Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => Some(s.clone()),
            Value::Number(n, _) => Some(n.clone()),
            _ => None,
        },
        Expr::UnaryOp {
            op: ast::UnaryOperator::Minus,
            expr,
        } => texto(expr).map(|t| format!("-{t}")),
        _ => None,
    }
}

/// **Los argumentos de `at`**: el nombre (un texto) y un punto con nombre, `snapshot => n` o `timestamp => '…'`.
fn argumentos(args: &[FunctionArg]) -> Result<(String, Punto)> {
    let uso = "`at('base.schema.nombre', snapshot => <id>)` o `at('base.schema.nombre', timestamp => '<instante>')`";
    let [FunctionArg::Unnamed(FunctionArgExpr::Expr(n)), punto] = args else {
        return plan_err!("{uso}");
    };
    let nombre = texto(n).ok_or_else(|| plan_datafusion_err!("{uso}: el nombre, como texto"))?;
    // `snapshot => n`: según el dialecto, el nombre llega como identificador o como expresión.
    let (clave, v) = match punto {
        FunctionArg::Named {
            name,
            arg: FunctionArgExpr::Expr(v),
            ..
        } => (name.value.clone(), v),
        FunctionArg::ExprNamed {
            name: Expr::Identifier(name),
            arg: FunctionArgExpr::Expr(v),
            ..
        } => (name.value.clone(), v),
        _ => return plan_err!("{uso}"),
    };
    let v = texto(v).ok_or_else(|| plan_datafusion_err!("{uso}: el punto, como literal"))?;
    let punto = match clave.to_ascii_lowercase().as_str() {
        "snapshot" => Punto::Snapshot(
            v.parse()
                .map_err(|_| plan_datafusion_err!("`snapshot => {v}` no es un id de snapshot"))?,
        ),
        "timestamp" => Punto::Momento(
            datafusion::arrow::compute::kernels::cast_utils::string_to_timestamp_nanos(&v)
                .map_err(|e| plan_datafusion_err!("`timestamp => '{v}'` no es un instante: {e}"))?
                / 1_000_000,
        ),
        otro => return plan_err!("{uso}: `{otro}` no es un punto"),
    };
    Ok((nombre, punto))
}

/// **Los destinos de `at(…)`** de una sentencia, como los escribió: para resolverlos y abrirlos en su foto antes de
/// planificar.
pub fn destinos(frase: &Statement) -> Vec<TableReference> {
    struct Destinos(Vec<TableReference>);
    impl Visitor for Destinos {
        type Break = ();
        fn pre_visit_table_factor(&mut self, t: &TableFactor) -> ControlFlow<()> {
            // Sólo el nombre: si lo demás está mal, lo dice el planificador.
            if let TableFactor::Table {
                name,
                args: Some(a),
                ..
            } = t
                && es_at(name)
                && let Some(FunctionArg::Unnamed(FunctionArgExpr::Expr(n))) = a.args.first()
                && let Some(nombre) = texto(n)
            {
                self.0.push(TableReference::parse_str(&nombre));
            }
            ControlFlow::Continue(())
        }
    }
    let mut d = Destinos(Vec::new());
    if let Statement::Statement(s) = frase {
        let _ = s.visit(&mut d);
    }
    d.0
}

/// **El planificador de `at(…)`** de una sentencia: con sus destinos ya abiertos, por nombre entero.
#[derive(Debug)]
pub struct ViajeEnElTiempo {
    pub tablas: HashMap<String, Arc<dyn TableProvider>>,
    pub catalogo: Arc<dyn Catalogo>,
}

impl RelationPlanner for ViajeEnElTiempo {
    fn plan_relation(
        &self,
        relacion: TableFactor,
        contexto: &mut dyn RelationPlannerContext,
    ) -> Result<RelationPlanning> {
        let TableFactor::Table {
            name,
            args: Some(args),
            alias,
            ..
        } = &relacion
        else {
            return Ok(RelationPlanning::Original(Box::new(relacion)));
        };
        if !es_at(name) {
            return Ok(RelationPlanning::Original(Box::new(relacion)));
        }
        let (nombre, punto) = argumentos(&args.args)?;
        let c = &contexto.context_provider().options().catalog;
        let r = TableReference::parse_str(&nombre).resolve(&c.default_catalog, &c.default_schema);
        let tabla = self
            .tablas
            .get(&r.to_string())
            .ok_or_else(|| plan_datafusion_err!("table '{r}' not found"))?;
        let viajada = self
            .catalogo
            .en_el_tiempo(tabla, &punto)
            .map_err(|e| plan_datafusion_err!("`{r}`: {e}"))?;
        // Un nombre propio para el escaneo de entonces, bajo el de la tabla: DataFusion compara dos `TableScan` por su
        // nombre y no por su tabla, y fundiría `a` y `at('a', …)` de la misma sentencia en uno (lo encontró C9).
        let punto = match punto {
            Punto::Snapshot(id) => format!("snapshot={id}"),
            Punto::Momento(ms) => format!("timestamp={ms}"),
        };
        let plan = LogicalPlanBuilder::scan(
            TableReference::bare(format!("{}@{punto}", r.table)),
            provider_as_source(viajada),
            None,
        )?
        .alias(TableReference::bare(r.table.to_string()))?
        .build()?;
        Ok(RelationPlanning::Planned(Box::new(PlannedRelation::new(
            plan,
            alias.clone(),
        ))))
    }
}
