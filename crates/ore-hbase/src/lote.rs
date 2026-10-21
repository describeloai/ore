//! **De las filas de HBase a un `RecordBatch` de Arrow.** Cada columna tiene el tipo de Arrow que su tipo de OOS
//! manda (0032 §1): `Int64`, `Float64`, `Boolean`, `Decimal128(p, s)`, `Date32`, `Timestamp(us)` con zona UTC o sin
//! ella, y `Utf8` para `String` y para `Opaque` (en base64). Los valores llegan ya decodificados
//! ([`crate::codificacion`]) en la unidad en que Arrow los guarda.
//!
//! La regla que manda, como en Phoenix: **un valor que no cabe en el tipo de su columna es un error que nombra
//! columna y fila; no se convierte en nulo ni se arregla** (0032 §4). De eso se ocupa quien decodifica; aqui solo
//! se arma el lote.

use crate::esquema::Tipo;
use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use ore_core::tipos::{Fisico, Valor};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorLote {
    /// El valor no cabe en su tipo. `clave` es la de la fila, en texto, porque es lo que sirve para ir a mirarla.
    NoCabe {
        columna: String,
        clave: String,
        detalle: String,
    },
    Arrow(String),
}

impl fmt::Display for ErrorLote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorLote::NoCabe {
                columna,
                clave,
                detalle,
            } => write!(
                f,
                "columna `{columna}`, fila con clave `{clave}`: {detalle}"
            ),
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

enum Col {
    Entero(Int64Builder),
    Real(Float64Builder),
    Decimal(Decimal128Builder),
    Logico(BooleanBuilder),
    Instante(TimestampMicrosecondBuilder),
    Fecha(Date32Builder),
    FechaHora(TimestampMicrosecondBuilder),
    Hora(Time64MicrosecondBuilder),
    Texto(StringBuilder),
}

/// Arma lotes de filas.
pub struct Constructor {
    nombres: Vec<String>,
    cols: Vec<Col>,
    en_lote: usize,
    schema: SchemaRef,
}

impl Constructor {
    /// `(nombre de la propiedad, tipo)` por columna, en el orden de la proyeccion.
    pub fn nuevo(columnas: &[(String, Tipo)]) -> Result<Constructor, ErrorLote> {
        let mut cols = Vec::new();
        let mut campos = Vec::new();
        for (nombre, tipo) in columnas {
            let (dt, col) = match tipo.fisico {
                Fisico::Entero => (DataType::Int64, Col::Entero(Int64Builder::new())),
                Fisico::Real => (DataType::Float64, Col::Real(Float64Builder::new())),
                Fisico::Decimal { precision, escala } => (
                    DataType::Decimal128(precision, escala as i8),
                    Col::Decimal(
                        Decimal128Builder::new()
                            .with_precision_and_scale(precision, escala as i8)
                            .map_err(|e| ErrorLote::Arrow(e.to_string()))?,
                    ),
                ),
                Fisico::Logico => (DataType::Boolean, Col::Logico(BooleanBuilder::new())),
                Fisico::Instante => (
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    Col::Instante(TimestampMicrosecondBuilder::new().with_timezone("UTC")),
                ),
                Fisico::Fecha => (DataType::Date32, Col::Fecha(Date32Builder::new())),
                Fisico::FechaHora => (
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    Col::FechaHora(TimestampMicrosecondBuilder::new()),
                ),
                Fisico::Hora => (
                    DataType::Time64(TimeUnit::Microsecond),
                    Col::Hora(Time64MicrosecondBuilder::new()),
                ),
                Fisico::Texto => (DataType::Utf8, Col::Texto(StringBuilder::new())),
            };
            let meta: HashMap<String, String> = [("ore:tipo".to_string(), tipo.oos.clone())].into();
            campos.push(Field::new(nombre, dt, true).with_metadata(meta));
            cols.push(col);
        }
        Ok(Constructor {
            nombres: columnas.iter().map(|(n, _)| n.clone()).collect(),
            cols,
            en_lote: 0,
            schema: Arc::new(Schema::new(campos)),
        })
    }

    pub fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    /// Las filas del lote en curso.
    pub fn filas(&self) -> usize {
        self.en_lote
    }

    /// Una fila: un valor por columna, `None` si no tiene. Todas se comprueban antes de escribir ninguna: un valor de
    /// otro fisico no deja columnas desiguales.
    pub fn push(&mut self, fila: &[Option<Valor>]) -> Result<(), ErrorLote> {
        if fila.len() != self.cols.len() {
            return Err(ErrorLote::Arrow(format!(
                "la fila trae {} valores y el lote {} columnas",
                fila.len(),
                self.cols.len()
            )));
        }
        for (i, (c, v)) in self.cols.iter().zip(fila).enumerate() {
            let bien = matches!(
                (c, v),
                (_, None)
                    | (Col::Entero(_), Some(Valor::Entero(_)))
                    | (Col::Real(_), Some(Valor::Real(_)))
                    | (Col::Decimal(_), Some(Valor::Decimal(_)))
                    | (Col::Logico(_), Some(Valor::Logico(_)))
                    | (Col::Instante(_), Some(Valor::Instante(_)))
                    | (Col::Fecha(_), Some(Valor::Fecha(_)))
                    | (Col::FechaHora(_), Some(Valor::FechaHora(_)))
                    | (Col::Hora(_), Some(Valor::Hora(_)))
                    | (Col::Texto(_), Some(Valor::Texto(_)))
            );
            if !bien {
                return Err(ErrorLote::Arrow(format!(
                    "la columna `{}` recibio un valor de otro fisico",
                    self.nombres[i]
                )));
            }
        }
        for (c, v) in self.cols.iter_mut().zip(fila) {
            match (c, v) {
                (Col::Entero(b), Some(Valor::Entero(x))) => b.append_value(*x),
                (Col::Real(b), Some(Valor::Real(x))) => b.append_value(*x),
                (Col::Decimal(b), Some(Valor::Decimal(x))) => b.append_value(*x),
                (Col::Logico(b), Some(Valor::Logico(x))) => b.append_value(*x),
                (Col::Instante(b), Some(Valor::Instante(x))) => b.append_value(*x),
                (Col::Fecha(b), Some(Valor::Fecha(x))) => b.append_value(*x),
                (Col::FechaHora(b), Some(Valor::FechaHora(x))) => b.append_value(*x),
                (Col::Hora(b), Some(Valor::Hora(x))) => b.append_value(*x),
                (Col::Texto(b), Some(Valor::Texto(x))) => b.append_value(x),
                (Col::Entero(b), None) => b.append_null(),
                (Col::Real(b), None) => b.append_null(),
                (Col::Decimal(b), None) => b.append_null(),
                (Col::Logico(b), None) => b.append_null(),
                (Col::Instante(b), None) => b.append_null(),
                (Col::Fecha(b), None) => b.append_null(),
                (Col::FechaHora(b), None) => b.append_null(),
                (Col::Hora(b), None) => b.append_null(),
                (Col::Texto(b), None) => b.append_null(),
                _ => unreachable!("comprobado arriba"),
            }
        }
        self.en_lote += 1;
        Ok(())
    }

    /// Cierra el lote en curso y empieza otro.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorLote> {
        let columnas: Vec<ArrayRef> = self
            .cols
            .iter_mut()
            .map(|c| -> ArrayRef {
                match c {
                    Col::Entero(b) => Arc::new(b.finish()),
                    Col::Real(b) => Arc::new(b.finish()),
                    Col::Decimal(b) => Arc::new(b.finish()),
                    Col::Logico(b) => Arc::new(b.finish()),
                    Col::Instante(b) => Arc::new(b.finish()),
                    Col::Fecha(b) => Arc::new(b.finish()),
                    Col::FechaHora(b) => Arc::new(b.finish()),
                    Col::Hora(b) => Arc::new(b.finish()),
                    Col::Texto(b) => Arc::new(b.finish()),
                }
            })
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
    use arrow_array::types::{Date32Type, Decimal128Type, Int64Type, TimestampMicrosecondType};

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

    fn tipo(oos: &str, cod: &str) -> Tipo {
        Tipo::nuevo(Some(oos), Some(cod), "e.t", "c").unwrap()
    }

    #[test]
    fn cada_tipo_de_oos_sale_con_su_tipo_de_arrow_y_sus_metadatos() {
        let c = Constructor::nuevo(&[
            ("s".into(), Tipo::texto()),
            ("o".into(), Tipo::opaco()),
            ("i".into(), tipo("Integer", "i32be")),
            ("f".into(), tipo("Float", "f64be")),
            ("b".into(), tipo("Boolean", "bool")),
            ("d".into(), tipo("Decimal<12, 2>", "decimal")),
            ("fe".into(), tipo("Date", "epoch-ms")),
            ("fh".into(), tipo("DateTime", "epoch-ms")),
            ("in".into(), tipo("DateTimeTz", "epoch-ms")),
        ])
        .unwrap();
        let tipos: Vec<_> = c
            .schema()
            .fields()
            .iter()
            .map(|f| f.data_type().to_string())
            .collect();
        assert_eq!(
            tipos,
            [
                "Utf8",
                "Utf8",
                "Int64",
                "Float64",
                "Boolean",
                "Decimal128(12, 2)",
                "Date32",
                "Timestamp(µs)",
                "Timestamp(µs, \"UTC\")"
            ]
        );
        assert_eq!(c.schema().field(5).metadata()["ore:tipo"], "Decimal<12, 2>");
        assert_eq!(c.schema().field(1).metadata()["ore:tipo"], "Opaque");
    }

    #[test]
    fn un_lote_lleva_valores_tipados_y_nulos() {
        let mut c = Constructor::nuevo(&[
            ("s".into(), Tipo::texto()),
            ("i".into(), tipo("Integer", "i64be")),
            ("d".into(), tipo("Decimal<12, 2>", "decimal")),
            ("fe".into(), tipo("Date", "epoch-ms")),
            ("in".into(), tipo("DateTimeTz", "epoch-ms")),
        ])
        .unwrap();
        c.push(&[
            Some(Valor::Texto("hola".into())),
            Some(Valor::Entero(-7)),
            Some(Valor::Decimal(-12_345)),
            Some(Valor::Fecha(19_675)),
            Some(Valor::Instante(1_700_000_000_123_000)),
        ])
        .unwrap();
        c.push(&[None, None, None, None, None]).unwrap();
        assert_eq!(c.filas(), 2);
        let b = c.finish().unwrap();
        assert_eq!(c.filas(), 0);
        assert_eq!(b.column(0).as_string::<i32>().value(0), "hola");
        assert_eq!(b.column(1).as_primitive::<Int64Type>().value(0), -7);
        assert_eq!(
            b.column(2).as_primitive::<Decimal128Type>().value(0),
            -12_345
        );
        assert_eq!(b.column(3).as_primitive::<Date32Type>().value(0), 19_675);
        assert_eq!(
            b.column(4)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_700_000_000_123_000
        );
        for i in 0..5 {
            assert!(b.column(i).is_null(1), "columna {i}");
        }
    }

    #[test]
    fn un_valor_de_otro_fisico_es_un_error_y_no_deja_columnas_desiguales() {
        let mut c = Constructor::nuevo(&[
            ("a".into(), Tipo::texto()),
            ("n".into(), tipo("Integer", "i32be")),
        ])
        .unwrap();
        c.push(&[Some(Valor::Texto("x".into())), Some(Valor::Entero(1))])
            .unwrap();
        let e = c
            .push(&[
                Some(Valor::Texto("y".into())),
                Some(Valor::Texto("no es un entero".into())),
            ])
            .unwrap_err();
        assert!(
            matches!(e, ErrorLote::Arrow(ref m) if m.contains("`n`")),
            "{e:?}"
        );
        assert_eq!(c.finish().unwrap().num_rows(), 1);
    }

    #[test]
    fn una_tabla_vacia_tiene_su_esquema() {
        let mut c = Constructor::nuevo(&[("id".into(), Tipo::texto())]).unwrap();
        let b = c.finish().unwrap();
        assert_eq!(b.num_rows(), 0);
        assert_eq!(b.schema().fields()[0].name(), "id");
    }
}
