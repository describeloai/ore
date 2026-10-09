//! **Un bucket de GCS de un cliente, como origen** (ADR 0061 O2): la API JSON
//! de Cloud Storage, con la identidad que el cliente autorizó (D-O2).
//!
//! ```text
//! gs://<bucket>[/<prefijo>]                              la cuenta de la celda
//! gs://<bucket>[/<prefijo>]?suplantar=<cuenta del cliente>  una suya, por una hora
//! gs://<bucket>[/<prefijo>]?endpoint=http://…            otro servidor (el laboratorio)
//! ```
//!
//! La URL **no lleva secreto**: el token es el de la cuenta que corre (el
//! metadata server, `ore-gcp`) o el que esa cuenta obtiene suplantando a la del
//! cliente (`generateAccessToken`, que el cliente permite dándole
//! `roles/iam.serviceAccountTokenCreator` sobre la suya). Cada objeto se fija
//! por su **generación**: siempre existe, y leer `?generation=N` da esa o un
//! `404`, nunca otra.
//!
//! # Medido en `fake-gcs-server` (O2·0)
//!
//! Versiones (`versions=true`), generaciones viejas, rangos, `crc32c` y
//! `x-goog-hash`, sí; la paginación, `ifGenerationMatch`, `testIamPermissions`
//! y el token, no: la paginación se prueba aquí sin red ([`paginar`]) y lo demás
//! contra GCS de verdad.

pub mod firma;
mod origen;

use ore_objetos::Leido;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const API: &str = "https://storage.googleapis.com";
const AGENTE: &str = "ore-gcs/0.1";
const IAM: &str = "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts";
const CORREO: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/email";
/// Lo que pide una suplantación: leer, y nada más.
const ALCANCE: &str = "https://www.googleapis.com/auth/devstorage.read_only";

static PETICIONES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Cuántas peticiones a GCS y cuántos bytes de listados y metadatos, para los
/// avisos del driver (los de los objetos van en flujo y no se cuentan).
pub fn contadores() -> (usize, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (PETICIONES.load(Relaxed), BYTES.load(Relaxed))
}

fn contar(texto: &str) {
    BYTES.fetch_add(texto.len(), std::sync::atomic::Ordering::Relaxed);
}

/// La coordenada de una fuente de GCS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fuente {
    pub bucket: String,
    /// Vacío o acabado en `/`.
    pub prefijo: String,
    /// `https://storage.googleapis.com`, u otro (el laboratorio).
    pub endpoint: String,
    /// La cuenta del cliente que se suplanta, si es por suplantación.
    pub suplantar: Option<String>,
}

/// `gs://bucket/prefijo?…` → la coordenada. Sin secreto que leer.
pub fn leer(url: &str) -> Result<Fuente, String> {
    let resto = url
        .trim()
        .strip_prefix("gs://")
        .ok_or("la URL de un bucket de GCS es `gs://<bucket>[/<prefijo>]`")?;
    let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let (bucket, prefijo) = camino.split_once('/').unwrap_or((camino, ""));
    if bucket.is_empty()
        || !bucket
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
    {
        return Err(format!("`{bucket}` no es el nombre de un bucket de GCS"));
    }
    let mut endpoint = API.to_string();
    let mut suplantar = None;
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        let v = descodificar(v);
        match k {
            "endpoint" => endpoint = v.trim_end_matches('/').to_string(),
            "suplantar" => {
                if !v.ends_with(".gserviceaccount.com") || !v.contains('@') {
                    return Err(format!(
                        "`suplantar={v}` no es una cuenta de servicio (`<nombre>@<proyecto>.iam.gserviceaccount.com`)"
                    ));
                }
                suplantar = Some(v);
            }
            otro => return Err(format!("`{otro}` no es un parámetro de una URL `gs://`")),
        }
    }
    let prefijo = descodificar(prefijo);
    let prefijo = prefijo.trim_start_matches('/');
    let prefijo = if prefijo.is_empty() || prefijo.ends_with('/') {
        prefijo.to_string()
    } else {
        format!("{prefijo}/")
    };
    Ok(Fuente {
        bucket: bucket.to_string(),
        prefijo,
        endpoint,
        suplantar,
    })
}

/// La URL de un prefijo, que se puede enseñar: una `gs://` no lleva secreto.
pub fn publica(f: &Fuente, prefijo: &str) -> String {
    let mut u = format!("gs://{}/{prefijo}", f.bucket);
    let mut sep = '?';
    if f.endpoint != API {
        u.push_str(&format!("{sep}endpoint={}", f.endpoint));
        sep = '&';
    }
    if let Some(s) = &f.suplantar {
        u.push_str(&format!("{sep}suplantar={s}"));
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

/// Un nombre de objeto como segmento de la ruta de la API JSON: la barra
/// también se codifica.
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

/// **Lo que la API JSON dice de un objeto** (o de una generación suya).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    pub nombre: String,
    pub tamano: u64,
    pub generacion: String,
    /// Base64 del CRC-32C en big-endian: siempre lo da.
    pub crc32c: Option<String>,
    pub md5: Option<String>,
    pub tipo: Option<String>,
    pub actualizado: String,
    /// Una generación que ya no es la vigente (`timeDeleted`).
    pub borrada: bool,
}

impl Meta {
    fn de(n: &ore_core::parse::Node) -> Meta {
        let c = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        Meta {
            nombre: c("name").unwrap_or_default(),
            tamano: c("size").and_then(|s| s.parse().ok()).unwrap_or(0),
            generacion: c("generation").unwrap_or_default(),
            crc32c: c("crc32c"),
            md5: c("md5Hash"),
            tipo: c("contentType"),
            actualizado: c("updated").unwrap_or_default(),
            borrada: n.get("timeDeleted").is_some(),
        }
    }
}

/// **Un listado entero, página a página** (`pageToken`): `pagina(token)` pide
/// una y devuelve su JSON. Fuera de la red para probarlo, porque el emulador
/// no pagina (O2·0). Devuelve los objetos y los prefijos (con `delimiter`).
pub fn paginar(
    pagina: impl FnMut(Option<&str>) -> Result<String, String>,
) -> Result<(Vec<Meta>, Vec<String>), String> {
    paginar_hasta(pagina, usize::MAX)
}

/// Como [`paginar`], hasta `paginas` páginas.
fn paginar_hasta(
    mut pagina: impl FnMut(Option<&str>) -> Result<String, String>,
    paginas: usize,
) -> Result<(Vec<Meta>, Vec<String>), String> {
    let mut metas = Vec::new();
    let mut prefijos = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..paginas {
        let texto = pagina(token.as_deref())?;
        let n = ore_core::parse::parse(&texto)
            .map_err(|e| format!("el listado de GCS no analiza: {e:?}"))?;
        if let Some((_, items)) = n.get("items") {
            metas.extend(items.items().iter().map(Meta::de));
        }
        if let Some((_, ps)) = n.get("prefixes") {
            prefijos.extend(
                ps.items()
                    .iter()
                    .filter_map(|p| p.as_str().map(String::from)),
            );
        }
        token = n
            .get("nextPageToken")
            .and_then(|(_, v)| v.as_str().map(String::from))
            .filter(|t| !t.is_empty());
        if token.is_none() {
            break;
        }
    }
    Ok((metas, prefijos))
}

/// El token con el que se lee: el de la cuenta que corre, o el que esa cuenta
/// obtiene suplantando a la del cliente (una hora, renovado antes de caducar).
enum Token {
    Propio(ore_gcp::Credencial),
    Suplantado {
        base: ore_gcp::Credencial,
        cuenta: String,
        vigente: Mutex<Option<(String, Instant)>>,
    },
}

/// **Un bucket de GCS, listo para leer**: su coordenada, el cliente HTTP (uno
/// para toda la vida: con conexiones vivas) y el token.
pub struct Gcs {
    pub fuente: Fuente,
    agente: ureq::Agent,
    token: Token,
}

/// Lo que GCS contestó cuando no fue un 2xx: el estado y el cuerpo (`0`: no
/// contestó).
#[derive(Debug, Clone)]
pub struct Fallo {
    pub estado: u16,
    pub cuerpo: String,
}

impl Fallo {
    pub fn motivo(&self) -> String {
        if self.estado == 0 {
            return self.cuerpo.clone();
        }
        let n = ore_core::parse::parse(&self.cuerpo).ok();
        let m = n
            .as_ref()
            .and_then(|n| n.get("error"))
            .and_then(|(_, e)| e.get("message"))
            .and_then(|(_, m)| m.as_str().map(String::from))
            .unwrap_or_else(|| self.cuerpo.trim().chars().take(200).collect());
        format!("{} {m}", self.estado)
    }
}

impl Gcs {
    /// El de una coordenada, con la credencial del entorno (`ore-gcp`: el
    /// metadata server, o `ORE_GCP_TOKEN` en local).
    pub fn de(fuente: Fuente) -> Result<Gcs, String> {
        let base = ore_gcp::Credencial::del_entorno();
        let token = match &fuente.suplantar {
            None => Token::Propio(base),
            Some(c) => Token::Suplantado {
                base,
                cuenta: c.clone(),
                vigente: Mutex::new(None),
            },
        };
        Ok(Gcs {
            fuente,
            agente: ore_gcp::cliente()?,
            token,
        })
    }

    /// El de una URL `gs://…`.
    pub fn de_url(url: &str) -> Result<Gcs, String> {
        Gcs::de(leer(url)?)
    }

    /// **¿Hay identidad con la que leer?** El token de la cuenta que corre, y
    /// con `suplantar`, el de la del cliente: si el cliente no dio a la de la
    /// celda `roles/iam.serviceAccountTokenCreator` sobre la suya, aquí se sabe.
    pub fn identidad(&self) -> Result<(), String> {
        self.token().map(|_| ())
    }

    fn token(&self) -> Result<String, String> {
        match &self.token {
            Token::Propio(c) => c.token(),
            Token::Suplantado {
                base,
                cuenta,
                vigente,
            } => {
                let mut v = vigente.lock().unwrap_or_else(|e| e.into_inner());
                if let Some((t, caduca)) = v.as_ref()
                    && Instant::now() + Duration::from_secs(300) < *caduca
                {
                    return Ok(t.clone());
                }
                let cuerpo = format!("{{\"scope\":[\"{ALCANCE}\"],\"lifetime\":\"3600s\"}}");
                let texto = self
                    .agente
                    .post(&format!("{IAM}/{cuenta}:generateAccessToken"))
                    .set("user-agent", AGENTE)
                    .set("authorization", &format!("Bearer {}", base.token()?))
                    .set("content-type", "application/json")
                    .send_string(&cuerpo)
                    .map_err(|e| match e {
                        ureq::Error::Status(403, _) => format!(
                            "no se pudo suplantar a `{cuenta}` (403): el cliente tiene que dar a la \
                             cuenta de esta celda `roles/iam.serviceAccountTokenCreator` sobre la suya"
                        ),
                        e => format!("no se pudo suplantar a `{cuenta}`: {e}"),
                    })?
                    .into_string()
                    .map_err(|e| format!("la suplantación no se pudo leer: {e}"))?;
                let t = campo(&texto, "accessToken")
                    .ok_or("la suplantación no devolvió `accessToken`")?;
                *v = Some((t.clone(), Instant::now() + Duration::from_secs(3600)));
                Ok(t)
            }
        }
    }

    fn pide(&self, metodo: &str, url: &str) -> Result<ureq::Request, Fallo> {
        let t = self.token().map_err(|e| Fallo {
            estado: 0,
            cuerpo: e,
        })?;
        PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(self
            .agente
            .request(metodo, url)
            .set("user-agent", AGENTE)
            .set("authorization", &format!("Bearer {t}")))
    }

    fn llamar(r: ureq::Request) -> Result<ureq::Response, Fallo> {
        match r.call() {
            Ok(x) => Ok(x),
            Err(ureq::Error::Status(c, x)) => Err(Fallo {
                estado: c,
                cuerpo: x.into_string().unwrap_or_default(),
            }),
            Err(e) => Err(Fallo {
                estado: 0,
                cuerpo: format!("GCS no contesta: {e}"),
            }),
        }
    }

    fn objeto(&self, clave: &str) -> String {
        format!(
            "{}/storage/v1/b/{}/o/{}",
            self.fuente.endpoint,
            self.fuente.bucket,
            codificar(clave)
        )
    }

    /// **El listado bajo un prefijo**, entero: con `versiones`, cada generación
    /// (`versions=true`); con `delimitador`, un nivel y sus prefijos.
    pub fn listar(
        &self,
        prefijo: &str,
        versiones: bool,
        delimitador: Option<&str>,
    ) -> Result<(Vec<Meta>, Vec<String>), Fallo> {
        self.listado(prefijo, versiones, delimitador, None)
    }

    /// **Una página** del listado, de hasta `max` (`maxResults`): lo que mira
    /// `check` y lo que enseña `explorar`, sin recorrer un bucket entero.
    pub fn pagina(
        &self,
        prefijo: &str,
        delimitador: Option<&str>,
        max: u32,
    ) -> Result<(Vec<Meta>, Vec<String>), Fallo> {
        self.listado(prefijo, false, delimitador, Some(max))
    }

    fn listado(
        &self,
        prefijo: &str,
        versiones: bool,
        delimitador: Option<&str>,
        max: Option<u32>,
    ) -> Result<(Vec<Meta>, Vec<String>), Fallo> {
        let base = format!(
            "{}/storage/v1/b/{}/o?prefix={}{}{}{}",
            self.fuente.endpoint,
            self.fuente.bucket,
            codificar(prefijo),
            if versiones { "&versions=true" } else { "" },
            delimitador
                .map(|d| format!("&delimiter={}", codificar(d)))
                .unwrap_or_default(),
            max.map(|m| format!("&maxResults={m}")).unwrap_or_default()
        );
        let mut fallo = None;
        let r = paginar_hasta(
            |token| {
                let url = match token {
                    Some(t) => format!("{base}&pageToken={}", codificar(t)),
                    None => base.clone(),
                };
                let x = self.pide("GET", &url).and_then(Gcs::llamar).map_err(|f| {
                    let m = f.motivo();
                    fallo = Some(f);
                    m
                })?;
                let t = x
                    .into_string()
                    .map_err(|e| format!("el listado no se pudo leer: {e}"))?;
                contar(&t);
                Ok(t)
            },
            if max.is_some() { 1 } else { usize::MAX },
        );
        r.map_err(|m| {
            fallo.unwrap_or(Fallo {
                estado: 0,
                cuerpo: m,
            })
        })
    }

    /// Los metadatos de un objeto, o de una generación suya.
    pub fn meta(&self, clave: &str, generacion: Option<&str>) -> Result<Meta, Fallo> {
        let mut url = self.objeto(clave);
        if let Some(g) = generacion {
            url.push_str(&format!("?generation={g}"));
        }
        let texto = Gcs::llamar(self.pide("GET", &url)?)?
            .into_string()
            .map_err(|e| Fallo {
                estado: 0,
                cuerpo: format!("los metadatos no se pudieron leer: {e}"),
            })?;
        contar(&texto);
        let n = ore_core::parse::parse(&texto).map_err(|e| Fallo {
            estado: 0,
            cuerpo: format!("los metadatos no analizan: {e:?}"),
        })?;
        Ok(Meta::de(&n))
    }

    /// **Los bytes de un objeto**, de su generación si se dice (`?generation=N`:
    /// esa o un `404`) y del rango si se pide (`bytes=…`), en flujo.
    pub fn bajar(
        &self,
        clave: &str,
        generacion: Option<&str>,
        rango: Option<&str>,
    ) -> Result<Leido, Fallo> {
        let mut url = format!("{}?alt=media", self.objeto(clave));
        if let Some(g) = generacion.filter(|g| !g.is_empty()) {
            url.push_str(&format!("&generation={g}"));
        }
        let mut r = self.pide("GET", &url)?;
        if let Some(x) = rango {
            r = r.set("range", x);
        }
        let x = Gcs::llamar(r)?;
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

    /// **`testIamPermissions`**: de los permisos pedidos, los que la identidad
    /// tiene sobre el bucket. `None` si el servidor no lo sabe (el emulador).
    pub fn permisos(&self, pedidos: &[&str]) -> Result<Option<Vec<String>>, Fallo> {
        let q = pedidos
            .iter()
            .map(|p| format!("permissions={}", codificar(p)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!(
            "{}/storage/v1/b/{}/iam/testPermissions?{q}",
            self.fuente.endpoint, self.fuente.bucket
        );
        match self.pide("GET", &url).and_then(Gcs::llamar) {
            Ok(x) => {
                let t = x.into_string().unwrap_or_default();
                let n = ore_core::parse::parse(&t).map_err(|e| Fallo {
                    estado: 0,
                    cuerpo: format!("`testIamPermissions` no analiza: {e:?}"),
                })?;
                Ok(Some(
                    n.get("permissions")
                        .map(|(_, v)| {
                            v.items()
                                .iter()
                                .filter_map(|p| p.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                ))
            }
            Err(f) if f.estado == 404 || f.estado == 501 => Ok(None),
            Err(f) => Err(f),
        }
    }

    /// El correo de quien firma: la cuenta suplantada, o la que corre (el
    /// metadata server; `ORE_GCS_FIRMANTE` fuera de GCP).
    fn firmante(&self) -> Result<String, String> {
        if let Token::Suplantado { cuenta, .. } = &self.token {
            return Ok(cuenta.clone());
        }
        if let Ok(f) = std::env::var("ORE_GCS_FIRMANTE") {
            return Ok(f);
        }
        self.agente
            .get(CORREO)
            .set("metadata-flavor", "Google")
            .call()
            .map_err(|e| format!("el metadata server no dice qué cuenta corre ({e})"))?
            .into_string()
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("el correo de la cuenta no se pudo leer: {e}"))
    }

    /// **Una URL V4 de un objeto**, fijada a su generación, firmada por IAM
    /// (`signBlob`, como la cuenta que lee): no hay clave en ningún sitio.
    pub fn firmar(
        &self,
        clave: &str,
        generacion: &str,
        tipo: &str,
        disposicion: &str,
        segundos: u64,
    ) -> Result<String, String> {
        let firmante = self.firmante()?;
        let (marca, fecha) = ore_sigv4::firma::ahora();
        let mut extra: Vec<(&str, &str)> = vec![
            ("response-content-type", tipo),
            ("response-content-disposition", disposicion),
        ];
        if !generacion.is_empty() {
            extra.push(("generation", generacion));
        }
        let host = self
            .fuente
            .endpoint
            .split_once("://")
            .map_or(self.fuente.endpoint.as_str(), |(_, h)| h);
        let p = firma::Pedida {
            host,
            bucket: &self.fuente.bucket,
            clave,
            firmante: &firmante,
            marca: &marca,
            fecha: &fecha,
            segundos: segundos.clamp(1, 604_800),
            extra: &extra,
        };
        let (ruta, consulta, can) = firma::canonica(&p);
        let cuerpo = format!(
            "{{\"payload\":\"{}\"}}",
            ore_gcp::base64(firma::por_firmar(&p, &can).as_bytes())
        );
        let texto = self
            .agente
            .post(&format!("{IAM}/{firmante}:signBlob"))
            .set("user-agent", AGENTE)
            .set("authorization", &format!("Bearer {}", self.token()?))
            .set("content-type", "application/json")
            .send_string(&cuerpo)
            .map_err(|e| match e {
                ureq::Error::Status(403, _) => format!(
                    "`signBlob` 403: `{firmante}` no puede firmar como sí misma (le falta \
                     `roles/iam.serviceAccountTokenCreator` sobre sí)"
                ),
                e => format!("`signBlob` falla: {e}"),
            })?
            .into_string()
            .map_err(|e| format!("`signBlob` no se pudo leer: {e}"))?;
        let firma = ore_gcp::de_base64(
            &campo(&texto, "signedBlob").ok_or("`signBlob` no devolvió `signedBlob`")?,
        )?;
        Ok(format!(
            "{}{ruta}?{consulta}&X-Goog-Signature={}",
            self.fuente.endpoint,
            ore_sigv4::firma::hex(&firma)
        ))
    }
}

fn campo(json: &str, k: &str) -> Option<String> {
    ore_core::parse::parse(json)
        .ok()
        .and_then(|n| n.get(k).and_then(|(_, v)| v.as_str().map(String::from)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_url_dice_bucket_prefijo_y_como_se_lee() {
        let f = leer("gs://mi-cubo/Nueva%20carpeta/docs").unwrap();
        assert_eq!(
            (
                f.bucket.as_str(),
                f.prefijo.as_str(),
                f.endpoint.as_str(),
                f.suplantar
            ),
            ("mi-cubo", "Nueva carpeta/docs/", API, None)
        );
        let f = leer("gs://c/?suplantar=lector%40cliente.iam.gserviceaccount.com&endpoint=http://o2-gcs:4443/")
            .unwrap();
        assert_eq!(
            f.suplantar.as_deref(),
            Some("lector@cliente.iam.gserviceaccount.com")
        );
        assert_eq!(f.endpoint, "http://o2-gcs:4443");
        assert_eq!(
            publica(&f, "docs/"),
            "gs://c/docs/?endpoint=http://o2-gcs:4443&suplantar=lector@cliente.iam.gserviceaccount.com"
        );
        for mala in [
            "s3://c/",
            "gs:///x",
            "gs://Mayus/",
            "gs://c/?suplantar=yo@gmail.com",
            "gs://c/?access_key_id=x",
        ] {
            assert!(leer(mala).is_err(), "{mala}");
        }
    }

    /// La paginación: tres páginas por `pageToken`, con los prefijos. El
    /// emulador no pagina (O2·0): esto es lo que lo cubre.
    #[test]
    fn el_listado_sigue_las_paginas() {
        let paginas = [
            r#"{"items":[{"name":"a","size":"3","generation":"7","crc32c":"AAAAAA=="}],"prefixes":["d/"],"nextPageToken":"t1"}"#,
            r#"{"items":[{"name":"b","generation":"8","timeDeleted":"2026-10-09T00:00:00Z"}],"nextPageToken":"t2"}"#,
            r#"{"items":[{"name":"c","generation":"9"}]}"#,
        ];
        let mut pedidos = Vec::new();
        let (metas, prefijos) = paginar(|t| {
            pedidos.push(t.map(String::from));
            Ok(paginas[pedidos.len() - 1].to_string())
        })
        .unwrap();
        assert_eq!(pedidos, [None, Some("t1".into()), Some("t2".into())]);
        assert_eq!(
            metas.iter().map(|m| m.nombre.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(
            (metas[0].tamano, metas[0].crc32c.as_deref()),
            (3, Some("AAAAAA=="))
        );
        assert!(metas[1].borrada && !metas[0].borrada);
        assert_eq!(prefijos, ["d/"]);
    }
}
