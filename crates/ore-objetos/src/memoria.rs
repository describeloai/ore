//! **Un almacén en memoria, para las pruebas**: los mismos nombres raros que un
//! bucket de verdad, sin red. Cuenta lo que se le pide —el catálogo no debe
//! bajar los ficheros— y sabe hacer de un objeto que alguien reescribió entre
//! listar y leer.

use crate::{Abierto, Objeto, Origen, Version, huella};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

/// El motivo de una lectura fijada que no se pudo hacer porque el objeto
/// cambió: como lo dice el driver de S3 ante un `412`.
pub fn cambio(clave: &str) -> String {
    format!(
        "`{clave}` cambió mientras se leía (412): no se mezclan dos versiones; la copia \
         siguiente lo lee entero"
    )
}

#[derive(Default)]
pub struct EnMemoria {
    pub objetos: BTreeMap<String, Vec<u8>>,
    /// Cuántas lecturas se pidieron: el catálogo no debe bajar los ficheros.
    pub lecturas: Cell<usize>,
    pub bytes_leidos: Cell<usize>,
    /// Claves que el listado da con un ETag viejo: leerlas fijadas es un `412`,
    /// como un fichero que alguien reescribió entre listar y leer.
    pub cambiadas: BTreeSet<String>,
    /// La historia de versiones, si la prueba la quiere (E8·1): sin ella, cada
    /// objeto es su única versión, `null` y vigente. El contenido de una
    /// versión vieja va en `por_version`.
    pub historia: Vec<Version>,
    pub por_version: BTreeMap<(String, String), Vec<u8>>,
    /// Cuántas huellas se pidieron (un `HEAD` cada una, en S3).
    pub huellas: Cell<usize>,
}

impl EnMemoria {
    /// El ETag de unos bytes: lo que un origen da como validador.
    pub fn etag(v: &[u8]) -> String {
        format!("\"{}\"", &crate::hex(&crate::sha256(v))[..16])
    }

    fn fijado(&self, clave: &str, etag: &str) -> Result<&Vec<u8>, String> {
        let v = self
            .objetos
            .get(clave)
            .ok_or_else(|| format!("`{clave}` no está"))?;
        if EnMemoria::etag(v) != etag {
            return Err(cambio(clave));
        }
        Ok(v)
    }

    pub fn con(pares: &[(&str, Vec<u8>)]) -> EnMemoria {
        EnMemoria {
            objetos: pares
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            ..Default::default()
        }
    }
}

impl Origen for EnMemoria {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        Ok(self
            .objetos
            .iter()
            .filter(|(k, _)| k.starts_with(prefijo))
            .map(|(k, v)| Objeto {
                clave: k.clone(),
                tamano: v.len() as u64,
                etag: if self.cambiadas.contains(k) {
                    "\"viejo\"".into()
                } else {
                    EnMemoria::etag(v)
                },
                modificado: "2026-09-28T00:00:00.000Z".into(),
            })
            .collect())
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        let v = self
            .objetos
            .get(clave)
            .ok_or_else(|| format!("`{clave}` no está"))?;
        let (a, b) = rango.split_once('-').ok_or("rango sin guion")?;
        let trozo: &[u8] = if a.is_empty() {
            let n: usize = b.parse().map_err(|_| "sufijo malo")?;
            &v[v.len().saturating_sub(n)..]
        } else {
            let a: usize = a.parse().map_err(|_| "inicio malo")?;
            let b: usize = b
                .parse::<usize>()
                .map_err(|_| "fin malo")?
                .min(v.len().saturating_sub(1));
            if a > b || a >= v.len() {
                &[]
            } else {
                &v[a..=b]
            }
        };
        self.lecturas.set(self.lecturas.get() + 1);
        self.bytes_leidos.set(self.bytes_leidos.get() + trozo.len());
        Ok(trozo.to_vec())
    }

    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String> {
        let v = self.fijado(clave, etag)?;
        self.lecturas.set(self.lecturas.get() + 1);
        self.bytes_leidos.set(self.bytes_leidos.get() + v.len());
        Ok(Box::new(&v[..]))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        self.fijado(clave, etag)?;
        self.rango(clave, rango)
    }

    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        if !self.historia.is_empty() {
            return Ok(self
                .historia
                .iter()
                .filter(|v| v.clave.starts_with(prefijo))
                .cloned()
                .collect());
        }
        Ok(self
            .listar(prefijo)?
            .into_iter()
            .map(|o| Version {
                clave: o.clave,
                version: "null".into(),
                actual: true,
                marca: false,
                tamano: o.tamano,
                etag: o.etag,
                modificado: o.modificado,
            })
            .collect())
    }

    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        self.huellas.set(self.huellas.get() + 1);
        let v = self
            .por_version
            .get(&(clave.to_string(), version.to_string()))
            .or_else(|| self.objetos.get(clave).filter(|_| version == "null"))
            .ok_or_else(|| format!("`{clave}` (versión {version}) no está"))?;
        Ok(Some(huella::de(v)))
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String> {
        let v = self
            .por_version
            .get(&(clave.to_string(), version.to_string()))
            .or_else(|| self.objetos.get(clave).filter(|_| version == "null"))
            .ok_or_else(|| {
                format!("no se pudo leer `{clave}` (versión {version}): 404 NoSuchVersion")
            })?;
        self.lecturas.set(self.lecturas.get() + 1);
        self.bytes_leidos.set(self.bytes_leidos.get() + v.len());
        let a = Abierto {
            tamano: Some(v.len() as u64),
            tipo: Some("binary/octet-stream".into()),
            huella: Some(huella::de(v)),
        };
        Ok((Box::new(&v[..]), a))
    }
}
