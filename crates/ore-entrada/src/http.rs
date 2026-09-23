//! HTTP/1.1, con la biblioteca estándar y nada más.
//!
//! # Por qué a mano
//!
//! Porque lo que hace falta cabe: una línea de petición, unas cabeceras, un
//! cuerpo acotado y una respuesta que siempre es JSON. Traer un servidor
//! completo metería un planificador asíncrono y su árbol entero en **el proceso
//! que mira a internet**, que es justo donde este repositorio lleva catorce
//! crates sin meter nada.
//!
//! No se sirven ficheros, no hay plantillas, no hay sesiones y no hay
//! `keep-alive`. Cada conexión atiende **una** petición y se cierra: es más
//! lento y quita una clase entera de errores —la de dos peticiones que se
//! solapan en el mismo flujo— por un precio que en un plano de control no se
//! nota.
//!
//! # Y una respuesta que no termina (0037 ②)
//!
//! Lo anterior sigue valiendo para **una petición y su respuesta**. Pero un
//! editor que quiere saber lo que pasa mientras pasa no puede preguntar una vez
//! por cosa: medido con el servicio de lenguaje de Monaco, teclear 32
//! caracteres son **296 mensajes en 3,5 s** —84 por segundo, 9,2 por
//! pulsación—, y una petición por mensaje serían 296 conexiones contra un techo
//! de [`CONEXIONES`]. Escribir una línea dejaría al inquilino sin plazas.
//!
//! Así que un manejador puede devolver, en vez de una [`Respuesta`], un
//! [`Flujo`]: la respuesta se abre, se trocea (`transfer-encoding: chunked`) y
//! **sigue escribiendo eventos** hasta que el que emite termina o el que lee se
//! va. No es un websocket a propósito —no hay saludo que firmar, ni marcos, ni
//! máscara, y el sentido de vuelta viaja como lo que ya viaja: una petición con
//! su cuerpo—.
//!
//! ⛔ Y **no comparte presupuesto con las peticiones**. Un flujo dura minutos;
//!   una petición, milisegundos. Contarlos juntos significaría que unos
//!   editores abiertos dejan al plano de control sin conexiones para trabajar,
//!   que es exactamente el fallo que [`CONEXIONES`] existe para no tener. Son
//!   dos techos, [`CONEXIONES`] y [`FLUJOS`], y la plaza **se cambia** —no se
//!   suma— en cuanto la respuesta resulta ser un flujo.
//!
//! # Los tres límites, y por qué son parte del contrato
//!
//! Un servidor sin límites no es un servidor: es una forma de quedarse sin
//! recursos a petición de cualquiera.
//!
//! - **el cuerpo** está acotado a [`CUERPO_MAXIMO`]. Sin esto, un `POST` con un
//!   `Content-Length` enorme reserva memoria a voluntad de quien llama;
//! - **la conexión** tiene plazo de lectura y de escritura. Sin esto, un
//!   cliente que abre y no habla se queda con un hilo para siempre;
//! - **las conexiones a la vez** están contadas, y la que sobra recibe un `503`
//!   inmediato. Sin esto, cada socket es un hilo y el techo lo pone la máquina.

use ore_core::json::Json;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Un cuerpo más grande que esto se rechaza sin leerlo. Las respuestas de una
/// cola de decisiones son kilobytes; un megabyte ya es holgura.
pub const CUERPO_MAXIMO: usize = 1 << 20;

/// Cuántas conexiones se atienden a la vez. Cada una es un hilo.
const CONEXIONES: usize = 64;

/// Cuántos flujos abiertos a la vez, **aparte** de las peticiones. La mitad:
/// un flujo es un hilo parado casi todo el tiempo, pero es un hilo, y quien
/// los abre es un editor por persona y repositorio, no cada clic.
const FLUJOS: usize = 32;

/// Lo que se espera a que un cliente hable, y a que lea.
const PLAZO: Duration = Duration::from_secs(30);

/// La cabecera más larga que se acepta, y cuántas.
const CABECERA_MAXIMA: usize = 8 * 1024;
const CABECERAS_MAXIMAS: usize = 64;

pub struct Peticion {
    pub metodo: String,
    /// El camino, ya sin la cadena de consulta.
    pub ruta: String,
    /// Los nombres llegan **en minúsculas**: HTTP no distingue mayúsculas en
    /// una cabecera y quien las lee no debería tener que acordarse.
    pub cabeceras: BTreeMap<String, String>,
    pub cuerpo: String,
}

impl Peticion {
    /// Los segmentos del camino, sin los vacíos. `/paquetes/ventas/decisiones`
    /// da `["paquetes", "ventas", "decisiones"]`.
    pub fn segmentos(&self) -> Vec<&str> {
        self.ruta.split('/').filter(|s| !s.is_empty()).collect()
    }
}

pub struct Respuesta {
    pub codigo: u16,
    pub cuerpo: Json,
}

impl Respuesta {
    pub fn ok(cuerpo: Json) -> Respuesta {
        Respuesta {
            codigo: 200,
            cuerpo,
        }
    }

    /// Hecho, y nada que decir (`204`): el cuerpo no se escribe.
    pub fn sin_contenido() -> Respuesta {
        Respuesta {
            codigo: 204,
            cuerpo: Json::obj([]),
        }
    }

    pub fn creado(cuerpo: Json) -> Respuesta {
        Respuesta {
            codigo: 201,
            cuerpo,
        }
    }

    /// Un error, **con su motivo escrito**. Un cuerpo vacío obliga a quien
    /// llama a adivinar, y adivinar acaba en reintentos que no pueden funcionar.
    pub fn error(codigo: u16, motivo: impl Into<String>) -> Respuesta {
        Respuesta {
            codigo,
            cuerpo: Json::obj([("error", Json::s(motivo.into()))]),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// UNA RESPUESTA QUE NO TERMINA
// ═══════════════════════════════════════════════════════════════════════════

/// Lo que un manejador contesta: una respuesta, o un flujo de eventos.
pub enum Salida {
    Una(Respuesta),
    Flujo(Flujo),
}

impl From<Respuesta> for Salida {
    fn from(r: Respuesta) -> Salida {
        Salida::Una(r)
    }
}

/// Una respuesta abierta: el tipo que declara y quién escribe dentro.
///
/// ⭐ Lo que decide **cuándo termina** es la propia función: vuelve cuando no
///   tiene más que decir, o en cuanto [`Emisor::evento`] devuelve `false`
///   —que es como se entera de que el que leía se fue—.
pub struct Flujo {
    /// `text/event-stream` para eventos; cualquier otro tipo también vale.
    pub tipo: &'static str,
    pub escribir: Box<dyn FnOnce(&mut Emisor<'_>) + Send>,
}

/// Por donde se escribe un flujo. Cada evento es un trozo y sale **ya**.
pub struct Emisor<'a> {
    flujo: &'a mut TcpStream,
}

impl Emisor<'_> {
    /// Un evento con su nombre, su número y su dato.
    ///
    /// El número viaja como `id:`, que es lo que un `EventSource` devuelve en
    /// `last-event-id` al reconectar: **así se retoma donde se dejó**, y hace
    /// falta, porque un balanceador corta la conexión cada tanto y reconectar
    /// sin saber por dónde ibas es repetir o perderse cosas.
    ///
    /// `false` si el que leía se fue: **hay que parar**.
    pub fn evento(&mut self, nombre: &str, id: Option<u64>, dato: &Json) -> bool {
        let mut t = String::new();
        if let Some(i) = id {
            t.push_str(&format!("id: {i}\n"));
        }
        // `jcs()` es UNA línea, que es justo lo que `data:` admite.
        t.push_str(&format!("event: {nombre}\ndata: {}\n\n", dato.jcs()));
        self.trozo(&t)
    }

    /// Un comentario: no lo ve quien lee, pero mueve bytes. Sirve para saber
    /// que el otro lado sigue ahí sin inventarse un evento que no pasó.
    pub fn latido(&mut self) -> bool {
        self.trozo(": latido\n\n")
    }

    fn trozo(&mut self, t: &str) -> bool {
        let cabeza = format!("{:x}\r\n", t.len());
        self.flujo.write_all(cabeza.as_bytes()).is_ok()
            && self.flujo.write_all(t.as_bytes()).is_ok()
            && self.flujo.write_all(b"\r\n").is_ok()
            && self.flujo.flush().is_ok()
    }
}

fn texto(codigo: u16) -> &'static str {
    match codigo {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        422 => "Unprocessable Content",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// Escucha y atiende hasta que la mate alguien. No vuelve.
pub fn servir<F>(escucha: TcpListener, manejador: F) -> std::io::Result<()>
where
    F: Fn(&Peticion) -> Respuesta + Send + Sync + 'static,
{
    servir_con_flujos(escucha, move |p| Salida::Una(manejador(p)))
}

/// Como [`servir`], pero el manejador puede contestar con un [`Flujo`].
pub fn servir_con_flujos<F>(escucha: TcpListener, manejador: F) -> std::io::Result<()>
where
    F: Fn(&Peticion) -> Salida + Send + Sync + 'static,
{
    let manejador = Arc::new(manejador);
    let vivas = Arc::new(AtomicUsize::new(0));
    let abiertos = Arc::new(AtomicUsize::new(0));

    for flujo in escucha.incoming() {
        let Ok(flujo) = flujo else { continue };
        let _ = flujo.set_read_timeout(Some(PLAZO));
        let _ = flujo.set_write_timeout(Some(PLAZO));

        if vivas.load(Ordering::Relaxed) >= CONEXIONES {
            // Decirlo y cerrar. Encolar en silencio es peor: quien llama no
            // distingue «lento» de «colgado» y reintenta encima.
            let mut f = flujo;
            responder(
                &mut f,
                &Respuesta::error(503, "servidor al límite de conexiones"),
            );
            continue;
        }

        vivas.fetch_add(1, Ordering::Relaxed);
        let manejador = Arc::clone(&manejador);
        let vivas_hilo = Arc::clone(&vivas);
        let abiertos_hilo = Arc::clone(&abiertos);
        let _ = std::thread::Builder::new()
            .name("ore-serve".into())
            .spawn(move || {
                // ⛔⛔ EL DESCUENTO VA EN UN `Drop`, Y NO ES ESTILO.
                //
                //   Estaba escrito como una línea DESPUÉS de atender, y esa línea
                //   **no corre si el manejador entra en pánico**. Un pánico mata
                //   sólo su hilo —el proceso sigue— pero deja el contador inflado,
                //   y `CONEXIONES` es un techo: **64 pánicos y este servidor deja
                //   de aceptar nada, para siempre, sin decir por qué**.
                //
                //   Medido el 2026-09-08 con un pánico de verdad: un `f.get(4)`
                //   sobre una consulta de cuatro columnas. Un fallo de programación
                //   se habría convertido en una denegación de servicio en el plano
                //   que administra personas.
                //
                // ⭐ Es la misma figura que `ore-serve/git.rs` usa para el préstamo
                //   del repositorio: lo que hay que deshacer pase lo que pase se
                //   deshace en `Drop`, no en la última línea del camino feliz.
                let plaza = Viva(vivas_hilo);
                atender(flujo, manejador.as_ref(), plaza, abiertos_hilo);
            });
    }
    Ok(())
}

/// Descuenta una conexión viva al salir, **también si el hilo entra en pánico**.
struct Viva(Arc<AtomicUsize>);

impl Drop for Viva {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn atender<F>(mut flujo: TcpStream, manejador: &F, plaza: Viva, abiertos: Arc<AtomicUsize>)
where
    F: Fn(&Peticion) -> Salida,
{
    let salida = match leer(&mut flujo) {
        Ok(p) => manejador(&p),
        Err(r) => Salida::Una(r),
    };
    match salida {
        Salida::Una(r) => responder(&mut flujo, &r),
        Salida::Flujo(f) => {
            if abiertos.load(Ordering::Relaxed) >= FLUJOS {
                responder(
                    &mut flujo,
                    &Respuesta::error(503, "servidor al límite de flujos abiertos"),
                );
                return;
            }
            abiertos.fetch_add(1, Ordering::Relaxed);
            let _abierto = Viva(abiertos);
            // ⭐ LA PLAZA SE CAMBIA, NO SE SUMA. A partir de aquí esto ya no es
            //   una petición en curso: es un flujo abierto, y cuenta en el techo
            //   de los flujos. Soltarla es lo que impide que unos cuantos
            //   editores abiertos dejen al plano de control sin conexiones.
            drop(plaza);
            emitir(&mut flujo, f);
        }
    }
}

/// Abre la respuesta, deja escribir dentro y la cierra.
fn emitir(flujo: &mut TcpStream, f: Flujo) {
    let cabeza = format!(
        "HTTP/1.1 200 OK\r\n\
         content-type: {}\r\n\
         transfer-encoding: chunked\r\n\
         connection: close\r\n\
         cache-control: no-store\r\n\
         x-content-type-options: nosniff\r\n\
         x-accel-buffering: no\r\n\
         \r\n",
        f.tipo
    );
    // Troceada y NO `content-length`: lo que se va a escribir no se sabe aún.
    // `x-accel-buffering` es para el que haya en medio: que no junte trozos,
    // porque un evento que llega tarde es un evento que no sirve.
    if flujo.write_all(cabeza.as_bytes()).is_err() || flujo.flush().is_err() {
        return;
    }
    let mut emisor = Emisor { flujo };
    (f.escribir)(&mut emisor);
    // El trozo vacío es el punto final. Si el otro lado ya se fue, da igual.
    let _ = flujo.write_all(b"0\r\n\r\n");
    let _ = flujo.flush();
}

fn leer(flujo: &mut TcpStream) -> Result<Peticion, Respuesta> {
    let mut lector = BufReader::new(
        flujo
            .try_clone()
            .map_err(|_| Respuesta::error(500, "no se pudo leer la conexión"))?,
    );

    let mut linea = String::new();
    let n = lector
        .by_ref()
        .take(CABECERA_MAXIMA as u64)
        .read_line(&mut linea)
        .map_err(|_| Respuesta::error(400, "petición ilegible"))?;
    if n == 0 {
        return Err(Respuesta::error(400, "petición vacía"));
    }

    let mut partes = linea.split_whitespace();
    let metodo = partes
        .next()
        .ok_or_else(|| Respuesta::error(400, "sin método"))?
        .to_string();
    let destino = partes
        .next()
        .ok_or_else(|| Respuesta::error(400, "sin destino"))?;
    // La cadena de consulta se descarta a propósito: **ningún dato entra por la
    // URL**. Una URL viaja en registros de acceso y en cabeceras `Referer`, y
    // aquí se manejan nombres de fuentes y decisiones de gobierno.
    let ruta = destino.split(['?', '#']).next().unwrap_or("/").to_string();

    let mut cabeceras = BTreeMap::new();
    for _ in 0..CABECERAS_MAXIMAS {
        let mut l = String::new();
        let n = lector
            .by_ref()
            .take(CABECERA_MAXIMA as u64)
            .read_line(&mut l)
            .map_err(|_| Respuesta::error(400, "cabecera ilegible"))?;
        if n == 0 || l.trim_end().is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            cabeceras.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    let largo: usize = cabeceras
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if largo > CUERPO_MAXIMO {
        return Err(Respuesta::error(413, "el cuerpo excede el máximo"));
    }

    let mut bruto = vec![0u8; largo];
    if largo > 0 {
        lector
            .read_exact(&mut bruto)
            .map_err(|_| Respuesta::error(400, "el cuerpo no llegó entero"))?;
    }
    let cuerpo =
        String::from_utf8(bruto).map_err(|_| Respuesta::error(400, "el cuerpo no es UTF-8"))?;

    Ok(Peticion {
        metodo,
        ruta,
        cabeceras,
        cuerpo,
    })
}

fn responder(flujo: &mut TcpStream, r: &Respuesta) {
    // Un 204 no lleva cuerpo, por definición: lo que trajera se calla.
    let cuerpo = if r.codigo == 204 {
        String::new()
    } else {
        r.cuerpo.jcs()
    };
    let cabeza = format!(
        "HTTP/1.1 {} {}\r\n\
         content-type: application/json\r\n\
         content-length: {}\r\n\
         connection: close\r\n\
         cache-control: no-store\r\n\
         x-content-type-options: nosniff\r\n\
         \r\n",
        r.codigo,
        texto(r.codigo),
        cuerpo.len()
    );
    let _ = flujo.write_all(cabeza.as_bytes());
    let _ = flujo.write_all(cuerpo.as_bytes());
    let _ = flujo.flush();
}

// ═══════════════════════════════════════════════════════════════════════════
// Y EL OTRO LADO: PEDIR, no sólo atender
// ═══════════════════════════════════════════════════════════════════════════
//
// ⭐⭐ Esto entra el 2026-09-09 y hay que justificarlo, porque la cabecera de
// este fichero presume de lo contrario: *«catorce crates sin meter nada»*.
//
// Lo que lo hace admisible no es que quepa —que cabe, son cuarenta líneas— sino
// **a dónde puede llamar**: HTTP PLANO, sin TLS y sin resolución de nada que no
// sea un nombre de servicio del clúster. Así que `ore-serve` gana la capacidad
// de hablar con el custodio y **no gana la de hablar con internet**.
//
// ⇒ La afirmación que sostenía a `ore-serve` —«el binario no lleva cliente
//   TLS»— sigue en pie palabra por palabra. Un cliente que sí lo llevara habría
//   abierto la puerta que tres cerraduras estaban cuidando.
//
// ⚠️ Y por eso no hay `https` ni redirecciones ni reintentos: no es un cliente
// HTTP de propósito general y no debe llegar a serlo. Si algún día hace falta
// hablar con algo de fuera, eso es un proceso aparte — que es la respuesta que
// este árbol ya dio cuatro veces.

/// Una petición a un servicio del clúster. Devuelve `(código, cuerpo)`.
///
/// `destino` es `host:puerto` sin esquema — no hay esquema que elegir.
pub fn pedir(
    metodo: &str,
    destino: &str,
    camino: &str,
    testigo: Option<&str>,
    cuerpo: Option<&Json>,
) -> Result<(u16, String), String> {
    // ⛔ Y un plazo TAMBIÉN para conectar. `TcpStream::connect` no tiene ninguno:
    //   con una máquina apagada (0027 E3 I5, `modelos-e0` TERMINATED) los paquetes
    //   se tiran sin contestar y el SO tarda lo que quiera —medido: `GET /modelos`
    //   moría a los 30 s de la entrada pública, con `504 stream timeout`—. Un
    //   servicio de la VPC que no acepta en 5 s no va a aceptar.
    let destinos = std::net::ToSocketAddrs::to_socket_addrs(&destino)
        .map_err(|e| format!("no se pudo resolver `{destino}`: {e}"))?;
    let mut flujo = None;
    let mut ultimo = String::from("sin direcciones");
    for d in destinos {
        match TcpStream::connect_timeout(&d, std::time::Duration::from_secs(5)) {
            Ok(f) => {
                flujo = Some(f);
                break;
            }
            Err(e) => ultimo = e.to_string(),
        }
    }
    let Some(mut flujo) = flujo else {
        return Err(format!("no se pudo conectar con `{destino}`: {ultimo}"));
    };
    // ⛔ Un tiempo límite en las dos direcciones. Sin esto, un servicio que
    //   acepta la conexión y no contesta deja al plano de control colgado — y
    //   ése es exactamente el síntoma que una `NetworkPolicy` produce.
    let plazo = std::time::Duration::from_secs(15);
    let _ = flujo.set_read_timeout(Some(plazo));
    let _ = flujo.set_write_timeout(Some(plazo));

    // ⭐ La forma canonica y no la indentada: esto lo lee un programa. Es la
    //   misma que usa el sellado, asi que dos peticiones identicas producen
    //   bytes identicos.
    let serializado = cuerpo.map(|c| c.jcs()).unwrap_or_default();
    let mut peticion =
        format!("{metodo} {camino} HTTP/1.1\r\nHost: {destino}\r\nConnection: close\r\n");
    if let Some(t) = testigo {
        peticion.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    if cuerpo.is_some() {
        peticion.push_str("Content-Type: application/json\r\n");
        peticion.push_str(&format!("Content-Length: {}\r\n", serializado.len()));
    }
    peticion.push_str("\r\n");
    peticion.push_str(&serializado);

    flujo
        .write_all(peticion.as_bytes())
        .map_err(|e| format!("no se pudo escribir a `{destino}`: {e}"))?;

    let mut crudo = Vec::new();
    // ⚠️ Acotado como el del servidor, y por lo mismo: una respuesta sin límite
    //   es una forma de quedarse sin memoria a petición de otro proceso.
    flujo
        .take(CUERPO_MAXIMO as u64 + 4096)
        .read_to_end(&mut crudo)
        .map_err(|e| format!("no se pudo leer de `{destino}`: {e}"))?;
    let texto = String::from_utf8_lossy(&crudo).into_owned();

    let (cabeza, cuerpo) = texto
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("`{destino}` no contestó un HTTP entero"))?;
    let codigo = cabeza
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| format!("`{destino}` no dijo un código: {}", primera_linea(cabeza)))?;
    // ⭐ La forja (Go) trocea las respuestas largas (`Transfer-Encoding:
    //   chunked`): sin esto, un diff de una PR llegaría con los tamaños de los
    //   trozos dentro. Los servidores nuestros mandan `Content-Length`.
    let troceado = cabeza
        .lines()
        .any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
    let cuerpo = if troceado {
        destrocear(cuerpo)
    } else {
        cuerpo.to_string()
    };
    Ok((codigo, cuerpo))
}

/// Un cuerpo `chunked` (RFC 9112 §7.1), junto: `tamaño-hex\r\ntrozo\r\n…0\r\n`.
/// Un trozo mal formado corta la lectura ahí y devuelve lo que había: lo que
/// sigue es un JSON que no analiza, y eso ya se dice.
fn destrocear(cuerpo: &str) -> String {
    let b = cuerpo.as_bytes();
    let mut i = 0;
    let mut salida = Vec::with_capacity(b.len());
    while let Some(fin) = b[i..].windows(2).position(|w| w == b"\r\n") {
        let linea = &cuerpo[i..i + fin];
        let tamano = linea.split(';').next().unwrap_or("").trim();
        let Ok(n) = usize::from_str_radix(tamano, 16) else {
            break;
        };
        i += fin + 2;
        if n == 0 || i + n > b.len() {
            break;
        }
        salida.extend_from_slice(&b[i..i + n]);
        i += n + 2;
    }
    String::from_utf8_lossy(&salida).into_owned()
}

fn primera_linea(s: &str) -> String {
    s.lines().next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod pruebas_de_pedir {
    use super::*;

    #[test]
    fn un_cuerpo_troceado_se_junta() {
        assert_eq!(
            destrocear("4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n"),
            "{\"a\":1}"
        );
        assert_eq!(destrocear("5;ext=1\r\nhola \r\n0\r\n\r\n"), "hola ");
        assert_eq!(destrocear("zz\r\n"), "");
    }

    /// Una dirección que no contesta (10.255.255.1 no enruta a ningún sitio): el
    /// plazo de conectar es lo que acota la espera, no el SO.
    #[test]
    fn una_maquina_apagada_no_cuelga_el_plano_de_control() {
        let t0 = std::time::Instant::now();
        let r = pedir("GET", "10.255.255.1:9000", "/admin/health", None, None);
        assert!(r.is_err(), "{r:?}");
        assert!(r.unwrap_err().contains("no se pudo conectar"));
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(8),
            "tardó {:?}",
            t0.elapsed()
        );
    }
}

#[cfg(test)]
mod pruebas_del_flujo {
    use super::*;

    /// Lo que un cliente recibe de un flujo: la cabeza, y el cuerpo troceado.
    fn pedir_crudo(puerto: u16, camino: &str) -> String {
        let mut c = TcpStream::connect(("127.0.0.1", puerto)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        c.write_all(format!("GET {camino} HTTP/1.1\r\nhost: p\r\n\r\n").as_bytes())
            .unwrap();
        let mut t = String::new();
        let _ = c.read_to_string(&mut t);
        t
    }

    /// Una respuesta que no termina: tres eventos, con su `id`, y el punto
    /// final. Y los eventos salen **según se escriben**, no al cerrar.
    #[test]
    fn un_flujo_sale_troceado_y_con_sus_eventos() {
        let escucha = TcpListener::bind("127.0.0.1:0").unwrap();
        let puerto = escucha.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let _ = servir_con_flujos(escucha, |_p| {
                Salida::Flujo(Flujo {
                    tipo: "text/event-stream",
                    escribir: Box::new(|e| {
                        for n in 1..=3u64 {
                            assert!(e.evento(
                                "celda",
                                Some(n),
                                &Json::obj([("n", Json::Int(n as i64))])
                            ));
                        }
                        assert!(e.latido());
                    }),
                })
            });
        });
        let t = pedir_crudo(puerto, "/flujo");
        assert!(t.contains("transfer-encoding: chunked"), "{t}");
        assert!(t.contains("content-type: text/event-stream"), "{t}");
        assert!(!t.contains("content-length"), "{t}");
        let cuerpo = destrocear(t.split_once("\r\n\r\n").unwrap().1);
        assert!(
            cuerpo.contains("id: 2\nevent: celda\ndata: {\"n\":2}"),
            "{cuerpo:?}"
        );
        assert_eq!(cuerpo.matches("event: celda").count(), 3, "{cuerpo:?}");
        assert!(cuerpo.contains(": latido"), "{cuerpo:?}");
        assert!(t.ends_with("0\r\n\r\n"), "{t:?}");
    }

    /// Y si el que leía se va, `evento` lo dice y el que emite para. Sin esto,
    /// un editor cerrado dejaría un hilo escribiendo contra nadie.
    #[test]
    fn cuando_el_que_lee_se_va_el_que_emite_se_entera() {
        let escucha = TcpListener::bind("127.0.0.1:0").unwrap();
        let puerto = escucha.local_addr().unwrap().port();
        let (avisa, espera) = std::sync::mpsc::channel::<u64>();
        std::thread::spawn(move || {
            let _ = servir_con_flujos(escucha, move |_p| {
                let avisa = avisa.clone();
                Salida::Flujo(Flujo {
                    tipo: "text/event-stream",
                    escribir: Box::new(move |e| {
                        // Un dato gordo: el que se va deja de leer y el buffer
                        // se llena, que es como se nota que no hay nadie.
                        let gordo = Json::s("x".repeat(64 * 1024));
                        let mut n = 0u64;
                        while e.evento("ruido", Some(n), &gordo) && n < 10_000 {
                            n += 1;
                        }
                        let _ = avisa.send(n);
                    }),
                })
            });
        });
        let mut c = TcpStream::connect(("127.0.0.1", puerto)).unwrap();
        c.write_all(b"GET /flujo HTTP/1.1\r\nhost: p\r\n\r\n")
            .unwrap();
        let mut algo = [0u8; 64];
        let _ = c.read(&mut algo);
        drop(c);
        let n = espera.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(n < 10_000, "siguió escribiendo contra nadie: {n} eventos");
    }
}
