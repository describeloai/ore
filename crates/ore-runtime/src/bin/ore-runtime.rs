//! **`ore-runtime`**, el binario del ORE Runtime (0081 §7.4.4): un binario, tres papeles.
//!
//! ```text
//! ore-runtime local [--scheduler df://host:puerto] [--tabla nombre=ruta]... "<sql>"
//! ore-runtime scheduler [--escucha 0.0.0.0:50050] [--anuncia host]
//! ore-runtime diagnostico
//! ore-runtime executor --scheduler host:puerto [--escucha 0.0.0.0] [--anuncia host] [--puerto 50051] [--puerto-grpc 50052]
//! ```
//!
//! Fino a propósito: lee los argumentos, pone el asignador, el registro y las señales, arranca el `Motor` del
//! papel y llama a la librería. Todo lo demás es de `ore_runtime`.

// DataFusion vuelve a lanzar el pánico de un operador y Ballista lo para por tarea con `catch_unwind`: con
// `abort`, un pánico tumba el proceso entero. Se compila con el perfil `runtime` (0081 D-S8).
#[cfg(panic = "abort")]
compile_error!(
    "`ore-runtime` necesita `panic = \"unwind\"`: compílalo con `--profile runtime` (0081 §7.4.4)"
);

// El `malloc` de musl tiene un candado global: sin mimalloc, la sesión cuesta el doble (0081 S0).
#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use datafusion::arrow::util::pretty::pretty_format_batches;
use datafusion::catalog::CatalogProviderList;
use datafusion::common::TableReference;
use datafusion::common::config::ExtensionOptions;
use datafusion::execution::SessionState;
use datafusion::prelude::{ParquetReadOptions, SessionContext};
use ore_runtime::papeles::{self, RedDelEjecutor};
use ore_runtime::zocalos::Catalogo;
use ore_runtime::{Contexto, Fallo, Manifiesto, Motor, OreOpciones, Papel, Zocalos};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "ore-runtime", version = Manifiesto::actual().to_string(), about = "ORE Runtime: el suelo del motor de ORE")]
struct Argumentos {
    /// Una opción de ORE, `ore.clave=valor`. Se puede repetir.
    #[arg(long = "opcion", global = true, value_name = "ore.clave=valor")]
    opciones: Vec<String>,
    /// El nivel del registro: error, warn, info, debug o trace.
    #[arg(long, global = true, default_value = "info")]
    registro: log::Level,
    #[command(subcommand)]
    papel: Orden,
}

#[derive(Subcommand)]
enum Orden {
    /// Una consulta en este proceso, o en un clúster con `--scheduler`.
    Local {
        /// El scheduler al que mandarla (`df://host:puerto`): lo mismo que `--opcion ore.scheduler=…`.
        #[arg(long)]
        scheduler: Option<String>,
        /// Una tabla Parquet, `nombre=ruta`. Se puede repetir.
        #[arg(long = "tabla", value_name = "nombre=ruta")]
        tablas: Vec<String>,
        /// La consulta.
        sql: String,
    },
    /// El scheduler de Ballista.
    Scheduler {
        #[arg(long, default_value = "0.0.0.0:50050")]
        escucha: String,
        /// El nombre con que lo encuentran los ejecutores.
        #[arg(long, default_value = "localhost")]
        anuncia: String,
    },
    /// Qué es este runtime, qué lee de su pod y lo que cuesta, medido aquí (0081 S6).
    Diagnostico,
    /// Un ejecutor de Ballista.
    Executor {
        /// El scheduler, `host:puerto`.
        #[arg(long)]
        scheduler: String,
        #[arg(long, default_value = "0.0.0.0")]
        escucha: String,
        /// El nombre con que lo encuentran el scheduler y los demás ejecutores.
        #[arg(long, default_value = "localhost")]
        anuncia: String,
        /// El puerto de Flight: por él salen los datos barajados.
        #[arg(long, default_value_t = 50051)]
        puerto: u16,
        /// El puerto de gRPC: por él le llegan las tareas.
        #[arg(long = "puerto-grpc", default_value_t = 50052)]
        puerto_grpc: u16,
    },
}

/// Las tablas Parquet de la línea de órdenes, en una lista de catálogos de cada consulta.
#[derive(Debug)]
struct Tablas(Vec<(String, String)>);

#[async_trait]
impl Catalogo for Tablas {
    async fn preparar(
        &self,
        base: &SessionState,
        _referencias: &[TableReference],
    ) -> Result<Arc<dyn CatalogProviderList>, Fallo> {
        if self.0.is_empty() {
            return Ok(base.catalog_list().clone());
        }
        let ctx = SessionContext::new();
        for (nombre, ruta) in &self.0 {
            ctx.register_parquet(nombre, ruta, ParquetReadOptions::default())
                .await?;
        }
        Ok(ctx.state().catalog_list().clone())
    }
}

fn par(texto: &str, que: &str) -> Result<(String, String), Fallo> {
    texto
        .split_once('=')
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .ok_or_else(|| Fallo::Config(format!("`{texto}`: se espera {que}")))
}

fn opciones(pares: &[String]) -> Result<OreOpciones, Fallo> {
    let mut o = OreOpciones::default();
    for p in pares {
        let (clave, valor) = par(p, "`ore.clave=valor`")?;
        let campo = clave
            .strip_prefix("ore.")
            .ok_or_else(|| Fallo::Config(format!("`{clave}` no es una opción de ORE")))?;
        o.set(campo, &valor)
            .map_err(|e| Fallo::Config(format!("`{clave}`: {e}")))?;
    }
    Ok(o)
}

/// SIGTERM (lo que manda Kubernetes al parar un pod) o Ctrl+C.
async fn senal_de_parada() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

fn local(
    o: OreOpciones,
    scheduler: Option<String>,
    tablas: Vec<String>,
    sql: String,
) -> Result<(), Fallo> {
    let o = OreOpciones {
        scheduler: scheduler.or(o.scheduler),
        ..o
    };
    let tablas = tablas
        .iter()
        .map(|t| par(t, "`nombre=ruta`"))
        .collect::<Result<_, _>>()?;
    let zocalos = Zocalos {
        catalogo: Arc::new(Tablas(tablas)),
        ..Zocalos::default()
    };
    let motor = Motor::arrancar(Papel::Local, o, &zocalos)?;
    let mut sesion = motor.sesion(&Contexto::nuevo(format!("local-{}", std::process::id())))?;
    let lotes = motor.es().block_on(async {
        let plan = sesion.planificar(&sql).await?;
        motor.correr(&sesion, plan)?.recoger().await
    })?;
    println!(
        "{}",
        pretty_format_batches(&lotes).map_err(|e| Fallo::Ejecucion(e.into()))?
    );
    Ok(())
}

fn scheduler(o: OreOpciones, escucha: String, anuncia: String) -> Result<(), Fallo> {
    let motor = Motor::arrancar(Papel::Scheduler, o, &Zocalos::default())?;
    motor.es().block_on(async {
        let escucha = tokio::net::TcpListener::bind(&escucha)
            .await
            .map_err(|e| Fallo::Config(format!("`{escucha}`: {e}")))?;
        log::info!(
            "{} · scheduler en {}",
            Manifiesto::actual(),
            escucha
                .local_addr()
                .map_err(|e| Fallo::Config(e.to_string()))?
        );
        let servicio = papeles::scheduler(&motor, escucha, &anuncia)?;
        tokio::select! {
            r = servicio => r,
            _ = senal_de_parada() => {
                log::info!("parada pedida: el scheduler termina");
                Ok(())
            }
        }
    })
}

fn executor(o: OreOpciones, red: RedDelEjecutor) -> Result<(), Fallo> {
    let motor = Motor::arrancar(Papel::Ejecutor, o, &Zocalos::default())?;
    log::info!(
        "{} · ejecutor con {} CPU, raíz de {} MiB",
        Manifiesto::actual(),
        motor.limites().cpu,
        motor.raiz_de_memoria() >> 20
    );
    // Ballista atiende él mismo SIGTERM y Ctrl+C, y termina limpio.
    motor.es().block_on(papeles::ejecutor(&motor, red)?)
}

fn diagnostico(o: OreOpciones) -> Result<(), Fallo> {
    let motor = Motor::arrancar(Papel::Local, o, &Zocalos::default())?;
    for (clave, valor) in ore_runtime::diagnostico::medir(&motor)? {
        println!("{clave} {valor}");
    }
    Ok(())
}

fn principal() -> Result<(), Fallo> {
    let a = Argumentos::parse();
    ore_runtime::registro::iniciar(a.registro);
    let o = opciones(&a.opciones)?;
    match a.papel {
        Orden::Local {
            scheduler,
            tablas,
            sql,
        } => local(o, scheduler, tablas, sql),
        Orden::Scheduler { escucha, anuncia } => scheduler(o, escucha, anuncia),
        Orden::Diagnostico => diagnostico(o),
        Orden::Executor {
            scheduler,
            escucha,
            anuncia,
            puerto,
            puerto_grpc,
        } => {
            let (host, p) = scheduler
                .rsplit_once(':')
                .ok_or_else(|| Fallo::Config(format!("`{scheduler}`: se espera `host:puerto`")))?;
            let p = p
                .parse()
                .map_err(|_| Fallo::Config(format!("`{scheduler}`: el puerto no es un número")))?;
            executor(
                o,
                RedDelEjecutor {
                    escucha,
                    anuncia,
                    puerto,
                    puerto_grpc,
                    scheduler: (host.to_string(), p),
                },
            )
        }
    }
}

fn main() -> ExitCode {
    match principal() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ore-runtime: {e}");
            ExitCode::FAILURE
        }
    }
}
