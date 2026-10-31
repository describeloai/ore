//! **La sesión de cada consulta** (0081 §7.4, propiedad 1): vive lo que la consulta.
//!
//! - Nace de la **plantilla** del motor, que se construye una vez por proceso: clonarla cuesta decenas de
//!   microsegundos (medido en S0), y comparte el `RuntimeEnv` (la memoria, el disco, las cachés).
//! - El [`Contexto`] sólo cambia una lista cerrada de claves ([`POR_CONSULTA`]) y pone el id.
//! - Después, **inmutable**: una consulta no cambia su propia configuración (`SET`) ni crea objetos (DDL);
//!   eso es de quien la abre y del catálogo.
//! - Antes de planificar, el zócalo de catálogo da la lista de catálogos de **esa** consulta.
//! - **Planificar da un [`Plan`]** (0081 §7.5.2, C2 y C2b): lo que planifica DataFusion, una escritura de ORE o un
//!   cambio del catálogo ([`crate::sentencia`], [`crate::ddl`]). Lo que se admite depende de la superficie: una que
//!   sólo lee no escribe ni cambia el catálogo, y ninguna cambia su sesión (`SET`) ni usa el DDL de DataFusion.

use crate::ddl::{CambioDeCatalogo, Objetivo, Retirable};
use crate::fallo::Fallo;
use crate::opciones::fijar;
use crate::sentencia::{self, Ddl, Sentencia};
use crate::zocalos::Catalogo;
use datafusion::common::{ResolvedTableReference, TableReference};
use datafusion::execution::{SessionState, SessionStateBuilder};
use datafusion::logical_expr::DdlStatement;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SQLOptions;
use datafusion::sql::parser::Statement;
use datafusion::sql::planner::object_name_to_table_reference;
use datafusion::sql::sqlparser::ast::ObjectName;
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
    /// el dataset no puede existir (con `if not exists`, si existe no hace nada). La consulta ya está planificada.
    CrearDataset {
        nombre: ResolvedTableReference,
        reemplazar: bool,
        si_no_existe: bool,
        consulta: LogicalPlan,
    },
    /// Un cambio del catálogo, con su forma y su plan ([`crate::ddl`]).
    Catalogo(CambioDeCatalogo),
}

impl Plan {
    /// El plan de DataFusion, si es lo que es.
    pub fn logico(self) -> Result<LogicalPlan, Fallo> {
        match self {
            Plan::Logico(p) => Ok(p),
            Plan::CrearDataset { nombre, .. } => Err(Fallo::Config(format!(
                "`create dataset {nombre}` es una escritura de ORE, no un plan de DataFusion"
            ))),
            Plan::Catalogo(c) => Err(Fallo::Config(format!(
                "`{c}` es un cambio del catálogo de ORE, no un plan de DataFusion"
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
        let sentencia = sentencia::analizar(
            sql,
            &opciones.sql_parser.dialect,
            opciones.sql_parser.recursion_limit.get(),
        )?;
        if !matches!(sentencia, Sentencia::DataFusion(_)) && !self.escribe {
            return Err(Fallo::Config(format!(
                "esta superficie sólo lee: `{}` cambia el lago o el catálogo",
                sql.trim()
            )));
        }
        match sentencia {
            Sentencia::DataFusion(frase) => {
                let plan = self.resolver_y_planificar(frase, None).await?;
                if let LogicalPlan::Copy(_) = plan {
                    return Err(Fallo::Config(
                        "`COPY` escribe ficheros fuera del lago; en ORE se escribe en un dataset"
                            .into(),
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
                Ok(Plan::Logico(plan))
            }
            Sentencia::CrearDataset {
                nombre,
                reemplazar,
                si_no_existe,
                consulta,
            } => {
                let nombre = self.objeto(nombre)?;
                let consulta = self.consulta(consulta, Some(&nombre)).await?;
                Ok(Plan::CrearDataset {
                    nombre,
                    reemplazar,
                    si_no_existe,
                    consulta,
                })
            }
            Sentencia::Ddl(d) => Ok(Plan::Catalogo(self.ddl(d).await?)),
        }
    }

    /// Resuelve los nombres de `frase` (y `destino`, si lo hay) por el zócalo de catálogo, y la planifica.
    async fn resolver_y_planificar(
        &mut self,
        frase: Statement,
        destino: Option<&ResolvedTableReference>,
    ) -> Result<LogicalPlan, Fallo> {
        let mut referencias = self.estado.resolve_table_references(&frase)?;
        if let Some(d) = destino {
            referencias.push(TableReference::from(d.clone()));
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
        Ok(self.estado.statement_to_plan(frase).await?)
    }

    /// La consulta de una sentencia de ORE (`as select …`): sólo lee.
    async fn consulta(
        &mut self,
        consulta: Statement,
        destino: Option<&ResolvedTableReference>,
    ) -> Result<LogicalPlan, Fallo> {
        let plan = self.resolver_y_planificar(consulta, destino).await?;
        SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false)
            .with_allow_statements(false)
            .verify_plan(&plan)
            .map_err(|e| Fallo::Config(format!("la consulta de dentro sólo lee: {e}")))?;
        Ok(plan)
    }

    fn normalizar(&self) -> bool {
        self.estado
            .config()
            .options()
            .sql_parser
            .enable_ident_normalization
    }

    /// Un nombre de tres partes, con la base y el schema de la sesión donde falten.
    fn objeto(&self, nombre: ObjectName) -> Result<ResolvedTableReference, Fallo> {
        let c = &self.estado.config().options().catalog;
        Ok(object_name_to_table_reference(nombre, self.normalizar())?
            .resolve(&c.default_catalog, &c.default_schema))
    }

    /// `[b.]s`: un schema, en la base de la sesión si no la nombra.
    fn schema(&self, nombre: ObjectName) -> Result<(String, String), Fallo> {
        let texto = nombre.to_string();
        match object_name_to_table_reference(nombre, self.normalizar())? {
            TableReference::Bare { table } => Ok((
                self.estado
                    .config()
                    .options()
                    .catalog
                    .default_catalog
                    .clone(),
                table.to_string(),
            )),
            TableReference::Partial { schema, table } => {
                Ok((schema.to_string(), table.to_string()))
            }
            TableReference::Full { .. } => Err(Fallo::Config(format!(
                "`{texto}` no es un schema: un schema es `[base.]schema`"
            ))),
        }
    }

    /// Una base: un nombre de una parte.
    fn base(&self, nombre: ObjectName) -> Result<String, Fallo> {
        let texto = nombre.to_string();
        match object_name_to_table_reference(nombre, self.normalizar())? {
            TableReference::Bare { table } => Ok(table.to_string()),
            _ => Err(Fallo::Config(format!(
                "`{texto}` no es una base: una base es un nombre de una parte"
            ))),
        }
    }

    async fn ddl(&mut self, d: Ddl) -> Result<CambioDeCatalogo, Fallo> {
        Ok(match d {
            Ddl::CrearBase {
                nombre,
                clase,
                si_no_existe,
                origen,
            } => CambioDeCatalogo::CrearBase {
                base: self.base(nombre)?,
                clase,
                si_no_existe,
                origen,
            },
            Ddl::CrearSchema {
                nombre,
                si_no_existe,
            } => {
                let (base, schema) = self.schema(nombre)?;
                CambioDeCatalogo::CrearSchema {
                    base,
                    schema,
                    si_no_existe,
                }
            }
            Ddl::CrearDatasetVacio {
                nombre,
                si_no_existe,
                columnas,
            } => {
                let nombre = self.objeto(nombre)?;
                // Las columnas, tipadas por DataFusion (con el planificador de tipos de ORE): las de su
                // `CREATE TABLE`, que se planifica y no se ejecuta.
                let columnas: Vec<String> = columnas.iter().map(|c| c.to_string()).collect();
                let tabla = format!("create table t ({})", columnas.join(", "));
                let dialecto = self.estado.config().options().sql_parser.dialect;
                let frase = self.estado.sql_to_statement(&tabla, &dialecto)?;
                let esquema = match self.estado.statement_to_plan(frase).await? {
                    LogicalPlan::Ddl(DdlStatement::CreateMemoryTable(t)) => {
                        Arc::clone(t.input.schema().inner())
                    }
                    otro => {
                        return Err(Fallo::Config(format!(
                            "las columnas de `{nombre}` no dieron un esquema: {otro}"
                        )));
                    }
                };
                CambioDeCatalogo::CrearDatasetVacio {
                    nombre,
                    si_no_existe,
                    esquema,
                }
            }
            Ddl::CrearVista {
                nombre,
                reemplazar,
                si_no_existe,
                materializada,
                columnas,
                comentario,
                evolucion,
                consulta,
            } => {
                let nombre = self.objeto(nombre)?;
                let sql = consulta.to_string();
                let consulta = self.consulta(consulta, Some(&nombre)).await?;
                let n = consulta.schema().fields().len();
                if !columnas.is_empty() && columnas.len() != n {
                    return Err(Fallo::Config(format!(
                        "la vista `{nombre}` nombra {} columnas y su consulta da {n}",
                        columnas.len()
                    )));
                }
                CambioDeCatalogo::CrearVista {
                    nombre,
                    reemplazar,
                    si_no_existe,
                    materializada,
                    columnas,
                    comentario,
                    evolucion,
                    sql,
                    consulta,
                }
            }
            Ddl::CrearColeccion {
                nombre,
                reemplazar,
                si_no_existe,
                medio,
                formatos,
                comentario,
                consulta,
            } => {
                let nombre = self.objeto(nombre)?;
                let consulta = match consulta {
                    Some(c) => {
                        let sql = c.to_string();
                        Some((sql, self.consulta(c, Some(&nombre)).await?))
                    }
                    None => None,
                };
                CambioDeCatalogo::CrearColeccion {
                    nombre,
                    reemplazar,
                    si_no_existe,
                    medio,
                    formatos,
                    comentario,
                    consulta,
                }
            }
            Ddl::Retirar {
                que,
                nombre,
                si_existe,
                cascada,
            } => {
                let objetivo = match que {
                    Retirable::Base => Objetivo::Base(self.base(nombre)?),
                    Retirable::Schema => {
                        let (base, schema) = self.schema(nombre)?;
                        Objetivo::Schema { base, schema }
                    }
                    Retirable::Dataset | Retirable::Vista | Retirable::Coleccion => {
                        Objetivo::Objeto(self.objeto(nombre)?)
                    }
                };
                CambioDeCatalogo::Retirar {
                    que,
                    objetivo,
                    si_existe,
                    cascada,
                }
            }
        })
    }
}
