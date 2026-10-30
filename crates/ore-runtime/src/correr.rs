//! **Ejecutar sin ahogar ni caer** (0081 §7.4, propiedad 6). `Motor::correr` es la única entrada para
//! ejecutar un plan, y un solo mecanismo da tres propiedades:
//!
//! 1. **La CPU nunca ahoga a la E/S ni a la salud**: el plan corre en el runtime de CPU, y quien lo pide lo
//!    lee desde el runtime que quiera, por un canal acotado.
//! 2. **Un pánico llega como [`Fallo::Panico`] y el proceso sigue**: la tarea que corre el plan es la
//!    frontera. DataFusion vuelve a lanzar los pánicos de sus operadores (0081 S0 C7) y tokio los para en
//!    la tarea. Exige `panic = "unwind"`, que el perfil del binario pone en S3.
//! 3. **Soltar la [`Consulta`] la cancela**: aborta la tarea, que suelta el stream, y DataFusion aborta sus
//!    subtareas al soltarse (`SpawnedTask`).
//!
//! El precio es un canal por consulta, despreciable frente a la E/S.

use crate::fallo::Fallo;
use crate::manifiesto::Manifiesto;
use crate::opciones::entradas;
use crate::sesion::Sesion;
use crate::zocalos::{Ficha, Observacion, Resultado};
use datafusion::arrow::array::RecordBatch;
use datafusion::logical_expr::LogicalPlan;
use datafusion::physical_plan::{displayable, execute_stream};
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

/// Lanza `plan` en el runtime de CPU (`cpu`). Ver el módulo.
pub(crate) fn lanzar(
    cpu: &Handle,
    sesion: &Sesion,
    plan: LogicalPlan,
    observacion: Arc<dyn Observacion>,
) -> Consulta {
    let (envia, lotes) = mpsc::channel(LOTES_EN_VUELO);
    let estado = sesion.estado().clone();
    let tarea = cpu.spawn(async move {
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
/// tarea la suelta en su `await` pendiente, y con ella esta guarda).
struct Guarda<'a> {
    ficha: &'a Ficha,
    observacion: &'a Arc<dyn Observacion>,
    cerrada: bool,
}

impl Guarda<'_> {
    fn cerrar(mut self, resultado: &Resultado) {
        self.cerrada = true;
        self.observacion.al_terminar(self.ficha, resultado);
    }
}

impl Drop for Guarda<'_> {
    fn drop(&mut self) {
        if !self.cerrada {
            let mensaje = if std::thread::panicking() {
                Fallo::Panico {
                    mensaje: "en la tarea".into(),
                }
            } else {
                Fallo::Cancelada
            };
            self.observacion.al_terminar(
                self.ficha,
                &Resultado::Fallida {
                    mensaje: mensaje.to_string(),
                },
            );
        }
    }
}

/// El texto de un pánico, si lo tiene.
fn mensaje_del_panico(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "sin mensaje".to_string())
}
