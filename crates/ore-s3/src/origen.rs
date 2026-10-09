//! **Un bucket es un origen de objetos** (ADR 0061 O0·1): el rasgo de
//! `ore-objetos` sobre las lecturas de S3. Lo usan el driver (`ore-read-s3`) y
//! cualquiera que lea un bucket como origen; los que hablan la API de S3 (R2,
//! MinIO…) lo son también, con su `endpoint`.

use crate::Bucket;
use ore_objetos::{Abierto, Objeto, Origen, Version};

/// **Un bucket, como origen**: el `Bucket` es de `ore-sigv4` (la firma sin red)
/// y el rasgo de `ore-objetos`, así que el rasgo va sobre esta envoltura.
#[derive(Clone, Copy)]
pub struct Cubo<'a>(pub &'a Bucket);

/// El motivo de una lectura fijada que no se pudo hacer: un `412` es que el
/// fichero cambió entre el listado y la lectura, y se dice así.
fn motivo_fijado(clave: &str, r: &crate::Respuesta) -> String {
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

impl Origen for Cubo<'_> {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        crate::listar_todo(self.0, prefijo).map_err(|e| match e {
            Ok(r) => format!("no se pudo listar `{prefijo}`: {}", r.motivo()),
            Err(t) => t,
        })
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        let r = crate::rango(self.0, clave, rango)?;
        if !r.ok() {
            return Err(format!(
                "no se pudo leer `{clave}` ({rango}): {}",
                r.motivo()
            ));
        }
        Ok(r.cuerpo)
    }

    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn std::io::Read + '_>, String> {
        crate::abrir(self.0, clave, etag)
            .map(|r| r as Box<dyn std::io::Read>)
            .map_err(|r| motivo_fijado(clave, &r))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        let r = crate::rango_de(self.0, clave, rango, etag)?;
        if !r.ok() {
            return Err(motivo_fijado(clave, &r));
        }
        Ok(r.cuerpo)
    }

    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        crate::listar_versiones(self.0, prefijo).map_err(|e| match e {
            Ok(r) => format!(
                "no se pudieron listar las versiones de `{prefijo}`: {}",
                r.motivo()
            ),
            Err(t) => t,
        })
    }

    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        let r = crate::cabeza_de(self.0, clave, version)?;
        if !r.ok() {
            return Err(format!(
                "no se pudo mirar `{clave}` (versión {version}): {}",
                r.motivo()
            ));
        }
        Ok(r.cabecera("x-amz-checksum-crc64nvme")
            .map(|c| format!("crc64nvme:{c}")))
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn std::io::Read + '_>, Abierto), String> {
        crate::abrir_version(self.0, clave, version)
            .map(|(r, a)| (r as Box<dyn std::io::Read>, a))
            .map_err(|r| {
                if r.estado == 0 {
                    String::from_utf8_lossy(&r.cuerpo).into_owned()
                } else {
                    format!(
                        "no se pudo leer `{clave}` (versión {version}): {}",
                        r.motivo()
                    )
                }
            })
    }
}
