//! **El suelo de punta a punta** (0081 §7.4.3, S2·5–S2·7): la sesión que nace de la plantilla, `correr` y
//! los zócalos, con un motor de verdad.

// El asignador del binario, para que la medida de musl sea la del binario (0081 §7.4.4, S3·4).
#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use async_trait::async_trait;
use datafusion::arrow::array::{Int64Array, RecordBatch};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::catalog::{
    CatalogProvider, CatalogProviderList, MemTable, MemoryCatalogProvider,
    MemoryCatalogProviderList, MemorySchemaProvider,
};
use datafusion::common::TableReference;
use datafusion::execution::object_store::{DefaultObjectStoreRegistry, ObjectStoreRegistry};
use datafusion::execution::{SessionState, SessionStateBuilder};
use datafusion::logical_expr::{ColumnarValue, Volatility, create_udf};
use ore_runtime::Fallo;
use ore_runtime::limites::{Fuente, Limites};
use ore_runtime::manifiesto::Manifiesto;
use ore_runtime::zocalos::{
    Almacenes, Catalogo, Extensiones, Ficha, Observacion, Resultado, Zocalos,
};
use ore_runtime::{Contexto, Motor, OreOpciones, Papel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn limites(cpu: usize) -> Limites {
    Limites {
        memoria: 1 << 30,
        memoria_de: Fuente::Host,
        cpu,
        cpu_de: Fuente::Host,
        disco: None,
    }
}

fn motor(zocalos: Zocalos, cpu: usize) -> Motor {
    Motor::arrancar_con(Papel::Local, OreOpciones::default(), &zocalos, limites(cpu)).unwrap()
}

/// Planifica y corre `sql` en una sesión nueva, desde el runtime de E/S (quien pide no es el de CPU).
fn consultar(m: &Motor, id: &str, sql: &str) -> Result<Vec<RecordBatch>, Fallo> {
    let mut s = m.sesion(&Contexto::nuevo(id))?;
    let sql = sql.to_string();
    m.es().block_on(async {
        let plan = s.planificar(&sql).await?;
        m.correr(&s, plan)?.recoger().await
    })
}

fn un_entero(lotes: &[RecordBatch]) -> i64 {
    lotes[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

/// Una consulta que tiene a la CPU ocupada unos segundos.
const PESADA: &str = "select sum((v * 7919) % 1000003) from generate_series(1, 400000000) t(v)";

// ── S2·5 · la plantilla y la sesión ────────────────────────────────────────────

#[test]
fn dos_sesiones_comparten_el_entorno_y_no_la_configuracion() {
    let m = motor(Zocalos::default(), 2);
    let a = m
        .sesion(&Contexto::nuevo("a").con("datafusion.execution.batch_size", "100"))
        .unwrap();
    let b = m.sesion(&Contexto::nuevo("b")).unwrap();
    assert_eq!(a.id(), "a");
    assert_eq!(b.id(), "b");
    assert_eq!(a.estado().config().batch_size(), 100);
    assert_eq!(b.estado().config().batch_size(), m.config().batch_size());
    assert!(Arc::ptr_eq(a.estado().runtime_env(), m.entorno()));
    assert!(Arc::ptr_eq(b.estado().runtime_env(), m.entorno()));
}

#[test]
fn el_contexto_solo_cambia_lo_permitido() {
    let m = motor(Zocalos::default(), 2);
    let e = m
        .sesion(&Contexto::nuevo("x").con("datafusion.execution.target_partitions", "64"))
        .unwrap_err();
    assert!(matches!(e, Fallo::Config(_)), "{e}");
    assert!(e.to_string().contains("target_partitions"), "{e}");
}

#[test]
fn una_consulta_no_cambia_su_sesion_ni_crea_objetos() {
    let m = motor(Zocalos::default(), 2);
    for sql in [
        "set datafusion.execution.batch_size = 1",
        "create table t as select 1 as x",
    ] {
        let e = consultar(&m, "x", sql).unwrap_err();
        assert!(matches!(e, Fallo::Config(_)), "{sql}: {e}");
    }
}

/// Lo que cuesta abrir una sesión. Se corre a mano y la cifra va al ADR (0081 §7.4.3).
#[test]
#[ignore = "medida: cargo test -p ore-runtime --release -- --ignored --nocapture"]
fn mide_lo_que_cuesta_una_sesion() {
    let m = motor(Zocalos::default(), 2);
    let n = 20_000u32;
    let t = Instant::now();
    for i in 0..n {
        std::hint::black_box(m.sesion(&Contexto::nuevo(i.to_string())).unwrap());
    }
    println!("abrir una sesión: {:?}", t.elapsed() / n);
    let n = 2_000u32;
    let t = Instant::now();
    for i in 0..n {
        let mut s = m.sesion(&Contexto::nuevo(i.to_string())).unwrap();
        std::hint::black_box(m.es().block_on(s.planificar("select 1")).unwrap());
    }
    println!("abrir y planificar `select 1`: {:?}", t.elapsed() / n);
}

// ── S2·6 · correr ──────────────────────────────────────────────────────────────

#[test]
fn correr_devuelve_lo_que_la_consulta_calcula() {
    let m = motor(Zocalos::default(), 2);
    let lotes = consultar(&m, "x", "select count(*) from generate_series(1, 1000000)").unwrap();
    assert_eq!(un_entero(&lotes), 1_000_000);
}

#[test]
fn con_la_cpu_saturada_el_runtime_de_es_sigue_respondiendo() {
    let m = motor(Zocalos::default(), 2);
    // Dos consultas pesadas para dos hilos de CPU: la CPU, llena.
    let mut corriendo = Vec::new();
    for i in 0..2 {
        let mut s = m.sesion(&Contexto::nuevo(format!("pesada-{i}"))).unwrap();
        let plan = m.es().block_on(s.planificar(PESADA)).unwrap();
        corriendo.push(m.correr(&s, plan).unwrap());
    }
    std::thread::sleep(Duration::from_millis(300));
    // Un latido cada 20 ms en el runtime de E/S: lo que se retrasa cada uno.
    let peor = m.es().block_on(async {
        let mut peor = Duration::ZERO;
        for _ in 0..25 {
            let t = Instant::now();
            tokio::time::sleep(Duration::from_millis(20)).await;
            peor = peor.max(t.elapsed().saturating_sub(Duration::from_millis(20)));
        }
        peor
    });
    assert!(
        peor < Duration::from_millis(50),
        "un latido se retrasó {peor:?} con la CPU llena"
    );
    drop(corriendo);
}

#[derive(Debug, Default)]
struct ConUnaQueExplota;

impl Extensiones for ConUnaQueExplota {
    fn aplicar(&self, plantilla: SessionStateBuilder) -> SessionStateBuilder {
        let explota = create_udf(
            "explota",
            vec![DataType::Int64],
            DataType::Int64,
            // Volátil: si no, el optimizador la evaluaría al planificar, fuera de `correr`.
            Volatility::Volatile,
            Arc::new(
                |_args: &[ColumnarValue]| -> datafusion::error::Result<ColumnarValue> {
                    panic!("boom en un operador")
                },
            ),
        );
        plantilla.with_scalar_functions(vec![Arc::new(explota)])
    }
}

#[test]
fn un_panico_vuelve_como_fallo_y_el_motor_sigue() {
    let z = Zocalos {
        extensiones: Arc::new(ConUnaQueExplota),
        ..Zocalos::default()
    };
    let m = motor(z, 2);
    let e = consultar(
        &m,
        "x",
        "select explota(v) from generate_series(1, 10) t(v)",
    )
    .unwrap_err();
    match &e {
        Fallo::Panico { mensaje } => assert!(mensaje.contains("boom"), "{e}"),
        otro => panic!("tenía que ser un pánico: {otro}"),
    }
    // Y el proceso sigue: la consulta siguiente corre.
    let lotes = consultar(&m, "y", "select count(*) from generate_series(1, 10)").unwrap();
    assert_eq!(un_entero(&lotes), 10);
}

#[derive(Debug, Default)]
struct Apuntes {
    empezadas: Mutex<Vec<Ficha>>,
    terminadas: Mutex<Vec<(String, Resultado)>>,
}

impl Observacion for Apuntes {
    fn al_empezar(&self, ficha: &Ficha) {
        self.empezadas.lock().unwrap().push(ficha.clone());
    }
    fn al_terminar(&self, ficha: &Ficha, resultado: &Resultado) {
        self.terminadas
            .lock()
            .unwrap()
            .push((ficha.id.clone(), resultado.clone()));
    }
}

#[test]
fn soltar_la_consulta_la_cancela() {
    let apuntes = Arc::new(Apuntes::default());
    let z = Zocalos {
        observacion: apuntes.clone(),
        ..Zocalos::default()
    };
    let m = motor(z, 2);
    let mut s = m.sesion(&Contexto::nuevo("soltada")).unwrap();
    let plan = m.es().block_on(s.planificar(PESADA)).unwrap();
    let c = m.correr(&s, plan).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let soltada = Instant::now();
    drop(c);
    // La tarea se aborta en su `await` pendiente y la observación lo dice.
    while apuntes.terminadas.lock().unwrap().is_empty() {
        assert!(
            soltada.elapsed() < Duration::from_secs(5),
            "la consulta soltada no terminó"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let (id, r) = apuntes.terminadas.lock().unwrap()[0].clone();
    assert_eq!(id, "soltada");
    assert!(
        matches!(&r, Resultado::Fallida { mensaje } if mensaje.contains("cancel")),
        "{r:?}"
    );
    // Y la CPU queda libre: una consulta nueva corre enseguida.
    let t = Instant::now();
    consultar(&m, "despues", "select 1").unwrap();
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
}

// ── S2·7 · los zócalos y la observación ────────────────────────────────────────

#[derive(Debug, Default)]
struct UnaTabla {
    vistas: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl Catalogo for UnaTabla {
    async fn preparar(
        &self,
        _base: &SessionState,
        referencias: &[TableReference],
    ) -> Result<Arc<dyn CatalogProviderList>, Fallo> {
        self.vistas
            .lock()
            .unwrap()
            .push(referencias.iter().map(|r| r.to_string()).collect());
        let esquema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]));
        let lote = RecordBatch::try_new(
            esquema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        let schema = MemorySchemaProvider::new();
        datafusion::catalog::SchemaProvider::register_table(
            &schema,
            "t".into(),
            Arc::new(MemTable::try_new(esquema, vec![vec![lote]]).unwrap()),
        )
        .unwrap();
        let catalogo = MemoryCatalogProvider::new();
        catalogo
            .register_schema("public", Arc::new(schema))
            .unwrap();
        let lista = MemoryCatalogProviderList::new();
        lista.register_catalog("datafusion".into(), Arc::new(catalogo));
        Ok(Arc::new(lista))
    }
}

#[test]
fn el_catalogo_resuelve_cada_consulta_sin_tocar_la_plantilla() {
    let cat = Arc::new(UnaTabla::default());
    let m = motor(
        Zocalos {
            catalogo: cat.clone(),
            ..Zocalos::default()
        },
        2,
    );
    let lotes = consultar(&m, "x", "select count(*) from t").unwrap();
    assert_eq!(un_entero(&lotes), 3);
    assert_eq!(*cat.vistas.lock().unwrap(), vec![vec!["t".to_string()]]);
    // La plantilla no ganó la tabla: una sesión nueva no la ve hasta resolver la suya.
    let otra = m.sesion(&Contexto::nuevo("y")).unwrap();
    let en_la_plantilla = otra
        .estado()
        .catalog_list()
        .catalog("datafusion")
        .and_then(|c| c.schema("public"))
        .is_some_and(|s| s.table_exist("t"));
    assert!(!en_la_plantilla);
}

#[test]
fn la_observacion_recibe_la_ficha_y_el_resultado() {
    let apuntes = Arc::new(Apuntes::default());
    let m = motor(
        Zocalos {
            observacion: apuntes.clone(),
            ..Zocalos::default()
        },
        2,
    );
    consultar(&m, "vista", "select count(*) from generate_series(1, 1000)").unwrap();
    let fichas = apuntes.empezadas.lock().unwrap().clone();
    assert_eq!(fichas.len(), 1);
    let f = &fichas[0];
    assert_eq!(f.id, "vista");
    assert_eq!(f.manifiesto, Manifiesto::actual());
    assert!(
        f.config
            .iter()
            .any(|(k, v)| k == "ore.holgura_memoria" && v.as_deref() == Some("0.15")),
        "{:?}",
        f.config
    );
    assert!(!f.plan.is_empty());
    let terminadas = apuntes.terminadas.lock().unwrap().clone();
    match &terminadas[..] {
        [(id, Resultado::Hecha { filas, metricas })] => {
            assert_eq!(id, "vista");
            assert_eq!(*filas, 1);
            assert!(metricas.contains("output_rows"), "{metricas}");
        }
        otro => panic!("{otro:?}"),
    }
}

#[derive(Debug)]
struct EnMemoria(Arc<DefaultObjectStoreRegistry>);

impl Almacenes for EnMemoria {
    fn registro(&self) -> Option<Arc<dyn ObjectStoreRegistry>> {
        Some(self.0.clone())
    }
}

#[test]
fn el_registro_de_almacenes_es_el_del_zocalo() {
    let registro = Arc::new(DefaultObjectStoreRegistry::new());
    let url = url::Url::parse("memory://prueba").unwrap();
    registro.register_store(&url, Arc::new(object_store::memory::InMemory::new()));
    let m = motor(
        Zocalos {
            almacenes: Arc::new(EnMemoria(registro)),
            ..Zocalos::default()
        },
        1,
    );
    assert!(m.entorno().object_store_registry.get_store(&url).is_ok());
}
