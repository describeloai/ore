//! Los fallos del cliente, separados por lo que quien llama puede hacer con ellos.

use std::fmt;

/// Un fallo del cliente de HBase.
#[derive(Debug)]
pub enum Error {
    /// La red: no se pudo conectar, se cerro la conexion o se agoto el tiempo.
    Io(std::io::Error),
    /// El servidor contesto algo que no es el protocolo (una trama corta, un mensaje que no decodifica).
    Protocolo(String),
    /// ZooKeeper: no hay quorum alcanzable, la sesion no se abrio o el nodo no existe.
    Zk(String),
    /// Una excepcion de HBase: la clase Java y su texto, tal como llegan en `ResponseHeader.exception`.
    Servidor { clase: String, mensaje: String },
    /// La tabla no tiene regiones en `hbase:meta` (no existe, o el espacio de nombres es otro).
    TablaSinRegiones(String),
    /// Una region sin servidor asignado (se esta moviendo).
    RegionSinServidor(String),
    /// Lo que se pidio no tiene sentido (un nombre de tabla vacio, un quorum sin direcciones).
    Peticion(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "red: {e}"),
            Error::Protocolo(m) => write!(f, "protocolo de HBase: {m}"),
            Error::Zk(m) => write!(f, "ZooKeeper: {m}"),
            Error::Servidor { clase, mensaje } => {
                let corta = clase.rsplit('.').next().unwrap_or(clase);
                write!(f, "HBase: {corta}: {mensaje}")
            }
            Error::TablaSinRegiones(t) => {
                write!(f, "la tabla `{t}` no tiene regiones en hbase:meta")
            }
            Error::RegionSinServidor(r) => write!(f, "la region `{r}` no tiene servidor asignado"),
            Error::Peticion(m) => write!(f, "peticion: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Resultado<T> = Result<T, Error>;
