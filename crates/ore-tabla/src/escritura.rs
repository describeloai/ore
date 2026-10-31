//! **Escribir en el lago** (0081 §7.5.2 y §7.5.3, decisión 5; C5). DataFusion planifica los verbos y el
//! [`TablaOre`](crate::tabla::TablaOre) del lago los cumple; cada sentencia que escribe es **un commit de ORE**:
//!
//! 1. **La primera mitad** la hace `ore-store` en el sink ([`OreSink`]): los ficheros de datos, los manifiestos y los
//!    `requirements + updates` (`Lago::preparar_async`), sobre la tabla de la foto (o su esbozo, si nace).
//! 2. **El commit** lo hace la [`Puerta`]: el CAS contra lo que la foto vio y la **clave de idempotencia**, que se fija
//!    al planificar y es la misma en cada reintento (de Ballista o de la red). En producción, `/v1` (`updateTable` con
//!    `Idempotency-Key`); en las pruebas, la del lago en memoria.
//!
//! DataFusion define el sink como «se llama una vez por sentencia y hace el commit» (`DataSink::write_all`), y
//! `DataSinkExec` comprueba `NOT NULL` contra el esquema del sink, que es el de la tabla (`required`).
//!
//! **Los verbos:** `INSERT INTO` anexa; `INSERT OVERWRITE` y `create [or replace] dataset … as` sobrescriben;
//! `TRUNCATE` sobrescribe con nada; `create dataset … (columnas)` nace vacío y `drop dataset` retira el puntero. Los
//! demás cambios del catálogo (bases, schemas, vistas, colecciones) son del árbol y de sus productos: no los cumple
//! el lago ([`la_cumple_el_lago`]).

use crate::catalogo::ListaOre;
use crate::foto::{Nombre, Objeto};
use crate::tabla::TablaOre;
use async_trait::async_trait;
use datafusion::arrow::array::BooleanArray;
use datafusion::arrow::compute::filter_record_batch;
use datafusion::arrow::compute::kernels::boolean::not;
use datafusion::arrow::datatypes::{DataType, SchemaRef, TimeUnit};
use datafusion::arrow::record_batch::{RecordBatch, RecordBatchOptions};
use datafusion::catalog::{Session, TableProvider};
use datafusion::common::{DFSchema, DataFusionError, Result, TableReference};
use datafusion::datasource::provider_as_source;
use datafusion::datasource::sink::{DataSink, DataSinkExec};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::dml::InsertOp;
use datafusion::logical_expr::when;
use datafusion::logical_expr::{
    DmlStatement, EmptyRelation, Expr, LogicalPlan, LogicalPlanBuilder, WriteOp, cast, col,
};
use datafusion::physical_expr::projection::ProjectionExpr;
use datafusion::physical_plan::projection::ProjectionExec;
use datafusion::physical_plan::{DisplayAs, DisplayFormatType, ExecutionPlan};
use futures::StreamExt;
use iceberg::spec::Schema;
use iceberg::table::Table;
use iceberg::{TableRequirement, TableUpdate};
use ore_runtime::ddl::{CambioDeCatalogo, Objetivo, Retirable};
use ore_runtime::{Fallo, Plan, Sesion};
use ore_store::lago::{Lago, Operacion, esquema_deseado};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Lo que puede contestar la puerta al confirmar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalloCommit {
    /// Alguien escribió después de la foto: lo que hay ahora (o nada).
    Conflicto { hay: Option<String> },
    /// Lo demás, dicho.
    Otro(String),
}

impl std::fmt::Display for FalloCommit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FalloCommit::Conflicto { hay } => write!(
                f,
                "alguien escribió después de que esta sentencia leyera el catálogo (ahora: {}); \
                 nada se publicó: vuelve a correrla",
                hay.as_deref().unwrap_or("nada")
            ),
            FalloCommit::Otro(m) => f.write_str(m),
        }
    }
}

/// **La puerta del commit** (0081 §7.5.3, decisión 5): una sola, la del catálogo de ORE.
#[async_trait]
pub trait Puerta: Send + Sync + Debug {
    /// **Una tabla que nace**: su esbozo, sin `metadata_location`, sobre el que se escriben los ficheros antes del
    /// commit (`stage-create`).
    async fn esbozar(&self, nombre: &Nombre, esquema: Schema) -> Result<Table, String>;

    /// **El commit**: aplica `cambios` sobre `tabla` si lo vigente es lo que `tabla` es (su `metadata_location`, o
    /// nada si nace) y mueve el puntero. La misma `clave` dos veces es un solo commit: la segunda contesta lo de la
    /// primera. Devuelve el `metadata_location` nuevo.
    async fn confirmar(
        &self,
        nombre: &Nombre,
        tabla: &Table,
        requisitos: Vec<TableRequirement>,
        cambios: Vec<TableUpdate>,
        clave: &str,
    ) -> Result<String, FalloCommit>;

    /// **Retira el puntero** de `nombre` si lo vigente es `esperado` (queda en la historia).
    async fn retirar(
        &self,
        nombre: &Nombre,
        esperado: &str,
        clave: &str,
    ) -> Result<(), FalloCommit>;
}

/// Lo que una tabla necesita para escribir: dónde están los bytes y por dónde se commitea.
#[derive(Debug, Clone)]
pub struct Escribe {
    pub lago: Lago,
    pub puerta: Arc<dyn Puerta>,
    pub nombre: Nombre,
}

/// Una clave de idempotencia nueva: un UUIDv7, como la del SDK. Se fija al planificar y viaja en el plan.
pub fn clave_nueva() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// La columna que marca, en `DELETE` y `UPDATE`, las filas que la condición toca (C6). No se escribe.
pub const MARCA: &str = "__ore_toca";

/// Qué hace el sink con las filas marcadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marca {
    /// `DELETE`: las marcadas no se escriben.
    Borrar,
    /// `UPDATE`: se escriben todas (las marcadas, ya cambiadas).
    Cambiar,
}

/// **El sink de una tabla del lago**: la primera mitad de `ore-store` y el commit por la puerta, una vez.
#[derive(Debug)]
pub struct OreSink {
    escribe: Escribe,
    tabla: Table,
    /// El esquema de Iceberg con que se escribe: el de la tabla, o el que pide `create or replace … as`.
    esquema: Schema,
    /// El mismo, en Arrow.
    arrow: SchemaRef,
    /// Lo que `DataSinkExec` coteja (`NOT NULL`): el de Arrow y, si hay marca, su columna al final.
    entrada: SchemaRef,
    operacion: Operacion,
    marca: Option<Marca>,
    clave: String,
}

impl OreSink {
    pub fn new(
        escribe: Escribe,
        tabla: Table,
        esquema: Schema,
        operacion: Operacion,
        clave: String,
    ) -> Result<OreSink> {
        let arrow = Arc::new(
            iceberg::arrow::schema_to_arrow_schema(&esquema)
                .map_err(|e| DataFusionError::External(Box::new(e)))?,
        );
        Ok(OreSink {
            escribe,
            tabla,
            esquema,
            entrada: arrow.clone(),
            arrow,
            operacion,
            marca: None,
            clave,
        })
    }

    /// Con una marca (C6): la entrada trae al final la columna [`MARCA`], y el sink cuenta lo que toca.
    pub fn con_marca(mut self, marca: Marca) -> OreSink {
        let mut campos: Vec<_> = self.arrow.fields().iter().cloned().collect();
        campos.push(Arc::new(datafusion::arrow::datatypes::Field::new(
            MARCA,
            DataType::Boolean,
            false,
        )));
        self.entrada = Arc::new(datafusion::arrow::datatypes::Schema::new(campos));
        self.marca = Some(marca);
        self
    }
}

impl DisplayAs for OreSink {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "OreSink: {} ({:?}{}, clave {})",
            self.escribe.nombre,
            self.operacion,
            self.marca.map(|m| format!(", {m:?}")).unwrap_or_default(),
            self.clave
        )
    }
}

#[async_trait]
impl DataSink for OreSink {
    fn schema(&self) -> &SchemaRef {
        &self.entrada
    }

    async fn write_all(
        &self,
        datos: SendableRecordBatchStream,
        _contexto: &Arc<TaskContext>,
    ) -> Result<u64> {
        let e = &self.escribe;
        let tocadas = Arc::new(AtomicU64::new(0));
        let (marca, arrow, t) = (self.marca, self.arrow.clone(), tocadas.clone());
        let lotes = datos.map(move |l| {
            let l = l.map_err(|e| e.to_string())?;
            match marca {
                None => Ok(l),
                Some(m) => sin_marca(l, m, &arrow, &t).map_err(|e| e.to_string()),
            }
        });
        let p = e
            .lago
            .preparar_async(
                &self.tabla,
                self.esquema.clone(),
                lotes,
                self.operacion,
                HashMap::new(),
            )
            .await
            .map_err(|m| DataFusionError::Execution(format!("`{}`: {m}", e.nombre)))?;
        let tocadas = tocadas.load(Ordering::Relaxed);
        // Un `DELETE` o un `UPDATE` que no toca ninguna fila no hace snapshot: lo escrito queda huérfano, y
        // `recoger-huerfanas` lo barre.
        if self.marca.is_some() && tocadas == 0 {
            return Ok(0);
        }
        e.puerta
            .confirmar(&e.nombre, &self.tabla, p.requisitos, p.cambios, &self.clave)
            .await
            .map_err(|f| DataFusionError::Execution(format!("`{}`: {f}", e.nombre)))?;
        Ok(if self.marca.is_some() {
            tocadas
        } else {
            p.anadidas
        })
    }
}

/// Un lote sin la columna de la marca: cuenta las marcadas y, al borrar, las quita.
fn sin_marca(
    l: RecordBatch,
    marca: Marca,
    arrow: &SchemaRef,
    tocadas: &AtomicU64,
) -> Result<RecordBatch> {
    let n = l.num_columns() - 1;
    let toca = l
        .column(n)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .ok_or_else(|| DataFusionError::Internal(format!("`{MARCA}` no es booleana")))?
        .clone();
    tocadas.fetch_add(toca.true_count() as u64, Ordering::Relaxed);
    let l = RecordBatch::try_new_with_options(
        arrow.clone(),
        l.columns()[..n].to_vec(),
        &RecordBatchOptions::new().with_row_count(Some(l.num_rows())),
    )?;
    Ok(match marca {
        Marca::Borrar => filter_record_batch(&l, &not(&toca)?)?,
        Marca::Cambiar => l,
    })
}

/// **`DELETE` y `UPDATE`** (C6), en copy-on-write sobre la foto: el escaneo entero de la tabla, una proyección con
/// la marca (`condición IS TRUE`: una fila cuya condición da `NULL` no se toca, como en SQL) y, en `UPDATE`, cada
/// columna asignada como `CASE WHEN marca THEN CAST(valor) ELSE columna END`; el sink cuenta lo que toca, quita lo
/// borrado y sobrescribe. Devuelve cuántas filas tocó.
pub(crate) async fn cambiar(
    tabla: &TablaOre,
    estado: &dyn Session,
    asignaciones: Option<Vec<(String, Expr)>>,
    filtros: Vec<Expr>,
) -> Result<Arc<dyn ExecutionPlan>> {
    let Some(escribe) = tabla.escribe().cloned() else {
        return datafusion::common::plan_err!(
            "`{}` no se escribe desde esta sesión: el catálogo no tiene puerta",
            tabla.tabla().identifier()
        );
    };
    let esquema = tabla.schema();
    let df = DFSchema::try_from(esquema.as_ref().clone())?;
    let condicion = filtros
        .into_iter()
        .reduce(Expr::and)
        .unwrap_or(datafusion::logical_expr::lit(true));
    let toca = Expr::IsTrue(Box::new(condicion));
    let asignadas: HashMap<String, Expr> = asignaciones
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    for c in asignadas.keys() {
        if esquema.field_with_name(c).is_err() {
            return datafusion::common::plan_err!("`UPDATE`: la columna `{c}` no existe");
        }
    }
    let mut proyeccion = Vec::new();
    for f in esquema.fields() {
        let columna = col(datafusion::common::Column::from_name(f.name()));
        let e = match asignadas.get(f.name()) {
            Some(valor) => {
                when(toca.clone(), cast(valor.clone(), f.data_type().clone())).otherwise(columna)?
            }
            None => columna,
        };
        proyeccion.push(ProjectionExpr {
            expr: estado.create_physical_expr(e, &df)?,
            alias: f.name().clone(),
        });
    }
    proyeccion.push(ProjectionExpr {
        expr: estado.create_physical_expr(toca, &df)?,
        alias: MARCA.to_string(),
    });
    let escaneo = tabla.scan(estado, None, &[], None).await?;
    let entrada = Arc::new(ProjectionExec::try_new(proyeccion, escaneo)?);
    let marca = if asignaciones.is_some() {
        Marca::Cambiar
    } else {
        Marca::Borrar
    };
    let sink = OreSink::new(
        escribe,
        tabla.tabla().clone(),
        tabla.esquema_de_escritura().clone(),
        Operacion::Sobrescribir,
        clave_nueva(),
    )?
    .con_marca(marca);
    Ok(Arc::new(DataSinkExec::new(entrada, Arc::new(sink), None)))
}

/// El plan físico que escribe `entrada` en una tabla con este sink.
pub fn escribir(
    escribe: Escribe,
    tabla: Table,
    esquema: Schema,
    operacion: Operacion,
    entrada: Arc<dyn ExecutionPlan>,
) -> Result<Arc<dyn ExecutionPlan>> {
    let sink = OreSink::new(escribe, tabla, esquema, operacion, clave_nueva())?;
    Ok(Arc::new(DataSinkExec::new(entrada, Arc::new(sink), None)))
}

/// Si el lago cumple este cambio del catálogo (C5): los de los datasets. Los demás son del árbol y sus productos.
pub fn la_cumple_el_lago(c: &CambioDeCatalogo) -> bool {
    matches!(
        c,
        CambioDeCatalogo::CrearDatasetVacio { .. }
            | CambioDeCatalogo::Retirar {
                que: Retirable::Dataset,
                ..
            }
    )
}

/// **El tipo con que el lago guarda una columna de cálculo** (0081 §7.5.2, la frontera): las cadenas y los binarios
/// de vista como los de siempre, el tiempo en microsegundos y la zona en UTC. Lo demás, tal cual (Iceberg dirá si
/// no cabe).
pub fn al_lago(t: &DataType) -> DataType {
    match t {
        DataType::Utf8View | DataType::LargeUtf8 => DataType::Utf8,
        DataType::BinaryView => DataType::LargeBinary,
        DataType::Timestamp(_, zona) => DataType::Timestamp(
            TimeUnit::Microsecond,
            zona.as_ref()
                .map(|_| Arc::from(ore_runtime::contrato::ZONA)),
        ),
        DataType::Time64(_) | DataType::Time32(_) => DataType::Time64(TimeUnit::Microsecond),
        DataType::Date64 => DataType::Date32,
        otro => otro.clone(),
    }
}

/// Una sentencia que no hace nada más (ya está hecha, o no había nada que hacer).
fn nada() -> LogicalPlan {
    LogicalPlan::EmptyRelation(EmptyRelation {
        produce_one_row: false,
        schema: Arc::new(DFSchema::empty()),
    })
}

fn fallo(m: impl Into<String>) -> Fallo {
    Fallo::Config(m.into())
}

/// **Lo ejecutable de un plan de ORE** (C5): un plan de DataFusion que `Motor::correr` corre. Lo de DataFusion, tal
/// cual; `create dataset … as`, como un `INSERT OVERWRITE` sobre la tabla (la que hay, o su esbozo); y los cambios
/// del catálogo que cumple el lago se hacen aquí (son un commit sin datos que correr). La sesión tiene que haber
/// planificado con el catálogo de ORE.
pub async fn ejecutable(sesion: &Sesion, plan: Plan) -> Result<LogicalPlan, Fallo> {
    let catalogos = sesion.estado().catalog_list().clone();
    let lista = || {
        catalogos
            .downcast_ref::<ListaOre>()
            .ok_or_else(|| fallo("escribir en el lago pide el catálogo de ORE (`CatalogoOre`)"))
    };
    match plan {
        // Un DML sobre una vista: no tiene filas propias.
        Plan::Logico(LogicalPlan::Dml(d))
            if d.target.table_type() == datafusion::datasource::TableType::View =>
        {
            Err(fallo(format!(
                "`{}` es una vista: no tiene filas propias que {}; se cambia su definición con `create or replace view`",
                d.table_name,
                match d.op {
                    WriteOp::Delete => "borrar",
                    WriteOp::Update => "cambiar",
                    _ => "escribir",
                }
            )))
        }
        Plan::Logico(p) => Ok(p),
        Plan::CrearDataset {
            nombre,
            reemplazar,
            si_no_existe,
            consulta,
        } => {
            let lista = lista()?;
            let n = Nombre::new(&nombre.catalog, &nombre.schema, &nombre.table);
            let escribe = lista.escribe(&n)?;
            let previa = match lista.objeto(&n) {
                Some(Objeto::Vista { .. }) => {
                    return Err(fallo(format!("`{n}` es una vista, no un dataset")));
                }
                Some(Objeto::Dataset { .. }) if si_no_existe => return Ok(nada()),
                Some(Objeto::Dataset { .. }) if !reemplazar => {
                    return Err(fallo(format!(
                        "`{n}` ya existe: `create or replace dataset` lo sustituye"
                    )));
                }
                Some(Objeto::Dataset { metadata_location }) => Some(
                    escribe
                        .lago
                        .abrir_async(&metadata_location, &n.to_string())
                        .await
                        .map_err(fallo)?,
                ),
                None => None,
            };
            // Las columnas, con los tipos del lago; la consulta, llevada a ellos con `CAST` (que falla, no inventa).
            let columnas: Vec<(String, DataType)> = consulta
                .schema()
                .fields()
                .iter()
                .map(|f| (f.name().clone(), al_lago(f.data_type())))
                .collect();
            let obligatorias: BTreeSet<String> = consulta
                .schema()
                .fields()
                .iter()
                .filter(|f| !f.is_nullable())
                .map(|f| f.name().clone())
                .collect();
            let base = previa
                .as_ref()
                .map(|t| t.metadata().current_schema().clone());
            let esquema =
                esquema_deseado(&columnas, &obligatorias, base.as_deref(), true).map_err(fallo)?;
            let tabla = match previa {
                Some(t) => t,
                None => escribe
                    .puerta
                    .esbozar(&n, esquema.clone())
                    .await
                    .map_err(fallo)?,
            };
            let proyeccion: Vec<Expr> = columnas
                .iter()
                .map(|(c, t)| {
                    cast(col(datafusion::common::Column::from_name(c)), t.clone()).alias(c)
                })
                .collect();
            let entrada = LogicalPlanBuilder::from(consulta)
                .project(proyeccion)?
                .build()?;
            let destino = TablaOre::para_escribir(tabla, esquema, escribe)?;
            Ok(LogicalPlan::Dml(DmlStatement::new(
                TableReference::from(nombre),
                provider_as_source(Arc::new(destino)),
                WriteOp::Insert(InsertOp::Overwrite),
                Arc::new(entrada),
            )))
        }
        Plan::Catalogo(CambioDeCatalogo::CrearDatasetVacio {
            nombre,
            si_no_existe,
            esquema,
        }) => {
            let lista = lista()?;
            let n = Nombre::new(&nombre.catalog, &nombre.schema, &nombre.table);
            match lista.objeto(&n) {
                Some(_) if si_no_existe => return Ok(nada()),
                Some(_) => return Err(fallo(format!("`{n}` ya existe"))),
                None => {}
            }
            let escribe = lista.escribe(&n)?;
            let columnas: Vec<(String, DataType)> = esquema
                .fields()
                .iter()
                .map(|f| (f.name().clone(), al_lago(f.data_type())))
                .collect();
            let obligatorias: BTreeSet<String> = esquema
                .fields()
                .iter()
                .filter(|f| !f.is_nullable())
                .map(|f| f.name().clone())
                .collect();
            let esquema = esquema_deseado(&columnas, &obligatorias, None, true).map_err(fallo)?;
            let tabla = escribe
                .puerta
                .esbozar(&n, esquema.clone())
                .await
                .map_err(fallo)?;
            let p = escribe
                .lago
                .preparar_async(
                    &tabla,
                    esquema,
                    futures::stream::empty(),
                    Operacion::Sobrescribir,
                    HashMap::new(),
                )
                .await
                .map_err(fallo)?;
            escribe
                .puerta
                .confirmar(&n, &tabla, p.requisitos, p.cambios, &clave_nueva())
                .await
                .map_err(|f| fallo(format!("`{n}`: {f}")))?;
            Ok(nada())
        }
        Plan::Catalogo(CambioDeCatalogo::Retirar {
            que: Retirable::Dataset,
            objetivo: Objetivo::Objeto(nombre),
            si_existe,
            ..
        }) => {
            let lista = lista()?;
            let n = Nombre::new(&nombre.catalog, &nombre.schema, &nombre.table);
            match lista.objeto(&n) {
                Some(Objeto::Dataset { metadata_location }) => {
                    let escribe = lista.escribe(&n)?;
                    escribe
                        .puerta
                        .retirar(&n, &metadata_location, &clave_nueva())
                        .await
                        .map_err(|f| fallo(format!("`{n}`: {f}")))?;
                    Ok(nada())
                }
                Some(Objeto::Vista { .. }) => Err(fallo(format!(
                    "`{n}` es una vista: se retira con `drop view`"
                ))),
                None if si_existe => Ok(nada()),
                None => Err(fallo(format!("`{n}` no existe"))),
            }
        }
        Plan::Catalogo(c) => Err(fallo(format!(
            "`{c}` lo cumple el catálogo del árbol, que vuelve con su producto; el lago cumple los datasets"
        ))),
    }
}

/// Lo que `TablaOre` hace al escribir: `INSERT INTO` anexa, `INSERT OVERWRITE` sobrescribe, `TRUNCATE` sobrescribe
/// con nada. Con el esquema vigente de la tabla: un `INSERT` no cambia el esquema (DataFusion ya llevó la entrada a
/// él).
pub(crate) async fn insertar(
    tabla: &TablaOre,
    _estado: &dyn Session,
    entrada: Arc<dyn ExecutionPlan>,
    op: InsertOp,
) -> Result<Arc<dyn ExecutionPlan>> {
    let Some(escribe) = tabla.escribe().cloned() else {
        return datafusion::common::plan_err!(
            "`{}` no se escribe desde esta sesión: el catálogo no tiene puerta",
            tabla.tabla().identifier()
        );
    };
    let operacion = match op {
        InsertOp::Append => Operacion::Anexar,
        InsertOp::Overwrite => Operacion::Sobrescribir,
        InsertOp::Replace => {
            return datafusion::common::not_impl_err!(
                "`REPLACE INTO` no es de ORE SQL: `MERGE INTO` sustituye por clave"
            );
        }
    };
    escribir(
        escribe,
        tabla.tabla().clone(),
        tabla.esquema_de_escritura().clone(),
        operacion,
        entrada,
    )
}

/// `TRUNCATE`: sobrescribe con nada.
pub(crate) fn vaciar(tabla: &TablaOre) -> Result<Arc<dyn ExecutionPlan>> {
    let Some(escribe) = tabla.escribe().cloned() else {
        return datafusion::common::plan_err!(
            "`{}` no se escribe desde esta sesión: el catálogo no tiene puerta",
            tabla.tabla().identifier()
        );
    };
    let vacio = Arc::new(datafusion::physical_plan::empty::EmptyExec::new(
        tabla.schema(),
    ));
    escribir(
        escribe,
        tabla.tabla().clone(),
        tabla.esquema_de_escritura().clone(),
        Operacion::Sobrescribir,
        vacio,
    )
}
