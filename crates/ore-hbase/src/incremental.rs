//! **El incremental de HBase** (ficha 0072, N5): `start`/`end` sobre la columna sintetica `_ts`, la marca de tiempo de
//! la fila. Toda tabla de HBase tiene marca en cada celda, asi que el incremental no pide ninguna columna al usuario.
//!
//! # Lo que mide y lo que no
//!
//! - `start` es **exclusivo** y `end` **inclusivo** (0016): el rango de HBase es `[start + 1, end + 1)`.
//! - El `testigo` es un **limite**, no un maximo: `reloj − rezago`. Una lectura con `end = T` entrega todo lo que tenia
//!   marca ≤ T, asi que el testigo despues de leerla es `T` sin buscar el maximo (HBase no tiene `ORDER BY` por marca).
//!   Una fila con la marca en el futuro no envenena nada: no entra hasta que el reloj la alcanza.
//! - **Ve las actualizaciones** (una celda nueva tiene marca nueva) y **no ve los borrados** (no queda celda que
//!   señale). Un refresco completo si los refleja.
//!
//! # Los dos pasos
//!
//! Un `Scan` con rango de tiempo devuelve **solo las celdas dentro del rango**: una fila con una celda actualizada
//! llegaria con una sola columna. Por eso son dos pasos, [`Cambiadas`]:
//!
//! 1. un scan con el rango de tiempo y `FirstKeyOnlyFilter`: las **claves** de las filas con alguna celda en el rango;
//! 2. por tandas de claves, un scan con [`ore_hbase_rpc::filtro_de_filas`]: las filas **completas** en su ultima
//!    version, de una sola pasada y con los reintentos del lector. Una fila que se borro entre los dos pasos no sale.
//!
//! El paso 2 puede traer una marca mayor que el `end` (la fila cambio entre medias): es idempotente, porque el cambio
//! es un `upsert` por clave.

use ore_driver::{Fallo, Peticion};
use ore_hbase_rpc::{Cancelacion, Cliente, Consulta, Fila, Resultado, filtro_de_filas};
use std::collections::VecDeque;

use crate::esquema::MARCA;
use crate::lectura::{FILAS_POR_LLAMADA, SOLO_PRIMERA_CELDA};

/// La mayor marca de tiempo que HBase admite: `Long.MAX_VALUE`.
const MAX_MARCA: u64 = i64::MAX as u64;

/// Claves por tanda del paso 2: acota la memoria y el tamano del filtro.
pub const CLAVES_POR_TANDA: usize = 1000;

/// El rango de tiempo de celda `[desde, hasta)` que pide el incremental.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rango {
    pub desde: u64,
    /// `None`: hasta donde haya.
    pub hasta: Option<u64>,
}

impl Rango {
    /// El rango de HBase: sin tope es «hasta el final de los tiempos». **Un `long` de Java con signo**: medido contra
    /// un servidor de verdad, un tope por encima de `Long.MAX_VALUE` llega como −1 y HBase contesta `Timestamp cannot
    /// be negative` (un `IOException` que el cliente, como cualquiera, reintenta).
    pub fn tiempo(&self) -> (u64, u64) {
        (
            self.desde.min(MAX_MARCA),
            self.hasta.map_or(MAX_MARCA, |h| h.min(MAX_MARCA)),
        )
    }

    /// Un rango que no deja pasar nada (`start >= end`): no se pregunta, porque HBase lo rechaza.
    pub fn vacio(&self) -> bool {
        let (d, h) = self.tiempo();
        d >= h
    }
}

fn marca(valor: &str, que: &str) -> Result<u64, Fallo> {
    valor.trim().parse::<u64>().map_err(|_| {
        Fallo::operador(format!(
            "`{que}={valor}`: el rango de `{MARCA}` son milisegundos desde la epoca (un entero), \
             que es lo que dice el `testigo`"
        ))
    })
}

/// El rango que la peticion pide, o por que no se sirve. Sin `start` ni `end` no hay rango.
pub fn rango_de(p: &Peticion) -> Result<Option<Rango>, Fallo> {
    if p.start.is_none() && p.end.is_none() {
        return Ok(None);
    }
    if let Some(porque) = ore_driver::rango_servible(p, true, false) {
        return Err(Fallo::operador(porque));
    }
    match p.cursor.as_deref() {
        Some(MARCA) => {}
        Some(otro) => {
            return Err(Fallo::operador(format!(
                "un rango sobre `{otro}` de `{}`: HBase solo sabe avanzar por la marca de tiempo de la fila \
                 (`{MARCA}`); sobre otra columna seria un barrido entero que se llama incremental",
                p.objeto
            )));
        }
        None => unreachable!("rango_servible rechaza un rango sin cursor"),
    }
    let desde = match &p.start {
        Some(s) => marca(s, "start")?.saturating_add(1),
        None => 0,
    };
    let hasta = match &p.end {
        Some(e) => Some(marca(e, "end")?.saturating_add(1)),
        None => None,
    };
    Ok(Some(Rango { desde, hasta }))
}

/// Las filas que cambiaron en un intervalo de claves, completas. Es un iterador de `Resultado<Fila>` como el de un
/// `Lector`: despues de un error no entrega mas.
pub struct Cambiadas<'a> {
    cliente: &'a Cliente,
    tabla: &'a str,
    token: &'a Cancelacion,
    claves: Box<dyn Iterator<Item = Resultado<Fila>> + 'a>,
    /// `false`: basta con las claves (no se pide ninguna celda ni la marca).
    filas: bool,
    listas: VecDeque<Fila>,
    acabado: bool,
}

impl<'a> Cambiadas<'a> {
    /// `intervalo`: las claves `[inicio, fin)` a mirar; `rango`: el tiempo; `filas`: si hay que traer las filas.
    pub fn nuevo(
        cliente: &'a Cliente,
        tabla: &'a str,
        token: &'a Cancelacion,
        (inicio, fin): (&[u8], &[u8]),
        rango: &Rango,
        filas: bool,
    ) -> Resultado<Self> {
        let paso1 = Consulta {
            inicio: inicio.to_vec(),
            fin: fin.to_vec(),
            tiempo: Some(rango.tiempo()),
            filas_por_llamada: FILAS_POR_LLAMADA,
            filtro: Some((SOLO_PRIMERA_CELDA.to_string(), Vec::new())),
            ..Consulta::default()
        };
        Ok(Cambiadas {
            cliente,
            tabla,
            token,
            claves: Box::new(cliente.leer(tabla, &paso1, token)?),
            filas,
            listas: VecDeque::new(),
            acabado: false,
        })
    }

    /// El paso 2 de una tanda de claves ordenadas.
    fn traer(&self, claves: &[Vec<u8>]) -> Resultado<Vec<Fila>> {
        let ultima = claves.last().expect("una tanda no vacia");
        let consulta = Consulta {
            inicio: claves[0].clone(),
            fin: [ultima.as_slice(), &[0]].concat(),
            filtro: Some(filtro_de_filas(claves)),
            filas_por_llamada: CLAVES_POR_TANDA as u32,
            ..Consulta::default()
        };
        self.cliente
            .leer(self.tabla, &consulta, self.token)?
            .collect()
    }
}

impl Iterator for Cambiadas<'_> {
    type Item = Resultado<Fila>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(f) = self.listas.pop_front() {
                return Some(Ok(f));
            }
            if self.acabado {
                return None;
            }
            let mut tanda: Vec<Fila> = Vec::new();
            for fila in self.claves.by_ref() {
                match fila {
                    Ok(f) => tanda.push(f),
                    Err(e) => {
                        self.acabado = true;
                        return Some(Err(e));
                    }
                }
                if tanda.len() >= CLAVES_POR_TANDA {
                    break;
                }
            }
            if tanda.len() < CLAVES_POR_TANDA {
                self.acabado = true;
            }
            if tanda.is_empty() {
                return None;
            }
            if !self.filas {
                self.listas.extend(tanda);
                continue;
            }
            let claves: Vec<Vec<u8>> = tanda.into_iter().map(|f| f.clave).collect();
            match self.traer(&claves) {
                Ok(filas) => self.listas.extend(filas),
                Err(e) => {
                    self.acabado = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(start: Option<&str>, end: Option<&str>, cursor: Option<&str>) -> Peticion {
        Peticion {
            objeto: "default.t".into(),
            start: start.map(String::from),
            end: end.map(String::from),
            cursor: cursor.map(String::from),
            ..Peticion::default()
        }
    }

    #[test]
    fn start_es_exclusivo_y_end_inclusivo() {
        let r = rango_de(&p(Some("100"), Some("200"), Some("_ts")))
            .unwrap()
            .unwrap();
        assert_eq!(r.tiempo(), (101, 201));
        let r = rango_de(&p(None, Some("0"), Some("_ts"))).unwrap().unwrap();
        assert_eq!(r.tiempo(), (0, 1));
        let r = rango_de(&p(Some("5"), None, Some("_ts"))).unwrap().unwrap();
        assert_eq!(r.tiempo(), (6, i64::MAX as u64));
    }

    #[test]
    fn sin_start_ni_end_no_hay_rango() {
        assert_eq!(rango_de(&p(None, None, Some("_ts"))).unwrap(), None);
        assert_eq!(rango_de(&p(None, None, None)).unwrap(), None);
    }

    #[test]
    fn un_rango_vacio_no_se_pregunta() {
        let r = rango_de(&p(Some("200"), Some("200"), Some("_ts")))
            .unwrap()
            .unwrap();
        assert!(r.vacio());
        let r = rango_de(&p(Some("100"), Some("200"), Some("_ts")))
            .unwrap()
            .unwrap();
        assert!(!r.vacio());
    }

    #[test]
    fn lo_que_no_es_la_marca_se_rechaza_diciendo_por_que() {
        for (s, e, c) in [
            (Some("1"), None, None),
            (Some("1"), None, Some("f:fecha")),
            (Some("ayer"), None, Some("_ts")),
            (None, Some("2026-10-21"), Some("_ts")),
            (Some("-5"), None, Some("_ts")),
        ] {
            let m = rango_de(&p(s, e, c)).unwrap_err();
            assert_eq!(m.codigo, ore_driver::Codigo::Operador, "{s:?} {e:?} {c:?}");
        }
        let m = rango_de(&p(Some("1"), None, Some("f:fecha")))
            .unwrap_err()
            .mensaje;
        assert!(m.contains("_ts") && m.contains("f:fecha"), "{m}");
    }

    #[test]
    fn el_extremo_del_tiempo_no_desborda() {
        let r = rango_de(&p(Some("18446744073709551615"), None, Some("_ts")))
            .unwrap()
            .unwrap();
        assert_eq!(r.desde, u64::MAX);
        // pero a HBase le llega como un long con signo, y un rango que empieza en el fin de los tiempos no deja nada
        assert_eq!(r.tiempo(), (i64::MAX as u64, i64::MAX as u64));
        assert!(r.vacio());
    }
}
