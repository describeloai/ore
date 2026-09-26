//! **El contrato de tipos como código** — [ADR 0032](../../../docs/decisions/0032-el-contrato-de-tipos.md).
//!
//! Tres cosas, y las tres en un sitio para que no haya dos tablas que digan lo
//! mismo con distinta letra:
//!
//! 1. **El físico** de cada escalar de OOS: en qué tipo de Arrow/Parquet vive
//!    una columna de la copia ([`Fisico::de`]). Es la tabla de 0032 §1, columna
//!    «Arrow / Parquet».
//! 2. **La forma canónica del texto**: cómo se escribe un valor de cada escalar
//!    cuando viaja como cadena —que es como lo entrega el protocolo del driver—
//!    y cómo se analiza de vuelta ([`Fisico::analizar`], [`Valor::texto`]).
//!    Son las formas que `ore-read-postgres::texto` emite: ISO 8601 con espacio
//!    entre fecha y hora, `+00` en un instante, `true`/`false`, el decimal con
//!    sus dígitos y nada más.
//! 3. **La regla de lo que no analiza**: no se inventa. Quien estrecha una
//!    columna llama a [`Fisico::analizar`] valor a valor, y si uno no cabe **la
//!    columna entera queda como texto y se dice**. Aquí solo se contesta
//!    `None`; decidir y decir es de quien sella.
//!
//! # Por qué el físico no es `arrow_schema::DataType`
//!
//! El núcleo no enlaza Arrow: es L0, lo usa el compilador, `ore-serve` y la
//! consola de línea, y ninguno escribe Parquet. `Fisico` es el vocabulario
//! mínimo que la tabla necesita; `ore-store::carga` lo traduce a Arrow en una
//! función de diez líneas, que es donde Arrow ya está.
//!
//! # Lo que aquí NO hay
//!
//! `struct`, `map`, `uint64` y `timestamp[ns]` no salen de ningún driver y no
//! tienen escalar en OOS; `Opaque` viaja como texto mientras la cabecera de la
//! copia no lleve el tipo físico del origen (un `bytea` es `binary`; un `json`
//! es texto), y `list<T>` también, porque el protocolo del driver entrega
//! escalares y una lista en texto no tiene forma canónica acordada. La tabla de
//! 0032 lo dice; esto es lo que hay hecho.

use crate::types::Type;

/// Precisión y escala por defecto de un `Decimal` cuyo origen no las dice: la
/// misma que Foundry (0032, «Lo mirado» 4). Cabe cualquier `numeric` con hasta
/// 20 dígitos enteros y 18 decimales; lo que no quepa se queda texto y se dice.
pub const DECIMAL_POR_DEFECTO: (u8, u8) = (38, 18);

/// En qué tipo de Arrow/Parquet vive una columna de la copia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fisico {
    /// `string`. Lo que llega sin tipo, `String`, `Opaque` y `list<T>` (por
    /// ahora: arriba).
    Texto,
    /// `int64`.
    Entero,
    /// `float64`.
    Real,
    /// `bool`.
    Logico,
    /// `decimal128(precision, escala)`, exacto.
    Decimal { precision: u8, escala: u8 },
    /// `date32`: días desde 1970-01-01.
    Fecha,
    /// `time64[us]`: microsegundos desde la medianoche.
    Hora,
    /// `timestamp[us]` sin zona: **hora de pared**, microsegundos desde
    /// 1970-01-01T00:00 en el calendario, sin decir dónde.
    FechaHora,
    /// `timestamp[us, UTC]`: **un instante**. La zona del origen no se guarda;
    /// se convierte al leer el texto y se escribe en UTC.
    Instante,
}

impl Fisico {
    /// El físico de un tipo de OOS. Es la tabla de 0032 §1.
    pub fn de(tipo: &Type) -> Fisico {
        match tipo {
            Type::Scalar(s) => match s.as_str() {
                "Integer" => Fisico::Entero,
                "Decimal" => Fisico::Decimal {
                    precision: DECIMAL_POR_DEFECTO.0,
                    escala: DECIMAL_POR_DEFECTO.1,
                },
                "Float" => Fisico::Real,
                "Boolean" => Fisico::Logico,
                "Date" => Fisico::Fecha,
                "Time" => Fisico::Hora,
                "DateTime" => Fisico::FechaHora,
                "DateTimeTz" => Fisico::Instante,
                _ => Fisico::Texto,
            },
            // `Money<EUR, 2>` y `Quantity<km, 1>`: la precisión del tipo es la
            // escala del decimal, y la unidad es metadato del campo (0032 §1).
            Type::Parametric { precision, .. } => Fisico::Decimal {
                precision: DECIMAL_POR_DEFECTO.0,
                escala: (*precision).min(u32::from(DECIMAL_POR_DEFECTO.1)) as u8,
            },
            // La precisión y la escala del tipo, no las de por defecto (0032 T4).
            // Con `(38, 18)` un NUMERIC de BigQuery de más de 20 cifras enteras
            // no cabía y la columna entera se quedaba como texto (medido).
            Type::Decimal { precision, escala } => Fisico::Decimal {
                precision: *precision,
                escala: *escala,
            },
            Type::List(_) | Type::Imported(_) => Fisico::Texto,
        }
    }

    /// El nombre del tipo de Arrow, como lo escribe `pyarrow` (`str(field.type)`).
    /// Es lo que el informe y las medidas enseñan; `ore-store` construye el
    /// `DataType` de verdad a partir del enum, no de este texto.
    pub fn arrow(&self) -> String {
        match self {
            Fisico::Texto => "string".into(),
            Fisico::Entero => "int64".into(),
            Fisico::Real => "double".into(),
            Fisico::Logico => "bool".into(),
            Fisico::Decimal { precision, escala } => format!("decimal128({precision}, {escala})"),
            Fisico::Fecha => "date32[day]".into(),
            Fisico::Hora => "time64[us]".into(),
            Fisico::FechaHora => "timestamp[us]".into(),
            Fisico::Instante => "timestamp[us, tz=UTC]".into(),
        }
    }

    /// El tipo de Iceberg (la spec de tablas, «Primitive Types»): lo que el
    /// esquema de una View servida por `/v1` dice de cada columna. Mismo
    /// físico que la copia, así que un motor lo lee sin conversión que no sea
    /// ensanchar (`decimal(18, 2)` escrito → `decimal(38, 2)` declarado).
    pub fn iceberg(&self) -> String {
        match self {
            Fisico::Texto => "string".into(),
            Fisico::Entero => "long".into(),
            Fisico::Real => "double".into(),
            Fisico::Logico => "boolean".into(),
            Fisico::Decimal { precision, escala } => format!("decimal({precision}, {escala})"),
            Fisico::Fecha => "date".into(),
            Fisico::Hora => "time".into(),
            Fisico::FechaHora => "timestamp".into(),
            Fisico::Instante => "timestamptz".into(),
        }
    }

    /// Cómo se llama el escalar cuando se dice que un valor no lo es.
    pub fn nombre(&self) -> &'static str {
        match self {
            Fisico::Texto => "String",
            Fisico::Entero => "Integer",
            Fisico::Real => "Float",
            Fisico::Logico => "Boolean",
            Fisico::Decimal { .. } => "Decimal",
            Fisico::Fecha => "Date",
            Fisico::Hora => "Time",
            Fisico::FechaHora => "DateTime",
            Fisico::Instante => "DateTimeTz",
        }
    }

    /// **Analiza la forma canónica del texto.** `None` es «no es un valor de
    /// este tipo», y no se inventa nada: ni se recorta, ni se redondea, ni se
    /// interpreta un entero como fecha.
    pub fn analizar(&self, texto: &str) -> Option<Valor> {
        Some(match self {
            Fisico::Texto => Valor::Texto(texto.to_string()),
            Fisico::Entero => Valor::Entero(entero(texto)?),
            // `NaN`, `Infinity` y `-Infinity` son valores de un `float8` y
            // Arrow los tiene; Rust los lee tal cual. Lo que no se admite es
            // el espacio: `" 1"` no es un número.
            Fisico::Real => {
                if texto.trim() != texto || texto.is_empty() {
                    return None;
                }
                Valor::Real(texto.parse::<f64>().ok()?)
            }
            Fisico::Logico => match texto {
                "true" => Valor::Logico(true),
                "false" => Valor::Logico(false),
                _ => return None,
            },
            Fisico::Decimal { precision, escala } => {
                Valor::Decimal(decimal(texto, *precision, *escala)?)
            }
            Fisico::Fecha => Valor::Fecha(fecha(texto)?),
            Fisico::Hora => Valor::Hora(hora(texto)?),
            Fisico::FechaHora => {
                let (d, h) = fecha_y_hora(texto)?;
                if !h.1.is_empty() {
                    return None;
                }
                Valor::FechaHora(i64::from(d) * US_POR_DIA + h.0)
            }
            Fisico::Instante => {
                let (d, h) = fecha_y_hora(texto)?;
                let desfase = desfase(h.1)?;
                Valor::Instante(i64::from(d) * US_POR_DIA + h.0 - desfase)
            }
        })
    }
}

/// Un valor ya analizado, en la unidad en que Arrow lo guarda.
#[derive(Debug, Clone, PartialEq)]
pub enum Valor {
    Texto(String),
    Entero(i64),
    Real(f64),
    Logico(bool),
    /// Sin escala: el entero que, dividido por 10^escala del físico, es el valor.
    Decimal(i128),
    /// Días desde 1970-01-01.
    Fecha(i32),
    /// Microsegundos desde la medianoche.
    Hora(i64),
    /// Microsegundos desde 1970-01-01T00:00, hora de pared.
    FechaHora(i64),
    /// Microsegundos desde 1970-01-01T00:00Z.
    Instante(i64),
}

impl Valor {
    /// **La forma canónica del texto**, la misma que analiza [`Fisico::analizar`]:
    /// `analizar(v.texto(f)) == v` para todo valor de su físico. Es la que
    /// `ore-store leer` devuelve cuando vuelve a texto una columna estrechada.
    pub fn texto(&self, fisico: &Fisico) -> String {
        match self {
            Valor::Texto(s) => s.clone(),
            Valor::Entero(n) => n.to_string(),
            Valor::Real(f) => real(*f),
            Valor::Logico(b) => b.to_string(),
            Valor::Decimal(n) => {
                let escala = match fisico {
                    Fisico::Decimal { escala, .. } => *escala,
                    _ => 0,
                };
                decimal_texto(*n, escala)
            }
            Valor::Fecha(d) => fecha_texto(*d),
            Valor::Hora(us) => hora_texto(*us),
            Valor::FechaHora(us) => {
                let (d, h) = (us.div_euclid(US_POR_DIA), us.rem_euclid(US_POR_DIA));
                format!("{} {}", fecha_texto(d as i32), hora_texto(h))
            }
            Valor::Instante(us) => {
                let (d, h) = (us.div_euclid(US_POR_DIA), us.rem_euclid(US_POR_DIA));
                format!("{} {}+00", fecha_texto(d as i32), hora_texto(h))
            }
        }
    }
}

const US_POR_DIA: i64 = 86_400_000_000;

fn entero(t: &str) -> Option<i64> {
    // `parse` admite `+1`; el texto canónico no lleva signo más.
    if t.starts_with('+') {
        return None;
    }
    t.parse::<i64>().ok()
}

fn real(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else {
        format!("{f}")
    }
}

/// `[-]dígitos[.dígitos]` → entero sin escala. Falla si hay más decimales de
/// los que la escala admite (aunque sean ceros no: `10.500` a escala 2 es
/// `10.50`, exacto), o si el total no cabe en la precisión.
fn decimal(t: &str, precision: u8, escala: u8) -> Option<i128> {
    let (neg, cuerpo) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let (ent, frac) = cuerpo.split_once('.').unwrap_or((cuerpo, ""));
    if ent.is_empty() && frac.is_empty() {
        return None;
    }
    if !ent.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Los decimales que sobran solo pueden ser ceros: quitarlos no cambia el valor.
    let frac = if frac.len() > usize::from(escala) {
        let (cabe, sobra) = frac.split_at(usize::from(escala));
        if sobra.bytes().any(|b| b != b'0') {
            return None;
        }
        cabe
    } else {
        frac
    };
    let ent_sin_ceros = ent.trim_start_matches('0');
    if ent_sin_ceros.len() + usize::from(escala) > usize::from(precision) {
        return None;
    }
    let mut n: i128 = 0;
    for b in ent.bytes().chain(frac.bytes()) {
        n = n * 10 + i128::from(b - b'0');
    }
    for _ in frac.len()..usize::from(escala) {
        n *= 10;
    }
    Some(if neg { -n } else { n })
}

fn decimal_texto(n: i128, escala: u8) -> String {
    let neg = n < 0;
    let digitos = n.unsigned_abs().to_string();
    let escala = usize::from(escala);
    let (ent, frac) = if digitos.len() > escala {
        digitos.split_at(digitos.len() - escala)
    } else {
        ("", digitos.as_str())
    };
    let ent = if ent.is_empty() { "0" } else { ent };
    let frac = format!("{frac:0>escala$}");
    // Los ceros finales de la parte decimal no son valor: `10.500000000000000000`
    // a la escala por defecto es `10.5`. Postgres conserva la escala del origen;
    // aquí la escala es la de la columna, no la del valor, y enseñarla sería
    // enseñar un dato que el origen no dijo.
    let frac = frac.trim_end_matches('0');
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(ent);
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// `YYYY-MM-DD` → días desde 1970-01-01. Años 0001–9999; el calendario
/// proléptico gregoriano, que es el de Arrow y el de Postgres.
fn fecha(t: &str) -> Option<i32> {
    let b = t.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y = numero(&t[0..4])? as i64;
    let m = numero(&t[5..7])? as i64;
    let d = numero(&t[8..10])? as i64;
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > dias_del_mes(y, m) {
        return None;
    }
    // Howard Hinnant, `days_from_civil`.
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) as i32)
}

fn fecha_texto(dias: i32) -> String {
    // `civil_from_days`.
    let z = i64::from(dias) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn dias_del_mes(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// `HH:MM:SS[.f{1,6}]` → microsegundos. Devuelve también lo que sobra tras la
/// hora (el desfase de un instante), para que quien lo llame decida.
fn hora_y_resto(t: &str) -> Option<(i64, &str)> {
    let b = t.as_bytes();
    if b.len() < 8 || b[2] != b':' || b[5] != b':' {
        return None;
    }
    let h = numero(&t[0..2])?;
    let m = numero(&t[3..5])?;
    let s = numero(&t[6..8])?;
    if h > 23 || m > 59 || s > 59 {
        return None;
    }
    let mut us = (i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(s)) * 1_000_000;
    let mut resto = &t[8..];
    if let Some(f) = resto.strip_prefix('.') {
        let n = f.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 || n > 6 {
            return None;
        }
        let frac = numero(&f[..n])?;
        us += i64::from(frac) * 10i64.pow(6 - n as u32);
        resto = &f[n..];
    }
    Some((us, resto))
}

fn hora(t: &str) -> Option<i64> {
    match hora_y_resto(t)? {
        (us, "") => Some(us),
        _ => None,
    }
}

fn hora_texto(us: i64) -> String {
    let (s, frac) = (us.div_euclid(1_000_000), us.rem_euclid(1_000_000));
    let mut out = format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60);
    if frac != 0 {
        let f = format!("{frac:06}");
        out.push('.');
        out.push_str(f.trim_end_matches('0'));
    }
    out
}

/// `YYYY-MM-DD HH:MM:SS[.f]…` o con `T`. Devuelve los días, y la hora con lo
/// que sobre.
fn fecha_y_hora(t: &str) -> Option<(i32, (i64, &str))> {
    if t.len() < 19 || !matches!(t.as_bytes()[10], b' ' | b'T') {
        return None;
    }
    Some((fecha(&t[..10])?, hora_y_resto(&t[11..])?))
}

/// El desfase de un instante, en microsegundos: `Z`, `+00`, `+01:00`, `-0530`.
/// Un instante **sin** desfase no es un instante: es una hora de pared, y se
/// niega.
fn desfase(t: &str) -> Option<i64> {
    if t == "Z" {
        return Some(0);
    }
    let (signo, resto) = match t.as_bytes().first()? {
        b'+' => (1, &t[1..]),
        b'-' => (-1, &t[1..]),
        _ => return None,
    };
    let (h, m) = match resto.len() {
        2 => (numero(resto)?, 0),
        4 => (numero(&resto[..2])?, numero(&resto[2..])?),
        5 if resto.as_bytes()[2] == b':' => (numero(&resto[..2])?, numero(&resto[3..])?),
        _ => return None,
    };
    if h > 14 || m > 59 {
        return None;
    }
    Some(signo * (i64::from(h) * 3600 + i64::from(m) * 60) * 1_000_000)
}

fn numero(t: &str) -> Option<u32> {
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::parse_type;

    fn f(t: &str) -> Fisico {
        Fisico::de(&parse_type(t).expect("un tipo de OOS"))
    }

    /// La tabla de 0032 §1, columna «Arrow / Parquet», como está escrita.
    #[test]
    fn cada_escalar_tiene_su_fisico() {
        let tabla = [
            ("Integer", "int64"),
            ("Decimal", "decimal128(38, 18)"),
            ("Float", "double"),
            ("Boolean", "bool"),
            ("String", "string"),
            ("Date", "date32[day]"),
            ("Time", "time64[us]"),
            ("DateTime", "timestamp[us]"),
            ("DateTimeTz", "timestamp[us, tz=UTC]"),
            ("Opaque", "string"),
            ("list<Integer>", "string"),
            ("Money<EUR, 2>", "decimal128(38, 2)"),
            ("Decimal<38, 9>", "decimal128(38, 9)"),
            ("Decimal<10, 2>", "decimal128(10, 2)"),
            ("Quantity<km, 1>", "decimal128(38, 1)"),
        ];
        for (oos, arrow) in tabla {
            assert_eq!(f(oos).arrow(), arrow, "{oos}");
        }
    }

    /// Lo que el driver de Postgres escribe (`texto.rs`) se lee, y lo que se
    /// lee se vuelve a escribir igual: la forma canónica es una.
    #[test]
    fn la_forma_canonica_va_y_vuelve() {
        let casos = [
            ("Integer", "-9223372036854775808"),
            ("Integer", "42"),
            ("Float", "1.5"),
            ("Float", "NaN"),
            ("Float", "-Infinity"),
            ("Float", "1000000000000000000000"),
            ("Boolean", "true"),
            ("Decimal", "12345.6789"),
            ("Decimal", "-0.000000000000000001"),
            ("Decimal", "99999999999999999999.999999999999999999"),
            ("Money<EUR, 2>", "10.5"),
            ("Date", "2020-02-29"),
            ("Date", "1969-12-31"),
            ("Date", "0001-01-01"),
            ("Date", "9999-12-31"),
            ("Time", "01:01:01"),
            ("Time", "23:59:59.999999"),
            ("Time", "10:00:00.5"),
            ("DateTime", "2020-02-29 10:00:00.5"),
            ("DateTime", "1999-12-31 23:59:59.999999"),
            ("DateTimeTz", "2020-02-29 10:00:00.5+00"),
            ("DateTimeTz", "1970-01-01 00:00:00+00"),
        ];
        for (oos, texto) in casos {
            let fis = f(oos);
            let v = fis
                .analizar(texto)
                .unwrap_or_else(|| panic!("`{texto}` no analiza como {oos}"));
            assert_eq!(v.texto(&fis), texto, "{oos} `{texto}`");
        }
        // `1e21` se lee, y al volver es lo que Rust escribe: sin exponente.
        assert_eq!(
            f("Float").analizar("1e21").unwrap().texto(&f("Float")),
            "1000000000000000000000"
        );
    }

    /// Las unidades son las de Arrow: días y microsegundos desde 1970.
    #[test]
    fn las_unidades_son_las_de_arrow() {
        assert_eq!(f("Date").analizar("1970-01-01"), Some(Valor::Fecha(0)));
        assert_eq!(f("Date").analizar("1969-12-31"), Some(Valor::Fecha(-1)));
        assert_eq!(f("Date").analizar("2000-01-01"), Some(Valor::Fecha(10_957)));
        assert_eq!(f("Time").analizar("00:00:01"), Some(Valor::Hora(1_000_000)));
        assert_eq!(
            f("DateTime").analizar("1970-01-02 00:00:00"),
            Some(Valor::FechaHora(US_POR_DIA))
        );
        assert_eq!(
            f("Decimal").analizar("1.5"),
            Some(Valor::Decimal(1_500_000_000_000_000_000))
        );
        assert_eq!(
            f("Money<EUR, 2>").analizar("1.5"),
            Some(Valor::Decimal(150))
        );
    }

    /// Un instante es un instante: el desfase se convierte y se guarda en UTC.
    /// La zona del origen no sobrevive (0032, «Lo que se acepta a cambio»).
    #[test]
    fn un_instante_se_convierte_a_utc_y_uno_sin_desfase_se_niega() {
        let i = f("DateTimeTz");
        let utc = i.analizar("2024-06-01 12:00:00+00").unwrap();
        assert_eq!(i.analizar("2024-06-01 14:00:00+02:00").unwrap(), utc);
        assert_eq!(i.analizar("2024-06-01 14:00:00+0200").unwrap(), utc);
        assert_eq!(i.analizar("2024-06-01T06:30:00-05:30").unwrap(), utc);
        assert_eq!(i.analizar("2024-06-01T12:00:00Z").unwrap(), utc);
        assert_eq!(utc.texto(&i), "2024-06-01 12:00:00+00");
        assert_eq!(
            i.analizar("2024-06-01 12:00:00"),
            None,
            "sin desfase no es un instante"
        );
        assert_eq!(
            f("DateTime").analizar("2024-06-01 12:00:00+00"),
            None,
            "y una hora de pared no lleva desfase"
        );
    }

    /// **No se inventa una conversión.** Cada uno de estos es un texto que
    /// alguien querría «arreglar» al vuelo, y la respuesta es `None`.
    #[test]
    fn lo_que_no_es_del_tipo_no_analiza() {
        let malos = [
            ("Integer", "1.0"),
            ("Integer", "uno"),
            ("Integer", " 1"),
            ("Integer", "+1"),
            ("Integer", "9223372036854775808"),
            ("Float", " 1.5"),
            ("Float", ""),
            ("Boolean", "t"),
            ("Boolean", "TRUE"),
            ("Boolean", "1"),
            ("Decimal", "1,5"),
            ("Decimal", "1e3"),
            ("Decimal", "NaN"),
            ("Decimal", "."),
            ("Decimal", "1.0000000000000000001"),
            ("Decimal", "100000000000000000000"),
            ("Money<EUR, 2>", "1.005"),
            ("Date", "2021-02-29"),
            ("Date", "2020-13-01"),
            ("Date", "20200101"),
            ("Date", "2020-1-1"),
            ("Date", "infinity"),
            ("Time", "24:00:00"),
            ("Time", "10:00"),
            ("Time", "10:00:00.1234567"),
            ("DateTime", "2020-02-29"),
            ("DateTime", "2020-02-29 25:00:00"),
            ("DateTimeTz", "1700000000"),
            ("DateTimeTz", "1.7E9"),
            ("DateTimeTz", "2020-02-29 10:00:00+15"),
        ];
        for (oos, texto) in malos {
            assert_eq!(
                f(oos).analizar(texto),
                None,
                "{oos} `{texto}` no debería analizar"
            );
        }
    }

    /// Los ceros que sobran a la derecha del decimal no son valor: se admiten al
    /// leer (`10.500` a escala 2) y no se escriben al volver.
    #[test]
    fn los_ceros_de_mas_no_son_valor() {
        let m = f("Money<EUR, 2>");
        assert_eq!(m.analizar("10.500"), Some(Valor::Decimal(1050)));
        assert_eq!(Valor::Decimal(1050).texto(&m), "10.5");
        assert_eq!(Valor::Decimal(1000).texto(&m), "10");
        assert_eq!(Valor::Decimal(5).texto(&m), "0.05");
        assert_eq!(Valor::Decimal(-5).texto(&m), "-0.05");
        assert_eq!(Valor::Decimal(0).texto(&m), "0");
    }
}
