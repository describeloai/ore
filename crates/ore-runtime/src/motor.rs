//! **El motor del proceso** (0081 §7.4, propiedades 1, 6 y 7): uno por proceso, vive lo que el proceso.
//!
//! - El `RuntimeEnv`: la raíz de memoria (del zócalo, sobre el límite menos la holgura) y el `DiskManager`
//!   en el directorio de derrame con su tope.
//! - **Los dos runtimes de tokio**: el de E/S (pocos hilos: la red, la salud) y el de CPU (un hilo por CPU
//!   del proceso, sin E/S). La CPU nunca ahoga a la E/S ni a la salud (el ejemplo `thread_pools` de
//!   DataFusion; InfluxData).
//! - La configuración del papel ([`crate::opciones::config_de`]) y **la plantilla** de sesión, construida una
//!   vez con las extensiones del zócalo: cada consulta la clona ([`Motor::sesion`]).
//! - [`Motor::correr`]: la única entrada para ejecutar ([`crate::correr`]).

use crate::correr::{self, Consulta};
use crate::fallo::Fallo;
use crate::limites::Limites;
use crate::manifiesto::Manifiesto;
use crate::opciones::{self, OreOpciones, Papel};
use crate::sesion::{Contexto, Sesion};
use crate::zocalos::Zocalos;
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
    limites: Limites,
    config: SessionConfig,
    entorno: Arc<RuntimeEnv>,
    raiz: usize,
    derrame: Option<PathBuf>,
    plantilla: SessionState,
    zocalos: Zocalos,
    // `Option` para poder soltarlos con `shutdown_background` al caer el motor: soltar un `Runtime` desde
    // dentro de otro hace pánico.
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

        let plantilla = zocalos
            .extensiones
            .aplicar(
                SessionStateBuilder::new()
                    .with_config(config.clone())
                    .with_runtime_env(entorno.clone())
                    .with_default_features(),
            )
            .build();

        let es = Builder::new_multi_thread()
            .worker_threads(HILOS_DE_ES)
            .thread_name("ore-es")
            .enable_all()
            .build()
            .map_err(|e| Fallo::Config(format!("el runtime de E/S: {e}")))?;
        let cpu = Builder::new_multi_thread()
            .worker_threads(limites.cpu)
            .thread_name("ore-cpu")
            .enable_time()
            .build()
            .map_err(|e| Fallo::Config(format!("el runtime de CPU: {e}")))?;

        Ok(Motor {
            papel,
            limites,
            config,
            entorno,
            raiz,
            derrame,
            plantilla,
            zocalos: zocalos.clone(),
            es: Some(es),
            cpu: Some(cpu),
        })
    }

    /// El papel de este proceso.
    pub fn papel(&self) -> Papel {
        self.papel
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

    /// El runtime de CPU: lo que DataFusion ejecuta.
    pub fn cpu(&self) -> &Handle {
        self.cpu
            .as_ref()
            .expect("vivo mientras vive el motor")
            .handle()
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
    pub fn correr(&self, sesion: &Sesion, plan: LogicalPlan) -> Consulta {
        correr::lanzar(self.cpu(), sesion, plan, self.zocalos.observacion.clone())
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
    use datafusion::execution::SessionStateBuilder;
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
        m.cpu().block_on(async move {
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
        assert_eq!(m.cpu().metrics().num_workers(), 3);
        assert_eq!(m.es().metrics().num_workers(), HILOS_DE_ES);
        assert_eq!(m.papel(), Papel::Local);
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
