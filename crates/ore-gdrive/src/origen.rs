//! **Una carpeta de Drive es un origen de objetos** (ADR 0082): el rasgo de
//! `ore-objetos` sobre la API v3. El validador de cada fichero es
//! `<id>@<revisión>` —también lo que el listado da como `etag` y como
//! versión—, y leer fijado es leer `revisions/{revisión}?alt=media`: esa, o un
//! `404` que es `media/cambiado` (X0 B2). Drive no tiene lectura condicional
//! (X0 B4): no hay `If-Match` que mandar.
//!
//! Los nativos (Docs, Hojas, Presentaciones) se listan, pero **no se leen
//! todavía**: sólo se exportan (X0 D1), y cómo se fijan es el paso 2.

use crate::{Clase, Fallo, Gdrive, Item};
use ore_objetos::{Abierto, Leido, Objeto, Origen, Rechazo, Version, memoria};
use std::io::Read;

/// `<id>@<revisión>` → (`id`, `revisión`).
fn partir(validador: &str) -> Option<(&str, &str)> {
    validador
        .trim_matches('"')
        .split_once('@')
        .filter(|(i, r)| !i.is_empty() && !r.is_empty())
}

/// Lo que se dice de un nativo, que aún no se lee.
fn nativo(clave: &str) -> String {
    format!(
        "`{clave}` is a native Google file: it is read by export, which is not built yet (ADR 0082)"
    )
}

/// Una lectura fijada que no se pudo hacer: la revisión ya no está (`404`) es
/// que el fichero cambió; lo demás se dice como vino.
fn motivo(clave: &str, f: &Fallo) -> String {
    match f.estado {
        404 => memoria::cambio(clave),
        _ => format!("could not read `{clave}`: {}", f.motivo()),
    }
}

impl Gdrive {
    fn todo(
        &self,
        id: &str,
        revision: Option<&str>,
        rango: Option<&str>,
        clave: &str,
    ) -> Result<Vec<u8>, String> {
        let mut l = self
            .bajar(id, revision, rango)
            .map_err(|f| motivo(clave, &f))?;
        let mut b = Vec::new();
        l.lector
            .read_to_end(&mut b)
            .map_err(|e| format!("could not read `{clave}` in full: {e}"))?;
        Ok(b)
    }

    /// El fichero de una clave que se puede leer (un binario).
    fn binario(&self, clave: &str) -> Result<Item, String> {
        let i = self
            .resolver(clave)
            .map_err(|f| format!("could not find `{clave}`: {}", f.motivo()))?;
        match i.clase {
            Clase::Fichero => Ok(i),
            _ => Err(nativo(clave)),
        }
    }
}

impl Origen for Gdrive {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        // Se recorre desde la carpeta del prefijo, que acaba en `/` o en un
        // trozo de nombre: sólo lo que empieza por él.
        let dir = prefijo.rsplit_once('/').map_or("", |(d, _)| d);
        let (items, _) = self
            .recorrer(dir)
            .map_err(|f| format!("could not list `{prefijo}`: {}", f.motivo()))?;
        Ok(items
            .into_iter()
            // Los nativos no se leen todavía: no entran en el catálogo.
            .filter(|i| i.clase == Clase::Fichero && i.ruta.starts_with(prefijo))
            .map(|i| Objeto {
                etag: i.validador(),
                tamano: i.tamano,
                modificado: i.modificado,
                clave: i.ruta,
            })
            .collect())
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        let i = self.binario(clave)?;
        self.todo(&i.id, None, Some(&format!("bytes={rango}")), clave)
    }

    /// Fijado a la revisión que el listado dijo (su `etag`).
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String> {
        let (id, rev) = partir(etag).ok_or_else(|| nativo(clave))?;
        let l = self
            .bajar(id, Some(rev), None)
            .map_err(|f| motivo(clave, &f))?;
        Ok(l.lector)
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        let (id, rev) = partir(etag).ok_or_else(|| nativo(clave))?;
        self.todo(id, Some(rev), Some(&format!("bytes={rango}")), clave)
    }

    /// Un lector sólo ve la revisión actual (X0 B2): una versión por fichero.
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        Ok(Origen::listar(self, prefijo)?
            .into_iter()
            .filter(|o| partir(&o.etag).is_some())
            .map(|o| Version {
                actual: true,
                marca: false,
                version: o.etag.clone(),
                clave: o.clave,
                tamano: o.tamano,
                etag: o.etag,
                modificado: o.modificado,
            })
            .collect())
    }

    /// El `md5` de esa revisión, que Drive da siempre y sin bajarla (X0 B1).
    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        let i = self.binario(clave)?;
        if !version.is_empty() && i.validador() != version.trim_matches('"') {
            return Err(format!(
                "could not inspect `{clave}` (version {version}): 404 no longer exists"
            ));
        }
        Ok(i.huella())
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String> {
        let (id, rev) = partir(version).ok_or_else(|| nativo(clave))?;
        let l = self
            .bajar(id, Some(rev), None)
            .map_err(|f| match f.estado {
                404 => {
                    format!("could not read `{clave}` (version {version}): 404 no longer exists")
                }
                _ => format!(
                    "could not read `{clave}` (version {version}): {}",
                    f.motivo()
                ),
            })?;
        let a = Abierto {
            tamano: l.cabecera("content-length").and_then(|v| v.parse().ok()),
            tipo: l.cabecera("content-type").map(String::from),
            huella: self
                .binario(clave)
                .ok()
                .filter(|i| i.validador() == version)
                .and_then(|i| i.huella()),
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
        // La revisión: la versión, o el validador si la versión no vino.
        let v = if version.is_empty() { etag } else { version };
        let Some((id, rev)) = partir(v) else {
            return Err(Rechazo::Origen(nativo(clave)));
        };
        self.bajar(id, Some(rev), rango)
            .map_err(|f| match f.estado {
                404 => Rechazo::Cambiado(format!(
                    "`{clave}` at the source is no longer the pinned revision ({})",
                    f.motivo()
                )),
                416 => Rechazo::Rango(format!("the range does not fit in `{clave}`")),
                0 => Rechazo::Origen(format!("the source is not responding: {}", f.cuerpo)),
                _ => Rechazo::Origen(format!("the source answered {} for `{clave}`", f.motivo())),
            })
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_validador_se_parte_en_id_y_revision() {
        assert_eq!(partir("15-5Fy@0Byo6"), Some(("15-5Fy", "0Byo6")));
        assert_eq!(partir("\"15-5Fy@0Byo6\""), Some(("15-5Fy", "0Byo6")));
        assert_eq!(partir("x@"), None);
        assert_eq!(partir("sin-arroba"), None);
    }
}
