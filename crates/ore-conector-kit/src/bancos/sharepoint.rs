//! **El banco de SharePoint** (ADR 0061 O5·3): la biblioteca `Documentos` del
//! sitio `sites/Finanzas` del Graph de mentira (`pruebas-de-fuego/graph-de-mentira.py`)
//! —no hay emulador de Graph—, bajo `ore-kit/`, con la misma semilla en Parquet
//! que S3, GCS y Azure (`tipos/`, `vacia/`, `grande/`).
//!
//! `GRAPH_KIT_ENDPOINT` dice dónde (`http://127.0.0.1:8790`); el token lo pone
//! `ORE_GRAPH_TOKEN`, el mismo que el Graph de mentira exige, y el conector
//! sólo le habla con `ORE_SHAREPOINT_LABORATORIO=1`. La credencial mala es un
//! sitio sin la concesión de `Sites.Selected` (`sites/RRHH`: `403
//! accessDenied`). Es un Graph de pruebas: **nunca la biblioteca de un
//! cliente** —el kit escribe—.

use super::Banco;
use super::s3::{completar_fichero, sembrar};
use crate::semilla::Tabla;
use ore_core::json::Json;
use std::collections::BTreeMap;

const SITIO: &str = "sites/Finanzas";
const BIBLIOTECA: &str = "Documentos";
const CARPETA: &str = "ore-kit";

pub struct SharePoint {
    endpoint: String,
}

impl SharePoint {
    pub fn new(endpoint: &str) -> SharePoint {
        SharePoint {
            endpoint: endpoint.trim_end_matches('/').to_string(),
        }
    }

    fn url_de(&self, sitio: &str) -> String {
        format!(
            "sharepoint://contoso.sharepoint.com/{sitio}/{BIBLIOTECA}/{CARPETA}/?tenant=kit&cliente=kit&endpoint={}",
            self.endpoint
        )
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

impl Banco for SharePoint {
    fn familia(&self) -> &'static str {
        "sharepoint"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let agente = ore_gcp::cliente()?;
        sembrar(|clave, datos| {
            let url = format!(
                "{}/_mentira?sitio={}&biblioteca={BIBLIOTECA}&ruta={}",
                self.endpoint,
                codificar(SITIO),
                codificar(&format!("{CARPETA}/{clave}"))
            );
            match agente.put(&url).send_bytes(datos) {
                Ok(x) if x.status() == 201 => Ok(()),
                Ok(x) => Err(format!("`{clave}` no se sube: {}", x.status())),
                Err(e) => Err(format!("no se llega al Graph de mentira: {e}")),
            }
        })
    }

    fn url(&self) -> String {
        self.url_de(SITIO)
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Un sitio sin la concesión de la app: Graph contesta `403 accessDenied`.
    fn url_mala(&self) -> Option<String> {
        Some(self.url_de("sites/RRHH"))
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{CARPETA}/{}/", tabla.nombre())
    }

    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        completar_fichero(tabla, peticion)
    }
}
