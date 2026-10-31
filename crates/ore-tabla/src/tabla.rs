//! **Una tabla Iceberg del lago como `TableProvider`** (0081 §7.5, C4). Se abre en el `metadata_location` que da
//! la foto, así que lee el snapshot de la foto y no el de cuando se ejecute.
//!
//! - **El esquema** es el de Iceberg en Arrow (`schema_to_arrow_schema`): los tipos del lago, el `field_id` de
//!   cada campo y `required` como no nulable (§7.5.2, los tipos y los nulos).
//! - **El escaneo** es el de iceberg-rust: `TableScan` planifica los ficheros (con sus borrados) y su
//!   `ArrowReader` los lee. Se reparten en particiones por `FileScanTask`, hasta `target_partitions`.
//! - **Sin poda todavía**: ni filtros ni estadísticas. Leer bien es C4; leer al máximo es del frente 5.
//! - **Escribir** (C5–C7): `INSERT INTO`, `INSERT OVERWRITE`, `TRUNCATE`, `DELETE`, `UPDATE` y `MERGE INTO` van al sink de ORE
//!   ([`crate::escritura`]), si el catálogo de la consulta tiene puerta.

use crate::escritura::Escribe;
use async_trait::async_trait;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::arrow::record_batch::{RecordBatch, RecordBatchOptions};
use datafusion::catalog::{Session, TableProvider};
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::common::{DataFusionError, Result};
use datafusion::datasource::TableType;
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::Expr;
use datafusion::logical_expr::dml::InsertOp;
use datafusion::physical_expr::{EquivalenceProperties, PhysicalExpr};
use datafusion::physical_plan::execution_plan::{
    Boundedness, ChildrenPropertiesMode, EmissionType, ReplaceChildrenOptions,
};
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
};
use futures::{StreamExt, TryStreamExt};
use iceberg::scan::FileScanTask;
use iceberg::spec::Schema;
use iceberg::table::Table;
use ore_runtime::viaje::Punto;
use std::sync::Arc;

fn iceberg(e: iceberg::Error) -> DataFusionError {
    DataFusionError::External(Box::new(e))
}

/// **Una tabla del lago**, en el snapshot de la foto.
#[derive(Debug, Clone)]
pub struct TablaOre {
    tabla: Table,
    esquema: SchemaRef,
    /// El esquema de Iceberg con que se escribe: el vigente, o el que pide `create or replace … as`.
    escritura: Schema,
    /// Por dónde se escribe, si se escribe (C5).
    escribe: Option<Escribe>,
    /// El snapshot que se lee, si no es el vigente de la foto (`at(…)`, C9).
    snapshot: Option<i64>,
}

impl TablaOre {
    pub fn new(tabla: Table) -> Result<TablaOre> {
        let escritura = tabla.metadata().current_schema().as_ref().clone();
        TablaOre::con_esquema(tabla, escritura)
    }

    fn con_esquema(tabla: Table, escritura: Schema) -> Result<TablaOre> {
        let esquema = iceberg::arrow::schema_to_arrow_schema(&escritura).map_err(iceberg)?;
        Ok(TablaOre {
            tabla,
            esquema: Arc::new(esquema),
            escritura,
            escribe: None,
            snapshot: None,
        })
    }

    /// **La tabla como estaba en `punto`** (C9): un snapshot que sus metadatos guardan (lo que la retención no expiró),
    /// con el esquema de entonces. Sólo se lee.
    pub fn en(&self, punto: &Punto) -> std::result::Result<TablaOre, String> {
        let meta = self.tabla.metadata();
        let snapshot = match *punto {
            Punto::Snapshot(id) => meta.snapshot_by_id(id).cloned().ok_or_else(|| {
                format!("el snapshot {id} no está: no existió, o la retención lo expiró")
            })?,
            Punto::Momento(ms) => meta
                .snapshots()
                .filter(|s| s.timestamp_ms() <= ms)
                .max_by_key(|s| (s.timestamp_ms(), s.sequence_number()))
                .cloned()
                .ok_or_else(|| {
                    "no hay ningún snapshot de entonces: la tabla es posterior, o la retención lo expiró".to_string()
                })?,
        };
        let esquema = snapshot
            .schema_id()
            .and_then(|i| meta.schema_by_id(i))
            .unwrap_or_else(|| meta.current_schema())
            .as_ref()
            .clone();
        let mut t =
            TablaOre::con_esquema(self.tabla.clone(), esquema).map_err(|e| e.to_string())?;
        t.snapshot = Some(snapshot.snapshot_id());
        Ok(t)
    }

    /// **El destino de `create dataset … as`**: la tabla (la que hay, o su esbozo) con el esquema que se escribe.
    pub fn para_escribir(tabla: Table, escritura: Schema, escribe: Escribe) -> Result<TablaOre> {
        Ok(TablaOre::con_esquema(tabla, escritura)?.con_escribe(Some(escribe)))
    }

    pub fn con_escribe(mut self, escribe: Option<Escribe>) -> TablaOre {
        self.escribe = escribe;
        self
    }

    /// La tabla de Iceberg.
    pub fn tabla(&self) -> &Table {
        &self.tabla
    }

    pub fn escribe(&self) -> Option<&Escribe> {
        self.escribe.as_ref()
    }

    pub fn esquema_de_escritura(&self) -> &Schema {
        &self.escritura
    }
}

#[async_trait]
impl TableProvider for TablaOre {
    fn schema(&self) -> SchemaRef {
        self.esquema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        estado: &dyn Session,
        proyeccion: Option<&Vec<usize>>,
        _filtros: &[Expr],
        _limite: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let esquema = match proyeccion {
            Some(p) => Arc::new(self.esquema.project(p)?),
            None => self.esquema.clone(),
        };
        let columnas: Vec<String> = esquema.fields().iter().map(|f| f.name().clone()).collect();
        let constructor = match self.snapshot {
            Some(id) => self.tabla.scan().snapshot_id(id),
            None => self.tabla.scan(),
        };
        let constructor = if columnas.is_empty() {
            constructor.select_empty()
        } else {
            constructor.select(columnas)
        };
        let tareas: Vec<FileScanTask> = constructor
            .build()
            .map_err(iceberg)?
            .plan_files()
            .await
            .map_err(iceberg)?
            .try_collect()
            .await
            .map_err(iceberg)?;
        let n = estado.config().target_partitions().min(tareas.len()).max(1);
        let mut particiones: Vec<Vec<FileScanTask>> = vec![Vec::new(); n];
        for (i, t) in tareas.into_iter().enumerate() {
            particiones[i % n].push(t);
        }
        Ok(Arc::new(EscaneoOre::new(
            self.tabla.clone(),
            esquema,
            particiones,
        )))
    }

    async fn insert_into(
        &self,
        estado: &dyn Session,
        entrada: Arc<dyn ExecutionPlan>,
        op: InsertOp,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        crate::escritura::insertar(self, estado, entrada, op).await
    }

    async fn truncate(&self, _estado: &dyn Session) -> Result<Arc<dyn ExecutionPlan>> {
        crate::escritura::vaciar(self)
    }

    async fn delete_from(
        &self,
        estado: &dyn Session,
        filtros: Vec<Expr>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        crate::escritura::cambiar(self, estado, None, filtros).await
    }

    async fn update(
        &self,
        estado: &dyn Session,
        asignaciones: Vec<(String, Expr)>,
        filtros: Vec<Expr>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        crate::escritura::cambiar(self, estado, Some(asignaciones), filtros).await
    }

    async fn merge_into(
        &self,
        estado: &dyn Session,
        fuente: Arc<dyn ExecutionPlan>,
        esquema: datafusion::common::DFSchemaRef,
        on: Expr,
        clausulas: Vec<datafusion::logical_expr::dml::MergeIntoClause>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        crate::escritura::fusionar(self, estado, fuente, esquema, on, clausulas).await
    }
}

/// **El escaneo de una tabla del lago**: cada partición lee sus ficheros con el `ArrowReader` de iceberg-rust,
/// que aplica sus borrados.
#[derive(Debug)]
pub struct EscaneoOre {
    tabla: Table,
    esquema: SchemaRef,
    particiones: Vec<Vec<FileScanTask>>,
    propiedades: Arc<PlanProperties>,
}

impl EscaneoOre {
    fn new(tabla: Table, esquema: SchemaRef, particiones: Vec<Vec<FileScanTask>>) -> Self {
        let propiedades = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(esquema.clone()),
            Partitioning::UnknownPartitioning(particiones.len()),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        EscaneoOre {
            tabla,
            esquema,
            particiones,
            propiedades,
        }
    }
}

impl DisplayAs for EscaneoOre {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let ficheros: usize = self.particiones.iter().map(Vec::len).sum();
        write!(
            f,
            "EscaneoOre: tabla={}, ficheros={ficheros}, particiones={}",
            self.tabla.identifier(),
            self.particiones.len()
        )
    }
}

impl ExecutionPlan for EscaneoOre {
    fn name(&self) -> &'static str {
        "EscaneoOre"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.propiedades
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }

    fn apply_expressions(
        &self,
        _f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> Result<TreeNodeRecursion>,
    ) -> Result<TreeNodeRecursion> {
        Ok(TreeNodeRecursion::Continue)
    }

    fn replace_children(
        self: Arc<Self>,
        _hijos: Vec<Arc<dyn ExecutionPlan>>,
        _opciones: ReplaceChildrenOptions,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        Ok(self)
    }

    fn with_new_children(
        self: Arc<Self>,
        hijos: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.replace_children(
            hijos,
            ReplaceChildrenOptions::new(ChildrenPropertiesMode::Recompute),
        )
    }

    fn execute(
        &self,
        particion: usize,
        contexto: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let tareas = self.particiones.get(particion).cloned().ok_or_else(|| {
            DataFusionError::Internal(format!(
                "EscaneoOre: la partición {particion} no existe ({})",
                self.particiones.len()
            ))
        })?;
        let lotes = self
            .tabla
            .reader_builder()
            .with_batch_size(contexto.session_config().batch_size())
            .build()
            .read(Box::pin(futures::stream::iter(tareas.into_iter().map(Ok))))
            .map_err(iceberg)?
            .stream();
        // Lo leído, con el esquema de la tabla (sus nombres, sus nulables y sus `field_id`): el lector da los
        // mismos tipos, y el plan cuenta con este esquema exacto.
        let esquema = self.esquema.clone();
        let lotes = lotes.map(move |l| {
            let l = l.map_err(iceberg)?;
            let opciones = RecordBatchOptions::new().with_row_count(Some(l.num_rows()));
            Ok(RecordBatch::try_new_with_options(
                esquema.clone(),
                l.columns().to_vec(),
                &opciones,
            )?)
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.esquema.clone(),
            lotes,
        )))
    }
}
