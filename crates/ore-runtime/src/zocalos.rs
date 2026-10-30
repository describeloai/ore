//! **Los zócalos** (0081 §7.4, propiedad 2): los puntos de enchufe donde encajan los demás frentes. El
//! suelo los llama; los frentes los llenan. Se definen por lo que el suelo necesita llamar, no por lo que
//! harán los frentes: cada frente puede ensanchar el suyo, pero no añadir uno sin pasar por el suelo.
//!
//! Hoy (S2·4) está el de memoria. Catálogo, almacenes, extensiones y observación llegan en S2·7.

use datafusion::execution::memory_pool::{GreedyMemoryPool, MemoryPool, TrackConsumersPool};
use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// **El zócalo de memoria**: la raíz de memoria del proceso. Lo llena el frente 7 con su pool jerárquico.
pub trait Memoria: Send + Sync + Debug {
    /// La raíz, para `limite` bytes (ya descontada la holgura).
    fn raiz(&self, limite: usize) -> Arc<dyn MemoryPool>;
}

/// Cuántos consumidores nombra un `ResourcesExhausted`.
pub const CONSUMIDORES_NOMBRADOS: usize = 5;

/// La raíz por defecto: `Greedy`, que da lo que hay mientras lo haya, envuelto para que el error nombre a
/// los mayores consumidores. Es lo que DataFusion construye con `with_memory_limit`.
#[derive(Debug, Default)]
pub struct MemoriaPorDefecto;

impl Memoria for MemoriaPorDefecto {
    fn raiz(&self, limite: usize) -> Arc<dyn MemoryPool> {
        Arc::new(TrackConsumersPool::new(
            GreedyMemoryPool::new(limite),
            NonZeroUsize::new(CONSUMIDORES_NOMBRADOS).expect("mayor que 0"),
        ))
    }
}

/// Los zócalos de un motor. Por defecto, los que no hacen nada propio.
#[derive(Debug, Clone)]
pub struct Zocalos {
    /// La raíz de memoria (frente 7).
    pub memoria: Arc<dyn Memoria>,
}

impl Default for Zocalos {
    fn default() -> Self {
        Zocalos {
            memoria: Arc::new(MemoriaPorDefecto),
        }
    }
}
