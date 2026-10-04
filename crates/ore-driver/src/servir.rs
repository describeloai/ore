//! **El verbo `servir`** (`docs/federation.md` §1.1 y §1.3): peticiones una
//! tras otra por stdin, cada respuesta enmarcada por stdout, y las conexiones
//! al origen abiertas entre una y otra.
//!
//! # El marco
//!
//! ```text
//! {"estado":"ok","id":…}\n                       la cabecera
//! <u32 big-endian n><n bytes> …  <u32 0>          el flujo Arrow, en trozos
//! {"bytes":B,"filas":N,"fin":"ok"}\n              el cierre
//!   ó {"codigo":…,"fin":"error","mensaje":…,"reintentable":…}\n
//! ```
//!
//! o, si la petición falla antes de dar un solo byte, una sola línea
//! `{"codigo":…,"estado":"error","id":…,"mensaje":…,"reintentable":…}`.
//!
//! **Por qué en trozos y no el flujo Arrow tal cual.** Un error a mitad deja
//! el flujo Arrow sin su marca de fin, y en `leer` eso basta: el proceso
//! termina y quien lee ve un flujo corto. En `servir` el proceso sigue, y lo
//! siguiente que escribe es la respuesta a otra petición: sin longitudes,
//! quien lee la tomaría por el resto del flujo roto. Con ellas, un error a
//! mitad se cierra con su trozo vacío y su cierre, y la siguiente respuesta
//! empieza limpia. Es la misma idea que el cierre de [`crate::tramas`].

use crate::Peticion;
use crate::fallo::{Codigo, Fallo};
use ore_core::json::Json;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

/// Lo que un trozo lleva como mucho. Lo que se escribe se junta hasta aquí.
pub const TROZO: usize = 64 * 1024;

/// El escritor de una respuesta: escribe la cabecera con el primer byte, y lo
/// demás en trozos con su longitud.
pub struct Respuesta<'a, W: Write> {
    salida: &'a mut W,
    id: String,
    empezada: bool,
    pendiente: Vec<u8>,
    bytes: u64,
}

impl<'a, W: Write> Respuesta<'a, W> {
    pub fn new(salida: &'a mut W, id: &str) -> Respuesta<'a, W> {
        Respuesta {
            salida,
            id: id.to_string(),
            empezada: false,
            pendiente: Vec::with_capacity(TROZO),
            bytes: 0,
        }
    }

    /// Si ya salió algún byte: después de eso, un error es un cierre y no una
    /// línea suelta.
    pub fn empezada(&self) -> bool {
        self.empezada
    }

    fn cabecera(&mut self) -> std::io::Result<()> {
        if !self.empezada {
            self.empezada = true;
            let c = Json::obj([("estado", Json::s("ok")), ("id", Json::s(self.id.as_str()))]);
            writeln!(self.salida, "{}", c.jcs())?;
        }
        Ok(())
    }

    fn vaciar(&mut self) -> std::io::Result<()> {
        if self.pendiente.is_empty() {
            return Ok(());
        }
        self.cabecera()?;
        let n = self.pendiente.len() as u32;
        self.salida.write_all(&n.to_be_bytes())?;
        self.salida.write_all(&self.pendiente)?;
        self.pendiente.clear();
        Ok(())
    }

    /// El cierre: `Ok(filas)` o el fallo. Consume la respuesta.
    pub fn cerrar(mut self, r: Result<u64, Fallo>) -> std::io::Result<()> {
        match r {
            Err(f) if !self.empezada && self.pendiente.is_empty() => {
                let mut o = f.campos();
                o.insert("estado".to_string(), Json::s("error"));
                o.insert("id".to_string(), Json::s(self.id.as_str()));
                writeln!(self.salida, "{}", Json::Obj(o).jcs())?;
            }
            r => {
                self.vaciar()?;
                self.cabecera()?;
                self.salida.write_all(&0u32.to_be_bytes())?;
                let fin = match r {
                    Ok(filas) => Json::obj([
                        ("bytes", Json::Int(self.bytes as i64)),
                        ("filas", Json::Int(filas as i64)),
                        ("fin", Json::s("ok")),
                    ]),
                    Err(f) => {
                        let mut o = f.campos();
                        o.insert("fin".to_string(), Json::s("error"));
                        Json::Obj(o)
                    }
                };
                writeln!(self.salida, "{}", fin.jcs())?;
            }
        }
        self.salida.flush()
    }
}

impl<W: Write> Write for Respuesta<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = buf.len().min(TROZO - self.pendiente.len());
        self.pendiente.extend_from_slice(&buf[..n]);
        self.bytes += n as u64;
        if self.pendiente.len() == TROZO {
            self.vaciar()?;
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.vaciar()?;
        self.salida.flush()
    }
}

/// **El bucle**: una petición por línea hasta que se cierra la entrada.
/// Devuelve cuántas atendió.
///
/// `atender` escribe el flujo Arrow en la [`Respuesta`] y devuelve las filas, o
/// el fallo. Una línea que no es una petición se contesta con `operador` y el
/// bucle sigue: un error de quien pide no tumba las conexiones de los demás.
/// Los mensajes de los fallos se tapan con la `url` de su petición.
pub fn servir<R, W, F>(entrada: R, salida: &mut W, mut atender: F) -> std::io::Result<u64>
where
    R: BufRead,
    W: Write,
    F: FnMut(&Peticion, &mut Respuesta<'_, W>) -> Result<u64, Fallo>,
{
    let mut n = 0;
    for linea in entrada.lines() {
        let linea = linea?;
        if linea.trim().is_empty() {
            continue;
        }
        n += 1;
        match crate::leer_peticion(&linea) {
            Ok(p) => {
                let id = p.id.clone().unwrap_or_default();
                let mut r = Respuesta::new(salida, &id);
                let hecho = atender(&p, &mut r).map_err(|f| f.tapado(&p.url));
                r.cerrar(hecho)?;
            }
            Err(e) => {
                let id = ore_core::parse::parse(&linea)
                    .ok()
                    .and_then(|n| n.get("id").and_then(|(_, v)| v.as_str()).map(String::from))
                    .unwrap_or_default();
                let url = ore_core::parse::parse(&linea)
                    .ok()
                    .and_then(|n| n.get("url").and_then(|(_, v)| v.as_str()).map(String::from))
                    .unwrap_or_default();
                Respuesta::new(salida, &id)
                    .cerrar(Err(Fallo::new(Codigo::Operador, e).tapado(&url)))?;
            }
        }
    }
    Ok(n)
}

/// Cómo terminó una respuesta, leída del otro lado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fin {
    Ok {
        id: String,
        filas: u64,
        bytes: u64,
    },
    /// `empezada`: si llegaron bytes antes del fallo (y quien lee los tira).
    Error {
        id: String,
        fallo: Fallo,
        empezada: bool,
    },
}

/// **Lee una respuesta** y pasa su flujo Arrow a `destino`. `Ok(None)` si la
/// entrada se acabó antes de empezar otra; `Err` si el marco está roto.
pub fn leer_respuesta<R: BufRead, D: Write>(
    entrada: &mut R,
    destino: &mut D,
) -> Result<Option<Fin>, String> {
    let roto = |e: std::io::Error| format!("el marco de la respuesta está roto: {e}");
    let linea = |entrada: &mut R| -> Result<Option<ore_core::parse::Node>, String> {
        let mut l = String::new();
        if entrada.read_line(&mut l).map_err(roto)? == 0 {
            return Ok(None);
        }
        ore_core::parse::parse(l.trim())
            .map(Some)
            .map_err(|e| format!("una línea del marco no es JSON: {e:?}"))
    };
    let Some(c) = linea(entrada)? else {
        return Ok(None);
    };
    let texto = |n: &ore_core::parse::Node, k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let id = texto(&c, "id");
    match texto(&c, "estado").as_str() {
        "error" => {
            let fallo = Fallo::de_nodo(&c).ok_or("un error sin código")?;
            return Ok(Some(Fin::Error {
                id,
                fallo,
                empezada: false,
            }));
        }
        "ok" => {}
        otro => return Err(format!("`estado: {otro}` no es `ok` ni `error`")),
    }
    let mut largo = [0u8; 4];
    let mut trozo = Vec::new();
    loop {
        entrada.read_exact(&mut largo).map_err(roto)?;
        let n = u32::from_be_bytes(largo) as usize;
        if n == 0 {
            break;
        }
        trozo.resize(n, 0);
        entrada.read_exact(&mut trozo).map_err(roto)?;
        destino.write_all(&trozo).map_err(roto)?;
    }
    let f = linea(entrada)?.ok_or("la respuesta acaba sin su cierre")?;
    let numero = |k: &str| texto(&f, k).parse::<u64>().unwrap_or(0);
    match texto(&f, "fin").as_str() {
        "ok" => Ok(Some(Fin::Ok {
            id,
            filas: numero("filas"),
            bytes: numero("bytes"),
        })),
        "error" => Ok(Some(Fin::Error {
            id,
            fallo: Fallo::de_nodo(&f).ok_or("un cierre con error sin código")?,
            empezada: true,
        })),
        otro => Err(format!("`fin: {otro}` no es `ok` ni `error`")),
    }
}

/// **Las conexiones abiertas de un `servir`**, por `url`.
///
/// Una conexión no se comparte entre credenciales (`docs/federation.md` §1.5):
/// la clave es la `url` entera, que lleva la credencial, así que dos
/// credenciales son dos conexiones. Por eso esto no implementa `Debug`: la
/// clave no se imprime.
pub struct Conexiones<C> {
    abiertas: HashMap<String, (C, Instant)>,
    ociosa: Duration,
}

impl<C> Conexiones<C> {
    /// `ociosa`: lo que una conexión sin uso sigue abierta.
    pub fn new(ociosa: Duration) -> Conexiones<C> {
        Conexiones {
            abiertas: HashMap::new(),
            ociosa,
        }
    }

    /// La de `url`, abriéndola con `abrir` si no está. Antes, cierra las que
    /// llevan más de `ociosa` sin uso.
    pub fn tomar<E>(
        &mut self,
        url: &str,
        abrir: impl FnOnce(&str) -> Result<C, E>,
    ) -> Result<&mut C, E> {
        self.barrer(Instant::now());
        if !self.abiertas.contains_key(url) {
            let c = abrir(url)?;
            self.abiertas.insert(url.to_string(), (c, Instant::now()));
        }
        let (c, uso) = self.abiertas.get_mut(url).expect("recién puesta");
        *uso = Instant::now();
        Ok(c)
    }

    /// Cierra la de `url`: tras un error de conexión no se reutiliza.
    pub fn quitar(&mut self, url: &str) {
        self.abiertas.remove(url);
    }

    /// Cierra las que llevan más de `ociosa` sin uso a `ahora`.
    pub fn barrer(&mut self, ahora: Instant) {
        let ociosa = self.ociosa;
        self.abiertas
            .retain(|_, (_, uso)| ahora.saturating_duration_since(*uso) < ociosa);
    }

    pub fn len(&self) -> usize {
        self.abiertas.len()
    }

    pub fn is_empty(&self) -> bool {
        self.abiertas.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn peticion(id: &str, url: &str) -> String {
        format!(r#"{{"id":"{id}","objeto":"t","url":"{url}","proyeccion":{{"a":"a"}}}}"#)
    }

    /// **Un error a mitad no se come la respuesta siguiente**: la segunda
    /// petición llega entera después de una que falló con bytes ya escritos.
    #[test]
    fn un_error_a_mitad_deja_la_siguiente_respuesta_limpia() {
        let entrada = [
            peticion("1", "x://u:clave@h"),
            "esto no es una petición".to_string(),
            peticion("2", "x://u:clave@h"),
            peticion("3", "x://u:clave@h"),
        ]
        .join("\n");
        let mut salida: Vec<u8> = Vec::new();
        let n = servir(Cursor::new(entrada), &mut salida, |p, r| {
            match p.id.as_deref() {
                // Un flujo de más de un trozo, y luego el fallo.
                Some("1") => {
                    r.write_all(&vec![7u8; TROZO + 10]).unwrap();
                    Err(Fallo::new(Codigo::Conexion, "se cortó x://u:clave@h"))
                }
                Some("2") => {
                    r.write_all(b"ARROW").unwrap();
                    Ok(1)
                }
                // Falla sin escribir: una sola línea.
                _ => Err(Fallo::new(Codigo::Objeto, "no existe `t`")),
            }
        })
        .expect("sirve");
        assert_eq!(n, 4);

        let mut c = Cursor::new(salida);
        let mut flujo = Vec::new();
        match leer_respuesta(&mut c, &mut flujo).unwrap().unwrap() {
            Fin::Error {
                id,
                fallo,
                empezada,
            } => {
                assert_eq!((id.as_str(), empezada), ("1", true));
                assert_eq!(fallo.codigo, Codigo::Conexion);
                assert!(fallo.reintentable);
                assert!(!fallo.mensaje.contains("clave"), "{}", fallo.mensaje);
            }
            otro => panic!("{otro:?}"),
        }
        assert_eq!(flujo.len(), TROZO + 10);

        match leer_respuesta(&mut c, &mut Vec::new()).unwrap().unwrap() {
            Fin::Error {
                fallo,
                empezada: false,
                ..
            } => {
                assert_eq!(fallo.codigo, Codigo::Operador)
            }
            otro => panic!("{otro:?}"),
        }

        let mut flujo = Vec::new();
        assert_eq!(
            leer_respuesta(&mut c, &mut flujo).unwrap(),
            Some(Fin::Ok {
                id: "2".into(),
                filas: 1,
                bytes: 5
            })
        );
        assert_eq!(flujo, b"ARROW");

        match leer_respuesta(&mut c, &mut Vec::new()).unwrap().unwrap() {
            Fin::Error {
                id,
                fallo,
                empezada: false,
            } => {
                assert_eq!(id, "3");
                assert_eq!(fallo.codigo, Codigo::Objeto);
            }
            otro => panic!("{otro:?}"),
        }
        assert_eq!(leer_respuesta(&mut c, &mut Vec::new()).unwrap(), None);
    }

    /// **Una conexión por credencial**, reutilizada, y cerrada tras su ocio.
    #[test]
    fn las_conexiones_se_reutilizan_por_url_y_caducan() {
        let mut abiertas = 0;
        let mut cs: Conexiones<u32> = Conexiones::new(Duration::from_secs(60));
        for _ in 0..100 {
            cs.tomar("pg://a:1@h", |_| -> Result<u32, ()> {
                abiertas += 1;
                Ok(abiertas)
            })
            .unwrap();
        }
        assert_eq!(abiertas, 1, "cien peticiones, una conexión");
        cs.tomar("pg://b:2@h", |_| -> Result<u32, ()> {
            abiertas += 1;
            Ok(abiertas)
        })
        .unwrap();
        assert_eq!(
            (abiertas, cs.len()),
            (2, 2),
            "otra credencial, otra conexión"
        );
        cs.barrer(Instant::now() + Duration::from_secs(61));
        assert!(cs.is_empty());
    }
}
