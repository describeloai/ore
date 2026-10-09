//! **Un bucket de GCS es un origen de objetos** (ADR 0061 O2·1): el rasgo de
//! `ore-objetos` sobre la API JSON. El validador de cada objeto es su
//! **generación** —también lo que el listado da como `etag`—, y leer fijado es
//! leer `?generation=N`: esa, o un `404` que es `media/cambiado`.

use crate::{Fallo, Gcs, Meta};
use ore_objetos::{Abierto, Leido, Objeto, Origen, Rechazo, Version, memoria};
use std::io::Read;

/// Una lectura fijada que no se pudo hacer: la generación ya no está (`404`),
/// o no casa (`412`), es que el objeto cambió; lo demás se dice como vino.
fn motivo(clave: &str, f: &Fallo) -> String {
    match f.estado {
        404 | 412 => memoria::cambio(clave),
        _ => format!("no se pudo leer `{clave}`: {}", f.motivo()),
    }
}

/// `crc32c:<b64>`, como la escribe la colección.
fn huella(m: &Meta) -> Option<String> {
    m.crc32c.as_ref().map(|c| format!("crc32c:{c}"))
}

/// La del `x-goog-hash` de una descarga (`crc32c=…,md5=…`).
fn huella_de_cabecera(l: &Leido) -> Option<String> {
    l.cabecera("x-goog-hash")?
        .split(',')
        .find_map(|p| p.trim().strip_prefix("crc32c="))
        .map(|c| format!("crc32c:{c}"))
}

impl Gcs {
    fn todo(
        &self,
        clave: &str,
        generacion: Option<&str>,
        rango: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let mut l = self
            .bajar(clave, generacion, rango)
            .map_err(|f| motivo(clave, &f))?;
        let mut b = Vec::new();
        l.lector
            .read_to_end(&mut b)
            .map_err(|e| format!("`{clave}` no se pudo leer entero: {e}"))?;
        Ok(b)
    }
}

impl Origen for Gcs {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        let (metas, _) = self
            .listar(prefijo, false, None)
            .map_err(|f| format!("no se pudo listar `{prefijo}`: {}", f.motivo()))?;
        Ok(metas
            .into_iter()
            .map(|m| Objeto {
                clave: m.nombre,
                tamano: m.tamano,
                etag: m.generacion,
                modificado: m.actualizado,
            })
            .collect())
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        self.todo(clave, None, Some(&format!("bytes={rango}")))
    }

    /// Fijado a la generación que el listado dijo (su `etag`).
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String> {
        let l = self
            .bajar(clave, Some(etag.trim_matches('"')), None)
            .map_err(|f| motivo(clave, &f))?;
        Ok(l.lector)
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        self.todo(
            clave,
            Some(etag.trim_matches('"')),
            Some(&format!("bytes={rango}")),
        )
    }

    /// Cada generación (`versions=true`): la vigente y, con Object Versioning,
    /// las de antes (`timeDeleted`), que se siguen pudiendo leer.
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        let (metas, _) = self.listar(prefijo, true, None).map_err(|f| {
            format!(
                "no se pudieron listar las versiones de `{prefijo}`: {}",
                f.motivo()
            )
        })?;
        Ok(metas
            .into_iter()
            .map(|m| Version {
                actual: !m.borrada,
                marca: false,
                tamano: m.tamano,
                etag: m.generacion.clone(),
                version: m.generacion,
                clave: m.nombre,
                modificado: m.actualizado,
            })
            .collect())
    }

    /// El `crc32c` de esa generación, que GCS da siempre y sin bajarla.
    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        let m = self
            .meta(clave, Some(version).filter(|v| !v.is_empty()))
            .map_err(|f| {
                format!(
                    "no se pudo mirar `{clave}` (generación {version}): {}",
                    f.motivo()
                )
            })?;
        Ok(huella(&m))
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String> {
        let l = self
            .bajar(clave, Some(version), None)
            .map_err(|f| match f.estado {
                404 => format!("no se pudo leer `{clave}` (generación {version}): 404 ya no está"),
                _ => format!(
                    "no se pudo leer `{clave}` (generación {version}): {}",
                    f.motivo()
                ),
            })?;
        let a = Abierto {
            tamano: l.cabecera("content-length").and_then(|v| v.parse().ok()),
            tipo: l.cabecera("content-type").map(String::from),
            huella: huella_de_cabecera(&l),
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
        // La generación: la versión, o el validador si la versión no vino.
        let g = if version.is_empty() {
            etag.trim_matches('"')
        } else {
            version
        };
        self.bajar(clave, Some(g).filter(|g| !g.is_empty()), rango)
            .map_err(|f| match f.estado {
                404 | 412 => Rechazo::Cambiado(format!(
                    "`{clave}` ya no es, en el origen, la generación fijada ({})",
                    f.motivo()
                )),
                416 => Rechazo::Rango(format!("el rango no cabe en `{clave}`")),
                0 => Rechazo::Origen(format!("el origen no contesta: {}", f.cuerpo)),
                _ => Rechazo::Origen(format!("el origen contestó {} a `{clave}`", f.motivo())),
            })
    }

    fn firmar(
        &self,
        clave: &str,
        version: &str,
        tipo: &str,
        disposicion: &str,
        segundos: u64,
    ) -> Result<Option<String>, String> {
        Gcs::firmar(self, clave, version, tipo, disposicion, segundos).map(Some)
    }
}
