//! **Un contenedor de Azure Blob Storage de un cliente, como origen** (ADR 0061
//! O3): la API REST de Blob —que vale también sobre una cuenta ADLS Gen2—, con
//! la identidad que el cliente autorizó (D-O3).
//!
//! ```text
//! az://<cuenta>/<contenedor>[/<prefijo>]?tenant=<tenant>&cliente=<id de la app>
//! ```
//!
//! La URL **no lleva secreto**: nombra la app de Entra del cliente, que tiene
//! una *federated identity credential* para la cuenta de Google de esta celda.
//! Quien lee pide su token de identidad de Google (`ore-gcp`, audiencia
//! `api://AzureADTokenExchange`) y Entra se lo cambia por uno de Storage: el
//! *client credentials* con aserción federada. La clave de la cuenta, una SAS
//! del cliente o el secreto de una app no se admiten.
//!
//! Cada blob se fija por su **`versionId`** si la cuenta versiona, y siempre,
//! además, por su **ETag** (`If-Match`): Azurite ignora un `versionid` que no
//! existe y da los bytes vigentes (medido en O3·0), y un servidor así no debe
//! poder dar otros bytes. Sin versionado —y en ADLS Gen2, que no lo tiene— la
//! versión es el ETag (`etag:…`, D-O1).
//!
//! ⭐ **El token es al portador**, y uno de Storage vale para cualquier cuenta
//! que el principal pueda leer: sólo se habla con `https://<cuenta>.blob.core.windows.net`.
//! Otro servidor (`?endpoint=`, el laboratorio con Azurite) sólo con
//! `ORE_AZURE_LABORATORIO=1`, y `ORE_AZURE_TOKEN` pone un token fijo.

mod origen;
pub mod sas;

use ore_objetos::Leido;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LOGIN: &str = "https://login.microsoftonline.com";
/// La audiencia del token de Google que Entra acepta en una credencial federada.
const AUDIENCIA: &str = "api://AzureADTokenExchange";
const ALCANCE: &str = "https://storage.azure.com/.default";
const AGENTE: &str = "ore-azure/0.1";
/// Cuánto se guarda un token de Entra (viven entre 60 y 90 minutos).
const VIDA_DEL_TOKEN: Duration = Duration::from_secs(45 * 60);
/// Lo que se pide de vida a una clave de delegación (el máximo es 7 días).
const VIDA_DE_LA_CLAVE: u64 = 6 * 86_400;

static PETICIONES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Cuántas peticiones a Azure y cuántos bytes de listados, para los avisos del
/// driver (los de los blobs van en flujo y no se cuentan).
pub fn contadores() -> (usize, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (PETICIONES.load(Relaxed), BYTES.load(Relaxed))
}

/// La coordenada de una fuente de Azure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fuente {
    pub cuenta: String,
    pub contenedor: String,
    /// Vacío o acabado en `/`.
    pub prefijo: String,
    /// El tenant de Entra del cliente (su id, o su dominio).
    pub tenant: String,
    /// El id de la app (o de la managed identity) que confía en la celda.
    pub cliente: String,
    /// Otro servidor, sólo en el laboratorio: `<endpoint>/<cuenta>/…` (Azurite).
    pub endpoint: Option<String>,
}

fn laboratorio() -> bool {
    std::env::var("ORE_AZURE_LABORATORIO").as_deref() == Ok("1")
}

/// `az://cuenta/contenedor/prefijo?tenant=…&cliente=…` → la coordenada.
pub fn leer(url: &str) -> Result<Fuente, String> {
    let resto = url.trim().strip_prefix("az://").ok_or(
        "la URL de Azure es `az://<cuenta>/<contenedor>[/<prefijo>]?tenant=<id>&cliente=<id de la app>`",
    )?;
    let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let mut partes = camino.splitn(3, '/');
    let cuenta = partes.next().unwrap_or("");
    let contenedor = partes.next().unwrap_or("");
    let prefijo = descodificar(partes.next().unwrap_or(""));
    if !(3..=24).contains(&cuenta.len())
        || !cuenta
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(format!(
            "`{cuenta}` no es el nombre de una cuenta de almacenamiento (3 a 24 minúsculas y cifras)"
        ));
    }
    if !(3..=63).contains(&contenedor.len())
        || !contenedor
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "`{contenedor}` no es el nombre de un contenedor (3 a 63 minúsculas, cifras y `-`)"
        ));
    }
    let (mut tenant, mut cliente, mut endpoint) = (None, None, None);
    let id = |v: &str| {
        !v.is_empty()
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))
    };
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        let v = descodificar(v);
        match k {
            "tenant" if id(&v) => tenant = Some(v),
            "cliente" if id(&v) => cliente = Some(v),
            "tenant" | "cliente" => return Err(format!("`{k}={v}` no es un id de Entra")),
            "endpoint" => {
                if !laboratorio() {
                    return Err(format!(
                        "`endpoint={v}`: sólo se habla con `https://{cuenta}.blob.core.windows.net` \
                         (el token de la celda no se le da a otro servidor)"
                    ));
                }
                endpoint = Some(v.trim_end_matches('/').to_string());
            }
            otro => {
                return Err(format!(
                    "`{otro}` no es un parámetro de una URL `az://` (sin claves ni SAS: D-O3)"
                ));
            }
        }
    }
    let (Some(tenant), Some(cliente)) = (tenant, cliente) else {
        return Err(
            "falta `tenant` o `cliente`: la URL nombra la app de Entra que confía en esta celda"
                .into(),
        );
    };
    let prefijo = prefijo.trim_start_matches('/');
    let prefijo = if prefijo.is_empty() || prefijo.ends_with('/') {
        prefijo.to_string()
    } else {
        format!("{prefijo}/")
    };
    Ok(Fuente {
        cuenta: cuenta.to_string(),
        contenedor: contenedor.to_string(),
        prefijo,
        tenant,
        cliente,
        endpoint,
    })
}

/// La URL de un prefijo, que se puede enseñar: una `az://` no lleva secreto.
pub fn publica(f: &Fuente, prefijo: &str) -> String {
    let mut u = format!(
        "az://{}/{}/{prefijo}?tenant={}&cliente={}",
        f.cuenta, f.contenedor, f.tenant, f.cliente
    );
    if let Some(e) = &f.endpoint {
        u.push_str(&format!("&endpoint={e}"));
    }
    u
}

fn descodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encoding; con `ruta`, la barra se queda (el nombre de un blob en su
/// URL).
pub(crate) fn codificar(s: &str, ruta: bool) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if ruta => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── XML ─────────────────────────────────────────────────────────────────────

/// El texto de la primera `<t>…</t>` (o vacío si es `<t />`), con las
/// entidades deshechas.
fn etiqueta(xml: &str, t: &str) -> Option<String> {
    let abre = format!("<{t}>");
    let cierra = format!("</{t}>");
    let i = xml.find(&abre)? + abre.len();
    let j = xml[i..].find(&cierra)? + i;
    Some(entidades(&xml[i..j]))
}

/// Los trozos `<t>…</t>`, en orden.
fn bloques<'a>(xml: &'a str, t: &str) -> Vec<&'a str> {
    let abre = format!("<{t}>");
    let cierra = format!("</{t}>");
    let mut out = Vec::new();
    let mut resto = xml;
    while let Some(i) = resto.find(&abre) {
        let dentro = &resto[i + abre.len()..];
        let Some(j) = dentro.find(&cierra) else { break };
        out.push(&dentro[..j]);
        resto = &dentro[j + cierra.len()..];
    }
    out
}

fn entidades(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#34;", "\"")
        .replace("&amp;", "&")
}

// ── El tiempo ───────────────────────────────────────────────────────────────

fn ahora_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// (año, mes, día) de unos días desde 1970 (Howard Hinnant, `civil_from_days`).
fn civil(dias: i64) -> (i64, u32, u32) {
    let z = dias + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DDTHH:MM:SSZ`, como lo piden la clave de delegación y la SAS.
pub fn iso(s: u64) -> String {
    let (y, m, d) = civil((s / 86_400) as i64);
    let r = s % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        r / 3600,
        r / 60 % 60,
        r % 60
    )
}

const MESES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `x-ms-date`: RFC 1123 (`Fri, 09 Oct 2026 17:20:52 GMT`).
fn rfc1123(s: u64) -> String {
    const DIAS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let dias = (s / 86_400) as i64;
    let (y, m, d) = civil(dias);
    let r = s % 86_400;
    format!(
        "{}, {d:02} {} {y:04} {:02}:{:02}:{:02} GMT",
        DIAS[(dias % 7) as usize],
        MESES[(m - 1) as usize],
        r / 3600,
        r / 60 % 60,
        r % 60
    )
}

/// El `Last-Modified` de Azure (RFC 1123) como lo dice el resto de orígenes:
/// `2026-10-09T17:20:52.000Z`. Si no se entiende, tal cual.
fn modificado(rfc: &str) -> String {
    let p: Vec<&str> = rfc.split_whitespace().collect();
    if let [_, d, mes, y, hms, _] = p.as_slice()
        && let Some(m) = MESES.iter().position(|x| x == mes)
    {
        return format!("{y}-{:02}-{d:0>2}T{hms}.000Z", m + 1);
    }
    rfc.to_string()
}

// ── Lo que Azure dice de un blob ────────────────────────────────────────────

/// **Lo que Azure dice de un blob** (o de una versión suya).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    pub nombre: String,
    pub tamano: u64,
    /// Entre comillas, como va en `If-Match`.
    pub etag: String,
    /// El `versionId`, si la cuenta versiona (y el blob tiene versión).
    pub version: Option<String>,
    /// Si es la versión vigente (sin versionado, siempre).
    pub actual: bool,
    /// Base64 del `Content-MD5`, si el blob lo tiene.
    pub md5: Option<String>,
    pub tipo: Option<String>,
    pub modificado: String,
}

/// Un ETag entre comillas: el listado lo da sin ellas, las cabeceras con.
fn comillas(e: &str) -> String {
    format!("\"{}\"", e.trim_matches('"'))
}

impl Meta {
    fn de_xml(b: &str) -> Meta {
        let p = etiqueta(b, "Properties").unwrap_or_default();
        let c = |k: &str| etiqueta(&p, k).filter(|v| !v.is_empty());
        let version = etiqueta(b, "VersionId").filter(|v| !v.is_empty());
        // Azure sólo dice `IsCurrentVersion` de la vigente: una versión sin
        // él es una de antes; un blob sin versión, el único que hay.
        let actual = match etiqueta(b, "IsCurrentVersion") {
            Some(v) => v == "true",
            None => version.is_none(),
        };
        Meta {
            nombre: etiqueta(b, "Name").unwrap_or_default(),
            tamano: c("Content-Length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            etag: c("Etag").map(|e| comillas(&e)).unwrap_or_default(),
            version,
            actual,
            md5: c("Content-MD5"),
            tipo: c("Content-Type"),
            modificado: c("Last-Modified")
                .map(|m| modificado(&m))
                .unwrap_or_default(),
        }
    }
}

/// **Un listado entero, página a página** (`marker`/`NextMarker`), hasta
/// `paginas`: `pagina(marker)` pide una y devuelve su XML. Fuera de la red
/// para probarlo. Devuelve los blobs y los prefijos (con `delimiter`).
pub fn paginar(
    mut pagina: impl FnMut(Option<&str>) -> Result<String, String>,
    paginas: usize,
) -> Result<(Vec<Meta>, Vec<String>), String> {
    let mut metas = Vec::new();
    let mut prefijos = Vec::new();
    let mut marca: Option<String> = None;
    for _ in 0..paginas {
        let xml = pagina(marca.as_deref())?;
        if !xml.contains("<EnumerationResults") {
            return Err(format!(
                "el listado de Azure no es un `EnumerationResults`: {}",
                xml.chars().take(120).collect::<String>()
            ));
        }
        metas.extend(bloques(&xml, "Blob").into_iter().map(Meta::de_xml));
        prefijos.extend(
            bloques(&xml, "BlobPrefix")
                .into_iter()
                .filter_map(|b| etiqueta(b, "Name")),
        );
        marca = etiqueta(&xml, "NextMarker").filter(|m| !m.is_empty());
        if marca.is_none() {
            break;
        }
    }
    Ok((metas, prefijos))
}

// ── El cliente ──────────────────────────────────────────────────────────────

/// Lo que Azure contestó cuando no fue un 2xx: el estado, su código
/// (`x-ms-error-code`) y el cuerpo (`0`: no contestó).
#[derive(Debug, Clone)]
pub struct Fallo {
    pub estado: u16,
    pub codigo: String,
    pub cuerpo: String,
}

impl Fallo {
    fn de(estado: u16, cuerpo: String) -> Fallo {
        Fallo {
            estado,
            codigo: String::new(),
            cuerpo,
        }
    }

    pub fn motivo(&self) -> String {
        if self.estado == 0 {
            return self.cuerpo.clone();
        }
        let codigo = if self.codigo.is_empty() {
            etiqueta(&self.cuerpo, "Code").unwrap_or_default()
        } else {
            self.codigo.clone()
        };
        let m = etiqueta(&self.cuerpo, "Message")
            .map(|m| m.lines().next().unwrap_or("").to_string())
            .unwrap_or_default();
        format!("{} {codigo} {m}", self.estado).trim().to_string()
    }
}

/// **Un contenedor de Azure, listo para leer**: su coordenada, el cliente HTTP
/// (uno para toda la vida), el token de Entra y la clave de delegación, cada
/// uno guardado mientras vale.
pub struct Azure {
    pub fuente: Fuente,
    agente: ureq::Agent,
    token: Mutex<Option<(String, Instant)>>,
    clave: Mutex<Option<(sas::Clave, u64)>>,
}

impl Azure {
    pub fn de(fuente: Fuente) -> Result<Azure, String> {
        Ok(Azure {
            fuente,
            agente: ore_gcp::cliente()?,
            token: Mutex::new(None),
            clave: Mutex::new(None),
        })
    }

    pub fn de_url(url: &str) -> Result<Azure, String> {
        Azure::de(leer(url)?)
    }

    /// **El token de Storage**: el de Google de la cuenta que corre, cambiado
    /// por Entra por uno de la app del cliente (o `ORE_AZURE_TOKEN`).
    pub fn token(&self) -> Result<String, String> {
        if let Ok(t) = std::env::var("ORE_AZURE_TOKEN")
            && !t.is_empty()
        {
            return Ok(t);
        }
        let mut v = self.token.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, caduca)) = v.as_ref()
            && Instant::now() < *caduca
        {
            return Ok(t.clone());
        }
        let f = &self.fuente;
        let id = ore_gcp::identidad(AUDIENCIA)?;
        let r = self
            .agente
            .post(&format!("{LOGIN}/{}/oauth2/v2.0/token", f.tenant))
            .set("user-agent", AGENTE)
            .send_form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &f.cliente),
                ("scope", ALCANCE),
                (
                    "client_assertion_type",
                    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                ),
                ("client_assertion", &id),
            ]);
        PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let texto = match r {
            Ok(x) => x
                .into_string()
                .map_err(|e| format!("el token de Entra no se pudo leer: {e}"))?,
            Err(ureq::Error::Status(c, x)) => {
                return Err(motivo_de_entra(f, c, &x.into_string().unwrap_or_default()));
            }
            Err(e) => return Err(format!("Entra no contesta: {e}")),
        };
        let n = ore_core::parse::parse(&texto)
            .map_err(|e| format!("el token de Entra no analiza: {e:?}"))?;
        let t = n
            .get("access_token")
            .and_then(|(_, v)| v.as_str())
            .ok_or("Entra no devolvió `access_token`")?
            .to_string();
        *v = Some((t.clone(), Instant::now() + VIDA_DEL_TOKEN));
        Ok(t)
    }

    fn base(&self) -> String {
        let f = &self.fuente;
        match &f.endpoint {
            Some(e) => format!("{e}/{}", f.cuenta),
            None => format!("https://{}.blob.core.windows.net", f.cuenta),
        }
    }

    fn url_del_blob(&self, clave: &str) -> String {
        format!(
            "{}/{}/{}",
            self.base(),
            self.fuente.contenedor,
            codificar(clave, true)
        )
    }

    fn pide(&self, metodo: &str, url: &str) -> Result<ureq::Request, Fallo> {
        let t = self.token().map_err(|e| Fallo::de(0, e))?;
        PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(self
            .agente
            .request(metodo, url)
            .set("user-agent", AGENTE)
            .set("x-ms-version", sas::VERSION)
            .set("x-ms-date", &rfc1123(ahora_s()))
            .set("authorization", &format!("Bearer {t}")))
    }

    fn llamar(r: ureq::Request, cuerpo: Option<&str>) -> Result<ureq::Response, Fallo> {
        let hecho = match cuerpo {
            Some(c) => r.send_string(c),
            None => r.call(),
        };
        match hecho {
            Ok(x) => Ok(x),
            Err(ureq::Error::Status(c, x)) => Err(Fallo {
                estado: c,
                codigo: x.header("x-ms-error-code").unwrap_or("").to_string(),
                cuerpo: x.into_string().unwrap_or_default(),
            }),
            Err(e) => Err(Fallo::de(0, format!("Azure no contesta: {e}"))),
        }
    }

    /// **El listado bajo un prefijo**: con `versiones`, cada versión
    /// (`include=versions`); con `delimitador`, un nivel y sus prefijos; con
    /// `max`, una página de hasta tantos.
    pub fn listar(
        &self,
        prefijo: &str,
        versiones: bool,
        delimitador: Option<&str>,
        max: Option<u32>,
    ) -> Result<(Vec<Meta>, Vec<String>), Fallo> {
        let mut base = format!(
            "{}/{}?restype=container&comp=list&prefix={}",
            self.base(),
            self.fuente.contenedor,
            codificar(prefijo, false)
        );
        if versiones {
            base.push_str("&include=versions");
        }
        if let Some(d) = delimitador {
            base.push_str(&format!("&delimiter={}", codificar(d, false)));
        }
        if let Some(m) = max {
            base.push_str(&format!("&maxresults={m}"));
        }
        let mut fallo = None;
        let r = paginar(
            |marca| {
                let url = match marca {
                    Some(m) => format!("{base}&marker={}", codificar(m, false)),
                    None => base.clone(),
                };
                let x = self
                    .pide("GET", &url)
                    .and_then(|r| Azure::llamar(r, None))
                    .map_err(|f| {
                        let m = f.motivo();
                        fallo = Some(f);
                        m
                    })?;
                let t = x
                    .into_string()
                    .map_err(|e| format!("el listado no se pudo leer: {e}"))?;
                BYTES.fetch_add(t.len(), std::sync::atomic::Ordering::Relaxed);
                Ok(t)
            },
            if max.is_some() { 1 } else { usize::MAX },
        );
        r.map_err(|m| fallo.unwrap_or(Fallo::de(0, m)))
    }

    /// Lo que Azure dice de un blob, o de una versión suya (`HEAD`).
    pub fn meta(&self, clave: &str, version: Option<&str>) -> Result<Meta, Fallo> {
        let mut url = self.url_del_blob(clave);
        if let Some(v) = version {
            url.push_str(&format!("?versionid={}", codificar(v, false)));
        }
        let x = Azure::llamar(self.pide("HEAD", &url)?, None)?;
        let h = |k: &str| x.header(k).map(String::from).filter(|v| !v.is_empty());
        Ok(Meta {
            nombre: clave.to_string(),
            tamano: h("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            etag: h("etag").map(|e| comillas(&e)).unwrap_or_default(),
            version: h("x-ms-version-id"),
            actual: h("x-ms-is-current-version").is_none_or(|v| v == "true"),
            md5: h("content-md5"),
            tipo: h("content-type"),
            modificado: h("last-modified")
                .map(|m| modificado(&m))
                .unwrap_or_default(),
        })
    }

    /// **Los bytes de un blob**: de su versión si se dice (`versionid`),
    /// condicionados a su ETag si se dice (`If-Match`: `412` si ya no es), y
    /// del rango si se pide; en flujo.
    pub fn bajar(
        &self,
        clave: &str,
        version: Option<&str>,
        etag: Option<&str>,
        rango: Option<&str>,
    ) -> Result<Leido, Fallo> {
        let mut url = self.url_del_blob(clave);
        if let Some(v) = version.filter(|v| !v.is_empty()) {
            url.push_str(&format!("?versionid={}", codificar(v, false)));
        }
        let mut r = self.pide("GET", &url)?;
        if let Some(e) = etag.filter(|e| !e.is_empty()) {
            r = r.set("if-match", &comillas(e));
        }
        if let Some(x) = rango {
            r = r.set("range", x);
        }
        let x = Azure::llamar(r, None)?;
        let cabeceras = x
            .headers_names()
            .into_iter()
            .filter_map(|n| {
                x.header(&n)
                    .map(|v| (n.to_ascii_lowercase(), v.to_string()))
            })
            .collect();
        Ok(Leido {
            estado: x.status(),
            cabeceras,
            lector: Box::new(x.into_reader()),
        })
    }

    /// **La clave de delegación** (`userdelegationkey`), guardada mientras le
    /// quede vida para una URL de `segundos`: pedirla es de la cuenta, y la
    /// identidad necesita `generateUserDelegationKey` sobre ella (D-O3:
    /// `Storage Blob Delegator`).
    pub fn clave_de_delegacion(&self, segundos: u64) -> Result<sas::Clave, Fallo> {
        let ahora = ahora_s();
        let mut g = self.clave.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, fin)) = g.as_ref()
            && *fin > ahora + segundos + 300
        {
            return Ok(k.clone());
        }
        let cuerpo = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><KeyInfo><Start>{}</Start><Expiry>{}</Expiry></KeyInfo>",
            iso(ahora - 300),
            iso(ahora + VIDA_DE_LA_CLAVE)
        );
        let url = format!("{}/?restype=service&comp=userdelegationkey", self.base());
        let r = self
            .pide("POST", &url)?
            .set("content-type", "application/xml");
        let xml = Azure::llamar(r, Some(&cuerpo))?
            .into_string()
            .map_err(|e| Fallo::de(0, format!("la clave de delegación no se pudo leer: {e}")))?;
        let c = |k: &str| etiqueta(&xml, k).unwrap_or_default();
        let k = sas::Clave {
            valor: ore_gcp::de_base64(&c("Value")).map_err(|e| Fallo::de(0, e))?,
            oid: c("SignedOid"),
            tid: c("SignedTid"),
            inicio: c("SignedStart"),
            fin: c("SignedExpiry"),
            servicio: c("SignedService"),
            version: c("SignedVersion"),
        };
        *g = Some((k.clone(), ahora + VIDA_DE_LA_CLAVE));
        Ok(k)
    }

    /// **Una URL de un blob** —de su versión, si se dice—, firmada con la
    /// clave de delegación: viva `segundos` (no más que la clave), con el tipo
    /// y la disposición dentro.
    pub fn firmar(
        &self,
        clave: &str,
        version: Option<&str>,
        tipo: &str,
        disposicion: &str,
        segundos: u64,
    ) -> Result<String, String> {
        let segundos = segundos.clamp(1, VIDA_DE_LA_CLAVE - 600);
        let k = self.clave_de_delegacion(segundos).map_err(|f| match f.estado {
            403 => format!(
                "no se pudo pedir la clave de delegación ({}): la app del cliente necesita \
                 `Storage Blob Delegator` (o el lector) sobre la cuenta `{}`, no sólo sobre el contenedor",
                f.motivo(),
                self.fuente.cuenta
            ),
            _ => format!("no se pudo pedir la clave de delegación: {}", f.motivo()),
        })?;
        let ahora = ahora_s();
        let (inicio, fin) = (iso(ahora - 300), iso(ahora + segundos));
        let p = sas::Pedida {
            cuenta: &self.fuente.cuenta,
            contenedor: &self.fuente.contenedor,
            blob: clave,
            version: version.filter(|v| !v.is_empty()),
            inicio: &inicio,
            fin: &fin,
            tipo,
            disposicion,
        };
        Ok(format!(
            "{}?{}",
            self.url_del_blob(clave),
            sas::consulta(&p, &k)
        ))
    }
}

/// Lo que Entra contesta cuando no canjea, dicho para quien da de alta: el
/// error de la federación (`AADSTS…`) en palabras de qué falta.
fn motivo_de_entra(f: &Fuente, estado: u16, cuerpo: &str) -> String {
    let n = ore_core::parse::parse(cuerpo).ok();
    let d = n
        .as_ref()
        .and_then(|n| n.get("error_description"))
        .and_then(|(_, v)| v.as_str().map(String::from))
        .unwrap_or_else(|| cuerpo.chars().take(200).collect());
    let d = d.lines().next().unwrap_or("").to_string();
    let que = if d.contains("AADSTS70021") {
        format!(
            "la app `{}` no tiene una credencial federada para la cuenta de Google de esta celda \
             (issuer `https://accounts.google.com`, subject = su ID único, audiencia \
             `{AUDIENCIA}`), o aún se está propagando: espera unos minutos",
            f.cliente
        )
    } else if d.contains("AADSTS700016") {
        format!("no hay una app `{}` en el tenant `{}`", f.cliente, f.tenant)
    } else if d.contains("AADSTS90002") || d.contains("AADSTS900023") {
        format!("no hay un tenant `{}` en Entra", f.tenant)
    } else {
        "Entra no canjea el token de la celda".to_string()
    };
    format!("{que} ({estado}: {d})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_url_dice_cuenta_contenedor_prefijo_y_la_app() {
        let f = leer("az://micuenta/cubo/Nueva%20carpeta/docs?tenant=ab1f708d-50f6-404c-a006-d71b2ac7a606&cliente=11111111-2222-3333-4444-555555555555").unwrap();
        assert_eq!(
            (f.cuenta.as_str(), f.contenedor.as_str(), f.prefijo.as_str()),
            ("micuenta", "cubo", "Nueva carpeta/docs/")
        );
        assert_eq!(f.endpoint, None);
        assert_eq!(
            publica(&f, "docs/"),
            "az://micuenta/cubo/docs/?tenant=ab1f708d-50f6-404c-a006-d71b2ac7a606&cliente=11111111-2222-3333-4444-555555555555"
        );
        for mala in [
            "s3://c/",
            "az://Mayus/cubo?tenant=t&cliente=c",
            "az://cuenta/c?tenant=t&cliente=c",
            "az://cuenta/cubo",
            "az://cuenta/cubo?tenant=t",
            "az://cuenta/cubo?tenant=t&cliente=c&sig=secreto",
            "az://cuenta/cubo?tenant=t&cliente=c&account_key=x",
            "az://cuenta/cubo?tenant=t%2Fx&cliente=c",
        ] {
            assert!(leer(mala).is_err(), "{mala}");
        }
    }

    /// El token es al portador: fuera del laboratorio, ningún `endpoint`.
    #[test]
    fn el_token_no_se_le_da_a_otro_servidor() {
        if laboratorio() {
            return;
        }
        let e = leer("az://cuenta/cubo?tenant=t&cliente=c&endpoint=https://evil.io").unwrap_err();
        assert!(e.contains("no se le da a otro servidor"), "{e}");
    }

    /// El listado: los blobs con sus versiones (o sin ellas), los prefijos, y
    /// las páginas por `NextMarker`. Azurite pagina (O3·0), pero esto no
    /// necesita red.
    #[test]
    fn el_listado_sigue_las_paginas_y_lee_las_versiones() {
        let paginas = [
            r#"<?xml version="1.0"?><EnumerationResults><Blobs><Blob><Name>docs/a&amp;b.pdf</Name><VersionId>2026-10-09T10:00:00.0000000Z</VersionId><IsCurrentVersion>true</IsCurrentVersion><Properties><Last-Modified>Thu, 09 Oct 2026 10:00:00 GMT</Last-Modified><Etag>0x8DE1</Etag><Content-Length>21</Content-Length><Content-Type>application/pdf</Content-Type><Content-MD5>CInfjbZ21DfOIWDhgYr6dw==</Content-MD5></Properties></Blob><BlobPrefix><Name>docs/sub/</Name></BlobPrefix></Blobs><NextMarker>m1</NextMarker></EnumerationResults>"#,
            r#"<?xml version="1.0"?><EnumerationResults><Blobs><Blob><Name>docs/a&amp;b.pdf</Name><VersionId>2026-10-08T10:00:00.0000000Z</VersionId><Properties><Etag>"0x8DE0"</Etag><Content-Length>20</Content-Length><Content-MD5 /></Properties></Blob><Blob><Name>docs/c.pdf</Name><Properties><Etag>0x1</Etag><Content-Length>3</Content-Length></Properties></Blob></Blobs><NextMarker /></EnumerationResults>"#,
        ];
        let mut pedidas = Vec::new();
        let (metas, prefijos) = paginar(
            |m| {
                pedidas.push(m.map(String::from));
                Ok(paginas[pedidas.len() - 1].to_string())
            },
            usize::MAX,
        )
        .unwrap();
        assert_eq!(pedidas, vec![None, Some("m1".into())]);
        assert_eq!(prefijos, vec!["docs/sub/".to_string()]);
        assert_eq!(metas.len(), 3);
        let a = &metas[0];
        assert_eq!(
            (a.nombre.as_str(), a.etag.as_str(), a.tamano, a.actual),
            ("docs/a&b.pdf", "\"0x8DE1\"", 21, true)
        );
        assert_eq!(a.md5.as_deref(), Some("CInfjbZ21DfOIWDhgYr6dw=="));
        assert_eq!(a.modificado, "2026-10-09T10:00:00.000Z");
        // la de antes: sin IsCurrentVersion dicho, pero con versión → no vigente
        assert_eq!(
            (metas[1].etag.as_str(), metas[1].actual),
            ("\"0x8DE0\"", false)
        );
        assert_eq!(metas[1].md5, None);
        // sin versionado: sin versión, vigente
        assert_eq!((metas[2].version.clone(), metas[2].actual), (None, true));
        // una página sola, aunque diga que hay más
        let (m, _) = paginar(|_| Ok(paginas[0].to_string()), 1).unwrap();
        assert_eq!(m.len(), 1);
        assert!(paginar(|_| Ok("<Error/>".into()), 1).is_err());
    }

    #[test]
    fn las_fechas_son_las_de_azure() {
        // 2026-10-09T17:20:52Z
        let s = 1_791_566_452;
        assert_eq!(iso(s), "2026-10-09T17:20:52Z");
        assert_eq!(rfc1123(s), "Fri, 09 Oct 2026 17:20:52 GMT");
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            modificado("Fri, 09 Oct 2026 17:20:52 GMT"),
            "2026-10-09T17:20:52.000Z"
        );
        assert_eq!(modificado("raro"), "raro");
    }

    #[test]
    fn entra_se_explica() {
        let f = leer("az://cuenta/cubo?tenant=t&cliente=app").unwrap();
        let m = motivo_de_entra(
            &f,
            400,
            r#"{"error":"invalid_request","error_description":"AADSTS70021: No matching federated identity record found for presented assertion.\r\nTrace ID: x"}"#,
        );
        assert!(
            m.contains("credencial federada") && m.contains("espera unos minutos"),
            "{m}"
        );
        assert!(!m.contains("Trace ID"), "{m}");
    }
}
