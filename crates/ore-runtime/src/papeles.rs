//! **Los papeles distribuidos** (0081 §7.4.4): el scheduler y el ejecutor de Ballista, arrancados como
//! librería desde el [`Motor`] del proceso.
//!
//! Ballista pide estas cosas para construir sesiones y mover planes, y todas salen del motor; nadie más las da:
//!
//! | gancho | sale de |
//! |---|---|
//! | `ConfigProducer` (scheduler y ejecutor) | la configuración del papel ([`crate::opciones::config_de`]): `ore.*` y `ballista.*` registradas, o Ballista las perdería en silencio |
//! | `SessionBuilder` (scheduler) | la plantilla del motor con la configuración que llega del cliente |
//! | `RuntimeProducer` (ejecutor) | **el** `RuntimeEnv` del proceso: la raíz de memoria del zócalo, sin el reparto por tarea de Ballista (0081 D-S6) |
//! | `BallistaFunctionRegistry` (ejecutor) | las funciones de la plantilla: el registro por defecto sólo trae las de DataFusion |
//! | los codecs lógico y físico (los tres papeles) | los del zócalo de extensiones, envueltos con los de Ballista ([`crate::codecs`], C8) |
//!
//! El ejecutor corre las tareas en su propio pool de CPU, a `limites.cpu` hilos: es el runtime de CPU del
//! suelo en ese papel (0081 D-S5). Las dos funciones devuelven un futuro que hay que correr en el runtime de
//! E/S del motor: Ballista lanza ahí sus servicios.

use crate::fallo::Fallo;
use crate::motor::Motor;
use crate::opciones::Papel;
use ballista_core::registry::BallistaFunctionRegistry;
use ballista_core::{ConfigProducer, RuntimeProducer};
use ballista_executor::executor_process::{ExecutorProcessConfig, start_executor_process};
use ballista_scheduler::cluster::BallistaCluster;
use ballista_scheduler::config::SchedulerConfig;
use ballista_scheduler::scheduler_process::{create_scheduler, start_grpc_service_with_listener};
use ballista_scheduler::scheduler_server::SessionBuilder;
use datafusion::execution::SessionStateBuilder;
use datafusion::prelude::SessionConfig;
use datafusion_proto::protobuf::{LogicalPlanNode, PhysicalPlanNode};
use std::future::Future;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Lo que un ejecutor espera del scheduler antes de rendirse: el scheduler puede arrancar después que él
/// (dos contenedores a la vez).
pub const ESPERA_AL_SCHEDULER_S: u16 = 30;

/// Dónde escucha un ejecutor, y cómo lo encuentran.
#[derive(Debug, Clone)]
pub struct RedDelEjecutor {
    /// La dirección donde escucha (`0.0.0.0` en un contenedor).
    pub escucha: String,
    /// El nombre con que lo encuentran el scheduler y los demás ejecutores.
    pub anuncia: String,
    /// El puerto de Flight: por él salen los datos barajados.
    pub puerto: u16,
    /// El puerto de gRPC: por él le llegan las tareas.
    pub puerto_grpc: u16,
    /// El scheduler: `host` y puerto.
    pub scheduler: (String, u16),
}

/// La configuración del papel, para quien construye sesiones en Ballista.
fn config_producer(motor: &Motor) -> ConfigProducer {
    let config = motor.config().clone();
    Arc::new(move || config.clone())
}

fn exige(motor: &Motor, papel: Papel) -> Result<(), Fallo> {
    if motor.papel() == papel {
        Ok(())
    } else {
        Err(Fallo::Config(format!(
            "el motor es de papel {:?}, no {papel:?}",
            motor.papel()
        )))
    }
}

/// **El scheduler**, sirviendo en `escucha` hasta que el futuro se suelte. `anuncia` es el nombre con que
/// lo encuentran.
pub fn scheduler(
    motor: &Motor,
    escucha: TcpListener,
    anuncia: &str,
) -> Result<impl Future<Output = Result<(), Fallo>> + Send + 'static, Fallo> {
    exige(motor, Papel::Scheduler)?;
    let puerto = escucha
        .local_addr()
        .map_err(|e| Fallo::Config(e.to_string()))?
        .port();
    let plantilla = motor.plantilla().clone();
    // La plantilla con la configuración que llega: la del papel más lo que mande el cliente. Sin volver a
    // crear el catálogo por defecto: la lista de catálogos es la de la plantilla, compartida, y crearlo lo
    // pisaría en cada sesión (`new_from_existing` lo apaga por eso; `with_config` lo vuelve a encender).
    let sesiones: SessionBuilder = Arc::new(move |config: SessionConfig| {
        Ok(SessionStateBuilder::new_from_existing(plantilla.clone())
            .with_config(config.with_create_default_catalog_and_schema(false))
            .build())
    });
    let mut config = SchedulerConfig::default()
        .with_hostname(anuncia)
        .with_port(puerto)
        .with_override_config_producer(config_producer(motor))
        .with_override_session_builder(sesiones);
    let (logico, fisico) = motor.codecs();
    config.override_logical_codec = Some(logico);
    config.override_physical_codec = Some(fisico);
    Ok(async move {
        let config = Arc::new(config);
        let cluster = BallistaCluster::new_from_config(&config).await?;
        let servidor =
            create_scheduler::<LogicalPlanNode, PhysicalPlanNode>(cluster, config).await?;
        start_grpc_service_with_listener(escucha, servidor).await?;
        Ok(())
    })
}

/// **Un ejecutor**, hasta SIGTERM, Ctrl+C o que el scheduler lo pare.
pub fn ejecutor(
    motor: &Motor,
    red: RedDelEjecutor,
) -> Result<impl Future<Output = Result<(), Fallo>> + Send + 'static, Fallo> {
    exige(motor, Papel::Ejecutor)?;
    let entorno = motor.entorno().clone();
    let runtime: RuntimeProducer = Arc::new(move |_| Ok(entorno.clone()));
    // Lo barajado, junto al derrame: el mismo disco del pod, con su tope. Sin derrame, el temporal de Ballista.
    let barajado = match motor.derrame() {
        Some(d) => {
            let dir = d.join("barajado");
            std::fs::create_dir_all(&dir)
                .map_err(|e| Fallo::Config(format!("`{}`: {e}", dir.display())))?;
            Some(dir.display().to_string())
        }
        None => None,
    };
    let config = ExecutorProcessConfig {
        bind_host: red.escucha,
        external_host: Some(red.anuncia),
        port: red.puerto,
        grpc_port: red.puerto_grpc,
        scheduler_host: red.scheduler.0,
        scheduler_port: red.scheduler.1,
        scheduler_connect_timeout_seconds: ESPERA_AL_SCHEDULER_S,
        vcores: motor.limites().cpu,
        work_dir: barajado,
        // `Some(0)`: Ballista deja el `RuntimeEnv` tal cual, con la raíz del zócalo (0081 D-S6).
        memory_pool_size: Some(0),
        override_config_producer: Some(config_producer(motor)),
        override_runtime_producer: Some(runtime),
        override_function_registry: Some(Arc::new(BallistaFunctionRegistry::from(
            motor.plantilla(),
        ))),
        override_logical_codec: Some(motor.codecs().0),
        override_physical_codec: Some(motor.codecs().1),
        ..ExecutorProcessConfig::default()
    };
    Ok(async move {
        start_executor_process(Arc::new(config)).await?;
        Ok(())
    })
}
