//! **Una carpeta de Google Drive de un cliente, como origen** (ADR 0082): la
//! API v3 de Drive, con la identidad que el cliente autorizó compartiéndole la
//! carpeta (D-O2).
//!
//! ```text
//! gdrive://<id de la carpeta>[/<prefijo>]                   la cuenta de la celda
//! gdrive://<id de la carpeta>[/<prefijo>]?endpoint=http://…  otro servidor (el laboratorio)
//! ```
//!
//! La carpeta es la **raíz** de la fuente: sus claves son rutas relativas a
//! ella. Drive no tiene prefijos —las carpetas son padres por ID—, así que el
//! árbol se arma listando por `parents` (X0 C2), y como el nombre no es una
//! clave (dos `a.csv` en la misma carpeta, X0 C3) los repetidos se llaman
//! `a~<id>.csv`.
//!
//! ⭐ **El token es al portador**: un `endpoint` que no es `https://…googleapis.com`
//! se lo daría a otro servidor, así que sólo se admite con
//! `ORE_GDRIVE_LABORATORIO=1` en el entorno (el laboratorio y el CI).
//!
//! # La identidad
//!
//! Drive pide el alcance `drive.readonly` (X0 F1) y el token de un metadata
//! server no lo trae, así que la cuenta que corre **se suplanta a sí misma** con
//! ese alcance (`generateAccessToken`, una hora, renovado antes de caducar):
//! necesita `roles/iam.serviceAccountTokenCreator` sobre sí misma, que
//! `ore-medios` ya tiene. En local, `ORE_GDRIVE_CUENTA` dice la cuenta y
//! `ORE_GCP_TOKEN` el token con el que se suplanta.
//!
//! # Cómo se fija una lectura
//!
//! La versión de un binario es su `headRevisionId` y la huella su `md5Checksum`
//! (X0 B1). Un lector sólo ve la revisión actual (X0 B2): pedir por esa
//! revisión da los bytes de entonces o un `404`, nunca otros. Drive no tiene
//! lectura condicional (X0 B4), así que no hay `If-Match` que mandar.

mod origen;

use ore_base::parse::Node;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

const API: &str = "https://www.googleapis.com";
const AGENTE: &str = concat!("ore-gdrive/", env!("CARGO_PKG_VERSION"));
const CORREO: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/email";
/// Lo mínimo para leer bytes (X0 F1): `drive.metadata.readonly` no los lee.
const ALCANCE: &str = "https://www.googleapis.com/auth/drive.readonly";
const CARPETA: &str = "application/vnd.google-apps.folder";
const ACCESO: &str = "application/vnd.google-apps.shortcut";
const CAMPOS: &str = "id,name,mimeType,size,md5Checksum,headRevisionId,modifiedTime,version,shortcutDetails(targetId,targetMimeType)";
const REINTENTOS: usize = 5;
const ESPERA_MAXIMA: u64 = 60;

static PETICIONES: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static ESPERAS: AtomicUsize = AtomicUsize::new(0);

/// Cuántas peticiones a Drive, cuántos bytes de listados y metadatos, y cuántas
/// esperas por el ritmo, para los avisos del driver (los ficheros van en flujo
/// y no se cuentan).
pub fn contadores() -> (usize, usize, usize) {
    (
        PETICIONES.load(Relaxed),
        BYTES.load(Relaxed),
        ESPERAS.load(Relaxed),
    )
}

/// La coordenada de una fuente de Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fuente {
    /// El ID de la carpeta que el cliente compartió.
    pub raiz: String,
    /// Vacío o acabado en `/`.
    pub prefijo: String,
    /// `https://www.googleapis.com`, u otro (el laboratorio).
    pub endpoint: String,
}

fn laboratorio() -> bool {
    std::env::var("ORE_GDRIVE_LABORATORIO").as_deref() == Ok("1")
}

/// `gdrive://<id>/<prefijo>?…` → la coordenada. Sin secreto que leer.
pub fn leer(url: &str) -> Result<Fuente, String> {
    const FORMA: &str = "a Google Drive URL is `gdrive://<folder id>[/<prefix>]`";
    let resto = url.trim().strip_prefix("gdrive://").ok_or(FORMA)?;
    let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let (raiz, prefijo) = camino.split_once('/').unwrap_or((camino, ""));
    if raiz.is_empty()
        || !raiz
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("`{raiz}` is not a Drive folder id; {FORMA}"));
    }
    let mut endpoint = API.to_string();
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        let v = descodificar(v);
        match k {
            "endpoint" => {
                endpoint = v.trim_end_matches('/').to_string();
                if !de_google(&endpoint) && !laboratorio() {
                    return Err(format!(
                        "`endpoint={endpoint}` is not a Google endpoint (`https://…googleapis.com`): the cell's token is never given to another server"
                    ));
                }
            }
            otro => return Err(format!("`{otro}` is not a parameter of a `gdrive://` URL")),
        }
    }
    let prefijo = descodificar(prefijo);
    let prefijo = prefijo.trim_matches('/');
    let prefijo = if prefijo.is_empty() {
        String::new()
    } else {
        format!("{prefijo}/")
    };
    Ok(Fuente {
        raiz: raiz.to_string(),
        prefijo,
        endpoint,
    })
}

/// `https://<algo>.googleapis.com`, sin puerto ni ruta.
fn de_google(endpoint: &str) -> bool {
    endpoint.strip_prefix("https://").is_some_and(|h| {
        h.ends_with(".googleapis.com")
            && h.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.".contains(&b))
    })
}

/// La URL de un prefijo, que se puede enseñar: una `gdrive://` no lleva secreto.
pub fn publica(f: &Fuente, prefijo: &str) -> String {
    let mut u = format!("gdrive://{}/{prefijo}", f.raiz);
    if f.endpoint != API {
        u.push_str(&format!("?endpoint={}", f.endpoint));
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

/// Un texto como valor de una consulta.
fn codificar(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Un valor entre comillas simples de la consulta `q` de Drive.
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Lo que un Doc, una Hoja o una Presentación nativos dan al exportarse
/// (X0 D2, D3): sólo los formatos que salen idénticos dos veces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nativo {
    pub extension: &'static str,
    pub tipo: &'static str,
}

fn nativo(mime: &str) -> Option<Nativo> {
    Some(match mime {
        "application/vnd.google-apps.document" => Nativo {
            extension: "txt",
            tipo: "text/plain",
        },
        "application/vnd.google-apps.spreadsheet" => Nativo {
            extension: "csv",
            tipo: "text/csv",
        },
        "application/vnd.google-apps.presentation" => Nativo {
            extension: "pdf",
            tipo: "application/pdf",
        },
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    Carpeta,
    /// Un fichero con bytes: `md5Checksum` y `headRevisionId`.
    Fichero,
    /// Un Doc, Hoja o Presentación: sin bytes, sólo `export` (X0 D1).
    Nativo(Nativo),
    /// Un acceso directo: se lista como tal y no se sigue (X0 C4).
    Acceso,
    /// Un formulario, un mapa, un script…: no es un fichero.
    Otro,
}

/// **Lo que Drive dice de un fichero o una carpeta.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    /// El nombre en Drive.
    pub nombre: String,
    pub tipo: String,
    pub clase: Clase,
    pub tamano: u64,
    /// El `md5Checksum`, en hexadecimal tal como lo da Drive.
    pub md5: Option<String>,
    /// La revisión actual: la versión de un binario.
    pub revision: Option<String>,
    pub modificado: String,
    /// El `version` de Drive: sube también con cambios de metadatos (X0 B5).
    pub version: String,
    /// El destino de un acceso directo.
    pub destino: Option<String>,
    /// Su ruta bajo la raíz, única entre hermanos. Vacía hasta que se arma.
    pub ruta: String,
}

impl Item {
    fn de(n: &Node) -> Item {
        let c = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let tipo = c("mimeType").unwrap_or_default();
        let clase = match tipo.as_str() {
            CARPETA => Clase::Carpeta,
            ACCESO => Clase::Acceso,
            t if nativo(t).is_some() => Clase::Nativo(nativo(t).expect("se acaba de ver")),
            t if t.starts_with("application/vnd.google-apps.") => Clase::Otro,
            _ => Clase::Fichero,
        };
        let destino = n
            .get("shortcutDetails")
            .and_then(|(_, d)| d.get("targetId"))
            .and_then(|(_, v)| v.as_str().map(String::from));
        // Los nativos dicen un `size` que no es de nadie (X0 D4).
        let tamano = match clase {
            Clase::Fichero => c("size").and_then(|s| s.parse().ok()).unwrap_or(0),
            _ => 0,
        };
        Item {
            id: c("id").unwrap_or_default(),
            nombre: c("name").unwrap_or_default(),
            tipo,
            clase,
            tamano,
            md5: c("md5Checksum"),
            revision: c("headRevisionId"),
            modificado: c("modifiedTime").unwrap_or_default(),
            version: c("version").unwrap_or_default(),
            destino,
            ruta: String::new(),
        }
    }

    /// La versión que fija una lectura: `<id>@<revisión>`. Un nativo no tiene
    /// revisión que un lector vea (X0 D5): `<id>#v<version>`.
    pub fn validador(&self) -> String {
        match &self.revision {
            Some(r) => format!("{}@{r}", self.id),
            None => format!("{}#v{}", self.id, self.version),
        }
    }

    /// `md5:<base64>`, como la escribe la colección: Drive la da en hexadecimal.
    pub fn huella(&self) -> Option<String> {
        let h = self.md5.as_deref()?;
        let bytes: Option<Vec<u8>> = (0..h.len() / 2)
            .map(|i| u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok())
            .collect();
        bytes
            .filter(|b| b.len() == 16)
            .map(|b| format!("md5:{}", ore_gcp::base64(&b)))
    }
}

/// El nombre que un item lleva en una ruta: sin barras, y con la extensión de
/// su exportación si es un nativo.
fn visible(i: &Item) -> String {
    let n = i.nombre.replace('/', "%2F");
    match i.clase {
        Clase::Nativo(x) if !n.ends_with(&format!(".{}", x.extension)) => {
            format!("{n}.{}", x.extension)
        }
        _ => n,
    }
}

/// `a.csv` → (`a`, `.csv`).
fn partir(n: &str) -> (&str, &str) {
    match n.rfind('.') {
        Some(p) if p > 0 => n.split_at(p),
        _ => (n, ""),
    }
}

/// **Los hijos de una carpeta con nombres únicos**: el nombre no es una clave en
/// Drive (X0 C3), así que cuando se repite, cada uno lleva su ID (`a~<id>.csv`).
pub fn con_rutas(mut hijos: Vec<Item>, base: &str) -> Vec<Item> {
    let mut cuenta: HashMap<String, usize> = HashMap::new();
    for i in &hijos {
        *cuenta.entry(visible(i)).or_default() += 1;
    }
    for i in &mut hijos {
        let v = visible(i);
        let nombre = if cuenta[&v] > 1 {
            let (tronco, ext) = partir(&v);
            format!("{tronco}~{}{ext}", i.id)
        } else {
            v
        };
        i.ruta = format!("{base}{nombre}");
    }
    hijos
}

/// **Un listado entero, página a página** (`pageToken`): `pagina(token)` pide
/// una y devuelve su JSON. Fuera de la red para probarlo.
pub fn paginar(
    mut pagina: impl FnMut(Option<&str>) -> Result<String, String>,
) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let texto = pagina(token.as_deref())?;
        let n = ore_base::parse::parse(&texto)
            .map_err(|e| format!("the Drive listing does not parse: {e:?}"))?;
        if let Some((_, fs)) = n.get("files") {
            items.extend(fs.items().iter().map(Item::de));
        }
        token = n
            .get("nextPageToken")
            .and_then(|(_, v)| v.as_str().map(String::from))
            .filter(|t| !t.is_empty());
        if token.is_none() {
            return Ok(items);
        }
    }
}

/// Lo que Drive contestó cuando no fue un 2xx: el estado, su `reason` y el
/// cuerpo (`0`: no contestó).
#[derive(Debug, Clone)]
pub struct Fallo {
    pub estado: u16,
    pub motivo_api: String,
    pub cuerpo: String,
}

impl Fallo {
    /// Uno ya hecho (para probar el texto de lo que se dice).
    pub fn nuevo(estado: u16, motivo_api: &str, cuerpo: &str) -> Fallo {
        Fallo {
            estado,
            motivo_api: motivo_api.into(),
            cuerpo: cuerpo.into(),
        }
    }

    fn de(estado: u16, cuerpo: String) -> Fallo {
        let n = ore_base::parse::parse(&cuerpo).ok();
        let motivo_api = n
            .as_ref()
            .and_then(|n| n.get("error"))
            .and_then(|(_, e)| e.get("errors"))
            .and_then(|(_, es)| es.items().first())
            .and_then(|e| e.get("reason"))
            .and_then(|(_, r)| r.as_str().map(String::from))
            .unwrap_or_default();
        Fallo {
            estado,
            motivo_api,
            cuerpo,
        }
    }

    pub fn propio(m: impl Into<String>) -> Fallo {
        Fallo {
            estado: 0,
            motivo_api: String::new(),
            cuerpo: m.into(),
        }
    }

    pub fn motivo(&self) -> String {
        if self.estado == 0 {
            return self.cuerpo.clone();
        }
        let m = ore_base::parse::parse(&self.cuerpo)
            .ok()
            .and_then(|n| {
                n.get("error")
                    .and_then(|(_, e)| e.get("message"))
                    .and_then(|(_, m)| m.as_str().map(String::from))
            })
            .unwrap_or_else(|| self.cuerpo.trim().chars().take(200).collect());
        format!("{} {m}", self.estado)
    }

    /// Drive pide esperar: `429`, un `5xx`, o un `403` por ritmo (no por cuota
    /// de almacenamiento, que no se arregla esperando).
    fn reintentable(&self) -> bool {
        self.estado == 429
            || (500..600).contains(&self.estado)
            || (self.estado == 403 && self.motivo_api.contains("ateLimitExceeded"))
    }

    pub fn no_esta(&self) -> bool {
        self.estado == 404
    }
}

/// **Una carpeta de Drive, lista para leer**: su coordenada, el cliente HTTP (uno
/// para toda la vida: con conexiones vivas), el token y lo que ya sabe de lo
/// listado (para leer por clave sin volver a recorrer).
pub struct Gdrive {
    pub fuente: Fuente,
    agente: ureq::Agent,
    token: Mutex<Option<(String, Instant)>>,
    conocidos: Mutex<HashMap<String, Item>>,
}

impl Gdrive {
    pub fn de(fuente: Fuente) -> Result<Gdrive, String> {
        Ok(Gdrive {
            fuente,
            agente: ore_gcp::cliente()?,
            token: Mutex::new(None),
            conocidos: Mutex::new(HashMap::new()),
        })
    }

    pub fn de_url(url: &str) -> Result<Gdrive, String> {
        Gdrive::de(leer(url)?)
    }

    /// La cuenta que lee: `ORE_GDRIVE_CUENTA`, o la que corre (metadata server).
    pub fn cuenta(&self) -> Result<String, String> {
        if let Ok(c) = std::env::var("ORE_GDRIVE_CUENTA")
            && !c.is_empty()
        {
            return Ok(c);
        }
        self.agente
            .get(CORREO)
            .set("metadata-flavor", "Google")
            .call()
            .map_err(|e| {
                format!(
                    "the metadata server does not say which account is running ({e}); outside GCP, set `ORE_GDRIVE_CUENTA`"
                )
            })?
            .into_string()
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("could not read the account email: {e}"))
    }

    /// **¿Hay identidad con la que leer?** El token de Drive de la cuenta, que se
    /// suplanta a sí misma con `drive.readonly`: si no tiene
    /// `roles/iam.serviceAccountTokenCreator` sobre sí misma, aquí se sabe.
    pub fn token(&self) -> Result<String, String> {
        let mut v = self.token.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, caduca)) = v.as_ref()
            && Instant::now() + Duration::from_secs(300) < *caduca
        {
            return Ok(t.clone());
        }
        let cuenta = self.cuenta()?;
        let base = ore_gcp::Credencial::del_entorno();
        let t = ore_gcp::suplantar(&base, &cuenta, &[ALCANCE])?;
        *v = Some((t.clone(), Instant::now() + Duration::from_secs(3600)));
        Ok(t)
    }

    /// Una petición, con las esperas que Drive pide (`429`, `5xx`, el `403` de
    /// ritmo): la espera es la de `Retry-After`, o crece sola, con un tope.
    fn llamar(
        &self,
        metodo: &str,
        url: &str,
        rango: Option<&str>,
    ) -> Result<ureq::Response, Fallo> {
        let mut intento = 0;
        loop {
            let t = self.token().map_err(Fallo::propio)?;
            PETICIONES.fetch_add(1, Relaxed);
            let mut r = self
                .agente
                .request(metodo, url)
                .set("user-agent", AGENTE)
                .set("authorization", &format!("Bearer {t}"));
            if let Some(x) = rango {
                r = r.set("range", x);
            }
            let (f, espera) = match r.call() {
                Ok(x) => return Ok(x),
                Err(ureq::Error::Status(c, x)) => {
                    let espera = x.header("retry-after").and_then(|v| v.parse::<u64>().ok());
                    (Fallo::de(c, x.into_string().unwrap_or_default()), espera)
                }
                Err(e) => (Fallo::propio(format!("Drive is not responding: {e}")), None),
            };
            if !f.reintentable() || intento >= REINTENTOS {
                return Err(f);
            }
            ESPERAS.fetch_add(1, Relaxed);
            let s = espera.unwrap_or(1 << intento).min(ESPERA_MAXIMA);
            std::thread::sleep(Duration::from_secs(s));
            intento += 1;
        }
    }

    fn texto(&self, url: &str) -> Result<String, Fallo> {
        let t = self
            .llamar("GET", url, None)?
            .into_string()
            .map_err(|e| Fallo::propio(format!("could not read the answer of Drive: {e}")))?;
        BYTES.fetch_add(t.len(), Relaxed);
        Ok(t)
    }

    fn lista(&self, q: &str) -> Result<Vec<Item>, Fallo> {
        let base = format!(
            "{}/drive/v3/files?q={}&fields={}&pageSize=1000&supportsAllDrives=true&includeItemsFromAllDrives=true&orderBy=name",
            self.fuente.endpoint,
            codificar(q),
            codificar(&format!("nextPageToken,files({CAMPOS})"))
        );
        let mut fallo = None;
        let r = paginar(|token| {
            let url = match token {
                Some(t) => format!("{base}&pageToken={}", codificar(t)),
                None => base.clone(),
            };
            self.texto(&url).map_err(|f| {
                let m = f.motivo();
                fallo = Some(f);
                m
            })
        });
        r.map_err(|m| fallo.unwrap_or_else(|| Fallo::propio(m)))
    }

    /// Los hijos directos de una carpeta, sin papelera (un lector tampoco la ve:
    /// X0 C5), con su ruta bajo `base`.
    pub fn hijos(&self, carpeta: &str, base: &str) -> Result<Vec<Item>, Fallo> {
        let q = format!("{} in parents and trashed=false", literal(carpeta));
        Ok(con_rutas(self.lista(&q)?, base))
    }

    /// Los metadatos de un fichero por su ID.
    pub fn item(&self, id: &str) -> Result<Item, Fallo> {
        let url = format!(
            "{}/drive/v3/files/{}?supportsAllDrives=true&fields={}",
            self.fuente.endpoint,
            codificar(id),
            codificar(CAMPOS)
        );
        let texto = self.texto(&url)?;
        let n = ore_base::parse::parse(&texto)
            .map_err(|e| Fallo::propio(format!("the metadata does not parse: {e:?}")))?;
        Ok(Item::de(&n))
    }

    /// El ID de la carpeta de una ruta bajo la raíz (`a/b/`), segmento a
    /// segmento. Un segmento `nombre~<id>` es la carpeta de ese ID.
    pub fn carpeta_de(&self, ruta: &str) -> Result<String, Fallo> {
        let mut id = self.fuente.raiz.clone();
        for seg in ruta.split('/').filter(|s| !s.is_empty()) {
            let hijos = self.hijos(&id, "")?;
            let Some(c) = hijos
                .into_iter()
                .find(|h| h.clase == Clase::Carpeta && h.ruta == seg)
            else {
                return Err(Fallo {
                    estado: 404,
                    motivo_api: "notFound".into(),
                    cuerpo: format!("the folder `{seg}` of `{ruta}` is not in the Drive folder"),
                });
            };
            id = c.id;
        }
        Ok(id)
    }

    /// **Todo lo que hay bajo una ruta**, recursivo: los ficheros y los nativos
    /// con su ruta, y cuántos accesos directos y otros se saltaron.
    pub fn recorrer(&self, ruta: &str) -> Result<(Vec<Item>, usize), Fallo> {
        let ruta = ruta.trim_matches('/');
        let base = if ruta.is_empty() {
            String::new()
        } else {
            format!("{ruta}/")
        };
        let mut pendientes = vec![(self.carpeta_de(ruta)?, base)];
        let mut vistas = std::collections::HashSet::new();
        let mut items = Vec::new();
        let mut saltados = 0;
        while let Some((id, base)) = pendientes.pop() {
            if !vistas.insert(id.clone()) {
                continue;
            }
            for h in self.hijos(&id, &base)? {
                match h.clase {
                    Clase::Carpeta => pendientes.push((h.id.clone(), format!("{}/", h.ruta))),
                    Clase::Fichero | Clase::Nativo(_) => items.push(h),
                    Clase::Acceso | Clase::Otro => saltados += 1,
                }
            }
        }
        items.sort_by(|a, b| a.ruta.cmp(&b.ruta));
        let mut c = self.conocidos.lock().unwrap_or_else(|e| e.into_inner());
        for i in &items {
            c.insert(i.ruta.clone(), i.clone());
        }
        Ok((items, saltados))
    }

    /// El item de una clave: el que ya se listó, o se busca en su carpeta.
    pub fn resolver(&self, clave: &str) -> Result<Item, Fallo> {
        if let Some(i) = self
            .conocidos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(clave)
        {
            return Ok(i.clone());
        }
        let (dir, _) = clave.rsplit_once('/').unwrap_or(("", clave));
        let id = self.carpeta_de(dir)?;
        let base = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        self.hijos(&id, &base)?
            .into_iter()
            .find(|h| h.ruta == clave && matches!(h.clase, Clase::Fichero | Clase::Nativo(_)))
            .ok_or_else(|| Fallo {
                estado: 404,
                motivo_api: "notFound".into(),
                cuerpo: format!("`{clave}` is not in the Drive folder"),
            })
    }

    /// **Los bytes de un fichero**, de su revisión si se dice
    /// (`revisions/{id}?alt=media`: esa o un `404`, nunca otra: X0 B2) y del
    /// rango si se pide (`bytes=…`, también por el final: X0 B3), en flujo.
    pub fn bajar(
        &self,
        id: &str,
        revision: Option<&str>,
        rango: Option<&str>,
    ) -> Result<ore_objetos::Leido, Fallo> {
        let url = match revision.filter(|r| !r.is_empty()) {
            Some(r) => format!(
                "{}/drive/v3/files/{}/revisions/{}?alt=media",
                self.fuente.endpoint,
                codificar(id),
                codificar(r)
            ),
            None => format!(
                "{}/drive/v3/files/{}?alt=media&supportsAllDrives=true",
                self.fuente.endpoint,
                codificar(id)
            ),
        };
        let x = self.llamar("GET", &url, rango)?;
        let cabeceras = x
            .headers_names()
            .into_iter()
            .filter_map(|n| {
                x.header(&n)
                    .map(|v| (n.to_ascii_lowercase(), v.to_string()))
            })
            .collect();
        Ok(ore_objetos::Leido {
            estado: x.status(),
            cabeceras,
            lector: Box::new(x.into_reader()),
        })
    }

    /// Una página de hijos de la raíz, para `check`: ¿se ve la carpeta?
    pub fn raiz(&self) -> Result<Item, Fallo> {
        self.item(&self.fuente.raiz)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_url_dice_la_carpeta_y_el_prefijo() {
        let f = leer("gdrive://1ZcGAslw7gRV4B0-E/datos/2026").unwrap();
        assert_eq!(f.raiz, "1ZcGAslw7gRV4B0-E");
        assert_eq!(f.prefijo, "datos/2026/");
        assert_eq!(f.endpoint, API);
        assert_eq!(
            publica(&f, &f.prefijo),
            "gdrive://1ZcGAslw7gRV4B0-E/datos/2026/"
        );
        assert!(leer("gdrive://").is_err());
        assert!(leer("gdrive://a b").is_err());
        assert!(leer("gdrive://abc?otro=1").unwrap_err().contains("otro"));
        assert!(leer("s3://x").is_err());
    }

    #[test]
    fn el_token_no_se_da_a_otro_servidor() {
        let e = leer("gdrive://abc?endpoint=http://127.0.0.1:9").unwrap_err();
        assert!(e.contains("never given to another server"), "{e}");
        assert!(leer("gdrive://abc?endpoint=https://private.googleapis.com").is_ok());
        assert!(leer("gdrive://abc?endpoint=https://googleapis.com.evil.io").is_err());
    }

    fn pagina(n: usize) -> String {
        let fs: Vec<String> = (0..n)
            .map(|i| format!(r#"{{"id":"i{i}","name":"f{i}.csv","mimeType":"text/csv","size":"8","md5Checksum":"344fd2323dd33d5c058b7cf27de029e8","headRevisionId":"r{i}","version":"3","modifiedTime":"2026-10-31T08:55:28.709Z"}}"#))
            .collect();
        fs.join(",")
    }

    #[test]
    fn un_listado_se_recorre_por_paginas() {
        let mut pedidas = Vec::new();
        let items = paginar(|t| {
            pedidas.push(t.map(String::from));
            Ok(match t {
                None => format!(r#"{{"nextPageToken":"p2","files":[{}]}}"#, pagina(2)),
                Some("p2") => format!(r#"{{"files":[{}]}}"#, pagina(1)),
                Some(o) => panic!("{o}"),
            })
        })
        .unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(pedidas, vec![None, Some("p2".to_string())]);
        assert_eq!(items[0].clase, Clase::Fichero);
        assert_eq!(items[0].tamano, 8);
        assert_eq!(items[0].validador(), "i0@r0");
    }

    #[test]
    fn la_huella_es_el_md5_de_drive_en_base64() {
        let i = Item::de(
            &ore_base::parse::parse(
                r#"{"id":"x","name":"a","mimeType":"text/csv","md5Checksum":"344fd2323dd33d5c058b7cf27de029e8"}"#,
            )
            .unwrap(),
        );
        // El mismo md5, hex → base64 (a.csv de X0: `x,y\n1,2\n`).
        assert_eq!(i.huella().as_deref(), Some("md5:NE/SMj3TPVwFi3zyfeAp6A=="));
        assert_eq!(Item { md5: None, ..i }.huella(), None);
    }

    fn item(j: &str) -> Item {
        Item::de(&ore_base::parse::parse(j).unwrap())
    }

    #[test]
    fn los_nombres_repetidos_llevan_su_id_y_los_nativos_su_extension() {
        let hijos = vec![
            item(r#"{"id":"A1","name":"a.csv","mimeType":"text/csv"}"#),
            item(r#"{"id":"A2","name":"a.csv","mimeType":"text/csv"}"#),
            item(r#"{"id":"B","name":"b.csv","mimeType":"text/csv"}"#),
            item(
                r#"{"id":"D","name":"doc nativo","mimeType":"application/vnd.google-apps.document","size":"1024"}"#,
            ),
            item(
                r#"{"id":"H","name":"hoja","mimeType":"application/vnd.google-apps.spreadsheet"}"#,
            ),
            item(r#"{"id":"S","name":"a/b","mimeType":"text/plain"}"#),
        ];
        let rutas: Vec<_> = con_rutas(hijos, "x/").into_iter().map(|i| i.ruta).collect();
        assert_eq!(
            rutas,
            vec![
                "x/a~A1.csv",
                "x/a~A2.csv",
                "x/b.csv",
                "x/doc nativo.txt",
                "x/hoja.csv",
                "x/a%2Fb"
            ]
        );
    }

    #[test]
    fn cada_clase_se_distingue_y_un_nativo_no_tiene_tamano() {
        let c = |m: &str| {
            item(&format!(
                r#"{{"id":"x","name":"n","mimeType":"{m}","size":"1024"}}"#
            ))
        };
        assert_eq!(c(CARPETA).clase, Clase::Carpeta);
        assert_eq!(c(ACCESO).clase, Clase::Acceso);
        assert_eq!(c("application/vnd.google-apps.form").clase, Clase::Otro);
        assert!(matches!(
            c("application/vnd.google-apps.document").clase,
            Clase::Nativo(_)
        ));
        assert_eq!(c("application/vnd.google-apps.document").tamano, 0);
        assert_eq!(c("text/csv").tamano, 1024);
        assert_eq!(c("application/vnd.google-apps.document").validador(), "x#v");
    }

    #[test]
    fn la_consulta_escapa_las_comillas() {
        assert_eq!(literal("it's"), r"'it\'s'");
    }

    #[test]
    fn un_403_de_ritmo_se_espera_y_uno_de_cuota_no() {
        let f = |c, cuerpo: &str| Fallo::de(c, cuerpo.to_string());
        let ritmo =
            r#"{"error":{"code":403,"message":"x","errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        let cuota =
            r#"{"error":{"code":403,"message":"x","errors":[{"reason":"storageQuotaExceeded"}]}}"#;
        assert!(f(403, ritmo).reintentable());
        assert!(!f(403, cuota).reintentable());
        assert!(f(429, "").reintentable() && f(503, "").reintentable());
        assert!(!f(404, "").reintentable());
    }
}
