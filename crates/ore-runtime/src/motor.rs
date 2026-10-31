//! **El motor del proceso** (0081 §7.4, propiedades 1, 6 y 7): uno por proceso, vive lo que el proceso.
//!
//! - El `RuntimeEnv`: la raíz de memoria (del zócalo, sobre el límite menos la holgura) y el `DiskManager`
//!   en el directorio de derrame con su tope.
//! - **Los dos runtimes de tokio**: el de E/S (pocos hilos: la red, la salud) y el de CPU (un hilo por CPU
//!   del proceso, sin E/S). La CPU nunca ahoga a la E/S ni a la salud (el ejemplo `thread_pools` de
//!   DataFusion; InfluxData). **El de CPU es del papel** (0081 D-S5): local y prueba lo levantan; el
//!   ejecutor usa el pool de CPU de Ballista, a `limites.cpu` hilos, y el scheduler sólo planifica.
//! - La configuración del papel ([`crate::opciones::config_de`]) y **la plantilla** de sesión, construida una
//!   vez con las extensiones del zócalo: cada consulta la clona ([`Motor::sesion`]). Si el papel distribuye,
//!   la plantilla lleva lo de Ballista; el planificador distribuido, sólo el cliente.
//! - [`Motor::correr`]: la única entrada para ejecutar ([`crate::correr`]).

use crate::contrato::TiposDeOre;
use crate::correr::{self, Consulta};
use crate::fallo::Fallo;
use crate::limites::Limites;
use crate::manifiesto::Manifiesto;
use crate::opciones::{self, OreOpciones, Papel, distribuida};
use crate::sesion::{Contexto, Sesion};
use crate::zocalos::Zocalos;
use ballista_core::extension::{
    SessionStateExt, ballista_aggregate_functions, ballista_scalar_functions,
    ballista_window_functions,
};
use ballista_core::optimizer::with_ballista_optimizer_rules;
use datafusion::execution::SessionStateBuilder;
use datafusion::execution::disk_manager::{DiskManagerBuilder, DiskManagerMode};
use datafusion::execution::runtime_env::{RuntimeEnv, RuntimeEnvBuilder};
use datafusion::execution::session_state::SessionState;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::{Builder, Handle, Runtime};

/// Los hilos del runtime de E/S: la red espera, no calcula.
pub const HILOS_DE_ES: usize = 2;

/// La parte del disco libre que puede ocupar el derrame cuando no se da un tope.
pub const PARTE_DEL_DISCO: f64 = 0.9;

/// El motor del proceso.
#[derive(Debug)]
pub struct Motor {
    papel: Papel,
    opciones: OreOpciones,
    limites: Limites,
    config: SessionConfig,
    entorno: Arc<RuntimeEnv>,
    raiz: usize,
    derrame: Option<PathBuf>,
    plantilla: SessionState,
    zocalos: Zocalos,
    // `Option` para poder soltarlos con `shutdown_background` al caer el motor: soltar un `Runtime` desde
    // dentro de otro hace pánico. El de CPU, además, sólo lo tienen los papeles que ejecutan aquí.
    es: Option<Runtime>,
    cpu: Option<Runtime>,
}

impl Motor {
    /// Arranca el motor de este proceso: lee sus límites y construye lo demás.
    pub fn arrancar(
        papel: Papel,
        opciones: OreOpciones,
        zocalos: &Zocalos,
    ) -> Result<Motor, Fallo> {
        let derrame = opciones.derrame_dir.as_ref().map(PathBuf::from);
        let limites = Limites::leer(derrame.as_deref());
        Motor::arrancar_con(papel, opciones, zocalos, limites)
    }

    /// Arranca el motor con unos límites dados (las pruebas, y quien ya los leyó).
    pub fn arrancar_con(
        papel: Papel,
        opciones: OreOpciones,
        zocalos: &Zocalos,
        limites: Limites,
    ) -> Result<Motor, Fallo> {
        let config = opciones::config_de(papel, &limites, &opciones)?;

        let raiz = ((limites.memoria as f64) * (1.0 - opciones.holgura_memoria)) as usize;
        let derrame = opciones.derrame_dir.as_ref().map(PathBuf::from);
        let mut disco = DiskManagerBuilder::default().with_mode(match &derrame {
            Some(dir) => {
                std::fs::create_dir_all(dir).map_err(|e| {
                    Fallo::Config(format!("`ore.derrame_dir` = `{}`: {e}", dir.display()))
                })?;
                DiskManagerMode::Directories(vec![dir.clone()])
            }
            None => DiskManagerMode::OsTmpDirectory,
        });
        let tope = opciones
            .derrame_tope
            .or_else(|| limites.disco.map(|d| (d as f64 * PARTE_DEL_DISCO) as u64));
        if let Some(t) = tope {
            disco = disco.with_max_temp_directory_size(t);
        }
        let mut entorno = RuntimeEnvBuilder::new()
            .with_memory_pool(zocalos.memoria.raiz(raiz))
            .with_disk_manager_builder(disco);
        if let Some(registro) = zocalos.almacenes.registro() {
            entorno = entorno.with_object_store_registry(registro);
        }
        let entorno = entorno.build_arc()?;

        let base = SessionStateBuilder::new()
            .with_config(config.clone())
            .with_runtime_env(entorno.clone())
            .with_default_features()
            .with_type_planner(Arc::new(TiposDeOre))
            .build();
        let base = distribuir(base, papel, &opciones)?;
        // Las extensiones van las últimas: lo de Ballista **reemplaza** la lista de funciones, y una función de
        // ORE puesta antes se perdería.
        let plantilla = zocalos
            .extensiones
            .aplicar(SessionStateBuilder::new_from_existing(base))
            .build();

        let es = Builder::new_multi_thread()
            .worker_threads(HILOS_DE_ES)
            .thread_name("ore-es")
            .enable_all()
            .build()
            .map_err(|e| Fallo::Config(format!("el runtime de E/S: {e}")))?;
        let cpu = match papel {
            Papel::Local | Papel::Prueba => Some(
                Builder::new_multi_thread()
                    .worker_threads(limites.cpu)
                    .thread_name("ore-cpu")
                    .enable_time()
                    .build()
                    .map_err(|e| Fallo::Config(format!("el runtime de CPU: {e}")))?,
            ),
            Papel::Scheduler | Papel::Ejecutor => None,
        };

        Ok(Motor {
            papel,
            opciones,
            limites,
            config,
            entorno,
            raiz,
            derrame,
            plantilla,
            zocalos: zocalos.clone(),
            es: Some(es),
            cpu,
        })
    }

    /// El papel de este proceso.
    pub fn papel(&self) -> Papel {
        self.papel
    }

    /// Las opciones con que arrancó.
    pub fn opciones(&self) -> &OreOpciones {
        &self.opciones
    }

    /// Si las consultas de este proceso son distribuidas.
    pub fn distribuido(&self) -> bool {
        distribuida(self.papel, &self.opciones)
    }

    /// Lo que el proceso tiene.
    pub fn limites(&self) -> &Limites {
        &self.limites
    }

    /// La configuración del papel, de la que nace cada sesión.
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// El `RuntimeEnv` del proceso: lo comparten todas sus sesiones.
    pub fn entorno(&self) -> &Arc<RuntimeEnv> {
        &self.entorno
    }

    /// Los bytes de la raíz de memoria: el límite menos la holgura.
    pub fn raiz_de_memoria(&self) -> usize {
        self.raiz
    }

    /// El directorio de derrame, si se dio uno.
    pub fn derrame(&self) -> Option<&std::path::Path> {
        self.derrame.as_deref()
    }

    /// El runtime de E/S: la red y la salud.
    pub fn es(&self) -> &Handle {
        self.es
            .as_ref()
            .expect("vivo mientras vive el motor")
            .handle()
    }

    /// El runtime de CPU: lo que DataFusion ejecuta. Sólo en los papeles que ejecutan en este proceso: el
    /// ejecutor usa el de Ballista y el scheduler no ejecuta (0081 D-S5).
    pub fn cpu(&self) -> Option<&Handle> {
        self.cpu.as_ref().map(Runtime::handle)
    }

    /// La plantilla de sesión: de ella nacen las sesiones de este proceso, y los ganchos de Ballista.
    pub(crate) fn plantilla(&self) -> &SessionState {
        &self.plantilla
    }

    /// Qué runtime es.
    pub fn manifiesto(&self) -> Manifiesto {
        Manifiesto::actual()
    }

    /// **La sesión de una consulta**: la plantilla clonada con su contexto ([`crate::sesion`]).
    pub fn sesion(&self, contexto: &Contexto) -> Result<Sesion, Fallo> {
        Sesion::de(&self.plantilla, contexto, self.zocalos.catalogo.clone())
    }

    /// **Correr un plan**: en el runtime de CPU, leído lote a lote desde cualquier otro ([`crate::correr`]).
    ///
    /// Con la consulta distribuida, en el de E/S: en el cliente, el plan sólo espera y reenvía los lotes que
    /// calculan los ejecutores, y eso es red, que el runtime de CPU no tiene.
    pub fn correr(&self, sesion: &Sesion, plan: LogicalPlan) -> Result<Consulta, Fallo> {
        // El cliente de una consulta distribuida, en el de E/S; una local, en el de CPU. El scheduler y el
        // ejecutor no corren consultas por aquí: el ejecutor corre tareas de Ballista en su pool.
        let (runtime, scheduler) = match (self.papel, &self.opciones.scheduler) {
            (Papel::Local | Papel::Prueba, Some(url)) => (self.es(), Some(url.clone())),
            (Papel::Local | Papel::Prueba, None) => (
                self.cpu().expect("local y prueba tienen runtime de CPU"),
                None,
            ),
            (Papel::Scheduler | Papel::Ejecutor, _) => {
                return Err(Fallo::Config(format!(
                    "el papel {:?} no ejecuta consultas en su proceso",
                    self.papel
                )));
            }
        };
        Ok(correr::lanzar(
            runtime,
            sesion,
            plan,
            self.zocalos.observacion.clone(),
            scheduler,
        ))
    }
}

/// **Lo de Ballista en la plantilla**, según el papel:
///
/// - el **cliente** (local o prueba con `ore.scheduler`): `upgrade_for_ballista`, que pone su planificador
///   distribuido, sus reglas y sus funciones;
/// - el **scheduler** y el **ejecutor**: las mismas reglas y funciones que su `default_session_builder`, y
///   **nunca** el planificador distribuido. Al scheduler le llega `ore.scheduler` con la configuración de cada
///   cliente; si su plantilla planificara en distribuido, se mandaría las consultas a sí mismo;
/// - una consulta local, nada.
fn distribuir(
    base: SessionState,
    papel: Papel,
    opciones: &OreOpciones,
) -> Result<SessionState, Fallo> {
    match (papel, &opciones.scheduler) {
        (Papel::Scheduler | Papel::Ejecutor, _) => {
            let reglas = with_ballista_optimizer_rules(base.optimizers());
            Ok(SessionStateBuilder::new_from_existing(base)
                .with_optimizer_rules(reglas)
                .with_scalar_functions(ballista_scalar_functions())
                .with_aggregate_functions(ballista_aggregate_functions())
                .with_window_functions(ballista_window_functions())
                .build())
        }
        (Papel::Local | Papel::Prueba, Some(url)) => Ok(base.upgrade_for_ballista(url.clone())?),
        (Papel::Local | Papel::Prueba, None) => Ok(base),
    }
}

impl Drop for Motor {
    fn drop(&mut self) {
        for r in [self.es.take(), self.cpu.take()].into_iter().flatten() {
            r.shutdown_background();
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::limites::Fuente;
    use datafusion::prelude::SessionContext;

    fn limites(memoria: u64, cpu: usize) -> Limites {
        Limites {
            memoria,
            memoria_de: Fuente::Host,
            cpu,
            cpu_de: Fuente::Host,
            disco: None,
        }
    }

    /// Una sesión directa sobre el motor. La de verdad, con su plantilla, es S2·5.
    fn contexto(m: &Motor) -> SessionContext {
        SessionContext::new_with_state(
            SessionStateBuilder::new()
                .with_config(m.config().clone())
                .with_runtime_env(m.entorno().clone())
                .with_default_features()
                .build(),
        )
    }

    /// Corre `sql` en el runtime de CPU del motor y devuelve el texto de lo que sale. `correr`, con su canal,
    /// es S2·6.
    fn en_la_cpu(m: &Motor, sql: &str) -> Result<String, datafusion::error::DataFusionError> {
        let ctx = contexto(m);
        let sql = sql.to_string();
        m.cpu().unwrap().block_on(async move {
            let lotes = ctx.sql(&sql).await?.collect().await?;
            Ok(datafusion::arrow::util::pretty::pretty_format_batches(&lotes)?.to_string())
        })
    }

    #[test]
    fn el_motor_nace_de_sus_limites() {
        let m = Motor::arrancar_con(
            Papel::Local,
            OreOpciones::default(),
            &Zocalos::default(),
            limites(1 << 30, 3),
        )
        .unwrap();
        assert_eq!(m.config().target_partitions(), 3);
        assert_eq!(m.raiz_de_memoria(), ((1u64 << 30) as f64 * 0.85) as usize);
        assert_eq!(m.cpu().unwrap().metrics().num_workers(), 3);
        assert_eq!(m.es().metrics().num_workers(), HILOS_DE_ES);
        assert_eq!(m.papel(), Papel::Local);
    }

    #[test]
    fn la_cpu_es_del_papel() {
        let m = |p| {
            Motor::arrancar_con(
                p,
                OreOpciones::default(),
                &Zocalos::default(),
                limites(1 << 30, 2),
            )
            .unwrap()
        };
        assert!(m(Papel::Local).cpu().is_some());
        assert!(m(Papel::Prueba).cpu().is_some());
        for p in [Papel::Scheduler, Papel::Ejecutor] {
            let m = m(p);
            assert!(m.cpu().is_none(), "{p:?}");
            let s = m.sesion(&Contexto::nuevo("q")).unwrap();
            let plan = m
                .es()
                .block_on(async { s.clone().planificar("select 1").await?.logico() })
                .unwrap();
            let e = m.correr(&s, plan).unwrap_err();
            assert!(matches!(e, Fallo::Config(_)), "{p:?}: {e}");
        }
    }

    /// El planificador distribuido sólo lo lleva el cliente: el scheduler que lo llevara se mandaría las
    /// consultas a sí mismo.
    #[test]
    fn solo_el_cliente_planifica_en_distribuido() {
        let planificador = |p, scheduler: Option<&str>| {
            let o = OreOpciones {
                scheduler: scheduler.map(String::from),
                ..OreOpciones::default()
            };
            let m = Motor::arrancar_con(p, o, &Zocalos::default(), limites(1 << 30, 1)).unwrap();
            format!("{:?}", m.plantilla().query_planner())
        };
        let url = Some("df://scheduler:50050");
        assert!(planificador(Papel::Local, url).contains("Ballista"));
        assert!(!planificador(Papel::Local, None).contains("Ballista"));
        assert!(!planificador(Papel::Scheduler, url).contains("Ballista"));
        assert!(!planificador(Papel::Ejecutor, url).contains("Ballista"));
    }

    #[test]
    fn agotar_la_raiz_falla_nombrando_a_sus_consumidores() {
        let o = OreOpciones {
            derrame_dir: None,
            ..OreOpciones::default()
        };
        let m = Motor::arrancar_con(Papel::Local, o, &Zocalos::default(), limites(32 << 20, 2))
            .unwrap();
        // `array_agg` no derrama (0081 §6.6): con 27 MB de raíz, 20 M de enteros no caben.
        let e = en_la_cpu(
            &m,
            "select array_agg(v) from generate_series(1, 20000000) t(v)",
        )
        .unwrap_err();
        let texto = e.to_string();
        assert!(texto.contains("Resources exhausted"), "{texto}");
        assert!(
            texto.to_lowercase().contains("consumers"),
            "el error nombra a sus consumidores: {texto}"
        );
    }

    #[test]
    fn un_sort_grande_derrama_en_el_directorio_dado() {
        let dir = tempfile::tempdir().unwrap();
        let derrame = dir.path().join("derrame");
        let o = OreOpciones {
            derrame_dir: Some(derrame.display().to_string()),
            ..OreOpciones::default()
        };
        let m = Motor::arrancar_con(Papel::Local, o, &Zocalos::default(), limites(64 << 20, 2))
            .unwrap();
        assert!(derrame.is_dir(), "el motor crea el directorio de derrame");
        assert_eq!(m.derrame(), Some(derrame.as_path()));
        let plan = en_la_cpu(
            &m,
            "explain analyze select v from generate_series(1, 6000000) t(v) order by random()",
        )
        .unwrap();
        let derramado = plan
            .split("spill_count=")
            .skip(1)
            .filter_map(|s| {
                s.split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .sum::<u64>();
        assert!(derramado > 0, "el sort tenía que derramar:\n{plan}");
    }

    #[test]
    fn el_derrame_respeta_su_tope() {
        let dir = tempfile::tempdir().unwrap();
        let o = OreOpciones {
            derrame_dir: Some(dir.path().display().to_string()),
            derrame_tope: Some(1 << 20),
            ..OreOpciones::default()
        };
        let m = Motor::arrancar_con(Papel::Local, o, &Zocalos::default(), limites(64 << 20, 2))
            .unwrap();
        let e = en_la_cpu(
            &m,
            "select v from generate_series(1, 6000000) t(v) order by random()",
        )
        .unwrap_err();
        assert!(
            e.to_string().to_lowercase().contains("temp"),
            "el tope de 1 MiB se dice: {e}"
        );
    }

    #[test]
    fn un_directorio_de_derrame_que_no_se_puede_crear_es_un_error() {
        let fichero = tempfile::NamedTempFile::new().unwrap();
        let o = OreOpciones {
            derrame_dir: Some(fichero.path().join("dentro").display().to_string()),
            ..OreOpciones::default()
        };
        let e = Motor::arrancar_con(Papel::Local, o, &Zocalos::default(), limites(64 << 20, 1))
            .unwrap_err();
        assert!(matches!(e, Fallo::Config(_)), "{e}");
    }

    #[test]
    fn soltar_el_motor_desde_dentro_de_otro_runtime_no_hace_panico() {
        let fuera = Builder::new_current_thread().build().unwrap();
        fuera.block_on(async {
            let m = Motor::arrancar_con(
                Papel::Prueba,
                OreOpciones::default(),
                &Zocalos::default(),
                limites(1 << 30, 1),
            )
            .unwrap();
            drop(m);
        });
    }
}
