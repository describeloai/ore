//! **La vara de ORE** (0081 §7.5.2): la definición ejecutable del contrato, ORE SQL. Cada `.slt` de
//! `tests/contrato/` corre sobre una sesión de ORE de verdad, por el mismo camino que una consulta: `Motor` →
//! `Sesion::planificar` (con la política de la sesión y el zócalo de catálogo de ORE) → `Motor::correr`. La vara
//! de DataFusion (`ore-runtime/tests/vara.rs`) prueba lo adoptado; ésta, lo que pone ORE.
//!
//! Vive aquí y no en `ore-runtime` porque necesita el lago: un lago en memoria ([`LagoEnMemoria`]) con los
//! datasets y las vistas de [`sembrar`], escritos con el mismo `Lago` que en producción. La sesión trabaja en la
//! base `ventas` y escribe.
//!
//! El formato y los tipos de columna son los del corpus de DataFusion (`convert_batches`), para que un `.slt`
//! de ORE se lea igual que uno de arriba.

use async_trait::async_trait;
use datafusion::arrow::array::{ArrayRef, Int64Array, StringArray};
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::error::DataFusionError;
use datafusion::execution::SessionStateBuilder;
use datafusion::logical_expr::{ColumnarValue, Volatility, create_udf};
use datafusion_sqllogictest::{DFColumnType, DFOutput, convert_batches, convert_schema_to_types};
use ore_runtime::zocalos::Extensiones;
use ore_runtime::{Contexto, Fallo, Motor, OreOpciones, Papel, Plan, Zocalos};
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::{CatalogoOre, Nombre};
use sqllogictest::{DBOutput, Runner};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// La base en la que trabaja la sesión de la vara.
const BASE: &str = "ventas";

/// Una conexión de la vara: cada sentencia abre su sesión, como cada consulta en ORE.
struct Ore {
    motor: Arc<Motor>,
    n: Arc<AtomicU64>,
}

#[async_trait]
impl sqllogictest::AsyncDB for Ore {
    type Error = Fallo;
    type ColumnType = DFColumnType;

    async fn run(&mut self, sql: &str) -> Result<DFOutput, Fallo> {
        let id = format!("contrato-{}", self.n.fetch_add(1, Ordering::Relaxed));
        // La vara es una superficie que escribe: lo que una que sólo lee no admite se prueba en `suelo.rs`.
        let contexto = Contexto::nuevo(id).en_base(BASE).que_escribe();
        let mut sesion = self.motor.sesion(&contexto)?;
        // Escribir en el lago y cambiar el catálogo llegan con C5: hasta entonces, la vara enseña lo planificado,
        // en una fila, y un `.slt` lo comprueba.
        let planificado = |texto: String| {
            Ok(DBOutput::Rows {
                types: vec![DFColumnType::Text],
                rows: vec![vec![texto]],
            })
        };
        let plan = match sesion.planificar(sql).await? {
            Plan::Logico(plan) => plan,
            Plan::CrearDataset {
                nombre,
                reemplazar,
                si_no_existe,
                consulta,
            } => {
                let columnas: Vec<&str> = consulta
                    .schema()
                    .fields()
                    .iter()
                    .map(|c| c.name().as_str())
                    .collect();
                return planificado(format!(
                    "create {}dataset {}{nombre} as a query of ({})",
                    if reemplazar { "or replace " } else { "" },
                    if si_no_existe { "if not exists " } else { "" },
                    columnas.join(", ")
                ));
            }
            Plan::Catalogo(cambio) => return planificado(cambio.to_string()),
        };
        let esquema = Arc::clone(plan.schema().inner());
        let lotes = self.motor.correr(&sesion, plan)?.recoger().await?;
        let tipos = convert_schema_to_types(esquema.fields());
        let filas = convert_batches(&esquema, lotes, false)
            .map_err(|e| Fallo::Ejecucion(DataFusionError::Execution(e.to_string())))?;
        Ok(if filas.is_empty() && tipos.is_empty() {
            DBOutput::StatementComplete(0)
        } else {
            DBOutput::Rows {
                types: tipos,
                rows: filas,
            }
        })
    }

    fn engine_name(&self) -> &str {
        "ORE"
    }

    async fn sleep(dur: std::time::Duration) {
        tokio::time::sleep(dur).await;
    }

    async fn shutdown(&mut self) {}
}

/// Una Function del árbol, como la registrará su producto: con su nombre entero, `functions.<def>`.
#[derive(Debug)]
struct ConUnaFunction;

impl Extensiones for ConUnaFunction {
    fn aplicar(&self, mut plantilla: SessionStateBuilder) -> SessionStateBuilder {
        let doble = create_udf(
            "functions.doble",
            vec![DataType::Int64],
            DataType::Int64,
            Volatility::Immutable,
            Arc::new(|args: &[ColumnarValue]| {
                let a = args[0].clone().into_array(1)?;
                let a = a.as_any().downcast_ref::<Int64Array>().unwrap();
                Ok(ColumnarValue::Array(Arc::new(
                    a.iter().map(|v| v.map(|v| v * 2)).collect::<Int64Array>(),
                )))
            }),
        );
        plantilla
            .scalar_functions()
            .get_or_insert_with(Vec::new)
            .push(Arc::new(doble));
        plantilla
    }
}

fn lote(columnas: Vec<(&str, ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter(columnas).unwrap()
}

fn enteros(v: &[i64]) -> ArrayRef {
    Arc::new(Int64Array::from(v.to_vec()))
}

fn textos(v: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(v.to_vec()))
}

/// **El lago de la vara**: dos bases, tres schemas, tres datasets y tres vistas.
fn sembrar(l: &LagoEnMemoria) {
    l.escribir(
        &Nombre::new("ventas", "default", "pedidos"),
        vec![
            lote(vec![
                ("id", enteros(&[1, 2])),
                ("cliente", textos(&["ana", "luis"])),
                ("importe", enteros(&[50, 150])),
            ]),
            lote(vec![
                ("id", enteros(&[3])),
                ("cliente", textos(&["ana"])),
                ("importe", enteros(&[300])),
            ]),
        ],
    )
    .unwrap();
    l.escribir(
        &Nombre::new("ventas", "eu", "clientes"),
        vec![lote(vec![
            ("cliente", textos(&["ana", "luis"])),
            ("pais", textos(&["es", "pt"])),
        ])],
    )
    .unwrap();
    l.escribir(
        &Nombre::new("rrhh", "default", "personas"),
        vec![lote(vec![
            ("id", enteros(&[7])),
            ("nombre", textos(&["eva"])),
        ])],
    )
    .unwrap();
    // Una vista nombra en su base y su schema: `pedidos` es `ventas.default.pedidos`.
    l.vista(
        &Nombre::new("ventas", "default", "grandes"),
        "select id, importe from pedidos where importe > 100",
    );
    // Desde `ventas.eu`: `clientes` es la de `eu`, y `default.pedidos`, la de su base.
    l.vista(
        &Nombre::new("ventas", "eu", "cruce"),
        "select p.id, c.pais from \"default\".pedidos p join clientes c on p.cliente = c.cliente",
    );
    l.vista(
        &Nombre::new("ventas", "default", "bucle"),
        "select * from bucle",
    );
}

fn ficheros() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contrato");
    let mut f: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("tests/contrato")
        .map(|e| e.expect("una entrada").path())
        .filter(|p| p.extension().is_some_and(|e| e == "slt"))
        .collect();
    f.sort();
    f
}

#[test]
fn el_contrato_se_cumple() {
    let lago = LagoEnMemoria::default();
    sembrar(&lago);
    let zocalos = Zocalos {
        catalogo: Arc::new(CatalogoOre::new(lago.fuente(), lago.lago().clone())),
        extensiones: Arc::new(ConUnaFunction),
        ..Zocalos::default()
    };
    let motor = Arc::new(
        Motor::arrancar(Papel::Prueba, OreOpciones::default(), &zocalos)
            .expect("el motor de prueba"),
    );
    let ficheros = ficheros();
    assert!(!ficheros.is_empty(), "la vara de ORE sin ficheros");
    let n = Arc::new(AtomicU64::new(0));
    let mut fallos = Vec::new();
    for f in &ficheros {
        let (m, n) = (Arc::clone(&motor), Arc::clone(&n));
        let mut runner = Runner::new(move || {
            let (motor, n) = (Arc::clone(&m), Arc::clone(&n));
            async move { Ok(Ore { motor, n }) }
        });
        runner.with_column_validator(sqllogictest::strict_column_validator);
        runner.with_normalizer(datafusion_sqllogictest::value_normalizer);
        runner.with_validator(datafusion_sqllogictest::df_value_validator);
        if let Err(e) = motor.es().block_on(runner.run_file_async(f)) {
            fallos.push(format!("{}: {e}", f.display()));
        }
    }
    assert!(fallos.is_empty(), "la vara de ORE:\n{}", fallos.join("\n"));
}
