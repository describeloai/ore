//! **Ejecutar sin ahogar ni caer** (0081 §7.4, propiedad 6). `Motor::correr` es la única entrada para
//! ejecutar un plan, y un solo mecanismo da tres propiedades:
//!
//! 1. **La CPU nunca ahoga a la E/S ni a la salud**: el plan corre en el runtime de CPU, y quien lo pide lo
//!    lee desde el runtime que quiera, por un canal acotado.
//! 2. **Un pánico llega como [`Fallo::Panico`] y el proceso sigue**: la tarea que corre el plan es la
//!    frontera. DataFusion vuelve a lanzar los pánicos de sus operadores (0081 S0 C7) y tokio los para en
//!    la tarea. Exige `panic = "unwind"`, que el perfil del binario pone en S3.
//! 3. **Soltar la [`Consulta`] la cancela**: aborta la tarea, que suelta el stream, y DataFusion aborta sus
//!    subtareas al soltarse (`SpawnedTask`). En una consulta distribuida eso no basta: Ballista 55 no cancela
//!    el trabajo al soltar su stream, y el clúster lo seguiría calculando (medido en S3·3). Así que la tarea
//!    le pide al scheduler que lo cancele ([`CancelaEnElCluster`]).
//!
//! El precio es un canal por consulta, despreciable frente a la E/S.

use crate::fallo::Fallo;
use crate::manifiesto::Manifiesto;
use crate::opciones::entradas;
use crate::sesion::Sesion;
use crate::zocalos::{Ficha, Observacion, Resultado};
use ballista_core::execution_plans::DistributedQueryExec;
use ballista_core::serde::protobuf::CancelJobParams;
use ballista_core::serde::protobuf::scheduler_grpc_client::SchedulerGrpcClient;
use datafusion::arrow::array::RecordBatch;
use datafusion::logical_expr::LogicalPlan;
use datafusion::physical_plan::{ExecutionPlan, displayable, execute_stream};
use datafusion_proto::protobuf::LogicalPlanNode;
use futures::StreamExt;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Los lotes que una consulta puede tener hechos y sin leer: lo que se adelanta, sin juntar el resultado.
pub const LOTES_EN_VUELO: usize = 2;

/// Una consulta en marcha. Se lee lote a lote desde cualquier runtime; soltarla la cancela.
#[derive(Debug)]
pub struct Consulta {
    lotes: mpsc::Receiver<Result<RecordBatch, Fallo>>,
    tarea: Option<JoinHandle<()>>,
}

impl Consulta {
    /// El lote siguiente, o `None` cuando terminó. Un fallo, o un pánico, llega como el último.
    pub async fn siguiente(&mut self) -> Option<Result<RecordBatch, Fallo>> {
        if let Some(l) = self.lotes.recv().await {
            return Some(l);
        }
        // El canal se cerró: o terminó bien, o la tarea cayó sin poder decirlo.
        let tarea = self.tarea.take()?;
        match tarea.await {
            Ok(()) => None,
            Err(e) if e.is_panic() => Some(Err(Fallo::Panico {
                mensaje: mensaje_del_panico(e.into_panic()),
            })),
            Err(_) => Some(Err(Fallo::Cancelada)),
        }
    }

    /// Todos los lotes.
    pub async fn recoger(mut self) -> Result<Vec<RecordBatch>, Fallo> {
        let mut lotes = Vec::new();
        while let Some(l) = self.siguiente().await {
            lotes.push(l?);
        }
        Ok(lotes)
    }
}

impl Drop for Consulta {
    fn drop(&mut self) {
        if let Some(t) = self.tarea.take() {
            t.abort();
        }
    }
}

/// Lanza `plan` en `runtime`: el de CPU, o el de E/S si la consulta es distribuida y `scheduler` dice a quién
/// se manda. Ver el módulo.
pub(crate) fn lanzar(
    runtime: &Handle,
    sesion: &Sesion,
    plan: LogicalPlan,
    observacion: Arc<dyn Observacion>,
    scheduler: Option<String>,
) -> Consulta {
    let (envia, lotes) = mpsc::channel(LOTES_EN_VUELO);
    let estado = sesion.estado().clone();
    let tarea = runtime.spawn(async move {
        let fisico = match estado.create_physical_plan(&plan).await {
            Ok(p) => p,
            Err(e) => {
                let _ = envia.send(Err(e.into())).await;
                return;
            }
        };
        let ficha = Ficha {
            id: estado.session_id().to_string(),
            manifiesto: Manifiesto::actual(),
            config: entradas(estado.config()),
            plan: displayable(fisico.as_ref()).indent(true).to_string(),
        };
        observacion.al_empezar(&ficha);
        // Si esta tarea entra en pánico, `al_terminar` lo dice desde aquí: la guarda se suelta al deshacer.
        let guarda = Guarda {
            ficha: &ficha,
            observacion: &observacion,
            cluster: scheduler.map(|url| CancelaEnElCluster {
                plan: fisico.clone(),
                scheduler: url,
            }),
            cerrada: false,
        };
        let mut filas = 0usize;
        let resultado = async {
            let mut stream = execute_stream(fisico.clone(), estado.task_ctx())?;
            while let Some(lote) = stream.next().await {
                let lote = lote?;
                filas += lote.num_rows();
                if envia.send(Ok(lote)).await.is_err() {
                    // Quien la pidió ya no escucha: se suelta el stream y con él sus subtareas.
                    return Err(Fallo::Cancelada);
                }
            }
            Ok::<(), Fallo>(())
        }
        .await;
        let cierre = match &resultado {
            Ok(()) => Resultado::Hecha {
                filas,
                metricas:
                    datafusion::physical_plan::display::DisplayableExecutionPlan::with_metrics(
                        fisico.as_ref(),
                    )
                    .indent(true)
                    .to_string(),
            },
            // Quien la pidió dejó de leer: también el clúster la deja.
            Err(Fallo::Cancelada) => Resultado::Fallida {
                mensaje: guarda.cancelar(),
            },
            // Terminó, o falló por sí misma: el clúster ya la dio por acabada.
            Err(e) => Resultado::Fallida {
                mensaje: e.to_string(),
            },
        };
        guarda.cerrar(&cierre);
        if let Err(e) = resultado
            && !matches!(e, Fallo::Cancelada)
        {
            let _ = envia.send(Err(e)).await;
        }
    });
    Consulta {
        lotes,
        tarea: Some(tarea),
    }
}

/// Que `al_terminar` se llame también si la tarea cae: por un pánico, o porque se canceló (abortar una
/// tarea la suelta en su `await` pendiente, y con ella esta guarda). Y que, si la consulta es distribuida,
/// el clúster deje de calcularla.
struct Guarda<'a> {
    ficha: &'a Ficha,
    observacion: &'a Arc<dyn Observacion>,
    cluster: Option<CancelaEnElCluster>,
    cerrada: bool,
}

impl Guarda<'_> {
    fn cerrar(mut self, resultado: &Resultado) {
        self.cerrada = true;
        self.observacion.al_terminar(self.ficha, resultado);
    }

    /// Cancela el trabajo en el clúster, si lo hay, y dice qué se canceló.
    fn cancelar(&self) -> String {
        match self.cluster.as_ref().and_then(CancelaEnElCluster::cancelar) {
            Some(trabajo) => format!(
                "{}, y su trabajo `{trabajo}` en el scheduler",
                Fallo::Cancelada
            ),
            None => Fallo::Cancelada.to_string(),
        }
    }
}

impl Drop for Guarda<'_> {
    fn drop(&mut self) {
        if !self.cerrada {
            let cancelada = self.cancelar();
            let mensaje = if std::thread::panicking() {
                Fallo::Panico {
                    mensaje: "en la tarea".into(),
                }
                .to_string()
            } else {
                cancelada
            };
            self.observacion
                .al_terminar(self.ficha, &Resultado::Fallida { mensaje });
        }
    }
}

/// **Que soltar una consulta distribuida la cancele también en el clúster.** Ballista 55 no lo hace al soltar
/// su stream. Pide al scheduler que cancele el trabajo, en una tarea aparte del mismo runtime: la de la
/// consulta ya se está deshaciendo.
///
/// Necesita el id del trabajo, que el cliente sólo tiene al enviarlo en modo pull (`ballista.client.pull`,
/// en [`crate::opciones::DESVIOS`]).
struct CancelaEnElCluster {
    plan: Arc<dyn ExecutionPlan>,
    /// `df://host:puerto`.
    scheduler: String,
}

impl CancelaEnElCluster {
    /// El trabajo que se pidió cancelar, si el plan llegó a enviarse.
    fn cancelar(&self) -> Option<String> {
        let trabajo = self
            .plan
            .downcast_ref::<DistributedQueryExec<LogicalPlanNode>>()?
            .job_id()?;
        let runtime = Handle::try_current().ok()?;
        let url = self.scheduler.replacen("df://", "http://", 1);
        let id = trabajo.to_string();
        runtime.spawn(async move {
            if let Ok(mut s) = SchedulerGrpcClient::connect(url).await {
                let _ = s
                    .cancel_job(CancelJobParams {
                        job_id: trabajo.into(),
                    })
                    .await;
            }
        });
        Some(id)
    }
}

/// El texto de un pánico, si lo tiene.
fn mensaje_del_panico(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "sin mensaje".to_string())
}
