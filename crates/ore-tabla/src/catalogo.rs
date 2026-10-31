//! **El zócalo de catálogo de ORE** (0081 §7.5.2 y §7.5.3, decisión 2). Antes de planificar, lee **la foto** una
//! vez y da los catálogos de esa consulta: cada base de la foto es un catálogo de DataFusion, cada schema un
//! schema y cada nombre una tabla. Una sentencia ve una sola foto para todas sus tablas; la siguiente lee otra y
//! ve lo que escribió la anterior.
//!
//! - **Un dataset** se abre en su `metadata_location`, el snapshot de la foto ([`TablaOre`]).
//! - **Una vista** se planifica al nombrarla, sobre la misma foto, con su base y su schema por defecto: es un
//!   `ViewTable` de DataFusion. Una vista que se nombra a sí misma, directa o indirectamente, es un error.

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

/// **El catálogo de ORE**: de dónde sale la foto y el lago en el que están sus datasets.
#[derive(Debug, Clone)]
pub struct CatalogoOre {
    fuente: Arc<dyn Fuente>,
    lago: Lago,
}

impl CatalogoOre {
    pub fn new(fuente: Arc<dyn Fuente>, lago: Lago) -> CatalogoOre {
        CatalogoOre { fuente, lago }
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
        Ok(ListaOre::nueva(foto, self.lago.clone(), base.clone()))
    }
}

/// Lo que comparten los schemas de una consulta.
#[derive(Debug)]
struct Comun {
    lago: Lago,
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
}

type Arbol = BTreeMap<String, BTreeMap<String, BTreeMap<String, Objeto>>>;

impl ListaOre {
    fn nueva(foto: Foto, lago: Lago, estado: SessionState) -> Arc<ListaOre> {
        Arc::new_cyclic(|lista| {
            let comun = Arc::new(Comun {
                lago,
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
            ListaOre { bases }
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
                Ok(Some(Arc::new(TablaOre::new(tabla)?)))
            }
            Some(Objeto::Vista { sql }) => self.vista(nombre, sql).await.map(Some),
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
