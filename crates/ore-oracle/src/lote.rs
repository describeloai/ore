//! **De las celdas de Oracle a un `RecordBatch` de Arrow** (ficha 0074).
//!
//! El conector v2 contesta Arrow en flujo, un lote cada pocos miles de filas mientras se lee
//! (`docs/federation.md` §1.3). Cada celda llega ya como **texto canónico** (el que [`Fisico::analizar`]
//! entiende: `2026-10-25 13:14:15.123456`, `2026-10-25 07:44:15.123456+00`, `1234567890.12`…) y esa función
//! es la única que decide si cabe: el constructor no sabe de decimales ni de calendarios, y por eso no puede
//! estar en desacuerdo con el lago.
//!
//! # La regla que manda
//!
//! **Un valor que no cabe en el tipo de su columna es un error que nombra columna y fila; no se convierte en
//! nulo ni se redondea** (0032 §4): un `NUMBER(12, 2)` con tres decimales, o un instante fuera del rango de los
//! microsegundos de 64 bits.

use crate::tipos::Tipo;
use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use ore_base::tipos::{Fisico, Valor};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorLote {
    /// La celda llegó, pero no cabe en el tipo de su columna.
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
            ErrorLote::Arrow(m) => write!(f, "arrow: {m}"),
        }
    }
}

impl std::error::Error for ErrorLote {}

impl From<ErrorLote> for ore_driver::Fallo {
    /// Un valor que no cabe es un cambio de esquema que quien sirve tiene que decidir (`esquema`); un fallo de
    /// Arrow es del origen.
    fn from(e: ErrorLote) -> Self {
        match e {
            ErrorLote::Arrow(_) => ore_driver::Fallo::origen(e.to_string()),
            _ => ore_driver::Fallo::new(ore_driver::Codigo::Esquema, e.to_string()),
        }
    }
}

enum Col {
    Entero(Int64Builder),
    Real(Float64Builder),
    Decimal(Decimal128Builder),
    Logico(BooleanBuilder),
    Instante(TimestampMicrosecondBuilder),
    FechaHora(TimestampMicrosecondBuilder),
    Fecha(Date32Builder),
    Texto(StringBuilder),
}

struct Campo {
    propiedad: String,
    tipo: Tipo,
    fisico: Fisico,
    col: Col,
}

pub struct Constructor {
    schema: SchemaRef,
    campos: Vec<Campo>,
    /// Filas del lote en curso.
    filas: usize,
    /// Filas vistas desde el principio: para decir dónde falló.
    total: u64,
}

impl Constructor {
    /// `(propiedad, tipo)` por columna, en el orden de la proyección. La propiedad es el nombre que sale en el
    /// lote (0008: no la columna física).
    pub fn nuevo(columnas: &[(String, Tipo)]) -> Result<Constructor, ErrorLote> {
        let mut campos = Vec::new();
        let mut fields = Vec::new();
        for (propiedad, tipo) in columnas {
            let fisico = tipo.fisico();
            let (dt, col) = match fisico {
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
                Fisico::FechaHora => (
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    Col::FechaHora(TimestampMicrosecondBuilder::new()),
                ),
                Fisico::Fecha => (DataType::Date32, Col::Fecha(Date32Builder::new())),
                // Oracle no tiene `TIME`: el físico no se produce.
                Fisico::Texto | Fisico::Hora => (DataType::Utf8, Col::Texto(StringBuilder::new())),
            };
            let meta: HashMap<String, String> = [("ore:tipo".to_string(), tipo.oos())].into();
            fields.push(Field::new(propiedad, dt, true).with_metadata(meta));
            campos.push(Campo {
                propiedad: propiedad.clone(),
                tipo: tipo.clone(),
                fisico,
                col,
            });
        }
        Ok(Constructor {
            schema: Arc::new(Schema::new(fields)),
            campos,
            filas: 0,
            total: 0,
        })
    }

    pub fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    /// Filas del lote en curso.
    pub fn filas(&self) -> usize {
        self.filas
    }

    /// Añade una fila: una celda por columna, en el orden de [`Constructor::nuevo`]; `None` es `NULL`. Si
    /// falla, el lote en curso queda con las columnas a medias: quien lo recibe corta la lectura y no lo sella.
    pub fn push(&mut self, fila: &[Option<String>]) -> Result<(), ErrorLote> {
        let n = self.total;
        if fila.len() != self.campos.len() {
            return Err(ErrorLote::Arrow(format!(
                "la fila trae {} celdas y el lote {} columnas",
                fila.len(),
                self.campos.len()
            )));
        }
        for (c, v) in self.campos.iter_mut().zip(fila) {
            let valor = match v {
                None => None,
                Some(t) => Some(c.fisico.analizar(t).ok_or_else(|| ErrorLote::NoCabe {
                    columna: c.propiedad.clone(),
                    fila: n,
                    detalle: format!(
                        "`{}` no cabe en {} ({})",
                        recortar(t),
                        c.tipo.oos(),
                        c.tipo.descripcion()
                    ),
                })?),
            };
            añadir(c, valor)?;
        }
        self.filas += 1;
        self.total += 1;
        Ok(())
    }

    /// Cierra el lote en curso y deja el constructor listo para el siguiente.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorLote> {
        let arrays: Vec<ArrayRef> = self
            .campos
            .iter_mut()
            .map(|c| -> ArrayRef {
                match &mut c.col {
                    Col::Entero(b) => Arc::new(b.finish()),
                    Col::Real(b) => Arc::new(b.finish()),
                    Col::Decimal(b) => Arc::new(b.finish()),
                    Col::Logico(b) => Arc::new(b.finish()),
                    Col::Instante(b) => Arc::new(b.finish()),
                    Col::FechaHora(b) => Arc::new(b.finish()),
                    Col::Fecha(b) => Arc::new(b.finish()),
                    Col::Texto(b) => Arc::new(b.finish()),
                }
            })
            .collect();
        self.filas = 0;
        RecordBatch::try_new(self.schema.clone(), arrays)
            .map_err(|e| ErrorLote::Arrow(e.to_string()))
    }
}

/// Un valor largo no cabe entero en un mensaje de error.
fn recortar(t: &str) -> String {
    const MAX: usize = 60;
    if t.chars().count() <= MAX {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(MAX).collect::<String>())
    }
}

fn añadir(c: &mut Campo, v: Option<Valor>) -> Result<(), ErrorLote> {
    let raro = || {
        ErrorLote::Arrow(format!(
            "la columna `{}` recibió un valor de otro físico",
            c.propiedad
        ))
    };
    match (&mut c.col, v) {
        (Col::Entero(b), None) => b.append_null(),
        (Col::Entero(b), Some(Valor::Entero(n))) => b.append_value(n),
        (Col::Real(b), None) => b.append_null(),
        (Col::Real(b), Some(Valor::Real(x))) => b.append_value(x),
        (Col::Decimal(b), None) => b.append_null(),
        (Col::Decimal(b), Some(Valor::Decimal(x))) => b.append_value(x),
        (Col::Logico(b), None) => b.append_null(),
        (Col::Logico(b), Some(Valor::Logico(x))) => b.append_value(x),
        (Col::Instante(b), None) => b.append_null(),
        (Col::Instante(b), Some(Valor::Instante(x))) => b.append_value(x),
        (Col::FechaHora(b), None) => b.append_null(),
        (Col::FechaHora(b), Some(Valor::FechaHora(x))) => b.append_value(x),
        (Col::Fecha(b), None) => b.append_null(),
        (Col::Fecha(b), Some(Valor::Fecha(x))) => b.append_value(x),
        (Col::Texto(b), None) => b.append_null(),
        (Col::Texto(b), Some(Valor::Texto(x))) => b.append_value(x),
        _ => return Err(raro()),
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Decimal128Type, Int64Type, TimestampMicrosecondType};

    fn s(t: &str) -> Option<String> {
        Some(t.to_string())
    }

    fn columnas() -> Vec<(String, Tipo)> {
        vec![
            ("id".into(), Tipo::Entero),
            (
                "importe".into(),
                Tipo::Decimal {
                    precision: 12,
                    escala: 2,
                },
            ),
            ("creado".into(), Tipo::FechaHora),
            ("firmado".into(), Tipo::Instante),
            ("nota".into(), Tipo::Texto),
            ("libre".into(), Tipo::DecimalLibre),
        ]
    }

    #[test]
    fn un_lote_con_cada_tipo_y_sus_nulos() {
        let mut c = Constructor::nuevo(&columnas()).unwrap();
        c.push(&[
            s("42"),
            s("1234567890.12"),
            s("2026-10-25 13:14:15.123456"),
            s("2026-10-25 07:44:15.123456+00"),
            s("texto ñ €"),
            s("3.14159265358979323846264338327950288"),
        ])
        .unwrap();
        c.push(&[None, None, None, None, None, None]).unwrap();
        assert_eq!(c.filas(), 2);
        let lote = c.finish().unwrap();
        assert_eq!(lote.num_rows(), 2);
        assert_eq!(c.filas(), 0);

        assert_eq!(lote.column(0).as_primitive::<Int64Type>().value(0), 42);
        assert!(lote.column(0).is_null(1));
        assert_eq!(
            lote.column(1).as_primitive::<Decimal128Type>().value(0),
            123_456_789_012
        );
        // 2026-10-25T13:14:15.123456 en µs desde 1970.
        assert_eq!(
            lote.column(2)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_792_934_055_123_456
        );
        assert_eq!(
            lote.column(3)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_792_914_255_123_456
        );
        assert_eq!(lote.column(4).as_string::<i32>().value(0), "texto ñ €");
        assert_eq!(
            lote.column(5).as_string::<i32>().value(0),
            "3.14159265358979323846264338327950288"
        );
        // El esquema lleva el tipo de OOS en los metadatos y la zona del instante.
        let sch = lote.schema();
        assert_eq!(sch.field(1).metadata()["ore:tipo"], "Decimal<12, 2>");
        assert_eq!(
            sch.field(3).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        assert_eq!(
            sch.field(2).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None)
        );
    }

    #[test]
    fn lo_que_no_cabe_es_un_error_con_columna_y_fila() {
        let mut c = Constructor::nuevo(&columnas()).unwrap();
        let buena = [
            s("1"),
            s("1.00"),
            s("2026-01-01 00:00:00"),
            s("2026-01-01 00:00:00+00"),
            s("a"),
            s("1"),
        ];
        c.push(&buena).unwrap();
        // Tres decimales en un `Decimal<12, 2>`: no se redondea.
        let mut mala = buena.clone();
        mala[1] = s("1.005");
        let e = c.push(&mala).unwrap_err();
        match &e {
            ErrorLote::NoCabe { columna, fila, .. } => {
                assert_eq!((columna.as_str(), *fila), ("importe", 1));
            }
            otro => panic!("{otro:?}"),
        }
        assert!(e.to_string().contains("Decimal<12, 2>"), "{e}");
        // Y se traduce a `esquema`, no a un fallo del origen.
        let f: ore_driver::Fallo = e.into();
        assert_eq!(f.codigo, ore_driver::Codigo::Esquema);
        // Un entero que desborda un i64.
        let mut mala = buena.clone();
        mala[0] = s("9223372036854775808");
        assert!(c.push(&mala).is_err());
        // Un instante con zona en una columna sin zona.
        let mut mala = buena.clone();
        mala[2] = s("2026-01-01 00:00:00+01");
        assert!(c.push(&mala).is_err());
    }

    #[test]
    fn una_fecha_es_date32() {
        use arrow_array::Array;
        use arrow_array::types::Date32Type;
        let mut c = Constructor::nuevo(&[("dia".into(), Tipo::Fecha)]).unwrap();
        c.push(&[s("2026-10-25")]).unwrap();
        c.push(&[s("1970-01-01")]).unwrap();
        c.push(&[None]).unwrap();
        let lote = c.finish().unwrap();
        let a = lote.column(0).as_primitive::<Date32Type>();
        assert_eq!((a.value(0), a.value(1), a.is_null(2)), (20751, 0, true));
        assert_eq!(lote.schema().field(0).data_type(), &DataType::Date32);
        // Con hora no cabe en un día.
        assert!(c.push(&[s("2026-10-25 13:00:00")]).is_err());
    }

    #[test]
    fn una_fila_con_otro_numero_de_celdas_se_rechaza() {
        let mut c = Constructor::nuevo(&columnas()).unwrap();
        assert!(matches!(c.push(&[s("1")]), Err(ErrorLote::Arrow(_))));
    }
}
