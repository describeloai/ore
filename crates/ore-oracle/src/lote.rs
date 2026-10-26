//! **De las celdas de Oracle a un `RecordBatch` de Arrow** (ficha 0075).
//!
//! El conector v2 contesta Arrow en flujo, un lote cada pocos miles de filas mientras se lee
//! (`docs/federation.md` §1.3). Cada celda llega ya como **texto canónico** (el que
//! [`Fisico::analizar`](ore_base::tipos::Fisico::analizar) entiende: `2026-10-25 13:14:15.123456`, `2026-10-25 07:44:15.123456+00`, `1234567890.12`…) y esa función
//! es la única que decide si cabe: el constructor no sabe de decimales ni de calendarios, y por eso no puede
//! estar en desacuerdo con el lago. **Armar el lote** es de `ore-arrow`, igual para todos los orígenes; aquí solo
//! está lo de Oracle: de texto canónico a un `Valor`, y qué error dar si no cabe.
//!
//! # La regla que manda
//!
//! **Un valor que no cabe en el tipo de su columna es un error que nombra columna y fila; no se convierte en
//! nulo ni se redondea** (0032 §4): un `NUMBER(12, 2)` con tres decimales, o un instante fuera del rango de los
//! microsegundos de 64 bits.

use crate::tipos::Tipo;
use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use ore_arrow::{Columna, ErrorArrow, RecordBatchBuilder};
use ore_base::tipos::{Fisico, Valor};
use std::fmt;

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

impl From<ErrorArrow> for ErrorLote {
    fn from(e: ErrorArrow) -> Self {
        ErrorLote::Arrow(e.0)
    }
}

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

pub struct Constructor {
    lote: RecordBatchBuilder,
    /// `(propiedad, tipo)` por columna: lo que hace falta para decir dónde falló un valor.
    columnas: Vec<(String, Tipo)>,
    /// El físico de cada columna: se calcula una vez, no en cada celda.
    fisicos: Vec<Fisico>,
    /// La fila en curso, ya analizada (se reutiliza: una fila entra entera o no entra). Un texto no se analiza
    /// —no hay nada que decidir— y va de su celda a Arrow sin pasar por un `Valor`.
    fila: Vec<Option<Valor>>,
    /// Filas vistas desde el principio: para decir dónde falló.
    total: u64,
}

impl Constructor {
    /// `(propiedad, tipo)` por columna, en el orden de la proyección. La propiedad es el nombre que sale en el
    /// lote (0008: no la columna física).
    pub fn nuevo(columnas: &[(String, Tipo)]) -> Result<Constructor, ErrorLote> {
        let cols: Vec<Columna> = columnas
            .iter()
            .map(|(p, t)| Columna::new(p, t.fisico(), t.oos()))
            .collect();
        Ok(Constructor {
            // Con el sitio de un lote ya reservado, y vuelto a reservar tras cada uno (ADR 0075).
            lote: RecordBatchBuilder::con_capacidad(&cols, crate::lectura::FILAS_POR_LOTE)?,
            columnas: columnas.to_vec(),
            fisicos: columnas.iter().map(|(_, t)| t.fisico()).collect(),
            fila: Vec::with_capacity(columnas.len()),
            total: 0,
        })
    }

    pub fn schema(&self) -> SchemaRef {
        self.lote.schema()
    }

    /// Filas del lote en curso.
    pub fn filas(&self) -> usize {
        self.lote.filas()
    }

    /// Añade una fila: una celda por columna, en el orden de [`Constructor::nuevo`]; `None` es `NULL`. Si falla, el
    /// lote en curso no cambia: la fila entra entera o no entra.
    pub fn push(&mut self, fila: &[Option<String>]) -> Result<(), ErrorLote> {
        let n = self.total;
        if fila.len() != self.columnas.len() {
            return Err(ErrorLote::Arrow(format!(
                "la fila trae {} celdas y el lote {} columnas",
                fila.len(),
                self.columnas.len()
            )));
        }
        // Primero se analiza la fila entera: es lo único que puede fallar, y nada se ha escrito todavía.
        self.fila.clear();
        for (i, ((propiedad, tipo), v)) in self.columnas.iter().zip(fila).enumerate() {
            self.fila.push(match v {
                None => None,
                Some(_) if self.fisicos[i] == Fisico::Texto => None,
                Some(t) => Some(
                    self.fisicos[i]
                        .analizar(t)
                        .ok_or_else(|| ErrorLote::NoCabe {
                            columna: propiedad.clone(),
                            fila: n,
                            detalle: format!(
                                "`{}` no cabe en {} ({})",
                                recortar(t),
                                tipo.oos(),
                                tipo.descripcion()
                            ),
                        })?,
                ),
            });
        }
        // Y después se escribe, celda a celda y tipado: ya solo falla un físico que no es el de su columna.
        for (i, celda) in fila.iter().enumerate() {
            let mut e = self.lote.col(i);
            match (celda, &self.fila[i]) {
                (None, _) => e.nulo(),
                (Some(t), None) => e.texto(t)?,
                (Some(_), Some(v)) => e.valor(v)?,
            }
        }
        self.lote.fila_escrita();
        self.total += 1;
        Ok(())
    }

    /// Cierra el lote en curso y deja el constructor listo para el siguiente.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorLote> {
        Ok(self.lote.finish()?)
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

#[cfg(test)]
mod pruebas {
    use super::*;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Decimal128Type, Int64Type, TimestampMicrosecondType};
    use arrow_schema::{DataType, TimeUnit};

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
            &DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into()))
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

    /// v1alpha35: un `RAW`/`BLOB` es `Binary`: los bytes llegan como base64 y salen como `binary`; el vacío no es
    /// nulo; y una columna declarada `String`/`Opaque` (lo de antes) sigue siendo texto base64.
    #[test]
    fn un_binario_es_binary_y_declarado_texto_sigue_siendo_texto() {
        use arrow_array::Array;
        let mut c = Constructor::nuevo(&[
            ("firma".into(), Tipo::Binario),
            ("antes".into(), Tipo::BinarioTexto),
        ])
        .unwrap();
        c.push(&[s("3q2+7w=="), s("3q2+7w==")]).unwrap();
        c.push(&[s(""), s("")]).unwrap();
        c.push(&[None, None]).unwrap();
        assert!(c.push(&[s("no es base64!"), None]).is_err());
        let lote = c.finish().unwrap();
        let a = lote.column(0).as_binary::<i32>();
        assert_eq!(a.value(0), &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(!a.is_null(1) && a.value(1).is_empty());
        assert!(a.is_null(2));
        assert_eq!(lote.schema().field(0).data_type(), &DataType::Binary);
        assert_eq!(lote.schema().field(0).metadata()["ore:tipo"], "Binary");
        // Lo declarado como texto: el mismo base64, `string`, y el tipo que dijo la tabla (`Opaque`).
        assert_eq!(lote.column(1).as_string::<i32>().value(0), "3q2+7w==");
        assert_eq!(lote.schema().field(1).data_type(), &DataType::Utf8);
        assert_eq!(lote.schema().field(1).metadata()["ore:tipo"], "Opaque");
    }

    #[test]
    fn una_fila_con_otro_numero_de_celdas_se_rechaza() {
        let mut c = Constructor::nuevo(&columnas()).unwrap();
        assert!(matches!(c.push(&[s("1")]), Err(ErrorLote::Arrow(_))));
    }
}
