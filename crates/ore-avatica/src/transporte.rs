//! El HTTP de Avatica: un `POST` con un `WireMessage` protobuf y otro `WireMessage` de vuelta.
//! Medido en X0 (0071 §3.2): `Content-Type: application/x-google-protobuf`, el nombre del mensaje es
//! `org.apache.calcite.avatica.proto.Requests$<Nombre>` y su respuesta `…Responses$<Nombre>`.
//!
//! Aquí se decide qué es un error de red, de HTTP, del servidor o de protocolo; nada más.

use std::io::Read;
use std::time::Duration;

use prost::Message;

use crate::error::Error;
use crate::proto::{ErrorResponse, WireMessage};

const PREFIJO_PETICION: &str = "org.apache.calcite.avatica.proto.Requests$";
const PREFIJO_RESPUESTA: &str = "org.apache.calcite.avatica.proto.Responses$";
/// Lo máximo que se lee de un cuerpo que no es de Avatica (una página de error): se recorta.
const MAX_CUERPO_AJENO: usize = 512;
/// Lo máximo que se acepta de una respuesta de Avatica. Un frame de 50 000 filas de ~100 B son 5 MB.
const MAX_RESPUESTA: u64 = 256 * 1024 * 1024;

/// Cómo hablar con el servidor.
#[derive(Clone)]
pub struct Opciones {
    /// `http://host:8765/` (o `https://`). **Sin credenciales**: van aparte.
    pub url: String,
    /// Usuario y contraseña de HTTP *Basic*. Nunca salen en un mensaje de error ni en `Debug`.
    pub credencial: Option<(String, String)>,
    pub tiempo_conexion: Duration,
    /// Lo que se espera a una respuesta, **por petición**. Un `PrepareAndExecute` pesado tarda lo que
    /// tarde el servidor: el tope de la consulta es `phoenix.query.timeoutMs`, y éste lo cubre.
    pub tiempo_peticion: Duration,
    /// Propiedades de `OpenConnection` (`TenantId`, `CurrentSCN`, `phoenix.query.timeoutMs`…).
    pub propiedades: std::collections::BTreeMap<String, String>,
    /// Filas por frame, en el primero y en los `Fetch`. Ver 0071 §3.7: ≥ 10 000.
    pub filas_por_frame: i32,
}

impl Opciones {
    pub fn nuevas(url: impl Into<String>) -> Self {
        Opciones {
            url: url.into(),
            credencial: None,
            tiempo_conexion: Duration::from_secs(10),
            tiempo_peticion: Duration::from_secs(600),
            propiedades: Default::default(),
            filas_por_frame: 20_000,
        }
    }
}

impl std::fmt::Debug for Opciones {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opciones")
            .field("url", &self.url)
            .field("credencial", &self.credencial.as_ref().map(|_| "***"))
            .field("tiempo_conexion", &self.tiempo_conexion)
            .field("tiempo_peticion", &self.tiempo_peticion)
            .field("propiedades", &self.propiedades.keys().collect::<Vec<_>>())
            .field("filas_por_frame", &self.filas_por_frame)
            .finish()
    }
}

/// El agente HTTP (con su pool de conexiones) y la dirección. Compartido por una conexión y sus lecturas.
pub(crate) struct Transporte {
    agente: ureq::Agent,
    url: String,
    autorizacion: Option<String>,
}

impl Transporte {
    pub(crate) fn nuevo(o: &Opciones) -> Result<Self, Error> {
        if !(o.url.starts_with("http://") || o.url.starts_with("https://")) {
            return Err(Error::protocolo(
                "la URL del servidor tiene que empezar por http:// o https://",
            ));
        }
        // Una credencial en la URL acabaría en un registro: no se acepta aquí.
        let resto = o.url.split("://").nth(1).unwrap_or("");
        if resto.split('/').next().is_some_and(|a| a.contains('@')) {
            return Err(Error::protocolo(
                "la URL no lleva usuario ni contraseña: van en la credencial",
            ));
        }
        let agente = ureq::AgentBuilder::new()
            .timeout_connect(o.tiempo_conexion)
            .timeout(o.tiempo_peticion)
            .build();
        let autorizacion = o
            .credencial
            .as_ref()
            .map(|(u, c)| format!("Basic {}", base64(format!("{u}:{c}").as_bytes())));
        Ok(Transporte {
            agente,
            url: o.url.clone(),
            autorizacion,
        })
    }

    /// Como [`Transporte::llamar`], pero **repite una vez** si la red falló (`Error::Red`: un reset, una conexión
    /// que el servidor cerró entre dos peticiones, un puerto que se reenvía mal). No se repite un tiempo agotado
    /// (se esperaría el doble) ni lo que el servidor contestó. **Solo para lo que se puede repetir**: abrir,
    /// crear un statement, los metadatos y un `SELECT`; nunca un `Fetch` (el cursor ya avanzó).
    pub(crate) fn llamar_repitiendo<P: Message, R: Message + Default>(
        &self,
        nombre: &str,
        peticion: &P,
        esperado: &str,
    ) -> Result<R, Error> {
        match self.llamar(nombre, peticion, esperado) {
            Err(Error::Red(_)) => {
                std::thread::sleep(std::time::Duration::from_millis(150));
                self.llamar(nombre, peticion, esperado)
            }
            otro => otro,
        }
    }

    /// Envía `peticion` (de nombre `nombre`) y devuelve el cuerpo de la respuesta del tipo `esperado`.
    pub(crate) fn llamar<P: Message, R: Message + Default>(
        &self,
        nombre: &str,
        peticion: &P,
        esperado: &str,
    ) -> Result<R, Error> {
        let envoltorio = WireMessage {
            name: format!("{PREFIJO_PETICION}{nombre}"),
            wrapped_message: peticion.encode_to_vec(),
        };
        let mut req = self
            .agente
            .post(&self.url)
            .set("Content-Type", "application/x-google-protobuf");
        if let Some(a) = &self.autorizacion {
            req = req.set("Authorization", a);
        }
        let resp = match req.send_bytes(&envoltorio.encode_to_vec()) {
            Ok(r) => r,
            // Con 4xx/5xx, `ureq` da un `Error::Status` con la respuesta: el PQS devuelve un
            // `ErrorResponse` en un 500 (medido), así que se lee igual que un 200.
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(de_transporte(&t)),
        };
        let estado = resp.status();
        let mut cuerpo = Vec::new();
        resp.into_reader()
            .take(MAX_RESPUESTA)
            .read_to_end(&mut cuerpo)
            .map_err(|e| de_io(&e))?;

        let recibido = WireMessage::decode(cuerpo.as_slice());
        let w = match recibido {
            Ok(w) if w.name.starts_with(PREFIJO_RESPUESTA) => w,
            _ => return Err(ajeno(estado, &cuerpo)),
        };
        if w.name == format!("{PREFIJO_RESPUESTA}ErrorResponse") {
            let e = ErrorResponse::decode(w.wrapped_message.as_slice()).map_err(|e| {
                Error::protocolo(format!("un ErrorResponse que no se entiende: {e}"))
            })?;
            return Err(Error::Servidor {
                codigo: e.error_code,
                sql_state: e.sql_state,
                mensaje: e.error_message,
                excepciones: e.exceptions,
            });
        }
        if estado != 200 {
            return Err(Error::Http {
                estado,
                cuerpo: recortar(&cuerpo),
            });
        }
        let esperado = format!("{PREFIJO_RESPUESTA}{esperado}");
        if w.name != esperado {
            return Err(Error::protocolo(format!(
                "se esperaba {esperado} y llegó {}",
                w.name
            )));
        }
        R::decode(w.wrapped_message.as_slice())
            .map_err(|e| Error::protocolo(format!("{esperado} no se entiende: {e}")))
    }
}

/// Una respuesta que no es un `WireMessage` de Avatica: un proxy, un balanceador, una página de error.
fn ajeno(estado: u16, cuerpo: &[u8]) -> Error {
    Error::Http {
        estado,
        cuerpo: recortar(cuerpo),
    }
}

fn recortar(cuerpo: &[u8]) -> String {
    let n = cuerpo.len().min(MAX_CUERPO_AJENO);
    let t = String::from_utf8_lossy(&cuerpo[..n]).replace(['\r', '\n'], " ");
    if cuerpo.len() > n {
        format!("{t}…")
    } else {
        t
    }
}

fn de_transporte(t: &ureq::Transport) -> Error {
    let m = t.to_string();
    // `ureq` 2 no separa el tiempo agotado del resto en el tipo: va en el texto del error de E/S.
    if m.contains("timed out") || m.contains("timeout") || m.contains("os error 10060") {
        Error::TiempoRed(m)
    } else {
        Error::Red(m)
    }
}

fn de_io(e: &std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            Error::TiempoRed(e.to_string())
        }
        _ => Error::Red(e.to_string()),
    }
}

/// Base64 estándar con relleno. Es lo único que hace falta aquí y no merece una dependencia.
fn base64(entrada: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(entrada.len().div_ceil(3) * 4);
    for trozo in entrada.chunks(3) {
        let b = [
            trozo[0],
            *trozo.get(1).unwrap_or(&0),
            *trozo.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if trozo.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if trozo.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn base64_de_la_rfc_4648() {
        for (e, s) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(e.as_bytes()), s);
        }
    }

    #[test]
    fn una_url_con_credenciales_se_rechaza_y_no_se_repite_la_contrasena() {
        let o = Opciones::nuevas("http://u:secreta-1234@host:8765/");
        let e = Transporte::nuevo(&o).err().expect("tiene que rechazarse");
        assert!(!e.to_string().contains("secreta-1234"), "{e}");
    }

    #[test]
    fn debug_de_las_opciones_no_lleva_la_contrasena() {
        let mut o = Opciones::nuevas("http://host:8765/");
        o.credencial = Some(("usuario".into(), "secreta-1234".into()));
        assert!(!format!("{o:?}").contains("secreta-1234"));
    }

    #[test]
    fn un_cuerpo_ajeno_se_recorta() {
        let e = ajeno(502, &vec![b'x'; 5000]);
        let Error::Http { cuerpo, .. } = e else {
            panic!()
        };
        assert!(cuerpo.chars().count() <= MAX_CUERPO_AJENO + 1);
    }
}
