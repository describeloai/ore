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

fn texto(codigo: u16) -> &'static str {
    match codigo {
        200 => "OK",
        201 => "Created",
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
    let manejador = Arc::new(manejador);
    let vivas = Arc::new(AtomicUsize::new(0));

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
                let _viva = Viva(vivas_hilo);
                atender(flujo, manejador.as_ref());
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

fn atender<F>(mut flujo: TcpStream, manejador: &F)
where
    F: Fn(&Peticion) -> Respuesta,
{
    let respuesta = match leer(&mut flujo) {
        Ok(p) => manejador(&p),
        Err(r) => r,
    };
    responder(&mut flujo, &respuesta);
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
    let cuerpo = r.cuerpo.jcs();
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
    let mut flujo = TcpStream::connect(destino)
        .map_err(|e| format!("no se pudo conectar con `{destino}`: {e}"))?;
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
    Ok((codigo, cuerpo.to_string()))
}

fn primera_linea(s: &str) -> String {
    s.lines().next().unwrap_or_default().to_string()
}
