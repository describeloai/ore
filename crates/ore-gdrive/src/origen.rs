//! **Una carpeta de Drive es un origen de objetos** (ADR 0082): el rasgo de
//! `ore-objetos` sobre la API v3.
//!
//! Un **binario** tiene como validador `<id>@<revisión>` —también lo que el
//! listado da como `etag` y como versión—, y leer fijado es leer
//! `revisions/{revisión}?alt=media`: esa, o un `404` que es `media/cambiado`
//! (X0 B2). Drive no tiene lectura condicional (X0 B4): no hay `If-Match`.
//!
//! Un **nativo** (Doc, Hoja, Presentación) no tiene bytes ni revisión que un
//! lector vea (X0 D1, D5): se exporta (`Gdrive::exportar`), su validador es
//! `<id>#v<version>` y se lee **vigilado**: si el `version` cambió mientras se
//! exportaba, falla. Su huella se calcula de lo exportado (no hay `md5`), y el
//! rango se corta de ello, porque Drive ignora el `Range` de un export.

use crate::{Clase, Fallo, Gdrive, Item};
use ore_objetos::huella::{Calculo, Md5};
use ore_objetos::{Abierto, Leido, Objeto, Origen, Rechazo, Version, memoria};
use std::io::Read;
use std::sync::Arc;

/// `<id>@<revisión>` → (`id`, `revisión`). Un nativo (`<id>#v<version>`) no.
fn partir(validador: &str) -> Option<(&str, &str)> {
    validador
        .trim_matches('"')
        .split_once('@')
        .filter(|(i, r)| !i.is_empty() && !r.is_empty())
}

/// Una lectura fijada que no se pudo hacer: la revisión ya no está (`404`) es
/// que el fichero cambió; lo demás se dice como vino.
fn motivo(clave: &str, f: &Fallo) -> String {
    match f.estado {
        404 => memoria::cambio(clave),
        _ => format!("could not read `{clave}`: {}", f.motivo()),
    }
}

/// `bytes=0-15`, `0-15`, `-8` o `5-` → el trozo de `len` bytes que dice
/// (inicio, fin exclusivo), o `Err` si no cabe (`416`).
fn cortar(len: usize, r: &str) -> Result<(usize, usize), ()> {
    let r = r.trim();
    let r = r.strip_prefix("bytes=").unwrap_or(r);
    let (a, b) = r.split_once('-').ok_or(())?;
    let num = |x: &str| x.parse::<usize>().map_err(|_| ());
    let (ini, fin) = if a.is_empty() {
        (len.saturating_sub(num(b)?), len)
    } else {
        let fin = if b.is_empty() {
            len
        } else {
            (num(b)? + 1).min(len)
        };
        (num(a)?, fin)
    };
    if ini >= len || ini >= fin {
        return Err(());
    }
    Ok((ini, fin))
}

/// Un trozo de lo exportado, que se lee sin copiarlo.
struct Trozo {
    bytes: Arc<Vec<u8>>,
    pos: usize,
    fin: usize,
}

impl Read for Trozo {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let resto = &self.bytes[self.pos..self.fin];
        let n = resto.len().min(buf.len());
        buf[..n].copy_from_slice(&resto[..n]);
        self.pos += n;
        Ok(n)
    }
}

fn md5(b: &[u8]) -> String {
    let mut c = Md5::default();
    c.sumar(b);
    c.texto()
}

impl Gdrive {
    /// El fichero de una clave.
    fn fichero(&self, clave: &str) -> Result<Item, String> {
        self.resolver(clave)
            .map_err(|f| format!("could not find `{clave}`: {}", f.motivo()))
    }

    /// Los bytes de una revisión de un binario (o la vigente, sin fijar).
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

    /// **Lo exportado de un nativo, fijado a un validador** (`<id>#v<version>`;
    /// vacío: el vigente): si el fichero ya no está en esa versión, `404`.
    fn exportado(&self, clave: &str, validador: &str) -> Result<(Item, Arc<Vec<u8>>), Fallo> {
        let i = self.resolver(clave)?;
        if !validador.is_empty() && i.validador() != validador.trim_matches('"') {
            return Err(Fallo::nuevo(
                404,
                "changed",
                &format!("`{clave}` is no longer at {validador}"),
            ));
        }
        let b = self.exportar(&i)?;
        Ok((i, b))
    }

    /// Un trozo de un nativo exportado, entero o por un rango.
    fn trozo(&self, clave: &str, validador: &str, rango: Option<&str>) -> Result<Vec<u8>, String> {
        let (_, b) = self
            .exportado(clave, validador)
            .map_err(|f| motivo(clave, &f))?;
        match rango {
            None => Ok(b.to_vec()),
            Some(r) => {
                let (i, f) = cortar(b.len(), r)
                    .map_err(|_| format!("the range `{r}` does not fit in `{clave}`"))?;
                Ok(b[i..f].to_vec())
            }
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
            .filter(|i| i.ruta.starts_with(prefijo))
            .map(|i| Objeto {
                etag: i.validador(),
                // Un nativo no tiene tamaño hasta que se exporta (X0 D4): el que
                // se midió, si ya se exportó en esta versión.
                tamano: self.medida(&i).map_or(i.tamano, |(t, _)| t),
                modificado: i.modificado,
                clave: i.ruta,
            })
            .collect())
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        let i = self.fichero(clave)?;
        match i.clase {
            Clase::Nativo(_) => self.trozo(clave, "", Some(rango)),
            _ => self.todo(&i.id, None, Some(&format!("bytes={rango}")), clave),
        }
    }

    /// Fijado al validador que el listado dijo (su `etag`).
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String> {
        if let Some((id, rev)) = partir(etag) {
            let l = self
                .bajar(id, Some(rev), None)
                .map_err(|f| motivo(clave, &f))?;
            return Ok(l.lector);
        }
        let (_, b) = self.exportado(clave, etag).map_err(|f| motivo(clave, &f))?;
        let fin = b.len();
        Ok(Box::new(Trozo {
            bytes: b,
            pos: 0,
            fin,
        }))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        match partir(etag) {
            Some((id, rev)) => self.todo(id, Some(rev), Some(&format!("bytes={rango}")), clave),
            None => self.trozo(clave, etag, Some(rango)),
        }
    }

    /// Un lector sólo ve la revisión actual (X0 B2): una versión por fichero.
    ///
    /// Un nativo se **exporta aquí** para saber lo que mide: `bajar` cotejará el
    /// tamaño y la huella de lo que baja con lo que `versiones` dijo, y el listado
    /// sólo sabe decir 0. La exportación queda medida, y `huella_de` no la repite.
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        let mut objetos = Origen::listar(self, prefijo)?;
        for o in objetos.iter_mut().filter(|o| partir(&o.etag).is_none()) {
            if let Ok(i) = self.fichero(&o.clave) {
                // Si no se deja exportar, queda en 0 y `bajar` lo dirá.
                let _ = self.exportar(&i);
                if let Some((t, _)) = self.medida(&i) {
                    o.tamano = t;
                }
            }
        }
        Ok(objetos
            .into_iter()
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

    /// El `md5` de esa revisión, que Drive da siempre y sin bajarla (X0 B1); el
    /// de un nativo, el de lo que se exporta (no hay otro).
    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        let i = self.fichero(clave)?;
        if !version.is_empty() && i.validador() != version.trim_matches('"') {
            return Err(format!(
                "could not inspect `{clave}` (version {version}): 404 no longer exists"
            ));
        }
        match i.clase {
            Clase::Nativo(_) => {
                if let Some((_, h)) = self.medida(&i) {
                    return Ok(Some(h));
                }
                let b = self.exportar(&i).map_err(|f| motivo(clave, &f))?;
                Ok(Some(md5(&b)))
            }
            _ => Ok(i.huella()),
        }
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String> {
        let no_se_pudo = |f: Fallo| match f.estado {
            404 => format!("could not read `{clave}` (version {version}): 404 no longer exists"),
            _ => format!(
                "could not read `{clave}` (version {version}): {}",
                f.motivo()
            ),
        };
        if let Some((id, rev)) = partir(version) {
            let l = self.bajar(id, Some(rev), None).map_err(no_se_pudo)?;
            let a = Abierto {
                tamano: l.cabecera("content-length").and_then(|v| v.parse().ok()),
                tipo: l.cabecera("content-type").map(String::from),
                huella: self
                    .fichero(clave)
                    .ok()
                    .filter(|i| i.validador() == version)
                    .and_then(|i| i.huella()),
            };
            return Ok((l.lector, a));
        }
        let (i, b) = self.exportado(clave, version).map_err(no_se_pudo)?;
        let a = Abierto {
            tamano: Some(b.len() as u64),
            tipo: match i.clase {
                Clase::Nativo(n) => Some(n.tipo.to_string()),
                _ => None,
            },
            huella: Some(md5(&b)),
        };
        let fin = b.len();
        Ok((
            Box::new(Trozo {
                bytes: b,
                pos: 0,
                fin,
            }),
            a,
        ))
    }

    fn leer_fijado(
        &self,
        clave: &str,
        version: &str,
        etag: &str,
        rango: Option<&str>,
    ) -> Result<Leido, Rechazo> {
        // El validador: la versión, o el `etag` si la versión no vino.
        let v = if version.is_empty() { etag } else { version };
        let fallo = |f: Fallo| match f.estado {
            404 => Rechazo::Cambiado(format!(
                "`{clave}` at the source is no longer the pinned version ({})",
                f.motivo()
            )),
            416 => Rechazo::Rango(format!("the range does not fit in `{clave}`")),
            0 => Rechazo::Origen(format!("the source is not responding: {}", f.cuerpo)),
            _ => Rechazo::Origen(format!("the source answered {} for `{clave}`", f.motivo())),
        };
        if let Some((id, rev)) = partir(v) {
            return self.bajar(id, Some(rev), rango).map_err(fallo);
        }
        let (i, b) = self.exportado(clave, v).map_err(fallo)?;
        let largo = b.len();
        let (ini, fin, estado) = match rango {
            None => (0, largo, 200),
            Some(r) => {
                let (a, z) = cortar(largo, r)
                    .map_err(|_| Rechazo::Rango(format!("the range does not fit in `{clave}`")))?;
                (a, z, 206)
            }
        };
        let mut cabeceras = vec![("content-length".to_string(), (fin - ini).to_string())];
        if let Clase::Nativo(n) = i.clase {
            cabeceras.push(("content-type".into(), n.tipo.into()));
        }
        if estado == 206 {
            cabeceras.push((
                "content-range".into(),
                format!("bytes {ini}-{}/{largo}", fin - 1),
            ));
        }
        Ok(Leido {
            estado,
            cabeceras,
            lector: Box::new(Trozo {
                bytes: b,
                pos: ini,
                fin,
            }),
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
        // Un nativo no es de una revisión.
        assert_eq!(partir("15-5Fy#v7"), None);
    }

    #[test]
    fn un_rango_se_corta_de_lo_exportado() {
        assert_eq!(cortar(13, "bytes=0-3"), Ok((0, 4)));
        assert_eq!(cortar(13, "0-3"), Ok((0, 4)));
        assert_eq!(cortar(13, "-4"), Ok((9, 13)));
        assert_eq!(cortar(13, "5-"), Ok((5, 13)));
        assert_eq!(cortar(13, "10-99"), Ok((10, 13)));
        assert_eq!(cortar(13, "13-20"), Err(()));
        assert_eq!(cortar(0, "0-0"), Err(()));
        assert_eq!(cortar(13, "x"), Err(()));
    }

    #[test]
    fn un_trozo_se_lee_hasta_su_fin() {
        let mut t = Trozo {
            bytes: Arc::new(b"abcdefgh".to_vec()),
            pos: 2,
            fin: 5,
        };
        let mut s = String::new();
        t.read_to_string(&mut s).unwrap();
        assert_eq!(s, "cde");
    }
}
