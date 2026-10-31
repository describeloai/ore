//! **Los nodos de ORE en el plan distribuido** (0081 §7.5, C8). El cliente fija la foto y planifica; el scheduler
//! planifica en físico (lista los ficheros y fija la clave de cada commit) y reparte; los ejecutores leen y escriben.
//! Cada nodo viaja con **todo lo que el contrato fijó al planificar**, y decodificarlo no lee nada ni vuelve a
//! resolver nada: un papel que resolviera por su cuenta podría ver otra foto, u otra clave.
//!
//! | nodo | codec | lleva |
//! |---|---|---|
//! | [`TablaOre`] (fuente de `TableScan`, destino de `Dml`) | lógico | los metadatos de la foto y su `metadata_location` (o el esbozo, sin él), el esquema de escritura, el snapshot de `at(…)` y, si escribe, su nombre |
//! | [`EscaneoOre`] | físico | las `FileScanTask` de cada partición y el esquema con que se leen; los metadatos, no |
//! | `DataSinkExec` con [`OreSink`] | físico | la tabla de la foto, el esquema de Iceberg, la operación, las marcas y **la clave de idempotencia** |
//! | una vista rota | lógico | su error, que da al planificarla en físico, como en local |
//!
//! Lo vivo no viaja: el `Lago` (de dónde se leen los bytes) y la `Puerta` (por dónde se confirma) los pone el codec
//! de cada proceso ([`CodecOre`]). El suelo lo envuelve con el de Ballista ([`ore_runtime::codecs`]): lo que no es de
//! ORE, este codec lo rechaza.

use crate::catalogo::VistaRota;
use crate::escritura::{Escribe, OreSink, Puerta};
use crate::foto::Nombre;
use crate::lagos::Lagos;
use crate::tabla::{EscaneoOre, TablaOre};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::TableProvider;
use datafusion::common::{DataFusionError, Result, TableReference, not_impl_err};
use datafusion::datasource::sink::DataSinkExec;
use datafusion::execution::TaskContext;
use datafusion::logical_expr::{Extension, LogicalPlan};
use datafusion::physical_plan::ExecutionPlan;
use datafusion_proto::logical_plan::LogicalExtensionCodec;
use datafusion_proto::physical_plan::{PhysicalExtensionCodec, PhysicalProtoConverterExtension};
use iceberg::scan::FileScanTask;
use iceberg::spec::{Schema, TableMetadata};
use iceberg::table::Table;
use ore_runtime::codecs::Codecs;
use ore_store::lago::Operacion;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// **El codec de ORE** en un proceso: con su lago y, si confirma, su puerta.
#[derive(Debug, Clone)]
pub struct CodecOre {
    lagos: Arc<dyn Lagos>,
    puerta: Option<Arc<dyn Puerta>>,
}

impl CodecOre {
    pub fn new(lagos: Arc<dyn Lagos>, puerta: Option<Arc<dyn Puerta>>) -> CodecOre {
        CodecOre { lagos, puerta }
    }

    /// El lago de una tabla del plan, por su nombre (`base.schema.nombre`) y dónde está: sin leer nada.
    fn lago(&self, dataset: &str, ubicacion: &str) -> Result<ore_store::lago::Lago> {
        // Como lo nombra su `TableIdent`: `base.schema.nombre`, o `catalogo/base/schema/nombre` si nació aquí.
        let nombre = Nombre::de_corto(dataset)
            .or_else(|| {
                let p: Vec<&str> = dataset.strip_prefix("catalogo/")?.split('/').collect();
                (p.len() == 3).then(|| Nombre::new(p[0], p[1], p[2]))
            })
            .unwrap_or_else(|| Nombre::new("", "", dataset));
        self.lagos
            .de(&nombre, ubicacion)
            .map_err(DataFusionError::Execution)
    }

    /// Los dos, para el zócalo de extensiones.
    pub fn codecs(self) -> Codecs {
        let c = Arc::new(self);
        Codecs {
            logico: c.clone(),
            fisico: c,
        }
    }

    fn puerta(&self, nombre: &Nombre) -> Result<Arc<dyn Puerta>> {
        self.puerta.clone().ok_or_else(|| {
            DataFusionError::Plan(format!(
                "`{nombre}` no se escribe desde este proceso: su codec no tiene puerta"
            ))
        })
    }

    fn escribe(&self, nombre: Nombre, tabla: &Table) -> Result<Escribe> {
        Ok(Escribe {
            lago: self.lago(&nombre.to_string(), tabla.metadata().location())?,
            puerta: self.puerta(&nombre)?,
            nombre,
        })
    }
}

fn json<T: Serialize>(v: &T, buf: &mut Vec<u8>) -> Result<()> {
    serde_json::to_writer(buf, v).map_err(|e| DataFusionError::External(Box::new(e)))
}

fn de_json<'a, T: Deserialize<'a>>(buf: &'a [u8]) -> Result<T> {
    serde_json::from_slice(buf)
        .map_err(|e| DataFusionError::Internal(format!("un nodo de ORE que no se entiende: {e}")))
}

/// **Una tabla de la foto**: sus metadatos tal cual se abrieron, sin volver a leerlos.
#[derive(Serialize, Deserialize)]
struct LaTabla {
    /// `None`: un esbozo de `create dataset … as`, que todavía no tiene fichero.
    metadata_location: Option<String>,
    metadata: TableMetadata,
    dataset: String,
}

impl LaTabla {
    fn de(t: &Table) -> LaTabla {
        LaTabla {
            metadata_location: t.metadata_location().map(str::to_string),
            metadata: t.metadata().clone(),
            dataset: t.identifier().name().to_string(),
        }
    }

    fn abrir(self, codec: &CodecOre) -> Result<Table> {
        codec.lago(&self.dataset, self.metadata.location())?.de_metadatos(
            self.metadata,
            self.metadata_location.as_deref(),
            &self.dataset,
        )
        .map_err(DataFusionError::Execution)
    }
}

/// Lo que viaja en el plan lógico como tabla.
#[derive(Serialize, Deserialize)]
enum Tabla {
    Ore(Box<LaTablaOre>),
    VistaRota { error: String },
}

/// `TablaOre` en el plan lógico.
#[derive(Serialize, Deserialize)]
struct LaTablaOre {
    tabla: LaTabla,
    escritura: Schema,
    snapshot: Option<i64>,
    /// El nombre por el que escribe, si escribe.
    escribe: Option<Nombre>,
}

#[derive(Serialize, Deserialize)]
enum Nodo {
    Escaneo {
        nombre: String,
        ubicacion: String,
        escritura: Schema,
        proyeccion: Option<Vec<usize>>,
        particiones: Vec<Vec<FileScanTask>>,
    },
    Sink {
        nombre: Nombre,
        tabla: Box<LaTabla>,
        esquema: Schema,
        sobrescribe: bool,
        marcas: bool,
        clave: String,
    },
}

impl LogicalExtensionCodec for CodecOre {
    fn try_decode(&self, _: &[u8], _: &[LogicalPlan], _: &TaskContext) -> Result<Extension> {
        not_impl_err!("ORE no pone nodos lógicos propios")
    }

    fn try_encode(&self, _: &Extension, _: &mut Vec<u8>) -> Result<()> {
        not_impl_err!("ORE no pone nodos lógicos propios")
    }

    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        _: &TableReference,
        esquema: SchemaRef,
        _: &TaskContext,
    ) -> Result<Arc<dyn TableProvider>> {
        let t = match de_json(buf)? {
            Tabla::Ore(t) => t,
            Tabla::VistaRota { error } => return Ok(Arc::new(VistaRota { error, esquema })),
        };
        let abierta = t.tabla.abrir(self)?;
        let escribe = t.escribe.map(|n| self.escribe(n, &abierta)).transpose()?;
        let mut tabla = TablaOre::con_esquema(abierta, t.escritura)?;
        tabla.snapshot = t.snapshot;
        Ok(Arc::new(tabla.con_escribe(escribe)))
    }

    fn try_encode_table_provider(
        &self,
        _: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> Result<()> {
        if let Some(v) = node.as_ref().downcast_ref::<VistaRota>() {
            return json(
                &Tabla::VistaRota {
                    error: v.error.clone(),
                },
                buf,
            );
        }
        let Some(t) = node.as_ref().downcast_ref::<TablaOre>() else {
            return not_impl_err!("no es una tabla del lago");
        };
        json(
            &Tabla::Ore(Box::new(LaTablaOre {
                tabla: LaTabla::de(t.tabla()),
                escritura: t.esquema_de_escritura().clone(),
                snapshot: t.snapshot,
                escribe: t.escribe().map(|e| e.nombre.clone()),
            })),
            buf,
        )
    }
}

impl PhysicalExtensionCodec for CodecOre {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[Arc<dyn ExecutionPlan>],
        _: &TaskContext,
        _: &dyn PhysicalProtoConverterExtension,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        match de_json(buf)? {
            Nodo::Escaneo {
                nombre,
                ubicacion,
                escritura,
                proyeccion,
                particiones,
            } => Ok(Arc::new(EscaneoOre::new(
                self.lago(&nombre, &ubicacion)?.file_io(),
                nombre,
                ubicacion,
                escritura,
                proyeccion,
                particiones,
            )?)),
            Nodo::Sink {
                nombre,
                tabla,
                esquema,
                sobrescribe,
                marcas,
                clave,
            } => {
                let [entrada] = inputs else {
                    return Err(DataFusionError::Internal(format!(
                        "el sink de `{nombre}` con {} entradas",
                        inputs.len()
                    )));
                };
                let operacion = if sobrescribe {
                    Operacion::Sobrescribir
                } else {
                    Operacion::Anexar
                };
                let tabla = tabla.abrir(self)?;
                let sink = OreSink::new(
                    self.escribe(nombre, &tabla)?,
                    tabla,
                    esquema,
                    operacion,
                    clave,
                )?;
                let sink = if marcas { sink.con_marcas() } else { sink };
                Ok(Arc::new(DataSinkExec::new(
                    entrada.clone(),
                    Arc::new(sink),
                    None,
                )))
            }
        }
    }

    fn try_encode(
        &self,
        node: Arc<dyn ExecutionPlan>,
        buf: &mut Vec<u8>,
        _: &dyn PhysicalProtoConverterExtension,
    ) -> Result<()> {
        if let Some(e) = node.as_ref().downcast_ref::<EscaneoOre>() {
            return json(
                &Nodo::Escaneo {
                    nombre: e.nombre.clone(),
                    ubicacion: e.ubicacion.clone(),
                    escritura: e.escritura.clone(),
                    proyeccion: e.proyeccion.clone(),
                    particiones: e.particiones.clone(),
                },
                buf,
            );
        }
        if let Some(d) = node.as_ref().downcast_ref::<DataSinkExec>()
            && let Some(s) = (d.sink() as &dyn std::any::Any).downcast_ref::<OreSink>()
        {
            return json(
                &Nodo::Sink {
                    nombre: s.escribe.nombre.clone(),
                    tabla: Box::new(LaTabla::de(&s.tabla)),
                    esquema: s.esquema.clone(),
                    sobrescribe: matches!(s.operacion, Operacion::Sobrescribir),
                    marcas: s.marcas,
                    clave: s.clave.clone(),
                },
                buf,
            );
        }
        not_impl_err!("no es un nodo de ORE")
    }
}
