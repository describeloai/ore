//! **ORE 0072 C2, fino (F2·1)**: el descriptor de una capa y sus teselas.
//!
//! - `GET /v1/layers/{layer}/descriptor` (con sujeto): resuelve la capa —hasta F2·2,
//!   la capa ES el dataset `base[.schema].nombre` (K-F2·1-a)— en la rama de `x-ore-rama`
//!   (sin ella, `main`), su columna geo y su
//!   CRS; calcula la **clave** (la huella de lo que se pinta: protocolo, capa,
//!   `metadata_location`, columna, CRS, suelo) y la guarda con su fuente; devuelve
//!   **TileJSON 3.0** con las URLs de las teselas **ya firmadas**.
//! - `GET /v1/tiles/{layer}/{clave}/WebMercatorQuad/{z}/{x}/{y}.mvt?e=…&f=…`
//!   (**sin sujeto**: la firma es la autorización, K-F2·1-b): firma y caducidad (403),
//!   clave conocida (410: el cliente pide otro descriptor), la credencial prestada
//!   del dataset y el motor (`ore-teselas`, C3). La tesela es **inmutable**: la clave
//!   cambia con la instantánea, así que se cachea para siempre (y el borde con ella).
//!
//! La tabla clave → fuente vive en memoria (K-F2·1-e): `ore-serve` es una réplica, y
//! una clave que no está es un 410 que se arregla pidiendo otro descriptor.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ore_core::json::Json;
use ore_entrada::http::{Bytes, Peticion, Respuesta, Salida};
use sha2::{Digest, Sha256};

use crate::rutas::Servidor;

pub(crate) const TMS: &str = "WebMercatorQuad";
const MAXZOOM: u32 = 16;
/// El suelo del crudo por defecto (K-F2·1-c: lo medido para los edificios en F2·0a).
const MINZOOM: u32 = 13;
/// Cuánto vale una firma: el descriptor se pide al abrir el mapa (F2·1; más corta, F5).
const VIDA_FIRMA: u64 = 12 * 3600;
/// Cuánto se usa una credencial prestada antes de pedir otra (STS da 1 h).
const VIDA_CREDENCIAL: u64 = 45 * 60;
const PROTOCOLO: u32 = 1;

#[derive(Clone)]
struct Fuente {
    capa: String,
    /// La rama del descriptor: la credencial se renueva en ella.
    rama: Option<String>,
    metadata_location: String,
    geometria: String,
    crs: String,
    minzoom: u32,
    credencial: Json,
    credencial_hasta: u64,
}

fn tabla() -> &'static Mutex<HashMap<String, Fuente>> {
    static T: OnceLock<Mutex<HashMap<String, Fuente>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// El secreto de la firma: el de la celda (`ORE_TESELAS_SECRETO`) o, sin él, uno al
/// azar por proceso (las claves también viven en memoria: mueren juntos).
fn secreto() -> &'static [u8; 32] {
    static S: OnceLock<[u8; 32]> = OnceLock::new();
    S.get_or_init(|| {
        if let Ok(s) = std::env::var("ORE_TESELAS_SECRETO")
            && !s.is_empty()
        {
            return sha256(s.as_bytes());
        }
        let mut b = [0u8; 32];
        if std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut b))
            .is_ok()
        {
            return b;
        }
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(ahora_ns());
        h.write_u32(std::process::id());
        sha256(&h.finish().to_le_bytes())
    })
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

fn hmac(clave: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    k[..clave.len().min(64)].copy_from_slice(&clave[..clave.len().min(64)]);
    let mut i = Sha256::new();
    i.update(k.map(|b| b ^ 0x36));
    i.update(msg);
    let mut o = Sha256::new();
    o.update(k.map(|b| b ^ 0x5c));
    o.update(i.finalize());
    o.finalize().into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn ahora_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn ahora() -> u64 {
    (ahora_ns() / 1_000_000_000) as u64
}

/// La firma de una clave hasta `e` (32 hex: 128 bits).
pub(crate) fn firma(clave: &str, e: u64) -> String {
    hex(&hmac(secreto(), format!("{clave}|{e}").as_bytes()))[..32].to_string()
}

/// Comparar sin dar pistas por el tiempo.
fn iguales(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn consulta(cruda: &str) -> HashMap<&str, &str> {
    cruda
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .collect()
}

type Datos = (String, String, String, Json);

/// Lo que el dataset dice de sí en `rama` (`datos_en`, con su vuelta a `main`):
/// `(metadata_location, columna, crs, credencial)`.
fn resolver(srv: &Servidor, capa: &str, rama: Option<&str>) -> Result<Datos, Respuesta> {
    de_los_datos(srv.datos_en(rama.map(String::from), capa), capa)
}

fn de_los_datos(r: Respuesta, capa: &str) -> Result<Datos, Respuesta> {
    if r.codigo != 200 {
        return Err(r);
    }
    let Json::Obj(m) = &r.cuerpo else {
        return Err(Respuesta::error(500, "`datos` no devolvió un objeto"));
    };
    let ml = match m.get("metadata_location") {
        Some(Json::Str(s)) if !s.is_empty() => s.clone(),
        _ => {
            return Err(Respuesta::error(
                409,
                format!(
                    "`{capa}` no tiene una copia Iceberg que pintar (un sobre heredado no se tesela)"
                ),
            ));
        }
    };
    let Some(Json::Obj(geo)) = m.get("geo") else {
        return Err(Respuesta::error(
            422,
            format!("`{capa}` no tiene columnas geo"),
        ));
    };
    // La primera columna geo (por nombre): una capa pinta una geometría (F5: elegirla).
    let Some((col, g)) = geo.iter().next() else {
        return Err(Respuesta::error(
            422,
            format!("`{capa}` no tiene columnas geo"),
        ));
    };
    let crs = match g {
        Json::Obj(g) => match g.get("crs") {
            Some(Json::Str(c)) => c.clone(),
            _ => "OGC:CRS84".to_string(),
        },
        _ => "OGC:CRS84".to_string(),
    };
    let credencial = m
        .get("credencial")
        .cloned()
        .unwrap_or(Json::Obj(Default::default()));
    Ok((ml, col.clone(), crs, credencial))
}

/// `GET /v1/layers/{layer}/descriptor`: TileJSON 3.0, con las teselas firmadas.
/// La clave no lleva la rama: la lleva su `metadata_location` (otra instantánea, otra
/// clave; la misma, las mismas teselas).
pub(crate) fn descriptor(
    srv: &Servidor,
    p: &Peticion,
    capa: &str,
    rama: Option<&str>,
) -> Respuesta {
    descriptor_de(resolver(srv, capa, rama), p, capa, rama)
}

fn descriptor_de(
    datos: Result<Datos, Respuesta>,
    p: &Peticion,
    capa: &str,
    rama: Option<&str>,
) -> Respuesta {
    let (ml, geometria, crs, credencial) = match datos {
        Ok(v) => v,
        Err(r) => return r,
    };
    let q = consulta(&p.consulta_cruda);
    let minzoom = q
        .get("minzoom")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|z| *z <= MAXZOOM)
        .unwrap_or(MINZOOM);
    let clave = hex(&sha256(
        format!("v{PROTOCOLO}|{capa}|{ml}|{geometria}|{crs}|{minzoom}").as_bytes(),
    ))[..32]
        .to_string();
    tabla().lock().unwrap().insert(
        clave.clone(),
        Fuente {
            capa: capa.to_string(),
            rama: rama.map(String::from),
            metadata_location: ml.clone(),
            geometria: geometria.clone(),
            crs,
            minzoom,
            credencial,
            credencial_hasta: ahora() + VIDA_CREDENCIAL,
        },
    );
    let e = ahora() + VIDA_FIRMA;
    let f = firma(&clave, e);
    // Las teselas, por la entrada por la que llegó el descriptor (la de la celda).
    let host = p
        .cabeceras
        .get("x-forwarded-host")
        .or_else(|| p.cabeceras.get("host"))
        .cloned()
        .unwrap_or_default();
    let esquema = p
        .cabeceras
        .get("x-forwarded-proto")
        .cloned()
        .unwrap_or_else(|| {
            if host.starts_with("localhost") || host.starts_with("127.") {
                "http".into()
            } else {
                "https".into()
            }
        });
    let url = format!(
        "{esquema}://{host}/v1/tiles/{capa}/{clave}/{TMS}/{{z}}/{{x}}/{{y}}.mvt?e={e}&f={f}"
    );
    let z = |v: u32| Json::Int(v as i64);
    Respuesta::ok(Json::obj([
        ("tilejson", Json::s("3.0.0")),
        ("name", Json::s(capa)),
        ("scheme", Json::s("xyz")),
        ("tiles", Json::Arr(vec![Json::s(url)])),
        ("minzoom", z(minzoom)),
        ("maxzoom", z(MAXZOOM)),
        (
            "bounds",
            Json::Crudo("[-180,-85.0511287798066,180,85.0511287798066]".into()),
        ),
        (
            "vector_layers",
            Json::Arr(vec![Json::obj([
                ("id", Json::s(capa)),
                ("fields", Json::Obj(Default::default())),
                ("minzoom", z(minzoom)),
                ("maxzoom", z(MAXZOOM)),
            ])]),
        ),
        ("attribution", Json::s(format!("ORE · {capa}"))),
        (
            "ore",
            Json::obj([
                ("clave", Json::s(&clave)),
                ("metadata_location", Json::s(ml)),
                ("geometria", Json::s(geometria)),
                ("expira", Json::Int(e as i64)),
            ]),
        ),
    ]))
}

fn error(codigo: u16, m: impl Into<String>) -> Salida {
    let m = m.into();
    Salida::Bytes(Bytes {
        codigo,
        cabeceras: vec![
            ("content-type".into(), "application/json".into()),
            ("access-control-allow-origin".into(), "*".into()),
        ],
        largo: None,
        lector: Box::new(std::io::Cursor::new(
            Json::obj([("error", Json::s(m))]).jcs().into_bytes(),
        )),
        finales: None,
    })
}

/// Lo que se decide sin el motor: la forma, la firma, la clave y la pirámide.
fn comprobar(p: &Peticion, seg: &[&str]) -> Result<(Fuente, u32, u32, u32), Salida> {
    let ["v1", "tiles", capa, clave, tms, z, x, y] = seg else {
        return Err(error(
            404,
            "una tesela es /v1/tiles/{layer}/{clave}/WebMercatorQuad/{z}/{x}/{y}.mvt",
        ));
    };
    if *tms != TMS {
        return Err(error(404, format!("`{tms}`: sólo {TMS} (F2)")));
    }
    let (Ok(z), Ok(x), Some(Ok(y))) = (
        z.parse::<u32>(),
        x.parse::<u32>(),
        y.strip_suffix(".mvt").map(|y| y.parse::<u32>()),
    ) else {
        return Err(error(404, "z, x e y son enteros y la tesela acaba en .mvt"));
    };
    let q = consulta(&p.consulta_cruda);
    let (Some(e), Some(f)) = (q.get("e").and_then(|e| e.parse::<u64>().ok()), q.get("f")) else {
        return Err(error(
            403,
            "una tesela va firmada (`e` y `f`): pide el descriptor",
        ));
    };
    if e < ahora() {
        return Err(error(403, "la firma caducó: pide otro descriptor"));
    }
    if !iguales(&firma(clave, e), f) {
        return Err(error(403, "la firma no es de esta clave"));
    }
    let Some(fuente) = tabla().lock().unwrap().get(*clave).cloned() else {
        return Err(error(410, "esa clave ya no está: pide otro descriptor"));
    };
    if fuente.capa != *capa {
        return Err(error(404, "la clave es de otra capa"));
    }
    if z < fuente.minzoom || z > MAXZOOM || x >= 1 << z || y >= 1 << z {
        return Err(error(
            404,
            format!("fuera de la pirámide: z{} a z{MAXZOOM}", fuente.minzoom),
        ));
    }
    Ok((fuente, z, x, y))
}

/// `GET /v1/tiles/{layer}/{clave}/{tms}/{z}/{x}/{y}.mvt?e=…&f=…`
pub(crate) fn tesela(srv: &Servidor, p: &Peticion, seg: &[&str]) -> Salida {
    let (mut fuente, z, x, y) = match comprobar(p, seg) {
        Ok(v) => v,
        Err(s) => return s,
    };
    let (capa, clave) = (seg[2], seg[3]);
    // La credencial prestada caduca (STS, 1 h): se pide otra, para el mismo dataset.
    if ahora() >= fuente.credencial_hasta {
        match resolver(srv, capa, fuente.rama.as_deref()) {
            Ok((_, _, _, c)) => {
                fuente.credencial = c;
                fuente.credencial_hasta = ahora() + VIDA_CREDENCIAL;
                tabla()
                    .lock()
                    .unwrap()
                    .insert(clave.to_string(), fuente.clone());
            }
            Err(r) => return Salida::Una(r),
        }
    }
    let s = |v: &str| Json::s(v);
    let n = |v: u64| Json::s(v.to_string());
    let cuerpo = Json::obj([
        ("protocolo", n(PROTOCOLO as u64)),
        ("tms", s(TMS)),
        ("z", n(z as u64)),
        ("x", n(x as u64)),
        ("y", n(y as u64)),
        ("formato", s("mvt")),
        ("capa", s(capa)),
        (
            "fuente",
            Json::obj([
                ("metadata_location", s(&fuente.metadata_location)),
                ("geometria", s(&fuente.geometria)),
                ("crs", s(&fuente.crs)),
            ]),
        ),
        (
            "limites",
            Json::obj([("plazo_ms", n(10_000)), ("max_bytes", n(2_000_000))]),
        ),
        ("credencial", fuente.credencial.clone()),
    ])
    .jcs();
    let motor = std::env::var("ORE_TESELAS").unwrap_or_else(|_| "ore-teselas:8097".into());
    let (codigo, cabeceras, bytes) = match al_motor(&motor, &cuerpo) {
        Ok(v) => v,
        Err(m) => return error(502, m),
    };
    let de = |k: &str| {
        cabeceras
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.clone())
    };
    match codigo {
        200 | 204 => {
            let mut c = vec![
                (
                    "cache-control".to_string(),
                    "public, max-age=31536000, immutable".to_string(),
                ),
                ("access-control-allow-origin".into(), "*".into()),
            ];
            if codigo == 200 {
                c.push((
                    "content-type".into(),
                    "application/vnd.mapbox-vector-tile".into(),
                ));
            }
            for k in ["etag", "x-ore-cuenta"] {
                if let Some(v) = de(k) {
                    c.push((k.into(), v));
                }
            }
            Salida::Bytes(Bytes {
                codigo,
                cabeceras: c,
                largo: Some(bytes.len() as u64),
                lector: Box::new(std::io::Cursor::new(bytes)),
                finales: None,
            })
        }
        422 | 503 => error(codigo, String::from_utf8_lossy(&bytes).to_string()),
        otro => error(
            502,
            format!(
                "`ore-teselas` contestó {otro}: {}",
                String::from_utf8_lossy(&bytes)
            ),
        ),
    }
}

/// `POST /v1/teselas` al motor, por HTTP plano dentro de la celda (como `al_motor` de
/// `preview.rs`, con el cuerpo en bytes): `(código, cabeceras, cuerpo)`.
/// `(código, cabeceras, cuerpo)` de lo que contestó el motor.
type DelMotor = (u16, Vec<(String, String)>, Vec<u8>);

fn al_motor(motor: &str, cuerpo: &str) -> Result<DelMotor, String> {
    use std::net::ToSocketAddrs as _;
    let dir = motor
        .to_socket_addrs()
        .ok()
        .and_then(|mut d| d.next())
        .ok_or_else(|| format!("`ore-teselas` (`{motor}`) no tiene dirección"))?;
    let mut s = std::net::TcpStream::connect_timeout(&dir, Duration::from_secs(5))
        .map_err(|e| format!("`ore-teselas` no contesta: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let req = format!(
        "POST /v1/teselas HTTP/1.1\r\nhost: teselas\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
        cuerpo.len()
    );
    s.write_all(req.as_bytes())
        .map_err(|e| format!("`ore-teselas`: {e}"))?;
    let mut todo = Vec::new();
    s.read_to_end(&mut todo)
        .map_err(|e| format!("`ore-teselas`: {e}"))?;
    let fin = todo
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("`ore-teselas` contestó algo que no es HTTP")?;
    let cabeza = String::from_utf8_lossy(&todo[..fin]).to_string();
    let mut lineas = cabeza.lines();
    let codigo = lineas
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(502);
    let cabeceras = lineas
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let mut cuerpo = todo[fin + 4..].to_vec();
    // `ore-entrada` trocea lo que no sabe medir: el error JSON de un 4xx/5xx
    if cabeza
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        cuerpo = destrocear(&cuerpo);
    }
    Ok((codigo, cabeceras, cuerpo))
}

fn destrocear(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(fin) = b[i..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let n =
            usize::from_str_radix(String::from_utf8_lossy(&b[i..i + fin]).trim(), 16).unwrap_or(0);
        i += fin + 2;
        if n == 0 || i + n > b.len() {
            break;
        }
        out.extend_from_slice(&b[i..i + n]);
        i += n + 2;
    }
    out
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn hmac_es_el_de_la_rfc_4231() {
        // RFC 4231, caso 2
        let h = hmac(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&h),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn la_firma_es_de_una_clave_y_un_momento() {
        let f = firma("abc", 100);
        assert_eq!(f.len(), 32);
        assert_eq!(f, firma("abc", 100));
        assert_ne!(f, firma("abd", 100));
        assert_ne!(f, firma("abc", 101));
        assert!(iguales(&f, &firma("abc", 100)));
        assert!(!iguales(&f, &f[..31]));
    }

    fn pet(ruta: &str, consulta: &str) -> Peticion {
        Peticion {
            metodo: "GET".into(),
            ruta: ruta.into(),
            cabeceras: [("host".to_string(), "victor.ore.paladio.io".to_string())].into(),
            cuerpo: String::new(),
            consulta: Default::default(),
            consulta_cruda: consulta.into(),
        }
    }

    fn datos(geo: &str, ml: &str) -> Result<Datos, Respuesta> {
        let j = format!(
            r#"{{"dataset":"geo.edificios","metadata_location":"{ml}","geo":{geo},"credencial":{{"gcs.oauth2.token":"t"}}}}"#
        );
        let n = ore_core::parse::parse(&j).unwrap();
        de_los_datos(Respuesta::ok(Json::de_node(&n)), "geo.edificios")
    }

    fn codigo(s: Result<(Fuente, u32, u32, u32), Salida>) -> u16 {
        match s {
            Ok(_) => 200,
            Err(Salida::Bytes(b)) => b.codigo,
            Err(Salida::Una(r)) => r.codigo,
            Err(_) => 0,
        }
    }

    /// El descriptor da una URL firmada; con ella la tesela pasa, y con cualquier
    /// cosa cambiada no: firma (403), caducidad (403), clave (410), pirámide (404).
    #[test]
    fn el_descriptor_firma_y_la_tesela_lo_comprueba() {
        let d = datos(
            r#"{"geom":{"crs":"EPSG:4326"}}"#,
            "gs://b/ore/v2/datasets/geo_edificios/metadata/1.metadata.json",
        );
        let r = descriptor_de(
            d,
            &pet("/v1/layers/geo.edificios/descriptor", "minzoom=12"),
            "geo.edificios",
            Some("ana/geo"),
        );
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""tilejson":"3.0.0""#) && j.contains(r#""minzoom":12"#),
            "{j}"
        );
        let url = j
            .split("https://victor.ore.paladio.io")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let (ruta, q) = url.split_once('?').unwrap();
        let ruta = ruta.replace("{z}/{x}/{y}", "14/8023/6177");
        let seg: Vec<&str> = ruta.trim_start_matches('/').split('/').collect();
        assert_eq!(codigo(comprobar(&pet(&ruta, q), &seg)), 200);
        // la fuente recuerda la rama: la credencial se renueva en ella
        let Ok((fuente, ..)) = comprobar(&pet(&ruta, q), &seg) else {
            unreachable!()
        };
        assert_eq!(fuente.rama.as_deref(), Some("ana/geo"));
        // la firma de otra cosa
        // (otro último carácter: si ya era `0`, `0` la dejaba igual una vez de cada 16)
        let otro = if q.ends_with('0') { '1' } else { '0' };
        let q_mala = format!("{}{otro}", &q[..q.len() - 1]);
        assert_eq!(codigo(comprobar(&pet(&ruta, &q_mala), &seg)), 403);
        // sin firma
        assert_eq!(codigo(comprobar(&pet(&ruta, ""), &seg)), 403);
        // caducada, aunque bien firmada
        let clave = seg[3];
        let vieja = format!("e=100&f={}", firma(clave, 100));
        assert_eq!(codigo(comprobar(&pet(&ruta, &vieja), &seg)), 403);
        // una clave que no está, bien firmada: 410 (pide otro descriptor)
        let e = ahora() + 60;
        let otra = ruta.replace(clave, "0000000000000000000000000000beef");
        let seg_otra: Vec<&str> = otra.trim_start_matches('/').split('/').collect();
        let q_otra = format!("e={e}&f={}", firma("0000000000000000000000000000beef", e));
        assert_eq!(codigo(comprobar(&pet(&otra, &q_otra), &seg_otra)), 410);
        // por debajo del suelo: 404
        let baja = ruta.replace("14/8023/6177", "11/1002/772");
        let seg_baja: Vec<&str> = baja.trim_start_matches('/').split('/').collect();
        assert_eq!(codigo(comprobar(&pet(&baja, q), &seg_baja)), 404);
    }

    #[test]
    fn sin_geo_o_sin_copia_iceberg_no_hay_descriptor() {
        let sin_geo = descriptor_de(
            datos("{}", "gs://b/x/metadata/1.json"),
            &pet("/", ""),
            "geo.x",
            None,
        );
        assert_eq!(sin_geo.codigo, 422);
        let sin_copia = descriptor_de(datos(r#"{"geom":{}}"#, ""), &pet("/", ""), "geo.x", None);
        assert_eq!(sin_copia.codigo, 409);
        let no_esta = descriptor_de(
            Err(Respuesta::error(404, "no hay")),
            &pet("/", ""),
            "geo.x",
            None,
        );
        assert_eq!(no_esta.codigo, 404);
    }

    #[test]
    fn destrocear_junta_los_trozos() {
        assert_eq!(
            destrocear(b"4\r\nhola\r\n3\r\n ok\r\n0\r\n\r\n"),
            b"hola ok"
        );
    }
}
