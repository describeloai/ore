//! **Un servidor SFTP es un origen de objetos** (ADR 0061 O4·1), pero sólo para
//! copiar (D-O1): la versión de un fichero es su validador (`etag:<mtime>-<tamaño>`),
//! cada lectura lo comprueba al abrir y al terminar, y leer fijado —lo que
//! sirve una colección virtual— se niega con su porqué.

use crate::{Fallo, Sftp, Tipo, iso, validador};
use ore_objetos::{Abierto, Objeto, Origen, Rechazo, Version, etag_de, version_por_etag};
use std::io::Read;

fn texto(f: Fallo) -> String {
    f.mensaje
}

impl Origen for Sftp {
    /// Los ficheros (los enlaces simbólicos no: no se siguen).
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        Ok(Sftp::listar(self, prefijo)
            .map_err(texto)?
            .into_iter()
            .filter(|e| !e.enlace)
            .map(|e| Objeto {
                etag: validador(e.tamano, e.mtime),
                tamano: e.tamano,
                modificado: iso(e.mtime),
                clave: e.clave,
            })
            .collect())
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        Sftp::rango(self, clave, rango, None).map_err(texto)
    }

    /// Fijado al validador que el listado dijo.
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String> {
        Ok(Box::new(
            Sftp::leer(self, clave, Some(etag)).map_err(texto)?,
        ))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        Sftp::rango(self, clave, rango, Some(etag)).map_err(texto)
    }

    /// Lo vigente, con el validador como versión; sin los que llevan menos de
    /// `edad` segundos sin cambiar (pueden estar a medio escribir: se copian
    /// la vez siguiente) ni los enlaces.
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        Ok(Sftp::listar(self, prefijo)
            .map_err(texto)?
            .into_iter()
            .filter(|e| !e.enlace && !self.joven(e.mtime))
            .map(|e| {
                let v = validador(e.tamano, e.mtime);
                Version {
                    actual: true,
                    marca: false,
                    tamano: e.tamano,
                    version: version_por_etag(&v),
                    etag: v,
                    modificado: iso(e.mtime),
                    clave: e.clave,
                }
            })
            .collect())
    }

    /// Un SFTP no da huella sin bajar el fichero: la versión en el lago es su
    /// sha256, y se coteja el tamaño.
    fn huella_de(&self, _clave: &str, _version: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String> {
        let esperado = etag_de(version);
        let l = Sftp::leer(self, clave, esperado.as_deref()).map_err(|f| match f.tipo {
            Tipo::Cambio | Tipo::NoEsta => {
                format!("no se pudo leer `{clave}` (versión {version}): ya no está como se listó")
            }
            _ => format!(
                "no se pudo leer `{clave}` (versión {version}): {}",
                f.mensaje
            ),
        })?;
        let a = Abierto {
            tamano: Some(l.tamano),
            tipo: None,
            huella: None,
        };
        Ok((Box::new(l), a))
    }

    fn leer_fijado(
        &self,
        clave: &str,
        _version: &str,
        _etag: &str,
        _rango: Option<&str>,
    ) -> Result<ore_objetos::Leido, Rechazo> {
        Err(Rechazo::Origen(format!(
            "`{clave}` está en un SFTP, que no versiona: sus colecciones sólo pueden ser \
             mantenidas (la copia en el lago), nunca virtuales (ADR 0061, D-O1)"
        )))
    }
}
