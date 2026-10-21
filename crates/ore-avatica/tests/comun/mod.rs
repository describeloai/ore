//! Un servidor HTTP falso, en el propio proceso, que habla lo justo de HTTP/1.1 para que `ureq` le
//! mande `POST`s de Avatica. Las pruebas deciden qué contesta (los bytes grabados de un PQS real, o
//! mensajes fabricados) y miran qué recibió. Corre en el CI sin laboratorio.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ore_avatica::proto::WireMessage;
use prost::Message;

pub const PREFIJO_PETICION: &str = "org.apache.calcite.avatica.proto.Requests$";
pub const PREFIJO_RESPUESTA: &str = "org.apache.calcite.avatica.proto.Responses$";

/// Lo que el servidor recibió.
#[derive(Debug, Clone)]
pub struct Recibida {
    /// `OpenConnectionRequest`, `FetchRequest`…
    pub nombre: String,
    pub mensaje: Vec<u8>,
    pub autorizacion: Option<String>,
    pub tipo_contenido: Option<String>,
}

/// Lo que el servidor contesta.
pub struct Respuesta {
    /// El estado HTTP; 0 corta la conexión sin contestar.
    pub estado: u16,
    pub cuerpo: Vec<u8>,
}

impl Respuesta {
    pub fn avatica(estado: u16, nombre: &str, m: &impl Message) -> Respuesta {
        let w = WireMessage {
            name: format!("{PREFIJO_RESPUESTA}{nombre}"),
            wrapped_message: m.encode_to_vec(),
        };
        Respuesta {
            estado,
            cuerpo: w.encode_to_vec(),
        }
    }
}

pub struct Servidor {
    pub direccion: SocketAddr,
    pub recibidas: Arc<Mutex<Vec<Recibida>>>,
}

impl Servidor {
    /// `guion` recibe cada petición y decide la respuesta; `None` = no contesta (se cuelga).
    pub fn arrancar<F>(guion: F) -> Servidor
    where
        F: Fn(&Recibida) -> Option<Respuesta> + Send + Sync + 'static,
    {
        let escucha = TcpListener::bind("127.0.0.1:0").expect("puerto libre");
        let direccion = escucha.local_addr().unwrap();
        let recibidas: Arc<Mutex<Vec<Recibida>>> = Arc::default();
        let guion = Arc::new(guion);
        let log = recibidas.clone();
        std::thread::spawn(move || {
            for flujo in escucha.incoming().flatten() {
                let (guion, log) = (guion.clone(), log.clone());
                std::thread::spawn(move || atender(flujo, &*guion, &log));
            }
        });
        Servidor {
            direccion,
            recibidas,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}/", self.direccion)
    }

    pub fn nombres(&self) -> Vec<String> {
        self.recibidas
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.nombre.clone())
            .collect()
    }

    /// Todas las peticiones de ese nombre, decodificadas.
    pub fn de<M: Message + Default>(&self, nombre: &str) -> Vec<M> {
        self.recibidas
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.nombre == nombre)
            .map(|r| M::decode(r.mensaje.as_slice()).expect("petición bien formada"))
            .collect()
    }
}

fn atender(
    mut s: TcpStream,
    guion: &(dyn Fn(&Recibida) -> Option<Respuesta> + Send + Sync),
    log: &Mutex<Vec<Recibida>>,
) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
    loop {
        // Cabecera hasta \r\n\r\n
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            match s.read(&mut byte) {
                Ok(1) => buf.push(byte[0]),
                _ => return,
            }
        }
        let cab = String::from_utf8_lossy(&buf).to_string();
        let valor = |n: &str| {
            cab.lines().find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case(n))
                    .map(|(_, v)| v.trim().to_string())
            })
        };
        let largo: usize = valor("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut cuerpo = vec![0u8; largo];
        if s.read_exact(&mut cuerpo).is_err() {
            return;
        }
        let w = WireMessage::decode(cuerpo.as_slice()).unwrap_or_default();
        let r = Recibida {
            nombre: w.name.trim_start_matches(PREFIJO_PETICION).to_string(),
            mensaje: w.wrapped_message,
            autorizacion: valor("authorization"),
            tipo_contenido: valor("content-type"),
        };
        log.lock().unwrap().push(r.clone());
        let Some(resp) = guion(&r) else {
            std::thread::sleep(Duration::from_secs(30)); // un servidor que no contesta
            return;
        };
        // `estado` 0: el servidor corta la conexión sin contestar (un reset).
        if resp.estado == 0 {
            return;
        }
        let cab = format!(
            "HTTP/1.1 {} X\r\nContent-Type: application/x-google-protobuf\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            resp.estado,
            resp.cuerpo.len()
        );
        if s.write_all(cab.as_bytes()).is_err() || s.write_all(&resp.cuerpo).is_err() {
            return;
        }
    }
}
