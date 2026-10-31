//! **Los tres papeles, de punta a punta** (0081 §7.4.4, S3·3): un clúster en el proceso de la prueba, con un
//! scheduler, un ejecutor y un cliente, cada uno con su `Motor`, como serían tres procesos.

use async_trait::async_trait;
use datafusion::arrow::array::Int64Array;
use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::datatypes::{Field, Schema, SchemaRef};
use datafusion::arrow::util::pretty::pretty_format_batches;
use datafusion::catalog::CatalogProviderList;
use datafusion::catalog::{Session, TableProvider};
use datafusion::common::config::ExtensionOptions;
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::common::{DataFusionError, not_impl_err};
use datafusion::common::{ScalarValue, TableReference};
use datafusion::datasource::TableType;
use datafusion::execution::TaskContext;
use datafusion::execution::{SessionState, SessionStateBuilder};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::logical_expr::{Expr, Extension, LogicalPlan};
use datafusion::physical_expr::{EquivalenceProperties, PhysicalExpr};
use datafusion::physical_plan::execution_plan::{
    Boundedness, EmissionType, ReplaceChildrenOptions,
};
use datafusion::physical_plan::memory::MemoryStream;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
    SendableRecordBatchStream,
};
use datafusion::prelude::{ParquetReadOptions, SessionContext};
use datafusion_proto::logical_plan::LogicalExtensionCodec;
use datafusion_proto::physical_plan::{PhysicalExtensionCodec, PhysicalProtoConverterExtension};
use ore_runtime::codecs::Codecs;
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

    fn codecs(&self) -> Option<Codecs> {
        Some(Codecs {
            logico: Arc::new(CodecDeLaPrueba),
            fisico: Arc::new(CodecDeLaPrueba),
        })
    }
}

// ── Un nodo propio, que sólo viaja con su codec (C8) ─────────────────────────────────────────────────────────

/// `numeros`: los enteros de 1 a `n`, en dos particiones. Ni DataFusion ni Ballista saben serializar la tabla ni su
/// escaneo: sólo el codec de la prueba, que da el zócalo de extensiones.
#[derive(Debug)]
struct Numeros(i64);

fn esquema_de_numeros() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]))
}

#[async_trait]
impl TableProvider for Numeros {
    fn schema(&self) -> SchemaRef {
        esquema_de_numeros()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        _: &dyn Session,
        _: Option<&Vec<usize>>,
        _: &[Expr],
        _: Option<usize>,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        Ok(Arc::new(NumerosExec::new(self.0)))
    }
}

#[derive(Debug)]
struct NumerosExec {
    n: i64,
    propiedades: Arc<PlanProperties>,
}

impl NumerosExec {
    fn new(n: i64) -> Self {
        NumerosExec {
            n,
            propiedades: Arc::new(PlanProperties::new(
                EquivalenceProperties::new(esquema_de_numeros()),
                Partitioning::UnknownPartitioning(2),
                EmissionType::Incremental,
                Boundedness::Bounded,
            )),
        }
    }
}

impl DisplayAs for NumerosExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "NumerosExec: n={}", self.n)
    }
}

impl ExecutionPlan for NumerosExec {
    fn name(&self) -> &'static str {
        "NumerosExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.propiedades
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }
    fn apply_expressions(
        &self,
        _: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> datafusion::error::Result<TreeNodeRecursion>,
    ) -> datafusion::error::Result<TreeNodeRecursion> {
        Ok(TreeNodeRecursion::Continue)
    }
    fn replace_children(
        self: Arc<Self>,
        _: Vec<Arc<dyn ExecutionPlan>>,
        _: ReplaceChildrenOptions,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        Ok(self)
    }
    fn with_new_children(
        self: Arc<Self>,
        _: Vec<Arc<dyn ExecutionPlan>>,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        Ok(self)
    }
    fn execute(
        &self,
        particion: usize,
        _: Arc<TaskContext>,
    ) -> datafusion::error::Result<SendableRecordBatchStream> {
        // La partición 0, los pares; la 1, los impares.
        let valores: Vec<i64> = (1..=self.n)
            .filter(|v| (v % 2) as usize == particion)
            .collect();
        let lote = RecordBatch::try_new(
            esquema_de_numeros(),
            vec![Arc::new(Int64Array::from(valores))],
        )?;
        Ok(Box::pin(MemoryStream::try_new(
            vec![lote],
            esquema_de_numeros(),
            None,
        )?))
    }
}

/// El codec de la prueba: sólo `Numeros` y `NumerosExec`, con su `n`; lo demás no es suyo.
#[derive(Debug)]
struct CodecDeLaPrueba;

fn n_de(buf: &[u8]) -> datafusion::error::Result<i64> {
    Ok(i64::from_le_bytes(buf.try_into().map_err(|_| {
        DataFusionError::Internal("no es un `n`".into())
    })?))
}

impl LogicalExtensionCodec for CodecDeLaPrueba {
    fn try_decode(
        &self,
        _: &[u8],
        _: &[LogicalPlan],
        _: &TaskContext,
    ) -> datafusion::error::Result<Extension> {
        not_impl_err!("sin nodos lógicos")
    }
    fn try_encode(&self, _: &Extension, _: &mut Vec<u8>) -> datafusion::error::Result<()> {
        not_impl_err!("sin nodos lógicos")
    }
    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        _: &TableReference,
        _: SchemaRef,
        _: &TaskContext,
    ) -> datafusion::error::Result<Arc<dyn TableProvider>> {
        Ok(Arc::new(Numeros(n_de(buf)?)))
    }
    fn try_encode_table_provider(
        &self,
        _: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> datafusion::error::Result<()> {
        let Some(t) = node.as_ref().downcast_ref::<Numeros>() else {
            return not_impl_err!("no es `numeros`");
        };
        buf.extend_from_slice(&t.0.to_le_bytes());
        Ok(())
    }
}

impl PhysicalExtensionCodec for CodecDeLaPrueba {
    fn try_decode(
        &self,
        buf: &[u8],
        _: &[Arc<dyn ExecutionPlan>],
        _: &TaskContext,
        _: &dyn PhysicalProtoConverterExtension,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        Ok(Arc::new(NumerosExec::new(n_de(buf)?)))
    }
    fn try_encode(
        &self,
        node: Arc<dyn ExecutionPlan>,
        buf: &mut Vec<u8>,
        _: &dyn PhysicalProtoConverterExtension,
    ) -> datafusion::error::Result<()> {
        let Some(e) = node.as_ref().downcast_ref::<NumerosExec>() else {
            return not_impl_err!("no es `NumerosExec`");
        };
        buf.extend_from_slice(&e.n.to_le_bytes());
        Ok(())
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
        ctx.register_table("numeros", Arc::new(Numeros(1000)))?;
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
        let plan = s.planificar(&sql).await?.logico()?;
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

/// Un nodo que sólo sabe serializar el codec del zócalo hace el viaje: el cliente manda la tabla al scheduler, el
/// scheduler manda su escaneo al ejecutor, y el barajado de Ballista sigue viajando con su codec (C8).
#[test]
fn un_nodo_propio_viaja_con_el_codec_del_zocalo() {
    let c = cluster();
    let sql =
        "select x % 3 as r, count(*) as n, sum(x) as s from numeros group by x % 3 order by r";
    let distribuida = consultar(&c.cliente, "numeros", sql).unwrap();
    let local = consultar(&c.local, "numeros-local", sql).unwrap();
    assert_eq!(texto(&distribuida), texto(&local));
    assert!(
        texto(&distribuida).contains("| 0 | 333 | 166833 |"),
        "{}",
        texto(&distribuida)
    );
    let planes = c.planes.0.lock().unwrap();
    assert!(planes[0].contains("DistributedQueryExec"), "{}", planes[0]);
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
    let plan = c
        .cliente
        .es()
        .block_on(s.planificar(pesada))
        .unwrap()
        .logico()
        .unwrap();
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
