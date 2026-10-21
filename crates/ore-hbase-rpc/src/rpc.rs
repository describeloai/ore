//! **La conexion RPC con un RegionServer** (o con el servidor de `hbase:meta`): el preambulo, la cabecera de
//! conexion y las llamadas, de una en una. Una llamada en vuelo por conexion: la peticion y su respuesta van en
//! orden, y no hace falta demultiplexar por `call_id` (se comprueba que coincide, y si no es un error).
//!
//! Autenticacion **SIMPLE** (el cliente *afirma* un usuario, sin secreto) o **Kerberos** (SASL/GSSAPI, ficha 0072 §3.5 y
//! N6): el apreton va antes de la cabecera de conexion, y despues cada mensaje va envuelto en la capa que el servidor ofrece.

use crate::cancelacion::Cancelacion;
use crate::error::{Error, Resultado};
use crate::proto::{ConnectionHeader, RequestHeader, ResponseHeader, UserInformation};
use crate::sasl::{self, Capa, Contexto, Kerberos};
use prost::Message;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// El codigo de autenticacion SIMPLE del preambulo.
const AUTH_SIMPLE: u8 = 80;
/// El de SASL (Kerberos): el apreton sigue al preambulo.
const AUTH_SASL: u8 = 81;
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
    /// Cuantas veces se reintenta y cuanto se espera entre intentos.
    pub politica: Politica,
    /// Kerberos en vez de SIMPLE. El usuario de la cabecera sale del principal.
    pub kerberos: Option<Kerberos>,
}

/// Los reintentos: la espera crece como la del cliente de HBase (`hbase.client.pause` por 1, 2, 3, 5, 10, 20, 40,
/// 100, 100, 100, 100, 200, 300 y 300 en adelante) y se cuenta por intentos **seguidos sin avanzar**: cada fila
/// que llega los pone a cero. Con los valores por defecto (100 ms y 15) un RegionServer que tarda en volver tiene
/// unos dos minutos y medio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Politica {
    pub pausa: Duration,
    pub max_intentos: u32,
}

impl Default for Politica {
    fn default() -> Self {
        Politica {
            pausa: Duration::from_millis(100),
            max_intentos: 15,
        }
    }
}

impl Politica {
    const FACTORES: [u32; 13] = [1, 2, 3, 5, 10, 20, 40, 100, 100, 100, 100, 200, 300];

    /// Lo que se espera antes del intento `n` (el primero reintento es el 1).
    pub fn espera(&self, n: u32) -> Duration {
        let i = (n.max(1) as usize - 1).min(Self::FACTORES.len() - 1);
        self.pausa * Self::FACTORES[i]
    }
}

impl Default for Opciones {
    fn default() -> Self {
        Opciones {
            tiempo: Duration::from_secs(30),
            usuario: "ore".into(),
            traduccion: Vec::new(),
            politica: Politica::default(),
            kerberos: None,
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
    /// El tiempo que la llamada le dice al servidor que espera (`RequestHeader.timeout`), como hace el cliente de
    /// Java: con el, el servidor sabe cuando mandar un latido en vez de dejar al cliente sin respuesta.
    tiempo_ms: u32,
    /// Con Kerberos: el contexto y la capa acordada. Con `Nada` solo se autentico; con las otras, cada mensaje va
    /// envuelto.
    sasl: Option<(Box<dyn Contexto>, Capa)>,
}

/// El preambulo (`HBas`, version 0, autenticacion) seguido de la cabecera de conexion con su longitud.
pub fn apertura(usuario: &str) -> Vec<u8> {
    let mut m = b"HBas".to_vec();
    m.extend_from_slice(&[0, AUTH_SIMPLE]);
    let c = cabecera_de_conexion(usuario);
    m.extend_from_slice(&(c.len() as u32).to_be_bytes());
    m.extend_from_slice(&c);
    m
}

/// La `ConnectionHeader` (sin longitud ni preambulo): quien eres y a que servicio hablas.
fn cabecera_de_conexion(usuario: &str) -> Vec<u8> {
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
    h.encode_to_vec()
}

/// Una llamada armada: `[longitud][cabecera con su longitud][peticion con su longitud]`.
pub fn armar_llamada<P: Message>(id: u32, metodo: &str, peticion: &P, tiempo_ms: u32) -> Vec<u8> {
    let cab = RequestHeader {
        call_id: Some(id),
        trace_info: None,
        method_name: Some(metodo.to_string()),
        request_param: Some(true),
        cell_block_meta: None,
        priority: None,
        timeout: Some(tiempo_ms),
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
                no_reintentar: ex.do_not_retry.unwrap_or(false),
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
        Conexion::abrir_con(destino, o, None)
    }

    /// Como [`Conexion::abrir`], con el contexto de Kerberos dado (para probar el protocolo sin un KDC); sin el, se
    /// crea el de `o.kerberos`.
    fn abrir_con(
        destino: &Servidor,
        o: &Opciones,
        contexto: Option<Box<dyn Contexto>>,
    ) -> Resultado<Conexion> {
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
        // La lectura va por rodajas para poder mirar la cancelacion; el plazo de la llamada lo cuenta `llamar`.
        s.set_read_timeout(Some(Self::RODAJA.min(o.tiempo)))?;
        s.set_write_timeout(Some(o.tiempo))?;
        s.set_nodelay(true)?;
        let mut c = Conexion {
            s,
            siguiente: 0,
            destino: destino.clone(),
            tiempo_ms: u32::try_from(o.tiempo.as_millis()).unwrap_or(u32::MAX),
            sasl: None,
        };
        match &o.kerberos {
            None => c.s.write_all(&apertura(&o.usuario))?,
            Some(k) => {
                let ctx = match contexto {
                    Some(c) => c,
                    None => sasl::contexto(&destino.host, k)?,
                };
                let usuario = k.usuario().unwrap_or_else(|| o.usuario.clone());
                c.abrir_con_kerberos(ctx, &usuario, o.tiempo)?;
            }
        }
        Ok(c)
    }

    /// El preambulo de SASL, el apreton, y la cabecera de conexion por la capa acordada. **El nombre del servicio sale
    /// del nombre que el RegionServer anuncia**, no de a donde se conecta de verdad (§3.5, trampa 7).
    fn abrir_con_kerberos(
        &mut self,
        mut ctx: Box<dyn Contexto>,
        usuario: &str,
        tiempo: Duration,
    ) -> Resultado<()> {
        self.s.write_all(&[b'H', b'B', b'a', b's', 0, AUTH_SASL])?;
        let limite = Instant::now() + tiempo;
        let capa = self.apretar(ctx.as_mut(), limite)?;
        self.sasl = Some((ctx, capa));
        let cab = cabecera_de_conexion(usuario);
        let mut plano = (cab.len() as u32).to_be_bytes().to_vec();
        plano.extend_from_slice(&cab);
        self.escribir_mensaje(&plano)
    }

    /// El apreton SASL/GSSAPI. Cada mensaje del cliente es `[longitud][token]`; el servidor contesta `[estado][longitud]
    /// [token]` con estado 0, y si no, dos cadenas (clase y mensaje) de longitud de 2 bytes (§3.5, trampa 1).
    fn apretar(&mut self, ctx: &mut dyn Contexto, limite: Instant) -> Resultado<Capa> {
        let mut token = ctx
            .paso(None)?
            .ok_or_else(|| Error::Protocolo("Kerberos no dio un primer token".into()))?;
        loop {
            self.escribir_marco(&token)?;
            let estado = self.leer_u32(limite)?;
            if estado != 0 {
                let clase = self.leer_cadena(limite)?;
                let mensaje = self.leer_cadena(limite)?;
                return Err(Error::Autenticacion(format!(
                    "el servidor rechazo el apreton: {clase}: {mensaje}"
                )));
            }
            let n = self.leer_u32(limite)? as usize;
            if n > MAX_RESPUESTA {
                return Err(Error::Protocolo(format!(
                    "un reto SASL de {n} bytes: no parece un RegionServer"
                )));
            }
            let mut reto = vec![0u8; n];
            self.leer_exacto(&mut reto, limite, None)?;
            if !ctx.completo() {
                // Si este reto completa el contexto no hay nada que enviar, pero se envia igual un mensaje de
                // longitud 0: la respuesta del servidor a ese es la oferta de capas (trampa 2).
                token = ctx.paso(Some(&reto))?.unwrap_or_default();
                continue;
            }
            // La oferta de capas (RFC 4752): 4 bytes, envuelta. El servidor ofrece una sola (trampa 3).
            let oferta = ctx.desenvolver(&reto)?;
            if oferta.len() < 4 {
                return Err(Error::Protocolo(format!(
                    "la oferta de capas SASL trae {} bytes y no 4",
                    oferta.len()
                )));
            }
            let capa = Capa::de_oferta(oferta[0]).ok_or_else(|| {
                Error::Protocolo(format!(
                    "el servidor no ofrece ninguna capa ({:#04x})",
                    oferta[0]
                ))
            })?;
            // La eleccion, el mismo tamano de bufer y un `authzid` vacio: con el nombre del usuario lo rechaza. Y el
            // servidor NO contesta a este mensaje (trampa 4): lo siguiente es la cabecera de conexion.
            let mut eleccion = vec![capa as u8];
            eleccion.extend_from_slice(&oferta[1..4]);
            let w = ctx.envolver(false, &eleccion)?;
            self.escribir_marco(&w)?;
            return Ok(capa);
        }
    }

    fn escribir_marco(&mut self, cuerpo: &[u8]) -> Resultado<()> {
        let mut m = (cuerpo.len() as u32).to_be_bytes().to_vec();
        m.extend_from_slice(cuerpo);
        self.s.write_all(&m)?;
        Ok(())
    }

    fn leer_u32(&mut self, limite: Instant) -> Resultado<u32> {
        let mut b = [0u8; 4];
        self.leer_exacto(&mut b, limite, None)?;
        Ok(u32::from_be_bytes(b))
    }

    /// Una cadena UTF de Hadoop: longitud de 2 bytes y los bytes.
    fn leer_cadena(&mut self, limite: Instant) -> Resultado<String> {
        let mut l = [0u8; 2];
        self.leer_exacto(&mut l, limite, None)?;
        let mut b = vec![0u8; u16::from_be_bytes(l) as usize];
        self.leer_exacto(&mut b, limite, None)?;
        Ok(String::from_utf8_lossy(&b).into_owned())
    }

    /// Escribe un mensaje ya armado (`[longitud][cuerpo]`). Con integridad o confidencialidad **lo que se envuelve es el
    /// mensaje con su propia longitud**, y la trama lleva la longitud del envoltorio: envolver solo el cuerpo cuelga la
    /// conexion sin error (trampa 5).
    fn escribir_mensaje(&mut self, plano: &[u8]) -> Resultado<()> {
        match &mut self.sasl {
            Some((ctx, capa)) if capa.envuelve() => {
                let w = ctx.envolver(*capa == Capa::Confidencialidad, plano)?;
                self.escribir_marco(&w)
            }
            _ => {
                self.s.write_all(plano)?;
                Ok(())
            }
        }
    }

    /// El cuerpo de una respuesta tal como llego; si va envuelta se desenvuelve y se le quita la longitud propia.
    fn desenvolver_respuesta(&mut self, datos: Vec<u8>) -> Resultado<Vec<u8>> {
        match &mut self.sasl {
            Some((ctx, capa)) if capa.envuelve() => {
                let plano = ctx.desenvolver(&datos)?;
                if plano.len() < 4 {
                    return Err(Error::Protocolo(
                        "una respuesta envuelta sin su longitud".into(),
                    ));
                }
                Ok(plano[4..].to_vec())
            }
            _ => Ok(datos),
        }
    }

    /// A quien esta conectada.
    pub fn destino(&self) -> &Servidor {
        &self.destino
    }

    /// Cada cuanto mira la cancelacion quien espera una respuesta.
    pub const RODAJA: Duration = Duration::from_millis(100);

    /// Lee exactamente `buf.len()` bytes, en rodajas, hasta `limite` y mirando `cancelacion`.
    fn leer_exacto(
        &mut self,
        buf: &mut [u8],
        limite: Instant,
        cancelacion: Option<&Cancelacion>,
    ) -> Resultado<()> {
        let mut leidos = 0;
        while leidos < buf.len() {
            match self.s.read(&mut buf[leidos..]) {
                Ok(0) => return Err(Error::Io(ErrorKind::UnexpectedEof.into())),
                Ok(n) => leidos += n,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if cancelacion.is_some_and(Cancelacion::cancelada) {
                        return Err(Error::Cancelada);
                    }
                    if Instant::now() >= limite {
                        return Err(Error::Io(ErrorKind::TimedOut.into()));
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Una llamada: envia la peticion y espera su respuesta.
    pub fn llamar<P: Message, R: Message + Default>(
        &mut self,
        metodo: &str,
        peticion: &P,
    ) -> Resultado<R> {
        self.llamar_cancelable(metodo, peticion, None)
    }

    /// Como [`Conexion::llamar`], y para en cuanto `cancelacion` se active (en una rodaja, 100 ms).
    pub fn llamar_cancelable<P: Message, R: Message + Default>(
        &mut self,
        metodo: &str,
        peticion: &P,
        cancelacion: Option<&Cancelacion>,
    ) -> Resultado<R> {
        self.siguiente += 1;
        let id = self.siguiente;
        let llamada = armar_llamada(id, metodo, peticion, self.tiempo_ms);
        self.escribir_mensaje(&llamada)?;
        let limite = Instant::now() + Duration::from_millis(u64::from(self.tiempo_ms));
        let mut l = [0u8; 4];
        self.leer_exacto(&mut l, limite, cancelacion)?;
        let n = u32::from_be_bytes(l) as usize;
        if n > MAX_RESPUESTA {
            return Err(Error::Protocolo(format!(
                "una respuesta de {n} bytes: no parece un RegionServer"
            )));
        }
        let mut datos = vec![0u8; n];
        self.leer_exacto(&mut datos, limite, cancelacion)?;
        let datos = self.desenvolver_respuesta(datos)?;
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
    fn la_espera_crece_como_la_del_cliente_de_hbase_y_se_queda_en_300_veces_la_pausa() {
        let p = Politica::default();
        let en_ms = |n| p.espera(n).as_millis();
        assert_eq!(
            (1..=13).map(en_ms).collect::<Vec<_>>(),
            [
                100, 200, 300, 500, 1000, 2000, 4000, 10000, 10000, 10000, 10000, 20000, 30000
            ]
        );
        assert_eq!(en_ms(14), 30_000);
        assert_eq!(en_ms(15), 30_000);
        assert_eq!(
            en_ms(0),
            100,
            "el intento 0 no existe: se trata como el primero"
        );
        let suma: u128 = (1..=p.max_intentos).map(en_ms).sum();
        assert!(
            (150_000..170_000).contains(&suma),
            "unos dos minutos y medio: {suma} ms"
        );
    }

    /// Un servidor que acepta la conexion y no contesta nunca.
    fn mudo() -> Servidor {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let puerto = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (_s, _) = l.accept().unwrap();
            std::thread::sleep(Duration::from_secs(30));
        });
        Servidor {
            host: "127.0.0.1".into(),
            puerto,
        }
    }

    #[test]
    fn cancelar_corta_una_llamada_que_espera_y_no_espera_al_plazo() {
        let o = Opciones {
            tiempo: Duration::from_secs(20),
            ..Opciones::default()
        };
        let mut c = Conexion::abrir(&mudo(), &o).unwrap();
        let cancelacion = Cancelacion::nueva();
        let otra = cancelacion.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            otra.cancelar();
        });
        let t = Instant::now();
        let r: Resultado<GetRequest> =
            c.llamar_cancelable("Get", &GetRequest::default(), Some(&cancelacion));
        assert!(matches!(r, Err(Error::Cancelada)), "{r:?}");
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "tardo {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn sin_cancelar_el_plazo_de_la_llamada_se_cumple() {
        let o = Opciones {
            tiempo: Duration::from_millis(400),
            ..Opciones::default()
        };
        let mut c = Conexion::abrir(&mudo(), &o).unwrap();
        let t = Instant::now();
        let r: Resultado<GetRequest> = c.llamar("Get", &GetRequest::default());
        assert!(
            matches!(&r, Err(Error::Io(e)) if e.kind() == ErrorKind::TimedOut),
            "{r:?}"
        );
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "tardo {:?}",
            t.elapsed()
        );
    }

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
        let m = armar_llamada(7, "Get", &GetRequest::default(), 30_000);
        let n = u32::from_be_bytes([m[0], m[1], m[2], m[3]]) as usize;
        assert_eq!(m.len(), 4 + n);
        let mut cuerpo = &m[4..];
        let cab = RequestHeader::decode_length_delimited(&mut cuerpo).unwrap();
        assert_eq!(cab.call_id, Some(7));
        assert_eq!(cab.method_name.as_deref(), Some("Get"));
        assert_eq!(cab.request_param, Some(true));
        assert_eq!(cab.timeout, Some(30_000));
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

#[cfg(test)]
mod kerberos {
    //! El protocolo de SASL/GSSAPI, byte a byte, contra un servidor falso que lo habla como el de HBase (§3.5) y un
    //! contexto de juguete: lo que se comprueba es el orden de los mensajes, que se envuelve y que no.

    use super::*;
    use crate::proto::{GetRequest, GetResponse};
    use crate::sasl::prueba::{Juguete, destapar, tapar};
    use std::net::TcpListener;

    fn leer_n(s: &mut TcpStream, n: usize) -> Vec<u8> {
        let mut b = vec![0u8; n];
        s.read_exact(&mut b).unwrap();
        b
    }

    fn marco(s: &mut TcpStream) -> Vec<u8> {
        let l = leer_n(s, 4);
        leer_n(s, u32::from_be_bytes(l.try_into().unwrap()) as usize)
    }

    fn enviar(s: &mut TcpStream, cuerpo: &[u8]) {
        let mut m = (cuerpo.len() as u32).to_be_bytes().to_vec();
        m.extend_from_slice(cuerpo);
        s.write_all(&m).unwrap();
    }

    /// El servidor de HBase con Kerberos, que ofrece la capa `capa` (1, 2 o 4). Devuelve lo que vio.
    fn servidor(capa: u8) -> (Servidor, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let destino = Servidor {
            host: "127.0.0.1".into(),
            puerto: l.local_addr().unwrap().port(),
        };
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut visto = Vec::new();
            assert_eq!(leer_n(&mut s, 6), [b'H', b'B', b'a', b's', 0, 81]);
            visto.push("preambulo 81".to_string());
            assert_eq!(marco(&mut s), b"TOKEN-1");
            // estado 0, y el reto
            s.write_all(&0u32.to_be_bytes()).unwrap();
            enviar(&mut s, b"RETO");
            // el mensaje VACIO (trampa 2)
            assert_eq!(marco(&mut s), b"");
            visto.push("mensaje vacio".to_string());
            // la oferta de capas, envuelta
            s.write_all(&0u32.to_be_bytes()).unwrap();
            enviar(&mut s, &tapar(false, &[capa, 0, 0x10, 0]));
            // la eleccion: la oferta con la capa, envuelta y con el authzid vacio (trampa 3)
            let eleccion = destapar(&marco(&mut s)).unwrap();
            assert_eq!(eleccion, [capa, 0, 0x10, 0]);
            visto.push(format!("eleccion {capa}"));
            // y el servidor NO contesta (trampa 4); lo siguiente es la cabecera de conexion
            let envuelve = capa > 1;
            let mensaje = |s: &mut TcpStream| -> Vec<u8> {
                let m = marco(s);
                let plano = if envuelve {
                    destapar(&m).unwrap()
                } else {
                    let mut p = (m.len() as u32).to_be_bytes().to_vec();
                    p.extend_from_slice(&m);
                    p
                };
                // el mensaje envuelto lleva su propia longitud (trampa 5)
                let n = u32::from_be_bytes(plano[..4].try_into().unwrap()) as usize;
                assert_eq!(n, plano.len() - 4);
                plano[4..].to_vec()
            };
            let cab = ConnectionHeader::decode(mensaje(&mut s).as_slice()).unwrap();
            visto.push(format!("usuario {}", cab.user_info.unwrap().effective_user));
            let llamada = mensaje(&mut s);
            let mut cuerpo = llamada.as_slice();
            let rh = RequestHeader::decode_length_delimited(&mut cuerpo).unwrap();
            visto.push(format!("llamada {}", rh.method_name.unwrap()));
            let mut r = ResponseHeader {
                call_id: rh.call_id,
                exception: None,
                cell_block_meta: None,
            }
            .encode_length_delimited_to_vec();
            r.extend_from_slice(&GetResponse::default().encode_length_delimited_to_vec());
            let mut plano = (r.len() as u32).to_be_bytes().to_vec();
            plano.extend_from_slice(&r);
            if envuelve {
                enviar(&mut s, &tapar(capa == 4, &plano));
            } else {
                s.write_all(&plano).unwrap();
            }
            visto
        });
        (destino, h)
    }

    fn opciones() -> Opciones {
        Opciones {
            kerberos: Some(Kerberos {
                principal: Some("lector@ORE.TEST".into()),
                ..Kerberos::default()
            }),
            tiempo: Duration::from_secs(5),
            ..Opciones::default()
        }
    }

    #[test]
    fn el_apreton_y_las_llamadas_siguen_el_protocolo_en_cada_capa() {
        for capa in [1u8, 2, 4] {
            let (destino, h) = servidor(capa);
            let o = opciones();
            let mut c =
                Conexion::abrir_con(&destino, &o, Some(Box::new(Juguete::default()))).unwrap();
            let _: GetResponse = c.llamar("Get", &GetRequest::default()).unwrap();
            let visto = h.join().unwrap();
            assert_eq!(
                visto,
                [
                    "preambulo 81".to_string(),
                    "mensaje vacio".to_string(),
                    format!("eleccion {capa}"),
                    "usuario lector".to_string(),
                    "llamada Get".to_string(),
                ],
                "capa {capa}"
            );
        }
    }

    #[test]
    fn un_rechazo_del_servidor_llega_con_su_clase_y_su_mensaje_y_no_se_reintenta() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let destino = Servidor {
            host: "127.0.0.1".into(),
            puerto: l.local_addr().unwrap().port(),
        };
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            leer_n(&mut s, 6);
            marco(&mut s);
            // estado 1 y dos cadenas con longitud de 2 bytes
            s.write_all(&1u32.to_be_bytes()).unwrap();
            for t in ["javax.security.sasl.SaslException", "GSS initiate failed"] {
                s.write_all(&(t.len() as u16).to_be_bytes()).unwrap();
                s.write_all(t.as_bytes()).unwrap();
            }
        });
        let e = Conexion::abrir_con(&destino, &opciones(), Some(Box::new(Juguete::default())))
            .err()
            .unwrap();
        h.join().unwrap();
        assert!(
            e.to_string().contains("SaslException")
                && e.to_string().contains("GSS initiate failed"),
            "{e}"
        );
        assert_eq!(e.clase(), crate::Clase::Fatal);
    }

    #[test]
    fn kerberos_pedido_sin_el_feature_es_un_error_que_lo_dice() {
        #[cfg(not(feature = "kerberos"))]
        {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let destino = Servidor {
                host: "127.0.0.1".into(),
                puerto: l.local_addr().unwrap().port(),
            };
            let e = Conexion::abrir(&destino, &opciones()).err().unwrap();
            assert!(e.to_string().contains("kerberos"), "{e}");
        }
    }
}
