//! **Una columna entera de una vez**: de los valores de un físico (o de textos) a su array de Arrow.
//!
//! Es lo que necesita quien ya tiene la columna analizada y no una fila a la vez: `ore-store::carga::lote`
//! analiza una columna entera (todo o nada, ADR 0032) y después la arma. La capacidad va reservada desde el
//! principio y cada físico tiene su propio bucle: sin comprobar la variante en cada celda.
//!
//! El tipo es **el del contrato del lago**: un instante en [`UTC`] (`+00:00`), un decimal con su precisión y escala.

use crate::{ErrorArrow, UTC};
use arrow_array::ArrayRef;
use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder,
    StringBuilder, Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use ore_base::tipos::{Fisico, Valor};
use std::sync::Arc;

/// Los bytes que se reservan por texto o binario cuando no se sabe su tamaño.
const BYTES_POR_CELDA: usize = 16;

fn ajeno(f: &Fisico, i: usize) -> ErrorArrow {
    ErrorArrow(format!("el valor {i} no es de la columna ({})", f.nombre()))
}

/// La columna de Arrow de un físico, con estos valores (`None` es nulo), en el mismo orden.
///
/// Un valor que no es del físico es un error y no deja nada a medias: quien llama ya analizó la columna, así que
/// eso es un fallo suyo, no un dato.
pub fn columna_de_valores(
    fisico: &Fisico,
    valores: &[Option<Valor>],
) -> Result<ArrayRef, ErrorArrow> {
    let n = valores.len();
    macro_rules! primitiva {
        ($b:expr, $variante:path) => {{
            let mut b = $b;
            for (i, v) in valores.iter().enumerate() {
                match v {
                    None => b.append_null(),
                    Some($variante(x)) => b.append_value(*x),
                    Some(_) => return Err(ajeno(fisico, i)),
                }
            }
            Arc::new(b.finish()) as ArrayRef
        }};
    }
    Ok(match fisico {
        Fisico::Entero => primitiva!(Int64Builder::with_capacity(n), Valor::Entero),
        Fisico::Real => primitiva!(Float64Builder::with_capacity(n), Valor::Real),
        Fisico::Logico => primitiva!(BooleanBuilder::with_capacity(n), Valor::Logico),
        Fisico::Decimal { precision, escala } => primitiva!(
            Decimal128Builder::with_capacity(n)
                .with_precision_and_scale(*precision, *escala as i8)
                .map_err(|e| ErrorArrow(e.to_string()))?,
            Valor::Decimal
        ),
        Fisico::Fecha => primitiva!(Date32Builder::with_capacity(n), Valor::Fecha),
        Fisico::Hora => primitiva!(Time64MicrosecondBuilder::with_capacity(n), Valor::Hora),
        Fisico::FechaHora => primitiva!(
            TimestampMicrosecondBuilder::with_capacity(n),
            Valor::FechaHora
        ),
        Fisico::Instante => primitiva!(
            TimestampMicrosecondBuilder::with_capacity(n).with_timezone(UTC),
            Valor::Instante
        ),
        Fisico::Texto => {
            let mut b = StringBuilder::with_capacity(n, n * BYTES_POR_CELDA);
            for (i, v) in valores.iter().enumerate() {
                match v {
                    None => b.append_null(),
                    Some(Valor::Texto(x)) => b.append_value(x),
                    Some(_) => return Err(ajeno(fisico, i)),
                }
            }
            Arc::new(b.finish())
        }
        Fisico::Binario => {
            let mut b = BinaryBuilder::with_capacity(n, n * BYTES_POR_CELDA);
            for (i, v) in valores.iter().enumerate() {
                match v {
                    None => b.append_null(),
                    Some(Valor::Bytes(x)) => b.append_value(x),
                    Some(_) => return Err(ajeno(fisico, i)),
                }
            }
            Arc::new(b.finish())
        }
    })
}

/// Una columna de texto desde los textos (`None` es nulo): sin pasar por un `Valor` por celda.
pub fn columna_de_textos<'a>(textos: impl ExactSizeIterator<Item = Option<&'a str>>) -> ArrayRef {
    let n = textos.len();
    let mut b = StringBuilder::with_capacity(n, n * BYTES_POR_CELDA);
    for t in textos {
        match t {
            Some(t) => b.append_value(t),
            None => b.append_null(),
        }
    }
    Arc::new(b.finish())
}

/// Una columna binaria desde los bytes (`None` es nulo), por ejemplo el WKB ya validado de una geometría.
pub fn columna_de_bytes<'a>(bytes: impl ExactSizeIterator<Item = Option<&'a [u8]>>) -> ArrayRef {
    let n = bytes.len();
    let mut b = BinaryBuilder::with_capacity(n, n * BYTES_POR_CELDA);
    for x in bytes {
        match x {
            Some(x) => b.append_value(x),
            None => b.append_null(),
        }
    }
    Arc::new(b.finish())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::tipo_fisico;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Date32Type, Decimal128Type, Float64Type, Int64Type};
    use arrow_array::{Array, RecordBatch};

    /// Un valor de cada físico, un nulo, y otro valor.
    fn casos() -> Vec<(Fisico, Vec<Option<Valor>>)> {
        vec![
            (
                Fisico::Entero,
                vec![Some(Valor::Entero(i64::MIN)), None, Some(Valor::Entero(7))],
            ),
            (
                Fisico::Real,
                vec![Some(Valor::Real(-0.0)), None, Some(Valor::Real(1e300))],
            ),
            (
                Fisico::Logico,
                vec![Some(Valor::Logico(true)), None, Some(Valor::Logico(false))],
            ),
            (
                Fisico::Decimal {
                    precision: 12,
                    escala: 2,
                },
                vec![
                    Some(Valor::Decimal(-5)),
                    None,
                    Some(Valor::Decimal(999_999_999_999)),
                ],
            ),
            (
                Fisico::Fecha,
                vec![Some(Valor::Fecha(-1)), None, Some(Valor::Fecha(20_751))],
            ),
            (
                Fisico::Hora,
                vec![
                    Some(Valor::Hora(0)),
                    None,
                    Some(Valor::Hora(86_399_999_999)),
                ],
            ),
            (
                Fisico::FechaHora,
                vec![Some(Valor::FechaHora(1)), None, Some(Valor::FechaHora(-1))],
            ),
            (
                Fisico::Instante,
                vec![
                    Some(Valor::Instante(1_792_914_255_123_456)),
                    None,
                    Some(Valor::Instante(0)),
                ],
            ),
            (
                Fisico::Texto,
                vec![
                    Some(Valor::Texto("ñandú".into())),
                    None,
                    Some(Valor::Texto(String::new())),
                ],
            ),
            (
                Fisico::Binario,
                vec![
                    Some(Valor::Bytes(vec![0xde, 0xad])),
                    None,
                    Some(Valor::Bytes(vec![])),
                ],
            ),
        ]
    }

    #[test]
    fn cada_fisico_da_su_tipo_del_contrato_y_conserva_nulos_y_vacios() {
        for (f, valores) in casos() {
            let a = columna_de_valores(&f, &valores).unwrap();
            assert_eq!(a.data_type(), &tipo_fisico(&f), "el tipo de {}", f.nombre());
            assert_eq!(a.len(), 3);
            assert!(
                a.is_null(1) && !a.is_null(0) && !a.is_null(2),
                "{}",
                f.nombre()
            );
        }
        // Lo que distingue el vacío del nulo.
        let t =
            columna_de_valores(&Fisico::Texto, &[Some(Valor::Texto(String::new())), None]).unwrap();
        assert_eq!((t.as_string::<i32>().value(0), t.is_null(1)), ("", true));
    }

    #[test]
    fn un_instante_lleva_la_zona_del_lago_y_un_decimal_su_escala() {
        let i = columna_de_valores(&Fisico::Instante, &[Some(Valor::Instante(1))]).unwrap();
        assert_eq!(
            i.data_type(),
            &arrow_schema::DataType::Timestamp(
                arrow_schema::TimeUnit::Microsecond,
                Some("+00:00".into())
            )
        );
        let d = columna_de_valores(
            &Fisico::Decimal {
                precision: 12,
                escala: 2,
            },
            &[Some(Valor::Decimal(12_345))],
        )
        .unwrap();
        assert_eq!(d.data_type(), &arrow_schema::DataType::Decimal128(12, 2));
        assert_eq!(
            d.as_primitive::<Decimal128Type>().value_as_string(0),
            "123.45"
        );
    }

    #[test]
    fn los_valores_de_cada_fisico_salen_intactos() {
        let e = columna_de_valores(
            &Fisico::Entero,
            &[Some(Valor::Entero(i64::MIN)), None, Some(Valor::Entero(7))],
        )
        .unwrap();
        assert_eq!(e.as_primitive::<Int64Type>().value(0), i64::MIN);
        let r = columna_de_valores(&Fisico::Real, &[Some(Valor::Real(-0.0))]).unwrap();
        assert!(
            r.as_primitive::<Float64Type>().value(0).is_sign_negative(),
            "-0.0 sigue siendo -0.0"
        );
        let f = columna_de_valores(&Fisico::Fecha, &[Some(Valor::Fecha(-1))]).unwrap();
        assert_eq!(f.as_primitive::<Date32Type>().value(0), -1);
    }

    #[test]
    fn un_valor_de_otro_fisico_es_un_error_con_su_posicion() {
        let e = columna_de_valores(
            &Fisico::Entero,
            &[Some(Valor::Entero(1)), Some(Valor::Texto("x".into()))],
        )
        .unwrap_err();
        assert!(e.0.contains("valor 1"), "{e}");
    }

    #[test]
    fn la_columna_entera_y_el_lote_fila_a_fila_son_lo_mismo() {
        use crate::{Columna, RecordBatchBuilder};
        // Salvo el instante, que el lote por filas escribe como `UTC` y el contrato del lago como `+00:00`.
        for (f, valores) in casos().into_iter().filter(|(f, _)| *f != Fisico::Instante) {
            let por_columna = columna_de_valores(&f, &valores).unwrap();
            let mut b = RecordBatchBuilder::nuevo(&[Columna::new("c", f, "x")]).unwrap();
            for v in &valores {
                b.push(std::slice::from_ref(v)).unwrap();
            }
            let lote: RecordBatch = b.finish().unwrap();
            assert_eq!(lote.column(0), &por_columna, "{}", f.nombre());
        }
    }

    #[test]
    fn textos_y_bytes_sueltos() {
        let t = columna_de_textos(vec![Some("a"), None, Some("")].into_iter());
        assert_eq!(
            (t.len(), t.is_null(1), t.as_string::<i32>().value(2)),
            (3, true, "")
        );
        let b = columna_de_bytes(vec![Some(&[1u8, 2][..]), None].into_iter());
        assert_eq!(
            (b.len(), b.is_null(1), b.as_binary::<i32>().value(0)),
            (2, true, &[1u8, 2][..])
        );
        assert_eq!(columna_de_textos(std::iter::empty()).len(), 0);
        assert_eq!(columna_de_valores(&Fisico::Entero, &[]).unwrap().len(), 0);
    }
}
