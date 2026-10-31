//! **El zócalo de catálogo de ORE** (0081 §7.5.2 y §7.5.3, decisión 2). Antes de planificar, lee **la foto** una
//! vez y da los catálogos de esa consulta: cada base de la foto es un catálogo de DataFusion, cada schema un
//! schema y cada nombre una tabla. Una sentencia ve una sola foto para todas sus tablas; la siguiente lee otra y
//! ve lo que escribió la anterior.
//!
//! - **Un dataset** se abre en su `metadata_location`, el snapshot de la foto ([`TablaOre`]).
//! - **Una vista** se planifica al nombrarla, sobre la misma foto, con su base y su schema por defecto: es un
//!   `ViewTable` de DataFusion. Una vista que se nombra a sí misma, directa o indirectamente, es un error.

use crate::escritura::{Escribe, Puerta};
use crate::foto::{Foto, Fuente, Nombre, Objeto};
use crate::tabla::TablaOre;
use async_trait::async_trait;
use datafusion::catalog::{CatalogProvider, CatalogProviderList, SchemaProvider, TableProvider};
use datafusion::common::{DataFusionError, Result, TableReference, plan_err};
use datafusion::datasource::{TableType, ViewTable};
use datafusion::execution::{SessionState, SessionStateBuilder};
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SQLOptions;
use ore_runtime::Fallo;
use ore_runtime::zocalos::Catalogo;
use ore_store::lago::Lago;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

/// **El catálogo de ORE**: de dónde sale la foto, el lago en el que están sus datasets y, si se escribe, la puerta
/// del commit.
#[derive(Debug, Clone)]
pub struct CatalogoOre {
    fuente: Arc<dyn Fuente>,
    lago: Lago,
    puerta: Option<Arc<dyn Puerta>>,
}

impl CatalogoOre {
    /// Un catálogo que sólo lee.
    pub fn new(fuente: Arc<dyn Fuente>, lago: Lago) -> CatalogoOre {
        CatalogoOre {
            fuente,
            lago,
            puerta: None,
        }
    }

    /// Y escribe, por esta puerta (C5).
    pub fn con_puerta(mut self, puerta: Arc<dyn Puerta>) -> CatalogoOre {
        self.puerta = Some(puerta);
        self
    }
}

#[async_trait]
impl Catalogo for CatalogoOre {
    async fn preparar(
        &self,
        base: &SessionState,
        _referencias: &[TableReference],
    ) -> Result<Arc<dyn CatalogProviderList>, Fallo> {
        // Toda la foto y no sólo lo que la consulta nombra: las vistas nombran otras cosas que sólo se saben al
        // planificarlas, y tienen que verlas en la misma foto.
        let fuente = self.fuente.clone();
        let foto = tokio::task::spawn_blocking(move || fuente.foto())
            .await
            .map_err(|e| Fallo::Catalogo(format!("leer la foto: {e}")))??;
        Ok(ListaOre::nueva(
            foto,
            self.lago.clone(),
            self.puerta.clone(),
            base.clone(),
        ))
    }
}

/// Lo que comparten los schemas de una consulta.
#[derive(Debug)]
struct Comun {
    lago: Lago,
    puerta: Option<Arc<dyn Puerta>>,
    /// El estado de la sesión antes de resolver: con él se planifican las vistas.
    estado: SessionState,
    lista: Weak<ListaOre>,
    /// Las vistas que se están planificando, por si una se nombra a sí misma.
    planificando: Mutex<Vec<Nombre>>,
}

/// **Los catálogos de una consulta**: uno por base de la foto.
#[derive(Debug)]
pub struct ListaOre {
    bases: BTreeMap<String, Arc<BaseOre>>,
    /// La foto, por nombre: lo que una escritura mira antes de crear o sustituir.
    objetos: BTreeMap<Nombre, Objeto>,
    comun: Arc<Comun>,
}

type Arbol = BTreeMap<String, BTreeMap<String, BTreeMap<String, Objeto>>>;

impl ListaOre {
    fn nueva(
        foto: Foto,
        lago: Lago,
        puerta: Option<Arc<dyn Puerta>>,
        estado: SessionState,
    ) -> Arc<ListaOre> {
        Arc::new_cyclic(|lista| {
            let objetos = foto.objetos.clone();
            let comun = Arc::new(Comun {
                lago,
                puerta,
                estado,
                lista: lista.clone(),
                planificando: Mutex::new(Vec::new()),
            });
            let mut arbol = Arbol::new();
            for (n, o) in foto.objetos {
                arbol
                    .entry(n.base)
                    .or_default()
                    .entry(n.schema)
                    .or_default()
                    .insert(n.nombre, o);
            }
            let bases = arbol
                .into_iter()
                .map(|(base, schemas)| {
                    let schemas = schemas
                        .into_iter()
                        .map(|(schema, objetos)| {
                            let s = Arc::new(SchemaOre {
                                base: base.clone(),
                                schema: schema.clone(),
                                objetos,
                                comun: comun.clone(),
                            });
                            (schema, s)
                        })
                        .collect();
                    (base, Arc::new(BaseOre { schemas }))
                })
                .collect();
            ListaOre {
                bases,
                objetos,
                comun,
            }
        })
    }

    /// Lo que la foto de esta consulta tiene en `nombre`.
    pub fn objeto(&self, nombre: &Nombre) -> Option<Objeto> {
        self.objetos.get(nombre).cloned()
    }

    /// Lo que hace falta para escribir en `nombre`, si este catálogo escribe.
    pub fn escribe(&self, nombre: &Nombre) -> Result<Escribe, Fallo> {
        let puerta = self.comun.puerta.clone().ok_or_else(|| {
            Fallo::Config(format!(
                "`{nombre}` no se escribe desde esta sesión: el catálogo no tiene puerta"
            ))
        })?;
        Ok(Escribe {
            lago: self.comun.lago.clone(),
            puerta,
            nombre: nombre.clone(),
        })
    }
}

impl CatalogProviderList for ListaOre {
    fn register_catalog(
        &self,
        _name: String,
        _catalog: Arc<dyn CatalogProvider>,
    ) -> Option<Arc<dyn CatalogProvider>> {
        // Los catálogos de una consulta son los de su foto: nada se registra en ella.
        None
    }

    fn catalog_names(&self) -> Vec<String> {
        self.bases.keys().cloned().collect()
    }

    fn catalog(&self, name: &str) -> Option<Arc<dyn CatalogProvider>> {
        self.bases
            .get(name)
            .map(|b| b.clone() as Arc<dyn CatalogProvider>)
    }
}

/// **Una vista que no se pudo planificar**: está, sin columnas, y usarla da su error.
#[derive(Debug)]
struct VistaRota {
    error: String,
    esquema: datafusion::arrow::datatypes::SchemaRef,
}

#[async_trait]
impl TableProvider for VistaRota {
    fn schema(&self) -> datafusion::arrow::datatypes::SchemaRef {
        self.esquema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::View
    }

    async fn scan(
        &self,
        _estado: &dyn datafusion::catalog::Session,
        _proyeccion: Option<&Vec<usize>>,
        _filtros: &[datafusion::logical_expr::Expr],
        _limite: Option<usize>,
    ) -> Result<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
        plan_err!("{}", self.error)
    }
}

/// Una base: sus schemas.
#[derive(Debug)]
struct BaseOre {
    schemas: BTreeMap<String, Arc<SchemaOre>>,
}

impl CatalogProvider for BaseOre {
    fn schema_names(&self) -> Vec<String> {
        self.schemas.keys().cloned().collect()
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        self.schemas
            .get(name)
            .map(|s| s.clone() as Arc<dyn SchemaProvider>)
    }
}

/// Un schema: sus datasets y sus vistas.
#[derive(Debug)]
struct SchemaOre {
    base: String,
    schema: String,
    objetos: BTreeMap<String, Objeto>,
    comun: Arc<Comun>,
}

impl SchemaOre {
    async fn vista(&self, nombre: Nombre, sql: &str) -> Result<Arc<dyn TableProvider>> {
        {
            let mut p = self.comun.planificando.lock().unwrap();
            if p.contains(&nombre) {
                return plan_err!("la vista `{nombre}` se nombra a sí misma");
            }
            p.push(nombre.clone());
        }
        let plan = Box::pin(self.planificar_vista(sql)).await;
        self.comun
            .planificando
            .lock()
            .unwrap()
            .retain(|n| n != &nombre);
        let plan = plan.map_err(|e| e.context(format!("la vista `{nombre}`")))?;
        Ok(Arc::new(ViewTable::new(plan, Some(sql.to_string()))))
    }

    async fn planificar_vista(&self, sql: &str) -> Result<LogicalPlan> {
        let lista = self.comun.lista.upgrade().ok_or_else(|| {
            DataFusionError::Internal("los catálogos de la consulta ya no están".into())
        })?;
        let mut estado = SessionStateBuilder::new_from_existing(self.comun.estado.clone())
            .with_catalog_list(lista)
            .build();
        let catalogo = &mut estado.config_mut().options_mut().catalog;
        catalogo.default_catalog = self.base.clone();
        catalogo.default_schema = self.schema.clone();
        let plan = estado.create_logical_plan(sql).await?;
        // Una vista es una consulta: ni escribe, ni crea, ni cambia la sesión.
        SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false)
            .with_allow_statements(false)
            .verify_plan(&plan)?;
        Ok(plan)
    }
}

#[async_trait]
impl SchemaProvider for SchemaOre {
    fn table_names(&self) -> Vec<String> {
        self.objetos.keys().cloned().collect()
    }

    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        let nombre = Nombre::new(&self.base, &self.schema, name);
        match self.objetos.get(name) {
            None => Ok(None),
            Some(Objeto::Dataset { metadata_location }) => {
                let tabla = self
                    .comun
                    .lago
                    .abrir_async(metadata_location, &nombre.to_string())
                    .await
                    .map_err(|e| DataFusionError::Execution(format!("`{nombre}`: {e}")))?;
                let escribe = self.comun.puerta.clone().map(|puerta| Escribe {
                    lago: self.comun.lago.clone(),
                    puerta,
                    nombre: nombre.clone(),
                });
                Ok(Some(Arc::new(TablaOre::new(tabla)?.con_escribe(escribe))))
            }
            // Una vista que no se puede planificar sigue estando: se lista (sin columnas) y da su error al usarla.
            // Si no, una sola vista rota tumbaría `information_schema.columns` de todo el catálogo.
            Some(Objeto::Vista { sql }) => match self.vista(nombre, sql).await {
                Ok(v) => Ok(Some(v)),
                Err(e) => Ok(Some(Arc::new(VistaRota {
                    error: e.to_string(),
                    esquema: Arc::new(datafusion::arrow::datatypes::Schema::empty()),
                }))),
            },
        }
    }

    /// Sin abrir nada: listar (`information_schema`, `SHOW TABLES`) no abre cada dataset ni planifica cada vista.
    async fn table_type(&self, name: &str) -> Result<Option<TableType>> {
        Ok(self.objetos.get(name).map(|o| match o {
            Objeto::Dataset { .. } => TableType::Base,
            Objeto::Vista { .. } => TableType::View,
        }))
    }

    fn table_exist(&self, name: &str) -> bool {
        self.objetos.contains_key(name)
    }
}
