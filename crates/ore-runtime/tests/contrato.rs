//! **La vara de ORE** (0081 §7.5.2): la definición ejecutable del contrato, ORE SQL. Cada `.slt` de
//! `tests/contrato/` corre sobre una sesión de ORE de verdad, por el mismo camino que una consulta: `Motor` →
//! `Sesion::planificar` (con la política de la sesión y el zócalo de catálogo) → `Motor::correr`. La vara de
//! DataFusion (`tests/vara.rs`) prueba lo adoptado; ésta, lo que pone ORE.
//!
//! El formato y los tipos de columna son los del corpus de DataFusion (`convert_batches`), para que un `.slt`
//! de ORE se lea igual que uno de arriba.

use async_trait::async_trait;
use datafusion::error::DataFusionError;
use datafusion_sqllogictest::{DFColumnType, DFOutput, convert_batches, convert_schema_to_types};
use ore_runtime::{Contexto, Fallo, Motor, OreOpciones, Papel, Zocalos};
use sqllogictest::{DBOutput, Runner};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
        let mut sesion = self.motor.sesion(&Contexto::nuevo(id))?;
        let plan = sesion.planificar(sql).await?;
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

    async fn shutdown(&mut self) {}

    async fn sleep(dur: std::time::Duration) {
        tokio::time::sleep(dur).await;
    }
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
    let motor = Arc::new(
        Motor::arrancar(Papel::Prueba, OreOpciones::default(), &Zocalos::default())
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
