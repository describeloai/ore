//! **Un bucket es un origen de objetos** (ADR 0061 O0·1): el rasgo de
//! `ore-objetos` sobre las lecturas de S3. Lo usan el driver (`ore-read-s3`) y
//! cualquiera que lea un bucket como origen; los que hablan la API de S3 (R2,
//! MinIO…) lo son también, con su `endpoint`.

use crate::Bucket;
use ore_objetos::{Abierto, Leido, Objeto, Origen, Rechazo, Version};
use std::borrow::Borrow;

/// **Un bucket, como origen**: el `Bucket` es de `ore-sigv4` (la firma sin red)
/// y el rasgo de `ore-objetos`, así que el rasgo va sobre esta envoltura,
/// prestada (`Cubo(&bucket)`, el driver) o propia (`Cubo(bucket)`, quien lo
/// guarda: `ore-medios`).
pub struct Cubo<B>(pub B);

impl Cubo<Bucket> {
    /// El de una URL `s3://…` con su credencial (la de una fuente).
    pub fn de_url(url: &str) -> Result<Cubo<Bucket>, String> {
        ore_sigv4::fuente::leer(url).map(|f| Cubo(f.bucket))
    }
}

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

impl<B: Borrow<Bucket>> Origen for Cubo<B> {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        crate::listar_todo(self.0.borrow(), prefijo).map_err(|e| match e {
            Ok(r) => format!("no se pudo listar `{prefijo}`: {}", r.motivo()),
            Err(t) => t,
        })
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        let r = crate::rango(self.0.borrow(), clave, rango)?;
        if !r.ok() {
            return Err(format!(
                "no se pudo leer `{clave}` ({rango}): {}",
                r.motivo()
            ));
        }
        Ok(r.cuerpo)
    }

    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn std::io::Read + '_>, String> {
        crate::abrir(self.0.borrow(), clave, etag)
            .map(|r| r as Box<dyn std::io::Read>)
            .map_err(|r| motivo_fijado(clave, &r))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        let r = crate::rango_de(self.0.borrow(), clave, rango, etag)?;
        if !r.ok() {
            return Err(motivo_fijado(clave, &r));
        }
        Ok(r.cuerpo)
    }

    /// Un origen que no versiona (`501`, ADR 0061 O1·1) da su listado: cada
    /// objeto, su única versión, la vigente, fijada por su ETag.
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        match crate::listar_versiones(self.0.borrow(), prefijo) {
            Ok(v) => Ok(v),
            Err(Ok(r)) if crate::no_versiona(&r) => Ok(self
                .listar(prefijo)?
                .into_iter()
                .map(|o| Version {
                    version: ore_objetos::version_por_etag(&o.etag),
                    clave: o.clave,
                    actual: true,
                    marca: false,
                    tamano: o.tamano,
                    etag: o.etag,
                    modificado: o.modificado,
                })
                .collect()),
            Err(Ok(r)) => Err(format!(
                "no se pudieron listar las versiones de `{prefijo}`: {}",
                r.motivo()
            )),
            Err(Err(t)) => Err(t),
        }
    }

    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        // Una que sólo es su ETag: la del objeto de ahora, si sigue siendo ése.
        if let Some(etag) = ore_objetos::etag_de(version) {
            let r = crate::cabeza(self.0.borrow(), clave)?;
            if !r.ok() {
                return Err(format!("no se pudo mirar `{clave}`: {}", r.motivo()));
            }
            if r.cabecera("etag") != Some(etag.as_str()) {
                return Ok(None);
            }
            return Ok(r
                .cabecera("x-amz-checksum-crc64nvme")
                .map(|c| format!("crc64nvme:{c}")));
        }
        let r = crate::cabeza_de(self.0.borrow(), clave, version)?;
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
        // Una que sólo es su ETag: la de ahora con `If-Match`; si cambió, 412.
        if let Some(etag) = ore_objetos::etag_de(version) {
            let l = crate::leer_fijado(self.0.borrow(), clave, "", &etag, None)
                .map_err(|r| motivo_fijado(clave, &r))?;
            let a = Abierto {
                tamano: l.cabecera("content-length").and_then(|v| v.parse().ok()),
                tipo: l.cabecera("content-type").map(String::from),
                huella: l
                    .cabecera("x-amz-checksum-crc64nvme")
                    .map(|c| format!("crc64nvme:{c}")),
            };
            return Ok((l.lector as Box<dyn std::io::Read>, a));
        }
        crate::abrir_version(self.0.borrow(), clave, version)
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

    fn leer_fijado(
        &self,
        clave: &str,
        version: &str,
        etag: &str,
        rango: Option<&str>,
    ) -> Result<Leido, Rechazo> {
        // Una versión que sólo es su ETag no viaja como `versionId`: es el
        // `If-Match` (O1·1).
        let (version, etag) = match ore_objetos::etag_de(version) {
            Some(e) => ("", if etag.is_empty() { e } else { etag.to_string() }),
            None => (version, etag.to_string()),
        };
        let l = crate::leer_fijado(self.0.borrow(), clave, version, &etag, rango)
            .map_err(|r| rechazo(&r, clave))?;
        Ok(Leido {
            estado: l.estado,
            cabeceras: l.cabeceras,
            lector: l.lector,
        })
    }

    /// SigV4 prefirmada con la credencial de la fuente, fijada a su
    /// `versionId` —también `null`, la de un objeto de antes del versionado
    /// (0049 B3·0)—, con el tipo y la disposición dentro de la firma; sin red,
    /// como `ore-firmar-s3` (0046 E9·3). Lleva la clave de acceso, no el
    /// secreto.
    fn firmar(
        &self,
        clave: &str,
        version: &str,
        tipo: &str,
        disposicion: &str,
        segundos: u64,
    ) -> Option<String> {
        // Una URL de navegador no lleva `If-Match`: de un objeto que sólo se
        // fija por su ETag podría dar otros bytes, así que no se firma (D-O1).
        if ore_objetos::etag_de(version).is_some() {
            return None;
        }
        let b: &Bucket = self.0.borrow();
        let mut extra: Vec<(&str, &str)> = Vec::new();
        if !version.is_empty() {
            extra.push(("versionId", version));
        }
        extra.push(("response-content-type", tipo));
        extra.push(("response-content-disposition", disposicion));
        let ruta = b.ruta(Some(clave));
        let q = ore_sigv4::firma::prefirmar(
            &b.credencial,
            &b.region,
            &b.host(),
            &ruta,
            &extra,
            segundos,
        );
        Some(format!("{}{ruta}?{q}", b.endpoint))
    }
}

/// Lo que S3 contestó a una lectura fijada, en el idioma del contrato. Un ítem
/// que el manifiesto lista y el origen ya no tiene en esa versión **cambió**.
fn rechazo(r: &crate::Respuesta, clave: &str) -> Rechazo {
    let codigo_aws = r.error_de_aws().map(|(c, _)| c).unwrap_or_default();
    match (r.estado, codigo_aws.as_str()) {
        (412, _) | (404, _) | (400, "InvalidArgument") => Rechazo::Cambiado(format!(
            "`{clave}` ya no es, en el origen, la versión fijada ({})",
            r.motivo()
        )),
        (416, _) => Rechazo::Rango(format!("el rango no cabe en `{clave}`")),
        (0, _) => Rechazo::Origen(format!(
            "el origen no contesta: {}",
            String::from_utf8_lossy(&r.cuerpo)
        )),
        _ => Rechazo::Origen(format!("el origen contestó {} a `{clave}`", r.motivo())),
    }
}
