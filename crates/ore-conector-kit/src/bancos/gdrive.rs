//! **El banco de Google Drive** (ADR 0082 paso 5): la carpeta compartida `RAIZ` del
//! Drive de mentira (`pruebas-de-fuego/drive-de-mentira.py`) —no hay emulador de
//! Drive—, bajo `ore-kit/`, con la misma semilla en Parquet que S3, GCS, Azure y
//! SharePoint (`tipos/`, `vacia/`, `grande/`).
//!
//! `GDRIVE_KIT_ENDPOINT` dice dónde (`http://127.0.0.1:8792`). El conector sólo le
//! habla con `ORE_GDRIVE_LABORATORIO=1`, y se suplanta contra su IAM con
//! `ORE_GCP_IAM=<endpoint>/v1`, `ORE_GCP_TOKEN` y `ORE_GDRIVE_CUENTA` (lo que el
//! script del CI pone). La credencial mala es una carpeta que la cuenta ve y no
//! puede leer (`SINPERMISO`: `403`). Es un Drive de pruebas: **nunca la carpeta
//! de un cliente** —el kit escribe—.

use super::Banco;
use super::s3::{completar_fichero, sembrar};
use crate::semilla::Tabla;
use ore_base::json::Json;
use std::collections::BTreeMap;

const RAIZ: &str = "RAIZ";
const CARPETA: &str = "ore-kit";

pub struct GDrive {
    endpoint: String,
}

impl GDrive {
    pub fn new(endpoint: &str) -> GDrive {
        GDrive {
            endpoint: endpoint.trim_end_matches('/').to_string(),
        }
    }

    fn url_de(&self, raiz: &str) -> String {
        format!("gdrive://{raiz}/{CARPETA}/?endpoint={}", self.endpoint)
    }
}

/// Percent-encoding de un parámetro.
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

impl Banco for GDrive {
    fn familia(&self) -> &'static str {
        "gdrive"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let agente = ore_gcp::cliente()?;
        sembrar(|clave, datos| {
            let url = format!(
                "{}/_mentira?ruta={}",
                self.endpoint,
                codificar(&format!("{CARPETA}/{clave}"))
            );
            match agente.put(&url).send_bytes(datos) {
                Ok(x) if x.status() == 201 => Ok(()),
                Ok(x) => Err(format!("`{clave}` no se sube: {}", x.status())),
                Err(e) => Err(format!("no se llega al Drive de mentira: {e}")),
            }
        })
    }

    fn url(&self) -> String {
        self.url_de(RAIZ)
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Una carpeta que la cuenta ve y no puede leer: Drive contesta `403`.
    fn url_mala(&self) -> Option<String> {
        Some(self.url_de("SINPERMISO"))
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{CARPETA}/{}/", tabla.nombre())
    }

    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        completar_fichero(tabla, peticion)
    }
}
