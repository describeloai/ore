//! Google Cloud Storage por su **API JSON**, con el token de la cuenta que corre.
//!
//! # Sin clave estática, y no es una preferencia
//!
//! La política de la organización prohíbe crear claves de cuenta de servicio, y
//! las claves HMAC de la API S3 de GCS son una (`gcloud storage hmac create` →
//! 412 `iam.disableServiceAccountKeyCreation`, medido el 2026-09-16). Lo que un
//! proceso en GCP sí tiene es **un token de acceso de corta vida** que le da el
//! metadata server con la identidad del pod (Workload Identity) o de la máquina.
//! Es exactamente lo que los Jobs de la celda ya usan con `gcloud` para Secret
//! Manager; aquí se usa sin `gcloud` en medio.
//!
//! El token lo da `ore_gcp::Credencial`: el del metadata server, **renovado**
//! antes de caducar; en local, el de `ORE_GCP_TOKEN` (o `ORE_GCS_TOKEN`, el
//! nombre de antes), que no se renueva. Hasta A1 de BigQuery se guardaba aquí
//! el primero para siempre, y una copia de más de una hora fallaba a medias.
//!
//! # Las mismas dos garantías que R2 honraba
//!
//! - **no reescribir**: `ifGenerationMatch=0` en la subida es el `If-None-Match:
//!   *` de S3 — si el objeto ya existe, 412, y como el nombre es el contenido, ya
//!   estaba lo mismo;
//! - **integridad validada, no confiada**: GCS devuelve el `crc32c` de lo que
//!   guardó; se compara con el de lo que se mandó y, si no coinciden, el objeto
//!   se borra y la subida falla. (El `x-amz-checksum-sha256` de R2 lo validaba el
//!   servidor antes de escribir; aquí es después, y por eso se borra.)
//!
//! # Lo que este módulo no hace
//!
//! No resuelve el bucket: viene en `ORE_GCS_BUCKET`, uno por inquilino, y lo
//! pone el aprovisionador. No pagina más de lo que la recogida necesita. No
//! reintenta: un almacén que no contesta es un error que hay que ver.
use crate::almacen::Almacen;
use std::io::Read as _;

const API: &str = "https://storage.googleapis.com";
const AGENTE: &str = "ore-store-gcs/0.1";
/// El intercambio de tokens de Google: de un token de la cuenta a uno **acotado**
/// por *Credential Access Boundary* (0031 §11 ③, medido en
/// `medida-w3-escribir.py`: dentro del prefijo 200; fuera, borrar y sobrescribir
/// 403). No es una API de Cloud Storage: es STS, y vale con cualquier token
/// OAuth2 de la cuenta, el del metadata server incluido.
const STS: &str = "https://sts.googleapis.com/v1/token";

pub struct Cuenta {
    pub bucket: String,
    credencial: ore_gcp::Credencial,
}

impl Cuenta {
    pub fn del_entorno() -> Result<Cuenta, String> {
        let bucket = std::env::var("ORE_GCS_BUCKET")
            .map_err(|_| "falta la variable de entorno `ORE_GCS_BUCKET`".to_string())?;
        Ok(Cuenta {
            bucket,
            credencial: ore_gcp::Credencial::del_entorno(),
        })
    }

    fn token(&self) -> Result<String, String> {
        self.credencial.token()
    }

    fn pide(&self, metodo: &str, url: &str) -> Result<ureq::Request, String> {
        Ok(cliente()?
            .request(metodo, url)
            .set("user-agent", AGENTE)
            .set("authorization", &format!("Bearer {}", self.token()?)))
    }

    fn objeto(&self, clave: &str) -> String {
        format!("{API}/storage/v1/b/{}/o/{}", self.bucket, codificar(clave))
    }
}

use ore_gcp::cliente;

/// Percent-encoding de un nombre de objeto **como segmento**: la barra también,
/// porque en la API JSON el nombre entero es un solo segmento de la ruta.
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

/// CRC-32C (Castagnoli), que es el que GCS devuelve en `crc32c` (base64 del
/// valor en big-endian). Tabla de 256 entradas, como todos.
fn crc32c(datos: &[u8]) -> u32 {
    let mut c = crate::blobs::Crc32c::default();
    c.sumar(datos);
    c.valor()
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

fn campo(json: &str, k: &str) -> Option<String> {
    ore_core::parse::parse(json)
        .ok()
        .and_then(|n| n.get(k).and_then(|(_, v)| v.as_str().map(String::from)))
}

impl Almacen for Cuenta {
    fn base(&self) -> String {
        format!("gs://{}", self.bucket)
    }

    /// El token de esta cuenta, acotado a `prefijo` con `objectCreator` +
    /// `objectViewer`: escribe dentro, lee dentro, y no puede borrar,
    /// sobrescribir ni salir. Caduca cuando caduque el de la cuenta (STS lo
    /// dice; si no, se asume una hora menos un margen).
    fn prestar(&self, prefijo: &str) -> Result<crate::almacen::Prestamo, String> {
        self.acotar(prefijo, &["objectCreator", "objectViewer"])
    }

    /// Sólo `objectViewer` bajo el prefijo (②b): lee dentro y nada más.
    /// Medido en victor: STS acota en 50–60 ms; con ella se lee la tabla
    /// (200), otra tabla no (403), y no se lista nada (ni con prefijo).
    fn prestar_lectura(&self, prefijo: &str) -> Result<crate::almacen::Prestamo, String> {
        self.acotar(prefijo, &["objectViewer"])
    }
    fn leer(&self, clave: &str) -> Result<Option<String>, String> {
        Ok(self
            .leer_bytes(clave)?
            .map(|b| String::from_utf8_lossy(&b).trim().to_string()))
    }

    fn existe(&self, clave: &str) -> Result<bool, String> {
        Ok(self.tamano(clave)?.is_some())
    }

    /// Los metadatos del objeto (sin `alt=media`): traen `size` sin bajarlo.
    fn tamano(&self, clave: &str) -> Result<Option<u64>, String> {
        match self
            .pide("GET", &format!("{}?fields=size", self.objeto(clave)))?
            .call()
        {
            Ok(resp) => {
                let texto = resp
                    .into_string()
                    .map_err(|e| format!("los metadatos de `{clave}` no se pudieron leer: {e}"))?;
                Ok(Some(
                    campo(&texto, "size")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                ))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(format!("el `GET` de `{clave}` falla: {e}")),
        }
    }

    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
        let url = format!(
            "{API}/upload/storage/v1/b/{}/o?uploadType=media&name={}&ifGenerationMatch=0&fields=crc32c,name",
            self.bucket,
            codificar(clave)
        );
        let r = self
            .pide("POST", &url)?
            .set("content-type", "application/octet-stream")
            .set("content-length", &cuerpo.len().to_string());
        let resp = match r.send_bytes(cuerpo) {
            Ok(resp) => resp,
            // Ya estaba. Y como el nombre es el contenido, ya estaba lo mismo.
            Err(ureq::Error::Status(412, _)) => return Ok(false),
            Err(e) => return Err(format!("la subida de `{clave}` falla: {e}")),
        };
        let texto = resp
            .into_string()
            .map_err(|e| format!("la respuesta de la subida de `{clave}` no se pudo leer: {e}"))?;
        let dicho = campo(&texto, "crc32c").unwrap_or_default();
        let nuestro = base64(&crc32c(cuerpo).to_be_bytes());
        if dicho != nuestro {
            let _ = self.borrar(clave);
            return Err(format!(
                "GCS guardó otra cosa para `{clave}`: crc32c {dicho} y no {nuestro}; el objeto se ha borrado"
            ));
        }
        Ok(true)
    }

    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
        let mut claves = Vec::new();
        let mut pagina: Option<String> = None;
        loop {
            let mut url = format!(
                "{API}/storage/v1/b/{}/o?prefix={}&fields=items/name,nextPageToken",
                self.bucket,
                codificar(prefijo)
            );
            if let Some(p) = &pagina {
                url.push_str(&format!("&pageToken={}", codificar(p)));
            }
            let texto = self
                .pide("GET", &url)?
                .call()
                .map_err(|e| format!("no se pudo enumerar `{prefijo}`: {e}"))?
                .into_string()
                .map_err(|e| format!("la respuesta de la enumeración no se pudo leer: {e}"))?;
            let n = ore_core::parse::parse(&texto)
                .map_err(|e| format!("la enumeración de `{prefijo}` no analiza: {e:?}"))?;
            if let Some((_, items)) = n.get("items") {
                claves.extend(items.items().iter().filter_map(|o| {
                    o.get("name")
                        .and_then(|(_, v)| v.as_str().map(String::from))
                }));
            }
            pagina = n
                .get("nextPageToken")
                .and_then(|(_, v)| v.as_str().map(String::from))
                .filter(|t| !t.is_empty());
            if pagina.is_none() {
                return Ok(claves);
            }
        }
    }

    fn borrar(&self, clave: &str) -> Result<(), String> {
        match self.pide("DELETE", &self.objeto(clave))?.call() {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(404, _)) => Ok(()),
            Err(e) => Err(format!("no se pudo borrar `{clave}`: {e}")),
        }
    }

    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
        let url = format!("{}?alt=media", self.objeto(clave));
        match self.pide("GET", &url)?.call() {
            Ok(resp) => {
                let mut b = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut b)
                    .map_err(|e| format!("la copia `{clave}` no se pudo leer entera: {e}"))?;
                Ok(Some(b))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(format!("el `GET` de `{clave}` falla: {e}")),
        }
    }

    fn listar_con_fecha(&self, prefijo: &str) -> Result<Vec<crate::almacen::Listado>, String> {
        let mut out = Vec::new();
        let mut pagina: Option<String> = None;
        loop {
            let mut url = format!(
                "{API}/storage/v1/b/{}/o?prefix={}&maxResults=1000&fields=items(name,size,timeCreated,updated),nextPageToken",
                self.bucket,
                codificar(prefijo)
            );
            if let Some(p) = &pagina {
                url.push_str(&format!("&pageToken={}", codificar(p)));
            }
            let texto = self
                .pide_blob("GET", &url)?
                .call()
                .map_err(|e| format!("no se pudo enumerar `{prefijo}`: {e}"))?
                .into_string()
                .map_err(|e| format!("la enumeración de `{prefijo}` no se pudo leer: {e}"))?;
            let n = ore_core::parse::parse(&texto)
                .map_err(|e| format!("la enumeración de `{prefijo}` no analiza: {e:?}"))?;
            if let Some((_, items)) = n.get("items") {
                for o in items.items() {
                    let c = |k: &str| o.get(k).and_then(|(_, v)| v.as_str()).unwrap_or("");
                    let ms = |k: &str| crate::blobs::ms_de_iso(c(k)).unwrap_or(i64::MAX);
                    out.push(crate::almacen::Listado {
                        clave: c("name").to_string(),
                        tamano: c("size").parse().unwrap_or(0),
                        tocado_ms: ms("timeCreated").max(ms("updated").min(i64::MAX - 1)),
                    });
                }
            }
            pagina = n
                .get("nextPageToken")
                .and_then(|(_, v)| v.as_str().map(String::from))
                .filter(|t| !t.is_empty());
            if pagina.is_none() {
                return Ok(out);
            }
        }
    }

    /// Un `PATCH` de un metadato: cambia `updated`, no los bytes.
    fn tocar(&self, clave: &str) -> Result<bool, String> {
        let cuerpo = format!(
            "{{\"metadata\":{{\"ore-visto\":\"{}\"}}}}",
            crate::lago::ahora_ms()
        );
        match self
            .pide_blob("PATCH", &format!("{}?fields=updated", self.objeto(clave)))?
            .set("content-type", "application/json")
            .send_string(&cuerpo)
        {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(e) => Err(format!("no se pudo tocar `{clave}`: {e}")),
        }
    }

    fn poner_blob(&self, b: &crate::almacen::Blob) -> Result<bool, String> {
        match &b.cuerpo {
            crate::almacen::Cuerpo::Memoria(bytes) => self.blob_multiparte(b, bytes),
            crate::almacen::Cuerpo::Fichero(_) => self.blob_reanudable(b),
        }
    }

    fn leer_rango(
        &self,
        clave: &str,
        rango: Option<(u64, u64)>,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut r = self.pide_blob("GET", &format!("{}?alt=media", self.objeto(clave)))?;
        if let Some((a, z)) = rango {
            r = r.set("range", &format!("bytes={a}-{z}"));
        }
        match r.call() {
            Ok(resp) => {
                let mut b = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut b)
                    .map_err(|e| format!("`{clave}` no se pudo leer: {e}"))?;
                Ok(Some(b))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            // Un rango que empieza pasado el final: nada que leer.
            Err(ureq::Error::Status(416, _)) => Ok(Some(Vec::new())),
            Err(e) => Err(format!("el `GET` de `{clave}` falla: {e}")),
        }
    }
}

/// Los trozos de la subida reanudable: un múltiplo de 256 KiB, como pide GCS.
const TROZO: usize = 8 << 20;

/// **Un cliente para todos los blobs del proceso**, con sus conexiones vivas y
/// sin seguir redirecciones: el `308` de una subida reanudable a medias no es
/// una redirección. Medido en E8·2 B (clúster): 64 subidas en paralelo con
/// conexión viva, 589 blobs de 20 KiB por segundo.
fn cliente_de_blobs() -> Result<ureq::Agent, String> {
    static C: std::sync::OnceLock<Result<ureq::Agent, String>> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        let tls = native_tls::TlsConnector::new()
            .map_err(|e| format!("no se pudo abrir el TLS de la plataforma: {e}"))?;
        Ok(ureq::AgentBuilder::new()
            .tls_connector(std::sync::Arc::new(tls))
            .redirects(0)
            .max_idle_connections(256)
            .max_idle_connections_per_host(256)
            .timeout_connect(std::time::Duration::from_secs(30))
            .timeout_read(std::time::Duration::from_secs(120))
            .timeout_write(std::time::Duration::from_secs(120))
            .build())
    })
    .clone()
}

impl Cuenta {
    fn pide_blob(&self, metodo: &str, url: &str) -> Result<ureq::Request, String> {
        Ok(cliente_de_blobs()?
            .request(metodo, url)
            .set("user-agent", AGENTE)
            .set("authorization", &format!("Bearer {}", self.token()?)))
    }

    /// Los metadatos con los que GCS coteja: el nombre, el tipo y el crc32c
    /// que el servidor calcula y compara **antes** de crear el objeto.
    fn metadatos_de(b: &crate::almacen::Blob) -> String {
        ore_core::json::Json::obj([
            ("name", ore_core::json::Json::s(&b.clave)),
            ("contentType", ore_core::json::Json::s(&b.tipo)),
            (
                "crc32c",
                ore_core::json::Json::s(base64(&b.crc32c.to_be_bytes())),
            ),
        ])
        .jcs()
    }

    /// Lo pequeño: una petición multiparte (metadatos + bytes).
    fn blob_multiparte(&self, b: &crate::almacen::Blob, cuerpo: &[u8]) -> Result<bool, String> {
        // La frontera sale del contenido: no aparece dentro de él salvo que
        // alguien la busque, y tampoco podría (es su propio hash).
        let frontera = format!("ore-{}", &ore_s3::hex(&b.sha256)[..40]);
        let mut todo = Vec::with_capacity(cuerpo.len() + 512);
        todo.extend_from_slice(
            format!(
                "--{frontera}\r\ncontent-type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{frontera}\r\ncontent-type: {}\r\n\r\n",
                Cuenta::metadatos_de(b),
                b.tipo
            )
            .as_bytes(),
        );
        todo.extend_from_slice(cuerpo);
        todo.extend_from_slice(format!("\r\n--{frontera}--").as_bytes());
        let url = format!(
            "{API}/upload/storage/v1/b/{}/o?uploadType=multipart&ifGenerationMatch=0&fields=crc32c,name",
            self.bucket
        );
        let r = self
            .pide_blob("POST", &url)?
            .set(
                "content-type",
                &format!("multipart/related; boundary={frontera}"),
            )
            .send_bytes(&todo);
        self.respuesta_de_blob(b, r)
    }

    /// Lo grande: una subida reanudable, en trozos, desde el temporal.
    fn blob_reanudable(&self, b: &crate::almacen::Blob) -> Result<bool, String> {
        let url = format!(
            "{API}/upload/storage/v1/b/{}/o?uploadType=resumable&ifGenerationMatch=0&fields=crc32c,name",
            self.bucket
        );
        let inicio = self
            .pide_blob("POST", &url)?
            .set("content-type", "application/json; charset=UTF-8")
            .set("x-upload-content-type", &b.tipo)
            .set("x-upload-content-length", &b.tamano.to_string())
            .send_string(&Cuenta::metadatos_de(b));
        let sesion = match inicio {
            Ok(r) => r
                .header("location")
                .map(String::from)
                .ok_or("GCS no devolvió la sesión de la subida reanudable")?,
            Err(ureq::Error::Status(412, _)) => return Ok(false),
            Err(ureq::Error::Status(c, r)) => {
                return Err(format!(
                    "la subida de `{}` no empezó ({c}): {}",
                    b.clave,
                    r.into_string().unwrap_or_default()
                ));
            }
            Err(e) => return Err(format!("la subida de `{}` no empezó: {e}", b.clave)),
        };
        let mut lector = b.cuerpo.lector()?;
        let mut trozo = vec![0u8; TROZO];
        let mut hecho = 0u64;
        loop {
            let mut n = 0;
            while n < TROZO {
                let k = lector
                    .read(&mut trozo[n..])
                    .map_err(|e| format!("el temporal de `{}` no se pudo leer: {e}", b.clave))?;
                if k == 0 {
                    break;
                }
                n += k;
            }
            let fin = hecho + n as u64;
            let rango = if b.tamano == 0 {
                "bytes */0".to_string()
            } else {
                format!("bytes {hecho}-{}/{}", fin.saturating_sub(1), b.tamano)
            };
            let r = self
                .pide_blob("PUT", &sesion)?
                .set("content-range", &rango)
                .send_bytes(&trozo[..n]);
            hecho = fin;
            match r {
                // 308: el trozo llegó, faltan más. (ureq da un 3xx como
                // respuesta, no como error, sin redirecciones.)
                Ok(x) if x.status() == 308 && hecho < b.tamano => continue,
                Ok(x) if x.status() == 308 => {
                    return Err(format!(
                        "GCS no cerró la subida de `{}`: pidió más bytes de los que hay",
                        b.clave
                    ));
                }
                otro => return self.respuesta_de_blob(b, otro),
            }
        }
    }

    fn respuesta_de_blob(
        &self,
        b: &crate::almacen::Blob,
        r: Result<ureq::Response, ureq::Error>,
    ) -> Result<bool, String> {
        match r {
            Ok(resp) => {
                let texto = resp.into_string().unwrap_or_default();
                // La reanudable no lo devuelve en el último trozo (medido): se
                // pregunta a los metadatos.
                let dicho = match campo(&texto, "crc32c") {
                    Some(c) if !c.is_empty() => c,
                    _ => self
                        .pide_blob("GET", &format!("{}?fields=crc32c", self.objeto(&b.clave)))?
                        .call()
                        .ok()
                        .and_then(|r| r.into_string().ok())
                        .and_then(|t| campo(&t, "crc32c"))
                        .unwrap_or_default(),
                };
                let nuestro = base64(&b.crc32c.to_be_bytes());
                if dicho != nuestro {
                    // No debería poder pasar: el servidor ya lo cotejó.
                    let _ = self.borrar(&b.clave);
                    return Err(format!(
                        "GCS guardó otra cosa para `{}`: crc32c {dicho} y no {nuestro}; el objeto se ha borrado",
                        b.clave
                    ));
                }
                Ok(true)
            }
            Err(ureq::Error::Status(412, _)) => Ok(false),
            Err(ureq::Error::Status(c, r)) => Err(format!(
                "GCS rechazó `{}` ({c}): {}",
                b.clave,
                r.into_string()
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect::<String>()
            )),
            Err(e) => Err(format!("la subida de `{}` falla: {e}", b.clave)),
        }
    }

    /// El token de esta cuenta, acotado a `prefijo` con esos roles.
    fn acotar(&self, prefijo: &str, roles: &[&str]) -> Result<crate::almacen::Prestamo, String> {
        let permisos = roles
            .iter()
            .map(|r| format!("\"inRole:roles/storage.{r}\""))
            .collect::<Vec<_>>()
            .join(",");
        let regla = format!(
            "{{\"accessBoundary\":{{\"accessBoundaryRules\":[{{\"availableResource\":\"//storage.googleapis.com/projects/_/buckets/{b}\",\"availablePermissions\":[{permisos}],\"availabilityCondition\":{{\"expression\":\"resource.name.startsWith('projects/_/buckets/{b}/objects/{p}')\"}}}}]}}}}",
            b = self.bucket,
            p = prefijo
        );
        let cuerpo = format!(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange&subject_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token&requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token&subject_token={}&options={}",
            codificar(&self.token()?),
            codificar(&regla)
        );
        let r = cliente()?
            .post(STS)
            .set("content-type", "application/x-www-form-urlencoded")
            .set("user-agent", AGENTE)
            .timeout(std::time::Duration::from_secs(20))
            .send_string(&cuerpo);
        let texto = match r {
            Ok(r) => r
                .into_string()
                .map_err(|e| format!("STS contestó algo ilegible: {e}"))?,
            Err(ureq::Error::Status(c, r)) => {
                let t = r.into_string().unwrap_or_default();
                return Err(format!(
                    "STS no acotó el token ({c}): {}",
                    t.chars().take(200).collect::<String>()
                ));
            }
            Err(e) => return Err(format!("STS no contesta: {e}")),
        };
        let n = ore_core::parse::parse(&texto).map_err(|_| "STS no devolvió JSON".to_string())?;
        let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let token = campo("access_token").ok_or("STS no devolvió `access_token`")?;
        let segundos: i64 = campo("expires_in")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600 - 300);
        let caduca = crate::lago::ahora_ms() + segundos * 1000;
        Ok(crate::almacen::Prestamo {
            config: [
                ("gcs.oauth2.token".to_string(), token),
                (
                    "gcs.oauth2.token-expires-at".to_string(),
                    caduca.to_string(),
                ),
            ]
            .into(),
            caduca_ms: Some(caduca),
            acotada: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_crc32c_es_el_de_castagnoli() {
        // el vector de la RFC 3720 §B.4: 32 bytes a cero → 0x8a9136aa
        assert_eq!(crc32c(&[0u8; 32]), 0x8a91_36aa);
        assert_eq!(crc32c(b""), 0);
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn el_nombre_del_objeto_va_como_un_segmento() {
        assert_eq!(codificar("ore/v1/plan/abc"), "ore%2Fv1%2Fplan%2Fabc");
        assert_eq!(codificar("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn el_crc_que_gcs_devuelve_es_base64_del_big_endian() {
        assert_eq!(base64(&crc32c(b"123456789").to_be_bytes()), "4waSgw==");
    }
}
