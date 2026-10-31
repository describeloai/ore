//! **La vara de ORE** (0081 §7.5.2): la definición ejecutable del contrato, ORE SQL. Cada `.slt` de
//! `tests/contrato/` corre sobre una sesión de ORE de verdad, por el mismo camino que una consulta: `Motor` →
//! `Sesion::planificar` (con la política de la sesión y el zócalo de catálogo de ORE) → `Motor::correr`. La vara
//! de DataFusion (`ore-runtime/tests/vara.rs`) prueba lo adoptado; ésta, lo que pone ORE.
//!
//! Vive aquí y no en `ore-runtime` porque necesita el lago: un lago en memoria ([`LagoEnMemoria`]) con los
//! datasets y las vistas de [`sembrar`], escritos con el mismo `Lago` que en producción, y su puerta del commit
//! (C5). Cada fichero empieza con el lago recién sembrado. La sesión trabaja en la base `ventas` y escribe.
//!
//! El formato y los tipos de columna son los del corpus de DataFusion (`convert_batches`), para que un `.slt`
//! de ORE se lea igual que uno de arriba.
//!
//! **Y repartida** (C8·4): la misma vara en un clúster dentro del proceso de la prueba (un scheduler, un ejecutor y
//! el cliente, cada uno con su `Motor` y su codec, como serían tres procesos). El plan viaja por el cable de verdad;
//! lo único compartido es el lago en memoria, que en producción es el mismo bucket.

use async_trait::async_trait;
use datafusion::arrow::array::{ArrayRef, Int64Array, StringArray};
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::error::DataFusionError;
use datafusion::execution::SessionStateBuilder;
use datafusion::logical_expr::{ColumnarValue, Volatility, create_udf};
use datafusion_sqllogictest::{DFColumnType, DFOutput, convert_batches, convert_schema_to_types};
use ore_runtime::codecs::Codecs;
use ore_runtime::papeles::{self, RedDelEjecutor};
use ore_runtime::zocalos::{Extensiones, Ficha, Observacion, Resultado};
use ore_runtime::{Contexto, Fallo, Motor, OreOpciones, Papel, Plan, Zocalos};
use ore_tabla::ejecutable;
use ore_tabla::escritura::{FalloCommit, Puerta, la_cumple_el_lago};
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::{CatalogoOre, CodecOre, Nombre};
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
        let plan = match sesion.planificar(sql).await? {
            // Lo que el lago no cumple (bases, schemas, vistas, colecciones: del árbol y sus productos), la vara lo
            // enseña planificado, en una fila de la forma canónica, y un `.slt` lo comprueba.
            Plan::Catalogo(cambio) if !la_cumple_el_lago(&cambio) => {
                return Ok(DBOutput::Rows {
                    types: vec![DFColumnType::Text],
                    rows: vec![vec![cambio.to_string()]],
                });
            }
            plan => ejecutable(&sesion, plan).await?,
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

/// Una Function del árbol, como la registrará su producto: con su nombre entero, `functions.<def>`. Y el codec
/// de ORE, para que el plan viaje (C8).
#[derive(Debug)]
struct ConUnaFunction(CodecOre);

impl Extensiones for ConUnaFunction {
    fn codecs(&self) -> Option<Codecs> {
        Some(self.0.clone().codecs())
    }

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

/// Un lote con columnas nulables: `try_from_iter` haría `required` lo que no trae nulos.
fn lote(columnas: Vec<(&str, ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter_with_nullable(columnas.into_iter().map(|(n, a)| (n, a, true)))
        .unwrap()
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

/// Cuántas sentencias del lago corrió el cliente, y cuántas repartió. Las que no tocan el lago (un plan vacío, o
/// `information_schema`, que es del catálogo del cliente) Ballista las resuelve en el cliente, y así debe ser.
#[derive(Debug, Default)]
struct Repartidas {
    corridas: AtomicU64,
    repartidas: AtomicU64,
}

impl Observacion for Repartidas {
    fn al_empezar(&self, ficha: &Ficha) {
        if ficha.plan.contains("DistributedQueryExec") {
            self.corridas.fetch_add(1, Ordering::Relaxed);
            self.repartidas.fetch_add(1, Ordering::Relaxed);
        } else if ficha.plan.contains("EscaneoOre") || ficha.plan.contains("OreSink") {
            self.corridas.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn al_terminar(&self, _: &Ficha, _: &Resultado) {}
}

/// Los zócalos de un lago recién sembrado: cada fichero empieza en el mismo sitio, porque los ficheros escriben.
fn sembrado() -> (LagoEnMemoria, Zocalos) {
    sembrado_con(|l| l.puerta())
}

/// Lo mismo, con otra puerta delante de la del lago.
fn sembrado_con(puerta: impl Fn(&LagoEnMemoria) -> Arc<dyn Puerta>) -> (LagoEnMemoria, Zocalos) {
    let lago = LagoEnMemoria::default();
    sembrar(&lago);
    let catalogo = CatalogoOre::new(lago.fuente(), lago.lago().clone()).con_puerta(puerta(&lago));
    let zocalos = Zocalos {
        extensiones: Arc::new(ConUnaFunction(catalogo.codec())),
        catalogo: Arc::new(catalogo),
        ..Zocalos::default()
    };
    (lago, zocalos)
}

/// Dónde corre la vara: un motor que ejecuta en su proceso, o el cliente de un clúster.
struct Donde {
    // El orden importa al soltar: el cliente, el ejecutor y el scheduler.
    cliente: Arc<Motor>,
    _ejecutor: Option<Motor>,
    _scheduler: Option<Motor>,
    lago: LagoEnMemoria,
}

fn en_el_proceso() -> Donde {
    let (lago, z) = sembrado();
    Donde {
        cliente: Arc::new(
            Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).expect("el motor de prueba"),
        ),
        _ejecutor: None,
        _scheduler: None,
        lago,
    }
}

fn puerto_libre() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Un clúster en el proceso: scheduler, ejecutor y cliente, con los mismos zócalos.
fn en_el_cluster() -> Donde {
    let (lago, mut z) = sembrado();
    z.observacion = REPARTIDAS.clone();
    cluster(lago, z)
}

fn cluster(lago: LagoEnMemoria, z: Zocalos) -> Donde {
    let motor =
        |papel, opciones| Motor::arrancar(papel, opciones, &z).expect("un motor del clúster");
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
    let opciones = OreOpciones {
        scheduler: Some(format!("df://127.0.0.1:{puerto}")),
        ..OreOpciones::default()
    };
    Donde {
        cliente: Arc::new(motor(Papel::Prueba, opciones)),
        _ejecutor: Some(ejecutor),
        _scheduler: Some(scheduler),
        lago,
    }
}

#[test]
fn el_contrato_se_cumple() {
    la_vara(en_el_proceso);
}

static REPARTIDAS: std::sync::LazyLock<Arc<Repartidas>> =
    std::sync::LazyLock::new(Default::default);

/// C8·4: la misma vara, con el plan repartido por Ballista: todo lo que lee o escribe el lago, por el clúster.
#[test]
fn el_contrato_se_cumple_repartido() {
    la_vara(en_el_cluster);
    let corridas = REPARTIDAS.corridas.load(Ordering::Relaxed);
    assert!(corridas > 50, "{corridas}");
    assert_eq!(REPARTIDAS.repartidas.load(Ordering::Relaxed), corridas);
}

fn la_vara(donde: fn() -> Donde) {
    let ficheros = ficheros();
    assert!(!ficheros.is_empty(), "la vara de ORE sin ficheros");
    let n = Arc::new(AtomicU64::new(0));
    let mut fallos = Vec::new();
    for f in &ficheros {
        let d = donde();
        let motor = Arc::clone(&d.cliente);
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

// ── C8·5: el commit que pierde su respuesta ─────────────────────────────────────────────────────────────────

/// **Una puerta que confirma y pierde la respuesta** las `perder` primeras veces: el commit se hizo, pero quien lo
/// pidió no lo sabe. Es una red que se corta tras aplicar, o un sink que cae después de confirmar.
#[derive(Debug)]
struct QuePierde {
    puerta: Arc<dyn Puerta>,
    perder: AtomicU64,
    llamadas: AtomicU64,
}

#[async_trait]
impl Puerta for QuePierde {
    async fn esbozar(
        &self,
        nombre: &Nombre,
        esquema: iceberg::spec::Schema,
    ) -> Result<iceberg::table::Table, String> {
        self.puerta.esbozar(nombre, esquema).await
    }

    async fn confirmar(
        &self,
        nombre: &Nombre,
        tabla: &iceberg::table::Table,
        requisitos: Vec<iceberg::TableRequirement>,
        cambios: Vec<iceberg::TableUpdate>,
        clave: &str,
    ) -> Result<String, FalloCommit> {
        self.llamadas.fetch_add(1, Ordering::Relaxed);
        let ml = self
            .puerta
            .confirmar(nombre, tabla, requisitos, cambios, clave)
            .await?;
        if self
            .perder
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(FalloCommit::SinRespuesta(
                "se cortó después de aplicar".into(),
            ));
        }
        Ok(ml)
    }

    async fn retirar(
        &self,
        nombre: &Nombre,
        esperado: &str,
        clave: &str,
    ) -> Result<(), FalloCommit> {
        self.puerta.retirar(nombre, esperado, clave).await
    }
}

/// Una sentencia, por el mismo camino que la vara.
fn sentencia(d: &Donde, sql: &str) -> Result<Vec<RecordBatch>, Fallo> {
    let m = &d.cliente;
    let mut s = m.sesion(&Contexto::nuevo("c85").en_base(BASE).que_escribe())?;
    m.es().block_on(async {
        let plan = s.planificar(sql).await?;
        let plan = ejecutable(&s, plan).await?;
        m.correr(&s, plan)?.recoger().await
    })
}

fn cuantos(d: &Donde) -> i64 {
    let l = sentencia(d, "select count(*) from pedidos").unwrap();
    l[0].column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

/// C8·5: repartida, un commit que pierde su respuesta no falla ni duplica. Ballista reintenta la tarea del sink,
/// que vuelve a escribir sus ficheros (huérfanos, que se barren) y confirma con **la misma clave**: la puerta
/// contesta lo que ya hizo. Un snapshot por sentencia.
#[test]
fn un_commit_sin_respuesta_no_duplica_repartido() {
    let puerta: Arc<std::sync::Mutex<Option<Arc<QuePierde>>>> = Default::default();
    let p = puerta.clone();
    let (lago, z) = sembrado_con(move |l| {
        let q = Arc::new(QuePierde {
            puerta: l.puerta(),
            perder: AtomicU64::new(0),
            llamadas: AtomicU64::new(0),
        });
        *p.lock().unwrap() = Some(q.clone());
        q
    });
    let d = cluster(lago, z);
    let q = puerta.lock().unwrap().clone().unwrap();
    for (sql, filas) in [
        ("insert into pedidos values (9, 'eva', 90)", 4),
        ("delete from pedidos where id = 1", 3),
        (
            "merge into pedidos p using (values (2, 'luis', 1), (8, 'ana', 8)) f(id, cliente, importe) \
             on p.id = f.id when matched then update set importe = f.importe \
             when not matched then insert values (f.id, f.cliente, f.importe)",
            4,
        ),
    ] {
        let (commits, llamadas) = (d.lago.commits(), q.llamadas.load(Ordering::Relaxed));
        q.perder.store(1, Ordering::Relaxed);
        sentencia(&d, sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
        assert_eq!(d.lago.commits(), commits + 1, "{sql}: un commit");
        assert!(
            q.llamadas.load(Ordering::Relaxed) >= llamadas + 2,
            "{sql}: Ballista no reintentó"
        );
        assert_eq!(cuantos(&d), filas, "{sql}");
    }
}

/// En el proceso no hay quien reintente la tarea: la sentencia falla diciendo que no se sabe, y el commit está hecho
/// una vez. Repetir la sentencia es otra sentencia, con otra clave: eso es del cliente.
#[test]
fn un_commit_sin_respuesta_en_el_proceso_lo_dice() {
    let (lago, mut z) = sembrado();
    let q = Arc::new(QuePierde {
        puerta: lago.puerta(),
        perder: AtomicU64::new(1),
        llamadas: AtomicU64::new(0),
    });
    let catalogo = CatalogoOre::new(lago.fuente(), lago.lago().clone()).con_puerta(q.clone());
    z.catalogo = Arc::new(catalogo);
    let d = Donde {
        cliente: Arc::new(Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).unwrap()),
        _ejecutor: None,
        _scheduler: None,
        lago,
    };
    let e = sentencia(&d, "insert into pedidos values (9, 'eva', 90)").unwrap_err();
    assert!(
        e.to_string().contains("no se sabe si el commit se publicó"),
        "{e}"
    );
    assert_eq!(d.lago.commits(), 1);
    assert_eq!(cuantos(&d), 4);
}
