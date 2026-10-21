//! **La conexion RPC con un RegionServer** (o con el servidor de `hbase:meta`): el preambulo, la cabecera de
//! conexion y las llamadas, de una en una. Una llamada en vuelo por conexion: la peticion y su respuesta van en
//! orden, y no hace falta demultiplexar por `call_id` (se comprueba que coincide, y si no es un error).
//!
//! Autenticacion **SIMPLE** (el cliente *afirma* un usuario, sin secreto). Kerberos llega en N6 (ficha 0072 §7.7).

use crate::error::{Error, Resultado};
use crate::proto::{ConnectionHeader, RequestHeader, ResponseHeader, UserInformation};
use prost::Message;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// El codigo de autenticacion SIMPLE del preambulo.
const AUTH_SIMPLE: u8 = 80;
/// El servicio de lectura y escritura de datos.
const SERVICIO: &str = "ClientService";
/// Una respuesta mayor es basura o un servidor que no es HBase.
const MAX_RESPUESTA: usize = 256 << 20;

/// Como se conecta el cliente.
#[derive(Debug, Clone)]
pub struct Opciones {
    /// Tiempo maximo de conectar y de cada lectura o escritura del *socket*.
    pub tiempo: Duration,
    /// El usuario que el cliente afirma (autenticacion SIMPLE).
    pub usuario: String,
    /// **Traduccion de nombres** (ficha 0072, §7.3): el cliente se conecta al nombre que el RegionServer anuncia
    /// en ZooKeeper, que en un cluster real es un nombre interno que quien lee puede no resolver. Cada par dice
    /// «lo que anuncian» -> «a donde conectar de verdad».
    pub traduccion: Vec<(Servidor, Servidor)>,
}

impl Default for Opciones {
    fn default() -> Self {
        Opciones {
            tiempo: Duration::from_secs(30),
            usuario: "ore".into(),
            traduccion: Vec::new(),
        }
    }
}

impl Opciones {
    /// Lee `anunciado:puerto=real:puerto,otro:puerto=otro-real:puerto`.
    pub fn con_traduccion(mut self, spec: &str) -> Resultado<Opciones> {
        for par in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (de, a) = par.split_once('=').ok_or_else(|| {
                Error::Peticion(format!(
                    "la traduccion `{par}` no es `anunciado:puerto=real:puerto`"
                ))
            })?;
            self.traduccion
                .push((Servidor::analizar(de)?, Servidor::analizar(a)?));
        }
        Ok(self)
    }

    /// A donde hay que conectar para llegar a `s`.
    pub fn destino_de<'a>(&'a self, s: &'a Servidor) -> &'a Servidor {
        self.traduccion
            .iter()
            .find(|(de, _)| de == s)
            .map_or(s, |(_, a)| a)
    }
}

/// Un RegionServer (o el maestro): el nombre que ZooKeeper o `hbase:meta` anuncian, no el que uno quisiera.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Servidor {
    pub host: String,
    pub puerto: u16,
}

impl Servidor {
    /// `host:puerto`.
    pub fn analizar(s: &str) -> Resultado<Servidor> {
        let (h, p) = s
            .trim()
            .rsplit_once(':')
            .ok_or_else(|| Error::Peticion(format!("`{s}` no es host:puerto")))?;
        let puerto = p
            .parse::<u16>()
            .map_err(|_| Error::Peticion(format!("el puerto `{p}` de `{s}` no es un numero")))?;
        if h.is_empty() {
            return Err(Error::Peticion(format!("`{s}` no tiene host")));
        }
        Ok(Servidor {
            host: h.to_string(),
            puerto,
        })
    }
}

impl std::fmt::Display for Servidor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.puerto)
    }
}

/// Una conexion abierta.
pub struct Conexion {
    s: TcpStream,
    siguiente: u32,
    destino: Servidor,
}

/// El preambulo (`HBas`, version 0, autenticacion) seguido de la cabecera de conexion con su longitud.
pub fn apertura(usuario: &str) -> Vec<u8> {
    let mut m = b"HBas".to_vec();
    m.extend_from_slice(&[0, AUTH_SIMPLE]);
    let h = ConnectionHeader {
        user_info: Some(UserInformation {
            effective_user: usuario.to_string(),
            real_user: None,
        }),
        service_name: Some(SERVICIO.to_string()),
        cell_block_codec_class: None,
        cell_block_compressor_class: None,
        version_info: None,
    };
    let c = h.encode_to_vec();
    m.extend_from_slice(&(c.len() as u32).to_be_bytes());
    m.extend_from_slice(&c);
    m
}

/// Una llamada armada: `[longitud][cabecera con su longitud][peticion con su longitud]`.
pub fn armar_llamada<P: Message>(id: u32, metodo: &str, peticion: &P) -> Vec<u8> {
    let cab = RequestHeader {
        call_id: Some(id),
        trace_info: None,
        method_name: Some(metodo.to_string()),
        request_param: Some(true),
        cell_block_meta: None,
        priority: None,
        timeout: None,
    };
    let mut cuerpo = cab.encode_length_delimited_to_vec();
    cuerpo.extend_from_slice(&peticion.encode_length_delimited_to_vec());
    let mut m = (cuerpo.len() as u32).to_be_bytes().to_vec();
    m.extend_from_slice(&cuerpo);
    m
}

/// Una respuesta decodificada: su `call_id` y el mensaje, o la excepcion que HBase devolvio.
pub fn leer_respuesta<R: Message + Default>(mut datos: &[u8]) -> Resultado<(u32, Resultado<R>)> {
    let cab = ResponseHeader::decode_length_delimited(&mut datos)
        .map_err(|e| Error::Protocolo(format!("la cabecera de la respuesta no decodifica: {e}")))?;
    let id = cab.call_id.unwrap_or(0);
    if let Some(ex) = cab.exception {
        return Ok((
            id,
            Err(Error::Servidor {
                clase: ex.exception_class_name.unwrap_or_default(),
                mensaje: primera_linea(&ex.stack_trace.unwrap_or_default()),
            }),
        ));
    }
    let r = R::decode_length_delimited(&mut datos)
        .map_err(|e| Error::Protocolo(format!("la respuesta no decodifica: {e}")))?;
    Ok((id, Ok(r)))
}

/// La traza de Java ocupa decenas de lineas; el mensaje de la excepcion es la primera.
fn primera_linea(traza: &str) -> String {
    let l = traza.lines().next().unwrap_or("").trim();
    // «org.apache...NotServingRegionException: Region x is not online on y»: se queda lo que sigue a la clase.
    match l.split_once(": ") {
        Some((_, resto)) => resto.to_string(),
        None => l.to_string(),
    }
}

impl Conexion {
    /// Abre la conexion y envia el preambulo y la cabecera. HBase no contesta a la cabecera: el primer fallo
    /// (un usuario no admitido, un servidor que exige Kerberos) llega como respuesta a la primera llamada.
    pub fn abrir(destino: &Servidor, o: &Opciones) -> Resultado<Conexion> {
        let mut ultimo = None;
        let mut s = None;
        let real = o.destino_de(destino);
        for a in (real.host.as_str(), real.puerto).to_socket_addrs()? {
            match TcpStream::connect_timeout(&a, o.tiempo) {
                Ok(x) => {
                    s = Some(x);
                    break;
                }
                Err(e) => ultimo = Some(e),
            }
        }
        let s = s.ok_or_else(|| {
            ultimo.map_or_else(
                || Error::Peticion(format!("`{destino}` no resuelve")),
                Error::Io,
            )
        })?;
        s.set_read_timeout(Some(o.tiempo))?;
        s.set_write_timeout(Some(o.tiempo))?;
        s.set_nodelay(true)?;
        let mut c = Conexion {
            s,
            siguiente: 0,
            destino: destino.clone(),
        };
        c.s.write_all(&apertura(&o.usuario))?;
        Ok(c)
    }

    /// A quien esta conectada.
    pub fn destino(&self) -> &Servidor {
        &self.destino
    }

    /// Una llamada: envia la peticion y espera su respuesta.
    pub fn llamar<P: Message, R: Message + Default>(
        &mut self,
        metodo: &str,
        peticion: &P,
    ) -> Resultado<R> {
        self.siguiente += 1;
        let id = self.siguiente;
        self.s.write_all(&armar_llamada(id, metodo, peticion))?;
        let mut l = [0u8; 4];
        self.s.read_exact(&mut l)?;
        let n = u32::from_be_bytes(l) as usize;
        if n > MAX_RESPUESTA {
            return Err(Error::Protocolo(format!(
                "una respuesta de {n} bytes: no parece un RegionServer"
            )));
        }
        let mut datos = vec![0u8; n];
        self.s.read_exact(&mut datos)?;
        let (recibido, r) = leer_respuesta::<R>(&datos)?;
        if recibido != id {
            return Err(Error::Protocolo(format!(
                "la respuesta es de la llamada {recibido}, no de la {id}"
            )));
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{ExceptionResponse, GetRequest};

    #[test]
    fn la_traduccion_lleva_lo_anunciado_a_donde_se_conecta() {
        let o = Opciones::default()
            .con_traduccion("rs1.interno:16020=10.0.0.5:6020, rs2.interno:16020=10.0.0.6:6020")
            .unwrap();
        let a = Servidor {
            host: "rs2.interno".into(),
            puerto: 16020,
        };
        assert_eq!(o.destino_de(&a).to_string(), "10.0.0.6:6020");
        let otro = Servidor {
            host: "rs3.interno".into(),
            puerto: 16020,
        };
        assert_eq!(
            o.destino_de(&otro),
            &otro,
            "lo que no se traduce se conecta tal cual"
        );
        assert!(Opciones::default().con_traduccion("sin-igual").is_err());
        assert!(Opciones::default().con_traduccion("a:1=b").is_err());
    }

    #[test]
    fn la_apertura_lleva_preambulo_version_autenticacion_y_la_cabecera() {
        let a = apertura("lector");
        assert_eq!(&a[..6], b"HBas\x00\x50");
        let n = u32::from_be_bytes([a[6], a[7], a[8], a[9]]) as usize;
        assert_eq!(a.len(), 10 + n);
        let h = ConnectionHeader::decode(&a[10..]).unwrap();
        assert_eq!(h.user_info.unwrap().effective_user, "lector");
        assert_eq!(h.service_name.as_deref(), Some("ClientService"));
    }

    #[test]
    fn una_llamada_lleva_su_id_su_metodo_y_las_longitudes_de_protobuf() {
        let m = armar_llamada(7, "Get", &GetRequest::default());
        let n = u32::from_be_bytes([m[0], m[1], m[2], m[3]]) as usize;
        assert_eq!(m.len(), 4 + n);
        let mut cuerpo = &m[4..];
        let cab = RequestHeader::decode_length_delimited(&mut cuerpo).unwrap();
        assert_eq!(cab.call_id, Some(7));
        assert_eq!(cab.method_name.as_deref(), Some("Get"));
        assert_eq!(cab.request_param, Some(true));
    }

    #[test]
    fn una_excepcion_llega_con_su_clase_y_la_primera_linea_de_la_traza() {
        let cab = ResponseHeader {
            call_id: Some(3),
            exception: Some(ExceptionResponse {
                exception_class_name: Some("org.apache.hadoop.hbase.NotServingRegionException".into()),
                stack_trace: Some("org.apache.hadoop.hbase.NotServingRegionException: Region x is not online on y\n\tat a.b.C".into()),
                hostname: None,
                port: None,
                do_not_retry: None,
            }),
            cell_block_meta: None,
        };
        let (id, r) = leer_respuesta::<GetRequest>(&cab.encode_length_delimited_to_vec()).unwrap();
        assert_eq!(id, 3);
        let e = r.unwrap_err();
        assert_eq!(
            e.to_string(),
            "HBase: NotServingRegionException: Region x is not online on y"
        );
    }

    #[test]
    fn una_respuesta_truncada_es_un_error_de_protocolo_y_no_un_panico() {
        assert!(leer_respuesta::<GetRequest>(&[0x05, 0x08]).is_err());
        assert!(leer_respuesta::<GetRequest>(&[]).is_err());
    }
}
