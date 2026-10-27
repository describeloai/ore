//! **De las filas de HBase a un `RecordBatch` de Arrow.** Cada columna tiene el tipo de Arrow que su tipo de OOS
//! manda (0032 §1): `Int64`, `Float64`, `Boolean`, `Decimal128(p, s)`, `Date32`, `Timestamp(us)` con zona UTC o sin
//! ella, y `Utf8` para `String` y para `Opaque` (en base64). Los valores llegan ya decodificados
//! ([`crate::codificacion`]) en la unidad en que Arrow los guarda.
//!
//! La regla que manda, como en Phoenix: **un valor que no cabe en el tipo de su columna es un error que nombra
//! columna y fila; no se convierte en nulo ni se arregla** (0032 §4). De eso se ocupa quien decodifica; aqui solo
//! se arma el lote.

use crate::esquema::Tipo;
use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use ore_arrow::{Columna, ErrorArrow, RecordBatchBuilder};
use ore_base::tipos::Valor;
pub use ore_driver::base64::{codificar as base64, descodificar as de_base64};
use std::fmt;

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

impl From<ErrorArrow> for ErrorLote {
    fn from(e: ErrorArrow) -> Self {
        ErrorLote::Arrow(e.0)
    }
}

/// Arma lotes de filas. Los valores ya llegan decodificados ([`crate::codificacion`]): armar el Arrow es de
/// `ore-arrow`, igual para todos los orígenes.
pub struct Constructor {
    lote: RecordBatchBuilder,
}

impl Constructor {
    /// `(nombre de la propiedad, tipo)` por columna, en el orden de la proyeccion.
    pub fn nuevo(columnas: &[(String, Tipo)]) -> Result<Constructor, ErrorLote> {
        let cols: Vec<Columna> = columnas
            .iter()
            .map(|(n, t)| Columna::new(n, t.fisico, t.oos.clone()))
            .collect();
        Ok(Constructor {
            lote: RecordBatchBuilder::nuevo(&cols)?,
        })
    }

    pub fn schema(&self) -> SchemaRef {
        self.lote.schema()
    }

    /// Las filas del lote en curso.
    pub fn filas(&self) -> usize {
        self.lote.filas()
    }

    /// Una fila: un valor por columna, `None` si no tiene. Todas se comprueban antes de escribir ninguna: un valor de
    /// otro fisico no deja columnas desiguales.
    pub fn push(&mut self, fila: &[Option<Valor>]) -> Result<(), ErrorLote> {
        Ok(self.lote.push(fila)?)
    }

    /// Cierra el lote en curso y empieza otro.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorLote> {
        Ok(self.lote.finish()?)
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
