//! **Una tabla Iceberg del lago como `TableProvider`** (0081 §7.5, C4). Se abre en el `metadata_location` que da
//! la foto, así que lee el snapshot de la foto y no el de cuando se ejecute.
//!
//! - **El esquema** es el de Iceberg en Arrow (`schema_to_arrow_schema`): los tipos del lago, el `field_id` de
//!   cada campo y `required` como no nulable (§7.5.2, los tipos y los nulos).
//! - **El escaneo** es el de iceberg-rust: `TableScan` planifica los ficheros (con sus borrados) y su
//!   `ArrowReader` los lee. Se reparten en particiones por `FileScanTask`, hasta `target_partitions`.
//! - **Sin poda todavía**: ni filtros ni estadísticas. Leer bien es C4; leer al máximo es del frente 5.

use async_trait::async_trait;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::arrow::record_batch::{RecordBatch, RecordBatchOptions};
use datafusion::catalog::{Session, TableProvider};
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::common::{DataFusionError, Result};
use datafusion::datasource::TableType;
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::Expr;
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
use iceberg::table::Table;
use std::sync::Arc;

fn iceberg(e: iceberg::Error) -> DataFusionError {
    DataFusionError::External(Box::new(e))
}

/// **Una tabla del lago**, en el snapshot de la foto.
#[derive(Debug, Clone)]
pub struct TablaOre {
    tabla: Table,
    esquema: SchemaRef,
}

impl TablaOre {
    pub fn new(tabla: Table) -> Result<TablaOre> {
        let esquema = iceberg::arrow::schema_to_arrow_schema(tabla.metadata().current_schema())
            .map_err(iceberg)?;
        Ok(TablaOre {
            tabla,
            esquema: Arc::new(esquema),
        })
    }

    /// La tabla de Iceberg.
    pub fn tabla(&self) -> &Table {
        &self.tabla
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
        let constructor = self.tabla.scan();
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
