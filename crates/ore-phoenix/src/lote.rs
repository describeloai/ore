//! **De las filas de Avatica a un `RecordBatch` de Arrow** (ficha 0071 §3.4).
//!
//! El conector v2 contesta Arrow en flujo, un lote cada pocos miles de filas mientras se lee
//! (`docs/federation.md` §1.3). Cada celda llega como un [`Valor`] de `ore-avatica` y se convierte al
//! [`ValorOos`] que le toca ([`celda`]).
//!
//! # La regla que manda
//!
//! **Un valor que no cabe en el tipo de su columna es un error que nombra columna y fila; no se convierte en
//! nulo ni se redondea** (0032 §4):
//!
//! - un `DECIMAL(p, s)` con más cifras decimales que `s`, o que pasa de `p` dígitos, no cabe;
//! - una hora (`TIME`) fuera de un día no cabe;
//! - un instante que desborda los microsegundos de 64 bits no cabe;
//! - una celda de una clase que la columna no admite es `Inesperado`.
//!
//! Sin declaración, un `DECIMAL` sin precisión viaja como **texto exacto** (`String`).

use crate::tipos::Tipo;
use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use ore_arrow::{Columna, ErrorArrow, RecordBatchBuilder};
use ore_avatica::Valor;
use ore_base::tipos::Valor as ValorOos;
pub use ore_driver::base64::codificar as base64;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorLote {
    /// Una celda de una clase que la columna no admite.
    Inesperado {
        columna: String,
        fila: u64,
        esperado: &'static str,
        visto: &'static str,
    },
    /// La celda es de la clase, pero no cabe en su tipo.
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
            ErrorLote::Inesperado {
                columna,
                fila,
                esperado,
                visto,
            } => {
                write!(
                    f,
                    "columna `{columna}`, fila {fila}: se esperaba {esperado} y llegó {visto}"
                )
            }
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

/// Lo que falla al convertir una celda, sin saber aún de qué columna ni fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCelda {
    Inesperado {
        esperado: &'static str,
        visto: &'static str,
    },
    NoCabe(String),
}

impl ErrorCelda {
    fn en(self, columna: &str, fila: u64) -> ErrorLote {
        match self {
            ErrorCelda::Inesperado { esperado, visto } => ErrorLote::Inesperado {
                columna: columna.to_string(),
                fila,
                esperado,
                visto,
            },
            ErrorCelda::NoCabe(detalle) => ErrorLote::NoCabe {
                columna: columna.to_string(),
                fila,
                detalle,
            },
        }
    }
}

fn nombre(v: &Valor) -> &'static str {
    match v {
        Valor::Nulo => "null",
        Valor::Bool(_) => "boolean",
        Valor::Entero(_) => "un entero",
        Valor::Real(_) => "un real",
        Valor::Texto(_) => "texto",
        Valor::Bytes(_) => "bytes",
        Valor::Decimal(_) => "un decimal",
        Valor::Lista(_) => "una lista",
    }
}

fn inesperado(esperado: &'static str, v: &Valor) -> ErrorCelda {
    ErrorCelda::Inesperado {
        esperado,
        visto: nombre(v),
    }
}

const MS_POR_DIA: i64 = 86_400_000;

/// **La celda de una columna, como el `Valor` de OOS que le toca.** Lo que viaja como texto (un decimal sin
/// precisión, un binario, un array) sale como [`ValorOos::Texto`]. `Nulo` no llega aquí: es `None`.
pub fn celda(v: &Valor, tipo: &Tipo) -> Result<ValorOos, ErrorCelda> {
    match (tipo, v) {
        (Tipo::Texto, Valor::Texto(s)) => Ok(ValorOos::Texto(s.clone())),
        (Tipo::Entero, Valor::Entero(n)) => Ok(ValorOos::Entero(*n)),
        (Tipo::Real, Valor::Real(x)) => Ok(ValorOos::Real(*x)),
        // Una columna declarada `Float` sobre un entero: el entero cabe en un `f64` solo hasta 2^53.
        (Tipo::Real, Valor::Entero(n)) => {
            if n.unsigned_abs() > (1u64 << 53) {
                Err(ErrorCelda::NoCabe(format!(
                    "el entero {n} no cabe exacto en un float64"
                )))
            } else {
                Ok(ValorOos::Real(*n as f64))
            }
        }
        (Tipo::Logico, Valor::Bool(b)) => Ok(ValorOos::Logico(*b)),
        // `TIMESTAMP` llega en milisegundos desde la época (los nanosegundos no llegan, §3.4).
        (Tipo::Instante, Valor::Entero(ms)) => micros(*ms).map(ValorOos::Instante),
        (Tipo::FechaHora, Valor::Entero(ms)) => micros(*ms).map(ValorOos::FechaHora),
        // `DATE` sin hora (declarado `Date`): días desde la época.
        (Tipo::Fecha, Valor::Entero(d)) => i32::try_from(*d)
            .map(ValorOos::Fecha)
            .map_err(|_| ErrorCelda::NoCabe(format!("{d} días no caben en un date32"))),
        // `TIME`: milisegundos del día.
        (Tipo::Hora, Valor::Entero(ms)) => {
            if (0..MS_POR_DIA).contains(ms) {
                Ok(ValorOos::Hora(ms * 1000))
            } else {
                Err(ErrorCelda::NoCabe(format!(
                    "{ms} ms no son una hora de un día"
                )))
            }
        }
        (Tipo::Decimal { precision, escala }, Valor::Decimal(t)) => {
            decimal(t, *precision, *escala).map(ValorOos::Decimal)
        }
        (Tipo::Decimal { precision, escala }, Valor::Entero(n)) => {
            decimal(&n.to_string(), *precision, *escala).map(ValorOos::Decimal)
        }
        (Tipo::DecimalLibre, Valor::Decimal(t)) => Ok(ValorOos::Texto(t.clone())),
        (Tipo::Binario, Valor::Bytes(b)) => Ok(ValorOos::Bytes(b.clone())),
        (Tipo::BinarioTexto, Valor::Bytes(b)) => Ok(ValorOos::Texto(base64(b))),
        (Tipo::Lista, Valor::Lista(l)) => Ok(ValorOos::Texto(
            serde_json::Value::Array(l.iter().map(json).collect()).to_string(),
        )),
        (Tipo::Otro(_), _) => Err(ErrorCelda::NoCabe(
            "este origen no lee ese tipo de Phoenix".into(),
        )),
        (t, v) => Err(inesperado(t.descripcion(), v)),
    }
}

/// Milisegundos a microsegundos, sin desbordar.
fn micros(ms: i64) -> Result<i64, ErrorCelda> {
    ms.checked_mul(1000).ok_or_else(|| {
        ErrorCelda::NoCabe(format!("{ms} ms desbordan los microsegundos de 64 bits"))
    })
}

/// **El decimal exacto**, del texto de `BigDecimal.toString()` (`12345.678`, `-0.5`, `1E+3`, `1.23E-2`) al
/// entero con la escala de la columna. Si tiene más cifras decimales que `escala` o pasa de `precision`
/// dígitos **no cabe**: no se redondea.
pub fn decimal(texto: &str, precision: u8, escala: u8) -> Result<i128, ErrorCelda> {
    let no = |m: String| ErrorCelda::NoCabe(m);
    let t = texto.trim();
    let (negativo, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (mantisa, exp) = match t.split_once(['e', 'E']) {
        Some((m, e)) => (
            m,
            e.parse::<i32>()
                .map_err(|_| no(format!("`{texto}` no es un decimal")))?,
        ),
        None => (t, 0),
    };
    let (ent, frac) = mantisa.split_once('.').unwrap_or((mantisa, ""));
    if (ent.is_empty() && frac.is_empty())
        || !ent.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
    {
        return Err(no(format!("`{texto}` no es un decimal")));
    }
    let digitos = format!("{ent}{frac}");
    let digitos = digitos.trim_start_matches('0');
    // valor = digitos × 10^(exp − len(frac)); a la escala de la columna: × 10^escala.
    let desplazamiento = i64::from(exp) - frac.len() as i64 + i64::from(escala);
    let mut n: i128 = if digitos.is_empty() {
        0
    } else {
        digitos
            .parse::<i128>()
            .map_err(|_| no(format!("`{texto}` pasa de 38 dígitos")))?
    };
    if desplazamiento >= 0 {
        for _ in 0..desplazamiento {
            n = n
                .checked_mul(10)
                .ok_or_else(|| no(format!("`{texto}` pasa de 38 dígitos")))?;
        }
    } else {
        for _ in 0..(-desplazamiento) {
            if n % 10 != 0 {
                return Err(no(format!(
                    "`{texto}` tiene más de {escala} cifras decimales y no se redondea"
                )));
            }
            n /= 10;
        }
    }
    let tope = 10i128
        .checked_pow(u32::from(precision))
        .unwrap_or(i128::MAX);
    if n >= tope {
        return Err(no(format!("`{texto}` pasa de {precision} dígitos")));
    }
    Ok(if negativo { -n } else { n })
}

/// El JSON de un elemento de un `ARRAY`. Un decimal y un binario van como texto exacto; `NaN` e infinitos, como
/// texto (no son JSON).
fn json(v: &Valor) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Valor::Nulo => J::Null,
        Valor::Bool(b) => J::Bool(*b),
        Valor::Entero(n) => J::Number((*n).into()),
        Valor::Real(x) => match serde_json::Number::from_f64(*x) {
            Some(n) => J::Number(n),
            None if x.is_nan() => J::String("NaN".into()),
            None if *x > 0.0 => J::String("Infinity".into()),
            None => J::String("-Infinity".into()),
        },
        Valor::Texto(s) | Valor::Decimal(s) => J::String(s.clone()),
        Valor::Bytes(b) => J::String(base64(b)),
        Valor::Lista(l) => J::Array(l.iter().map(json).collect()),
    }
}

// ── Arrow ───────────────────────────────────────────────────────────────────

struct Campo {
    propiedad: String,
    tipo: Tipo,
}

impl From<ErrorArrow> for ErrorLote {
    fn from(e: ErrorArrow) -> Self {
        ErrorLote::Arrow(e.0)
    }
}

/// Convierte las celdas nativas a valores de OOS y las pasa al lote de `ore-arrow`, que es quien arma el Arrow
/// (igual para todos los orígenes).
pub struct Constructor {
    lote: RecordBatchBuilder,
    campos: Vec<Campo>,
    /// La fila en curso, ya convertida (se reutiliza: una fila entra entera o no entra).
    fila: Vec<Option<ValorOos>>,
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
            lote: RecordBatchBuilder::nuevo(&cols)?,
            campos: columnas
                .iter()
                .map(|(p, t)| Campo {
                    propiedad: p.clone(),
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

    /// Añade una fila: una celda por columna, en el orden de [`Constructor::nuevo`]. Si falla, el lote en curso no
    /// cambia: la fila entra entera o no entra.
    pub fn push(&mut self, fila: &[Valor]) -> Result<(), ErrorLote> {
        let n = self.total;
        if fila.len() != self.campos.len() {
            return Err(ErrorLote::Arrow(format!(
                "la fila trae {} celdas y el lote {} columnas",
                fila.len(),
                self.campos.len()
            )));
        }
        self.fila.clear();
        for (c, v) in self.campos.iter().zip(fila) {
            self.fila.push(match v {
                Valor::Nulo => None,
                v => Some(celda(v, &c.tipo).map_err(|e| e.en(&c.propiedad, n))?),
            });
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

#[cfg(test)]
mod pruebas {
    use super::*;
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{
        Date32Type, Decimal128Type, Int64Type, Time64MicrosecondType, TimestampMicrosecondType,
    };
    use arrow_schema::{DataType, TimeUnit};

    fn dec(t: &str, p: u8, s: u8) -> Result<i128, ErrorCelda> {
        decimal(t, p, s)
    }

    #[test]
    fn un_decimal_exacto_a_la_escala_de_la_columna() {
        assert_eq!(dec("12345.678", 12, 3), Ok(12_345_678));
        assert_eq!(dec("1.5", 10, 2), Ok(150)); // se rellena con ceros: no pierde nada
        assert_eq!(dec("1.50", 10, 2), Ok(150));
        assert_eq!(dec("-0.5", 10, 2), Ok(-50));
        assert_eq!(dec("0", 10, 2), Ok(0));
        assert_eq!(dec("0.00", 10, 2), Ok(0));
        assert_eq!(dec("+7", 3, 0), Ok(7));
        // 38 dígitos justos.
        assert_eq!(
            dec("99999999999999999999999999999999999999", 38, 0),
            Ok(99_999_999_999_999_999_999_999_999_999_999_999_999)
        );
    }

    #[test]
    fn la_notacion_cientifica_de_bigdecimal() {
        // `BigDecimal.toString()` usa `E` cuando la escala es negativa o el exponente es chico.
        assert_eq!(dec("1E+3", 10, 0), Ok(1000));
        assert_eq!(dec("1.23E-2", 10, 4), Ok(123));
        assert_eq!(dec("5E-1", 10, 1), Ok(5));
        assert_eq!(dec("-2.5E+1", 10, 0), Ok(-25));
    }

    #[test]
    fn lo_que_no_cabe_no_se_redondea() {
        // Más cifras decimales que la escala.
        let e = dec("1.255", 10, 2).unwrap_err();
        assert!(
            matches!(&e, ErrorCelda::NoCabe(m) if m.contains("no se redondea")),
            "{e:?}"
        );
        assert!(dec("1.250", 10, 2).is_ok()); // los ceros de más sí caben
        // Pasa de la precisión.
        assert!(matches!(dec("100", 2, 0), Err(ErrorCelda::NoCabe(m)) if m.contains("dígitos")));
        assert!(dec("99", 2, 0).is_ok());
        // Basura.
        for malo in ["", "abc", "1.2.3", "--1", "1e", ".", "1_0"] {
            assert!(dec(malo, 10, 2).is_err(), "`{malo}`");
        }
    }

    #[test]
    fn las_celdas_por_su_tipo() {
        let c = |v: Valor, t: Tipo| celda(&v, &t);
        assert_eq!(
            c(Valor::Texto("hé".into()), Tipo::Texto),
            Ok(ValorOos::Texto("hé".into()))
        );
        assert_eq!(c(Valor::Entero(-5), Tipo::Entero), Ok(ValorOos::Entero(-5)));
        assert_eq!(c(Valor::Real(1.5), Tipo::Real), Ok(ValorOos::Real(1.5)));
        assert_eq!(
            c(Valor::Bool(true), Tipo::Logico),
            Ok(ValorOos::Logico(true))
        );
        // TIMESTAMP en milisegundos → microsegundos.
        assert_eq!(
            c(Valor::Entero(1_792_503_959_123), Tipo::Instante),
            Ok(ValorOos::Instante(1_792_503_959_123_000))
        );
        assert_eq!(
            c(Valor::Entero(1_792_503_959_123), Tipo::FechaHora),
            Ok(ValorOos::FechaHora(1_792_503_959_123_000))
        );
        assert_eq!(
            c(Valor::Entero(20_746), Tipo::Fecha),
            Ok(ValorOos::Fecha(20_746))
        );
        assert_eq!(
            c(Valor::Entero(49_559_000), Tipo::Hora),
            Ok(ValorOos::Hora(49_559_000_000))
        );
        assert_eq!(
            c(
                Valor::Decimal("3.14159265358979323846".into()),
                Tipo::DecimalLibre
            ),
            Ok(ValorOos::Texto("3.14159265358979323846".into()))
        );
        assert_eq!(
            c(Valor::Bytes(b"ab\0\0".to_vec()), Tipo::Binario),
            Ok(ValorOos::Bytes(b"ab\0\0".to_vec()))
        );
        // Lo declarado `String`/`Opaque` sigue siendo el base64 de antes.
        assert_eq!(
            c(Valor::Bytes(b"ab\0\0".to_vec()), Tipo::BinarioTexto),
            Ok(ValorOos::Texto("YWIAAA==".into()))
        );
        assert_eq!(
            c(
                Valor::Lista(vec![
                    Valor::Entero(1),
                    Valor::Nulo,
                    Valor::Texto("x".into())
                ]),
                Tipo::Lista
            ),
            Ok(ValorOos::Texto("[1,null,\"x\"]".into()))
        );
    }

    #[test]
    fn un_valor_que_no_es_de_su_columna_se_dice() {
        let e = celda(&Valor::Texto("x".into()), &Tipo::Entero).unwrap_err();
        assert!(
            matches!(e, ErrorCelda::Inesperado { visto: "texto", .. }),
            "{e:?}"
        );
        // Una hora fuera de un día, un instante que desborda, un tipo que no se lee.
        assert!(matches!(
            celda(&Valor::Entero(86_400_000), &Tipo::Hora),
            Err(ErrorCelda::NoCabe(_))
        ));
        assert!(matches!(
            celda(&Valor::Entero(-1), &Tipo::Hora),
            Err(ErrorCelda::NoCabe(_))
        ));
        assert!(matches!(
            celda(&Valor::Entero(i64::MAX), &Tipo::Instante),
            Err(ErrorCelda::NoCabe(_))
        ));
        assert!(matches!(
            celda(&Valor::Entero(i64::MAX), &Tipo::Real),
            Err(ErrorCelda::NoCabe(_))
        ));
        assert!(matches!(
            celda(&Valor::Entero(1), &Tipo::Otro("BSON".into())),
            Err(ErrorCelda::NoCabe(_))
        ));
    }

    #[test]
    fn un_lote_de_arrow_con_todo() {
        let cols = vec![
            ("id".to_string(), Tipo::Entero),
            (
                "importe".to_string(),
                Tipo::Decimal {
                    precision: 12,
                    escala: 2,
                },
            ),
            ("creado".to_string(), Tipo::Instante),
            ("dia".to_string(), Tipo::Fecha),
            ("hora".to_string(), Tipo::Hora),
            ("nota".to_string(), Tipo::Texto),
        ];
        let mut c = Constructor::nuevo(&cols).unwrap();
        c.push(&[
            Valor::Entero(1),
            Valor::Decimal("1.25".into()),
            Valor::Entero(1000),
            Valor::Entero(20_746),
            Valor::Entero(1000),
            Valor::Texto("a".into()),
        ])
        .unwrap();
        c.push(&[
            Valor::Entero(2),
            Valor::Nulo,
            Valor::Nulo,
            Valor::Nulo,
            Valor::Nulo,
            Valor::Nulo,
        ])
        .unwrap();
        assert_eq!(c.filas(), 2);
        let s = c.schema();
        assert_eq!(s.field(1).data_type(), &DataType::Decimal128(12, 2));
        assert_eq!(
            s.field(2).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into()))
        );
        assert_eq!(
            s.field(1).metadata().get("ore:tipo").map(String::as_str),
            Some("Decimal<12, 2>")
        );
        let b = c.finish().unwrap();
        assert_eq!(c.filas(), 0);
        assert_eq!(b.num_rows(), 2);
        assert_eq!(b.column(0).as_primitive::<Int64Type>().value(0), 1);
        assert_eq!(b.column(1).as_primitive::<Decimal128Type>().value(0), 125);
        assert!(b.column(1).is_null(1));
        assert_eq!(
            b.column(2)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_000_000
        );
        assert_eq!(b.column(3).as_primitive::<Date32Type>().value(0), 20_746);
        assert_eq!(
            b.column(4).as_primitive::<Time64MicrosecondType>().value(0),
            1_000_000
        );
        assert_eq!(b.column(5).as_string::<i32>().value(0), "a");
    }

    #[test]
    fn el_error_de_un_valor_nombra_columna_y_fila() {
        let cols = vec![(
            "importe".to_string(),
            Tipo::Decimal {
                precision: 5,
                escala: 2,
            },
        )];
        let mut c = Constructor::nuevo(&cols).unwrap();
        c.push(&[Valor::Decimal("1.25".into())]).unwrap();
        let e = c.push(&[Valor::Decimal("1.255".into())]).unwrap_err();
        let m = e.to_string();
        assert!(
            m.contains("columna `importe`") && m.contains("fila 1") && m.contains("no se redondea"),
            "{m}"
        );
        // Y es `esquema`, no un fallo del origen.
        assert_eq!(
            ore_driver::Fallo::from(e).codigo,
            ore_driver::Codigo::Esquema
        );
        // Una fila con otro número de celdas es un fallo de Arrow, del origen.
        let e = c.push(&[]).unwrap_err();
        assert_eq!(
            ore_driver::Fallo::from(e).codigo,
            ore_driver::Codigo::Origen
        );
    }

    #[test]
    fn base64_de_la_rfc_4648() {
        for (e, s) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(e.as_bytes()), s);
        }
    }
}
