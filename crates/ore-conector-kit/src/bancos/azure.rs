//! **El banco de Azure** (ADR 0061 O3·3): un contenedor `ore-kit` en Azurite
//! —en el CI y en el laboratorio—, por HTTPS con `--oauth basic`, con la misma
//! semilla en Parquet que S3 y GCS (`tipos/`, `vacia/`, `grande/`).
//!
//! `AZURE_KIT_ENDPOINT` dice dónde (`https://<host>:10000`, con su certificado
//! en el almacén del sistema) y `ORE_AZURE_TOKEN` con qué: el mismo Bearer que
//! usa el conector, que Azurite acepta sin mirar la firma. El conector sólo
//! habla con un servidor que no es de Azure con `ORE_AZURE_LABORATORIO=1`, que
//! el CI pone. Es un Azure de pruebas: **nunca el contenedor de un cliente**
//! —el kit escribe—.

use super::Banco;
use super::s3::{completar_fichero, sembrar};
use crate::semilla::Tabla;
use ore_core::json::Json;
use std::collections::BTreeMap;

const CUENTA: &str = "devstoreaccount1";
const CONTENEDOR: &str = "ore-kit";

pub struct Azure {
    endpoint: String,
    token: String,
}

impl Azure {
    pub fn new(endpoint: &str, token: &str) -> Azure {
        Azure {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    fn poner(&self, ruta: &str, cabeceras: &[(&str, &str)], cuerpo: &[u8]) -> Result<u16, String> {
        let agente = ore_gcp::cliente()?;
        let mut r = agente
            .put(&format!("{}/{CUENTA}/{ruta}", self.endpoint))
            .set("authorization", &format!("Bearer {}", self.token))
            .set("x-ms-version", "2023-11-03");
        for (k, v) in cabeceras {
            r = r.set(k, v);
        }
        match r.send_bytes(cuerpo) {
            Ok(x) => Ok(x.status()),
            Err(ureq::Error::Status(s, _)) => Ok(s),
            Err(e) => Err(format!("no se llega al Azure de pruebas: {e}")),
        }
    }
}

impl Banco for Azure {
    fn familia(&self) -> &'static str {
        "azure"
    }

    fn cargar(&mut self) -> Result<(), String> {
        // 201 si se crea; 409 si ya era nuestro.
        match self.poner(&format!("{CONTENEDOR}?restype=container"), &[], b"")? {
            201 | 409 => {}
            s => return Err(format!("el contenedor `{CONTENEDOR}` no se crea: {s}")),
        }
        sembrar(|clave, datos| {
            match self.poner(
                &format!("{CONTENEDOR}/{clave}"),
                &[("x-ms-blob-type", "BlockBlob")],
                datos,
            )? {
                201 => Ok(()),
                s => Err(format!("`{clave}` no se sube: {s}")),
            }
        })
    }

    fn url(&self) -> String {
        format!(
            "az://{CUENTA}/{CONTENEDOR}/?tenant=kit&cliente=kit&endpoint={}",
            self.endpoint
        )
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Azurite no mira la firma del token: no hay credencial mala que probar.
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
