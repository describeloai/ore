//! **Que no desaparezca ninguna fila.** Medido por el kit de conformidad (ficha 0072, N4): un `Scan` de HBase que pide
//! columnas concretas **no devuelve las filas que no tienen ninguna de ellas**. Una fila sin `nombre` no sale en un
//! `SELECT id, nombre`: ni con nulo, ni de ninguna forma. Y `nombre IS NULL` no encuentra nunca nada. Es una perdida
//! silenciosa de filas, y para una tabla de SQL una fila es una fila de HBase, tenga las celdas que tenga.
//!
//! La solucion es leer **dos flujos en paso** por cada intervalo de claves:
//!
//! - el de las **claves**: todas las filas, con `FirstKeyOnlyFilter` (barato: el servidor salta el resto de las celdas
//!   de cada fila);
//! - el de las **celdas pedidas**: solo las filas que tienen alguna.
//!
//! Se emite una fila por cada clave del primero: con sus celdas si el segundo la trae, y **sin celdas** (todo nulo) si
//! no. Los dos van ordenados por clave, asi que es una mezcla de un solo paso y sin memoria.

use ore_hbase_rpc::{Fila, Resultado};
use std::iter::Peekable;

pub struct Completa<B: Iterator<Item = Resultado<Fila>>, A: Iterator<Item = Resultado<Fila>>> {
    claves: B,
    celdas: Peekable<A>,
}

impl<B: Iterator<Item = Resultado<Fila>>, A: Iterator<Item = Resultado<Fila>>> Completa<B, A> {
    /// `claves`: todas las filas, solo con su clave. `celdas`: las filas que tienen alguna de las celdas pedidas.
    pub fn nuevo(claves: B, celdas: A) -> Self {
        Completa {
            claves,
            celdas: celdas.peekable(),
        }
    }
}

impl<B: Iterator<Item = Resultado<Fila>>, A: Iterator<Item = Resultado<Fila>>> Iterator
    for Completa<B, A>
{
    type Item = Resultado<Fila>;

    fn next(&mut self) -> Option<Self::Item> {
        let con_clave = match self.claves.next()? {
            Ok(f) => f,
            Err(e) => return Some(Err(e)),
        };
        loop {
            match self.celdas.peek() {
                // la fila tiene alguna celda de las pedidas
                Some(Ok(f)) if f.clave == con_clave.clave => return self.celdas.next(),
                // una fila que el flujo de claves no vio (se escribio entre uno y otro): no es de este paso
                Some(Ok(f)) if f.clave < con_clave.clave => {
                    self.celdas.next();
                }
                Some(Err(_)) => return self.celdas.next(),
                // la fila existe y no tiene ninguna de las celdas pedidas: todo nulo
                _ => {
                    return Some(Ok(Fila {
                        clave: con_clave.clave,
                        celdas: Vec::new(),
                    }));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_hbase_rpc::{Celda, Error};

    fn solo(k: &str) -> Resultado<Fila> {
        Ok(Fila {
            clave: k.as_bytes().to_vec(),
            celdas: vec![Celda {
                familia: b"f".to_vec(),
                cualificador: b"id".to_vec(),
                marca: 1,
                valor: vec![],
            }],
        })
    }

    fn con(k: &str, v: &str) -> Resultado<Fila> {
        Ok(Fila {
            clave: k.as_bytes().to_vec(),
            celdas: vec![Celda {
                familia: b"f".to_vec(),
                cualificador: b"n".to_vec(),
                marca: 1,
                valor: v.as_bytes().to_vec(),
            }],
        })
    }

    fn resumen(v: Vec<Resultado<Fila>>) -> Vec<(String, usize)> {
        v.into_iter()
            .map(|f| {
                let f = f.unwrap();
                (String::from_utf8(f.clave).unwrap(), f.celdas.len())
            })
            .collect()
    }

    #[test]
    fn una_fila_sin_las_celdas_pedidas_sale_con_cero_celdas_y_no_desaparece() {
        let claves = vec![solo("a"), solo("b"), solo("c"), solo("d")];
        let celdas = vec![con("a", "1"), con("c", "3")];
        let r = resumen(Completa::nuevo(claves.into_iter(), celdas.into_iter()).collect());
        assert_eq!(
            r,
            [
                ("a".into(), 1),
                ("b".into(), 0),
                ("c".into(), 1),
                ("d".into(), 0)
            ]
        );
    }

    #[test]
    fn si_ninguna_fila_tiene_las_celdas_salen_todas_vacias() {
        let claves = vec![solo("a"), solo("b")];
        let r = resumen(
            Completa::nuevo(
                claves.into_iter(),
                Vec::<Resultado<Fila>>::new().into_iter(),
            )
            .collect(),
        );
        assert_eq!(r, [("a".into(), 0), ("b".into(), 0)]);
    }

    #[test]
    fn si_todas_las_tienen_son_las_del_flujo_de_celdas() {
        let r = resumen(
            Completa::nuevo(
                vec![solo("a"), solo("b")].into_iter(),
                vec![con("a", "1"), con("b", "2")].into_iter(),
            )
            .collect(),
        );
        assert_eq!(r, [("a".into(), 1), ("b".into(), 1)]);
    }

    #[test]
    fn una_fila_del_flujo_de_celdas_que_el_de_claves_no_vio_se_salta() {
        // «aa» nacio entre las dos lecturas
        let r = resumen(
            Completa::nuevo(
                vec![solo("a"), solo("b")].into_iter(),
                vec![con("a", "1"), con("aa", "x"), con("b", "2")].into_iter(),
            )
            .collect(),
        );
        assert_eq!(r, [("a".into(), 1), ("b".into(), 1)]);
    }

    #[test]
    fn un_fallo_de_cualquiera_de_los_dos_flujos_se_propaga() {
        let falla = || Err(Error::Peticion("se cayo".into()));
        let r: Vec<_> = Completa::nuevo(
            vec![solo("a"), falla()].into_iter(),
            Vec::<Resultado<Fila>>::new().into_iter(),
        )
        .collect();
        assert!(r[0].is_ok() && r[1].is_err());
        let r: Vec<_> =
            Completa::nuevo(vec![solo("a")].into_iter(), vec![falla()].into_iter()).collect();
        assert!(r[0].is_err());
    }

    #[test]
    fn sin_claves_no_hay_filas() {
        let mut c = Completa::nuevo(
            Vec::<Resultado<Fila>>::new().into_iter(),
            vec![con("a", "1")].into_iter(),
        );
        assert!(c.next().is_none());
    }
}
