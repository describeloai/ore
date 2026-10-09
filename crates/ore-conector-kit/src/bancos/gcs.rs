//! **El banco de GCS** (ADR 0061 O2·3): un bucket `ore-kit` en
//! `fake-gcs-server` —en el CI y en el laboratorio—, con la misma semilla en
//! Parquet que el de S3 (`tipos/`, `vacia/`, `grande/`).
//!
//! `GCS_KIT_ENDPOINT` dice dónde. El emulador no comprueba el token, así que
//! no hay una URL mala que probar; y el conector sólo admite un `endpoint` que
//! no es de Google con `ORE_GCS_LABORATORIO=1`, que el CI pone. Es un GCS de
//! pruebas: **nunca el bucket de un cliente** —el kit escribe—.

use super::Banco;
use super::s3::{completar_fichero, sembrar};
use crate::semilla::Tabla;
use ore_core::json::Json;
use std::collections::BTreeMap;

const BUCKET: &str = "ore-kit";

pub struct Gcs {
    endpoint: String,
}

impl Gcs {
    pub fn new(endpoint: &str) -> Gcs {
        Gcs {
            endpoint: endpoint.trim_end_matches('/').to_string(),
        }
    }

    fn enviar(&self, url: &str, tipo: &str, cuerpo: &[u8]) -> Result<u16, String> {
        match ureq::post(url)
            .set("authorization", "Bearer kit")
            .set("content-type", tipo)
            .send_bytes(cuerpo)
        {
            Ok(x) => Ok(x.status()),
            Err(ureq::Error::Status(s, _)) => Ok(s),
            Err(e) => Err(format!("no se llega al GCS de pruebas: {e}")),
        }
    }
}

/// Un nombre de objeto en la consulta de la subida (`name=`).
fn codificar(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Banco for Gcs {
    fn familia(&self) -> &'static str {
        "gcs"
    }

    fn cargar(&mut self) -> Result<(), String> {
        // 200 si se crea; 409 si ya era nuestro.
        let crear = format!("{}/storage/v1/b?project=kit", self.endpoint);
        let cuerpo = format!(r#"{{"name":"{BUCKET}","versioning":{{"enabled":true}}}}"#);
        match self.enviar(&crear, "application/json", cuerpo.as_bytes())? {
            200 | 409 => {}
            s => return Err(format!("el bucket `{BUCKET}` no se crea: {s}")),
        }
        sembrar(|clave, datos| {
            let url = format!(
                "{}/upload/storage/v1/b/{BUCKET}/o?uploadType=media&name={}",
                self.endpoint,
                codificar(clave)
            );
            match self.enviar(&url, "application/octet-stream", datos)? {
                200 => Ok(()),
                s => Err(format!("`{clave}` no se sube: {s}")),
            }
        })
    }

    fn url(&self) -> String {
        format!("gs://{BUCKET}/?endpoint={}", self.endpoint)
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// El emulador no comprueba el token: no hay credencial mala que probar.
    fn url_mala(&self) -> Option<String> {
        None
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{}/", tabla.nombre())
    }

    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        completar_fichero(tabla, peticion)
    }
}
