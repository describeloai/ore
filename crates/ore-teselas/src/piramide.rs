//! **La pirámide** (ADR 0072 F2·3·2·1, D-F2-15): por debajo del suelo de una capa, sus teselas
//! ya hechas, en un PMTiles v3 (las construye `ore-piramide`). El motor las sirve por rango:
//! **el directorio entero en memoria** (la cabecera, la raíz y las hojas, dos oleadas al abrir y
//! una vez por pirámide) y **un GET por tesela**, por [`Paginas::traer`](crate::lector::Paginas):
//! un rango que ya está en camino no se pide dos veces y lo traído va a la caché de páginas.
//!
//! La tesela sale tal cual la escribió tippecanoe, descomprimida y con la capa renombrada a la que
//! pide el descriptor (la pirámide no sabe cómo se llamará su capa: el nombre es de la `Layer`).
//!
//! El formato, el de <https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md>; el lector
//! es el de la prueba de concepto (`pruebas-de-fuego/geo/f2/piramide/lector_pmtiles.py`,
//! validado 340/340 contra la librería oficial).

use std::io::Read;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use ore_store::almacen::Almacen;

use crate::lector::Paginas;
use crate::{Cuenta, Fallo};

/// Lo que dice la cabecera (127 bytes) y hace falta para servir.
#[derive(Debug, Clone)]
pub struct Cabecera {
    pub raiz: Range<u64>,
    pub hojas: Range<u64>,
    pub datos: u64,
    /// `true`: los directorios van en gzip; `false`: tal cual.
    pub dir_gzip: bool,
    /// `true`: las teselas van en gzip.
    pub teselas_gzip: bool,
    pub minzoom: u32,
    pub maxzoom: u32,
}

/// Una entrada de un directorio: `corrida` 0 es un directorio hoja.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entrada {
    pub id: u64,
    pub corrida: u32,
    pub largo: u32,
    pub desplaz: u64,
}

/// Una pirámide abierta: su cabecera, su raíz y todas sus hojas ya leídas.
#[derive(Debug)]
pub struct Piramide {
    pub cabecera: Cabecera,
    raiz: Vec<Entrada>,
    /// Las hojas, por su desplazamiento dentro de la sección de hojas.
    hojas: std::collections::HashMap<u64, Vec<Entrada>>,
}

/// Lo que los directorios pueden pesar en memoria por pirámide: más, y no se abre (una pirámide
/// de 1 000 M de puntos tiene unos pocos MB de directorio; 256 MB es otra cosa).
const DIRECTORIO_MAX: u64 = 256 << 20;

pub fn cabecera(b: &[u8]) -> Result<Cabecera, String> {
    if b.len() < 127 || &b[..7] != b"PMTiles" {
        return Err("no es un PMTiles".into());
    }
    if b[7] != 3 {
        return Err(format!("PMTiles v{}: el motor lee la v3", b[7]));
    }
    let u = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    let compresion = |c: u8, que: &str| match c {
        1 => Ok(false),
        2 => Ok(true),
        0 => Ok(false),
        o => Err(format!(
            "{que} con compresión {o}: el motor lee ninguna o gzip"
        )),
    };
    if b[99] != 1 {
        return Err(format!("teselas de tipo {}: el motor sirve MVT (1)", b[99]));
    }
    Ok(Cabecera {
        raiz: u(8)..u(8) + u(16),
        hojas: u(40)..u(40) + u(48),
        datos: u(56),
        dir_gzip: compresion(b[97], "los directorios")?,
        teselas_gzip: compresion(b[98], "las teselas")?,
        minzoom: b[100] as u32,
        maxzoom: b[101] as u32,
    })
}

fn varint(b: &[u8], i: &mut usize) -> Result<u64, String> {
    let (mut r, mut s) = (0u64, 0u32);
    loop {
        let c = *b.get(*i).ok_or("directorio cortado")?;
        *i += 1;
        r |= ((c & 0x7F) as u64) << s;
        if c < 0x80 {
            return Ok(r);
        }
        s += 7;
        if s > 63 {
            return Err("varint de más de 64 bits".into());
        }
    }
}

pub fn directorio(b: &[u8]) -> Result<Vec<Entrada>, String> {
    let mut i = 0;
    let n = varint(b, &mut i)? as usize;
    if n > b.len() {
        return Err("directorio con más entradas que bytes".into());
    }
    let mut e = vec![
        Entrada {
            id: 0,
            corrida: 0,
            largo: 0,
            desplaz: 0
        };
        n
    ];
    let mut ultimo = 0u64;
    for x in e.iter_mut() {
        ultimo += varint(b, &mut i)?;
        x.id = ultimo;
    }
    for x in e.iter_mut() {
        x.corrida = varint(b, &mut i)? as u32;
    }
    for x in e.iter_mut() {
        x.largo = varint(b, &mut i)? as u32;
    }
    for k in 0..n {
        let v = varint(b, &mut i)?;
        e[k].desplaz = if v == 0 && k > 0 {
            e[k - 1].desplaz + e[k - 1].largo as u64
        } else {
            v.saturating_sub(1)
        };
    }
    Ok(e)
}

/// El id de PMTiles: las teselas de los zooms anteriores + la posición en la curva de Hilbert.
pub fn tile_id(z: u32, x: u32, y: u32) -> u64 {
    let acc = ((1u64 << (2 * z)) - 1) / 3;
    let (mut x, mut y) = (x as u64, y as u64);
    let mut d = 0u64;
    let n = 1u64 << z;
    let mut s = n >> 1;
    while s > 0 {
        let rx = (x & s > 0) as u64;
        let ry = (y & s > 0) as u64;
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            // reflejar con el lado entero (`n − 1 − x`) deja los mismos bits por debajo de `s`
            // que con el del cuadrante, y no se sale de los naturales
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s >>= 1;
    }
    acc + d
}

fn buscar(e: &[Entrada], id: u64) -> Option<Entrada> {
    match e.binary_search_by_key(&id, |x| x.id) {
        Ok(i) => Some(e[i]),
        Err(0) => None,
        Err(i) => {
            let x = e[i - 1];
            // una corrida cubre ids seguidos con los mismos bytes; una hoja, los suyos
            (x.corrida == 0 || id - x.id < x.corrida as u64).then_some(x)
        }
    }
}

fn gunzip(b: &[u8]) -> Result<Vec<u8>, String> {
    let mut o = Vec::with_capacity(b.len() * 4);
    flate2::read::GzDecoder::new(b)
        .read_to_end(&mut o)
        .map_err(|e| format!("gzip: {e}"))?;
    Ok(o)
}

impl Piramide {
    /// **Abrir**: la cabecera (una oleada) y, con ella, la raíz y todas las hojas (otra). Lo que
    /// falle dice de qué pirámide es (el plazo sigue siendo plazo).
    pub fn abrir(
        paginas: &Paginas,
        cuenta: &Arc<dyn Almacen>,
        clave: &str,
        fin: Instant,
        cu: &mut Cuenta,
    ) -> Result<Piramide, Fallo> {
        Piramide::abrir_ya(paginas, cuenta, clave, fin, cu).map_err(|e| match e {
            Fallo::Plazo(m) => Fallo::Plazo(m),
            Fallo::Interno(m) | Fallo::Invalida(m) if m.starts_with("la pirámide") => {
                Fallo::Interno(m)
            }
            Fallo::Interno(m) | Fallo::Invalida(m) => {
                Fallo::Interno(format!("la pirámide `{clave}`: {m}"))
            }
        })
    }

    fn abrir_ya(
        paginas: &Paginas,
        cuenta: &Arc<dyn Almacen>,
        clave: &str,
        fin: Instant,
        cu: &mut Cuenta,
    ) -> Result<Piramide, Fallo> {
        let mal = |m: String| Fallo::Interno(format!("la pirámide `{clave}`: {m}"));
        let un_rango = |r: Range<u64>, cu: &mut Cuenta| -> Result<bytes::Bytes, Fallo> {
            let a = r.start;
            let mut t = paginas.traer(cuenta, vec![(clave.to_string(), r)], fin, cu)?;
            t.remove(clave)
                .and_then(|mut m| m.remove(&a))
                .ok_or_else(|| mal("el almacén no devolvió el rango pedido".into()))
        };
        let c = cabecera(&un_rango(0..127, cu)?).map_err(mal)?;
        if (c.raiz.end - c.raiz.start) + (c.hojas.end - c.hojas.start) > DIRECTORIO_MAX {
            return Err(mal(format!(
                "el directorio pesa más de {} MB",
                DIRECTORIO_MAX >> 20
            )));
        }
        // la raíz y las hojas, a la vez
        let mut pedidas = vec![(clave.to_string(), c.raiz.clone())];
        if c.hojas.end > c.hojas.start {
            pedidas.push((clave.to_string(), c.hojas.clone()));
        }
        let mut t = paginas.traer(cuenta, pedidas, fin, cu)?;
        let mut t = t
            .remove(clave)
            .ok_or_else(|| mal("el almacén no devolvió el directorio".into()))?;
        let des = |b: &[u8]| -> Result<Vec<u8>, Fallo> {
            if c.dir_gzip {
                gunzip(b).map_err(mal)
            } else {
                Ok(b.to_vec())
            }
        };
        let raiz_b = t
            .remove(&c.raiz.start)
            .ok_or_else(|| mal("falta la raíz".into()))?;
        let raiz = directorio(&des(&raiz_b)?).map_err(mal)?;
        let mut hojas = std::collections::HashMap::new();
        if c.hojas.end > c.hojas.start {
            let sec = t
                .remove(&c.hojas.start)
                .ok_or_else(|| mal("faltan las hojas".into()))?;
            // cada hoja que nombra un directorio (la raíz o una hoja), descomprimida una vez
            let mut pendientes: Vec<Entrada> =
                raiz.iter().filter(|e| e.corrida == 0).copied().collect();
            while let Some(e) = pendientes.pop() {
                if hojas.contains_key(&e.desplaz) {
                    continue;
                }
                let (a, z) = (e.desplaz as usize, e.desplaz as usize + e.largo as usize);
                let b = sec
                    .get(a..z)
                    .ok_or_else(|| mal("una hoja fuera de su sección".into()))?;
                let d = directorio(&des(b)?).map_err(mal)?;
                pendientes.extend(d.iter().filter(|e| e.corrida == 0));
                hojas.insert(e.desplaz, d);
            }
        }
        Ok(Piramide {
            cabecera: c,
            raiz,
            hojas,
        })
    }

    /// Dónde está la tesela en el fichero (sin red: el directorio está en memoria).
    pub fn donde(&self, z: u32, x: u32, y: u32) -> Option<Range<u64>> {
        let id = tile_id(z, x, y);
        let mut e = &self.raiz;
        for _ in 0..4 {
            let x = buscar(e, id)?;
            if x.corrida > 0 {
                let a = self.cabecera.datos + x.desplaz;
                return Some(a..a + x.largo as u64);
            }
            e = self.hojas.get(&x.desplaz)?;
        }
        None
    }

    /// Los bytes de la tesela como MVT (descomprimidos) y con la capa renombrada.
    pub fn mvt(&self, crudo: &[u8], capa: &str) -> Result<Vec<u8>, String> {
        let b = if self.cabecera.teselas_gzip {
            gunzip(crudo)?
        } else {
            crudo.to_vec()
        };
        renombrar(&b, capa)
    }
}

/// El MVT con todas sus capas llamadas `capa` (la pirámide escribe una: la de la `Layer`).
/// Sólo toca el campo 1 de cada capa (el nombre); lo demás se copia tal cual.
pub fn renombrar(mvt: &[u8], capa: &str) -> Result<Vec<u8>, String> {
    let mut o = Vec::with_capacity(mvt.len() + capa.len());
    let mut i = 0;
    while i < mvt.len() {
        let inicio = i;
        let llave = varint(mvt, &mut i)?;
        match llave & 7 {
            2 => {
                let n = varint(mvt, &mut i)? as usize;
                let cuerpo = mvt.get(i..i + n).ok_or("MVT cortado")?;
                i += n;
                if llave >> 3 == 3 {
                    let capa_b = renombrar_capa(cuerpo, capa)?;
                    poner_varint(llave, &mut o);
                    poner_varint(capa_b.len() as u64, &mut o);
                    o.extend_from_slice(&capa_b);
                } else {
                    o.extend_from_slice(&mvt[inicio..i]);
                }
            }
            0 => {
                varint(mvt, &mut i)?;
                o.extend_from_slice(&mvt[inicio..i]);
            }
            t => return Err(format!("MVT: tipo de campo {t} en la tesela")),
        }
    }
    Ok(o)
}

fn renombrar_capa(capa_b: &[u8], nombre: &str) -> Result<Vec<u8>, String> {
    let mut o = Vec::with_capacity(capa_b.len() + nombre.len());
    let mut i = 0;
    while i < capa_b.len() {
        let inicio = i;
        let llave = varint(capa_b, &mut i)?;
        match llave & 7 {
            0 => {
                varint(capa_b, &mut i)?;
            }
            1 => i += 8,
            5 => i += 4,
            2 => {
                let n = varint(capa_b, &mut i)? as usize;
                i += n;
            }
            t => return Err(format!("MVT: tipo de campo {t} en una capa")),
        }
        if i > capa_b.len() {
            return Err("MVT: capa cortada".into());
        }
        if llave == (1 << 3) | 2 {
            poner_varint(llave, &mut o);
            poner_varint(nombre.len() as u64, &mut o);
            o.extend_from_slice(nombre.as_bytes());
        } else {
            o.extend_from_slice(&capa_b[inicio..i]);
        }
    }
    Ok(o)
}

fn poner_varint(mut v: u64, o: &mut Vec<u8>) {
    while v >= 0x80 {
        o.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
    o.push(v as u8);
}
