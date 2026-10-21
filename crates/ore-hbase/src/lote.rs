//! **De las filas de HBase a un `RecordBatch` de Arrow**. Cada columna es texto Arrow: `String` (los bytes, que
//! tienen que ser UTF-8) u `Opaque` (los bytes, en base64).
//!
//! La regla que manda, como en Phoenix: **un valor que no cabe en el tipo de su columna es un error que nombra
//! columna y fila; no se convierte en nulo ni se arregla** (0032 §4). Un texto con bytes que no son UTF-8 en una
//! columna declarada `String` es eso, y el mensaje dice que la columna debe declararse `Opaque`.

use crate::esquema::Tipo;
use arrow_array::builder::StringBuilder;
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorLote {
    /// El valor no cabe en su tipo.
    NoCabe {
        columna: String,
        fila: u64,
        detalle: String,
    },
    Arrow(String),
}

impl fmt::Display for ErrorLote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorLote::NoCabe {
                columna,
                fila,
                detalle,
            } => write!(f, "columna `{columna}`, fila {fila}: {detalle}"),
            ErrorLote::Arrow(m) => write!(f, "Arrow: {m}"),
        }
    }
}

const ALFABETO: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 estandar con relleno.
pub fn base64(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for t in b.chunks(3) {
        let n = (u32::from(t[0]) << 16)
            | (u32::from(*t.get(1).unwrap_or(&0)) << 8)
            | u32::from(*t.get(2).unwrap_or(&0));
        s.push(ALFABETO[(n >> 18) as usize & 63] as char);
        s.push(ALFABETO[(n >> 12) as usize & 63] as char);
        s.push(if t.len() > 1 {
            ALFABETO[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if t.len() > 2 {
            ALFABETO[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// Lo contrario de [`base64`]; `None` si no es base64 estandar con relleno.
pub fn de_base64(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let valor = |c: u8| ALFABETO.iter().position(|a| *a == c).map(|p| p as u32);
    let mut sal = Vec::with_capacity(b.len() / 4 * 3);
    for (i, t) in b.chunks(4).enumerate() {
        let ultimo = (i + 1) * 4 == b.len();
        let relleno = t.iter().rev().take_while(|c| **c == b'=').count();
        if relleno > 2 || (relleno > 0 && !ultimo) {
            return None;
        }
        let mut n = 0u32;
        for (j, c) in t.iter().enumerate() {
            n = (n << 6) | if j >= 4 - relleno { 0 } else { valor(*c)? };
        }
        sal.push((n >> 16) as u8);
        if relleno < 2 {
            sal.push((n >> 8) as u8);
        }
        if relleno < 1 {
            sal.push(n as u8);
        }
    }
    Some(sal)
}

/// Arma lotes de filas.
pub struct Constructor {
    nombres: Vec<String>,
    tipos: Vec<Tipo>,
    builders: Vec<StringBuilder>,
    en_lote: usize,
    total: u64,
    schema: SchemaRef,
}

impl Constructor {
    /// `(nombre de la propiedad, tipo)` por columna, en el orden de la proyeccion.
    pub fn nuevo(columnas: &[(String, Tipo)]) -> Constructor {
        let schema = Arc::new(Schema::new(
            columnas
                .iter()
                .map(|(n, _)| Field::new(n, DataType::Utf8, true))
                .collect::<Vec<_>>(),
        ));
        Constructor {
            nombres: columnas.iter().map(|(n, _)| n.clone()).collect(),
            tipos: columnas.iter().map(|(_, t)| *t).collect(),
            builders: columnas.iter().map(|_| StringBuilder::new()).collect(),
            en_lote: 0,
            total: 0,
            schema,
        }
    }

    pub fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    /// Las filas del lote en curso.
    pub fn filas(&self) -> usize {
        self.en_lote
    }

    /// Una fila: una celda por columna, `None` si no tiene valor.
    pub fn push(&mut self, celdas: &[Option<&[u8]>]) -> Result<(), ErrorLote> {
        debug_assert_eq!(celdas.len(), self.builders.len());
        let fila = self.total + 1;
        // Se comprueba la fila entera antes de escribir nada: un error no deja columnas desiguales.
        let mut textos: Vec<Option<String>> = Vec::with_capacity(celdas.len());
        for (i, c) in celdas.iter().enumerate() {
            textos.push(match (c, self.tipos[i]) {
                (None, _) => None,
                (Some(b), Tipo::Opaco) => Some(base64(b)),
                (Some(b), Tipo::Texto) => Some(
                    std::str::from_utf8(b)
                        .map_err(|e| ErrorLote::NoCabe {
                            columna: self.nombres[i].clone(),
                            fila,
                            detalle: format!(
                                "los bytes no son UTF-8 (falla en el byte {}): la columna debe declararse `Opaque`",
                                e.valid_up_to()
                            ),
                        })?
                        .to_string(),
                ),
            });
        }
        for (b, t) in self.builders.iter_mut().zip(textos) {
            match t {
                Some(t) => b.append_value(t),
                None => b.append_null(),
            }
        }
        self.en_lote += 1;
        self.total += 1;
        Ok(())
    }

    /// Cierra el lote en curso y empieza otro.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorLote> {
        let columnas: Vec<ArrayRef> = self
            .builders
            .iter_mut()
            .map(|b| Arc::new(b.finish()) as ArrayRef)
            .collect();
        self.en_lote = 0;
        RecordBatch::try_new(self.schema.clone(), columnas)
            .map_err(|e| ErrorLote::Arrow(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Array;
    use arrow_array::cast::AsArray;

    #[test]
    fn base64_coincide_con_los_vectores_del_rfc_4648() {
        for (plano, cod) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plano.as_bytes()), cod);
            assert_eq!(de_base64(cod).unwrap(), plano.as_bytes());
        }
        assert_eq!(de_base64("AAAAKg==").unwrap(), [0, 0, 0, 42]);
    }

    #[test]
    fn un_base64_mal_formado_no_se_acepta() {
        for malo in ["Zg=", "Z", "Zm9v!", "Zg==Zg==", "=Zg=", "Z==="] {
            assert!(de_base64(malo).is_none(), "{malo}");
        }
    }

    #[test]
    fn un_lote_lleva_texto_nulos_y_binarios_en_base64() {
        let mut c = Constructor::nuevo(&[
            ("id".into(), Tipo::Texto),
            ("nombre".into(), Tipo::Texto),
            ("raw".into(), Tipo::Opaco),
        ]);
        c.push(&[
            Some(b"r1"),
            Some("canción".as_bytes()),
            Some(&[0, 0, 0, 42]),
        ])
        .unwrap();
        c.push(&[Some(b"r2"), None, None]).unwrap();
        assert_eq!(c.filas(), 2);
        let b = c.finish().unwrap();
        assert_eq!(c.filas(), 0);
        assert_eq!(b.num_rows(), 2);
        assert_eq!(b.column(1).as_string::<i32>().value(0), "canción");
        assert!(b.column(1).is_null(1));
        assert_eq!(b.column(2).as_string::<i32>().value(0), "AAAAKg==");
    }

    #[test]
    fn un_texto_que_no_es_utf8_es_un_error_con_columna_y_fila_y_no_un_nulo() {
        let mut c = Constructor::nuevo(&[("id".into(), Tipo::Texto), ("n".into(), Tipo::Texto)]);
        c.push(&[Some(b"a"), Some(b"bien")]).unwrap();
        let e = c.push(&[Some(b"b"), Some(&[0xff, 0xfe])]).unwrap_err();
        let ErrorLote::NoCabe {
            columna,
            fila,
            detalle,
        } = e
        else {
            panic!("{e:?}")
        };
        assert_eq!((columna.as_str(), fila), ("n", 2));
        assert!(detalle.contains("Opaque"), "{detalle}");
        // y la fila mala no dejo columnas desiguales
        assert_eq!(c.finish().unwrap().num_rows(), 1);
    }

    #[test]
    fn una_tabla_vacia_tiene_su_esquema() {
        let mut c = Constructor::nuevo(&[("id".into(), Tipo::Texto)]);
        let b = c.finish().unwrap();
        assert_eq!(b.num_rows(), 0);
        assert_eq!(b.schema().fields()[0].name(), "id");
    }
}
