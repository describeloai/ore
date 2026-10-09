//! **Una biblioteca de SharePoint (o un OneDrive) de un cliente, como origen**
//! (ADR 0061 O5), por Microsoft Graph, con la identidad que el cliente
//! autorizó (D-O5).
//!
//! ```text
//! sharepoint://<host>/[sites|teams|personal/<nombre>/]<biblioteca>[/<prefijo>]?tenant=<id>&cliente=<id de la app>
//! ```
//!
//! La URL **no lleva secreto**: nombra la app de Entra del cliente, que confía
//! en la cuenta de Google de esta celda (`ore-entra`, el canje de Azure con el
//! ámbito de Graph) y a la que un administrador concedió `read` sobre el sitio
//! (`Sites.Selected`). Un sitio sin esa concesión contesta `403 accessDenied`.
//!
//! Cada fichero se fija por **su versión** (`<id del item>@<versión>`): una
//! vieja se baja por `/versions/{id}/content`; la actual no se puede pedir por
//! id y se baja por `/content` **vigilada**: su `cTag` (que sólo cambia con el
//! contenido) se mira al abrir y al terminar, y una lectura entera se coteja con
//! su `quickXorHash`. La `302` de Graph lleva a una URL de descarga al portador:
//! no se sigue a ciegas —se exige `https://*.sharepoint.com`— y se pide **sin**
//! el token.
//!
//! ⭐ El token de Graph sólo va a `https://graph.microsoft.com`: otro servidor
//! (`?endpoint=`, el Graph de mentira) sólo con `ORE_SHAREPOINT_LABORATORIO=1`,
//! y `ORE_GRAPH_TOKEN` pone un token fijo.

mod origen;

pub use origen::Vigilado;

use ore_core::parse::Node;
use ore_objetos::Leido;
use std::io::Read;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const GRAPH: &str = "https://graph.microsoft.com";
const ALCANCE: &str = "https://graph.microsoft.com/.default";
/// Como Microsoft pide que se identifique el tráfico de un ISV (su guía del
/// ritmo de SharePoint).
const AGENTE: &str = concat!("ISV|ORE|ore-graph/", env!("CARGO_PKG_VERSION"));
/// Cuántas veces se espera un `429`/`503` antes de rendirse.
const REINTENTOS: usize = 5;
/// Lo más que se espera por un `Retry-After`: más, y el error lo dice.
const ESPERA_MAXIMA: u64 = 120;
/// Una página de `children` (lo más que Graph da por defecto).
const PAGINA: usize = 200;

static PETICIONES: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static ESPERAS: AtomicUsize = AtomicUsize::new(0);

/// Cuántas peticiones a Graph, cuántos bytes de listados y cuántas veces se
/// esperó por el ritmo (`429`/`503`), para los avisos del driver.
pub fn contadores() -> (usize, usize, usize) {
    (
        PETICIONES.load(Relaxed),
        BYTES.load(Relaxed),
        ESPERAS.load(Relaxed),
    )
}

/// La coordenada de una fuente de SharePoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fuente {
    /// `contoso.sharepoint.com` (o `contoso-my.sharepoint.com`, un OneDrive).
    pub host: String,
    /// `sites/Finanzas`, `teams/X`, `personal/ana_contoso_com`, o vacío (el
    /// sitio raíz).
    pub sitio: String,
    /// La biblioteca, por su nombre visible (`Documentos`).
    pub biblioteca: String,
    /// Vacío o acabado en `/`.
    pub prefijo: String,
    pub tenant: String,
    pub cliente: String,
    /// Otro Graph, sólo en el laboratorio.
    pub endpoint: Option<String>,
}

fn laboratorio() -> bool {
    std::env::var("ORE_SHAREPOINT_LABORATORIO").as_deref() == Ok("1")
}

const FORMA: &str = "la URL de SharePoint es `sharepoint://<host>/[sites/<sitio>/]<biblioteca>[/<prefijo>]?tenant=<id>&cliente=<id de la app>`";

/// `sharepoint://host/sites/x/biblioteca/prefijo?tenant=…&cliente=…` → la
/// coordenada.
pub fn leer(url: &str) -> Result<Fuente, String> {
    let resto = url.trim().strip_prefix("sharepoint://").ok_or(FORMA)?;
    let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let mut partes = camino.split('/');
    let host = partes.next().unwrap_or("").to_ascii_lowercase();
    let segs: Vec<String> = partes.map(descodificar).collect();
    let (sitio, resto) = match segs.first().map(String::as_str) {
        Some("sites" | "teams" | "personal") => {
            let nombre = segs
                .get(1)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("falta el nombre del sitio tras `{}/`: {FORMA}", segs[0]))?;
            (format!("{}/{nombre}", segs[0]), &segs[2..])
        }
        _ => (String::new(), &segs[..]),
    };
    let biblioteca = resto
        .first()
        .filter(|b| !b.is_empty())
        .ok_or_else(|| format!("falta la biblioteca (`Documentos`, `Shared Documents`…): {FORMA}"))?
        .clone();
    let prefijo = resto[1..].join("/");
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:".contains(&b))
    {
        return Err(format!("`{host}` no es un host: {FORMA}"));
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
                        "`endpoint={v}`: sólo se habla con `{GRAPH}` (el token de la celda no se le \
                         da a otro servidor)"
                    ));
                }
                endpoint = Some(v.trim_end_matches('/').to_string());
            }
            otro => {
                return Err(format!(
                    "`{otro}` no es un parámetro de una URL `sharepoint://` (sin secretos: D-O5)"
                ));
            }
        }
    }
    if endpoint.is_none() && !host.ends_with(".sharepoint.com") {
        return Err(format!(
            "`{host}` no es un host de SharePoint Online (`<tenant>.sharepoint.com`, o \
             `<tenant>-my.sharepoint.com` para un OneDrive)"
        ));
    }
    let (Some(tenant), Some(cliente)) = (tenant, cliente) else {
        return Err(
            "falta `tenant` o `cliente`: la URL nombra la app de Entra que confía en esta celda"
                .into(),
        );
    };
    let prefijo = if prefijo.is_empty() || prefijo.ends_with('/') {
        prefijo
    } else {
        format!("{prefijo}/")
    };
    Ok(Fuente {
        host,
        sitio,
        biblioteca,
        prefijo,
        tenant,
        cliente,
        endpoint,
    })
}

/// La URL de un prefijo, que se puede enseñar: una `sharepoint://` no lleva
/// secreto.
pub fn publica(f: &Fuente, prefijo: &str) -> String {
    let sitio = if f.sitio.is_empty() {
        String::new()
    } else {
        format!("{}/", codificar(&f.sitio, true))
    };
    let mut u = format!(
        "sharepoint://{}/{sitio}{}/{}?tenant={}&cliente={}",
        f.host,
        codificar(&f.biblioteca, false),
        codificar(prefijo, true),
        f.tenant,
        f.cliente
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

/// Percent-encoding; con `ruta`, la barra se queda.
fn codificar(s: &str, ruta: bool) -> String {
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

// ── Lo que Graph dice ───────────────────────────────────────────────────────

/// Qué es un elemento de una biblioteca: sólo un `Fichero` es un objeto del
/// origen; un cuaderno de OneNote (`package`) y un acceso directo a otro drive
/// (`remoteItem`) se listan como tales y se saltan (D-O5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clase {
    #[default]
    Fichero,
    Carpeta,
    Cuaderno,
    Acceso,
}

/// Un `driveItem`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    /// Desde la raíz de la biblioteca: `docs/a.pdf`.
    pub ruta: String,
    pub clase: Clase,
    pub tamano: u64,
    /// Cambia con el contenido y con los metadatos.
    pub etag: String,
    /// Cambia sólo con el contenido: lo que fija la versión actual.
    pub ctag: String,
    pub modificado: String,
    /// `file.hashes.quickXorHash` (base64), si lo trae.
    pub quickxor: Option<String>,
}

fn texto(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
}

impl Item {
    /// De su JSON; `carpeta`, la ruta de su carpeta (vacía o acabada en `/`).
    fn de(n: &Node, carpeta: &str) -> Item {
        let nombre = texto(n, "name").unwrap_or_default();
        let clase = if n.get("folder").is_some() || n.get("root").is_some() {
            Clase::Carpeta
        } else if n.get("package").is_some() {
            Clase::Cuaderno
        } else if n.get("remoteItem").is_some() {
            Clase::Acceso
        } else {
            Clase::Fichero
        };
        Item {
            id: texto(n, "id").unwrap_or_default(),
            ruta: format!("{carpeta}{nombre}"),
            clase,
            tamano: texto(n, "size").and_then(|s| s.parse().ok()).unwrap_or(0),
            etag: texto(n, "eTag").unwrap_or_default(),
            ctag: texto(n, "cTag").unwrap_or_default(),
            modificado: texto(n, "lastModifiedDateTime").unwrap_or_default(),
            quickxor: n
                .get("file")
                .and_then(|(_, f)| f.get("hashes"))
                .and_then(|(_, h)| texto(h, "quickXorHash"))
                .filter(|q| !q.is_empty()),
        }
    }

    /// Su huella como la escribe la colección.
    pub fn huella(&self) -> Option<String> {
        self.quickxor.as_ref().map(|q| format!("quickxor:{q}"))
    }
}

/// Una versión de un fichero (`driveItemVersion`): `"3.0"`, su tamaño y su
/// fecha. Graph no da su huella.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub id: String,
    pub tamano: u64,
    pub modificado: String,
}

/// Lo que Graph contestó cuando no fue lo esperado: el estado, el `code` de su
/// error (contra el que se programa, nunca contra el mensaje) y el cuerpo
/// (`0`: no contestó, o lo dijo ORE).
#[derive(Debug, Clone)]
pub struct Fallo {
    pub estado: u16,
    pub codigo: String,
    pub cuerpo: String,
}

impl Fallo {
    fn de(estado: u16, cuerpo: String) -> Fallo {
        let codigo = ore_core::parse::parse(&cuerpo)
            .ok()
            .and_then(|n| n.get("error").and_then(|(_, e)| texto(e, "code")))
            .unwrap_or_default();
        Fallo {
            estado,
            codigo,
            cuerpo,
        }
    }

    fn ore(m: impl Into<String>) -> Fallo {
        Fallo {
            estado: 0,
            codigo: String::new(),
            cuerpo: m.into(),
        }
    }

    pub fn motivo(&self) -> String {
        if self.estado == 0 {
            return self.cuerpo.clone();
        }
        let m = ore_core::parse::parse(&self.cuerpo)
            .ok()
            .and_then(|n| n.get("error").and_then(|(_, e)| texto(e, "message")))
            .unwrap_or_else(|| self.cuerpo.trim().chars().take(200).collect());
        let m = m.lines().next().unwrap_or("").to_string();
        format!("{} {} {m}", self.estado, self.codigo)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Lo que no está: un `404` (`itemNotFound`), o la biblioteca que ORE no
    /// encontró en el sitio.
    pub fn no_esta(&self) -> bool {
        self.estado == 404
    }
}

/// **Una URL de descarga a la que se puede ir sin el token**: la de Graph
/// apunta a `https://<tenant>.sharepoint.com/…` (o a un `-my`); otra cosa —otro
/// host, `http`, un usuario en la URL— no se pide. En el laboratorio, cualquiera.
fn descarga_valida(url: &str, laboratorio: bool) -> bool {
    if laboratorio {
        return url.starts_with("http://") || url.starts_with("https://");
    }
    let Some(resto) = url.strip_prefix("https://") else {
        return false;
    };
    let autoridad = resto.split(['/', '?', '#']).next().unwrap_or("");
    if autoridad.contains('@') {
        return false;
    }
    let host = autoridad
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host.ends_with(".sharepoint.com") && host.len() > ".sharepoint.com".len()
}

/// **Un listado entero, página a página** (`@odata.nextLink`): `pagina(url)`
/// pide una y devuelve su JSON. La siguiente página sólo se pide si sigue en
/// `base` (el token no va a otro sitio). Fuera de la red, para probarlo.
pub fn paginar(
    primera: String,
    base: &str,
    mut pagina: impl FnMut(&str) -> Result<String, Fallo>,
) -> Result<Vec<Node>, Fallo> {
    let mut todo = Vec::new();
    let mut url = Some(primera);
    while let Some(u) = url.take() {
        if !u.starts_with(&format!("{base}/")) {
            return Err(Fallo::ore(format!(
                "Graph mandó la página siguiente a `{}`, fuera de `{base}`: no se sigue",
                u.split('?').next().unwrap_or("")
            )));
        }
        let t = pagina(&u)?;
        let n = ore_core::parse::parse(&t)
            .map_err(|e| Fallo::ore(format!("un listado de Graph no analiza: {e:?}")))?;
        if let Some((_, v)) = n.get("value") {
            todo.extend(v.items().iter().cloned());
        }
        url = texto(&n, "@odata.nextLink").filter(|s| !s.is_empty());
    }
    Ok(todo)
}

// ── El cliente ──────────────────────────────────────────────────────────────

/// Lo de dentro de un [`Graph`]: se comparte (`Arc`) para que una lectura
/// vigilada pueda volver a mirar el `cTag` al terminar.
pub struct Dentro {
    pub fuente: Fuente,
    agente: ureq::Agent,
    entra: ore_entra::Entra,
    /// El id del sitio y el de la biblioteca, cuando se saben.
    ids: Mutex<Option<(String, String)>>,
}

/// **Una biblioteca de SharePoint, lista para leer**: su coordenada, el
/// cliente HTTP (que no sigue redirecciones), el token de Entra y los ids del
/// sitio y la biblioteca, guardados.
#[derive(Clone)]
pub struct Graph(Arc<Dentro>);

impl Deref for Graph {
    type Target = Dentro;
    fn deref(&self) -> &Dentro {
        &self.0
    }
}

impl Graph {
    pub fn de(fuente: Fuente) -> Result<Graph, String> {
        let entra = ore_entra::Entra::nueva(
            &fuente.tenant,
            &fuente.cliente,
            ALCANCE,
            "ORE_GRAPH_TOKEN",
            AGENTE,
        );
        Ok(Graph(Arc::new(Dentro {
            fuente,
            agente: ore_gcp::cliente_sin_saltos()?,
            entra,
            ids: Mutex::new(None),
        })))
    }

    pub fn de_url(url: &str) -> Result<Graph, String> {
        Graph::de(leer(url)?)
    }

    /// **El token de Graph**: el de Google de la cuenta que corre, cambiado
    /// por Entra por uno de la app del cliente (o `ORE_GRAPH_TOKEN`).
    pub fn token(&self) -> Result<String, String> {
        self.entra.token(&self.agente)
    }

    fn base(&self) -> String {
        format!("{}/v1.0", self.fuente.endpoint.as_deref().unwrap_or(GRAPH))
    }

    /// **Un GET a Graph**, con el token, respetando su ritmo: a un `429` o un
    /// `503` se espera su `Retry-After` (hasta [`REINTENTOS`] veces y
    /// [`ESPERA_MAXIMA`] segundos cada una). `cabeceras`, las de más. Una
    /// `302` vuelve como respuesta: quien la pidió decide.
    fn pedir(&self, url: &str, cabeceras: &[(&str, &str)]) -> Result<ureq::Response, Fallo> {
        if !url.starts_with(&self.base()) {
            return Err(Fallo::ore(format!(
                "`{url}` no es de Graph: el token no se le da"
            )));
        }
        let t = self.token().map_err(Fallo::ore)?;
        let mut intento = 0;
        loop {
            PETICIONES.fetch_add(1, Relaxed);
            let mut r = self
                .agente
                .get(url)
                .set("user-agent", AGENTE)
                .set("authorization", &format!("Bearer {t}"));
            for (k, v) in cabeceras {
                r = r.set(k, v);
            }
            match r.call() {
                Ok(x) => return Ok(x),
                Err(ureq::Error::Status(c @ (429 | 503), x)) if intento < REINTENTOS => {
                    let espera = x
                        .header("retry-after")
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .unwrap_or(5);
                    if espera > ESPERA_MAXIMA {
                        return Err(Fallo::ore(format!(
                            "Graph pide esperar {espera} s ({c}): más de lo que espera un driver; \
                             el tenant está al límite de su ritmo"
                        )));
                    }
                    ESPERAS.fetch_add(1, Relaxed);
                    intento += 1;
                    std::thread::sleep(Duration::from_secs(espera));
                }
                Err(ureq::Error::Status(c, x)) => {
                    return Err(Fallo::de(c, x.into_string().unwrap_or_default()));
                }
                Err(e) => return Err(Fallo::ore(format!("Graph no contesta: {e}"))),
            }
        }
    }

    fn json(&self, url: &str) -> Result<Node, Fallo> {
        let t = self
            .pedir(url, &[])?
            .into_string()
            .map_err(|e| Fallo::ore(format!("Graph no se pudo leer: {e}")))?;
        BYTES.fetch_add(t.len(), Relaxed);
        ore_core::parse::parse(&t).map_err(|e| Fallo::ore(format!("Graph no analiza: {e:?}")))
    }

    fn paginado(&self, url: String) -> Result<Vec<Node>, Fallo> {
        let base = self.base();
        paginar(url, &base, |u| {
            let t = self
                .pedir(u, &[])?
                .into_string()
                .map_err(|e| Fallo::ore(format!("Graph no se pudo leer: {e}")))?;
            BYTES.fetch_add(t.len(), Relaxed);
            Ok(t)
        })
    }

    /// **El id del sitio** (por su ruta) y sus bibliotecas, por nombre.
    pub fn sitio(&self) -> Result<(String, Vec<(String, String)>), Fallo> {
        let f = &self.fuente;
        let url = if f.sitio.is_empty() {
            format!("{}/sites/{}", self.base(), f.host)
        } else {
            format!(
                "{}/sites/{}:/{}",
                self.base(),
                f.host,
                codificar(&f.sitio, true)
            )
        };
        let s = self.json(&url)?;
        let id = texto(&s, "id").ok_or_else(|| Fallo::ore("el sitio no trae `id`"))?;
        let drives = self
            .paginado(format!("{}/sites/{id}/drives", self.base()))?
            .iter()
            .filter_map(|d| Some((texto(d, "name")?, texto(d, "id")?)))
            .collect();
        Ok((id, drives))
    }

    /// El id de la biblioteca de la fuente (guardado).
    pub fn biblioteca(&self) -> Result<String, Fallo> {
        if let Some((_, d)) = self.ids.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(d.clone());
        }
        let (sitio, drives) = self.sitio()?;
        let b = &self.fuente.biblioteca;
        let d = drives
            .iter()
            .find(|(n, _)| n == b)
            .or_else(|| drives.iter().find(|(n, _)| n.eq_ignore_ascii_case(b)))
            .map(|(_, id)| id.clone())
            .ok_or_else(|| Fallo {
                estado: 404,
                codigo: "bibliotecaNoEsta".into(),
                cuerpo: format!(
                    "no hay una biblioteca `{b}` en el sitio; hay: {}",
                    drives
                        .iter()
                        .map(|(n, _)| format!("`{n}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })?;
        *self.ids.lock().unwrap_or_else(|e| e.into_inner()) = Some((sitio, d.clone()));
        Ok(d)
    }

    fn drive(&self) -> Result<String, Fallo> {
        Ok(format!("{}/drives/{}", self.base(), self.biblioteca()?))
    }

    /// Un nivel: lo que hay en una carpeta (vacía: la raíz; si no, acabada en
    /// `/` o no).
    pub fn hijos(&self, carpeta: &str) -> Result<Vec<Item>, Fallo> {
        let c = carpeta.trim_matches('/');
        let url = if c.is_empty() {
            format!("{}/root/children?$top={PAGINA}", self.drive()?)
        } else {
            format!(
                "{}/root:/{}:/children?$top={PAGINA}",
                self.drive()?,
                codificar(c, true)
            )
        };
        let carpeta = if c.is_empty() {
            String::new()
        } else {
            format!("{c}/")
        };
        Ok(self
            .paginado(url)?
            .iter()
            .map(|n| Item::de(n, &carpeta))
            .collect())
    }

    /// **Todo lo que hay bajo un prefijo**, carpeta a carpeta: los ficheros, y
    /// los cuadernos y accesos directos marcados (no las carpetas). Graph no
    /// lista por prefijo: se baja sólo por las carpetas que pueden casar. Un
    /// prefijo cuya carpeta no está, vacío.
    pub fn listar(&self, prefijo: &str) -> Result<Vec<Item>, Fallo> {
        let desde = match prefijo.rfind('/') {
            Some(i) => &prefijo[..i],
            None => "",
        };
        // La biblioteca, antes: que no esté no es un prefijo vacío.
        self.biblioteca()?;
        let mut pendientes = vec![desde.to_string()];
        let mut todo = Vec::new();
        while let Some(c) = pendientes.pop() {
            let hijos = match self.hijos(&c) {
                Ok(h) => h,
                Err(f) if f.no_esta() && c == desde => return Ok(vec![]),
                Err(f) => return Err(f),
            };
            for i in hijos {
                if i.clase == Clase::Carpeta {
                    let r = format!("{}/", i.ruta);
                    if r.starts_with(prefijo) || prefijo.starts_with(&r) {
                        pendientes.push(i.ruta);
                    }
                } else if i.ruta.starts_with(prefijo) {
                    todo.push(i);
                }
            }
        }
        todo.sort_by(|a, b| a.ruta.cmp(&b.ruta));
        Ok(todo)
    }

    /// Un elemento por su ruta.
    pub fn item_por_ruta(&self, ruta: &str) -> Result<Item, Fallo> {
        let n = self.json(&format!(
            "{}/root:/{}:",
            self.drive()?,
            codificar(ruta.trim_matches('/'), true)
        ))?;
        let carpeta = match ruta.rfind('/') {
            Some(i) => &ruta[..=i],
            None => "",
        };
        Ok(Item::de(&n, carpeta))
    }

    /// Un elemento por su id (la ruta, la de su `parentReference`).
    pub fn item(&self, id: &str) -> Result<Item, Fallo> {
        let n = self.json(&format!("{}/items/{}", self.drive()?, codificar(id, false)))?;
        let carpeta = n
            .get("parentReference")
            .and_then(|(_, p)| texto(p, "path"))
            .and_then(|p| p.split_once("root:").map(|(_, r)| descodificar(r)))
            .map(|r| {
                let r = r.trim_matches('/');
                if r.is_empty() {
                    String::new()
                } else {
                    format!("{r}/")
                }
            })
            .unwrap_or_default();
        Ok(Item::de(&n, &carpeta))
    }

    /// **Las versiones de un fichero**, de la nueva a la vieja.
    pub fn versiones(&self, id: &str) -> Result<Vec<Revision>, Fallo> {
        Ok(self
            .paginado(format!(
                "{}/items/{}/versions",
                self.drive()?,
                codificar(id, false)
            ))?
            .iter()
            .map(|v| Revision {
                id: texto(v, "id").unwrap_or_default(),
                tamano: texto(v, "size").and_then(|s| s.parse().ok()).unwrap_or(0),
                modificado: texto(v, "lastModifiedDateTime").unwrap_or_default(),
            })
            .collect())
    }

    /// **Bajar** la actual (`version: None`) o una vieja, entera o un rango
    /// (`bytes=…`): Graph contesta `302` a una URL al portador, que se mira
    /// ([`descarga_valida`]) y se pide **sin el token**.
    pub fn bajar(
        &self,
        id: &str,
        version: Option<&str>,
        rango: Option<&str>,
    ) -> Result<Leido, Fallo> {
        let url = match version {
            None => format!("{}/items/{}/content", self.drive()?, codificar(id, false)),
            Some(v) => format!(
                "{}/items/{}/versions/{}/content",
                self.drive()?,
                codificar(id, false),
                codificar(v, false)
            ),
        };
        let r = self.pedir(&url, &[])?;
        let r = if (300..400).contains(&r.status()) {
            let a = r.header("location").unwrap_or("").to_string();
            if !descarga_valida(&a, self.fuente.endpoint.is_some() && laboratorio()) {
                return Err(Fallo::ore(format!(
                    "Graph mandó la descarga a `{}`, que no es de SharePoint: no se pide",
                    a.split('?').next().unwrap_or("")
                )));
            }
            let mut p = self.agente.get(&a).set("user-agent", AGENTE);
            if let Some(g) = rango {
                p = p.set("range", g);
            }
            PETICIONES.fetch_add(1, Relaxed);
            match p.call() {
                Ok(x) if (300..400).contains(&x.status()) => {
                    return Err(Fallo::ore("la descarga de SharePoint volvió a redirigir"));
                }
                Ok(x) => x,
                Err(ureq::Error::Status(c, x)) => {
                    return Err(Fallo::de(c, x.into_string().unwrap_or_default()));
                }
                Err(e) => return Err(Fallo::ore(format!("SharePoint no contesta: {e}"))),
            }
        } else {
            r
        };
        let estado = r.status();
        let cabeceras = ["content-length", "content-range", "content-type"]
            .iter()
            .filter_map(|k| r.header(k).map(|v| (k.to_string(), v.to_string())))
            .collect();
        let lector: Box<dyn Read + Send> = Box::new(r.into_reader());
        Ok(Leido {
            estado,
            cabeceras,
            lector,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_url_dice_host_sitio_biblioteca_prefijo_y_la_app() {
        let f = leer("sharepoint://Contoso.sharepoint.com/sites/Finanzas/Documentos%20compartidos/2026/Q1?tenant=t&cliente=app").unwrap();
        assert_eq!(
            (
                f.host.as_str(),
                f.sitio.as_str(),
                f.biblioteca.as_str(),
                f.prefijo.as_str()
            ),
            (
                "contoso.sharepoint.com",
                "sites/Finanzas",
                "Documentos compartidos",
                "2026/Q1/"
            )
        );
        let r = leer("sharepoint://contoso.sharepoint.com/Documents?tenant=t&cliente=app").unwrap();
        assert_eq!((r.sitio.as_str(), r.prefijo.as_str()), ("", ""));
        let o = leer(
            "sharepoint://contoso-my.sharepoint.com/personal/ana_contoso_com/Documents/x/?tenant=t&cliente=a",
        )
        .unwrap();
        assert_eq!(
            (o.sitio.as_str(), o.prefijo.as_str()),
            ("personal/ana_contoso_com", "x/")
        );
        assert_eq!(
            publica(&f, "2026/a b.pdf"),
            "sharepoint://contoso.sharepoint.com/sites/Finanzas/Documentos%20compartidos/2026/a%20b.pdf?tenant=t&cliente=app"
        );
        for (mala, dice) in [
            (
                "sharepoint://contoso.sharepoint.com/sites/Finanzas?tenant=t&cliente=a",
                "falta la biblioteca",
            ),
            (
                "sharepoint://contoso.sharepoint.com/sites/?tenant=t&cliente=a",
                "falta el nombre del sitio",
            ),
            (
                "sharepoint://contoso.sharepoint.com/Documents",
                "falta `tenant`",
            ),
            (
                "sharepoint://evil.example.com/Documents?tenant=t&cliente=a",
                "no es un host de SharePoint",
            ),
            (
                "sharepoint://contoso.sharepoint.com/Documents?tenant=t&cliente=a&secreto=x",
                "sin secretos",
            ),
            (
                "sharepoint://contoso.sharepoint.com/Documents?tenant=t&cliente=a&endpoint=http://x",
                "no se le da a otro servidor",
            ),
            ("az://x/y", "la URL de SharePoint"),
        ] {
            let e = leer(mala).expect_err(mala);
            assert!(e.contains(dice), "{mala}: {e}");
        }
    }

    /// La URL de la descarga: sólo `https://<algo>.sharepoint.com`, sin
    /// usuario, sin trucos de host.
    #[test]
    fn la_descarga_solo_va_a_sharepoint() {
        for buena in [
            "https://contoso.sharepoint.com/sites/x/_layouts/15/download.aspx?UniqueId=1&tempauth=t",
            "https://contoso-my.sharepoint.com:443/personal/a/_layouts/15/download.aspx",
        ] {
            assert!(descarga_valida(buena, false), "{buena}");
        }
        for mala in [
            "http://contoso.sharepoint.com/x",
            "https://evil.com/x?h=contoso.sharepoint.com",
            "https://contoso.sharepoint.com.evil.com/x",
            "https://contoso.sharepoint.com@evil.com/x",
            "https://.sharepoint.com/x",
            "https://sharepoint.com/x",
            "",
        ] {
            assert!(!descarga_valida(mala, false), "{mala}");
        }
        assert!(descarga_valida("http://127.0.0.1:8791/x", true));
    }

    #[test]
    fn el_listado_sigue_las_paginas_y_no_sale_de_graph() {
        let base = "https://graph.microsoft.com/v1.0";
        let paginas = [
            r#"{"value":[{"id":"1","name":"a.pdf","size":"3","cTag":"\"c:{A},1\"","file":{"hashes":{"quickXorHash":"AAA="}}}],"@odata.nextLink":"https://graph.microsoft.com/v1.0/drives/d/items/r/children?$skiptoken=1"}"#,
            r#"{"value":[{"id":"2","name":"x","folder":{"childCount":0}},{"id":"3","name":"n","package":{"type":"oneNote"}},{"id":"4","name":"r","remoteItem":{"id":"z"}}]}"#,
        ];
        let mut i = 0;
        let todo = paginar(format!("{base}/drives/d/root/children"), base, |_| {
            i += 1;
            Ok(paginas[i - 1].to_string())
        })
        .unwrap();
        let items: Vec<Item> = todo.iter().map(|n| Item::de(n, "docs/")).collect();
        assert_eq!(
            items.iter().map(|x| x.clase).collect::<Vec<_>>(),
            [
                Clase::Fichero,
                Clase::Carpeta,
                Clase::Cuaderno,
                Clase::Acceso
            ]
        );
        assert_eq!(items[0].ruta, "docs/a.pdf");
        assert_eq!(items[0].ctag, "\"c:{A},1\"");
        assert_eq!(items[0].huella().as_deref(), Some("quickxor:AAA="));
        let fuera = paginar(format!("{base}/x"), base, |_| {
            Ok(r#"{"value":[],"@odata.nextLink":"https://evil.com/v1.0/x"}"#.to_string())
        })
        .expect_err("fuera de Graph");
        assert!(fuera.motivo().contains("no se sigue"), "{}", fuera.motivo());
    }

    #[test]
    fn el_error_de_graph_se_lee_por_su_codigo() {
        let f = Fallo::de(
            403,
            r#"{"error":{"code":"accessDenied","message":"Access denied\nmore","innerError":{}}}"#
                .into(),
        );
        assert_eq!(f.codigo, "accessDenied");
        assert_eq!(f.motivo(), "403 accessDenied Access denied");
        assert!(Fallo::de(404, "{}".into()).no_esta());
    }
}
