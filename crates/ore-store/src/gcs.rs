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
//! En local no hay metadata server: se pasa el token en `ORE_GCS_TOKEN`
//! (`gcloud auth print-access-token`). Vale una hora, que es más de lo que dura
//! una materialización.
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
const METADATA: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

pub struct Cuenta {
    pub bucket: String,
    /// Fijo si vino por `ORE_GCS_TOKEN`; si no, se pide al metadata server en la
    /// primera petición y se guarda.
    token: std::cell::RefCell<Option<String>>,
}

impl Cuenta {
    pub fn del_entorno() -> Result<Cuenta, String> {
        let bucket = std::env::var("ORE_GCS_BUCKET")
            .map_err(|_| "falta la variable de entorno `ORE_GCS_BUCKET`".to_string())?;
        Ok(Cuenta {
            bucket,
            token: std::cell::RefCell::new(
                std::env::var("ORE_GCS_TOKEN")
                    .ok()
                    .filter(|t| !t.is_empty()),
            ),
        })
    }

    fn token(&self) -> Result<String, String> {
        if let Some(t) = self.token.borrow().as_ref() {
            return Ok(t.clone());
        }
        let cuerpo = cliente()?
            .get(METADATA)
            .set("metadata-flavor", "Google")
            .timeout(std::time::Duration::from_secs(5))
            .call()
            .map_err(|e| format!("sin `ORE_GCS_TOKEN` y el metadata server no contesta ({e}): en GCP hace falta Workload Identity; en local, `ORE_GCS_TOKEN=$(gcloud auth print-access-token)`"))?
            .into_string()
            .map_err(|e| format!("el token del metadata server no se pudo leer: {e}"))?;
        let t = ore_core::parse::parse(&cuerpo)
            .ok()
            .and_then(|n| {
                n.get("access_token")
                    .and_then(|(_, v)| v.as_str().map(String::from))
            })
            .ok_or("el metadata server no devolvió `access_token`")?;
        *self.token.borrow_mut() = Some(t.clone());
        Ok(t)
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

fn cliente() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new()
        .map_err(|e| format!("no se pudo abrir el TLS de la plataforma: {e}"))?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .build())
}

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
    let mut tabla = [0u32; 256];
    for (i, e) in tabla.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0x82F6_3B78 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *e = c;
    }
    let mut crc = !0u32;
    for &b in datos {
        crc = tabla[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
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
    fn leer(&self, clave: &str) -> Result<Option<String>, String> {
        Ok(self
            .leer_bytes(clave)?
            .map(|b| String::from_utf8_lossy(&b).trim().to_string()))
    }

    fn existe(&self, clave: &str) -> Result<bool, String> {
        match self.pide("GET", &self.objeto(clave))?.call() {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
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
