//! **De las filas del driver a un `RecordBatch` de Arrow** (ficha 0070 §4.3).
//!
//! El conector v2 contesta Arrow en flujo, un lote cada pocos miles de filas
//! mientras se lee (`docs/federation.md` §1.3). Cada celda llega como un
//! [`CqlValue`] del driver y se convierte a su [`Valor`] de OOS ([`celda`]); el
//! mismo `Valor` sirve al [`crate::residual`] y a los constructores de Arrow.
//!
//! # La regla que manda
//!
//! **Un valor que no cabe en el tipo de su columna es un error que nombra columna
//! y fila; no se convierte en nulo ni se redondea** (0032 §4):
//!
//! - un `time` con nanosegundos sueltos no cabe en microsegundos;
//! - un `decimal` o un `varint` declarado `Decimal<p, s>` que pierde cifras o pasa
//!   de `p` no cabe;
//! - un `varint` declarado `Integer` que pasa de 64 bits no cabe;
//! - una celda de una clase que la columna no admite es `Inesperado`.
//!
//! Sin declaración, `varint` y `decimal` viajan como **texto exacto** (`String`).

use crate::tipos::Tipo;
use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use ore_base::tipos::{Fisico, Valor};
use scylla::value::CqlValue;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

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
            } => write!(
                f,
                "columna `{columna}`, fila {fila}: se esperaba {esperado} y llegó {visto}"
            ),
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
    /// Un valor que no cabe es un cambio de esquema que quien sirve tiene que
    /// decidir (`esquema`); un fallo de Arrow es del origen.
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

fn nombre(v: &CqlValue) -> &'static str {
    match v {
        CqlValue::Ascii(_) => "ascii",
        CqlValue::Text(_) => "text",
        CqlValue::TinyInt(_) => "tinyint",
        CqlValue::SmallInt(_) => "smallint",
        CqlValue::Int(_) => "int",
        CqlValue::BigInt(_) => "bigint",
        CqlValue::Counter(_) => "counter",
        CqlValue::Varint(_) => "varint",
        CqlValue::Float(_) => "float",
        CqlValue::Double(_) => "double",
        CqlValue::Decimal(_) => "decimal",
        CqlValue::Boolean(_) => "boolean",
        CqlValue::Blob(_) => "blob",
        CqlValue::Inet(_) => "inet",
        CqlValue::Timestamp(_) => "timestamp",
        CqlValue::Date(_) => "date",
        CqlValue::Time(_) => "time",
        CqlValue::Duration(_) => "duration",
        CqlValue::Uuid(_) => "uuid",
        CqlValue::Timeuuid(_) => "timeuuid",
        CqlValue::List(_) => "list",
        CqlValue::Set(_) => "set",
        CqlValue::Map(_) => "map",
        CqlValue::Tuple(_) => "tuple",
        CqlValue::UserDefinedType { .. } => "udt",
        CqlValue::Vector(_) => "vector",
        CqlValue::Empty => "empty",
        _ => "otro",
    }
}

fn inesperado(esperado: &'static str, v: &CqlValue) -> ErrorCelda {
    ErrorCelda::Inesperado {
        esperado,
        visto: nombre(v),
    }
}

/// El entero de un `tinyint`, `smallint`, `int`, `bigint` o `counter`.
fn entero(v: &CqlValue) -> Option<i64> {
    Some(match v {
        CqlValue::TinyInt(n) => i64::from(*n),
        CqlValue::SmallInt(n) => i64::from(*n),
        CqlValue::Int(n) => i64::from(*n),
        CqlValue::BigInt(n) => *n,
        CqlValue::Counter(c) => c.0,
        _ => return None,
    })
}

/// **La celda de una columna, como el `Valor` de OOS que le toca.** `String` y lo
/// que viaja como texto (`uuid`, `varint`, `decimal`, `blob`, colecciones…) salen
/// como [`Valor::Texto`].
pub fn celda(v: &CqlValue, tipo: &Tipo) -> Result<Valor, ErrorCelda> {
    match tipo {
        Tipo::Texto => match v {
            CqlValue::Ascii(s) | CqlValue::Text(s) => Ok(Valor::Texto(s.clone())),
            otro => Err(inesperado("text", otro)),
        },
        Tipo::Entero => {
            if let Some(n) = entero(v) {
                return Ok(Valor::Entero(n));
            }
            match v {
                // Un `varint` que la tabla declara `Integer`.
                CqlValue::Varint(x) => {
                    let b = x.as_signed_bytes_be_slice();
                    i128_de(b)
                        .and_then(|n| i64::try_from(n).ok())
                        .map(Valor::Entero)
                        .ok_or_else(|| {
                            ErrorCelda::NoCabe(format!(
                                "el `varint` {} no cabe en un entero de 64 bits",
                                entero_a_texto(b)
                            ))
                        })
                }
                otro => Err(inesperado("un entero", otro)),
            }
        }
        Tipo::Real => match v {
            CqlValue::Float(x) => Ok(Valor::Real(f64::from(*x))),
            CqlValue::Double(x) => Ok(Valor::Real(*x)),
            otro => match entero(otro) {
                // Un entero que la tabla declara `Float`: solo si es exacto en un `f64`.
                Some(n) if n.unsigned_abs() <= (1u64 << 53) => Ok(Valor::Real(n as f64)),
                Some(n) => Err(ErrorCelda::NoCabe(format!(
                    "el entero {n} no es exacto como `Float` (pasa de 2^53)"
                ))),
                None => Err(inesperado("un número", otro)),
            },
        },
        Tipo::Logico => match v {
            CqlValue::Boolean(b) => Ok(Valor::Logico(*b)),
            otro => Err(inesperado("boolean", otro)),
        },
        Tipo::Instante => match v {
            CqlValue::Timestamp(t) => {
                t.0.checked_mul(1000).map(Valor::Instante).ok_or_else(|| {
                    ErrorCelda::NoCabe("el instante no cabe en microsegundos".into())
                })
            }
            otro => Err(inesperado("timestamp", otro)),
        },
        Tipo::FechaHora => match v {
            CqlValue::Timestamp(t) => {
                t.0.checked_mul(1000)
                    .map(Valor::FechaHora)
                    .ok_or_else(|| ErrorCelda::NoCabe("la hora no cabe en microsegundos".into()))
            }
            otro => Err(inesperado("timestamp", otro)),
        },
        Tipo::Fecha => match v {
            CqlValue::Date(d) => Ok(Valor::Fecha(dias(d.0))),
            otro => Err(inesperado("date", otro)),
        },
        Tipo::Hora => match v {
            CqlValue::Time(t) if t.0 % 1000 == 0 => Ok(Valor::Hora(t.0 / 1000)),
            CqlValue::Time(t) => Err(ErrorCelda::NoCabe(format!(
                "el `time` {} tiene nanosegundos sueltos y OOS guarda microsegundos",
                hora_cql(t.0)
            ))),
            otro => Err(inesperado("time", otro)),
        },
        Tipo::Decimal { precision, escala } => {
            let (sin_escala, exp) = match v {
                CqlValue::Decimal(d) => {
                    let (b, e) = d.as_signed_be_bytes_slice_and_exponent();
                    (b.to_vec(), i64::from(e))
                }
                CqlValue::Varint(x) => (x.as_signed_bytes_be_slice().to_vec(), 0),
                otro => match entero(otro) {
                    Some(n) => (n.to_be_bytes().to_vec(), 0),
                    None => return Err(inesperado("decimal", otro)),
                },
            };
            a_escala(&sin_escala, exp, *precision, *escala).map(Valor::Decimal)
        }
        Tipo::Varint => match v {
            CqlValue::Varint(x) => Ok(Valor::Texto(entero_a_texto(x.as_signed_bytes_be_slice()))),
            otro => Err(inesperado("varint", otro)),
        },
        Tipo::DecimalLibre => match v {
            CqlValue::Decimal(d) => {
                let (b, e) = d.as_signed_be_bytes_slice_and_exponent();
                Ok(Valor::Texto(decimal_a_texto(b, i64::from(e))))
            }
            otro => Err(inesperado("decimal", otro)),
        },
        Tipo::Binario => match v {
            CqlValue::Blob(b) => Ok(Valor::Texto(base64(b))),
            otro => Err(inesperado("blob", otro)),
        },
        Tipo::Uuid => match v {
            CqlValue::Uuid(u) => Ok(Valor::Texto(u.to_string())),
            CqlValue::Timeuuid(u) => Ok(Valor::Texto(u.to_string())),
            otro => Err(inesperado("uuid", otro)),
        },
        Tipo::Duracion => match v {
            CqlValue::Duration(d) => Ok(Valor::Texto(format!(
                "{}mo{}d{}ns",
                d.months, d.days, d.nanoseconds
            ))),
            otro => Err(inesperado("duration", otro)),
        },
        Tipo::Inet => match v {
            CqlValue::Inet(ip) => Ok(Valor::Texto(ip.to_string())),
            otro => Err(inesperado("inet", otro)),
        },
        Tipo::Json => match v {
            CqlValue::List(_)
            | CqlValue::Set(_)
            | CqlValue::Map(_)
            | CqlValue::Tuple(_)
            | CqlValue::UserDefinedType { .. }
            | CqlValue::Vector(_) => Ok(Valor::Texto(json(v).to_string())),
            otro => Err(inesperado("una colección, tupla o UDT", otro)),
        },
    }
}

/// Días desde 1970-01-01 de un `date` de CQL (que cuenta desde 2^31).
fn dias(d: u32) -> i32 {
    (i64::from(d) - (1i64 << 31)) as i32
}

fn hora_cql(ns: i64) -> String {
    let s = ns / 1_000_000_000;
    format!(
        "{:02}:{:02}:{:02}.{:09}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        ns % 1_000_000_000
    )
}

// ── Enteros y decimales de tamaño libre ─────────────────────────────────────

/// El entero en complemento a dos, big-endian, si cabe en un `i128`.
fn i128_de(b: &[u8]) -> Option<i128> {
    if b.is_empty() {
        return Some(0);
    }
    // Los ceros (o unos) de relleno por la izquierda no cuentan.
    let signo = if b[0] & 0x80 != 0 { 0xFF } else { 0x00 };
    let significativos = b
        .iter()
        .position(|x| *x != signo)
        .map_or(0, |i| b.len() - i);
    // Sin el byte de signo propio, tienen que caber 16.
    if significativos > 16
        || (significativos == 16 && (b[b.len() - 16] & 0x80 != 0) != (signo == 0xFF))
    {
        return None;
    }
    let mut ext = [signo; 16];
    let n = b.len().min(16);
    ext[16 - n..].copy_from_slice(&b[b.len() - n..]);
    Some(i128::from_be_bytes(ext))
}

/// El texto decimal de un entero en complemento a dos, big-endian, de cualquier
/// tamaño: divisiones sucesivas por 10⁹ sobre la magnitud.
pub fn entero_a_texto(b: &[u8]) -> String {
    if b.is_empty() {
        return "0".into();
    }
    let negativo = b[0] & 0x80 != 0;
    let mut mag: Vec<u8> = b.to_vec();
    if negativo {
        // Complemento a dos: invertir y sumar uno.
        for x in &mut mag {
            *x = !*x;
        }
        for x in mag.iter_mut().rev() {
            let (n, acarreo) = x.overflowing_add(1);
            *x = n;
            if !acarreo {
                break;
            }
        }
    }
    // Limbs de 32 bits, big-endian.
    let mut limbs: Vec<u32> = Vec::with_capacity(mag.len() / 4 + 1);
    let resto = mag.len() % 4;
    if resto > 0 {
        let mut w = [0u8; 4];
        w[4 - resto..].copy_from_slice(&mag[..resto]);
        limbs.push(u32::from_be_bytes(w));
    }
    for c in mag[resto..].as_chunks::<4>().0 {
        limbs.push(u32::from_be_bytes([c[0], c[1], c[2], c[3]]));
    }
    let mut trozos: Vec<u32> = Vec::new();
    while limbs.iter().any(|l| *l != 0) {
        let mut r = 0u64;
        for l in &mut limbs {
            let cur = (r << 32) | u64::from(*l);
            *l = (cur / 1_000_000_000) as u32;
            r = cur % 1_000_000_000;
        }
        trozos.push(r as u32);
    }
    let mut s = String::new();
    if negativo && !trozos.is_empty() {
        s.push('-');
    }
    match trozos.pop() {
        None => return "0".into(),
        Some(primero) => s.push_str(&primero.to_string()),
    }
    for t in trozos.iter().rev() {
        s.push_str(&format!("{t:09}"));
    }
    s
}

/// El texto exacto de un `decimal` de CQL: el entero sin escala y su exponente
/// (`valor = entero × 10^-escala`). `1.2300` conserva sus ceros.
pub fn decimal_a_texto(b: &[u8], escala: i64) -> String {
    let entero = entero_a_texto(b);
    if escala <= 0 {
        // Una escala negativa multiplica: `5E+3` sería 5000.
        let ceros = usize::try_from(-escala).unwrap_or(0);
        return if entero == "0" {
            "0".into()
        } else {
            format!("{entero}{}", "0".repeat(ceros))
        };
    }
    let escala = escala as usize;
    let (signo, digitos) = match entero.strip_prefix('-') {
        Some(d) => ("-", d.to_string()),
        None => ("", entero),
    };
    let relleno = if digitos.len() <= escala {
        format!("{}{digitos}", "0".repeat(escala + 1 - digitos.len()))
    } else {
        digitos
    };
    let corte = relleno.len() - escala;
    format!("{signo}{}.{}", &relleno[..corte], &relleno[corte..])
}

/// El decimal `sin_escala × 10^-exponente` como el entero escalado de
/// `Decimal<precision, escala>`, **o no cabe**: nunca se redondea ni se pierden
/// cifras, ni pasa de `precision` dígitos.
fn a_escala(b: &[u8], exponente: i64, precision: u8, escala: u8) -> Result<i128, ErrorCelda> {
    let no = |m: String| Err(ErrorCelda::NoCabe(m));
    let Some(n) = i128_de(b) else {
        return no(format!(
            "{} no cabe en un `Decimal<{precision}, {escala}>`",
            decimal_a_texto(b, exponente)
        ));
    };
    let objetivo = i64::from(escala);
    let diferencia = objetivo - exponente;
    let r = if diferencia >= 0 {
        let f = u32::try_from(diferencia)
            .ok()
            .and_then(|d| 10i128.checked_pow(d));
        f.and_then(|f| n.checked_mul(f))
    } else {
        let f = u32::try_from(-diferencia)
            .ok()
            .and_then(|d| 10i128.checked_pow(d));
        match f {
            Some(f) if n % f == 0 => Some(n / f),
            // Perdería cifras: `10.505` no cabe en dos decimales.
            _ => None,
        }
    };
    let Some(r) = r else {
        return no(format!(
            "{} no cabe en un `Decimal<{precision}, {escala}>` sin perder cifras",
            decimal_a_texto(b, exponente)
        ));
    };
    if r.unsigned_abs() >= 10u128.pow(u32::from(precision)) {
        return no(format!(
            "{} pasa de {precision} cifras (`Decimal<{precision}, {escala}>`)",
            decimal_a_texto(b, exponente)
        ));
    }
    Ok(r)
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for trozo in bytes.chunks(3) {
        let n = (u32::from(trozo[0]) << 16)
            | (u32::from(*trozo.get(1).unwrap_or(&0)) << 8)
            | u32::from(*trozo.get(2).unwrap_or(&0));
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if trozo.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if trozo.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

// ── Lo compuesto, como JSON ─────────────────────────────────────────────────

/// El JSON de una celda. Un mapa es un **objeto** con la clave en texto (`{"1": …}`
/// aunque la clave sea un entero), una lista, un conjunto o un `vector` es un array,
/// una tupla es un array con `null` donde falta, y un UDT es un objeto con sus
/// campos. Un `decimal` y un `varint` van como texto exacto; un `blob`, en base64.
fn json(v: &CqlValue) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        CqlValue::Ascii(s) | CqlValue::Text(s) => J::String(s.clone()),
        CqlValue::Boolean(b) => J::Bool(*b),
        CqlValue::Float(x) => numero(f64::from(*x)),
        CqlValue::Double(x) => numero(*x),
        CqlValue::Varint(x) => J::String(entero_a_texto(x.as_signed_bytes_be_slice())),
        CqlValue::Decimal(d) => {
            let (b, e) = d.as_signed_be_bytes_slice_and_exponent();
            J::String(decimal_a_texto(b, i64::from(e)))
        }
        CqlValue::Blob(b) => J::String(base64(b)),
        CqlValue::Inet(ip) => J::String(ip.to_string()),
        CqlValue::Uuid(u) => J::String(u.to_string()),
        CqlValue::Timeuuid(u) => J::String(u.to_string()),
        CqlValue::Timestamp(t) => {
            J::String(Valor::Instante(t.0.saturating_mul(1000)).texto(&Fisico::Instante))
        }
        CqlValue::Date(d) => J::String(Valor::Fecha(dias(d.0)).texto(&Fisico::Fecha)),
        CqlValue::Time(t) => J::String(hora_cql(t.0)),
        CqlValue::Duration(d) => J::String(format!("{}mo{}d{}ns", d.months, d.days, d.nanoseconds)),
        CqlValue::List(l) | CqlValue::Set(l) | CqlValue::Vector(l) => {
            J::Array(l.iter().map(json).collect())
        }
        CqlValue::Tuple(l) => {
            J::Array(l.iter().map(|x| x.as_ref().map_or(J::Null, json)).collect())
        }
        CqlValue::Map(pares) => J::Object(
            pares
                .iter()
                .map(|(k, x)| (clave_de_mapa(k), json(x)))
                .collect(),
        ),
        CqlValue::UserDefinedType { fields, .. } => J::Object(
            fields
                .iter()
                .map(|(n, x)| (n.clone(), x.as_ref().map_or(J::Null, json)))
                .collect(),
        ),
        otro => match entero(otro) {
            Some(n) => J::Number(n.into()),
            None => J::Null,
        },
    }
}

/// `NaN` e infinitos no son JSON: van como texto, que es lo que SQL espera.
fn numero(x: f64) -> serde_json::Value {
    match serde_json::Number::from_f64(x) {
        Some(n) => serde_json::Value::Number(n),
        None if x.is_nan() => serde_json::Value::String("NaN".into()),
        None if x > 0.0 => serde_json::Value::String("Infinity".into()),
        None => serde_json::Value::String("-Infinity".into()),
    }
}

fn clave_de_mapa(k: &CqlValue) -> String {
    match json(k) {
        serde_json::Value::String(s) => s,
        otro => otro.to_string(),
    }
}

// ── Arrow ───────────────────────────────────────────────────────────────────

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

struct Campo {
    propiedad: String,
    tipo: Tipo,
    col: Col,
}

/// Construye lotes de Arrow a partir de las filas del driver.
pub struct Constructor {
    schema: SchemaRef,
    campos: Vec<Campo>,
    /// Filas del lote en curso.
    filas: usize,
    /// Filas vistas desde el principio: para decir dónde falló.
    total: u64,
}

impl Constructor {
    /// `(propiedad, tipo)` por columna, en el orden de la proyección. La propiedad
    /// es el nombre que sale en el lote (0008: no la columna física).
    pub fn nuevo(columnas: &[(String, Tipo)]) -> Result<Constructor, ErrorLote> {
        let mut campos = Vec::new();
        let mut fields = Vec::new();
        for (propiedad, tipo) in columnas {
            let (dt, col) = match tipo {
                Tipo::Entero => (DataType::Int64, Col::Entero(Int64Builder::new())),
                Tipo::Real => (DataType::Float64, Col::Real(Float64Builder::new())),
                Tipo::Decimal { precision, escala } => (
                    DataType::Decimal128(*precision, *escala as i8),
                    Col::Decimal(
                        Decimal128Builder::new()
                            .with_precision_and_scale(*precision, *escala as i8)
                            .map_err(|e| ErrorLote::Arrow(e.to_string()))?,
                    ),
                ),
                Tipo::Logico => (DataType::Boolean, Col::Logico(BooleanBuilder::new())),
                Tipo::Instante => (
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    Col::Instante(TimestampMicrosecondBuilder::new().with_timezone("UTC")),
                ),
                Tipo::Fecha => (DataType::Date32, Col::Fecha(Date32Builder::new())),
                Tipo::FechaHora => (
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    Col::FechaHora(TimestampMicrosecondBuilder::new()),
                ),
                Tipo::Hora => (
                    DataType::Time64(TimeUnit::Microsecond),
                    Col::Hora(Time64MicrosecondBuilder::new()),
                ),
                _ => (DataType::Utf8, Col::Texto(StringBuilder::new())),
            };
            let meta: HashMap<String, String> = [("ore:tipo".to_string(), tipo.oos())].into();
            fields.push(Field::new(propiedad, dt, true).with_metadata(meta));
            campos.push(Campo {
                propiedad: propiedad.clone(),
                tipo: tipo.clone(),
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

    /// Añade una fila: una celda por columna, en el orden de [`Constructor::nuevo`];
    /// `None` es nulo. Si falla, el lote en curso queda con las columnas a medias:
    /// quien lo recibe corta la lectura y no lo sella.
    pub fn push(&mut self, fila: &[Option<CqlValue>]) -> Result<(), ErrorLote> {
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
                None | Some(CqlValue::Empty) => None,
                Some(v) => Some(celda(v, &c.tipo).map_err(|e| e.en(&c.propiedad, n))?),
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
                    Col::Fecha(b) => Arc::new(b.finish()),
                    Col::FechaHora(b) => Arc::new(b.finish()),
                    Col::Hora(b) => Arc::new(b.finish()),
                    Col::Texto(b) => Arc::new(b.finish()),
                }
            })
            .collect();
        self.filas = 0;
        RecordBatch::try_new(self.schema.clone(), arrays)
            .map_err(|e| ErrorLote::Arrow(e.to_string()))
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
        (Col::Hora(b), None) => b.append_null(),
        (Col::Hora(b), Some(Valor::Hora(x))) => b.append_value(x),
        (Col::Texto(b), None) => b.append_null(),
        (Col::Texto(b), Some(Valor::Texto(x))) => b.append_value(x),
        _ => return Err(raro()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{
        Date32Type, Decimal128Type, Int64Type, Time64MicrosecondType, TimestampMicrosecondType,
    };
    use scylla::value::{CqlDate, CqlDecimal, CqlDuration, CqlTime, CqlTimestamp, CqlVarint};

    fn varint(b: &[u8]) -> CqlValue {
        CqlValue::Varint(CqlVarint::from_signed_bytes_be(b.to_vec()))
    }

    fn decimal(b: &[u8], escala: i32) -> CqlValue {
        CqlValue::Decimal(CqlDecimal::from_signed_be_bytes_and_exponent(
            b.to_vec(),
            escala,
        ))
    }

    #[test]
    fn un_entero_de_cualquier_tamano_sale_exacto() {
        // El `varint` de la semilla de X0: 123456789012345678901234567890.
        let b = [1, 142, 233, 15, 246, 195, 115, 224, 238, 78, 63, 10, 210];
        assert_eq!(entero_a_texto(&b), "123456789012345678901234567890");
        assert_eq!(entero_a_texto(&[]), "0");
        assert_eq!(entero_a_texto(&[0]), "0");
        assert_eq!(entero_a_texto(&[0x7F]), "127");
        assert_eq!(entero_a_texto(&[0xFF]), "-1");
        assert_eq!(entero_a_texto(&[0x80]), "-128");
        assert_eq!(entero_a_texto(&[0x00, 0x80]), "128");
        assert_eq!(entero_a_texto(&[0xFF, 0x7F]), "-129");
        // 2^64 y -2^64: pasan de 64 bits y los limbs de 32 no estorban.
        assert_eq!(
            entero_a_texto(&[1, 0, 0, 0, 0, 0, 0, 0, 0]),
            "18446744073709551616"
        );
        assert_eq!(
            entero_a_texto(&[0xFF, 0, 0, 0, 0, 0, 0, 0, 0]),
            "-18446744073709551616"
        );
        // 10^30, que ocupa 100 bits.
        assert_eq!(
            entero_a_texto(&[
                0x0C, 0x9F, 0x2C, 0x9C, 0xD0, 0x46, 0x74, 0xED, 0xEA, 0x40, 0x00, 0x00, 0x00
            ]),
            "1000000000000000000000000000000"
        );
    }

    #[test]
    fn i128_distingue_lo_que_cabe() {
        assert_eq!(i128_de(&[0xFF]), Some(-1));
        assert_eq!(i128_de(&[0x00, 0xFF]), Some(255));
        assert_eq!(i128_de(&[]), Some(0));
        let mut cabe = vec![0x7Fu8];
        cabe.extend([0xFF; 15]);
        assert_eq!(i128_de(&cabe), Some(i128::MAX));
        let mut no_cabe = vec![0x01u8];
        no_cabe.extend([0u8; 16]);
        assert_eq!(i128_de(&no_cabe), None);
        let mut minimo = vec![0x80u8];
        minimo.extend([0u8; 15]);
        assert_eq!(i128_de(&minimo), Some(i128::MIN));
    }

    #[test]
    fn el_decimal_libre_conserva_su_escala() {
        // 12345678901234567890.123456789 (la semilla de X0): 9 decimales.
        let b = [39, 228, 27, 50, 70, 190, 201, 177, 110, 57, 129, 21];
        assert_eq!(decimal_a_texto(&b, 9), "12345678901234567890.123456789");
        assert_eq!(decimal_a_texto(&[0x01, 0x2C], 2), "3.00");
        assert_eq!(decimal_a_texto(&[0x05], 3), "0.005");
        assert_eq!(decimal_a_texto(&[0xFB], 3), "-0.005");
        assert_eq!(decimal_a_texto(&[0xFF, 0x85], 2), "-1.23");
        assert_eq!(decimal_a_texto(&[0x05], 0), "5");
        assert_eq!(decimal_a_texto(&[0x05], -3), "5000");
        assert_eq!(decimal_a_texto(&[0], -3), "0");
    }

    #[test]
    fn un_decimal_declarado_cabe_o_se_dice_por_que_no() {
        let t = Tipo::Decimal {
            precision: 12,
            escala: 2,
        };
        // 10.5 (105 con escala 1) → 1050 con escala 2: exacto.
        assert_eq!(celda(&decimal(&[105], 1), &t), Ok(Valor::Decimal(1050)));
        // 10.50 (1050 con escala 2) → igual.
        assert_eq!(
            celda(&decimal(&[0x04, 0x1A], 2), &t),
            Ok(Valor::Decimal(1050))
        );
        // 10.505 perdería una cifra.
        let e = celda(&decimal(&[0x29, 0xF9], 3), &t).unwrap_err();
        assert!(
            matches!(&e, ErrorCelda::NoCabe(m) if m.contains("sin perder cifras")),
            "{e:?}"
        );
        // Pasa de 12 cifras.
        let grande =
            celda(&decimal(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 0), &t).unwrap_err();
        assert!(
            matches!(&grande, ErrorCelda::NoCabe(m) if m.contains("pasa de 12 cifras")),
            "{grande:?}"
        );
        // Un entero y un varint también se escalan.
        assert_eq!(celda(&CqlValue::Int(7), &t), Ok(Valor::Decimal(700)));
        assert_eq!(celda(&varint(&[0x05]), &t), Ok(Valor::Decimal(500)));
    }

    #[test]
    fn un_varint_declarado_entero_cabe_o_no() {
        assert_eq!(
            celda(&varint(&[0x2A]), &Tipo::Entero),
            Ok(Valor::Entero(42))
        );
        let grande = [1, 142, 233, 15, 246, 195, 115, 224, 238, 78, 63, 10, 210];
        let e = celda(&varint(&grande), &Tipo::Entero).unwrap_err();
        assert!(
            matches!(&e, ErrorCelda::NoCabe(m) if m.contains("123456789012345678901234567890")),
            "{e:?}"
        );
        // Sin declarar, viaja exacto como texto.
        assert_eq!(
            celda(&varint(&grande), &Tipo::Varint),
            Ok(Valor::Texto("123456789012345678901234567890".into()))
        );
    }

    #[test]
    fn el_tiempo_no_pierde_nanosegundos_en_silencio() {
        let micro = CqlValue::Time(CqlTime(45_296_123_456_000));
        assert_eq!(celda(&micro, &Tipo::Hora), Ok(Valor::Hora(45_296_123_456)));
        let nano = CqlValue::Time(CqlTime(45_296_123_456_789));
        let e = celda(&nano, &Tipo::Hora).unwrap_err();
        assert!(
            matches!(&e, ErrorCelda::NoCabe(m) if m.contains("12:34:56.123456789")),
            "{e:?}"
        );
    }

    #[test]
    fn fechas_e_instantes_de_la_semilla() {
        // 2026-10-20: 20 746 días desde 1970; CQL cuenta desde 2^31.
        let d = CqlValue::Date(CqlDate(2_147_504_394));
        assert_eq!(celda(&d, &Tipo::Fecha), Ok(Valor::Fecha(20_746)));
        assert_eq!(Valor::Fecha(20_746).texto(&Fisico::Fecha), "2026-10-20");
        let t = CqlValue::Timestamp(CqlTimestamp(1_792_499_696_789));
        assert_eq!(
            celda(&t, &Tipo::Instante),
            Ok(Valor::Instante(1_792_499_696_789_000))
        );
        // Antes de 1970 también.
        let antes = CqlValue::Date(CqlDate((1u32 << 31) - 1));
        assert_eq!(celda(&antes, &Tipo::Fecha), Ok(Valor::Fecha(-1)));
    }

    #[test]
    fn lo_compuesto_es_json_con_la_clave_en_texto() {
        let mapa = CqlValue::Map(vec![(CqlValue::Int(1), CqlValue::Text("a".into()))]);
        assert_eq!(
            celda(&mapa, &Tipo::Json),
            Ok(Valor::Texto(r#"{"1":"a"}"#.into()))
        );
        let lista = CqlValue::List(vec![CqlValue::Int(1), CqlValue::Int(2)]);
        assert_eq!(celda(&lista, &Tipo::Json), Ok(Valor::Texto("[1,2]".into())));
        let tupla = CqlValue::Tuple(vec![
            Some(CqlValue::Int(1)),
            None,
            Some(CqlValue::Text("z".into())),
        ]);
        assert_eq!(
            celda(&tupla, &Tipo::Json),
            Ok(Valor::Texto(r#"[1,null,"z"]"#.into()))
        );
        let udt = CqlValue::UserDefinedType {
            keyspace: "k".into(),
            name: "direccion".into(),
            fields: vec![
                ("calle".into(), Some(CqlValue::Text("Gran Via".into()))),
                ("num".into(), None),
                (
                    "geo".into(),
                    Some(CqlValue::Tuple(vec![
                        Some(CqlValue::Double(40.4)),
                        Some(CqlValue::Double(-3.7)),
                    ])),
                ),
            ],
        };
        let Ok(Valor::Texto(j)) = celda(&udt, &Tipo::Json) else {
            panic!()
        };
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["calle"], "Gran Via");
        assert_eq!(v["num"], serde_json::Value::Null);
        assert_eq!(v["geo"][1], -3.7);
        let vector = CqlValue::Vector(vec![CqlValue::Float(0.5), CqlValue::Float(1.0)]);
        assert_eq!(
            celda(&vector, &Tipo::Json),
            Ok(Valor::Texto("[0.5,1.0]".into()))
        );
        // Un decimal dentro de una colección no pierde cifras.
        let dec = CqlValue::List(vec![decimal(&[0x01, 0x2C], 2)]);
        assert_eq!(
            celda(&dec, &Tipo::Json),
            Ok(Valor::Texto(r#"["3.00"]"#.into()))
        );
    }

    #[test]
    fn duracion_blob_uuid_e_inet() {
        let d = CqlValue::Duration(CqlDuration {
            months: 14,
            days: 3,
            nanoseconds: 14_400_000_000_000,
        });
        assert_eq!(
            celda(&d, &Tipo::Duracion),
            Ok(Valor::Texto("14mo3d14400000000000ns".into()))
        );
        assert_eq!(
            celda(&CqlValue::Blob(vec![0xCA, 0xFE]), &Tipo::Binario),
            Ok(Valor::Texto("yv4=".into()))
        );
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        let ip = CqlValue::Inet("10.0.0.1".parse().unwrap());
        assert_eq!(celda(&ip, &Tipo::Inet), Ok(Valor::Texto("10.0.0.1".into())));
    }

    #[test]
    fn una_celda_de_otra_clase_se_dice() {
        let e = celda(&CqlValue::Text("x".into()), &Tipo::Entero).unwrap_err();
        assert_eq!(
            e,
            ErrorCelda::Inesperado {
                esperado: "un entero",
                visto: "text"
            }
        );
        assert!(celda(&CqlValue::Int(1), &Tipo::Logico).is_err());
        assert!(celda(&CqlValue::Int(1), &Tipo::Json).is_err());
    }

    #[test]
    fn el_lote_lleva_el_esquema_y_los_nulos() {
        let mut c = Constructor::nuevo(&[
            ("id".into(), Tipo::Entero),
            ("nombre".into(), Tipo::Texto),
            ("cuando".into(), Tipo::Instante),
            ("dia".into(), Tipo::Fecha),
            ("hora".into(), Tipo::Hora),
            (
                "total".into(),
                Tipo::Decimal {
                    precision: 12,
                    escala: 2,
                },
            ),
            ("tag".into(), Tipo::Varint),
        ])
        .unwrap();
        c.push(&[
            Some(CqlValue::Int(7)),
            Some(CqlValue::Text("ñandú 🦆".into())),
            Some(CqlValue::Timestamp(CqlTimestamp(1_792_499_696_789))),
            Some(CqlValue::Date(CqlDate(2_147_504_394))),
            Some(CqlValue::Time(CqlTime(1_000))),
            Some(decimal(&[105], 1)),
            Some(varint(&[0x2A])),
        ])
        .unwrap();
        c.push(&[
            Some(CqlValue::Int(8)),
            None,
            None,
            None,
            None,
            None,
            Some(CqlValue::Empty),
        ])
        .unwrap();
        assert_eq!(c.filas(), 2);
        let b = c.finish().unwrap();
        assert_eq!(c.filas(), 0);
        assert_eq!(b.num_rows(), 2);
        let s = b.schema();
        assert_eq!(s.field(0).data_type(), &DataType::Int64);
        assert_eq!(
            s.field(2).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        assert_eq!(s.field(5).data_type(), &DataType::Decimal128(12, 2));
        assert_eq!(
            s.field(4).data_type(),
            &DataType::Time64(TimeUnit::Microsecond)
        );
        assert_eq!(
            s.field(5).metadata().get("ore:tipo").map(String::as_str),
            Some("Decimal<12, 2>")
        );
        assert_eq!(b.column(0).as_primitive::<Int64Type>().value(0), 7);
        assert_eq!(b.column(1).as_string::<i32>().value(0), "ñandú 🦆");
        assert_eq!(
            b.column(2)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_792_499_696_789_000
        );
        assert_eq!(b.column(3).as_primitive::<Date32Type>().value(0), 20_746);
        assert_eq!(
            b.column(4).as_primitive::<Time64MicrosecondType>().value(0),
            1
        );
        assert_eq!(b.column(5).as_primitive::<Decimal128Type>().value(0), 1050);
        assert_eq!(b.column(6).as_string::<i32>().value(0), "42");
        for i in 1..7 {
            assert!(b.column(i).is_null(1), "columna {i}");
        }
    }

    #[test]
    fn un_valor_que_no_cabe_nombra_columna_y_fila() {
        let mut c = Constructor::nuevo(&[("hora".into(), Tipo::Hora)]).unwrap();
        c.push(&[Some(CqlValue::Time(CqlTime(1_000)))]).unwrap();
        let e = c.push(&[Some(CqlValue::Time(CqlTime(1_001)))]).unwrap_err();
        assert!(
            matches!(&e, ErrorLote::NoCabe { columna, fila: 1, .. } if columna == "hora"),
            "{e:?}"
        );
        assert!(e.to_string().contains("columna `hora`, fila 1"), "{e}");
        // Para quien sirve, es un cambio de esquema; un fallo de Arrow, del origen.
        let f: ore_driver::Fallo = e.into();
        assert_eq!(f.codigo, ore_driver::Codigo::Esquema);
        let a: ore_driver::Fallo = ErrorLote::Arrow("x".into()).into();
        assert_eq!(a.codigo, ore_driver::Codigo::Origen);
        let mal = c.push(&[]).unwrap_err();
        assert!(matches!(mal, ErrorLote::Arrow(_)));
    }
}
