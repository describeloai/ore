//! **Cada valor, del cable a texto.** Es el transporte, y es lo único que este
//! driver tiene de suyo.
//!
//! # El defecto que esto cierra
//!
//! `filas` leía cada columna con `try_get::<Option<String>>(i).unwrap_or(None)`.
//! `String` solo sabe leerse de `text`, `varchar` y parientes; un `float8`, un
//! `int8`, un `numeric`, una fecha o un `bool` **fallaban la conversión** y el
//! `unwrap_or(None)` convertía ese fallo en «nulo». La copia salía con todas
//! las filas, el informe decía `copiada`, y toda columna que no fuera texto
//! estaba vacía sin que nadie lo dijera. Se midió en `demo` (medida W1 §B):
//! `products` con 2 de 9 columnas, `sellers` con 3 de 4.
//!
//! # Lo que se hace, y por qué así
//!
//! El protocolo trae los valores en **binario** y cada tipo tiene un formato
//! documentado y pequeño: enteros en big-endian, IEEE 754, un byte para el
//! booleano, base 10 000 para `numeric`, días o microsegundos desde 2000-01-01
//! para fechas y horas. Se decodifica aquí, a mano, y se escribe **como
//! Postgres lo escribiría**: `12.5`, `10.50` (con su escala), `2020-02-29`,
//! `2020-02-29 10:00:00+00`. No hay crate nueva —`postgres` ya trae los tipos—
//! y no se pide al servidor un `::text` que exigiría saber el tipo antes de
//! preguntar.
//!
//! **Lo que no se sabe leer se niega**, nombrando la columna y el tipo. Un
//! valor que llega vacío porque no se supo leer es exactamente lo que había, y
//! es peor que una petición que falla: una copia que calla se sirve.
//!
//! Un `enum` es su etiqueta, y se lee como texto: el binario de un tipo
//! enumerado es la etiqueta en UTF-8.

use postgres::types::{Format, FromSql, IsNull, Kind, ToSql, Type, to_sql_checked};

/// **Un parámetro, como texto, para que lo coaccione el servidor.**
///
/// El dialecto de Postgres es posicional y su promesa era *«el servidor
/// coacciona el texto»* — cierto para un literal, y falso para un parámetro
/// mandado por el crate: `String` como `ToSql` solo acepta columnas de texto,
/// así que un `where` o una clave sobre un `int4` fallaba en el cliente antes
/// de llegar al servidor («cannot convert between String and int4»). Esto es
/// la otra mitad de [`Texto`]: el valor se manda **en formato texto** y con
/// cualquier tipo, y es Postgres quien lo lee como `int4`, `date` o lo que la
/// columna sea, igual que haría con `'1'` escrito en la consulta.
#[derive(Debug)]
pub struct Parametro(pub String);

impl ToSql for Parametro {
    fn to_sql(
        &self,
        _: &Type,
        out: &mut postgres::types::private::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.extend_from_slice(self.0.as_bytes());
        Ok(IsNull::No)
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn encode_format(&self, _: &Type) -> Format {
        Format::Text
    }

    to_sql_checked!();
}

/// Un valor ya en texto, leído de cualquier tipo que se sepa decodificar.
#[derive(Debug, PartialEq, Eq)]
pub struct Texto(pub String);

impl<'a> FromSql<'a> for Texto {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        decodificar(ty, raw).map(Texto).map_err(Into::into)
    }

    /// Todo se acepta aquí y se decide en [`decodificar`]: así el error dice
    /// **qué tipo** no se supo leer, en vez de un «no se puede convertir»
    /// genérico del crate.
    fn accepts(_: &Type) -> bool {
        true
    }
}

/// Del binario de Postgres al texto canónico, por tipo.
pub fn decodificar(ty: &Type, raw: &[u8]) -> Result<String, String> {
    if let Kind::Enum(_) = ty.kind() {
        return utf8(raw);
    }
    match *ty {
        Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::CHAR
        | Type::UNKNOWN
        | Type::JSON
        | Type::XML => utf8(raw),
        // jsonb binario: un byte de versión (1) y el JSON como texto.
        Type::JSONB => match raw.split_first() {
            Some((1, resto)) => utf8(resto),
            _ => Err("jsonb con una versión que no se conoce".into()),
        },
        Type::BOOL => match raw {
            [0] => Ok("false".into()),
            [1] => Ok("true".into()),
            _ => Err("un booleano que no es un byte".into()),
        },
        Type::INT2 => Ok(i16::from_be_bytes(fijo::<2>(raw)?).to_string()),
        Type::INT4 => Ok(i32::from_be_bytes(fijo::<4>(raw)?).to_string()),
        Type::INT8 => Ok(i64::from_be_bytes(fijo::<8>(raw)?).to_string()),
        Type::OID => Ok(u32::from_be_bytes(fijo::<4>(raw)?).to_string()),
        Type::FLOAT4 => Ok(flotante(f64::from(f32::from_be_bytes(fijo::<4>(raw)?)))),
        Type::FLOAT8 => Ok(flotante(f64::from_be_bytes(fijo::<8>(raw)?))),
        Type::NUMERIC => numerico(raw),
        Type::DATE => {
            let d = i32::from_be_bytes(fijo::<4>(raw)?);
            Ok(match d {
                i32::MAX => "infinity".into(),
                i32::MIN => "-infinity".into(),
                _ => fecha(i64::from(d)),
            })
        }
        Type::TIMESTAMP | Type::TIMESTAMPTZ => {
            let us = i64::from_be_bytes(fijo::<8>(raw)?);
            Ok(match us {
                i64::MAX => "infinity".into(),
                i64::MIN => "-infinity".into(),
                _ => {
                    let (dias, resto) =
                        (us.div_euclid(86_400_000_000), us.rem_euclid(86_400_000_000));
                    let mut s = format!("{} {}", fecha(dias), hora(resto));
                    if *ty == Type::TIMESTAMPTZ {
                        s.push_str("+00");
                    }
                    s
                }
            })
        }
        Type::TIME => Ok(hora(i64::from_be_bytes(fijo::<8>(raw)?))),
        Type::UUID => {
            let b = fijo::<16>(raw)?;
            let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
            Ok(format!(
                "{}-{}-{}-{}-{}",
                &h[..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..]
            ))
        }
        Type::BYTEA => Ok(format!(
            "\\x{}",
            raw.iter().map(|x| format!("{x:02x}")).collect::<String>()
        )),
        _ => Err(format!(
            "el tipo `{}` no se sabe leer como texto: este driver decodifica texto, enteros, \
             flotantes, numeric, booleanos, fechas, horas, uuid, json y bytea. Una columna de \
             otro tipo no se copia en silencio: se declara aquí cómo se escribe, o se deja fuera \
             de `fields`",
            ty.name()
        )),
    }
}

/// El error del crate envuelve el nuestro —«error deserializing column 1»— y
/// lo que dice el tipo está en la causa. Se sigue la cadena entera.
pub fn causa(e: &dyn std::error::Error) -> String {
    let mut partes = vec![e.to_string()];
    let mut actual = e.source();
    while let Some(c) = actual {
        partes.push(c.to_string());
        actual = c.source();
    }
    partes.join(": ")
}

fn utf8(raw: &[u8]) -> Result<String, String> {
    std::str::from_utf8(raw)
        .map(str::to_string)
        .map_err(|e| format!("texto que no es UTF-8: {e}"))
}

fn fijo<const N: usize>(raw: &[u8]) -> Result<[u8; N], String> {
    raw.try_into()
        .map_err(|_| format!("se esperaban {N} bytes y llegaron {}", raw.len()))
}

/// Como Postgres: `NaN`, `Infinity`, `-Infinity`; el resto, la forma más corta
/// que vuelve al mismo número.
fn flotante(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else {
        format!("{f}")
    }
}

/// `numeric` binario: `ndigits`, `weight`, `sign`, `dscale` (i16 cada uno) y
/// `ndigits` dígitos en base 10 000. Se escribe con exactamente `dscale`
/// decimales, que es lo que Postgres hace.
fn numerico(raw: &[u8]) -> Result<String, String> {
    if raw.len() < 8 {
        return Err("numeric con cabecera incompleta".into());
    }
    let n = usize::from(u16::from_be_bytes([raw[0], raw[1]]));
    let peso = i32::from(i16::from_be_bytes([raw[2], raw[3]]));
    let signo = u16::from_be_bytes([raw[4], raw[5]]);
    let escala = usize::from(u16::from_be_bytes([raw[6], raw[7]]));
    match signo {
        0xC000 => return Ok("NaN".into()),
        0xD000 => return Ok("Infinity".into()),
        0xF000 => return Ok("-Infinity".into()),
        0x0000 | 0x4000 => {}
        _ => return Err("numeric con un signo que no se conoce".into()),
    }
    let digitos: Vec<u16> = raw[8..]
        .as_chunks::<2>()
        .0
        .iter()
        .take(n)
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    if digitos.len() < n {
        return Err("numeric con menos dígitos de los declarados".into());
    }
    // La parte entera: los grupos con peso >= 0. Con peso < 0 no hay ninguno.
    let mut entera = String::new();
    for i in 0..=peso.max(-1) {
        let g = usize::try_from(i)
            .ok()
            .and_then(|i| digitos.get(i))
            .copied()
            .unwrap_or(0);
        if entera.is_empty() {
            entera.push_str(&g.to_string());
        } else {
            entera.push_str(&format!("{g:04}"));
        }
    }
    if entera.is_empty() {
        entera.push('0');
    }
    // La parte decimal: los grupos por debajo del peso, recortada a `escala`.
    let mut decimal = String::new();
    let mut i = peso + 1;
    while decimal.len() < escala {
        let g = usize::try_from(i)
            .ok()
            .and_then(|i| digitos.get(i))
            .copied()
            .unwrap_or(0);
        decimal.push_str(&format!("{g:04}"));
        i += 1;
    }
    decimal.truncate(escala);
    let mut s = String::new();
    if signo == 0x4000 {
        s.push('-');
    }
    s.push_str(&entera);
    if escala > 0 {
        s.push('.');
        s.push_str(&decimal);
    }
    Ok(s)
}

/// Días desde 2000-01-01 a `AAAA-MM-DD` (calendario gregoriano proléptico).
fn fecha(dias_desde_2000: i64) -> String {
    // 2000-01-01 son 10 957 días desde 1970-01-01.
    let z = dias_desde_2000 + 10_957 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Microsegundos desde medianoche a `HH:MM:SS[.ffffff]`, sin ceros de más.
fn hora(us: i64) -> String {
    let (s, frac) = (us.div_euclid(1_000_000), us.rem_euclid(1_000_000));
    let mut out = format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60);
    if frac != 0 {
        let f = format!("{frac:06}");
        out.push('.');
        out.push_str(f.trim_end_matches('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(ty: Type, raw: &[u8]) -> String {
        decodificar(&ty, raw).unwrap()
    }

    /// Los tipos que la copia de `demo` perdía: flotante y entero.
    #[test]
    fn flotantes_y_enteros_salen_como_postgres_los_escribe() {
        assert_eq!(d(Type::FLOAT8, &12.5f64.to_be_bytes()), "12.5");
        assert_eq!(d(Type::FLOAT8, &0.1f64.to_be_bytes()), "0.1");
        assert_eq!(d(Type::FLOAT4, &1.5f32.to_be_bytes()), "1.5");
        assert_eq!(d(Type::FLOAT8, &f64::NAN.to_be_bytes()), "NaN");
        assert_eq!(
            d(Type::FLOAT8, &f64::NEG_INFINITY.to_be_bytes()),
            "-Infinity"
        );
        assert_eq!(d(Type::INT8, &(-7i64).to_be_bytes()), "-7");
        assert_eq!(d(Type::INT4, &1024i32.to_be_bytes()), "1024");
        assert_eq!(d(Type::INT2, &3i16.to_be_bytes()), "3");
        assert_eq!(d(Type::BOOL, &[1]), "true");
        assert_eq!(d(Type::BOOL, &[0]), "false");
    }

    /// `numeric` conserva la escala: `10.50` es `10.50`, no `10.5`.
    #[test]
    fn numeric_es_exacto_y_conserva_la_escala() {
        // 10.50 → ndigits 2, weight 0, sign +, dscale 2, dígitos [10, 5000]
        let raw = [0, 2, 0, 0, 0, 0, 0, 2, 0, 10, 0x13, 0x88];
        assert_eq!(d(Type::NUMERIC, &raw), "10.50");
        // -12345678.9 → [1234, 5678, 9000], weight 1, sign -, dscale 1
        let raw = [
            0, 3, 0, 1, 0x40, 0, 0, 1, 0x04, 0xD2, 0x16, 0x2E, 0x23, 0x28,
        ];
        assert_eq!(d(Type::NUMERIC, &raw), "-12345678.9");
        // 0.05 → ndigits 1, weight -1, dscale 2, dígitos [500]
        let raw = [0, 1, 0xFF, 0xFF, 0, 0, 0, 2, 0x01, 0xF4];
        assert_eq!(d(Type::NUMERIC, &raw), "0.05");
        // 42 sin decimales → ndigits 1, weight 0, dscale 0
        let raw = [0, 1, 0, 0, 0, 0, 0, 0, 0, 42];
        assert_eq!(d(Type::NUMERIC, &raw), "42");
        // NaN
        let raw = [0, 0, 0, 0, 0xC0, 0, 0, 0];
        assert_eq!(d(Type::NUMERIC, &raw), "NaN");
    }

    /// Fechas y horas: días y microsegundos desde 2000-01-01, bisiestos incluidos.
    #[test]
    fn fechas_y_horas() {
        assert_eq!(fecha(0), "2000-01-01");
        assert_eq!(fecha(-1), "1999-12-31");
        assert_eq!(fecha(7364), "2020-02-29");
        assert_eq!(d(Type::DATE, &7364i32.to_be_bytes()), "2020-02-29");
        assert_eq!(d(Type::DATE, &i32::MAX.to_be_bytes()), "infinity");
        let us = 7364 * 86_400_000_000i64 + 10 * 3_600_000_000 + 500_000;
        assert_eq!(
            d(Type::TIMESTAMP, &us.to_be_bytes()),
            "2020-02-29 10:00:00.5"
        );
        assert_eq!(
            d(Type::TIMESTAMPTZ, &us.to_be_bytes()),
            "2020-02-29 10:00:00.5+00"
        );
        assert_eq!(
            d(Type::TIMESTAMP, &(-1i64).to_be_bytes()),
            "1999-12-31 23:59:59.999999"
        );
        assert_eq!(d(Type::TIME, &(3_661_000_000i64).to_be_bytes()), "01:01:01");
    }

    #[test]
    fn uuid_json_y_bytea() {
        let u = [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
            0xde, 0xf0,
        ];
        assert_eq!(d(Type::UUID, &u), "12345678-9abc-def0-1234-56789abcdef0");
        assert_eq!(d(Type::JSONB, b"\x01{\"a\":1}"), "{\"a\":1}");
        assert_eq!(d(Type::JSON, b"{\"a\":1}"), "{\"a\":1}");
        assert_eq!(d(Type::BYTEA, &[0xde, 0xad]), "\\xdead");
        assert_eq!(d(Type::TEXT, "hola".as_bytes()), "hola");
    }

    /// Lo que no se sabe leer se niega con el tipo en el mensaje — no sale nulo.
    #[test]
    fn lo_que_no_se_sabe_leer_se_niega_nombrando_el_tipo() {
        let e = decodificar(&Type::INTERVAL, &[0; 16]).unwrap_err();
        assert!(e.contains("`interval`"), "{e}");
        let e = decodificar(&Type::INT8, &[0; 3]).unwrap_err();
        assert!(e.contains("8 bytes"), "{e}");
    }
}
