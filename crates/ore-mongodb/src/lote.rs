//! **De documentos BSON a un `RecordBatch` de Arrow** (ficha 0069 §4.3).
//!
//! El conector v2 contesta Arrow en flujo, un lote cada pocos miles de filas
//! mientras se lee (`docs/federation.md` §1.3). Aquí se construye cada lote a
//! partir del BSON **crudo** (`RawDocument`): medido en X0, leer sin decodificar
//! a `Document` es 3,6 veces más rápido (347 000 frente a 95 000 docs/s sobre
//! 10⁶ documentos) y el pico de memoria no pasa de 8 MB.
//!
//! # La regla que manda
//!
//! **Un valor que no cabe en el tipo de su columna es un error que nombra
//! columna y fila; no se convierte en nulo ni se redondea.** Es la misma regla
//! de 0032 §4 y del contrato de los conectores. Una muestra no ve lo que
//! aparece tarde (ficha 0069 §6): cuando un documento trae una clase que la
//! columna no admite, el lote falla diciendo cuál y dónde, y quien sirve
//! decide si re-infiere (`esquema`, 409) o corta.
//!
//! - `Entero`, `Real`, `Decimal`, `Logico`, `Instante`, `Texto`, `ObjectId`,
//!   `Uuid`, `Binario` solo admiten **su** clase (un `Real` admite además
//!   enteros; un `Decimal`, enteros). `null` y ausente son nulo.
//! - `Json` y `Mixto` admiten cualquiera: se escriben como texto, cada valor
//!   con su forma canónica.

use crate::decimal;
use crate::esquema::{Tipo, clase_de};
use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use mongodb::bson::Bson;
use mongodb::bson::raw::{RawBsonRef, RawDocument};
use mongodb::bson::spec::BinarySubtype;
use ore_arrow::{Columna, ErrorArrow, RecordBatchBuilder};
use ore_base::tipos::{Fisico, Valor as ValorOos};
pub use ore_driver::base64::codificar as base64;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorLote {
    /// Un valor de una clase que la columna no admite.
    Inesperado {
        columna: String,
        fila: u64,
        esperado: &'static str,
        visto: &'static str,
    },
    /// El valor es de la clase, pero no cabe en su tipo.
    NoCabe {
        columna: String,
        fila: u64,
        detalle: String,
    },
    /// El documento no es BSON válido.
    Documento(String),
    Arrow(String),
}

impl fmt::Display for ErrorLote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorLote::Inesperado {
                columna,
                fila,
                esperado,
                visto,
            } => write!(
                f,
                "la columna `{columna}` es {esperado} y en la fila {fila} hay un valor {visto}: el esquema cambió"
            ),
            ErrorLote::NoCabe {
                columna,
                fila,
                detalle,
            } => {
                write!(f, "columna `{columna}`, fila {fila}: {detalle}")
            }
            ErrorLote::Documento(m) => write!(f, "documento mal formado: {m}"),
            ErrorLote::Arrow(m) => write!(f, "arrow: {m}"),
        }
    }
}

impl std::error::Error for ErrorLote {}

struct Campo {
    propiedad: String,
    ruta: String,
    tipo: Tipo,
}

impl From<ErrorArrow> for ErrorLote {
    fn from(e: ErrorArrow) -> Self {
        ErrorLote::Arrow(e.0)
    }
}

/// Convierte cada documento a valores de OOS y los pasa al lote de `ore-arrow`, que es quien arma el Arrow (igual
/// para todos los orígenes).
pub struct Constructor {
    lote: RecordBatchBuilder,
    campos: Vec<Campo>,
    /// La fila en curso, ya convertida (se reutiliza: una fila entra entera o no entra).
    fila: Vec<Option<ValorOos>>,
    /// Filas vistas desde el principio: para decir dónde falló.
    total: u64,
}

impl Constructor {
    /// `(propiedad, ruta física, tipo)` por columna, en el orden de la proyección.
    /// La propiedad es el nombre que sale en el lote (0008: no la columna física).
    pub fn nuevo(columnas: &[(String, String, Tipo)]) -> Result<Constructor, ErrorLote> {
        let cols: Vec<Columna> = columnas
            .iter()
            .map(|(p, _, t)| Columna::new(p, t.fisico(), t.oos()))
            .collect();
        Ok(Constructor {
            lote: RecordBatchBuilder::nuevo(&cols)?,
            campos: columnas
                .iter()
                .map(|(p, r, t)| Campo {
                    propiedad: p.clone(),
                    ruta: r.clone(),
                    tipo: t.clone(),
                })
                .collect(),
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

    /// Añade un documento. Si falla, el lote en curso no cambia: la fila entra entera o no entra.
    pub fn push(&mut self, doc: &RawDocument) -> Result<(), ErrorLote> {
        let fila = self.total;
        self.fila.clear();
        for c in &self.campos {
            let v = valor_en_ruta(doc, &c.ruta)?;
            self.fila.push(a_valor(c, v, fila)?);
        }
        self.lote.push(&self.fila)?;
        self.total += 1;
        Ok(())
    }

    /// Cierra el lote en curso y deja el constructor listo para el siguiente.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorLote> {
        Ok(self.lote.finish()?)
    }
}

/// El valor de una ruta `a.b.c`, bajando por documentos. Si algo del camino no
/// es un documento (un array, un escalar) o falta, no hay valor.
pub fn valor_en_ruta<'a>(
    doc: &'a RawDocument,
    ruta: &str,
) -> Result<Option<RawBsonRef<'a>>, ErrorLote> {
    let mut actual = doc;
    let mut partes = ruta.split('.').peekable();
    while let Some(p) = partes.next() {
        let v = actual
            .get(p)
            .map_err(|e| ErrorLote::Documento(e.to_string()))?;
        if partes.peek().is_none() {
            return Ok(v);
        }
        match v {
            Some(RawBsonRef::Document(d)) => actual = d,
            _ => return Ok(None),
        }
    }
    Ok(None)
}

fn inesperado(c: &Campo, fila: u64, esperado: &'static str, v: &RawBsonRef<'_>) -> ErrorLote {
    ErrorLote::Inesperado {
        columna: c.propiedad.clone(),
        fila,
        esperado,
        visto: clase_de(v).nombre(),
    }
}

fn no_cabe(c: &Campo, fila: u64, detalle: impl Into<String>) -> ErrorLote {
    ErrorLote::NoCabe {
        columna: c.propiedad.clone(),
        fila,
        detalle: detalle.into(),
    }
}

/// El valor de OOS de un valor BSON, o el error que dice por qué no cabe en la columna. `null` y ausente son nulo.
fn a_valor(c: &Campo, v: Option<RawBsonRef<'_>>, fila: u64) -> Result<Option<ValorOos>, ErrorLote> {
    let v = match v {
        None | Some(RawBsonRef::Null) => return Ok(None),
        Some(v) => v,
    };
    // Lo que se calcula antes de tomar el constructor, para no pelear con el préstamo.
    match (&c.tipo, &v) {
        (Tipo::Entero, RawBsonRef::Int32(_) | RawBsonRef::Int64(_)) => {}
        (Tipo::Entero, _) => return Err(inesperado(c, fila, "un entero", &v)),
        (Tipo::Real, RawBsonRef::Double(_) | RawBsonRef::Int32(_) | RawBsonRef::Int64(_)) => {}
        (Tipo::Real, _) => return Err(inesperado(c, fila, "un decimal de coma flotante", &v)),
        (Tipo::Logico, RawBsonRef::Boolean(_)) => {}
        (Tipo::Logico, _) => return Err(inesperado(c, fila, "un booleano", &v)),
        (Tipo::Instante | Tipo::Fecha | Tipo::FechaHora, RawBsonRef::DateTime(_)) => {}
        (Tipo::Instante | Tipo::Fecha | Tipo::FechaHora, _) => {
            return Err(inesperado(c, fila, "una fecha", &v));
        }
        (Tipo::Texto, RawBsonRef::String(_)) => {}
        (Tipo::Texto, _) => return Err(inesperado(c, fila, "un texto", &v)),
        (Tipo::ObjectId, RawBsonRef::ObjectId(_)) => {}
        (Tipo::ObjectId, _) => return Err(inesperado(c, fila, "un ObjectId", &v)),
        (Tipo::Uuid, RawBsonRef::Binary(b))
            if b.subtype == BinarySubtype::Uuid && b.bytes.len() == 16 => {}
        (Tipo::Uuid, _) => return Err(inesperado(c, fila, "un UUID", &v)),
        (Tipo::Binario, RawBsonRef::Binary(_)) => {}
        (Tipo::Binario, _) => return Err(inesperado(c, fila, "un binario", &v)),
        (
            Tipo::Decimal { .. },
            RawBsonRef::Decimal128(_) | RawBsonRef::Int32(_) | RawBsonRef::Int64(_),
        ) => {}
        (Tipo::Decimal { .. }, _) => return Err(inesperado(c, fila, "un decimal", &v)),
        (Tipo::Json | Tipo::Mixto, _) => {}
    }
    // Decimal: se calcula fuera del `match` de abajo, que toma `c.col` prestado.
    let decimal_i128 = match (&c.tipo, &v) {
        (Tipo::Decimal { precision, escala }, RawBsonRef::Decimal128(d)) => Some(
            decimal::de(d)
                .map_err(|e| {
                    no_cabe(
                        c,
                        fila,
                        format!("un `Decimal128` {e:?} no cabe en un decimal"),
                    )
                })
                .and_then(|dec| {
                    dec.a_i128(*escala, *precision)
                        .map_err(|m| no_cabe(c, fila, m))
                })?,
        ),
        (Tipo::Decimal { precision, escala }, RawBsonRef::Int32(n)) => Some(
            entero_decimal(i64::from(*n), *escala, *precision).map_err(|m| no_cabe(c, fila, m))?,
        ),
        (Tipo::Decimal { precision, escala }, RawBsonRef::Int64(n)) => {
            Some(entero_decimal(*n, *escala, *precision).map_err(|m| no_cabe(c, fila, m))?)
        }
        _ => None,
    };
    let micros = match &v {
        RawBsonRef::DateTime(d) if matches!(c.tipo, Tipo::Instante | Tipo::FechaHora) => Some(
            d.timestamp_millis()
                .checked_mul(1000)
                .ok_or_else(|| no_cabe(c, fila, "la fecha no cabe en un `timestamp[us]`"))?,
        ),
        _ => None,
    };
    let dias = match &v {
        RawBsonRef::DateTime(d) if matches!(c.tipo, Tipo::Fecha) => Some(
            i32::try_from(d.timestamp_millis().div_euclid(86_400_000))
                .map_err(|_| no_cabe(c, fila, "la fecha no cabe en un `date32`"))?,
        ),
        _ => None,
    };
    let texto_de = match c.tipo {
        Tipo::Texto | Tipo::ObjectId | Tipo::Uuid | Tipo::Binario | Tipo::Json | Tipo::Mixto => {
            Some(texto(&v, &c.tipo))
        }
        _ => None,
    };
    Ok(Some(match c.tipo.fisico() {
        Fisico::Entero => ValorOos::Entero(match v {
            RawBsonRef::Int32(n) => i64::from(n),
            RawBsonRef::Int64(n) => n,
            _ => unreachable!(),
        }),
        Fisico::Real => ValorOos::Real(match v {
            RawBsonRef::Double(x) => x,
            RawBsonRef::Int32(n) => f64::from(n),
            RawBsonRef::Int64(n) => n as f64,
            _ => unreachable!(),
        }),
        Fisico::Decimal { .. } => ValorOos::Decimal(decimal_i128.expect("decimal calculado")),
        Fisico::Logico => ValorOos::Logico(matches!(v, RawBsonRef::Boolean(true))),
        Fisico::Instante => ValorOos::Instante(micros.expect("instante calculado")),
        Fisico::FechaHora => ValorOos::FechaHora(micros.expect("hora calculada")),
        Fisico::Fecha => ValorOos::Fecha(dias.expect("día calculado")),
        Fisico::Texto | Fisico::Hora => ValorOos::Texto(texto_de.expect("texto calculado")),
    }))
}

fn entero_decimal(n: i64, escala: u8, precision: u8) -> Result<i128, String> {
    decimal::Dec {
        negativo: n < 0,
        digitos: n.unsigned_abs().to_string(),
        exp: 0,
    }
    .a_i128(escala, precision)
}

/// La forma canónica de un valor cuando viaja como texto.
pub fn texto(v: &RawBsonRef<'_>, tipo: &Tipo) -> String {
    match v {
        RawBsonRef::String(s) => (*s).to_string(),
        RawBsonRef::Int32(n) => n.to_string(),
        RawBsonRef::Int64(n) => n.to_string(),
        RawBsonRef::Double(x) => {
            if x.is_nan() {
                "NaN".into()
            } else if x.is_infinite() {
                if *x > 0.0 {
                    "Infinity".into()
                } else {
                    "-Infinity".into()
                }
            } else {
                x.to_string()
            }
        }
        RawBsonRef::Boolean(b) => b.to_string(),
        RawBsonRef::DateTime(d) => iso8601(d.timestamp_millis()),
        RawBsonRef::ObjectId(o) => o.to_hex(),
        RawBsonRef::Decimal128(d) => match decimal::de(d) {
            Ok(dec) => dec.texto(),
            Err(decimal::Especial::NaN) => "NaN".into(),
            Err(decimal::Especial::Infinito) => d.to_string(),
        },
        RawBsonRef::Binary(b) if b.subtype == BinarySubtype::Uuid && b.bytes.len() == 16 => {
            uuid(b.bytes)
        }
        RawBsonRef::Binary(b) => base64(b.bytes),
        RawBsonRef::Timestamp(t) => format!("{}:{}", t.time, t.increment),
        // Arrays, documentos y lo que OOS no tiene: extended JSON relajado.
        otro => {
            let _ = tipo;
            match Bson::try_from(otro.to_raw_bson()) {
                Ok(b) => b.into_relaxed_extjson().to_string(),
                Err(e) => format!("\"<no se pudo leer: {e}>\""),
            }
        }
    }
}

fn uuid(b: &[u8]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// `AAAA-MM-DDTHH:MM:SS.mmmZ` desde milisegundos de 1970 (calendario gregoriano
/// proléptico, el de Howard Hinnant). Años fuera de 0000–9999 llevan signo.
pub fn iso8601(ms: i64) -> String {
    let dias = ms.div_euclid(86_400_000);
    let resto = ms.rem_euclid(86_400_000);
    let (h, m, s, mm) = (
        resto / 3_600_000,
        resto / 60_000 % 60,
        resto / 1000 % 60,
        resto % 1000,
    );
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 };
    let año = yoe + era * 400 + i64::from(mes <= 2);
    let año = if (0..=9999).contains(&año) {
        format!("{año:04}")
    } else {
        format!("{año:+}")
    };
    format!("{año}-{mes:02}-{d:02}T{h:02}:{m:02}:{s:02}.{mm:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Decimal128Type, Int64Type, TimestampMicrosecondType};
    use arrow_schema::{DataType, TimeUnit};
    use mongodb::bson::{
        Binary, DateTime, Decimal128, Document, RawDocumentBuf, doc, oid::ObjectId,
    };
    use std::str::FromStr;

    fn raw(d: Document) -> RawDocumentBuf {
        RawDocumentBuf::from_document(&d).unwrap()
    }

    fn cons(cols: &[(&str, &str, Tipo)]) -> Constructor {
        let v: Vec<(String, String, Tipo)> = cols
            .iter()
            .map(|(p, r, t)| (p.to_string(), r.to_string(), t.clone()))
            .collect();
        Constructor::nuevo(&v).unwrap()
    }

    #[test]
    fn el_esquema_de_arrow_es_el_del_contrato_de_tipos() {
        let c = cons(&[
            ("a", "a", Tipo::Entero),
            ("b", "b", Tipo::Real),
            (
                "c",
                "c",
                Tipo::Decimal {
                    precision: 38,
                    escala: 3,
                },
            ),
            ("d", "d", Tipo::Logico),
            ("e", "e", Tipo::Instante),
            ("f", "f", Tipo::Texto),
            ("g", "g", Tipo::ObjectId),
        ]);
        let tipos: Vec<String> = c
            .schema()
            .fields()
            .iter()
            .map(|f| f.data_type().to_string())
            .collect();
        assert_eq!(
            tipos,
            [
                "Int64",
                "Float64",
                "Decimal128(38, 3)",
                "Boolean",
                "Timestamp(µs, \"UTC\")",
                "Utf8",
                "Utf8"
            ]
        );
        // Cada campo lleva el tipo de OOS en sus metadatos.
        assert_eq!(c.schema().field(2).metadata()["ore:tipo"], "Decimal<38, 3>");
    }

    #[test]
    fn convierte_los_escalares_y_los_nulos() {
        let mut c = cons(&[
            ("id", "_id", Tipo::Entero),
            (
                "v",
                "v",
                Tipo::Decimal {
                    precision: 38,
                    escala: 3,
                },
            ),
            ("f", "f", Tipo::Instante),
            ("n", "n", Tipo::Entero),
        ]);
        let d = |t: &str| Decimal128::from_str(t).unwrap();
        c.push(&raw(doc! {"_id": 1_i32, "v": d("12345678901234567890123456789.123"), "f": DateTime::from_millis(1_700_000_000_123), "n": 7_i64})).unwrap();
        c.push(&raw(
            doc! {"_id": 2_i64, "v": d("0.005"), "f": DateTime::from_millis(0)},
        ))
        .unwrap(); // `n` ausente
        c.push(&raw(doc! {"_id": 3_i32, "v": mongodb::bson::Bson::Null, "f": DateTime::from_millis(253_402_300_799_999), "n": mongodb::bson::Bson::Null})).unwrap(); // año 9999
        let b = c.finish().unwrap();
        assert_eq!(b.num_rows(), 3);
        let v = b.column(1).as_primitive::<Decimal128Type>();
        assert_eq!(v.value(0), 12345678901234567890123456789123);
        assert_eq!(v.value(1), 5);
        assert!(v.is_null(2));
        let f = b.column(2).as_primitive::<TimestampMicrosecondType>();
        assert_eq!(f.value(0), 1_700_000_000_123_000);
        assert_eq!(f.value(2), 253_402_300_799_999_000);
        let n = b.column(3).as_primitive::<Int64Type>();
        assert!(
            !n.is_null(0) && n.is_null(1) && n.is_null(2),
            "ausente y null son nulo"
        );
        // El constructor queda listo para otro lote, y sigue contando las filas.
        assert_eq!(c.filas(), 0);
        c.push(&raw(doc! {"_id": 9_i32})).unwrap();
        assert_eq!(c.finish().unwrap().num_rows(), 1);
    }

    /// Una ruta con puntos baja por documentos; por un array, no hay valor.
    #[test]
    fn las_rutas_anidadas() {
        let mut c = cons(&[
            ("w", "awards.wins", Tipo::Entero),
            ("c", "imdb.extra.c", Tipo::Texto),
        ]);
        c.push(&raw(
            doc! {"awards": {"wins": 3_i32}, "imdb": {"extra": {"c": "x"}}},
        ))
        .unwrap();
        c.push(&raw(doc! {"awards": [1, 2], "imdb": {"extra": 5_i32}}))
            .unwrap();
        c.push(&raw(doc! {"x": 1_i32})).unwrap();
        let b = c.finish().unwrap();
        let w = b.column(0).as_primitive::<Int64Type>();
        assert_eq!((w.value(0), w.is_null(1), w.is_null(2)), (3, true, true));
        assert_eq!(b.column(1).as_string::<i32>().value(0), "x");
        assert!(b.column(1).is_null(1));
    }

    /// El valor que no cabe se dice con columna y fila, y NO se convierte en nulo.
    #[test]
    fn lo_que_no_cabe_nombra_columna_y_fila() {
        let d = |t: &str| Decimal128::from_str(t).unwrap();
        let mut c = cons(&[(
            "v",
            "v",
            Tipo::Decimal {
                precision: 38,
                escala: 2,
            },
        )]);
        c.push(&raw(doc! {"v": d("1.25")})).unwrap();
        c.push(&raw(doc! {"v": d("2.50")})).unwrap();
        // fila 2: tres decimales no caben en escala 2, y no se redondea
        let e = c.push(&raw(doc! {"v": d("0.005")})).unwrap_err();
        assert!(
            matches!(&e, ErrorLote::NoCabe { columna, fila: 2, .. } if columna == "v"),
            "{e:?}"
        );
        assert!(
            e.to_string().contains("columna `v`, fila 2")
                && e.to_string().contains("más decimales"),
            "{e}"
        );
        // NaN
        let e = cons(&[(
            "v",
            "v",
            Tipo::Decimal {
                precision: 38,
                escala: 2,
            },
        )])
        .push(&raw(doc! {"v": d("NaN")}))
        .unwrap_err();
        assert!(matches!(e, ErrorLote::NoCabe { .. }), "{e:?}");
        // un instante fuera del rango de timestamp[us]
        let e = cons(&[("f", "f", Tipo::Instante)])
            .push(&raw(doc! {"f": DateTime::from_millis(i64::MAX)}))
            .unwrap_err();
        assert!(e.to_string().contains("timestamp[us]"), "{e}");
    }

    /// Una clase que la muestra no vio: error diciendo el esquema cambió.
    #[test]
    fn una_clase_que_la_columna_no_admite_dice_que_el_esquema_cambio() {
        let mut c = cons(&[("edad", "edad", Tipo::Entero)]);
        c.push(&raw(doc! {"edad": 7_i32})).unwrap();
        let e = c.push(&raw(doc! {"edad": "ocho"})).unwrap_err();
        assert_eq!(
            e,
            ErrorLote::Inesperado {
                columna: "edad".into(),
                fila: 1,
                esperado: "un entero",
                visto: "string"
            }
        );
        assert!(e.to_string().contains("el esquema cambió"));
        // Un Real admite enteros; un Decimal también.
        let mut r = cons(&[
            ("x", "x", Tipo::Real),
            (
                "d",
                "d",
                Tipo::Decimal {
                    precision: 38,
                    escala: 2,
                },
            ),
        ]);
        r.push(&raw(doc! {"x": 3_i32, "d": 4_i64})).unwrap();
        let b = r.finish().unwrap();
        assert_eq!(b.column(1).as_primitive::<Decimal128Type>().value(0), 400);
    }

    #[test]
    fn los_tipos_de_texto_con_su_forma_canonica() {
        let mut c = cons(&[
            ("o", "o", Tipo::ObjectId),
            ("u", "u", Tipo::Uuid),
            ("b", "b", Tipo::Binario),
            ("m", "m", Tipo::Mixto),
            ("j", "j", Tipo::Json),
            ("d", "d", Tipo::Mixto),
            ("t", "t", Tipo::Mixto),
        ]);
        let oid = ObjectId::parse_str("65a1b2c3d4e5f60718293a4b").unwrap();
        let uu = Binary {
            subtype: BinarySubtype::Uuid,
            bytes: vec![
                0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x56, 0x78, 0x12, 0x34,
                0x56, 0x78,
            ],
        };
        c.push(&raw(doc! {
            "o": oid, "u": uu, "b": Binary { subtype: BinarySubtype::Generic, bytes: b"foob".to_vec() },
            "m": 14_i32, "j": {"a": [1, {"k": "v"}], "oid": oid},
            "d": DateTime::from_millis(1_700_000_000_123), "t": mongodb::bson::Timestamp { time: 1_700_000_000, increment: 7 },
        })).unwrap();
        c.push(&raw(doc! {"m": "texto", "j": [1, 2], "d": 2.5, "t": true}))
            .unwrap();
        let b = c.finish().unwrap();
        let s = |i: usize, f: usize| b.column(i).as_string::<i32>().value(f).to_string();
        assert_eq!(s(0, 0), "65a1b2c3d4e5f60718293a4b");
        assert_eq!(s(1, 0), "12345678-1234-5678-1234-567812345678");
        assert_eq!(s(2, 0), "Zm9vYg==");
        assert_eq!((s(3, 0), s(3, 1)), ("14".into(), "texto".into()));
        // JSON extendido relajado: el ObjectId anidado conserva su tipo.
        let j = s(4, 0);
        assert!(
            j.contains("\"$oid\":\"65a1b2c3d4e5f60718293a4b\"") && j.contains("\"k\":\"v\""),
            "{j}"
        );
        assert_eq!(s(4, 1), "[1,2]");
        assert_eq!(
            (s(5, 0), s(5, 1)),
            ("2023-11-14T22:13:20.123Z".into(), "2.5".into())
        );
        assert_eq!((s(6, 0), s(6, 1)), ("1700000000:7".into(), "true".into()));
    }

    #[test]
    fn base64_con_los_vectores_de_la_rfc_4648() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o, "{i}");
        }
    }

    #[test]
    fn las_fechas_en_iso8601() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso8601(253_402_300_799_999), "9999-12-31T23:59:59.999Z");
        assert_eq!(iso8601(-2_208_988_800_000), "1900-01-01T00:00:00.000Z");
        // 29 de febrero de un año bisiesto y el instante anterior al 1970.
        assert_eq!(iso8601(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
        assert_eq!(iso8601(-1), "1969-12-31T23:59:59.999Z");
    }

    /// Un `Date` de BSON leído como `Date` o `DateTime` declarados: el día UTC
    /// (también antes de 1970, donde el cociente se redondea hacia abajo) y la
    /// hora de pared, en microsegundos y sin zona.
    #[test]
    fn un_date_de_bson_sale_como_fecha_o_como_fecha_y_hora() {
        use arrow_array::cast::AsArray;
        use arrow_array::types::{Date32Type, TimestampMicrosecondType};
        let mut c = cons(&[("d", "f", Tipo::Fecha), ("h", "f", Tipo::FechaHora)]);
        // 2000-02-29T12:30:00Z, 1969-12-31T23:59:59.999Z y 1970-01-01T00:00:00Z.
        for ms in [951_827_400_000_i64, -1, 0] {
            c.push(&raw(doc! {"f": DateTime::from_millis(ms)})).unwrap();
        }
        c.push(&raw(doc! {"f": Bson::Null})).unwrap();
        let l = c.finish().unwrap();
        assert_eq!(l.schema().field(0).data_type(), &DataType::Date32,);
        assert_eq!(
            l.schema().field(1).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None),
        );
        let d = l.column(0).as_primitive::<Date32Type>();
        assert_eq!(
            (d.value(0), d.value(1), d.value(2), d.is_null(3)),
            (11_016, -1, 0, true)
        );
        let h = l.column(1).as_primitive::<TimestampMicrosecondType>();
        assert_eq!(
            (h.value(0), h.value(1), h.value(2)),
            (951_827_400_000_000, -1_000, 0)
        );
    }
}
