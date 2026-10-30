//! **La sesión de cada consulta** (0081 §7.4, propiedad 1): vive lo que la consulta.
//!
//! - Nace de la **plantilla** del motor, que se construye una vez por proceso: clonarla cuesta decenas de
//!   microsegundos (medido en S0), y comparte el `RuntimeEnv` (la memoria, el disco, las cachés).
//! - El [`Contexto`] sólo cambia una lista cerrada de claves ([`POR_CONSULTA`]) y pone el id.
//! - Después, **inmutable**: una consulta no cambia su propia configuración (`SET`) ni crea objetos (DDL);
//!   eso es de quien la abre y del catálogo.
//! - Antes de planificar, el zócalo de catálogo da la lista de catálogos de **esa** consulta.

use crate::fallo::Fallo;
use crate::opciones::fijar;
use crate::zocalos::Catalogo;
use datafusion::execution::{SessionState, SessionStateBuilder};
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SQLOptions;
use std::sync::Arc;

/// Las claves que una consulta puede cambiar. Lo demás es del proceso.
pub const POR_CONSULTA: &[&str] = &[
    "datafusion.execution.batch_size",
    "datafusion.execution.time_zone",
];

/// Lo que una consulta trae al abrirse.
#[derive(Debug, Clone, Default)]
pub struct Contexto {
    /// Su id: el de la sesión, el de su ejecución.
    pub id: String,
    /// Las claves que cambia, de [`POR_CONSULTA`].
    pub ajustes: Vec<(String, String)>,
}

impl Contexto {
    pub fn nuevo(id: impl Into<String>) -> Self {
        Contexto {
            id: id.into(),
            ajustes: Vec::new(),
        }
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
    catalogo: Arc<dyn Catalogo>,
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

    /// **Planificar**: analizar, resolver los nombres por el zócalo de catálogo y planificar, sin `SET` ni
    /// DDL. El plan queda atado a los catálogos que se resolvieron para esta consulta.
    pub async fn planificar(&mut self, sql: &str) -> Result<LogicalPlan, Fallo> {
        let dialecto = self.estado.config().options().sql_parser.dialect;
        let frase = self.estado.sql_to_statement(sql, &dialecto)?;
        let referencias = self.estado.resolve_table_references(&frase)?;
        let catalogos = self.catalogo.preparar(&self.estado, &referencias).await?;
        // `new_from_existing` NO conserva el id de la sesión: le da un UUID nuevo (medido en S2·7). Se pone
        // aquí, con los catálogos de la consulta, en una sola reconstrucción.
        self.estado = SessionStateBuilder::new_from_existing(self.estado.clone())
            .with_catalog_list(catalogos)
            .with_session_id(self.id.clone())
            .build();
        let plan = self.estado.statement_to_plan(frase).await?;
        SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_statements(false)
            .verify_plan(&plan)
            .map_err(|e| {
                Fallo::Config(format!(
                    "una consulta no cambia su sesión ni crea objetos: {e}"
                ))
            })?;
        Ok(plan)
    }
}
