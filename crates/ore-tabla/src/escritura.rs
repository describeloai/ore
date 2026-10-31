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
use datafusion::arrow::array::{BooleanArray, UInt64Array};
use datafusion::arrow::compute::filter_record_batch;
use datafusion::arrow::compute::kernels::boolean::not;
use datafusion::arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef, TimeUnit};
use datafusion::arrow::record_batch::{RecordBatch, RecordBatchOptions};
use datafusion::catalog::{Session, TableProvider};
use datafusion::common::{
    Column, DFSchema, DFSchemaRef, DataFusionError, Result, ScalarValue, TableReference,
};
use datafusion::datasource::provider_as_source;
use datafusion::datasource::sink::{DataSink, DataSinkExec};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::functions_window::expr_fn::row_number;
use datafusion::logical_expr::dml::InsertOp;
use datafusion::logical_expr::dml::{MergeIntoAction, MergeIntoClause, MergeIntoClauseKind};
use datafusion::logical_expr::when;
use datafusion::logical_expr::{
    DmlStatement, EmptyRelation, Expr, LogicalPlan, LogicalPlanBuilder, WriteOp, cast, col,
};
use datafusion::logical_expr::{JoinType, lit};
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
use std::collections::HashSet;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;

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

/// **Las marcas** (C6 y C7): tres columnas al final de la entrada del sink en `DELETE`, `UPDATE` y `MERGE INTO`, que no
/// se escriben. `toca`: la fila cuenta como tocada. `quita`: la fila no se escribe. `fila`: de qué fila del destino viene
/// una fila casada en `MERGE`, para ver que ninguna case dos veces (o nula).
pub const TOCA: &str = "__ore_toca";
pub const QUITA: &str = "__ore_quita";
pub const FILA: &str = "__ore_fila";

/// **El sink de una tabla del lago**: la primera mitad de `ore-store` y el commit por la puerta, una vez.
#[derive(Debug)]
pub struct OreSink {
    escribe: Escribe,
    tabla: Table,
    /// El esquema de Iceberg con que se escribe: el de la tabla, o el que pide `create or replace … as`.
    esquema: Schema,
    /// El mismo, en Arrow.
    arrow: SchemaRef,
    /// Lo que `DataSinkExec` coteja (`NOT NULL`): el de Arrow y, con marcas, sus tres columnas al final.
    entrada: SchemaRef,
    operacion: Operacion,
    marcas: bool,
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
            marcas: false,
            clave,
        })
    }

    /// Con las marcas (C6 y C7): la entrada trae al final [`TOCA`], [`QUITA`] y [`FILA`]; el sink cuenta lo que toca,
    /// quita lo que se quita y cuida que ninguna fila del destino case dos veces.
    pub fn con_marcas(mut self) -> OreSink {
        let mut campos: Vec<_> = self.arrow.fields().iter().cloned().collect();
        campos.push(Arc::new(Field::new(TOCA, DataType::Boolean, false)));
        campos.push(Arc::new(Field::new(QUITA, DataType::Boolean, false)));
        campos.push(Arc::new(Field::new(FILA, DataType::UInt64, true)));
        self.entrada = Arc::new(ArrowSchema::new(campos));
        self.marcas = true;
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
            if self.marcas { ", con marcas" } else { "" },
            self.clave
        )
    }
}

/// Lo que el sink lleva contado de las marcas.
#[derive(Debug, Default)]
struct Cuentas {
    tocadas: u64,
    casadas: HashSet<u64>,
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
        let cuentas = Arc::new(Mutex::new(Cuentas::default()));
        let (marcas, arrow, c) = (self.marcas, self.arrow.clone(), cuentas.clone());
        let lotes = datos.map(move |l| {
            let l = l.map_err(|e| e.to_string())?;
            if marcas {
                sin_marcas(l, &arrow, &c).map_err(|e| e.to_string())
            } else {
                Ok(l)
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
        let tocadas = cuentas.lock().unwrap().tocadas;
        // Lo que no toca ninguna fila no hace snapshot: lo escrito queda huérfano, y `recoger-huerfanas` lo barre.
        if self.marcas && tocadas == 0 {
            return Ok(0);
        }
        e.puerta
            .confirmar(&e.nombre, &self.tabla, p.requisitos, p.cambios, &self.clave)
            .await
            .map_err(|f| DataFusionError::Execution(format!("`{}`: {f}", e.nombre)))?;
        Ok(if self.marcas { tocadas } else { p.anadidas })
    }
}

/// Un lote sin sus marcas: cuenta las tocadas, quita las que se quitan y cuida que ninguna fila case dos veces.
fn sin_marcas(l: RecordBatch, arrow: &SchemaRef, cuentas: &Mutex<Cuentas>) -> Result<RecordBatch> {
    let n = l.num_columns() - 3;
    let booleana = |i: usize, nombre: &str| {
        l.column(i)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .cloned()
            .ok_or_else(|| DataFusionError::Internal(format!("`{nombre}` no es booleana")))
    };
    let (toca, quita) = (booleana(n, TOCA)?, booleana(n + 1, QUITA)?);
    let fila = l
        .column(n + 2)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .cloned()
        .ok_or_else(|| DataFusionError::Internal(format!("`{FILA}` no es UInt64")))?;
    {
        let mut c = cuentas.lock().unwrap();
        c.tocadas += toca.true_count() as u64;
        for f in fila.iter().flatten() {
            if !c.casadas.insert(f) {
                return Err(DataFusionError::Execution(
                    "`MERGE INTO`: una fila del destino casa con más de una de la fuente; la condición `on` \
                     tiene que dar a cada fila del destino una sola de la fuente"
                        .into(),
                ));
            }
        }
    }
    let l = RecordBatch::try_new_with_options(
        arrow.clone(),
        l.columns()[..n].to_vec(),
        &RecordBatchOptions::new().with_row_count(Some(l.num_rows())),
    )?;
    Ok(filter_record_batch(&l, &not(&quita)?)?)
}

fn sin_puerta(tabla: &TablaOre) -> DataFusionError {
    DataFusionError::Plan(format!(
        "`{}` no se escribe desde esta sesión: el catálogo no tiene puerta",
        tabla.tabla().identifier()
    ))
}

/// El sink con marcas que sobrescribe `tabla` con `entrada`.
fn sobrescribir_con_marcas(
    tabla: &TablaOre,
    entrada: Arc<dyn ExecutionPlan>,
) -> Result<Arc<dyn ExecutionPlan>> {
    let escribe = tabla.escribe().cloned().ok_or_else(|| sin_puerta(tabla))?;
    let sink = OreSink::new(
        escribe,
        tabla.tabla().clone(),
        tabla.esquema_de_escritura().clone(),
        Operacion::Sobrescribir,
        clave_nueva(),
    )?
    .con_marcas();
    Ok(Arc::new(DataSinkExec::new(entrada, Arc::new(sink), None)))
}

/// **`DELETE` y `UPDATE`** (C6), en copy-on-write sobre la foto: el escaneo entero de la tabla y una proyección con
/// las marcas (`toca` es `condición IS TRUE`: una fila cuya condición da `NULL` no se toca, como en SQL; al borrar,
/// lo que toca se quita) y, en `UPDATE`, cada columna asignada como `CASE WHEN toca THEN CAST(valor) ELSE columna
/// END`. El sink sobrescribe y dice cuántas filas tocó.
pub(crate) async fn cambiar(
    tabla: &TablaOre,
    estado: &dyn Session,
    asignaciones: Option<Vec<(String, Expr)>>,
    filtros: Vec<Expr>,
) -> Result<Arc<dyn ExecutionPlan>> {
    let esquema = tabla.schema();
    let df = DFSchema::try_from(esquema.as_ref().clone())?;
    let condicion = filtros.into_iter().reduce(Expr::and).unwrap_or(lit(true));
    let toca = Expr::IsTrue(Box::new(condicion));
    let borrar = asignaciones.is_none();
    let asignadas: HashMap<String, Expr> = asignaciones.unwrap_or_default().into_iter().collect();
    for c in asignadas.keys() {
        if esquema.field_with_name(c).is_err() {
            return datafusion::common::plan_err!("`UPDATE`: la columna `{c}` no existe");
        }
    }
    let mut exprs = Vec::new();
    for f in esquema.fields() {
        let columna = col(Column::from_name(f.name()));
        let e = match asignadas.get(f.name()) {
            Some(valor) => {
                when(toca.clone(), cast(valor.clone(), f.data_type().clone())).otherwise(columna)?
            }
            None => columna,
        };
        exprs.push((e, f.name().clone()));
    }
    let quita = if borrar { toca.clone() } else { lit(false) };
    exprs.push((toca, TOCA.into()));
    exprs.push((quita, QUITA.into()));
    exprs.push((lit(ScalarValue::UInt64(None)), FILA.into()));
    let proyeccion = exprs
        .into_iter()
        .map(|(e, alias)| {
            Ok(ProjectionExpr {
                expr: estado.create_physical_expr(e, &df)?,
                alias,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let escaneo = tabla.scan(estado, None, &[], None).await?;
    sobrescribir_con_marcas(
        tabla,
        Arc::new(ProjectionExec::try_new(proyeccion, escaneo)?),
    )
}

/// La fuente de un `MERGE`, ya planificada por DataFusion, como tabla: para volver a ponerla en un plan lógico.
#[derive(Debug)]
struct FuenteDeMerge(Arc<dyn ExecutionPlan>);

#[async_trait]
impl TableProvider for FuenteDeMerge {
    fn schema(&self) -> SchemaRef {
        self.0.schema()
    }

    fn table_type(&self) -> datafusion::datasource::TableType {
        datafusion::datasource::TableType::Temporary
    }

    async fn scan(
        &self,
        _estado: &dyn Session,
        proyeccion: Option<&Vec<usize>>,
        _filtros: &[Expr],
        _limite: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let Some(p) = proyeccion else {
            return Ok(self.0.clone());
        };
        let esquema = self.0.schema();
        let columnas = p
            .iter()
            .map(|&i| {
                let nombre = esquema.field(i).name().clone();
                Ok(ProjectionExpr {
                    expr: Arc::new(datafusion::physical_expr::expressions::Column::new(
                        &nombre, i,
                    )),
                    alias: nombre,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Arc::new(ProjectionExec::try_new(columnas, self.0.clone())?))
    }
}

/// **`MERGE INTO`** (C7), en copy-on-write sobre la foto. Un `FULL JOIN` entre el destino (con el número de cada fila)
/// y la fuente por la condición `on`; cada fila casada, del destino sola o de la fuente sola, toma la **primera**
/// cláusula de su clase cuya condición se cumpla:
///
/// - `WHEN MATCHED [AND …] THEN UPDATE SET … | DELETE`;
/// - `WHEN NOT MATCHED [BY TARGET] [AND …] THEN INSERT [(cols)] VALUES (…)`;
/// - `WHEN NOT MATCHED BY SOURCE [AND …] THEN UPDATE SET … | DELETE`.
///
/// Lo que no toma ninguna se queda como estaba (y una fila sólo de la fuente, fuera). El sink cuenta lo tocado, quita
/// lo borrado, cuida que ninguna fila del destino case con dos de la fuente y sobrescribe en un commit. Es el upsert
/// de ORE: `when matched then update set … when not matched then insert …`.
pub(crate) async fn fusionar(
    tabla: &TablaOre,
    estado: &dyn Session,
    fuente: Arc<dyn ExecutionPlan>,
    esquema: DFSchemaRef,
    on: Expr,
    clausulas: Vec<MergeIntoClause>,
) -> Result<Arc<dyn ExecutionPlan>> {
    let destino = tabla.schema();
    let n = destino.fields().len();
    let campos: Vec<(Option<TableReference>, Arc<Field>)> = esquema
        .iter()
        .map(|(q, f)| (q.cloned(), f.clone()))
        .collect();
    let qualifier = campos
        .first()
        .and_then(|(q, _)| q.clone())
        .ok_or_else(|| DataFusionError::Plan("`MERGE INTO`: el destino sin nombre".into()))?;
    // El destino, con un número por fila y su presencia; la fuente, con sus nombres y su presencia.
    let d = LogicalPlanBuilder::scan(
        qualifier.clone(),
        provider_as_source(Arc::new(tabla.clone())),
        None,
    )?
    .window(vec![row_number().alias(FILA)])?
    .build()?;
    let mut d_cols: Vec<Expr> = destino
        .fields()
        .iter()
        .map(|f| col(Column::new(Some(qualifier.clone()), f.name())))
        .collect();
    d_cols.push(col(Column::from_name(FILA)).alias(FILA_D));
    d_cols.push(lit(true).alias(EN_DESTINO));
    let d = LogicalPlanBuilder::from(d).project(d_cols)?.build()?;
    let f = LogicalPlanBuilder::scan(
        "__ore_fuente",
        provider_as_source(Arc::new(FuenteDeMerge(fuente))),
        None,
    )?
    .build()?;
    let mut f_cols: Vec<Expr> = campos[n..]
        .iter()
        .map(|(q, campo)| {
            col(Column::new(Some("__ore_fuente"), campo.name()))
                .alias_qualified(q.clone(), campo.name())
        })
        .collect();
    f_cols.push(lit(true).alias(EN_FUENTE));
    let f = LogicalPlanBuilder::from(f).project(f_cols)?.build()?;
    let unido = LogicalPlanBuilder::from(d)
        .join_on(f, JoinType::Full, vec![on])?
        .build()?;

    // La cláusula que toma cada fila: la primera de su clase que se cumple, o -1.
    let en_d = col(Column::from_name(EN_DESTINO)).is_not_null();
    let en_f = col(Column::from_name(EN_FUENTE)).is_not_null();
    let casada = en_d.clone().and(en_f.clone());
    let solo_d = en_d.clone().and(!en_f.clone());
    let solo_f = en_f.clone().and(!en_d.clone());
    let mut cual = Vec::new();
    for (i, c) in clausulas.iter().enumerate() {
        let clase = match c.kind {
            MergeIntoClauseKind::Matched => casada.clone(),
            MergeIntoClauseKind::NotMatchedBySource => solo_d.clone(),
            _ => solo_f.clone(),
        };
        let condicion = match &c.predicate {
            Some(p) => clase.and(Expr::IsTrue(Box::new(p.clone()))),
            None => clase,
        };
        cual.push((condicion, lit(i as i64)));
    }
    let accion = if cual.is_empty() {
        lit(-1i64)
    } else {
        let mut caso = when(cual[0].0.clone(), cual[0].1.clone());
        for (c, v) in &cual[1..] {
            caso = caso.when(c.clone(), v.clone());
        }
        caso.otherwise(lit(-1i64))?
    };
    let mut paso: Vec<Expr> = unido.schema().columns().into_iter().map(col).collect();
    paso.push(accion.alias(ACCION));
    let unido = LogicalPlanBuilder::from(unido).project(paso)?.build()?;

    // Lo que sale, columna a columna, y las marcas.
    let accion = || col(Column::from_name(ACCION));
    let es = |i: usize| accion().eq(lit(i as i64));
    let mut salida = Vec::new();
    for (j, campo) in destino.fields().iter().enumerate() {
        let columna = col(Column::new(Some(qualifier.clone()), campo.name()));
        let mut casos: Vec<(Expr, Expr)> = Vec::new();
        for (i, c) in clausulas.iter().enumerate() {
            let valor = match &c.action {
                MergeIntoAction::Update(asignaciones) => asignaciones
                    .iter()
                    .find(|(nombre, _)| nombre == campo.name())
                    .map(|(_, v)| v.clone()),
                MergeIntoAction::Insert { columns, values } => {
                    if columns.is_empty() {
                        if values.len() != n {
                            return datafusion::common::plan_err!(
                                "`MERGE INTO`: `insert values` da {} valores y el destino tiene {n} columnas",
                                values.len()
                            );
                        }
                        Some(values[j].clone())
                    } else {
                        Some(
                            columns
                                .iter()
                                .position(|c| c == campo.name())
                                .map(|k| values[k].clone())
                                .unwrap_or(lit(ScalarValue::Null)),
                        )
                    }
                }
                MergeIntoAction::Delete => None,
            };
            if let Some(v) = valor {
                casos.push((es(i), cast(v, campo.data_type().clone())));
            }
        }
        let e = match casos.split_first() {
            None => columna,
            Some(((c0, v0), resto)) => {
                let mut caso = when(c0.clone(), v0.clone());
                for (c, v) in resto {
                    caso = caso.when(c.clone(), v.clone());
                }
                caso.otherwise(columna)?
            }
        };
        salida.push(e.alias(campo.name()));
    }
    let borra = clausulas
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.action, MergeIntoAction::Delete))
        .map(|(i, _)| es(i));
    let inserta = clausulas
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.action, MergeIntoAction::Insert { .. }))
        .map(|(i, _)| es(i))
        .reduce(Expr::or)
        .unwrap_or(lit(false));
    // Se quita lo borrado y lo que sólo está en la fuente sin insertarse.
    let quita = borra
        .chain(std::iter::once(
            col(Column::from_name(EN_DESTINO)).is_null().and(!inserta),
        ))
        .reduce(Expr::or)
        .unwrap_or(lit(false));
    salida.push(accion().gt_eq(lit(0i64)).alias(TOCA));
    salida.push(quita.alias(QUITA));
    salida.push(
        when(
            col(Column::from_name(EN_DESTINO))
                .is_not_null()
                .and(col(Column::from_name(EN_FUENTE)).is_not_null()),
            col(Column::from_name(FILA_D)),
        )
        .otherwise(lit(ScalarValue::UInt64(None)))?
        .alias(FILA),
    );
    let plan = LogicalPlanBuilder::from(unido).project(salida)?.build()?;
    let entrada = estado.create_physical_plan(&plan).await?;
    sobrescribir_con_marcas(tabla, entrada)
}

/// Las columnas de paso de `MERGE`: que la fila está en el destino, en la fuente, su número en el destino y la cláusula
/// que toma.
const EN_DESTINO: &str = "__ore_en_destino";
const EN_FUENTE: &str = "__ore_en_fuente";
const FILA_D: &str = "__ore_fila_destino";
const ACCION: &str = "__ore_accion";

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
