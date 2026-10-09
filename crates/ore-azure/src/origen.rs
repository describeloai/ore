//! **Un contenedor de Azure es un origen de objetos** (ADR 0061 O3·1): el
//! rasgo de `ore-objetos` sobre la API de Blob. La versión de un blob es su
//! `versionId` si la cuenta versiona y, si no, su ETag (`etag:…`, D-O1); y
//! toda lectura fijada lleva además el ETag (`If-Match`), porque un servidor
//! que ignore `versionid` —Azurite, medido en O3·0— no debe poder dar otros
//! bytes.

use crate::{Azure, Fallo, Meta};
use ore_objetos::{
    Abierto, Leido, Objeto, Origen, Rechazo, Version, etag_de, memoria, version_por_etag,
};
use std::io::Read;

/// Una lectura fijada que no se pudo hacer: el blob o la versión ya no está
/// (`404`), o el ETag ya no casa (`412`): el objeto cambió. Lo demás, como vino.
fn motivo(clave: &str, f: &Fallo) -> String {
    match f.estado {
        404 | 412 => memoria::cambio(clave),
        _ => format!("no se pudo leer `{clave}`: {}", f.motivo()),
    }
}

/// `md5:<b64>`, como la escribe la colección.
fn huella(md5: Option<&str>) -> Option<String> {
    md5.filter(|m| !m.is_empty()).map(|m| format!("md5:{m}"))
}

/// La versión de un blob como la dice la colección: su `versionId`, o su ETag.
fn version_de(m: &Meta) -> String {
    m.version
        .clone()
        .unwrap_or_else(|| version_por_etag(&m.etag))
}

/// Una versión de la colección partida en lo que se le pide a Azure: el
/// `versionid` (si no es un ETag) y el ETag de la precondición.
fn partir<'a>(version: &'a str, etag: &str) -> (Option<&'a str>, Option<String>) {
    match etag_de(version) {
        Some(e) => (None, Some(e)),
        None => (
            Some(version).filter(|v| !v.is_empty()),
            Some(etag.to_string()).filter(|e| !e.is_empty()),
        ),
    }
}

impl Azure {
    fn todo(
        &self,
        clave: &str,
        etag: Option<&str>,
        rango: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let mut l = self
            .bajar(clave, None, etag, rango)
            .map_err(|f| motivo(clave, &f))?;
        let mut b = Vec::new();
        l.lector
            .read_to_end(&mut b)
            .map_err(|e| format!("`{clave}` no se pudo leer entero: {e}"))?;
        Ok(b)
    }
}

impl Origen for Azure {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        let (metas, _) = Azure::listar(self, prefijo, false, None, None)
            .map_err(|f| format!("no se pudo listar `{prefijo}`: {}", f.motivo()))?;
        Ok(metas
            .into_iter()
            .map(|m| Objeto {
                clave: m.nombre,
                tamano: m.tamano,
                etag: m.etag,
                modificado: m.modificado,
            })
            .collect())
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        self.todo(clave, None, Some(&format!("bytes={rango}")))
    }

    /// Fijado al ETag que el listado dijo.
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String> {
        let l = self
            .bajar(clave, None, Some(etag), None)
            .map_err(|f| motivo(clave, &f))?;
        Ok(l.lector)
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        self.todo(clave, Some(etag), Some(&format!("bytes={rango}")))
    }

    /// Cada versión (`include=versions`) si la cuenta versiona; si no —o un
    /// blob de antes de activarlo, que aún no tiene—, su ETag.
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        let (metas, _) = Azure::listar(self, prefijo, true, None, None).map_err(|f| {
            format!(
                "no se pudieron listar las versiones de `{prefijo}`: {}",
                f.motivo()
            )
        })?;
        Ok(metas
            .into_iter()
            .map(|m| Version {
                actual: m.actual,
                marca: false,
                tamano: m.tamano,
                version: version_de(&m),
                etag: m.etag,
                clave: m.nombre,
                modificado: m.modificado,
            })
            .collect())
    }

    /// El `Content-MD5` de esa versión, si el blob lo tiene (Put Blob sí; Put
    /// Block List sólo si quien subió lo dio). De una versión que es un ETag,
    /// sólo si el blob sigue siendo ése.
    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        let (v, e) = partir(version, "");
        let m = self.meta(clave, v).map_err(|f| {
            format!(
                "no se pudo mirar `{clave}` (versión {version}): {}",
                f.motivo()
            )
        })?;
        if let Some(e) = e
            && e != m.etag
        {
            return Err(memoria::cambio(clave));
        }
        Ok(huella(m.md5.as_deref()))
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String> {
        let (v, e) = partir(version, "");
        let l = self
            .bajar(clave, v, e.as_deref(), None)
            .map_err(|f| match f.estado {
                404 | 412 => format!(
                    "no se pudo leer `{clave}` (versión {version}): {} ya no está",
                    f.estado
                ),
                _ => format!(
                    "no se pudo leer `{clave}` (versión {version}): {}",
                    f.motivo()
                ),
            })?;
        let a = Abierto {
            tamano: l.cabecera("content-length").and_then(|v| v.parse().ok()),
            tipo: l.cabecera("content-type").map(String::from),
            huella: huella(
                l.cabecera("content-md5")
                    .or_else(|| l.cabecera("x-ms-blob-content-md5")),
            ),
        };
        Ok((l.lector, a))
    }

    fn leer_fijado(
        &self,
        clave: &str,
        version: &str,
        etag: &str,
        rango: Option<&str>,
    ) -> Result<Leido, Rechazo> {
        let (v, e) = partir(version, etag);
        self.bajar(clave, v, e.as_deref(), rango)
            .map_err(|f| match f.estado {
                404 | 412 => Rechazo::Cambiado(format!(
                    "`{clave}` ya no es, en el origen, la versión fijada ({})",
                    f.motivo()
                )),
                416 => Rechazo::Rango(format!("el rango no cabe en `{clave}`")),
                0 => Rechazo::Origen(format!("el origen no contesta: {}", f.cuerpo)),
                _ => Rechazo::Origen(format!("el origen contestó {} a `{clave}`", f.motivo())),
            })
    }

    /// Una SAS de delegación fijada a la versión (`sr=bv`). De un blob que
    /// sólo se fija por su ETag, ninguna: una URL no lleva `If-Match` (D-O1),
    /// y se abre por `content`.
    fn firmar(
        &self,
        clave: &str,
        version: &str,
        tipo: &str,
        disposicion: &str,
        segundos: u64,
    ) -> Result<Option<String>, String> {
        if version.is_empty() || etag_de(version).is_some() {
            return Ok(None);
        }
        Azure::firmar(self, clave, Some(version), tipo, disposicion, segundos).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una versión de verdad va como `versionid` y con su ETag; una que es un
    /// ETag, sólo como precondición.
    #[test]
    fn la_version_se_parte_en_versionid_y_if_match() {
        assert_eq!(
            partir("2026-10-09T10:00:00.0000000Z", "\"0x1\""),
            (Some("2026-10-09T10:00:00.0000000Z"), Some("\"0x1\"".into()))
        );
        assert_eq!(partir("etag:0x1", ""), (None, Some("\"0x1\"".into())));
        assert_eq!(partir("", ""), (None, None));
        let m = Meta {
            etag: "\"0x2\"".into(),
            ..Meta::default()
        };
        assert_eq!(version_de(&m), "etag:0x2");
        assert_eq!(
            huella(Some("CInfjbZ21DfOIWDhgYr6dw==")).as_deref(),
            Some("md5:CInfjbZ21DfOIWDhgYr6dw==")
        );
        assert_eq!(huella(Some("")), None);
    }
}
