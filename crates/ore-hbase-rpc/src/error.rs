//! Los fallos del cliente, separados por lo que quien llama puede hacer con ellos.

use std::fmt;
use std::io::ErrorKind;

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
    /// `no_reintentar` es lo que el servidor dice (`do_not_retry`); **no manda** para las clases conocidas
    /// (ver [`Error::clase`]).
    Servidor {
        clase: String,
        mensaje: String,
        no_reintentar: bool,
    },
    /// La tabla no tiene regiones en `hbase:meta` (no existe, o el espacio de nombres es otro).
    TablaSinRegiones(String),
    /// Una region sin servidor asignado (se esta moviendo).
    RegionSinServidor(String),
    /// La tabla esta deshabilitada (o a punto): leerla no es cosa de esperar.
    TablaDeshabilitada(String),
    /// Lo que se pidio no tiene sentido (un nombre de tabla vacio, un quorum sin direcciones).
    Peticion(String),
    /// Quien llamo cancelo la lectura.
    Cancelada,
    /// Se agotaron los reintentos: el ultimo fallo y cuantos intentos seguidos se hicieron sin avanzar.
    Agotado { intentos: u32, ultimo: Box<Error> },
}

/// Lo que se hace con un fallo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    /// La region cambio de sitio, se partio o se esta abriendo: volver a preguntar a `hbase:meta` y seguir.
    Region,
    /// El scanner ya no existe en el servidor (arriendo caducado, secuencia perdida): abrir otro desde la ultima
    /// clave entregada.
    Scanner,
    /// El servidor esta saturado: esperar y repetir, sin tocar la cache.
    Ocupado,
    /// La red o ZooKeeper: soltar las conexiones, volver a preguntar y seguir.
    Red,
    /// Reintentar no lo arregla: credencial, tabla inexistente, peticion mal hecha.
    Fatal,
}

/// El nombre corto de una clase de Java: `org.apache.hadoop.hbase.NotServingRegionException` -> `NotServingRegionException`.
fn corta(clase: &str) -> &str {
    clase.rsplit('.').next().unwrap_or(clase)
}

impl Error {
    /// Que se hace con este fallo.
    ///
    /// **Manda la clase, no `do_not_retry`**: medido en el laboratorio (ficha 0072, N2), `UnknownScannerException` y
    /// `OutOfOrderScannerNextException` llegan con `do_not_retry = true` y, para un lector que reanuda por la ultima
    /// clave, son un «abre otro». `do_not_retry` solo decide las clases que no se conocen.
    pub fn clase(&self) -> Clase {
        match self {
            Error::Io(e) => match e.kind() {
                // un permiso denegado en el propio equipo no mejora con el tiempo
                ErrorKind::PermissionDenied | ErrorKind::InvalidInput => Clase::Fatal,
                _ => Clase::Red,
            },
            Error::Zk(_) => Clase::Red,
            Error::RegionSinServidor(_) => Clase::Region,
            Error::Servidor {
                clase,
                no_reintentar,
                ..
            } => match corta(clase) {
                "NotServingRegionException"
                | "RegionMovedException"
                | "RegionOpeningException"
                | "WrongRegionException"
                | "RegionServerStoppedException"
                | "RegionServerAbortedException"
                | "ServerNotRunningYetException"
                | "RegionOfflineException" => Clase::Region,
                "UnknownScannerException"
                | "OutOfOrderScannerNextException"
                | "ScannerResetException"
                | "LeaseException"
                | "LeaseExpiredException" => Clase::Scanner,
                "RegionTooBusyException"
                | "CallQueueTooBigException"
                | "CallDroppedException"
                | "ServerTooBusyException"
                | "RpcThrottlingException" => Clase::Ocupado,
                "ConnectionClosingException"
                | "FailedServerException"
                | "ConnectionClosedException" => Clase::Red,
                "AccessDeniedException"
                | "TableNotFoundException"
                | "TableNotEnabledException"
                | "NoSuchColumnFamilyException"
                | "FatalConnectionException"
                | "BadAuthException"
                | "WrongVersionException"
                | "UnsupportedCryptoException" => Clase::Fatal,
                // una clase que no se conoce: se hace caso al servidor
                _ if *no_reintentar => Clase::Fatal,
                _ => Clase::Region,
            },
            Error::Protocolo(_)
            | Error::TablaSinRegiones(_)
            | Error::TablaDeshabilitada(_)
            | Error::Peticion(_)
            | Error::Cancelada
            | Error::Agotado { .. } => Clase::Fatal,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "red: {e}"),
            Error::Protocolo(m) => write!(f, "protocolo de HBase: {m}"),
            Error::Zk(m) => write!(f, "ZooKeeper: {m}"),
            Error::Servidor { clase, mensaje, .. } => {
                write!(f, "HBase: {}: {mensaje}", corta(clase))
            }
            Error::TablaSinRegiones(t) => {
                write!(f, "la tabla `{t}` no tiene regiones en hbase:meta")
            }
            Error::RegionSinServidor(r) => write!(f, "la region `{r}` no tiene servidor asignado"),
            Error::TablaDeshabilitada(t) => write!(f, "la tabla `{t}` esta deshabilitada"),
            Error::Peticion(m) => write!(f, "peticion: {m}"),
            Error::Cancelada => write!(f, "lectura cancelada"),
            Error::Agotado { intentos, ultimo } => {
                write!(
                    f,
                    "se agotaron los reintentos ({intentos} seguidos sin avanzar); el ultimo fallo: {ultimo}"
                )
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn servidor(clase: &str, no_reintentar: bool) -> Error {
        Error::Servidor {
            clase: format!("org.apache.hadoop.hbase.{clase}"),
            mensaje: String::new(),
            no_reintentar,
        }
    }

    /// Lo que el laboratorio devolvio, con su `do_not_retry` (pruebas-de-fuego/hbase/trazas/excepciones.json).
    #[test]
    fn las_excepciones_grabadas_del_laboratorio_se_clasifican_como_se_recuperan() {
        let grabadas: serde_json::Value = serde_json::from_str(include_str!(
            "../../../pruebas-de-fuego/hbase/trazas/excepciones.json"
        ))
        .unwrap();
        let esperada = |escenario: &str| match escenario {
            "region_inexistente"
            | "clave_fuera_de_la_region"
            | "tabla_deshabilitada_scan_a_la_region"
            | "region_partida_scan_a_la_madre" => Clase::Region,
            "scanner_desconocido" | "secuencia_equivocada" | "arriendo_caducado" => Clase::Scanner,
            otro => panic!("un escenario grabado sin clasificar: {otro}"),
        };
        let mut vistos = 0;
        for g in grabadas.as_array().unwrap() {
            let e = Error::Servidor {
                clase: g["clase"].as_str().unwrap().into(),
                mensaje: g["mensaje"].as_str().unwrap().into(),
                no_reintentar: g["do_not_retry"].as_bool().unwrap(),
            };
            let escenario = g["escenario"].as_str().unwrap();
            assert_eq!(e.clase(), esperada(escenario), "{escenario}: {e}");
            vistos += 1;
        }
        assert_eq!(vistos, 7);
    }

    #[test]
    fn manda_la_clase_y_no_do_not_retry_salvo_en_las_desconocidas() {
        // medido: el servidor marca `do_not_retry` en las del scanner, y un lector las trata como «abre otro»
        assert_eq!(
            servidor("UnknownScannerException", true).clase(),
            Clase::Scanner
        );
        // una clase conocida como fatal lo es aunque el servidor diga que se reintente
        assert_eq!(
            servidor("TableNotFoundException", false).clase(),
            Clase::Fatal
        );
        // y una que no se conoce obedece al servidor
        assert_eq!(servidor("UnaClaseNueva", true).clase(), Clase::Fatal);
        assert_eq!(servidor("UnaClaseNueva", false).clase(), Clase::Region);
    }

    #[test]
    fn la_red_se_reintenta_y_lo_que_no_se_arregla_esperando_no() {
        let io = |k| Error::Io(std::io::Error::from(k));
        for k in [
            ErrorKind::ConnectionRefused,
            ErrorKind::ConnectionReset,
            ErrorKind::UnexpectedEof,
            ErrorKind::TimedOut,
        ] {
            assert_eq!(io(k).clase(), Clase::Red, "{k:?}");
        }
        assert_eq!(io(ErrorKind::PermissionDenied).clase(), Clase::Fatal);
        assert_eq!(Error::Zk("sin sesion".into()).clase(), Clase::Red);
        assert_eq!(
            servidor("RegionTooBusyException", false).clase(),
            Clase::Ocupado
        );
        assert_eq!(Error::TablaDeshabilitada("t".into()).clase(), Clase::Fatal);
        assert_eq!(Error::Cancelada.clase(), Clase::Fatal);
        assert_eq!(Error::Protocolo("x".into()).clase(), Clase::Fatal);
    }
}
