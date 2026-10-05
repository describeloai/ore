//! **La puerta de lectura: los bytes de un ítem, fijados** (ADR 0049 B3·1).
//!
//! Un ítem —de una colección mantenida o virtual— se lee **entero o por
//! rango, en flujo, fijado a su versión**, y la celda calcula su `sha256` al
//! paso cuando se lee entero (D1). Es lo único de `ore-medios` que mueve bytes
//! del contenido; quién puede y qué ítem es lo decide `ore-serve` antes.
//!
//! # Lo que B3·0 midió, y lo que decide aquí
//!
//! - **Fijar es doble**: `versionId` —también `null`, la versión de un objeto
//!   subido antes del versionado, que en un bucket versionado sobrevive a una
//!   sobrescritura— e `If-Match` con el ETag del manifiesto. S3 contesta `412`
//!   si no casa, y eso es `media/cambiado`: nunca otros bytes bajo la misma
//!   referencia.
//! - **Rangos**: el origen los da (`Accept-Ranges: bytes`), y cada petición
//!   cuesta ~125 ms a otra región: el que lee pide bloques grandes (el SDK),
//!   esto pasa el `Range` tal cual.
//! - **El `sha256` al paso** cuesta un 8 % en Python y menos aquí. S3 no
//!   guarda ninguno (el bucket de prueba no trae ni CRC): sin esto, una
//!   virtual no tendría identidad por contenido.
//!
//! # Un error a mitad
//!
//! Las cabeceras salen antes que los bytes, así que lo que falla después
//! (el origen se corta, el tamaño o el digest no casan) **corta la
//! conexión**: quien lee ve un cuerpo corto y no puede tomarlo por entero. El
//! SDK lo cuenta como `media/corrupto` (`docs/media.md`).

use crate::servicio::problema;
use ore_entrada::http::{Bytes, Respuesta};
use ore_store::almacen::Almacen;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};

/// De dónde salen los bytes.
pub enum Origen {
    /// Un blob del lago, por su `sha256` (una colección mantenida).
    Lago { sha256: String },
    /// Un objeto de un bucket S3 (una colección virtual). `fuente` es la URL
    /// `s3://…` **con la credencial temporal** que `ore-serve` canjeó (0046
    /// E9b): no se escribe en ningún sitio ni sale de aquí.
    S3 {
        fuente: String,
        clave: String,
        version: String,
        etag: String,
    },
}

/// Un ítem resuelto: lo que hay que leer y lo que se sabe de él.
pub struct Pieza {
    /// `b.s.n`, el camino y la versión: la referencia (y la clave de lo visto).
    pub coleccion: String,
    pub camino: String,
    pub version: String,
    pub origen: Origen,
    pub tamano: Option<u64>,
    /// El `sha256` en hexadecimal, si se conoce.
    pub sha256: Option<String>,
    pub tipo: Option<String>,
}

/// Cuántos digests vistos se guardan antes de olvidarlos todos (B3: en
/// memoria; escribirlos en el manifiesto es de B5).
const VISTOS_MAXIMOS: usize = 500_000;

/// **Los `sha256` calculados al paso**, por `(colección, camino, versión)`:
/// lo que un `stat` de una virtual dice después de una lectura entera
/// (`open-007`). En memoria: un reinicio los olvida, y la siguiente lectura
/// entera los vuelve a calcular.
#[derive(Default)]
pub struct Vistos {
    mapa: Mutex<HashMap<(String, String, String), String>>,
}

impl Vistos {
    pub fn anotar(&self, coleccion: &str, camino: &str, version: &str, sha256: &str) {
        let mut m = self.mapa.lock().unwrap();
        if m.len() >= VISTOS_MAXIMOS {
            m.clear();
        }
        m.insert(
            (coleccion.into(), camino.into(), version.into()),
            sha256.into(),
        );
    }

    pub fn de(&self, coleccion: &str, camino: &str, version: &str) -> Option<String> {
        self.mapa
            .lock()
            .unwrap()
            .get(&(coleccion.into(), camino.into(), version.into()))
            .cloned()
    }
}

/// Un rango de bytes de UNA parte (RFC 9110 §14.1.2). Varios rangos en una
/// petición no se sirven: nadie que lea un fichero los necesita.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rango {
    /// `bytes=a-b` o `bytes=a-`.
    Desde(u64, Option<u64>),
    /// `bytes=-n`: los últimos `n`.
    Sufijo(u64),
}

impl Rango {
    pub fn de(cabecera: &str) -> Option<Rango> {
        let r = cabecera.trim().strip_prefix("bytes=")?.trim();
        if r.contains(',') {
            return None;
        }
        let (a, z) = r.split_once('-')?;
        let num = |s: &str| s.trim().parse::<u64>().ok();
        match (a.trim().is_empty(), z.trim().is_empty()) {
            (true, false) => num(z).filter(|n| *n > 0).map(Rango::Sufijo),
            (false, true) => num(a).map(|a| Rango::Desde(a, None)),
            (false, false) => {
                let (a, z) = (num(a)?, num(z)?);
                (a <= z).then_some(Rango::Desde(a, Some(z)))
            }
            (true, true) => None,
        }
    }

    pub fn cabecera(&self) -> String {
        match self {
            Rango::Desde(a, Some(z)) => format!("bytes={a}-{z}"),
            Rango::Desde(a, None) => format!("bytes={a}-"),
            Rango::Sufijo(n) => format!("bytes=-{n}"),
        }
    }

    /// `(inicio, fin)` inclusivos dentro de `tamano`; `None` si no cabe.
    pub fn en(&self, tamano: u64) -> Option<(u64, u64)> {
        if tamano == 0 {
            return None;
        }
        match *self {
            Rango::Desde(a, _) if a >= tamano => None,
            Rango::Desde(a, z) => Some((a, z.unwrap_or(tamano - 1).min(tamano - 1))),
            Rango::Sufijo(n) => Some((tamano.saturating_sub(n), tamano - 1)),
        }
    }
}

/// **Abre un ítem**: las cabeceras y el cuerpo en flujo, o el error del
/// contrato. `rango` es la cabecera `Range` tal cual llegó.
pub fn abrir(
    cuenta: &dyn Almacen,
    pieza: &Pieza,
    rango: Option<&str>,
    vistos: Arc<Vistos>,
) -> Result<Bytes, Respuesta> {
    let rango = match rango {
        None => None,
        Some(r) => Some(Rango::de(r).ok_or_else(|| {
            problema(
                416,
                "media/rango",
                format!("`{r}` no es un rango de una parte (`bytes=a-b`, `bytes=a-`, `bytes=-n`)"),
            )
        })?),
    };
    let (codigo, mut cabeceras, largo, lector) = match &pieza.origen {
        Origen::Lago { sha256 } => del_lago(cuenta, pieza, sha256, rango)?,
        Origen::S3 {
            fuente,
            clave,
            version,
            etag,
        } => de_s3(fuente, clave, version, etag, rango)?,
    };
    cabeceras.push(("ore-media-version".into(), pieza.version.clone()));
    // Del entero también en un 206 (RFC 9530 §3).
    if let Some(b) = pieza.sha256.as_deref().and_then(hex_a_bytes) {
        cabeceras.push((
            "repr-digest".into(),
            format!("sha-256=:{}:", ore_s3::base64(&b)),
        ));
    }
    if !cabeceras.iter().any(|(k, _)| k == "content-type") {
        let tipo = pieza
            .tipo
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into());
        cabeceras.push(("content-type".into(), tipo));
    }
    // Entero: se verifica al paso y, si no se conocía, se anota.
    let lector: Box<dyn Read + Send> = if codigo == 200 {
        Box::new(Verificado {
            dentro: lector,
            hash: Sha256::new(),
            leidos: 0,
            tamano: largo.or(pieza.tamano),
            sha256: pieza.sha256.clone(),
            anotar: Some((
                vistos,
                pieza.coleccion.clone(),
                pieza.camino.clone(),
                pieza.version.clone(),
            )),
        })
    } else {
        lector
    };
    Ok(Bytes {
        codigo,
        cabeceras,
        largo,
        lector,
        finales: None,
    })
}

type Abierto = (
    u16,
    Vec<(String, String)>,
    Option<u64>,
    Box<dyn Read + Send>,
);

fn de_s3(
    fuente: &str,
    clave: &str,
    version: &str,
    etag: &str,
    rango: Option<Rango>,
) -> Result<Abierto, Respuesta> {
    let b = ore_sigv4::fuente::leer(fuente)
        .map_err(|e| {
            problema(
                502,
                "media/origen",
                format!("la fuente no se pudo leer: {e}"),
            )
        })?
        .bucket;
    let cabecera = rango.map(|r| r.cabecera());
    let l = ore_s3::leer_fijado(&b, clave, version, etag, cabecera.as_deref())
        .map_err(|r| error_de_s3(&r, clave))?;
    let mut cabeceras = Vec::new();
    for k in [
        "etag",
        "content-type",
        "content-range",
        "accept-ranges",
        "last-modified",
    ] {
        if let Some(v) = l.cabecera(k) {
            cabeceras.push((k.to_string(), v.to_string()));
        }
    }
    let largo = l.cabecera("content-length").and_then(|v| v.parse().ok());
    Ok((l.estado, cabeceras, largo, l.lector))
}

/// Lo que S3 contestó, en el idioma del contrato. Un ítem que el manifiesto
/// lista y el origen ya no tiene en esa versión **cambió**: `media/cambiado`.
fn error_de_s3(r: &ore_s3::Respuesta, clave: &str) -> Respuesta {
    let codigo_aws = r.error_de_aws().map(|(c, _)| c).unwrap_or_default();
    match (r.estado, codigo_aws.as_str()) {
        (412, _) | (404, _) | (400, "InvalidArgument") => problema(
            412,
            "media/cambiado",
            format!(
                "`{clave}` ya no es, en el origen, la versión fijada ({})",
                r.motivo()
            ),
        ),
        (416, _) => problema(416, "media/rango", format!("el rango no cabe en `{clave}`")),
        (0, _) => problema(
            502,
            "media/origen",
            format!(
                "el origen no contesta: {}",
                String::from_utf8_lossy(&r.cuerpo)
            ),
        ),
        _ => problema(
            502,
            "media/origen",
            format!("el origen contestó {} a `{clave}`", r.motivo()),
        ),
    }
}

/// Un blob del lago. `leer_rango` de GCS pide sólo el rango; el cuerpo se
/// sirve de memoria. (Una mantenida, de normal, no pasa por aquí: `ore-serve`
/// redirige a la URL firmada del blob, B3·3. Esto es para quien no puede.)
fn del_lago(
    cuenta: &dyn Almacen,
    pieza: &Pieza,
    sha256: &str,
    rango: Option<Rango>,
) -> Result<Abierto, Respuesta> {
    let clave = ore_store::blobs::clave_de(sha256);
    let no_esta = || {
        problema(
            404,
            "media/no-existe",
            format!("`{}` no tiene su blob en el lago", pieza.camino),
        )
    };
    let tamano = match pieza.tamano {
        Some(t) => t,
        None => cuenta
            .tamano(&clave)
            .map_err(|e| problema(502, "media/origen", e))?
            .ok_or_else(no_esta)?,
    };
    let tramo = match rango {
        None => None,
        Some(r) => Some(r.en(tamano).ok_or_else(|| {
            problema(
                416,
                "media/rango",
                format!("el rango no cabe en `{}` ({tamano} bytes)", pieza.camino),
            )
        })?),
    };
    let bytes = cuenta
        .leer_rango(&clave, tramo)
        .map_err(|e| problema(502, "media/origen", e))?
        .ok_or_else(no_esta)?;
    let mut cabeceras = vec![
        ("etag".to_string(), format!("\"{sha256}\"")),
        ("accept-ranges".to_string(), "bytes".to_string()),
    ];
    let codigo = match tramo {
        Some((a, z)) => {
            cabeceras.push(("content-range".into(), format!("bytes {a}-{z}/{tamano}")));
            206
        }
        None => 200,
    };
    let largo = Some(bytes.len() as u64);
    Ok((
        codigo,
        cabeceras,
        largo,
        Box::new(std::io::Cursor::new(bytes)),
    ))
}

/// Lee de `dentro` y, al llegar al final, **comprueba** el tamaño y el digest
/// que se conocían; si el digest no se conocía, lo anota en [`Vistos`]. Lo que
/// no casa es un error de lectura: la conexión se corta (ver el módulo).
struct Verificado {
    dentro: Box<dyn Read + Send>,
    hash: Sha256,
    leidos: u64,
    tamano: Option<u64>,
    sha256: Option<String>,
    anotar: Option<(Arc<Vistos>, String, String, String)>,
}

impl Read for Verificado {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.dentro.read(buf)?;
        if n > 0 {
            self.hash.update(&buf[..n]);
            self.leidos += n as u64;
            return Ok(n);
        }
        let Some((vistos, coleccion, camino, version)) = self.anotar.take() else {
            return Ok(0);
        };
        let corrupto = |m: String| {
            eprintln!("ore-medios · media/corrupto · {coleccion} · {camino} · {m}");
            Err(std::io::Error::new(std::io::ErrorKind::InvalidData, m))
        };
        if let Some(t) = self.tamano.filter(|t| *t != self.leidos) {
            return corrupto(format!("{} bytes de {t}", self.leidos));
        }
        let visto = ore_s3::hex(&std::mem::take(&mut self.hash).finalize());
        match &self.sha256 {
            Some(h) if !h.eq_ignore_ascii_case(&visto) => {
                corrupto(format!("sha256 {visto}, y el ítem dice {h}"))
            }
            Some(_) => Ok(0),
            None => {
                vistos.anotar(&coleccion, &camino, &version, &visto);
                Ok(0)
            }
        }
    }
}

fn hex_a_bytes(h: &str) -> Option<Vec<u8>> {
    if !h.len().is_multiple_of(2) {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
pub(crate) mod pruebas {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    const CUERPO: &[u8] = b"%PDF-hola, esto es un contrato";
    const ETAG: &str = "\"e1\"";

    /// Un S3 de mentira: una clave, `docs/a.pdf`, en las versiones `v1` y
    /// `null` con el mismo cuerpo. Fija como S3 (versión, `If-Match`), da
    /// rangos y anota lo que le llegó.
    fn s3_falso() -> (String, Arc<Mutex<Vec<String>>>) {
        let escucha = TcpListener::bind("127.0.0.1:0").unwrap();
        let puerto = escucha.local_addr().unwrap().port();
        let anotado = Arc::new(Mutex::new(Vec::new()));
        let a = anotado.clone();
        std::thread::spawn(move || {
            for c in escucha.incoming() {
                let Ok(mut c) = c else { continue };
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut linea = String::new();
                r.read_line(&mut linea).unwrap();
                let mut cab = HashMap::new();
                loop {
                    let mut l = String::new();
                    r.read_line(&mut l).unwrap();
                    if l.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = l.split_once(':') {
                        cab.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let destino = linea.split_whitespace().nth(1).unwrap_or("").to_string();
                a.lock().unwrap().push(format!(
                    "{destino} if-match={} range={}",
                    cab.get("if-match").cloned().unwrap_or_default(),
                    cab.get("range").cloned().unwrap_or_default()
                ));
                let (ruta, consulta) = destino.split_once('?').unwrap_or((&destino, ""));
                let version = consulta
                    .split('&')
                    .find_map(|p| p.strip_prefix("versionId="))
                    .unwrap_or("");
                let error = |c: &str, e: &str| {
                    let x = format!("<Error><Code>{e}</Code><Message>m</Message></Error>");
                    format!(
                        "HTTP/1.1 {c}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{x}",
                        x.len()
                    )
                    .into_bytes()
                };
                let resp = if ruta != "/cubo/docs/a.pdf" {
                    error("404 Not Found", "NoSuchKey")
                } else if !["", "v1", "null"].contains(&version) {
                    error("404 Not Found", "NoSuchVersion")
                } else if cab.get("if-match").is_some_and(|m| m != ETAG) {
                    error("412 Precondition Failed", "PreconditionFailed")
                } else {
                    let total = CUERPO.len() as u64;
                    let tramo = cab
                        .get("range")
                        .map(|r| Rango::de(r).and_then(|r| r.en(total)));
                    match tramo {
                        Some(None) => error("416 Range Not Satisfiable", "InvalidRange"),
                        Some(Some((x, z))) => {
                            let t = &CUERPO[x as usize..=z as usize];
                            let mut v = format!(
                                "HTTP/1.1 206 Partial Content\r\netag: {ETAG}\r\naccept-ranges: bytes\r\ncontent-range: bytes {x}-{z}/{total}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                t.len()
                            )
                            .into_bytes();
                            v.extend_from_slice(t);
                            v
                        }
                        None => {
                            let mut v = format!(
                                "HTTP/1.1 200 OK\r\netag: {ETAG}\r\naccept-ranges: bytes\r\ncontent-type: application/pdf\r\ncontent-length: {total}\r\nconnection: close\r\n\r\n"
                            )
                            .into_bytes();
                            v.extend_from_slice(CUERPO);
                            v
                        }
                    }
                };
                let _ = c.write_all(&resp);
            }
        });
        (
            format!(
                "s3://cubo?region=eu-north-1&endpoint=http://127.0.0.1:{puerto}&access_key_id=AK&secret_access_key=S&session_token=T"
            ),
            anotado,
        )
    }

    fn pieza(fuente: &str, version: &str, etag: &str, sha256: Option<String>) -> Pieza {
        Pieza {
            coleccion: "legal.archivo.contratos".into(),
            camino: "a.pdf".into(),
            version: version.into(),
            origen: Origen::S3 {
                fuente: fuente.into(),
                clave: "docs/a.pdf".into(),
                version: version.into(),
                etag: etag.into(),
            },
            tamano: Some(CUERPO.len() as u64),
            sha256,
            tipo: None,
        }
    }

    fn cab<'a>(b: &'a Bytes, k: &str) -> Option<&'a str> {
        b.cabeceras
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }

    fn todo(mut b: Bytes) -> std::io::Result<Vec<u8>> {
        let mut v = Vec::new();
        b.lector.read_to_end(&mut v).map(|_| v)
    }

    fn sin_lago() -> Memoria {
        Memoria::default()
    }

    #[test]
    fn entero_fijado_por_version_y_etag_y_el_sha256_queda_visto() {
        let (fuente, anotado) = s3_falso();
        let vistos = Arc::new(Vistos::default());
        let b = abrir(
            &sin_lago(),
            &pieza(&fuente, "v1", ETAG, None),
            None,
            vistos.clone(),
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(b.codigo, 200);
        assert_eq!(b.largo, Some(CUERPO.len() as u64));
        assert_eq!(cab(&b, "etag"), Some(ETAG));
        assert_eq!(cab(&b, "ore-media-version"), Some("v1"));
        assert_eq!(cab(&b, "content-type"), Some("application/pdf"));
        assert!(cab(&b, "repr-digest").is_none(), "no se conocía");
        assert_eq!(todo(b).unwrap(), CUERPO);
        let pedido = anotado.lock().unwrap().last().cloned().unwrap();
        assert!(
            pedido.contains("versionId=v1") && pedido.contains(&format!("if-match={ETAG}")),
            "{pedido}"
        );
        let esperado = ore_s3::hex(&Sha256::digest(CUERPO));
        assert_eq!(
            vistos.de("legal.archivo.contratos", "a.pdf", "v1"),
            Some(esperado)
        );
    }

    #[test]
    fn un_rango_es_206_y_no_anota_nada() {
        let (fuente, anotado) = s3_falso();
        let vistos = Arc::new(Vistos::default());
        let b = abrir(
            &sin_lago(),
            &pieza(&fuente, "v1", ETAG, None),
            Some("bytes=0-3"),
            vistos.clone(),
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(b.codigo, 206);
        assert_eq!(
            cab(&b, "content-range"),
            Some(&*format!("bytes 0-3/{}", CUERPO.len()))
        );
        assert_eq!(todo(b).unwrap(), b"%PDF");
        assert!(
            anotado
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("range=bytes=0-3")
        );
        assert!(
            vistos
                .de("legal.archivo.contratos", "a.pdf", "v1")
                .is_none()
        );
        let b = abrir(
            &sin_lago(),
            &pieza(&fuente, "v1", ETAG, None),
            Some("bytes=-8"),
            vistos,
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(todo(b).unwrap(), b"contrato");
    }

    #[test]
    fn la_version_null_tambien_se_fija() {
        let (fuente, anotado) = s3_falso();
        let b = abrir(
            &sin_lago(),
            &pieza(&fuente, "null", ETAG, None),
            None,
            Arc::default(),
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(todo(b).unwrap(), CUERPO);
        assert!(
            anotado
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("versionId=null")
        );
    }

    #[test]
    fn lo_que_ya_no_es_la_version_fijada_es_media_cambiado() {
        let (fuente, _) = s3_falso();
        for (version, etag) in [("v1", "\"otro\""), ("v9", ETAG)] {
            let r = match abrir(
                &sin_lago(),
                &pieza(&fuente, version, etag, None),
                None,
                Arc::default(),
            ) {
                Err(r) => r,
                Ok(_) => panic!("{version} {etag} no debía abrir"),
            };
            assert_eq!(r.codigo, 412, "{version} {etag}");
            assert!(
                r.cuerpo.jcs().contains("media/cambiado"),
                "{}",
                r.cuerpo.jcs()
            );
        }
    }

    #[test]
    fn un_rango_que_no_cabe_o_no_se_entiende_es_416() {
        let (fuente, _) = s3_falso();
        for r in ["bytes=999-1000", "bytes=0-1,4-5", "lineas=1-2"] {
            let e = match abrir(
                &sin_lago(),
                &pieza(&fuente, "v1", ETAG, None),
                Some(r),
                Arc::default(),
            ) {
                Err(e) => e,
                Ok(_) => panic!("{r} no debía abrir"),
            };
            assert_eq!(e.codigo, 416, "{r}");
            assert!(e.cuerpo.jcs().contains("media/rango"), "{r}");
        }
    }

    #[test]
    fn el_digest_conocido_va_en_repr_digest_y_si_no_casa_se_corta() {
        let (fuente, _) = s3_falso();
        let bueno = ore_s3::hex(&Sha256::digest(CUERPO));
        let b = abrir(
            &sin_lago(),
            &pieza(&fuente, "v1", ETAG, Some(bueno.clone())),
            Some("bytes=0-3"),
            Arc::default(),
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        let rd = cab(&b, "repr-digest").unwrap().to_string();
        assert!(rd.starts_with("sha-256=:") && rd.ends_with(':'), "{rd}");
        let b = abrir(
            &sin_lago(),
            &pieza(&fuente, "v1", ETAG, Some(bueno)),
            None,
            Arc::default(),
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(todo(b).unwrap(), CUERPO);
        let malo = "00".repeat(32);
        let b = abrir(
            &sin_lago(),
            &pieza(&fuente, "v1", ETAG, Some(malo)),
            None,
            Arc::default(),
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert!(todo(b).is_err(), "el digest no casa: se corta");
        let mut p = pieza(&fuente, "v1", ETAG, None);
        p.tamano = Some(3);
        let b = abrir(&sin_lago(), &p, None, Arc::default())
            .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        // S3 dice su content-length; el tamaño que manda es el de la respuesta.
        assert!(todo(b).is_ok());
    }

    #[test]
    fn un_blob_del_lago_entero_y_por_rango() {
        let lago = sin_lago();
        let sha = ore_s3::hex(&Sha256::digest(CUERPO));
        lago.subir(&ore_store::blobs::clave_de(&sha), CUERPO)
            .unwrap();
        let p = Pieza {
            coleccion: "legal.archivo.contratos".into(),
            camino: "a.pdf".into(),
            version: "v1".into(),
            origen: Origen::Lago {
                sha256: sha.clone(),
            },
            tamano: None,
            sha256: Some(sha.clone()),
            tipo: Some("application/pdf".into()),
        };
        let b =
            abrir(&lago, &p, None, Arc::default()).unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(
            (b.codigo, cab(&b, "etag").map(String::from)),
            (200, Some(format!("\"{sha}\"")))
        );
        assert_eq!(todo(b).unwrap(), CUERPO);
        let b = abrir(&lago, &p, Some("bytes=5-"), Arc::default())
            .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(b.codigo, 206);
        assert_eq!(todo(b).unwrap(), &CUERPO[5..]);
        let e = match abrir(&lago, &p, Some("bytes=500-"), Arc::default()) {
            Err(e) => e,
            Ok(_) => panic!("no cabe"),
        };
        assert_eq!(e.codigo, 416);
    }

    #[test]
    fn rangos() {
        assert_eq!(Rango::de("bytes=0-1023"), Some(Rango::Desde(0, Some(1023))));
        assert_eq!(Rango::de("bytes=10-"), Some(Rango::Desde(10, None)));
        assert_eq!(Rango::de("bytes=-8"), Some(Rango::Sufijo(8)));
        for malo in [
            "bytes=5-1",
            "bytes=-0",
            "bytes=-",
            "bytes=a-b",
            "bytes=0-1,3-4",
        ] {
            assert_eq!(Rango::de(malo), None, "{malo}");
        }
        assert_eq!(Rango::Desde(5, Some(100)).en(10), Some((5, 9)));
        assert_eq!(Rango::Sufijo(100).en(10), Some((0, 9)));
        assert_eq!(Rango::Desde(10, None).en(10), None);
    }

    pub(crate) use ore_store_memoria::Memoria;

    /// Un lago en memoria, para las pruebas.
    mod ore_store_memoria {
        use ore_store::almacen::Almacen;
        use std::collections::BTreeMap;
        use std::sync::Mutex;

        #[derive(Default)]
        pub struct Memoria(Mutex<BTreeMap<String, Vec<u8>>>);

        impl Almacen for Memoria {
            fn base(&self) -> String {
                "mem://".into()
            }
            fn leer(&self, c: &str) -> Result<Option<String>, String> {
                Ok(self
                    .leer_bytes(c)?
                    .map(|b| String::from_utf8_lossy(&b).into_owned()))
            }
            fn existe(&self, c: &str) -> Result<bool, String> {
                Ok(self.0.lock().unwrap().contains_key(c))
            }
            fn subir(&self, c: &str, b: &[u8]) -> Result<bool, String> {
                Ok(self
                    .0
                    .lock()
                    .unwrap()
                    .insert(c.into(), b.to_vec())
                    .is_none())
            }
            fn listar(&self, p: &str) -> Result<Vec<String>, String> {
                Ok(self
                    .0
                    .lock()
                    .unwrap()
                    .keys()
                    .filter(|k| k.starts_with(p))
                    .cloned()
                    .collect())
            }
            fn borrar(&self, c: &str) -> Result<(), String> {
                self.0.lock().unwrap().remove(c);
                Ok(())
            }
            fn leer_bytes(&self, c: &str) -> Result<Option<Vec<u8>>, String> {
                Ok(self.0.lock().unwrap().get(c).cloned())
            }
        }
    }
}
