//! Los errores del ORE Runtime.

use datafusion::error::DataFusionError;
use std::fmt;

/// Lo que puede salir mal en el suelo.
#[derive(Debug)]
pub enum Fallo {
    /// Una opción que no existe, o un valor que no vale: nunca un pánico (0081 S0 C2).
    Config(String),
    /// Lo que el proceso tiene no se pudo leer o no sirve.
    Limites(String),
    /// Lo que DataFusion contesta.
    Ejecucion(DataFusionError),
    /// Un operador entró en pánico: la consulta falla y el proceso sigue.
    Panico { mensaje: String },
    /// Quien pidió la consulta la soltó.
    Cancelada,
    /// Lo que Ballista contesta: el scheduler, el ejecutor o la red entre ellos.
    Distribucion(String),
}

impl fmt::Display for Fallo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fallo::Config(m) => write!(f, "configuración: {m}"),
            Fallo::Limites(m) => write!(f, "límites del proceso: {m}"),
            Fallo::Ejecucion(e) => write!(f, "{e}"),
            Fallo::Panico { mensaje } => write!(f, "la consulta entró en pánico: {mensaje}"),
            Fallo::Cancelada => write!(f, "la consulta se canceló"),
            Fallo::Distribucion(m) => write!(f, "distribución: {m}"),
        }
    }
}

impl std::error::Error for Fallo {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Fallo::Ejecucion(e) => Some(e),
            _ => None,
        }
    }
}

impl From<DataFusionError> for Fallo {
    fn from(e: DataFusionError) -> Self {
        Fallo::Ejecucion(e)
    }
}

impl From<ballista_core::error::BallistaError> for Fallo {
    fn from(e: ballista_core::error::BallistaError) -> Self {
        Fallo::Distribucion(e.to_string())
    }
}
