//! **La sesión de cada consulta** (0081 §7.4, propiedad 1): vive lo que la consulta.
//!
//! - Nace de la **plantilla** del motor, que se construye una vez por proceso: clonarla cuesta decenas de
//!   microsegundos (medido en S0), y comparte el `RuntimeEnv` (la memoria, el disco, las cachés).
//! - El [`Contexto`] sólo cambia una lista cerrada de claves ([`POR_CONSULTA`]) y pone el id.
//! - Después, **inmutable**: una consulta no cambia su propia configuración (`SET`) ni crea objetos (DDL);
//!   eso es de quien la abre y del catálogo.
//! - Antes de planificar, el zócalo de catálogo da la lista de catálogos de **esa** consulta.
//! - **Planificar da un [`Plan`]** (0081 §7.5.2, C2): lo que planifica DataFusion, o una sentencia de ORE
//!   ([`crate::sentencia`]). Lo que se admite depende de la superficie: una que sólo lee no escribe (el DML de
//!   DataFusion ni `create dataset`), y ninguna cambia su sesión (`SET`) ni crea objetos con el DDL de DataFusion.

use crate::fallo::Fallo;
use crate::opciones::fijar;
use crate::sentencia::{self, Sentencia};
use crate::zocalos::Catalogo;
use datafusion::common::TableReference;
use datafusion::execution::{SessionState, SessionStateBuilder};
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SQLOptions;
use datafusion::sql::parser::Statement;
use datafusion::sql::planner::object_name_to_table_reference;
use datafusion::sql::sqlparser::ast;
use std::sync::Arc;

/// Las claves que una consulta puede cambiar. Lo demás es del proceso.
pub const POR_CONSULTA: &[&str] = &[
    "datafusion.execution.batch_size",
    "datafusion.execution.time_zone",
    "datafusion.catalog.default_catalog",
    "datafusion.catalog.default_schema",
];

/// El schema de una base cuando no se nombra (0038): un nombre de una parte es `<base>.default.<nombre>`.
pub const SCHEMA_POR_DEFECTO: &str = "default";

/// Lo que una consulta trae al abrirse.
#[derive(Debug, Clone, Default)]
pub struct Contexto {
    /// Su id: el de la sesión, el de su ejecución.
    pub id: String,
    /// Las claves que cambia, de [`POR_CONSULTA`].
    pub ajustes: Vec<(String, String)>,
    /// Si la superficie que la abre escribe. Por defecto, no: sólo lee.
    pub escribe: bool,
}

impl Contexto {
    pub fn nuevo(id: impl Into<String>) -> Self {
        Contexto {
            id: id.into(),
            ajustes: Vec::new(),
            escribe: false,
        }
    }

    /// **La base en la que trabaja** (0081 §7.5.2): su catálogo por defecto, con el schema por defecto de ORE.
    /// Así `schema.nombre` y `nombre` se resuelven como en DataFusion, dentro de la base.
    pub fn en_base(self, base: impl Into<String>) -> Self {
        self.con("datafusion.catalog.default_catalog", base)
            .con("datafusion.catalog.default_schema", SCHEMA_POR_DEFECTO)
    }

    /// La abre una superficie que escribe.
    pub fn que_escribe(mut self) -> Self {
        self.escribe = true;
        self
    }

    pub fn con(mut self, clave: impl Into<String>, valor: impl Into<String>) -> Self {
        self.ajustes.push((clave.into(), valor.into()));
        self
    }
}

/// La sesión de una consulta.
#[derive(Debug, Clone)]
pub struct Sesion {
    estado: SessionState,
    id: String,
    escribe: bool,
    catalogo: Arc<dyn Catalogo>,
}

/// **Lo que se planificó.**
#[derive(Debug, Clone)]
pub enum Plan {
    /// Lo que planifica DataFusion: una consulta, o un DML (`INSERT`, `DELETE`…) que cumple el `TableProvider` de
    /// su tabla.
    Logico(LogicalPlan),
    /// `create [or replace] dataset <nombre> as <consulta>`: con `or replace` sustituye lo que hubiera; sin él,
    /// el dataset no puede existir. La consulta ya está planificada.
    CrearDataset {
        nombre: TableReference,
        reemplazar: bool,
        consulta: LogicalPlan,
    },
}

impl Plan {
    /// El plan de DataFusion, si es lo que es.
    pub fn logico(self) -> Result<LogicalPlan, Fallo> {
        match self {
            Plan::Logico(p) => Ok(p),
            Plan::CrearDataset { nombre, .. } => Err(Fallo::Config(format!(
                "`create dataset {nombre}` es una escritura de ORE, no un plan de DataFusion"
            ))),
        }
    }
}

impl Sesion {
    /// Clona la plantilla y aplica el contexto.
    pub(crate) fn de(
        plantilla: &SessionState,
        contexto: &Contexto,
        catalogo: Arc<dyn Catalogo>,
    ) -> Result<Sesion, Fallo> {
        let mut estado = plantilla.clone();
        for (clave, valor) in &contexto.ajustes {
            if !POR_CONSULTA.contains(&clave.as_str()) {
                return Err(Fallo::Config(format!(
                    "`{clave}` no se cambia por consulta: es del proceso (las que sí: {POR_CONSULTA:?})"
                )));
            }
            fijar(estado.config_mut(), clave, valor)?;
        }
        // El id se pone al planificar, en la misma reconstrucción que instala los catálogos: reconstruir el
        // estado cuesta ~160 µs y un clon ~40 (medido en S2·5), así que se reconstruye una vez, no dos.
        Ok(Sesion {
            estado,
            id: contexto.id.clone(),
            escribe: contexto.escribe,
            catalogo,
        })
    }

    /// El id de la consulta.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// El estado de DataFusion de esta sesión, para leerlo.
    pub fn estado(&self) -> &SessionState {
        &self.estado
    }

    /// **Planificar**: analizar (con las sentencias de ORE), resolver los nombres por el zócalo de catálogo y
    /// planificar, con la política de la superficie. El plan queda atado a los catálogos que se resolvieron para
    /// esta consulta.
    pub async fn planificar(&mut self, sql: &str) -> Result<Plan, Fallo> {
        let opciones = self.estado.config().options();
        let normalizar = opciones.sql_parser.enable_ident_normalization;
        let sentencia = sentencia::analizar(
            sql,
            &opciones.sql_parser.dialect,
            opciones.sql_parser.recursion_limit.get(),
        )?;
        let (frase, destino) = match sentencia {
            Sentencia::DataFusion(frase) => (frase, None),
            Sentencia::CrearDataset {
                nombre,
                reemplazar,
                consulta,
            } => {
                if !self.escribe {
                    return Err(Fallo::Config(format!(
                        "esta superficie sólo lee: `create dataset {nombre}` escribe"
                    )));
                }
                let nombre = object_name_to_table_reference(nombre, normalizar)?;
                (consulta, Some((nombre, reemplazar)))
            }
        };
        if let Statement::Statement(f) = &frase
            && matches!(**f, ast::Statement::CreateTable(_))
        {
            return Err(Fallo::Config(
                "en ORE un dataset se crea con `create [or replace] dataset <nombre> as <consulta>`;                  `CREATE TABLE` no existe"
                    .into(),
            ));
        }
        let mut referencias = self.estado.resolve_table_references(&frase)?;
        if let Some((nombre, _)) = &destino {
            referencias.push(nombre.clone());
        }
        let catalogos = self.catalogo.preparar(&self.estado, &referencias).await?;
        // `new_from_existing` NO conserva el id de la sesión: le da un UUID nuevo (medido en S2·7). Se pone
        // aquí, con los catálogos de la consulta, en una sola reconstrucción.
        self.estado = SessionStateBuilder::new_from_existing(self.estado.clone())
            .with_catalog_list(catalogos)
            .with_session_id(self.id.clone())
            .build();
        // El instante de la sentencia: `now()` vale lo mismo en toda ella y se fija aquí, como hace
        // `SessionContext::state`. Sin marcarlo, el optimizador no lo sustituye y `now()` falla al ejecutarse
        // (lo encontró la vara de ORE en C1).
        self.estado.mark_start_execution();
        let plan = self.estado.statement_to_plan(frase).await?;
        if let LogicalPlan::Copy(_) = plan {
            return Err(Fallo::Config(
                "`COPY` escribe ficheros fuera del lago; en ORE se escribe en un dataset".into(),
            ));
        }
        if matches!(plan, LogicalPlan::Dml(_)) && !self.escribe {
            return Err(Fallo::Config(format!(
                "esta superficie sólo lee: `{}` escribe",
                sql.trim()
            )));
        }
        SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(self.escribe)
            .with_allow_statements(false)
            .verify_plan(&plan)
            .map_err(|e| {
                Fallo::Config(format!(
                    "una consulta no cambia su sesión ni crea objetos: {e}"
                ))
            })?;
        Ok(match destino {
            None => Plan::Logico(plan),
            Some((nombre, reemplazar)) => Plan::CrearDataset {
                nombre,
                reemplazar,
                consulta: plan,
            },
        })
    }
}
