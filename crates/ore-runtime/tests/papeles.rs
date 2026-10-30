//! **Los tres papeles, de punta a punta** (0081 §7.4.4, S3·3): un clúster en el proceso de la prueba, con un
//! scheduler, un ejecutor y un cliente, cada uno con su `Motor`, como serían tres procesos.

use async_trait::async_trait;
use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::util::pretty::pretty_format_batches;
use datafusion::catalog::CatalogProviderList;
use datafusion::common::config::ExtensionOptions;
use datafusion::common::{ScalarValue, TableReference};
use datafusion::execution::{SessionState, SessionStateBuilder};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::prelude::{ParquetReadOptions, SessionContext};
use ore_runtime::limites::Fuente;
use ore_runtime::papeles::{self, RedDelEjecutor};
use ore_runtime::zocalos::{Catalogo, Extensiones, Ficha, Observacion, Resultado};
use ore_runtime::{Contexto, Fallo, Limites, Motor, OreOpciones, Papel, Zocalos};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Lo que una consulta puede tardar en el clúster antes de dar la prueba por perdida.
const PACIENCIA: Duration = Duration::from_secs(60);

fn limites() -> Limites {
    Limites {
        memoria: 1 << 30,
        memoria_de: Fuente::Host,
        cpu: 2,
        cpu_de: Fuente::Host,
        disco: None,
    }
}

// ── Las funciones de prueba: SÓLO están en el zócalo de extensiones ─────────────────────────────────────────

/// `opcion('ore.x')`: el valor de una opción de ORE donde corre, y el hilo que la corre.
#[derive(Debug, PartialEq, Eq, Hash)]
struct Opcion {
    firma: Signature,
}

impl ScalarUDFImpl for Opcion {
    fn name(&self) -> &str {
        "opcion"
    }
    fn signature(&self) -> &Signature {
        &self.firma
    }
    fn return_type(&self, _: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(DataType::Utf8)
    }
    fn invoke_with_args(&self, a: ScalarFunctionArgs) -> datafusion::error::Result<ColumnarValue> {
        let ColumnarValue::Scalar(ScalarValue::Utf8(Some(clave))) = &a.args[0] else {
            return datafusion::common::exec_err!("opcion('ore.x')");
        };
        let valor = a
            .config_options
            .extensions
            .get::<OreOpciones>()
            .and_then(|o| o.entries().into_iter().find(|e| &e.key == clave))
            .and_then(|e| e.value)
            .unwrap_or_else(|| "sin registrar".into());
        let hilo = std::thread::current().name().unwrap_or("?").to_string();
        Ok(ColumnarValue::Scalar(ScalarValue::Utf8(Some(format!(
            "{valor}@{hilo}"
        )))))
    }
}

/// `panico()`: un operador que entra en pánico.
#[derive(Debug, PartialEq, Eq, Hash)]
struct Panico {
    firma: Signature,
}

impl ScalarUDFImpl for Panico {
    fn name(&self) -> &str {
        "panico"
    }
    fn signature(&self) -> &Signature {
        &self.firma
    }
    fn return_type(&self, _: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(DataType::Int64)
    }
    fn invoke_with_args(&self, _: ScalarFunctionArgs) -> datafusion::error::Result<ColumnarValue> {
        panic!("un pánico a propósito en el ejecutor")
    }
}

/// Las dos funciones, añadidas a las que ya hay. Volátiles: el cliente no puede plegarlas al planificar,
/// así que corren donde corre el plan.
#[derive(Debug)]
struct DeLaPrueba;

impl Extensiones for DeLaPrueba {
    fn aplicar(&self, mut plantilla: SessionStateBuilder) -> SessionStateBuilder {
        let funciones = plantilla.scalar_functions().get_or_insert_default();
        funciones.push(Arc::new(ScalarUDF::from(Opcion {
            firma: Signature::exact(vec![DataType::Utf8], Volatility::Volatile),
        })));
        funciones.push(Arc::new(ScalarUDF::from(Panico {
            firma: Signature::nullary(Volatility::Volatile),
        })));
        plantilla
    }
}

/// La tabla `t`, en Parquet, en una lista de catálogos de cada consulta. El plan la lleva por dentro: el
/// scheduler y el ejecutor no la buscan en ningún catálogo.
#[derive(Debug)]
struct TablaT {
    dir: String,
}

#[async_trait]
impl Catalogo for TablaT {
    async fn preparar(
        &self,
        _base: &SessionState,
        _referencias: &[TableReference],
    ) -> Result<Arc<dyn CatalogProviderList>, Fallo> {
        let ctx = SessionContext::new();
        ctx.register_parquet("t", &self.dir, ParquetReadOptions::default())
            .await?;
        Ok(ctx.state().catalog_list().clone())
    }
}

/// Los planes que el cliente corrió, y cómo acabó cada consulta que falló.
#[derive(Debug, Default)]
struct Planes(Mutex<Vec<String>>, Mutex<Vec<String>>);

impl Observacion for Planes {
    fn al_empezar(&self, ficha: &Ficha) {
        self.0.lock().unwrap().push(ficha.plan.clone());
    }
    fn al_terminar(&self, _: &Ficha, r: &Resultado) {
        if let Resultado::Fallida { mensaje } = r {
            self.1.lock().unwrap().push(mensaje.clone());
        }
    }
}

// ── El clúster ───────────────────────────────────────────────────────────────────────────────────────────

struct Cluster {
    // El orden importa al soltar: el cliente, el ejecutor y el scheduler.
    cliente: Motor,
    _ejecutor: Motor,
    _scheduler: Motor,
    local: Motor,
    planes: Arc<Planes>,
    puerto: u16,
    _datos: tempfile::TempDir,
}

fn puerto_libre() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// 200 000 filas en Parquet: `k` en 0..10 y `v`.
fn datos() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ruta = dir.path().join("t");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        SessionContext::new()
            .sql(&format!(
                "copy (select v % 10 as k, v from generate_series(1, 200000) t(v)) \
                 to '{}/' stored as parquet",
                ruta.display()
            ))
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
    });
    dir
}

fn cluster() -> Cluster {
    if std::env::var_os("ORE_REGISTRO").is_some() {
        ore_runtime::registro::iniciar(log::Level::Info);
    }
    let datos = datos();
    let planes = Arc::new(Planes::default());
    let zocalos = Zocalos {
        catalogo: Arc::new(TablaT {
            dir: datos.path().join("t").display().to_string(),
        }),
        extensiones: Arc::new(DeLaPrueba),
        observacion: planes.clone(),
        ..Zocalos::default()
    };
    let motor =
        |papel, opciones| Motor::arrancar_con(papel, opciones, &zocalos, limites()).unwrap();

    let scheduler = motor(Papel::Scheduler, OreOpciones::default());
    let escucha = scheduler
        .es()
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let puerto = escucha.local_addr().unwrap().port();
    scheduler
        .es()
        .spawn(papeles::scheduler(&scheduler, escucha, "127.0.0.1").unwrap());

    let ejecutor = motor(Papel::Ejecutor, OreOpciones::default());
    let red = RedDelEjecutor {
        escucha: "127.0.0.1".into(),
        anuncia: "127.0.0.1".into(),
        puerto: puerto_libre(),
        puerto_grpc: puerto_libre(),
        scheduler: ("127.0.0.1".into(), puerto),
    };
    ejecutor
        .es()
        .spawn(papeles::ejecutor(&ejecutor, red).unwrap());

    // El cliente cambia una opción de ORE: la prueba la busca en el ejecutor.
    let opciones = OreOpciones {
        scheduler: Some(format!("df://127.0.0.1:{puerto}")),
        holgura_memoria: 0.25,
        ..OreOpciones::default()
    };
    let cliente = motor(Papel::Local, opciones);
    let local = motor(Papel::Local, OreOpciones::default());
    Cluster {
        cliente,
        _ejecutor: ejecutor,
        _scheduler: scheduler,
        local,
        planes,
        puerto,
        _datos: datos,
    }
}

fn consultar(m: &Motor, id: &str, sql: &str) -> Result<Vec<RecordBatch>, Fallo> {
    let mut s = m.sesion(&Contexto::nuevo(id))?;
    let sql = sql.to_string();
    m.es().block_on(async {
        let plan = s.planificar(&sql).await?;
        let consulta = m.correr(&s, plan)?;
        tokio::time::timeout(PACIENCIA, consulta.recoger())
            .await
            .map_err(|_| Fallo::Config(format!("`{sql}` no terminó en {PACIENCIA:?}")))?
    })
}

fn texto(lotes: &[RecordBatch]) -> String {
    pretty_format_batches(lotes).unwrap().to_string()
}

// ── Las pruebas ──────────────────────────────────────────────────────────────────────────────────────────

const AGRUPADA: &str = "select k, count(*) as n, sum(v) as s from t group by k order by k";

#[test]
fn una_consulta_distribuida_da_lo_mismo_que_en_local() {
    let c = cluster();
    let distribuida = consultar(&c.cliente, "distribuida", AGRUPADA).unwrap();
    let local = consultar(&c.local, "local", AGRUPADA).unwrap();
    assert_eq!(texto(&distribuida), texto(&local));
    assert_eq!(distribuida.iter().map(|l| l.num_rows()).sum::<usize>(), 10);
    let planes = c.planes.0.lock().unwrap();
    assert!(
        planes[0].contains("DistributedQueryExec"),
        "el cliente no distribuyó:\n{}",
        planes[0]
    );
    assert!(!planes[1].contains("DistributedQueryExec"), "{}", planes[1]);
}

#[test]
fn una_opcion_de_ore_llega_intacta_al_ejecutor() {
    let c = cluster();
    let lotes = consultar(
        &c.cliente,
        "opcion",
        "select opcion('ore.holgura_memoria') as o from t limit 1",
    )
    .unwrap();
    let o = texto(&lotes);
    // El valor del cliente (0,25, no el 0,15 por defecto del ejecutor), calculado en un hilo que no es del
    // cliente: ni su E/S ni su CPU.
    assert!(o.contains("0.25@"), "{o}");
    assert!(!o.contains("@ore-"), "corrió en el cliente: {o}");
}

#[test]
fn un_panico_en_el_ejecutor_vuelve_como_error_y_el_cluster_sigue() {
    let c = cluster();
    let e = consultar(&c.cliente, "panico", "select panico() from t").unwrap_err();
    assert!(
        e.to_string().contains("a propósito"),
        "el error dice el pánico: {e}"
    );
    let lotes = consultar(&c.cliente, "despues", AGRUPADA).unwrap();
    assert_eq!(lotes.iter().map(|l| l.num_rows()).sum::<usize>(), 10);
}

/// Soltar una consulta distribuida la cancela **en el scheduler**: lo que el suelo garantiza. Cuánto tarda
/// después el clúster en soltar la CPU es de Ballista, y está medido en 0081 §7.4.4.
#[test]
fn soltar_una_consulta_distribuida_la_cancela_en_el_scheduler() {
    use ballista_core::serde::protobuf::scheduler_grpc_client::SchedulerGrpcClient;
    use ballista_core::serde::protobuf::{GetJobStatusParams, job_status};
    let c = cluster();
    // Miles de millones de parejas: no termina sola en la prueba.
    let pesada = "select count(*) from t a join t b on a.k = b.k";
    let mut s = c.cliente.sesion(&Contexto::nuevo("pesada")).unwrap();
    let plan = c.cliente.es().block_on(s.planificar(pesada)).unwrap();
    let consulta = c.cliente.correr(&s, plan).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let soltada = Instant::now();
    drop(consulta);
    // La observación dice qué trabajo se canceló.
    let trabajo = loop {
        let dicho = c.planes.1.lock().unwrap().first().cloned();
        if let Some(m) = dicho {
            break m
                .split('`')
                .nth(1)
                .unwrap_or_else(|| panic!("sin trabajo en `{m}`"))
                .to_string();
        }
        assert!(
            soltada.elapsed() < Duration::from_secs(5),
            "sin observación"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    // Y el scheduler lo da por fallido.
    let url = format!("http://127.0.0.1:{}", c.puerto);
    let estado = c.cliente.es().block_on(async {
        let mut s = SchedulerGrpcClient::connect(url).await.unwrap();
        loop {
            let r = s
                .get_job_status(GetJobStatusParams {
                    job_id: trabajo.clone(),
                })
                .await
                .unwrap()
                .into_inner();
            let estado = r.status.and_then(|e| e.status);
            if matches!(estado, Some(job_status::Status::Failed(_)))
                || soltada.elapsed() > PACIENCIA
            {
                break estado;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    assert!(
        matches!(estado, Some(job_status::Status::Failed(_))),
        "el trabajo `{trabajo}` sigue: {estado:?}"
    );
    assert!(
        soltada.elapsed() < Duration::from_secs(5),
        "{:?}",
        soltada.elapsed()
    );
}
