//! **Una biblioteca de SharePoint es un origen de objetos** (ADR 0061 O5·1):
//! el rasgo de `ore-objetos` sobre Graph. La versión de un fichero es
//! `<id del item>@<versión>` —el id, porque un fichero borrado y vuelto a subir
//! con el mismo nombre es otro y empieza en `1.0`—. Una versión vieja se lee por
//! su id; **la actual no se puede** (Graph), así que se lee por `/content`
//! [`Vigilado`]: si su `cTag` cambia mientras tanto, la lectura falla en vez de
//! dar otros bytes.
//!
//! `listar_versiones` da la actual de cada fichero (una petición más por
//! fichero, para saber su número): las viejas no se listan, que en una
//! biblioteca de miles de ficheros serían miles de peticiones contra el ritmo
//! del tenant. Una colección fija la que vio; si la biblioteca la recorta, es
//! `media/cambiado`.

use crate::{Clase, Fallo, Graph, Item};
use ore_objetos::huella::{Calculo, QuickXor};
use ore_objetos::{Abierto, Leido, Objeto, Origen, Rechazo, Version, memoria};
use std::io::{self, Read};

/// `<id>@<versión>` → sus dos partes.
fn partir(version: &str) -> Option<(&str, &str)> {
    version
        .rsplit_once('@')
        .filter(|(i, v)| !i.is_empty() && !v.is_empty())
}

/// Un rango por el final (`-8`) en uno con principio y fin: no se sabe si la
/// descarga de SharePoint lo entiende (sin medir), con el tamaño no hace falta.
fn absoluto(rango: &str, tamano: u64) -> String {
    match rango.strip_prefix('-').and_then(|n| n.parse::<u64>().ok()) {
        Some(n) => format!("{}-{}", tamano.saturating_sub(n), tamano.saturating_sub(1)),
        None => rango.to_string(),
    }
}

/// **La versión actual de un fichero, leída vigilada**: al terminar, se vuelve
/// a pedir el `cTag` del item; si no es el del principio (o el item ya no
/// está), la lectura falla —lo que se leyó puede ser de otra versión, o una
/// mezcla—. Entera, además, se coteja con su `quickXorHash`.
pub struct Vigilado {
    lector: Box<dyn Read + Send>,
    graph: Graph,
    id: String,
    clave: String,
    ctag: String,
    cotejo: Option<(QuickXor, String)>,
    hecho: bool,
}

impl Vigilado {
    fn nuevo(g: &Graph, i: &Item, clave: &str, lector: Box<dyn Read + Send>, entero: bool) -> Self {
        Vigilado {
            lector,
            graph: g.clone(),
            id: i.id.clone(),
            clave: clave.to_string(),
            ctag: i.ctag.clone(),
            cotejo: i
                .huella()
                .filter(|_| entero)
                .map(|h| (QuickXor::default(), h)),
            hecho: false,
        }
    }

    fn al_terminar(&mut self) -> io::Result<()> {
        let cambio =
            |m: String| io::Error::other(format!("`{}` cambió mientras se leía: {m}", self.clave));
        match self.graph.item(&self.id) {
            Ok(i) if i.ctag == self.ctag => {}
            Ok(i) => {
                return Err(cambio(format!(
                    "su cTag era {} y ahora es {}",
                    self.ctag, i.ctag
                )));
            }
            Err(f) if f.no_esta() => return Err(cambio("ya no está".into())),
            Err(f) => {
                return Err(io::Error::other(format!(
                    "no se pudo comprobar que `{}` no cambió: {}",
                    self.clave,
                    f.motivo()
                )));
            }
        }
        if let Some((q, quiere)) = &self.cotejo
            && q.texto() != *quiere
        {
            return Err(io::Error::other(format!(
                "`{}` no casa con su huella: llegó {}, SharePoint dice {quiere}",
                self.clave,
                q.texto()
            )));
        }
        Ok(())
    }
}

impl Read for Vigilado {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.lector.read(buf)?;
        if n > 0 {
            if let Some((q, _)) = &mut self.cotejo {
                q.sumar(&buf[..n]);
            }
        } else if !buf.is_empty() && !self.hecho {
            self.hecho = true;
            self.al_terminar()?;
        }
        Ok(n)
    }
}

fn leer_todo(mut r: impl Read, clave: &str) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    r.read_to_end(&mut b)
        .map_err(|e| format!("`{clave}` no se pudo leer entero: {e}"))?;
    Ok(b)
}

impl Graph {
    /// La actual de un item, vigilada: entera o un rango (`inicio-fin` o `-n`).
    pub fn abrir_actual(
        &self,
        i: &Item,
        clave: &str,
        rango: Option<&str>,
    ) -> Result<(Vigilado, Leido), Fallo> {
        let rango = rango.map(|r| format!("bytes={}", absoluto(r, i.tamano)));
        let mut l = self.bajar(&i.id, None, rango.as_deref())?;
        let lector = std::mem::replace(&mut l.lector, Box::new(io::empty()));
        Ok((Vigilado::nuevo(self, i, clave, lector, rango.is_none()), l))
    }

    /// Si `<id>@<versión>` es la actual: el item si lo es (una petición por el
    /// item y otra por sus versiones), `None` si es una vieja.
    pub fn si_es_la_actual(&self, id: &str, version: &str) -> Result<Option<Item>, Fallo> {
        let i = self.item(id)?;
        let vs = self.versiones(id)?;
        Ok(vs.first().filter(|v| v.id == version).map(|_| i))
    }

    fn por_ruta(&self, clave: &str) -> Result<Item, String> {
        self.item_por_ruta(clave).map_err(|f| {
            if f.no_esta() {
                memoria::cambio(clave)
            } else {
                format!("no se pudo mirar `{clave}`: {}", f.motivo())
            }
        })
    }
}

impl Origen for Graph {
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String> {
        Ok(Graph::listar(self, prefijo)
            .map_err(|f| format!("no se pudo listar `{prefijo}`: {}", f.motivo()))?
            .into_iter()
            .filter(|i| i.clase == Clase::Fichero)
            .map(|i| Objeto {
                clave: i.ruta,
                tamano: i.tamano,
                etag: i.ctag,
                modificado: i.modificado,
            })
            .collect())
    }

    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String> {
        let i = self.por_ruta(clave)?;
        let (v, _) = self
            .abrir_actual(&i, clave, Some(rango))
            .map_err(|f| format!("no se pudo leer `{clave}`: {}", f.motivo()))?;
        leer_todo(v, clave)
    }

    /// La que el listado dijo (`etag` es su `cTag`): si ya es otra, falla.
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String> {
        let i = self.por_ruta(clave)?;
        if i.ctag != etag {
            return Err(memoria::cambio(clave));
        }
        let (v, _) = self
            .abrir_actual(&i, clave, None)
            .map_err(|f| format!("no se pudo leer `{clave}`: {}", f.motivo()))?;
        Ok(Box::new(v))
    }

    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String> {
        let i = self.por_ruta(clave)?;
        if i.ctag != etag {
            return Err(memoria::cambio(clave));
        }
        let (v, _) = self
            .abrir_actual(&i, clave, Some(rango))
            .map_err(|f| format!("no se pudo leer `{clave}`: {}", f.motivo()))?;
        leer_todo(v, clave)
    }

    /// La actual de cada fichero, con su número de versión.
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String> {
        let items = Graph::listar(self, prefijo).map_err(|f| {
            format!(
                "no se pudieron listar las versiones de `{prefijo}`: {}",
                f.motivo()
            )
        })?;
        let mut out = Vec::new();
        for i in items.into_iter().filter(|i| i.clase == Clase::Fichero) {
            let vs = self.versiones(&i.id).map_err(|f| {
                format!(
                    "no se pudieron listar las versiones de `{}`: {}",
                    i.ruta,
                    f.motivo()
                )
            })?;
            let Some(v) = vs.first() else {
                return Err(format!("SharePoint no da ninguna versión de `{}`", i.ruta));
            };
            out.push(Version {
                version: format!("{}@{}", i.id, v.id),
                actual: true,
                marca: false,
                tamano: i.tamano,
                etag: i.ctag,
                clave: i.ruta,
                modificado: i.modificado,
            });
        }
        Ok(out)
    }

    /// El `quickXorHash` si es la actual; de una vieja, ninguna (Graph no la da).
    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String> {
        let (id, v) = partir(version).ok_or_else(|| {
            format!("`{version}` no es una versión de SharePoint (`<id>@<versión>`)")
        })?;
        match self.si_es_la_actual(id, v) {
            Ok(i) => Ok(i.and_then(|i| i.huella())),
            Err(f) if f.no_esta() => Err(memoria::cambio(clave)),
            Err(f) => Err(format!(
                "no se pudo mirar `{clave}` (versión {v}): {}",
                f.motivo()
            )),
        }
    }

    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String> {
        let (id, v) = partir(version).ok_or_else(|| {
            format!("`{version}` no es una versión de SharePoint (`<id>@<versión>`)")
        })?;
        let ya_no = |f: Fallo| {
            if f.no_esta() {
                format!("no se pudo leer `{clave}` (versión {v}): ya no está")
            } else {
                format!("no se pudo leer `{clave}` (versión {v}): {}", f.motivo())
            }
        };
        match self.si_es_la_actual(id, v).map_err(ya_no)? {
            Some(i) => {
                let (r, l) = self.abrir_actual(&i, clave, None).map_err(ya_no)?;
                let a = Abierto {
                    tamano: Some(i.tamano),
                    tipo: l.cabecera("content-type").map(String::from),
                    huella: i.huella(),
                };
                Ok((Box::new(r), a))
            }
            None => {
                let l = self.bajar(id, Some(v), None).map_err(ya_no)?;
                let a = Abierto {
                    tamano: l.cabecera("content-length").and_then(|x| x.parse().ok()),
                    tipo: l.cabecera("content-type").map(String::from),
                    huella: None,
                };
                Ok((l.lector, a))
            }
        }
    }

    /// Lo que `ore-medios` sirve de una virtual: si el `cTag` del item es el
    /// fijado (`etag`), la actual vigilada; si no, la versión por su id.
    fn leer_fijado(
        &self,
        clave: &str,
        version: &str,
        etag: &str,
        rango: Option<&str>,
    ) -> Result<Leido, Rechazo> {
        let Some((id, v)) = partir(version) else {
            return Err(Rechazo::Origen(format!(
                "`{version}` no es una versión de SharePoint (`<id>@<versión>`)"
            )));
        };
        let rechazo = |f: Fallo| match f.estado {
            404 => Rechazo::Cambiado(format!(
                "`{clave}` ya no tiene, en el origen, la versión fijada {v} ({})",
                f.motivo()
            )),
            416 => Rechazo::Rango(format!("el rango no cabe en `{clave}`")),
            0 => Rechazo::Origen(format!("el origen no contesta: {}", f.cuerpo)),
            _ => Rechazo::Origen(format!("el origen contestó {} a `{clave}`", f.motivo())),
        };
        let r = rango.and_then(|r| r.strip_prefix("bytes="));
        let actual = if etag.is_empty() {
            self.si_es_la_actual(id, v).map_err(rechazo)?
        } else {
            Some(self.item(id).map_err(rechazo)?).filter(|i| i.ctag == etag)
        };
        match actual {
            Some(i) => {
                let (lector, mut l) = self.abrir_actual(&i, clave, r).map_err(rechazo)?;
                l.lector = Box::new(lector);
                Ok(l)
            }
            None => {
                // De una vieja, el tamaño para un rango por el final está en
                // su lista de versiones.
                let r = match r {
                    Some(x) if x.starts_with('-') => {
                        let t = self
                            .versiones(id)
                            .map_err(rechazo)?
                            .into_iter()
                            .find(|x| x.id == v)
                            .ok_or_else(|| {
                                Rechazo::Cambiado(format!(
                                    "`{clave}` ya no tiene, en el origen, la versión fijada {v}"
                                ))
                            })?
                            .tamano;
                        Some(absoluto(x, t))
                    }
                    otro => otro.map(String::from),
                };
                self.bajar(id, Some(v), r.map(|x| format!("bytes={x}")).as_deref())
                    .map_err(rechazo)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_version_es_el_item_y_su_numero() {
        assert_eq!(partir("01ABC@3.0"), Some(("01ABC", "3.0")));
        assert_eq!(partir("3.0"), None);
        assert_eq!(partir("@3.0"), None);
        assert_eq!(partir("01ABC@"), None);
        assert_eq!(absoluto("-8", 100), "92-99");
        assert_eq!(absoluto("0-15", 100), "0-15");
    }
}
