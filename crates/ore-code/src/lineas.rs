//! De un desplazamiento en bytes a línea y columna.

use crate::firma::Rango;

/// Los comienzos de línea de un fuente.
pub struct Lineas<'a> {
    fuente: &'a str,
    comienzos: Vec<u32>,
}

impl<'a> Lineas<'a> {
    pub fn new(fuente: &'a str) -> Self {
        let mut comienzos = vec![0];
        for (i, b) in fuente.bytes().enumerate() {
            if b == b'\n' {
                comienzos.push(i as u32 + 1);
            }
        }
        Lineas { fuente, comienzos }
    }

    /// `(línea, columna)`, las dos desde 1. La columna cuenta caracteres, no
    /// bytes: es lo que un editor enseña.
    pub fn posicion(&self, byte: u32) -> (u32, u32) {
        let i = self.comienzos.partition_point(|&c| c <= byte) - 1;
        let inicio = self.comienzos[i] as usize;
        let fin = (byte as usize).min(self.fuente.len());
        let columna = self
            .fuente
            .get(inicio..fin)
            .map_or(fin - inicio, |s| s.chars().count());
        (i as u32 + 1, columna as u32 + 1)
    }

    pub fn de(&self, r: Rango) -> (u32, u32) {
        self.posicion(r.inicio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuenta_caracteres_y_no_bytes() {
        let l = Lineas::new("a\nñandú = 1\n");
        assert_eq!(l.posicion(0), (1, 1));
        assert_eq!(l.posicion(2), (2, 1));
        // `=` está detrás de «ñandú » (6 caracteres, 8 bytes)
        assert_eq!(l.posicion(2 + 8), (2, 7));
    }
}
