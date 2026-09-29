//! **Lo que el catálogo le pide a un bucket**, detrás de un rasgo: listar y
//! leer un rango. Así el catálogo se prueba sin red —un bucket en memoria con
//! los mismos nombres raros que el de F1— y el de verdad es una capa fina
//! sobre `ore-s3`.

use ore_s3::{Bucket, Objeto};
#[cfg(test)]
use std::collections::BTreeMap;

pub trait Origen {
    /// Todo lo que hay bajo un prefijo.
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String>;
    /// Un rango de bytes: `0-15`, o el sufijo `-8`.
    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String>;
    /// **Leer (0046 E6)**: el objeto entero, en flujo, de la versión que el
    /// listado dijo (`etag`); si cambió, falla.
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn std::io::Read + '_>, String>;
    /// Un rango de la versión que el listado dijo.
    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String>;
    /// **Las versiones y las marcas** bajo un prefijo (0046 E8·1).
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<ore_s3::Version>, String>;
    /// La huella de contenido de UNA versión: su CRC64NVME si S3 lo da
    /// (`FULL_OBJECT`, por defecto desde 2025), o `None`.
    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String>;
}

/// El motivo de una lectura fijada que no se pudo hacer: un `412` es que el
/// fichero cambió entre el listado y la lectura, y se dice así.
fn motivo_fijado(clave: &str, r: &ore_s3::Respuesta) -> String {
    if r.estado == 412 {
        format!(
            "`{clave}` cambió mientras se leía (412): no se mezclan dos versiones; la copia \
             siguiente lo lee entero"
        )
    } else if r.estado == 0 {
        String::from_utf8_lossy(&r.cuerpo).into_owned()
    } else {
        format!("no se pudo leer `{clave}`: {}", r.motivo())
    }
}

impl Origen for Bucket {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        ore_s3::listar_todo(self, prefijo).map_err(|e| match e {
            Ok(r) => format!("no se pudo listar `{prefijo}`: {}", r.motivo()),
            Err(t) => t,
        })
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        let r = ore_s3::rango(self, clave, rango)?;
        if !r.ok() {
            return Err(format!(
                "no se pudo leer `{clave}` ({rango}): {}",
                r.motivo()
            ));
        }
        Ok(r.cuerpo)
    }

    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn std::io::Read + '_>, String> {
        ore_s3::abrir(self, clave, etag)
            .map(|r| r as Box<dyn std::io::Read>)
            .map_err(|r| motivo_fijado(clave, &r))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        let r = ore_s3::rango_de(self, clave, rango, etag)?;
        if !r.ok() {
            return Err(motivo_fijado(clave, &r));
        }
        Ok(r.cuerpo)
    }

    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<ore_s3::Version>, String> {
        ore_s3::listar_versiones(self, prefijo).map_err(|e| match e {
            Ok(r) => format!(
                "no se pudieron listar las versiones de `{prefijo}`: {}",
                r.motivo()
            ),
            Err(t) => t,
        })
    }

    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        let r = ore_s3::cabeza_de(self, clave, version)?;
        if !r.ok() {
            return Err(format!(
                "no se pudo mirar `{clave}` (versión {version}): {}",
                r.motivo()
            ));
        }
        Ok(r.cabecera("x-amz-checksum-crc64nvme")
            .map(|c| format!("crc64nvme:{c}")))
    }
}

/// Un bucket en memoria, para las pruebas.
#[cfg(test)]
#[derive(Default)]
pub struct EnMemoria {
    pub objetos: BTreeMap<String, Vec<u8>>,
    /// Cuántas lecturas se pidieron: el catálogo no debe bajar los ficheros.
    pub lecturas: std::cell::Cell<usize>,
    pub bytes_leidos: std::cell::Cell<usize>,
    /// Claves que el listado da con un ETag viejo: leerlas fijadas es un `412`,
    /// como un fichero que alguien reescribió entre listar y leer.
    pub cambiadas: std::collections::BTreeSet<String>,
    /// La historia de versiones, si la prueba la quiere (E8·1): sin ella, cada
    /// objeto es su única versión, `null` y vigente. El contenido de una
    /// versión vieja va en `por_version`.
    pub historia: Vec<ore_s3::Version>,
    pub por_version: BTreeMap<(String, String), Vec<u8>>,
    /// Cuántas huellas se pidieron (un `HEAD` cada una, en S3).
    pub huellas: std::cell::Cell<usize>,
}

#[cfg(test)]
impl EnMemoria {
    fn etag(v: &[u8]) -> String {
        format!("\"{}\"", &ore_s3::hex(&ore_s3::sha256(v))[..16])
    }

    fn fijado(&self, clave: &str, etag: &str) -> Result<&Vec<u8>, String> {
        let v = self
            .objetos
            .get(clave)
            .ok_or_else(|| format!("`{clave}` no está"))?;
        if EnMemoria::etag(v) != etag {
            return Err(motivo_fijado(
                clave,
                &ore_s3::Respuesta {
                    estado: 412,
                    cabeceras: Vec::new(),
                    cuerpo: Vec::new(),
                },
            ));
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

#[cfg(test)]
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

    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn std::io::Read + '_>, String> {
        let v = self.fijado(clave, etag)?;
        self.lecturas.set(self.lecturas.get() + 1);
        self.bytes_leidos.set(self.bytes_leidos.get() + v.len());
        Ok(Box::new(&v[..]))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        self.fijado(clave, etag)?;
        self.rango(clave, rango)
    }

    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<ore_s3::Version>, String> {
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
            .map(|o| ore_s3::Version {
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
        Ok(Some(format!(
            "crc64nvme:{}",
            &ore_s3::hex(&ore_s3::sha256(v))[..12]
        )))
    }
}
