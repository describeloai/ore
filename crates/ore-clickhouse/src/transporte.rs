//! **El transporte**: HTTP síncrono hacia ClickHouse, con el TLS del sistema, y **un vigilante** del flujo que no deja pasar un
//! error de mitad de respuesta como si fueran datos (ADR 0077 §3.7).
//!
//! # Por qué hace falta un vigilante
//!
//! ClickHouse manda la cabecera `200 OK` antes de ejecutar la consulta entera. Si falla a mitad, el estado ya no se puede cambiar:
//! el error llega **dentro del cuerpo**, pegado a los datos. Cómo, depende de la versión (medido, §3.7):
//!
//! | Versión | Cabecera | Cuerpo | Conexión |
//! |---|---|---|---|
//! | 26.4 | `X-ClickHouse-Exception-Tag: <16>` | `__exception__\r\n<ETIQUETA>\r\n<mensaje>\r\n<n> <ETIQUETA>\r\n__exception__\r\n` | *chunked* sin cerrar |
//! | 25.3, 25.8 | — | `__exception__\r\nCode: …` | *chunked* sin cerrar |
//! | 24.8 | — | el mensaje, sin marca | cierre limpio |
//!
//! El [`Vigia`] retiene los últimos bytes que lee antes de entregarlos, busca la marca (aunque llegue partida entre dos lecturas)
//! y, si la encuentra, entrega los datos que había **antes** y después el error. Al terminar, mira el final del flujo por si es
//! el mensaje de 24.8. Lo que un error de red corte sin marca se entrega como lo que es: un corte.

use crate::url::{Destino, analizar};
use ore_driver::{Codigo, Fallo};
use std::io::{self, Read};
use std::sync::Arc;
use std::time::Duration;

/// Lo que el vigilante retiene antes de entregar: más que la marca más larga y que un mensaje de error típico.
const RETENIDO: usize = 32 * 1024;
/// Cuánto de un mensaje de error se guarda.
const MAX_ERROR: usize = 64 * 1024;
/// Cuánto de un cuerpo pequeño (un catálogo, un `SELECT version()`) se lee entero.
const MAX_TEXTO: u64 = 16 * 1024 * 1024;
const MARCA: &[u8] = b"__exception__\r\n";

// ───────────────────────────── el error del servidor ─────────────────────────────

/// Un error que ClickHouse contestó: su código (`Code: 60`) y su texto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorServidor {
    pub codigo: u32,
    pub mensaje: String,
}

impl std::fmt::Display for ErrorServidor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.mensaje)
    }
}

impl std::error::Error for ErrorServidor {}

impl ErrorServidor {
    /// El código de `Code: 395. DB::Exception: …`, o 0 si el texto no lo trae.
    pub fn de_texto(texto: &str) -> ErrorServidor {
        let texto = texto.trim();
        let codigo = texto
            .find("Code: ")
            .map(|i| {
                texto[i + 6..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .and_then(|d| d.parse().ok())
            .unwrap_or(0);
        let mensaje: String = texto.chars().take(4000).collect();
        ErrorServidor { codigo, mensaje }
    }

    /// El error que va dentro del cuerpo: quita la marca, la etiqueta y el cierre si los hay.
    fn de_bloque(bloque: &[u8], etiqueta: Option<&[u8]>) -> ErrorServidor {
        let mut t = String::from_utf8_lossy(bloque).into_owned();
        if let Some(r) = t.strip_prefix("__exception__\r\n") {
            t = r.to_string();
        }
        if let Some(e) = etiqueta.and_then(|e| std::str::from_utf8(e).ok()) {
            if let Some(r) = t.strip_prefix(&format!("{e}\r\n")) {
                t = r.to_string();
            }
            // `\n<longitud> <ETIQUETA>\r\n__exception__\r\n` al final (medido en 26.4: el mensaje acaba en `\n`, no en `\r\n`).
            if let Some(i) = t.rfind(&format!(" {e}\r\n__exception__"))
                && let Some(j) = t[..i].rfind('\n')
            {
                t.truncate(j);
            }
        }
        ErrorServidor::de_texto(&t)
    }

    pub fn fallo(&self) -> Fallo {
        fallo_de_codigo(self.codigo, self.mensaje.clone())
    }
}

/// De un código de ClickHouse al [`Codigo`] del protocolo.
pub fn fallo_de_codigo(codigo: u32, mensaje: String) -> Fallo {
    let c = match codigo {
        // UNKNOWN_USER, WRONG_PASSWORD, REQUIRED_PASSWORD, AUTHENTICATION_FAILED, ACCESS_DENIED
        192 | 193 | 194 | 516 | 497 => Codigo::Credencial,
        // UNKNOWN_TABLE, UNKNOWN_DATABASE
        60 | 81 => Codigo::Objeto,
        // UNKNOWN_IDENTIFIER, NO_SUCH_COLUMN_IN_TABLE: la tabla está, su forma ya no es la declarada.
        47 | 16 => Codigo::Esquema,
        // TIMEOUT_EXCEEDED, SOCKET_TIMEOUT
        159 | 209 => Codigo::Tiempo,
        // SYNTAX_ERROR, BAD_ARGUMENTS, UNKNOWN_SETTING, READONLY, UNKNOWN_QUERY_PARAMETER, BAD_QUERY_PARAMETER,
        // ILLEGAL_TYPE_OF_ARGUMENT, TYPE_MISMATCH: un defecto de quien pidió.
        62 | 36 | 115 | 164 | 456 | 457 | 43 | 53 => Codigo::Operador,
        _ => Codigo::Origen,
    };
    Fallo::new(c, mensaje)
}

/// Un error de entrada y salida del flujo, como [`Fallo`]: el del servidor si lo es, y si no un corte.
pub fn fallo_de_io(e: &io::Error) -> Fallo {
    if let Some(s) = e.get_ref().and_then(|x| x.downcast_ref::<ErrorServidor>()) {
        return s.fallo();
    }
    let kind = e.kind();
    let tiempo = matches!(kind, io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock);
    Fallo::new(
        if tiempo {
            Codigo::Tiempo
        } else {
            Codigo::Conexion
        },
        format!("se cortó la respuesta de ClickHouse ({kind:?}): {e}"),
    )
}

// ───────────────────────────── el vigilante ─────────────────────────────

/// Un lector que entrega los datos y convierte en error lo que ClickHouse pegue a su final cuando falla a mitad.
pub struct Vigia<R> {
    dentro: R,
    etiqueta: Option<Vec<u8>>,
    /// Lo leído y aún no entregado.
    buf: Vec<u8>,
    /// Los primeros `listo` bytes de `buf` se pueden entregar.
    listo: usize,
    /// Dónde empieza, dentro de `buf`, el bloque de error.
    marca: Option<usize>,
    /// Hasta dónde de `buf` se ha buscado la marca.
    escaneado: usize,
    fin: bool,
    corte: Option<io::Error>,
    error: Option<ErrorServidor>,
}

impl<R: Read> Vigia<R> {
    pub fn nuevo(dentro: R, etiqueta: Option<Vec<u8>>) -> Vigia<R> {
        Vigia {
            dentro,
            etiqueta,
            buf: Vec::new(),
            listo: 0,
            marca: None,
            escaneado: 0,
            fin: false,
            corte: None,
            error: None,
        }
    }

    fn aguja(&self) -> Vec<u8> {
        match &self.etiqueta {
            Some(e) => [MARCA, e, b"\r\n"].concat(),
            // Sin etiqueta (25.x) la marca sola podría ser un dato: se pide también `Code: `.
            None => [MARCA, b"Code: "].concat(),
        }
    }

    fn llenar(&mut self) {
        let mut tmp = [0u8; 16 * 1024];
        match self.dentro.read(&mut tmp) {
            Ok(0) => {
                self.fin = true;
                self.al_terminar();
            }
            Ok(n) => {
                self.buf.extend_from_slice(&tmp[..n]);
                self.revisar();
            }
            Err(e) => {
                self.fin = true;
                self.corte = Some(e);
                self.al_terminar();
            }
        }
    }

    fn revisar(&mut self) {
        if self.marca.is_none() {
            let aguja = self.aguja();
            let ini = self.escaneado.saturating_sub(aguja.len() - 1);
            if let Some(i) = buscar(&self.buf[ini..], &aguja) {
                self.marca = Some(ini + i);
            }
            self.escaneado = self.buf.len();
        }
        match self.marca {
            // Lo de antes de la marca son datos buenos; lo de después es el mensaje, que se sigue leyendo hasta el final.
            Some(p) => {
                self.listo = p;
                if self.buf.len() - p > MAX_ERROR {
                    self.fin = true;
                    self.cerrar_error(p);
                }
            }
            None => self.listo = self.buf.len().saturating_sub(RETENIDO).max(self.listo),
        }
    }

    fn al_terminar(&mut self) {
        if let Some(p) = self.marca {
            self.cerrar_error(p);
        } else if let Some(p) = mensaje_final(&self.buf) {
            // 24.8: el mensaje pegado al final, sin marca.
            self.marca = Some(p);
            self.cerrar_error(p);
        } else {
            self.listo = self.buf.len();
        }
    }

    fn cerrar_error(&mut self, p: usize) {
        self.listo = p;
        self.error = Some(ErrorServidor::de_bloque(
            &self.buf[p..],
            self.etiqueta.as_deref(),
        ));
        // El *chunked* sin cerrar que sigue a un error es lo esperado: el error del servidor manda.
        self.corte = None;
    }
}

impl<R: Read> Read for Vigia<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.listo > 0 {
                let n = self.listo.min(out.len());
                out[..n].copy_from_slice(&self.buf[..n]);
                self.buf.drain(..n);
                self.listo -= n;
                self.escaneado = self.escaneado.saturating_sub(n);
                if let Some(m) = self.marca.as_mut() {
                    *m -= n;
                }
                return Ok(n);
            }
            if let Some(e) = self.error.take() {
                self.buf.clear();
                return Err(io::Error::other(e));
            }
            if let Some(e) = self.corte.take() {
                return Err(e);
            }
            if self.fin {
                return Ok(0);
            }
            self.llenar();
        }
    }
}

fn buscar(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w == n)
}

/// 24.8: el flujo termina con `Code: 395. DB::Exception: … (version 24.8…)\n` sin ninguna marca. Devuelve dónde empieza.
fn mensaje_final(buf: &[u8]) -> Option<usize> {
    if !buf.ends_with(b"\n") {
        return None;
    }
    let ventana = buf.len().saturating_sub(RETENIDO);
    let mut hasta = buf.len();
    while let Some(i) = buf[ventana..hasta].windows(6).rposition(|w| w == b"Code: ") {
        let p = ventana + i;
        let resto = &buf[p + 6..];
        let digitos = resto.iter().take_while(|b| b.is_ascii_digit()).count();
        if digitos > 0
            && resto[digitos..].starts_with(b". DB::Exception: ")
            && buscar(resto, b"(version ").is_some()
        {
            return Some(p);
        }
        hasta = p;
    }
    None
}

// ───────────────────────────── los parámetros ─────────────────────────────

/// El valor de un parámetro del servidor (`{nombre:Tipo}`).
///
/// En el cable el valor va en el formato de texto de ClickHouse (`TabSeparated`): la barra, el tabulador y el salto de línea se
/// escapan y `\N` es el nulo. **Se escapa aquí y no en quien llama**: un valor sin escapar no da error, da *otro valor*
/// (medido: `\N` en un `String` llegaba como dos bytes menos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param<'a> {
    Texto(&'a str),
    Nulo,
}

impl Param<'_> {
    fn en_el_cable(&self) -> String {
        match self {
            Param::Nulo => "\\N".to_string(),
            Param::Texto(v) => escapar_texto(v),
        }
    }
}

/// El texto como lo lee ClickHouse en un parámetro: la barra, el tabulador, el salto de línea, el retorno y el NUL.
pub fn escapar_texto(v: &str) -> String {
    let mut s = String::with_capacity(v.len() + 8);
    for c in v.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '\t' => s.push_str("\\t"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\0' => s.push_str("\\0"),
            c => s.push(c),
        }
    }
    s
}

// ───────────────────────────── el cliente ─────────────────────────────

/// El flujo de una respuesta: se lee como cualquier `Read`, y un error del servidor sale como error de lectura.
pub type Flujo = Vigia<Box<dyn Read + Send + Sync + 'static>>;

/// Una conexión (en HTTP no hay sesión: es el agente, con su *pool*, y las credenciales).
pub struct Cliente {
    destino: Destino,
    agente: ureq::Agent,
    autorizacion: Option<String>,
}

impl Cliente {
    pub fn abrir(url: &str) -> Result<Cliente, Fallo> {
        let destino = analizar(url).map_err(Fallo::operador)?;
        let agente = agente(None)?;
        let autorizacion = match (&destino.usuario, &destino.clave) {
            (Some(u), Some(c)) => Some(format!("Basic {}", base64(format!("{u}:{c}").as_bytes()))),
            _ => None,
        };
        Ok(Cliente {
            destino,
            agente,
            autorizacion,
        })
    }

    /// El mismo cliente con un **tope para toda la petición** (conectar, enviar y leer el cuerpo entero): lo que `timeoutMs` pide, o
    /// el `KILL` de quien cancela, que no puede esperar. Pasado el tope, la conexión se cierra y el servidor cancela la consulta.
    pub fn con_tope(&self, tope: Duration) -> Result<Cliente, Fallo> {
        Ok(Cliente {
            destino: self.destino.clone(),
            agente: agente(Some(tope))?,
            autorizacion: self.autorizacion.clone(),
        })
    }

    pub fn destino(&self) -> &Destino {
        &self.destino
    }

    /// La petición: la dirección, la base, el formato, los parámetros del servidor (`param_*`) y los ajustes.
    fn solicitud(
        &self,
        formato: &str,
        params: &[(&str, Param)],
        ajustes: &[(&str, &str)],
        query_id: Option<&str>,
    ) -> ureq::Request {
        let mut r = self.agente.post(&self.destino.direccion());
        if let Some(b) = &self.destino.base {
            r = r.query("database", b);
        }
        r = r.query("default_format", formato);
        if let Some(q) = query_id {
            r = r.query("query_id", q);
        }
        for (k, v) in ajustes {
            r = r.query(k, v);
        }
        for (k, v) in params {
            r = r.query(&format!("param_{k}"), &v.en_el_cable());
        }
        match (&self.autorizacion, &self.destino.usuario) {
            (Some(a), _) => r = r.set("Authorization", a),
            (None, Some(u)) => r = r.set("X-ClickHouse-User", u),
            _ => {}
        }
        r
    }

    /// Manda `sql` y devuelve el cuerpo como un flujo vigilado.
    pub fn flujo(
        &self,
        sql: &str,
        formato: &str,
        params: &[(&str, Param)],
        ajustes: &[(&str, &str)],
        query_id: Option<&str>,
    ) -> Result<Flujo, Fallo> {
        let r = self.solicitud(formato, params, ajustes, query_id);
        match r.send_string(sql) {
            Ok(resp) => {
                let etiqueta = resp
                    .header("X-ClickHouse-Exception-Tag")
                    .map(|t| t.as_bytes().to_vec());
                Ok(Vigia::nuevo(resp.into_reader(), etiqueta))
            }
            Err(ureq::Error::Status(estado, resp)) => Err(error_http(estado, resp)),
            Err(ureq::Error::Transport(t)) => Err(self.fallo_de_red(&t)),
        }
    }

    /// Una consulta cuya respuesta cabe en memoria (un catálogo, `SELECT version()`): el cuerpo en `TabSeparated`.
    pub fn texto(&self, sql: &str) -> Result<String, Fallo> {
        let mut f = self.flujo(sql, "TabSeparated", &[], &[], None)?;
        let mut s = String::new();
        (&mut f)
            .take(MAX_TEXTO)
            .read_to_string(&mut s)
            .map_err(|e| fallo_de_io(&e))?;
        Ok(s)
    }

    /// Como [`Cliente::texto`], con parámetros del servidor.
    pub fn texto_con(&self, sql: &str, params: &[(&str, Param)]) -> Result<String, Fallo> {
        let mut f = self.flujo(sql, "TabSeparated", params, &[], None)?;
        let mut s = String::new();
        (&mut f)
            .take(MAX_TEXTO)
            .read_to_string(&mut s)
            .map_err(|e| fallo_de_io(&e))?;
        Ok(s)
    }

    /// Una orden que no devuelve nada que importe (`CREATE`, `INSERT`).
    pub fn ejecutar(&self, sql: &str) -> Result<(), Fallo> {
        self.texto(sql).map(|_| ())
    }

    fn fallo_de_red(&self, t: &ureq::Transport) -> Fallo {
        let d = &self.destino;
        let texto = t.to_string();
        // Un tope vencido llega como `TimedOut` (o, en Windows, como el error 10060 sin ese texto): es `tiempo`, no `conexion`.
        let tiempo = std::error::Error::source(t)
            .and_then(|e| e.downcast_ref::<io::Error>())
            .is_some_and(|e| {
                matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                )
            })
            || texto.to_ascii_lowercase().contains("timed out")
            || texto.contains("os error 10060");
        Fallo::new(
            if tiempo {
                Codigo::Tiempo
            } else {
                Codigo::Conexion
            },
            format!(
                "no se llega a ClickHouse en {}:{}: {texto}",
                d.host, d.puerto
            ),
        )
    }
}

/// El agente HTTP: TLS del sistema con mínimo 1.2 **explícito** (nada de lo que llega por la URL apaga la verificación del
/// certificado ni del nombre), sin redirecciones (llevarían la cabecera `Authorization` a donde dijera el servidor) y, si se pide,
/// con un tope para toda la petición.
fn agente(tope: Option<Duration>) -> Result<ureq::Agent, Fallo> {
    let mut tls = native_tls::TlsConnector::builder();
    tls.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
    let tls = tls
        .build()
        .map_err(|e| Fallo::origen(format!("no se pudo crear el contexto de TLS: {e}")))?;
    let mut b = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .tls_connector(Arc::new(tls))
        .redirects(0);
    if let Some(t) = tope {
        b = b.timeout(t);
    }
    Ok(b.build())
}

/// Un error de **antes** de empezar el flujo: el estado HTTP y `X-ClickHouse-Exception-Code`, con el texto en el cuerpo.
fn error_http(estado: u16, resp: ureq::Response) -> Fallo {
    let cabecera: Option<u32> = resp
        .header("X-ClickHouse-Exception-Code")
        .and_then(|c| c.trim().parse().ok());
    let mut cuerpo = Vec::new();
    let _ = resp
        .into_reader()
        .take(MAX_ERROR as u64)
        .read_to_end(&mut cuerpo);
    let texto = String::from_utf8_lossy(&cuerpo).into_owned();
    let mut e = ErrorServidor::de_texto(&texto);
    if e.codigo == 0 {
        e.codigo = cabecera.unwrap_or(0);
    }
    if e.mensaje.is_empty() {
        e.mensaje = format!("ClickHouse contestó HTTP {estado}");
    }
    if e.codigo == 0 && matches!(estado, 401 | 403) {
        return Fallo::new(Codigo::Credencial, e.mensaje);
    }
    e.fallo()
}

fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for t in b.chunks(3) {
        let n = (t[0] as u32) << 16
            | (*t.get(1).unwrap_or(&0) as u32) << 8
            | *t.get(2).unwrap_or(&0) as u32;
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        s.push(if t.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if t.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

#[cfg(test)]
mod pruebas;
