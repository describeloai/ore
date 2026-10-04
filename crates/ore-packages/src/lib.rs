//! **Los registros de paquetes** (0050 L6·2·1): la ficha de una librería y la
//! búsqueda, en el registro de cada ecosistema, en UNA forma.
//!
//! | ecosistema | ficha | búsqueda |
//! |---|---|---|
//! | `node` | `registry.npmjs.org/<n>/latest` + el documento abreviado (versiones) | `/-/v1/search` |
//! | `python` | `pypi.org/pypi/<n>/json` | PyPI no busca: el nombre exacto, o nada |
//! | `jvm` | `repo1.maven.org`: `maven-metadata.xml` (versiones) + el POM de la última | `search.maven.org` |
//!
//! La forma de la ficha, la misma en los tres:
//!
//! ```json
//! {"nombre":"lodash","descripcion":"…","ultima":"4.17.21",
//!  "versiones":[{"version":"4.17.21","fecha":"2021-02-20"}],
//!  "licencia":"MIT","web":"https://lodash.com/","repositorio":"https://github.com/lodash/lodash",
//!  "registro":"https://www.npmjs.com/package/lodash"}
//! ```
//!
//! ⛔ Sólo se habla con [`HOSTS`]: las URLs se construyen aquí, con el nombre
//!   validado y codificado, y [`traer`] se niega a cualquier otra. Lo que entra
//!   es un ecosistema, un nombre o un texto; nunca una URL.
use serde_json::{Map, Value, json};
use std::io::Read;
use std::time::Duration;

/// Los únicos sitios con los que este programa habla.
pub const HOSTS: [&str; 4] = [
    "registry.npmjs.org",
    "pypi.org",
    "search.maven.org",
    "repo1.maven.org",
];

/// Cuántas versiones se devuelven: las recientes, para elegir; el historial
/// entero está en el registro (`registro`).
pub const VERSIONES: usize = 30;
/// Resultados de una búsqueda.
pub const RESULTADOS: usize = 20;
/// Lo más que se lee de una respuesta: el documento de npm de un paquete con
/// miles de versiones pesa megas; uno de cientos, no es un paquete.
const TOPE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ecosistema {
    Node,
    Python,
    Jvm,
}

impl Ecosistema {
    pub fn de(s: &str) -> Option<Self> {
        match s {
            "node" => Some(Self::Node),
            "python" => Some(Self::Python),
            "jvm" => Some(Self::Jvm),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Peticion {
    Ficha(Ecosistema, String),
    Buscar(Ecosistema, String),
}

/// Por qué no: `codigo` como HTTP (422 lo pedido no vale, 404 no existe, 502
/// el registro no contestó bien).
#[derive(Debug, PartialEq, Eq)]
pub struct Fallo {
    pub codigo: u16,
    pub mensaje: String,
}

fn fallo(codigo: u16, mensaje: impl Into<String>) -> Fallo {
    Fallo {
        codigo,
        mensaje: mensaje.into(),
    }
}

/// `{"entorno":"node","nombre":"lodash"}` o `{"entorno":"node","q":"date"}`.
pub fn peticion(entrada: &str) -> Result<Peticion, Fallo> {
    let v: Value = serde_json::from_str(entrada.trim()).map_err(|_| {
        fallo(
            422,
            "la petición es JSON: {\"entorno\", \"nombre\"} o {\"entorno\", \"q\"}",
        )
    })?;
    let e = v
        .get("entorno")
        .and_then(Value::as_str)
        .and_then(Ecosistema::de)
        .ok_or_else(|| fallo(422, "`entorno` es `node`, `python` o `jvm`"))?;
    match (
        v.get("nombre").and_then(Value::as_str),
        v.get("q").and_then(Value::as_str),
    ) {
        (Some(n), None) => {
            let n = n.trim();
            if nombre_valido(e, n) {
                Ok(Peticion::Ficha(e, n.to_string()))
            } else {
                Err(fallo(
                    422,
                    format!("`{n}` no es un nombre de paquete de {}", nombre_de(e)),
                ))
            }
        }
        (None, Some(q)) => {
            let q = q.trim();
            if q.is_empty() || q.chars().count() > 100 || q.chars().any(char::is_control) {
                return Err(fallo(
                    422,
                    "el texto de búsqueda tiene de 1 a 100 caracteres",
                ));
            }
            Ok(Peticion::Buscar(e, q.to_string()))
        }
        _ => Err(fallo(
            422,
            "o `nombre` (la ficha) o `q` (la búsqueda), uno de los dos",
        )),
    }
}

fn nombre_de(e: Ecosistema) -> &'static str {
    match e {
        Ecosistema::Node => "npm",
        Ecosistema::Python => "PyPI",
        Ecosistema::Jvm => "Maven (`grupo:artefacto`)",
    }
}

/// El nombre como lo admite cada registro: npm (`@ámbito/n`, minúsculas),
/// PEP 508 y `grupo:artefacto`. Lo que no pasa no llega a una URL.
pub fn nombre_valido(e: Ecosistema, n: &str) -> bool {
    let parte = |p: &str, extra: &[char]| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
    };
    match e {
        Ecosistema::Node => {
            if n.len() > 214 || n.chars().any(|c| c.is_ascii_uppercase()) {
                return false;
            }
            let (ambito, nombre) = match n.strip_prefix('@') {
                Some(r) => match r.split_once('/') {
                    Some((a, b)) => (Some(a), b),
                    None => return false,
                },
                None => (None, n),
            };
            let bien = |p: &str| parte(p, &['-', '.', '_', '~']) && !p.starts_with(['.', '_']);
            ambito.is_none_or(bien) && bien(nombre)
        }
        Ecosistema::Python => {
            n.len() <= 100
                && parte(n, &['-', '.', '_'])
                && n.starts_with(|c: char| c.is_ascii_alphanumeric())
                && n.ends_with(|c: char| c.is_ascii_alphanumeric())
        }
        Ecosistema::Jvm => {
            n.len() <= 200
                && matches!(n.split_once(':'), Some((g, a)) if parte(g, &['-', '.', '_']) && parte(a, &['-', '.', '_']))
        }
    }
}

/// Codificación de URL: se queda lo no reservado de RFC 3986.
pub fn codificar(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

// ── Las versiones ───────────────────────────────────────────────────────────

/// `1.10.0` > `1.9.3` > `1.9.3-rc.1`: los números como números, y lo que va
/// tras un `-` (una prerelease) por debajo de la versión sin él.
pub fn comparar(a: &str, b: &str) -> std::cmp::Ordering {
    let partes = |v: &str| -> (Vec<u64>, Option<String>) {
        let (n, pre) = match v.split_once('-') {
            Some((n, p)) => (n, Some(p.to_string())),
            None => (v, None),
        };
        let nums = n
            .split(['.', '+'])
            .map(|p| {
                p.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect();
        (nums, pre)
    };
    let (na, pa) = partes(a);
    let (nb, pb) = partes(b);
    let largo = na.len().max(nb.len());
    for i in 0..largo {
        let c = na.get(i).unwrap_or(&0).cmp(nb.get(i).unwrap_or(&0));
        if c.is_ne() {
            return c;
        }
    }
    match (pa, pb) {
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Una versión de prueba, como la entiende cada ecosistema: en npm, lo que
/// lleva `-` (semver); en Python, `a`/`b`/`rc`/`dev` (PEP 440; `post` es
/// estable); en Maven, `SNAPSHOT`, `alpha`, `beta`, `rc`, `M1`… (`-jre` no).
fn es_prerelease(e: Ecosistema, v: &str) -> bool {
    let v = v.to_ascii_lowercase();
    let palabras = || {
        v.split(|c: char| !c.is_ascii_alphabetic())
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect::<Vec<_>>()
    };
    match e {
        Ecosistema::Node => v.contains('-'),
        Ecosistema::Python => palabras().iter().any(|w| {
            [
                "a", "b", "c", "rc", "alpha", "beta", "pre", "preview", "dev",
            ]
            .contains(&w.as_str())
        }),
        Ecosistema::Jvm => {
            palabras().iter().any(|w| {
                ["snapshot", "alpha", "beta", "rc", "cr", "milestone", "ea"].contains(&w.as_str())
            }) || v.split(['-', '.']).any(|p| {
                p.len() >= 2 && p.starts_with('m') && p[1..].chars().all(|c| c.is_ascii_digit())
            })
        }
    }
}

fn texto(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// `git+https://github.com/a/b.git` → `https://github.com/a/b`: un enlace que
/// se puede abrir.
fn repositorio_limpio(u: &str) -> Option<String> {
    let u = u.trim().trim_start_matches("git+");
    let u = u
        .strip_prefix("git://")
        .map(|r| format!("https://{r}"))
        .or_else(|| {
            u.strip_prefix("git@github.com:")
                .map(|r| format!("https://github.com/{r}"))
        })
        .unwrap_or_else(|| u.to_string());
    let u = u.trim_end_matches(".git").to_string();
    u.starts_with("https://").then_some(u)
}

#[allow(clippy::too_many_arguments)]
fn ficha(
    nombre: String,
    descripcion: Option<String>,
    ultima: Option<String>,
    versiones: Vec<(String, Option<String>)>,
    licencia: Option<String>,
    web: Option<String>,
    repositorio: Option<String>,
    registro: String,
) -> Value {
    let mut m = Map::new();
    m.insert("nombre".into(), json!(nombre));
    for (k, v) in [
        ("descripcion", descripcion),
        ("ultima", ultima),
        ("licencia", licencia),
        ("web", web),
        ("repositorio", repositorio),
    ] {
        if let Some(v) = v {
            m.insert(k.into(), json!(v));
        }
    }
    m.insert(
        "versiones".into(),
        Value::Array(
            versiones
                .into_iter()
                .map(|(v, f)| match f {
                    Some(f) => json!({"version": v, "fecha": f}),
                    None => json!({"version": v}),
                })
                .collect(),
        ),
    );
    m.insert("registro".into(), json!(registro));
    Value::Object(m)
}

// ── npm ─────────────────────────────────────────────────────────────────────

/// El nombre de npm en una ruta: el `/` del ámbito, codificado (`@a%2Fb`).
fn ruta_npm(n: &str) -> String {
    match n.strip_prefix('@').and_then(|r| r.split_once('/')) {
        Some((a, b)) => format!("@{}%2F{}", codificar(a), codificar(b)),
        None => codificar(n),
    }
}

/// La ficha de npm: el manifiesto de `latest` (descripción, licencia, web) y
/// el documento abreviado (las versiones y las etiquetas).
pub fn ficha_npm(nombre: &str, latest: &Value, abreviado: &Value) -> Value {
    let ultima =
        texto(abreviado.pointer("/dist-tags/latest")).or_else(|| texto(latest.get("version")));
    let mut vs: Vec<String> = abreviado
        .get("versions")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    vs.sort_by(|a, b| comparar(b, a));
    let versiones = vs
        .into_iter()
        .filter(|v| !es_prerelease(Ecosistema::Node, v) || Some(v) == ultima.as_ref())
        .take(VERSIONES)
        .map(|v| (v, None))
        .collect();
    let licencia = texto(latest.get("license")).or_else(|| texto(latest.pointer("/license/type")));
    let repositorio = texto(latest.get("repository"))
        .or_else(|| texto(latest.pointer("/repository/url")))
        .and_then(|u| repositorio_limpio(&u));
    ficha(
        texto(latest.get("name")).unwrap_or_else(|| nombre.to_string()),
        texto(latest.get("description")),
        ultima,
        versiones,
        licencia,
        texto(latest.get("homepage")),
        repositorio,
        format!("https://www.npmjs.com/package/{nombre}"),
    )
}

pub fn busqueda_npm(v: &Value) -> Value {
    let r: Vec<Value> = v
        .get("objects")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|o| o.get("package"))
                .filter_map(|p| {
                    let mut m = Map::new();
                    m.insert("nombre".into(), json!(texto(p.get("name"))?));
                    if let Some(x) = texto(p.get("version")) {
                        m.insert("ultima".into(), json!(x));
                    }
                    if let Some(x) = texto(p.get("description")) {
                        m.insert("descripcion".into(), json!(x));
                    }
                    Some(Value::Object(m))
                })
                .take(RESULTADOS)
                .collect()
        })
        .unwrap_or_default();
    json!({ "resultados": r })
}

// ── PyPI ────────────────────────────────────────────────────────────────────

/// La ficha de PyPI (`/pypi/<n>/json`): las versiones por FECHA de subida —el
/// orden de los números en Python no siempre es el de PEP 440 a simple vista—,
/// sin las que no tienen ficheros.
pub fn ficha_pypi(v: &Value) -> Value {
    let info = v.get("info").cloned().unwrap_or(Value::Null);
    let nombre = texto(info.get("name")).unwrap_or_default();
    let mut vs: Vec<(String, String)> = v
        .get("releases")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(ver, fs)| {
                    let fs = fs.as_array()?;
                    if fs.is_empty()
                        || fs
                            .iter()
                            .all(|f| f.get("yanked") == Some(&Value::Bool(true)))
                    {
                        return None;
                    }
                    let fecha = fs
                        .iter()
                        .filter_map(|f| {
                            texto(f.get("upload_time_iso_8601"))
                                .or_else(|| texto(f.get("upload_time")))
                        })
                        .min()?;
                    Some((ver.clone(), fecha))
                })
                .collect()
        })
        .unwrap_or_default();
    vs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| comparar(&b.0, &a.0)));
    let ultima = texto(info.get("version"));
    let versiones = vs
        .into_iter()
        .filter(|(ver, _)| !es_prerelease(Ecosistema::Python, ver) || Some(ver) == ultima.as_ref())
        .take(VERSIONES)
        .map(|(ver, f)| (ver, Some(f.chars().take(10).collect())))
        .collect();
    // La licencia: la expresión SPDX si la hay; si no, el campo libre, que a
    // veces es el texto entero de la licencia: su primera línea, corta.
    // …y antes que el campo libre, el clasificador «License :: … :: MIT License»:
    // el campo libre a veces es una línea de copyright, no una licencia.
    let clasificada = info
        .get("classifiers")
        .and_then(Value::as_array)
        .and_then(|cs| {
            cs.iter()
                .filter_map(Value::as_str)
                .filter(|c| c.starts_with("License ::"))
                .filter_map(|c| c.rsplit(" :: ").next())
                .find(|l| *l != "OSI Approved")
                .map(String::from)
        });
    let licencia = texto(info.get("license_expression"))
        .or(clasificada)
        .or_else(|| {
            texto(info.get("license"))
                .and_then(|l| l.lines().next().map(str::trim).map(String::from))
                .filter(|l| {
                    !l.is_empty()
                        && l.chars().count() <= 60
                        && !l.to_ascii_lowercase().starts_with("copyright")
                })
        });
    let urls = info.get("project_urls").and_then(Value::as_object);
    let url_de = |claves: &[&str]| {
        urls.and_then(|m| {
            m.iter()
                .find(|(k, _)| claves.iter().any(|c| k.eq_ignore_ascii_case(c)))
                .and_then(|(_, v)| texto(Some(v)))
        })
    };
    let web =
        texto(info.get("home_page")).or_else(|| url_de(&["Homepage", "Home", "Documentation"]));
    let repositorio = url_de(&["Source", "Source Code", "Repository", "Code", "GitHub"])
        .and_then(|u| repositorio_limpio(&u));
    ficha(
        nombre.clone(),
        texto(info.get("summary")),
        ultima,
        versiones,
        licencia,
        web,
        repositorio,
        format!("https://pypi.org/project/{nombre}/"),
    )
}

/// PyPI no tiene búsqueda (la quitó en 2018): el nombre exacto es el único
/// resultado posible.
pub fn busqueda_pypi(ficha: &Value) -> Value {
    let mut m = Map::new();
    if let Some(n) = texto(ficha.get("nombre")) {
        m.insert("nombre".into(), json!(n));
    }
    for k in ["ultima", "descripcion"] {
        if let Some(x) = ficha.get(k) {
            m.insert(k.into(), x.clone());
        }
    }
    json!({ "resultados": [Value::Object(m)] })
}

// ── Maven Central ───────────────────────────────────────────────────────────

/// La primera `<etiqueta>…</etiqueta>` de un POM, recortada. Basta para lo que
/// se lee (descripción, url, la licencia): no es un lector de XML.
fn etiqueta(xml: &str, nombre: &str) -> Option<String> {
    let a = xml.find(&format!("<{nombre}>"))? + nombre.len() + 2;
    let b = xml[a..].find(&format!("</{nombre}>"))? + a;
    let t = xml[a..b].split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty() && !t.starts_with("${")).then_some(t)
}

/// El POM sin lo que hereda: `<parent>` dice la descripción del padre, no la suya.
fn sin_padre(xml: &str) -> String {
    match (xml.find("<parent>"), xml.find("</parent>")) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &xml[..a], &xml[b + 9..]),
        _ => xml.to_string(),
    }
}

/// Todas las `<etiqueta>` de un XML, en orden.
fn etiquetas(xml: &str, nombre: &str) -> Vec<String> {
    let (abre, cierra) = (format!("<{nombre}>"), format!("</{nombre}>"));
    let mut o = Vec::new();
    let mut resto = xml;
    while let Some(a) = resto.find(&abre) {
        let tras = &resto[a + abre.len()..];
        let Some(b) = tras.find(&cierra) else { break };
        o.push(tras[..b].trim().to_string());
        resto = &tras[b + cierra.len()..];
    }
    o
}

/// La ficha de Maven: las versiones de `maven-metadata.xml` (estático, en
/// `repo1.maven.org`: la búsqueda de versiones de `search.maven.org` agota el
/// plazo) y, si se pudo leer, el POM de la última.
pub fn ficha_maven(nombre: &str, metadatos: &str, pom: Option<&str>) -> Value {
    let mut vs = etiquetas(metadatos, "version");
    vs.sort_by(|a, b| comparar(b, a));
    vs.dedup();
    let ultima = etiqueta(metadatos, "release")
        .or_else(|| {
            vs.iter()
                .find(|v| !es_prerelease(Ecosistema::Jvm, v))
                .cloned()
        })
        .or_else(|| vs.first().cloned());
    let versiones = vs
        .into_iter()
        .filter(|v| !es_prerelease(Ecosistema::Jvm, v) || Some(v) == ultima.as_ref())
        .take(VERSIONES)
        .map(|v| (v, None))
        .collect();
    let pom = pom.map(sin_padre);
    let pom = pom.as_deref().unwrap_or("");
    let licencia = pom
        .find("<licenses>")
        .and_then(|i| etiqueta(&pom[i..], "name"));
    let repositorio = pom
        .find("<scm>")
        .and_then(|i| etiqueta(&pom[i..], "url"))
        .and_then(|u| repositorio_limpio(&u));
    let (g, a) = nombre.split_once(':').unwrap_or((nombre, ""));
    ficha(
        nombre.to_string(),
        etiqueta(pom, "description"),
        ultima,
        versiones,
        licencia,
        etiqueta(pom, "url"),
        repositorio,
        format!("https://central.sonatype.com/artifact/{g}/{a}"),
    )
}

pub fn busqueda_maven(v: &Value) -> Value {
    let r: Vec<Value> = v
        .pointer("/response/docs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    let mut m = Map::new();
                    m.insert("nombre".into(), json!(texto(d.get("id"))?));
                    if let Some(x) = texto(d.get("latestVersion")) {
                        m.insert("ultima".into(), json!(x));
                    }
                    Some(Value::Object(m))
                })
                .take(RESULTADOS)
                .collect()
        })
        .unwrap_or_default();
    json!({ "resultados": r })
}

// ── La red ──────────────────────────────────────────────────────────────────

fn agente() -> Result<ureq::Agent, Fallo> {
    let tls = native_tls::TlsConnector::new().map_err(|e| {
        fallo(
            502,
            format!("no se pudo abrir el TLS de la plataforma: {e}"),
        )
    })?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .timeout(Duration::from_secs(10))
        .user_agent("ore-packages (https://github.com/describeloai/ore)")
        .build())
}

/// Un GET a uno de [`HOSTS`], y a nada más; 404 es 404, lo demás del registro 502.
fn traer(a: &ureq::Agent, url: &str, acepta: &str) -> Result<String, Fallo> {
    let host = url
        .strip_prefix("https://")
        .and_then(|r| r.split('/').next())
        .unwrap_or("");
    if !HOSTS.contains(&host) {
        return Err(fallo(
            500,
            format!("`{host}` no es un registro de los que se consultan"),
        ));
    }
    match a.get(url).set("accept", acepta).call() {
        Ok(r) => {
            let mut s = String::new();
            r.into_reader()
                .take(TOPE_BYTES)
                .read_to_string(&mut s)
                .map_err(|e| fallo(502, format!("{host}: la respuesta no se pudo leer: {e}")))?;
            Ok(s)
        }
        Err(ureq::Error::Status(404, _)) => Err(fallo(404, "no existe en el registro")),
        Err(ureq::Error::Status(c, _)) => Err(fallo(502, format!("{host} contestó {c}"))),
        Err(e) => Err(fallo(502, format!("no se pudo hablar con {host}: {e}"))),
    }
}

fn json_de(a: &ureq::Agent, url: &str, acepta: &str) -> Result<Value, Fallo> {
    serde_json::from_str(&traer(a, url, acepta)?)
        .map_err(|_| fallo(502, "el registro no devolvió JSON"))
}

const JSON: &str = "application/json";

/// Lo pedido, contra su registro.
pub fn responder(p: &Peticion) -> Result<Value, Fallo> {
    let a = agente()?;
    match p {
        Peticion::Ficha(Ecosistema::Node, n) => {
            let base = format!("https://registry.npmjs.org/{}", ruta_npm(n));
            let latest = json_de(&a, &format!("{base}/latest"), JSON)?;
            let abreviado = json_de(
                &a,
                &base,
                "application/vnd.npm.install-v1+json; q=1.0, application/json; q=0.8",
            )?;
            Ok(ficha_npm(n, &latest, &abreviado))
        }
        Peticion::Buscar(Ecosistema::Node, q) => {
            let v = json_de(
                &a,
                &format!(
                    "https://registry.npmjs.org/-/v1/search?text={}&size={RESULTADOS}",
                    codificar(q)
                ),
                JSON,
            )?;
            Ok(busqueda_npm(&v))
        }
        Peticion::Ficha(Ecosistema::Python, n) => {
            let v = json_de(
                &a,
                &format!("https://pypi.org/pypi/{}/json", codificar(n)),
                JSON,
            )?;
            Ok(ficha_pypi(&v))
        }
        Peticion::Buscar(Ecosistema::Python, q) => {
            if !nombre_valido(Ecosistema::Python, q) {
                return Ok(json!({ "resultados": [] }));
            }
            match json_de(
                &a,
                &format!("https://pypi.org/pypi/{}/json", codificar(q)),
                JSON,
            ) {
                Ok(v) => Ok(busqueda_pypi(&ficha_pypi(&v))),
                Err(f) if f.codigo == 404 => Ok(json!({ "resultados": [] })),
                Err(f) => Err(f),
            }
        }
        Peticion::Ficha(Ecosistema::Jvm, n) => {
            let (g, art) = n.split_once(':').unwrap_or((n, ""));
            let base = format!(
                "https://repo1.maven.org/maven2/{}/{art}",
                g.replace('.', "/")
            );
            let metadatos = traer(&a, &format!("{base}/maven-metadata.xml"), "application/xml")?;
            let sin_pom = ficha_maven(n, &metadatos, None);
            // El POM de la última es para la descripción y la licencia: si no
            // llega, la ficha sale igual, sin ellas.
            let pom = texto(sin_pom.get("ultima")).and_then(|v| {
                traer(&a, &format!("{base}/{v}/{art}-{v}.pom"), "application/xml").ok()
            });
            Ok(ficha_maven(n, &metadatos, pom.as_deref()))
        }
        Peticion::Buscar(Ecosistema::Jvm, q) => {
            let v = json_de(
                &a,
                &format!(
                    "https://search.maven.org/solrsearch/select?q={}&rows={RESULTADOS}&wt=json",
                    codificar(q)
                ),
                JSON,
            )?;
            Ok(busqueda_maven(&v))
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_peticion_es_un_ecosistema_y_un_nombre_o_un_texto() {
        assert_eq!(
            peticion(r#"{"entorno":"node","nombre":"@types/node"}"#),
            Ok(Peticion::Ficha(Ecosistema::Node, "@types/node".into()))
        );
        assert_eq!(
            peticion(r#"{"entorno":"python","q":"polars"}"#),
            Ok(Peticion::Buscar(Ecosistema::Python, "polars".into()))
        );
        for mala in [
            r#"{"entorno":"ruby","nombre":"x"}"#,
            r#"{"entorno":"node"}"#,
            r#"{"entorno":"node","nombre":"a","q":"b"}"#,
            r#"{"entorno":"node","nombre":"https://evil/x"}"#,
            r#"{"entorno":"node","nombre":"Lodash"}"#,
            r#"{"entorno":"jvm","nombre":"sin-dos-puntos"}"#,
            r#"{"entorno":"python","nombre":"a/../b"}"#,
            "no es json",
        ] {
            assert_eq!(peticion(mala).map_err(|f| f.codigo), Err(422), "{mala}");
        }
    }

    #[test]
    fn el_nombre_va_codificado_a_la_url() {
        assert_eq!(ruta_npm("@types/node"), "@types%2Fnode");
        assert_eq!(ruta_npm("lodash"), "lodash");
        assert_eq!(codificar("date fns&x"), "date%20fns%26x");
    }

    #[test]
    fn las_versiones_se_comparan_como_numeros() {
        let mut v = vec!["1.9.3", "1.10.0", "1.10.0-rc.1", "0.9", "2.0.0"];
        v.sort_by(|a, b| comparar(b, a));
        assert_eq!(v, ["2.0.0", "1.10.0", "1.10.0-rc.1", "1.9.3", "0.9"]);
    }

    #[test]
    fn la_ficha_de_npm() {
        let latest = json!({"name":"lodash","version":"4.17.21","description":"Lodash modular utilities.",
            "license":"MIT","homepage":"https://lodash.com/","repository":{"type":"git","url":"git+https://github.com/lodash/lodash.git"}});
        let abreviado = json!({"dist-tags":{"latest":"4.17.21"},"versions":{"4.17.20":{},"4.17.21":{},"5.0.0-beta.1":{},"4.9.0":{}}});
        let f = ficha_npm("lodash", &latest, &abreviado);
        assert_eq!(f["ultima"], "4.17.21");
        assert_eq!(f["licencia"], "MIT");
        assert_eq!(f["repositorio"], "https://github.com/lodash/lodash");
        assert_eq!(f["registro"], "https://www.npmjs.com/package/lodash");
        let vs: Vec<&str> = f["versiones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["version"].as_str().unwrap())
            .collect();
        assert_eq!(
            vs,
            ["4.17.21", "4.17.20", "4.9.0"],
            "sin la beta, de mayor a menor"
        );
        let b = busqueda_npm(
            &json!({"objects":[{"package":{"name":"date-fns","version":"4.1.0","description":"Modern JavaScript date utility library"}}]}),
        );
        assert_eq!(b["resultados"][0]["nombre"], "date-fns");
        assert_eq!(b["resultados"][0]["ultima"], "4.1.0");
    }

    #[test]
    fn la_ficha_de_pypi() {
        let v = json!({"info":{"name":"polars","summary":"Blazingly fast DataFrame library","version":"1.40.0",
                "license":"MIT","home_page":null,"project_urls":{"Homepage":"https://www.pola.rs/","Repository":"https://github.com/pola-rs/polars"}},
            "releases":{"1.39.0":[{"upload_time_iso_8601":"2026-08-01T10:00:00Z"}],"1.40.0":[{"upload_time_iso_8601":"2026-09-01T10:00:00Z"}],
                "1.41.0b1":[{"upload_time_iso_8601":"2026-09-20T10:00:00Z"}],"0.1":[],"1.38.0":[{"upload_time_iso_8601":"2026-07-01T10:00:00Z","yanked":true}]}});
        let f = ficha_pypi(&v);
        assert_eq!(f["nombre"], "polars");
        assert_eq!(f["web"], "https://www.pola.rs/");
        assert_eq!(f["repositorio"], "https://github.com/pola-rs/polars");
        let vs: Vec<(&str, &str)> = f["versiones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (v["version"].as_str().unwrap(), v["fecha"].as_str().unwrap()))
            .collect();
        assert_eq!(
            vs,
            [("1.40.0", "2026-09-01"), ("1.39.0", "2026-08-01")],
            "sin la beta, la vacía ni la retirada"
        );
        assert_eq!(busqueda_pypi(&f)["resultados"][0]["ultima"], "1.40.0");
        assert!(
            !es_prerelease(Ecosistema::Python, "2.9.0.post0")
                && es_prerelease(Ecosistema::Python, "2.0.0rc1")
        );
        assert!(
            !es_prerelease(Ecosistema::Jvm, "33.1.0-jre")
                && es_prerelease(Ecosistema::Jvm, "6.0.0-M1")
        );
        // Una licencia que es el texto entero no se enseña como licencia.
        let larga = json!({"info":{"name":"x","license":"Copyright (c) 2026 Alguien con un texto muy largo de licencia que no cabe\nmás"}, "releases":{}});
        assert!(ficha_pypi(&larga).get("licencia").is_none());
        let clasif = json!({"info":{"name":"polars","license":"Copyright (c) 2025 Ritchie Vink",
            "classifiers":["License :: OSI Approved :: MIT License","Programming Language :: Python"]}, "releases":{}});
        assert_eq!(ficha_pypi(&clasif)["licencia"], "MIT License");
        let copy =
            json!({"info":{"name":"x","license":"Copyright (c) 2025 Alguien"}, "releases":{}});
        assert!(ficha_pypi(&copy).get("licencia").is_none());
    }

    #[test]
    fn la_ficha_de_maven() {
        let metadatos = "<metadata><groupId>com.google.guava</groupId><artifactId>guava</artifactId><versioning>\
            <latest>33.2.0-jre</latest><release>33.1.0-jre</release><versions><version>32.1.3-jre</version>\
            <version>33.0.0-jre</version><version>33.1.0-jre</version><version>34.0.0-SNAPSHOT</version></versions></versioning></metadata>";
        let pom = "<project><parent><description>la del padre</description></parent><description>Guava is a suite of core and expanded libraries.</description>\
            <url>https://github.com/google/guava</url><licenses><license><name>Apache License, Version 2.0</name></license></licenses>\
            <scm><url>https://github.com/google/guava</url></scm></project>";
        let f = ficha_maven("com.google.guava:guava", metadatos, Some(pom));
        assert_eq!(
            f["descripcion"],
            "Guava is a suite of core and expanded libraries."
        );
        assert_eq!(f["licencia"], "Apache License, Version 2.0");
        assert_eq!(
            f["registro"],
            "https://central.sonatype.com/artifact/com.google.guava/guava"
        );
        assert_eq!(f["ultima"], "33.1.0-jre", "la `release`, no la SNAPSHOT");
        let vs: Vec<&str> = f["versiones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["version"].as_str().unwrap())
            .collect();
        assert_eq!(vs, ["33.1.0-jre", "33.0.0-jre", "32.1.3-jre"]);
        let b = busqueda_maven(
            &json!({"response":{"docs":[{"id":"com.google.guava:guava","latestVersion":"33.1.0-jre"}]}}),
        );
        assert_eq!(b["resultados"][0]["nombre"], "com.google.guava:guava");
    }

    #[test]
    fn solo_se_habla_con_los_registros() {
        let a = agente().unwrap();
        assert_eq!(
            traer(&a, "https://evil.example/x", JSON).map_err(|f| f.codigo),
            Err(500)
        );
        assert_eq!(
            traer(&a, "http://registry.npmjs.org/x", JSON).map_err(|f| f.codigo),
            Err(500)
        );
    }
}
