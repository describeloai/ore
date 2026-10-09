//! **El banco de SFTP** (ADR 0061 O4·3): un directorio `ore-kit` en un
//! `atmoz/sftp` —OpenSSH de verdad, en el CI y en el laboratorio—, con la misma
//! semilla en Parquet que los demás almacenes (`tipos/`, `vacia/`, `grande/`).
//!
//! `SFTP_KIT_HOST` (y `SFTP_KIT_PUERTO`, 22 si no) dice dónde, `SFTP_KIT_HUELLA`
//! la huella del host que la URL fija, y `ORE_SFTP_CLAVE` la clave con la que
//! entra el conector —y con la que el kit sube, porque es un servidor de
//! pruebas: **nunca el de un cliente**, el kit escribe—. Un SFTP no fija nada
//! (D-O1): las lecturas se vigilan por tamaño y `mtime`, y el kit pide
//! `edad=0` para ver lo que acaba de subir.

use super::Banco;
use super::s3::{completar_fichero, sembrar};
use crate::semilla::Tabla;
use ore_core::json::Json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

const RAIZ: &str = "ore-kit";

pub struct Sftp {
    host: String,
    puerto: u16,
    huella: String,
    clave: String,
}

impl Sftp {
    pub fn new(host: &str, puerto: u16, huella: &str, clave: &str) -> Sftp {
        Sftp {
            host: host.to_string(),
            puerto,
            huella: huella.to_string(),
            clave: clave.to_string(),
        }
    }

    fn sesion(&self) -> Result<ssh2::Sftp, String> {
        let tcp = std::net::TcpStream::connect((self.host.as_str(), self.puerto))
            .map_err(|e| format!("no se llega al SFTP de pruebas: {e}"))?;
        let mut s = ssh2::Session::new().map_err(|e| e.to_string())?;
        s.set_tcp_stream(tcp);
        s.handshake()
            .map_err(|e| format!("el saludo SSH falla: {e}"))?;
        s.userauth_pubkey_file("ore", None, Path::new(&self.clave), None)
            .map_err(|e| format!("la clave no entra en el SFTP de pruebas: {e}"))?;
        s.sftp().map_err(|e| e.to_string())
    }
}

impl Banco for Sftp {
    fn familia(&self) -> &'static str {
        "sftp"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let s = self.sesion()?;
        sembrar(|clave, datos| {
            let ruta = format!("/{RAIZ}/{clave}");
            if let Some((dir, _)) = ruta.rsplit_once('/') {
                // ya existe, si no es la primera vez
                let _ = s.mkdir(Path::new(dir), 0o755);
            }
            let mut f = s
                .create(Path::new(&ruta))
                .map_err(|e| format!("`{clave}` no se sube: {e}"))?;
            f.write_all(datos)
                .map_err(|e| format!("`{clave}` no se sube: {e}"))
        })
    }

    fn url(&self) -> String {
        let puerto = if self.puerto == 22 {
            String::new()
        } else {
            format!(":{}", self.puerto)
        };
        format!(
            "sftp://ore@{}{puerto}/{RAIZ}/?huella={}&edad=0",
            self.host, self.huella
        )
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Otra huella fijada: el conector se niega antes de autenticar.
    fn url_mala(&self) -> Option<String> {
        Some(
            self.url()
                .replace(&self.huella, "SHA256:otraotraotraotraotraotraotra"),
        )
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{RAIZ}/{}/", tabla.nombre())
    }

    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        completar_fichero(tabla, peticion)
    }
}
