//! **Los zócalos** (0081 §7.4, propiedad 2): los puntos de enchufe donde encajan los demás frentes. El
//! suelo los llama; los frentes los llenan. Se definen por lo que el suelo necesita llamar, no por lo que
//! harán los frentes: cada frente puede ensanchar el suyo, pero no añadir uno sin pasar por el suelo.
//!
//! | zócalo | qué llama el suelo | lo llena |
//! |---|---|---|
//! | [`Catalogo`] | `preparar`: antes de planificar, la lista de catálogos **de esa consulta** | frente 3 |
//! | [`Almacenes`] | el `ObjectStoreRegistry` del `RuntimeEnv` | frente 4 |
//! | [`Memoria`] | la raíz de memoria del proceso | frente 7 |
//! | [`Extensiones`] | lo que se añade a la plantilla: funciones, reglas, planificadores | frentes 2, 5 y 6 |
//! | [`Observacion`] | al empezar y al terminar cada consulta, con su [`Ficha`] | frente 9 |
//!
//! La lectura y la escritura no son zócalos: llegan como `TableProvider` por el catálogo y como reglas y
//! planificadores por las extensiones. La distribución es de los papeles (S3) y del frente 8.

use crate::fallo::Fallo;
use crate::manifiesto::Manifiesto;
use crate::viaje::Punto;
use async_trait::async_trait;
use datafusion::catalog::{CatalogProviderList, TableProvider};
use datafusion::common::TableReference;
use datafusion::execution::SessionState;
use datafusion::execution::SessionStateBuilder;
use datafusion::execution::memory_pool::{GreedyMemoryPool, MemoryPool, TrackConsumersPool};
use datafusion::execution::object_store::ObjectStoreRegistry;
use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// **El zócalo de catálogo**: resolver los nombres de una consulta antes de planificarla (el patrón de
/// catálogo remoto de DataFusion: los traits del catálogo son síncronos y no deben hacer E/S al planificar).
///
/// Devuelve **la lista de catálogos de esa consulta**, que la sesión instala como suya. No registra nada en
/// la de la plantilla: esa la comparten todas las sesiones del proceso, y lo que se registrara en ella se
/// filtraría de una consulta a otra. Una lista resuelta por consulta es también donde se fija el snapshot de
/// cada tabla (0081 §4, el aislamiento).
#[async_trait]
pub trait Catalogo: Send + Sync + Debug {
    /// `base` es el estado de la sesión antes de resolver; `referencias`, las tablas que nombra la consulta.
    async fn preparar(
        &self,
        base: &SessionState,
        referencias: &[TableReference],
    ) -> Result<Arc<dyn CatalogProviderList>, Fallo>;

    /// **El viaje en el tiempo** (0081 §7.5, C9): la tabla que este catálogo dio, como estaba en `punto`. Sin E/S:
    /// lo llama el planificador de `at(…)`, que es síncrono. Por defecto, este catálogo no viaja.
    fn en_el_tiempo(
        &self,
        tabla: &Arc<dyn TableProvider>,
        punto: &Punto,
    ) -> Result<Arc<dyn TableProvider>, Fallo> {
        let _ = (tabla, punto);
        Err(Fallo::Catalogo(
            "este catálogo no viaja en el tiempo (`at(…)`)".into(),
        ))
    }
}

/// Sin catálogo propio: el de la plantilla (el de memoria de DataFusion).
#[derive(Debug, Default)]
pub struct SinCatalogo;

#[async_trait]
impl Catalogo for SinCatalogo {
    async fn preparar(
        &self,
        base: &SessionState,
        _referencias: &[TableReference],
    ) -> Result<Arc<dyn CatalogProviderList>, Fallo> {
        Ok(base.catalog_list().clone())
    }
}

/// **El zócalo de almacenes**: de dónde lee y escribe bytes el motor. Lo llena el frente 4, con un almacén
/// por tabla y la credencial prestada.
pub trait Almacenes: Send + Sync + Debug {
    /// El registro de almacenes del `RuntimeEnv`. `None`: el de DataFusion.
    fn registro(&self) -> Option<Arc<dyn ObjectStoreRegistry>>;
}

/// Sin almacenes propios: el registro de DataFusion.
#[derive(Debug, Default)]
pub struct SinAlmacenes;

impl Almacenes for SinAlmacenes {
    fn registro(&self) -> Option<Arc<dyn ObjectStoreRegistry>> {
        None
    }
}

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

/// **El zócalo de extensiones**: lo que se añade a la plantilla al construirla, una vez por proceso.
/// Funciones, reglas del analizador y del optimizador, planificadores.
pub trait Extensiones: Send + Sync + Debug {
    fn aplicar(&self, plantilla: SessionStateBuilder) -> SessionStateBuilder;
}

/// Sin extensiones: lo que DataFusion trae.
#[derive(Debug, Default)]
pub struct SinExtensiones;

impl Extensiones for SinExtensiones {
    fn aplicar(&self, plantilla: SessionStateBuilder) -> SessionStateBuilder {
        plantilla
    }
}

/// **Lo que se sabe de una consulta al empezar**: con esto se puede rehacer.
#[derive(Debug, Clone)]
pub struct Ficha {
    /// El id de la consulta (el de su sesión).
    pub id: String,
    /// Qué runtime la corrió.
    pub manifiesto: Manifiesto,
    /// Su configuración, en la foto canónica ([`crate::opciones::entradas`]).
    pub config: Vec<(String, Option<String>)>,
    /// Su plan físico.
    pub plan: String,
}

/// Cómo acabó una consulta.
#[derive(Debug, Clone)]
pub enum Resultado {
    /// Terminó: sus filas y el plan con sus métricas.
    Hecha { filas: usize, metricas: String },
    /// Falló, o entró en pánico: el mensaje.
    Fallida { mensaje: String },
}

/// **El zócalo de observación**: lo llena el frente 9. Se llama desde el runtime de CPU, así que no debe
/// bloquear: lo que tenga que mandar lejos, que lo encole.
pub trait Observacion: Send + Sync + Debug {
    fn al_empezar(&self, ficha: &Ficha);
    fn al_terminar(&self, ficha: &Ficha, resultado: &Resultado);
}

/// Sin observación.
#[derive(Debug, Default)]
pub struct SinObservacion;

impl Observacion for SinObservacion {
    fn al_empezar(&self, _ficha: &Ficha) {}
    fn al_terminar(&self, _ficha: &Ficha, _resultado: &Resultado) {}
}

/// Los zócalos de un motor. Por defecto, los que no hacen nada propio.
#[derive(Debug, Clone)]
pub struct Zocalos {
    pub catalogo: Arc<dyn Catalogo>,
    pub almacenes: Arc<dyn Almacenes>,
    pub memoria: Arc<dyn Memoria>,
    pub extensiones: Arc<dyn Extensiones>,
    pub observacion: Arc<dyn Observacion>,
}

impl Default for Zocalos {
    fn default() -> Self {
        Zocalos {
            catalogo: Arc::new(SinCatalogo),
            almacenes: Arc::new(SinAlmacenes),
            memoria: Arc::new(MemoriaPorDefecto),
            extensiones: Arc::new(SinExtensiones),
            observacion: Arc::new(SinObservacion),
        }
    }
}
