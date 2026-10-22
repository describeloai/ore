//! Los errores del cliente, separados por **quién** falló: la red, el servidor (con el código que dio)
//! o el protocolo (algo que el servidor hizo y la documentación no dice). Ficha 0071 §3.3 y §3.8.
//!
//! Nada de aquí lleva la contraseña: el mensaje nombra la URL sin credenciales, y un cuerpo HTTP
//! ajeno (una página de error de un balanceador) se recorta.

use std::fmt;

/// Lo que puede salir mal.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// No se llegó al servidor: DNS, conexión rechazada, TLS, cable cortado.
    Red(String),
    /// Se llegó, y no contestó a tiempo.
    TiempoRed(String),
    /// El *handshake* de TLS no terminó: el certificado no se pudo verificar (otra CA, caducado, otro nombre) o el
    /// servidor no habla TLS 1.2 o más. **No se reintenta** (volvería a pasar) y **no se envió la credencial**: la
    /// cabecera `Authorization` solo sale después del *handshake*.
    Tls(String),
    /// Una respuesta HTTP que no es de Avatica (un balanceador, un proxy, una página de error).
    Http { estado: u16, cuerpo: String },
    /// El PQS contestó con un `ErrorResponse`: Phoenix dijo que no, y con qué código.
    Servidor {
        codigo: u32,
        sql_state: String,
        mensaje: String,
        /// La pila de Java que trae el PQS, una cadena por excepción. Puede ser larga.
        excepciones: Vec<String>,
    },
    /// El servidor contestó algo que el protocolo no prevé (otro tipo de mensaje, un campo que falta).
    Protocolo(String),
    /// `missing_statement` o `missing_results`: el *statement* ya no existe en el PQS (caducó, se cerró
    /// o el PQS se reinició). **Sin esta bandera, el `Fetch` daría 0 filas y sin error**: un truncado
    /// silencioso (0071 §3.3).
    StatementPerdido,
    /// Un frame sin filas y sin `done`: el cursor no avanza. No se reintenta (el `offset` no se honra,
    /// §3.3): se corta antes de contar una lectura incompleta como completa.
    Truncado { leidas: u64 },
}

impl Error {
    pub(crate) fn protocolo(m: impl Into<String>) -> Self {
        Error::Protocolo(m.into())
    }

    /// El error es el servidor dando por vencido el tiempo límite (`phoenix.query.timeoutMs`). Como todo
    /// error SQL del PQS, llega sin código (`u32::MAX`, estado `00000`) y solo se reconoce por el texto:
    /// `SQLTimeoutException: … allotted time` en una consulta simple, `TimeoutException` a secas dentro de
    /// un *sub plan* (una unión). Medido en X0 y en las trazas (0071 §3.8).
    pub fn es_tiempo_limite(&self) -> bool {
        match self {
            Error::TiempoRed(_) => true,
            Error::Servidor { mensaje, .. } => {
                mensaje.contains("TimeoutException") || mensaje.contains("allotted time")
            }
            _ => false,
        }
    }
}

impl Error {
    /// El servidor ya no conoce la conexión (se reinició, o la dio por caducada): `NoSuchConnectionException`. El
    /// identificador lo inventa el cliente, así que se puede abrir otra vez con el mismo.
    pub fn conexion_perdida(&self) -> bool {
        matches!(self, Error::Servidor { mensaje, .. } if mensaje.contains("NoSuchConnectionException"))
    }

    /// ¿Vale la pena **empezar de nuevo la operación entera**? No es «repite esta petición»: un `Fetch` no
    /// se repite nunca, el servidor ignora el `offset` y el trozo ya está consumido (0071 §3.3). Lo que se
    /// reinicia es la lectura desde su `ejecutar`, o la apertura de la conexión (que se hace con un
    /// identificador nuevo). Se reintenta lo que es del camino (red, tiempo agotado, un 502/503/504 de un
    /// balanceador) y un statement perdido; **no** lo que el servidor contestó (un error SQL, un tiempo
    /// límite de Phoenix: volvería a pasar), ni lo que el protocolo no prevé.
    pub fn reintentable(&self) -> bool {
        match self {
            Error::Red(_) | Error::TiempoRed(_) | Error::StatementPerdido => true,
            Error::Http { estado, .. } => matches!(estado, 502..=504),
            Error::Servidor { .. }
            | Error::Protocolo(_)
            | Error::Truncado { .. }
            | Error::Tls(_) => false,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Red(m) => write!(f, "sin conexión con el servidor: {m}"),
            Error::TiempoRed(m) => write!(f, "el servidor no contestó a tiempo: {m}"),
            Error::Tls(m) => write!(
                f,
                "no se pudo verificar el servidor por TLS ({m}); no se envió la credencial"
            ),
            Error::Http { estado, cuerpo } => {
                write!(f, "respuesta HTTP {estado} que no es de Avatica: {cuerpo}")
            }
            Error::Servidor {
                codigo,
                sql_state,
                mensaje,
                ..
            } => {
                write!(
                    f,
                    "el servidor dijo que no ({codigo} / {sql_state}): {mensaje}"
                )
            }
            Error::Protocolo(m) => write!(f, "respuesta fuera del protocolo: {m}"),
            Error::StatementPerdido => write!(
                f,
                "el servidor ya no tiene el statement (caducó, se cerró o el servidor se reinició): \
                 la lectura hay que empezarla de nuevo"
            ),
            Error::Truncado { leidas } => write!(
                f,
                "el servidor dejó de dar filas sin terminar la consulta tras {leidas}: la lectura está incompleta"
            ),
        }
    }
}

impl std::error::Error for Error {}
