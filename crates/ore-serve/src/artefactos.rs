//! **La puerta Maven de los registros de artefactos** (ADR 0074 AR1·2).
//!
//! Maven habla con ORE como con cualquier repositorio —`mvn deploy` y la
//! resolución de dependencias, sin plugin ni paso propio— y **cada petición la
//! decide este proceso**: quién es (el token), en qué registro, si la ruta
//! cuadra con la coordenada, si la versión vale. Los bytes los guarda y los
//! sirve `ore-medios` (AR1·1); aquí **pasan en flujo**, sin guardarse, con la
//! memoria constante. Es la única excepción a «`ore-serve` no pasa bytes», y
//! se acota a `/artifacts/`: desde fuera de la celda no hay otra puerta.
//!
//! ```text
//! PUT  /artifacts/<b>/<s>/<registro>/maven/<g…>/<a>/<v>/<a>-<v>[-clas].<ext>   publicar (en flujo a ore-medios)
//! PUT  …/<fichero>.sha1|.md5|.sha256|.sha512                                    se coteja con lo calculado; no se guarda
//! PUT  …/<g…>/<a>/maven-metadata.xml[.sha1|.md5]                                se acepta y se tira: se genera
//! GET  /artifacts/<b>/<s>/<registro>/maven/…   y   /artifacts/maven/…           leer (la de la celda encuentra el registro por la coordenada)
//! ```
//!
//! **La credencial** es HTTP Basic (lo único que Maven sabe mandar): el usuario
//! da igual y la contraseña es un token `ore_art_…` que este proceso firmó (D-A9).
//! Sin ella, `401` con `WWW-Authenticate`: Maven no manda credenciales hasta que
//! se le piden (AR0 P4).

use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::{Bytes, Peticion, Respuesta, Salida, Subida};
use sha2::Digest;
use std::io::Read;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Los plazos con `ore-medios`: un jar de 100 MiB por la red de la celda cabe
/// de sobra en esto.
const PLAZOS: ore_entrada::http::Plazos = ore_entrada::http::Plazos {
    conectar: Duration::from_secs(5),
    responder: Duration::from_secs(120),
};

/// Si la petición sube en flujo: sólo el `PUT` de un artefacto.
pub fn es_subida(p: &Peticion) -> bool {
    p.metodo == "PUT" && p.ruta.starts_with("/artifacts/")
}

// ── el token ────────────────────────────────────────────────────────────────

/// Lo que un token dice: quién, sobre qué registro (`b.s.r`, o `*` para toda
/// la celda: el de leer de la capa), qué verbo y hasta cuándo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carga {
    pub persona: String,
    pub registro: String,
    pub verbo: String,
    pub caduca: u64,
    /// La decisión de `ore-iam` que dejó emitirlo (AR1·3): cada publicación la
    /// cita en la actividad, porque Maven no trae el token de la persona y
    /// `ore-iam` atribuye por ella (0047 A6.3, como un puesto). `None` en un
    /// banco sin `ore-iam`.
    pub decision: Option<String>,
}

const PREFIJO_DEL_TOKEN: &str = "ore_art_";

/// La clave de la firma: la de la celda (`ORE_ARTEFACTOS_SECRETO`) o, sin ella,
/// una al azar por proceso —el patrón de `teselas.rs`—: **un reinicio invalida
/// los tokens emitidos**, y se dice.
fn secreto() -> &'static [u8; 32] {
    static S: OnceLock<[u8; 32]> = OnceLock::new();
    S.get_or_init(|| {
        if let Ok(s) = std::env::var("ORE_ARTEFACTOS_SECRETO")
            && !s.is_empty()
        {
            return sha2::Sha256::digest(s.as_bytes()).into();
        }
        let mut b = [0u8; 32];
        if std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut b))
            .is_ok()
        {
            return b;
        }
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        sha2::Sha256::digest(format!("{t}{}", std::process::id()).as_bytes()).into()
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn de_hex(t: &str) -> Option<Vec<u8>> {
    if !t.len().is_multiple_of(2) {
        return None;
    }
    (0..t.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(t.get(i..i + 2)?, 16).ok())
        .collect()
}

fn ahora() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `ore_art_<hex de la carga>.<hex del hmac>`. Hex y no base64: cabe en una
/// contraseña de `settings.xml` sin escapar nada.
pub fn firmar(c: &Carga) -> String {
    let mut campos = vec![
        ("p", Json::s(&c.persona)),
        ("r", Json::s(&c.registro)),
        ("v", Json::s(&c.verbo)),
        ("e", Json::Int(c.caduca as i64)),
    ];
    if let Some(d) = &c.decision {
        campos.push(("d", Json::s(d)));
    }
    let carga = Json::obj(campos).jcs();
    let firma = crate::teselas::hmac(secreto(), carga.as_bytes());
    format!(
        "{PREFIJO_DEL_TOKEN}{}.{}",
        hex(carga.as_bytes()),
        hex(&firma)
    )
}

/// La carga de un token bien firmado y vigente; si no, por qué no.
pub fn verificar(token: &str) -> Result<Carga, &'static str> {
    let (carga, firma) = token
        .strip_prefix(PREFIJO_DEL_TOKEN)
        .and_then(|t| t.split_once('.'))
        .ok_or("not an ORE artifact token")?;
    let carga = de_hex(carga).ok_or("not an ORE artifact token")?;
    let firma = de_hex(firma).ok_or("not an ORE artifact token")?;
    let esperada = crate::teselas::hmac(secreto(), &carga);
    // Comparación sin atajos: el tiempo no dice cuántos bytes coinciden.
    if firma.len() != esperada.len()
        || firma
            .iter()
            .zip(esperada.iter())
            .fold(0u8, |a, (x, y)| a | (x ^ y))
            != 0
    {
        return Err(
            "the token is not signed by this cell (or the cell restarted: generate a new one)",
        );
    }
    let texto = String::from_utf8(carga).map_err(|_| "not an ORE artifact token")?;
    let n = ore_core::parse::parse(&texto).map_err(|_| "not an ORE artifact token")?;
    let campo = |k: &str| campo(&n, k).ok_or("not an ORE artifact token");
    let caduca: u64 = campo("e")?
        .parse()
        .map_err(|_| "not an ORE artifact token")?;
    if caduca <= ahora() {
        return Err("the token expired: generate a new one");
    }
    Ok(Carga {
        persona: campo("p")?,
        registro: campo("r")?,
        verbo: campo("v")?,
        caduca,
        decision: self::campo(&n, "d"),
    })
}

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
}

/// La contraseña de un `Authorization: Basic …`.
fn de_basic(p: &Peticion) -> Option<String> {
    let v = p.cabeceras.get("authorization")?;
    let (esquema, valor) = v.split_once(' ')?;
    if !esquema.eq_ignore_ascii_case("basic") {
        return None;
    }
    let claro = de_base64(valor.trim())?;
    let claro = String::from_utf8(claro).ok()?;
    claro.split_once(':').map(|(_, c)| c.to_string())
}

fn de_base64(t: &str) -> Option<Vec<u8>> {
    let valor = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let limpio: Vec<u8> = t.bytes().filter(|c| *c != b'=').collect();
    let mut salida = Vec::with_capacity(limpio.len() * 3 / 4);
    for trozo in limpio.chunks(4) {
        let mut n = 0u32;
        for (i, c) in trozo.iter().enumerate() {
            n |= valor(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        salida.extend_from_slice(&bytes[1..trozo.len()]);
    }
    Some(salida)
}

// ── la ruta de Maven ────────────────────────────────────────────────────────

/// Qué se pide de una librería.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Que {
    /// Un fichero de una versión.
    Fichero { version: String, nombre: String },
    /// Su suma (`sha1`, `md5`, `sha256`, `sha512`).
    Suma {
        version: String,
        nombre: String,
        algoritmo: String,
    },
    /// `maven-metadata.xml` de la librería, o su suma.
    Metadata { algoritmo: Option<String> },
}

/// Una ruta de Maven: el registro (si es la dirección de uno), la coordenada
/// (`grupo:artefacto`) y lo que se pide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruta {
    pub registro: Option<(String, String, String)>,
    pub grupo: String,
    pub artefacto: String,
    pub que: Que,
}

impl Ruta {
    pub fn coordenada(&self) -> String {
        format!("{}:{}", self.grupo, self.artefacto)
    }
}

const SUMAS: &[&str] = &["sha1", "md5", "sha256", "sha512"];

fn identificador(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 100
        && x.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// El alfabeto de una coordenada (el de la capa de la JVM), sin empezar por `.`.
fn parte(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 200
        && !x.starts_with('.')
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

pub fn ruta_de(camino: &str) -> Result<Ruta, String> {
    let resto = camino
        .strip_prefix("/artifacts/")
        .ok_or("not an artifact route")?;
    let seg: Vec<&str> = resto.split('/').collect();
    let (registro, maven) = match seg.as_slice() {
        ["maven", m @ ..] => (None, m),
        [b, s, r, "maven", m @ ..] => {
            for x in [b, s, r] {
                if !identificador(x) {
                    return Err(format!("`{x}` is not a name of a registry path"));
                }
            }
            (Some((b.to_string(), s.to_string(), r.to_string())), m)
        }
        _ => {
            return Err(
                "a Maven path is `/artifacts/<base>/<schema>/<registry>/maven/…` (to publish) or `/artifacts/maven/…`"
                    .into(),
            );
        }
    };
    if maven.iter().any(|x| !parte(x)) {
        return Err(
            "a Maven path segment is empty, starts with `.` or has a character Maven does not use"
                .into(),
        );
    }
    let n = maven.len();
    let fichero = *maven.last().ok_or("an empty Maven path")?;
    if let Some(sufijo) = fichero.strip_prefix("maven-metadata.xml") {
        let algoritmo = match sufijo {
            "" => None,
            s => Some(
                s.strip_prefix('.')
                    .filter(|a| SUMAS.contains(a))
                    .ok_or("not a metadata file")?
                    .to_string(),
            ),
        };
        if n < 3 {
            return Err("`maven-metadata.xml` goes under `<group…>/<artifact>/`".into());
        }
        return Ok(Ruta {
            registro,
            grupo: maven[..n - 2].join("."),
            artefacto: maven[n - 2].to_string(),
            que: Que::Metadata { algoritmo },
        });
    }
    if n < 4 {
        return Err("a Maven file goes under `<group…>/<artifact>/<version>/`".into());
    }
    let (artefacto, version) = (maven[n - 3], maven[n - 2]);
    let (nombre, algoritmo) = match fichero.rsplit_once('.') {
        Some((nombre, a)) if SUMAS.contains(&a) => (nombre, Some(a.to_string())),
        _ => (fichero, None),
    };
    // `<a>-<v>` y después `.` (la extensión) o `-` (un clasificador).
    let base = format!("{artefacto}-{version}");
    if !nombre
        .strip_prefix(&base)
        .is_some_and(|r| r.starts_with('.') || r.starts_with('-'))
    {
        return Err(format!(
            "`{fichero}` does not belong to `{artefacto}` `{version}`: it should start with `{base}`"
        ));
    }
    let grupo = maven[..n - 3].join(".");
    let que = match algoritmo {
        Some(algoritmo) => Que::Suma {
            version: version.to_string(),
            nombre: nombre.to_string(),
            algoritmo,
        },
        None => Que::Fichero {
            version: version.to_string(),
            nombre: nombre.to_string(),
        },
    };
    Ok(Ruta {
        registro,
        grupo,
        artefacto: artefacto.to_string(),
        que,
    })
}

// ── maven-metadata.xml ──────────────────────────────────────────────────────

/// El orden de versiones de Maven, lo bastante: por tramos (`.` y `-`), los
/// numéricos como números y antes que un calificador en el mismo sitio.
pub fn orden_de_versiones(a: &str, b: &str) -> std::cmp::Ordering {
    let tramos = |v: &str| -> Vec<String> { v.split(['.', '-']).map(str::to_string).collect() };
    let (ta, tb) = (tramos(a), tramos(b));
    for i in 0..ta.len().max(tb.len()) {
        let (x, y) = (ta.get(i), tb.get(i));
        let o = match (x, y) {
            (Some(x), Some(y)) => match (x.parse::<u64>(), y.parse::<u64>()) {
                (Ok(m), Ok(n)) => m.cmp(&n),
                (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
                (Err(_), Ok(_)) => std::cmp::Ordering::Less,
                _ => x.cmp(y),
            },
            // `1.0` < `1.0.1`; `1.0` > `1.0-rc1`.
            (Some(x), None) => {
                if x.parse::<u64>().is_ok() {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                }
            }
            (None, Some(y)) => {
                if y.parse::<u64>().is_ok() {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            }
            (None, None) => std::cmp::Ordering::Equal,
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

pub fn metadata(grupo: &str, artefacto: &str, versiones: &[String], cuando: &str) -> String {
    let mut v: Vec<&String> = versiones.iter().collect();
    v.sort_by(|a, b| orden_de_versiones(a, b));
    let ultima = v.last().map(|s| s.as_str()).unwrap_or_default();
    let lista: String = v
        .iter()
        .map(|x| format!("      <version>{x}</version>\n"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<metadata>\n  <groupId>{grupo}</groupId>\n  <artifactId>{artefacto}</artifactId>\n  <versioning>\n    <latest>{ultima}</latest>\n    <release>{ultima}</release>\n    <versions>\n{lista}    </versions>\n    <lastUpdated>{cuando}</lastUpdated>\n  </versioning>\n</metadata>\n"
    )
}

fn suma_de(algoritmo: &str, bytes: &[u8]) -> String {
    match algoritmo {
        "sha1" => hex(&sha1::Sha1::digest(bytes)),
        "md5" => hex(&md5::Md5::digest(bytes)),
        "sha512" => hex(&sha2::Sha512::digest(bytes)),
        _ => hex(&sha2::Sha256::digest(bytes)),
    }
}

/// `yyyyMMddHHmmss` en UTC, el `lastUpdated` de Maven.
fn marca_de_maven() -> String {
    let s = ahora() as i64;
    let (dias, resto) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02}",
        resto / 3600,
        (resto % 3600) / 60,
        resto % 60
    )
}

// ── las respuestas ──────────────────────────────────────────────────────────

fn error(codigo: u16, motivo: impl Into<String>) -> Salida {
    Salida::Una(Respuesta::error(codigo, motivo))
}

/// `401` con el reto Basic: sin él, Maven no manda la credencial.
fn pide_credencial(motivo: &str) -> Salida {
    let cuerpo = Json::obj([("error", Json::s(motivo))]).jcs().into_bytes();
    Salida::Bytes(Bytes {
        codigo: 401,
        cabeceras: vec![
            ("www-authenticate".into(), "Basic realm=\"ore\"".into()),
            ("content-type".into(), "application/json".into()),
        ],
        largo: Some(cuerpo.len() as u64),
        lector: Box::new(std::io::Cursor::new(cuerpo)),
        finales: None,
    })
}

fn texto_plano(codigo: u16, tipo: &str, cuerpo: Vec<u8>, cabeza: bool) -> Salida {
    let largo = cuerpo.len() as u64;
    Salida::Bytes(Bytes {
        codigo,
        cabeceras: vec![("content-type".into(), tipo.into())],
        largo: Some(largo),
        // HEAD: el largo y nada más.
        lector: if cabeza {
            Box::new(std::io::empty())
        } else {
            Box::new(std::io::Cursor::new(cuerpo))
        },
        finales: None,
    })
}

fn medios() -> Result<String, Salida> {
    std::env::var(crate::medios::ENTORNO)
        .ok()
        .filter(|d| !d.is_empty())
        .ok_or_else(|| {
            error(
                503,
                "this cell does not have `ore-medios` yet: artifacts are stored there (0074 AR1·1)",
            )
        })
}

// ── el registro y lo que tiene ──────────────────────────────────────────────

/// Los formatos de un registro y su dueño (`owner`), del árbol.
type FormatosYDueno = (Vec<String>, Option<String>);

/// Un fichero publicado, como lo cuenta `ore-medios`.
#[derive(Debug, Clone)]
struct Publicado {
    version: String,
    nombre: String,
    sha1: String,
    md5: String,
    sha256: String,
    tamano: u64,
}

impl Servidor {
    /// Los formatos que el árbol (`main`) declara para un registro, o `None`
    /// si no hay `ArtifactRegistry` con ese nombre.
    fn formatos_del_registro(
        &self,
        b: &str,
        s: &str,
        r: &str,
    ) -> Result<Option<Vec<String>>, Salida> {
        self.registro_del_arbol(b, s, r)
            .map(|h| h.map(|(formatos, _)| formatos))
            .map_err(Salida::Una)
    }

    /// Un `ArtifactRegistry` del árbol (`main`): sus formatos y su dueño, o
    /// `None` si no hay uno con ese nombre.
    fn registro_del_arbol(
        &self,
        b: &str,
        s: &str,
        r: &str,
    ) -> Result<Option<FormatosYDueno>, Respuesta> {
        let mut hallado: Option<Option<FormatosYDueno>> = None;
        let lectura = self.leyendo(|raiz| {
            let doc = crate::medios::documento_en(raiz, b, "ArtifactRegistry", s, r);
            hallado = Some(doc.map(|n| {
                let spec = n.get("spec").map(|(_, sp)| sp);
                let formatos = spec
                    .and_then(|sp| sp.get("formats"))
                    .map(|(_, f)| {
                        f.items()
                            .iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                (formatos, spec.and_then(|sp| campo(sp, "owner")))
            }));
            Respuesta::sin_contenido()
        });
        hallado.ok_or(lectura)
    }

    /// **`POST /artifact-registries/{b}/{s}/{r}/token`** (AR1·3, D-A9): el
    /// token con el que Maven publica en un registro. Lo emite quien es dueño
    /// del registro (0052) o tiene `artefacto:gestionar`; y todos, además,
    /// `artefacto:publicar` (de todo el que pertenece, `iam/migraciones/053`),
    /// para que haya una decisión de `ore-iam` a la que atribuir cada
    /// publicación. Se devuelve una vez y no se guarda.
    pub(crate) fn token_de_artefactos(
        &self,
        p: &Peticion,
        sujeto: &ore_entrada::identidad::Identidad,
        b: &str,
        s: &str,
        r: &str,
    ) -> Respuesta {
        let registro = format!("{b}.{s}.{r}");
        let dueno = match self.registro_del_arbol(b, s, r) {
            Ok(Some((_, dueno))) => dueno,
            Ok(None) => {
                return Respuesta::error(
                    404,
                    format!("there is no artifact registry `{registro}` in this cell"),
                );
            }
            Err(r) => return r,
        };
        let ruta = format!("POST /artifact-registries/{b}/{s}/{r}/token");
        let mut decision = match self.exigir(sujeto, "artefacto:publicar", &ruta) {
            Ok(d) => d,
            Err(r) => return r,
        };
        let es_suyo = match self.dueno_de_quien_crea(sujeto) {
            Ok(yo) => dueno.as_deref() == Some(yo.as_str()),
            Err(_) => false,
        };
        if !es_suyo {
            // Publicar en el registro de otro: lo que decide es esto, y su
            // decisión es la que se cita.
            match self.exigir(sujeto, "artefacto:gestionar", &ruta) {
                Ok(d) => decision = d.or(decision),
                Err(r) => return r,
            }
        }
        let caduca = ahora() + VIDA_DE_PUBLICAR;
        let token = firmar(&Carga {
            persona: sujeto.persona.clone(),
            registro: registro.clone(),
            verbo: "publicar".into(),
            caduca,
            decision,
        });
        // La dirección como la ve quien pide (la consola o `curl`, por el túnel).
        let host = p
            .cabeceras
            .get("x-forwarded-host")
            .or(p.cabeceras.get("host"));
        let esquema = p
            .cabeceras
            .get("x-forwarded-proto")
            .map(String::as_str)
            .unwrap_or("http");
        let url = format!(
            "{esquema}://{}/artifacts/{b}/{s}/{r}/maven",
            host.map(String::as_str).unwrap_or("localhost")
        );
        let settings = format!(
            "<settings>\n  <servers>\n    <server>\n      <id>ore</id>\n      <username>publish</username>\n      <password>{token}</password>\n    </server>\n  </servers>\n</settings>\n"
        );
        let pom = format!(
            "<distributionManagement>\n  <repository>\n    <id>ore</id>\n    <url>{url}</url>\n  </repository>\n</distributionManagement>\n"
        );
        Respuesta {
            codigo: 201,
            cuerpo: Json::obj([
                ("registry", Json::s(&registro)),
                ("username", Json::s("publish")),
                ("token", Json::s(&token)),
                ("expires_in", Json::Int(VIDA_DE_PUBLICAR as i64)),
                ("url", Json::s(&url)),
                ("settings_xml", Json::s(settings)),
                ("pom_xml", Json::s(pom)),
                (
                    "commands",
                    Json::Arr(vec![
                        Json::s("mvn deploy"),
                        Json::s(format!(
                            "mvn deploy:deploy-file -Durl={url} -DrepositoryId=ore -Dfile=<file>.jar -DgroupId=<group> -DartifactId=<artifact> -Dversion=<version> -Dpackaging=jar"
                        )),
                    ]),
                ),
            ]),
        }
    }

    /// La huella de una publicación (AR1·3): Maven no trae el token de la
    /// persona, así que el evento cita la decisión con que se emitió el suyo.
    fn huella_de_publicar(&self, carga: &Carga, sobre: &str) {
        let Some(buzon) = self.buzon.as_ref() else {
            return;
        };
        let e = crate::acceso::evento(
            "artefacto:publicar",
            sobre,
            "hecho",
            carga.decision.clone(),
            None,
        );
        buzon.echar(None, e);
    }

    /// **Lo que una petición a `/artifacts/` contesta** (las que no suben).
    pub(crate) fn atender_artefacto(&self, p: &Peticion) -> Salida {
        let ruta = match ruta_de(&p.ruta) {
            Ok(r) => r,
            Err(m) => return error(400, m),
        };
        let cabeza = match p.metodo.as_str() {
            "GET" => false,
            "HEAD" => true,
            "PUT" => return error(411, "a Maven upload says its `content-length`"),
            otro => {
                return error(
                    405,
                    format!("`{otro}` is not something a Maven repository answers"),
                );
            }
        };
        let carga = match de_basic(p).map(|t| verificar(&t)) {
            None => {
                return pide_credencial(
                    "an ORE artifact token is needed (HTTP Basic, as the password)",
                );
            }
            Some(Err(m)) => return pide_credencial(m),
            Some(Ok(c)) => c,
        };
        let direccion = match medios() {
            Ok(d) => d,
            Err(s) => return s,
        };
        // El registro: el de la dirección, o el que reclamó la coordenada.
        let registro = match &ruta.registro {
            Some((b, s, r)) => format!("{b}.{s}.{r}"),
            None => match duena(&direccion, &ruta.coordenada()) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return error(
                        404,
                        format!("no registry of this cell has `{}`", ruta.coordenada()),
                    );
                }
                Err(s) => return s,
            },
        };
        if carga.registro != "*" && carga.registro != registro {
            return error(
                403,
                format!(
                    "this token is for `{}`, not for `{registro}`",
                    carga.registro
                ),
            );
        }
        let publicados = match listar(&direccion, &registro, &ruta.coordenada()) {
            Ok(l) => l,
            Err(s) => return s,
        };
        match &ruta.que {
            Que::Metadata { algoritmo } => {
                let mut versiones: Vec<String> =
                    publicados.iter().map(|f| f.version.clone()).collect();
                versiones.dedup();
                if versiones.is_empty() {
                    return error(
                        404,
                        format!("`{}` has no versions in `{registro}`", ruta.coordenada()),
                    );
                }
                let xml = metadata(&ruta.grupo, &ruta.artefacto, &versiones, &marca_de_maven());
                match algoritmo {
                    None => texto_plano(200, "application/xml", xml.into_bytes(), cabeza),
                    Some(a) => texto_plano(
                        200,
                        "text/plain",
                        suma_de(a, xml.as_bytes()).into_bytes(),
                        cabeza,
                    ),
                }
            }
            Que::Suma {
                version,
                nombre,
                algoritmo,
            } => {
                let Some(f) = publicados
                    .iter()
                    .find(|f| &f.version == version && &f.nombre == nombre)
                else {
                    return error(404, format!("`{nombre}` is not published in `{registro}`"));
                };
                let valor = match algoritmo.as_str() {
                    "sha1" => f.sha1.clone(),
                    "md5" => f.md5.clone(),
                    "sha256" => f.sha256.clone(),
                    // `sha512` no se calcula al publicar: no se sirve.
                    _ => return error(404, "only sha1, md5 and sha256 are kept"),
                };
                texto_plano(200, "text/plain", valor.into_bytes(), cabeza)
            }
            Que::Fichero { version, nombre } => {
                let Some(f) = publicados
                    .iter()
                    .find(|f| &f.version == version && &f.nombre == nombre)
                else {
                    return error(404, format!("`{nombre}` is not published in `{registro}`"));
                };
                if cabeza {
                    return Salida::Bytes(Bytes {
                        codigo: 200,
                        cabeceras: vec![("content-type".into(), "application/octet-stream".into())],
                        largo: Some(f.tamano),
                        lector: Box::new(std::io::empty()),
                        finales: None,
                    });
                }
                let (b, s, r) = partes_del_registro(&registro);
                let camino = format!(
                    "/artefactos/{b}/{s}/{r}/maven/{}/{version}/{nombre}",
                    ruta.coordenada()
                );
                match ore_entrada::http::pedir_bajando("GET", &direccion, &camino, &[], PLAZOS) {
                    Ok(mut bytes) if bytes.codigo == 200 => {
                        bytes.cabeceras.retain(|(k, _)| k == "content-type");
                        Salida::Bytes(bytes)
                    }
                    Ok(bytes) => error(bytes.codigo, "`ore-medios` did not serve it"),
                    Err(e) => error(503, format!("`ore-medios` is not responding: {e}")),
                }
            }
        }
    }

    /// **`PUT /artifacts/…`**, con el cuerpo en flujo.
    pub(crate) fn subir_artefacto(&self, p: &Peticion, subida: &mut Subida) -> Salida {
        let ruta = match ruta_de(&p.ruta) {
            Ok(r) => r,
            Err(m) => return error(400, m),
        };
        let Some((b, s, r)) = ruta.registro.clone() else {
            return error(
                405,
                "publish to the address of a registry: `/artifacts/<base>/<schema>/<registry>/maven/…`",
            );
        };
        let registro = format!("{b}.{s}.{r}");
        let carga = match de_basic(p).map(|t| verificar(&t)) {
            None => {
                return pide_credencial(
                    "an ORE artifact token is needed (HTTP Basic, as the password)",
                );
            }
            Some(Err(m)) => return pide_credencial(m),
            Some(Ok(c)) => c,
        };
        if carga.verbo != "publicar" || carga.registro != registro {
            return error(
                403,
                format!(
                    "this token does not publish to `{registro}` (it is for `{}`, to {})",
                    carga.registro, carga.verbo
                ),
            );
        }
        match self.formatos_del_registro(&b, &s, &r) {
            Ok(Some(f)) if f.iter().any(|x| x == "maven") => {}
            Ok(Some(_)) => {
                return error(
                    400,
                    format!("`{registro}` does not take `maven`: see its `formats`"),
                );
            }
            Ok(None) => {
                return error(
                    404,
                    format!("there is no artifact registry `{registro}` in this cell"),
                );
            }
            Err(s) => return s,
        }
        let direccion = match medios() {
            Ok(d) => d,
            Err(s) => return s,
        };
        match &ruta.que {
            // Se genera al leer (AR0 P1): lo que Maven sube se tira.
            Que::Metadata { .. } => error_201("the metadata is generated from what is published"),
            Que::Suma {
                version,
                nombre,
                algoritmo,
            } => {
                let mut dicho = String::new();
                if subida.take(512).read_to_string(&mut dicho).is_err() {
                    return error(400, "a checksum is a line of hex");
                }
                let dicho = dicho
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let publicados = match listar(&direccion, &registro, &ruta.coordenada()) {
                    Ok(l) => l,
                    Err(s) => return s,
                };
                let Some(f) = publicados
                    .iter()
                    .find(|f| &f.version == version && &f.nombre == nombre)
                else {
                    return error(
                        409,
                        format!("`{nombre}` is not published: send the file before its checksum"),
                    );
                };
                let calculado = match algoritmo.as_str() {
                    "sha1" => &f.sha1,
                    "md5" => &f.md5,
                    "sha256" => &f.sha256,
                    _ => return error_201("sha512 is not kept: accepted"),
                };
                if &dicho == calculado {
                    error_201("matches what was received")
                } else {
                    error(
                        400,
                        format!(
                            "the {algoritmo} of `{nombre}` is {calculado}, not {dicho}: the upload was altered"
                        ),
                    )
                }
            }
            Que::Fichero { version, nombre } => {
                // D-A10: una versión que cambia no es una versión.
                if version.to_ascii_uppercase().ends_with("-SNAPSHOT") {
                    return error(
                        400,
                        format!(
                            "`{version}` is a SNAPSHOT: a registry only keeps versions that never change"
                        ),
                    );
                }
                let camino = format!(
                    "/artefactos/{b}/{s}/{r}/maven/{}/{version}/{nombre}",
                    ruta.coordenada()
                );
                let quien = carga.persona.clone();
                let largo = subida.largo;
                match ore_entrada::http::pedir_subiendo(
                    "PUT",
                    &direccion,
                    &camino,
                    &[("x-ore-quien", quien.as_str())],
                    subida,
                    largo,
                    PLAZOS,
                ) {
                    Ok((codigo, cuerpo)) => {
                        if codigo == 201 {
                            self.huella_de_publicar(
                                &carga,
                                &format!(
                                    "artifacts/{registro}/{}/{version}/{nombre}",
                                    ruta.coordenada()
                                ),
                            );
                        }
                        Salida::Una(Respuesta {
                            codigo,
                            cuerpo: Json::Crudo(if cuerpo.trim().is_empty() {
                                "{}".into()
                            } else {
                                cuerpo
                            }),
                        })
                    }
                    Err(e) => error(503, format!("`ore-medios` did not take it: {e}")),
                }
            }
        }
    }
}

/// Lo que vive un token de publicar (D-A9).
pub const VIDA_DE_PUBLICAR: u64 = 3600;

/// Lo que vive el token de leer de un Job de la capa (AR1·4): su plazo (20 min)
/// más lo que pueda esperar en la cola. Se ve en el Job (`kubectl get job`):
/// por eso sólo lee, y caduca.
pub const VIDA_DE_LEER: u64 = 3600;

/// **El token con el que un Job de la capa lee los registros de la celda**
/// (AR1·4): de toda la celda (`*`, D-A12), sólo para leer, a nombre del Job.
pub fn token_de_leer(job: &str) -> String {
    firmar(&Carga {
        persona: format!("capa:{job}"),
        registro: "*".into(),
        verbo: "leer".into(),
        caduca: ahora() + VIDA_DE_LEER,
        decision: None,
    })
}

/// **Las librerías que un registro tiene de un formato** (AR1·3): lo que impide
/// quitarle el formato o retirarlo. Sin `ore-medios` en la celda no puede haber
/// ninguna; sin respuesta suya, no se sabe, y quien pregunta no sigue.
pub(crate) fn librerias_de(registro: &str, formato: &str) -> Result<Vec<String>, Respuesta> {
    let Some(direccion) = std::env::var(crate::medios::ENTORNO)
        .ok()
        .filter(|d| !d.is_empty())
    else {
        return Ok(Vec::new());
    };
    let cuerpo = Json::obj([
        ("registro", Json::s(registro)),
        ("formato", Json::s(formato)),
    ]);
    match ore_entrada::http::pedir_con(
        "POST",
        &direccion,
        "/artefactos/librerias",
        &[],
        Some(&cuerpo),
        PLAZOS,
    ) {
        Ok((200, t)) => Ok(ore_core::parse::parse(&t)
            .ok()
            .and_then(|n| {
                n.get("librerias").map(|(_, l)| {
                    l.items()
                        .iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
            })
            .unwrap_or_default()),
        Ok((c, t)) => Err(Respuesta::error(
            502,
            format!("`ore-medios` answered {c}: {t}"),
        )),
        Err(e) => Err(Respuesta::error(
            503,
            format!(
                "`ore-medios` is not responding, so it is not known whether the registry has libraries: {e}"
            ),
        )),
    }
}

fn error_201(nota: &str) -> Salida {
    Salida::Una(Respuesta {
        codigo: 201,
        cuerpo: Json::obj([("note", Json::s(nota))]),
    })
}

fn partes_del_registro(r: &str) -> (String, String, String) {
    let mut p = r.splitn(3, '.');
    (
        p.next().unwrap_or_default().to_string(),
        p.next().unwrap_or_default().to_string(),
        p.next().unwrap_or_default().to_string(),
    )
}

/// Qué registro de la celda reclamó una coordenada de Maven.
fn duena(direccion: &str, coordenada: &str) -> Result<Option<String>, Salida> {
    let cuerpo = Json::obj([
        ("formato", Json::s("maven")),
        ("libreria", Json::s(coordenada)),
    ]);
    match ore_entrada::http::pedir_con(
        "POST",
        direccion,
        "/artefactos/coordenada",
        &[],
        Some(&cuerpo),
        PLAZOS,
    ) {
        Ok((200, t)) => Ok(ore_core::parse::parse(&t)
            .ok()
            .and_then(|n| campo(&n, "registro"))),
        Ok((404, _)) => Ok(None),
        Ok((c, t)) => Err(error(502, format!("`ore-medios` answered {c}: {t}"))),
        Err(e) => Err(error(503, format!("`ore-medios` is not responding: {e}"))),
    }
}

/// Lo publicado de una coordenada en un registro.
fn listar(direccion: &str, registro: &str, coordenada: &str) -> Result<Vec<Publicado>, Salida> {
    let cuerpo = Json::obj([
        ("registro", Json::s(registro)),
        ("formato", Json::s("maven")),
        ("libreria", Json::s(coordenada)),
    ]);
    let t = match ore_entrada::http::pedir_con(
        "POST",
        direccion,
        "/artefactos/listar",
        &[],
        Some(&cuerpo),
        PLAZOS,
    ) {
        Ok((200, t)) => t,
        Ok((c, t)) => return Err(error(502, format!("`ore-medios` answered {c}: {t}"))),
        Err(e) => return Err(error(503, format!("`ore-medios` is not responding: {e}"))),
    };
    let n =
        ore_core::parse::parse(&t).map_err(|_| error(502, "`ore-medios` did not answer JSON"))?;
    let mut out = Vec::new();
    for v in n
        .get("versiones")
        .map(|(_, v)| v.items())
        .unwrap_or_default()
    {
        let version = campo(v, "version").unwrap_or_default();
        for f in v
            .get("ficheros")
            .map(|(_, f)| f.items())
            .unwrap_or_default()
        {
            out.push(Publicado {
                version: version.clone(),
                nombre: campo(f, "nombre").unwrap_or_default(),
                sha1: campo(f, "sha1").unwrap_or_default(),
                md5: campo(f, "md5").unwrap_or_default(),
                sha256: campo(f, "sha256").unwrap_or_default(),
                tamano: campo(f, "tamano").and_then(|t| t.parse().ok()).unwrap_or(0),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn una_ruta_de_maven_se_lee_por_partes() {
        let r = ruta_de("/artifacts/banco/riesgo/libs/maven/com/banco/motor-riesgo/1.0.0/motor-riesgo-1.0.0.jar").unwrap();
        assert_eq!(
            r.registro,
            Some(("banco".into(), "riesgo".into(), "libs".into()))
        );
        assert_eq!(r.coordenada(), "com.banco:motor-riesgo");
        assert_eq!(
            r.que,
            Que::Fichero {
                version: "1.0.0".into(),
                nombre: "motor-riesgo-1.0.0.jar".into()
            }
        );
        let r = ruta_de(
            "/artifacts/maven/com/banco/motor-riesgo/1.0.0/motor-riesgo-1.0.0-sources.jar.sha1",
        )
        .unwrap();
        assert_eq!(r.registro, None);
        assert!(matches!(r.que, Que::Suma { ref algoritmo, .. } if algoritmo == "sha1"));
        let r = ruta_de("/artifacts/maven/com/banco/motor-riesgo/maven-metadata.xml.md5").unwrap();
        assert_eq!(
            r.que,
            Que::Metadata {
                algoritmo: Some("md5".into())
            }
        );
        assert_eq!(r.coordenada(), "com.banco:motor-riesgo");
    }

    #[test]
    fn una_ruta_que_no_cuadra_no_entra() {
        for c in [
            "/artifacts/banco/riesgo/libs/maven/com/banco/x/1.0.0/otro-1.0.0.jar",
            "/artifacts/banco/riesgo/libs/maven/com/banco/x/1.0.0/x-1.0.0jar",
            "/artifacts/banco/riesgo/libs/maven/com/../x/1.0.0/x-1.0.0.jar",
            "/artifacts/Banco/riesgo/libs/maven/com/x/1.0.0/x-1.0.0.jar",
            "/artifacts/banco/riesgo/libs/pypi/x/1.0.0/x.whl",
            "/artifacts/maven/x/1.0.0",
        ] {
            assert!(ruta_de(c).is_err(), "{c}");
        }
    }

    #[test]
    fn un_token_firmado_se_verifica_y_uno_tocado_no() {
        let c = Carga {
            persona: "persona:ana".into(),
            registro: "banco.riesgo.libs".into(),
            verbo: "publicar".into(),
            caduca: ahora() + 60,
            decision: None,
        };
        let t = firmar(&c);
        assert_eq!(verificar(&t), Ok(c.clone()));
        // Con la decisión de `ore-iam`, viaja y vuelve.
        let con = Carga {
            decision: Some("dec-1".into()),
            ..c.clone()
        };
        assert_eq!(verificar(&firmar(&con)), Ok(con));
        let mut tocado = t.clone();
        tocado.pop();
        tocado.push(if t.ends_with('0') { '1' } else { '0' });
        assert!(verificar(&tocado).is_err());
        let viejo = firmar(&Carga {
            caduca: ahora() - 1,
            ..c
        });
        assert_eq!(
            verificar(&viejo),
            Err("the token expired: generate a new one")
        );
        assert!(verificar("ghp_otracosa").is_err());
    }

    #[test]
    fn basic_lleva_el_token_en_la_contrasena() {
        assert_eq!(
            de_base64("cHVibGljYXI6c2VjcmV0bw=="),
            Some(b"publicar:secreto".to_vec())
        );
        assert_eq!(de_base64("YQ"), Some(b"a".to_vec()));
    }

    #[test]
    fn las_versiones_se_ordenan_como_en_maven() {
        let mut v = vec!["1.10.0", "1.2.0", "1.0", "1.0.1", "1.0-rc1", "2.0.0"];
        v.sort_by(|a, b| orden_de_versiones(a, b));
        assert_eq!(v, ["1.0-rc1", "1.0", "1.0.1", "1.2.0", "1.10.0", "2.0.0"]);
        let xml = metadata(
            "com.banco",
            "x",
            &["1.2.0".into(), "1.10.0".into()],
            "20261025120000",
        );
        assert!(
            xml.contains("<latest>1.10.0</latest>") && xml.contains("<release>1.10.0</release>")
        );
    }
}
