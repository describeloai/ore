//! **De lo que dice Salesforce a un valor de OOS**, en **un solo sitio** (ADR
//! 0083 §1.6, X0 C10).
//!
//! REST y Bulk no devuelven los mismos formatos: la fecha sale `…000+0000` por
//! REST y `…000Z` por Bulk; un `double` sale `20.0` y un `int`, `10`; un nulo es
//! `null` en JSON y la celda vacía en CSV (C11). Para que no haya dos
//! conversiones que diverjan, las dos vías entregan **texto** (`Option<&str>`)
//! y este módulo es el único que lo convierte.
//!
//! # La regla que manda
//!
//! **Un valor que no cabe en el tipo de su columna es un error que nombra
//! columna y fila; no se convierte en nulo ni se redondea** (la misma de 0032 §4
//! y de `ore-mongodb`). Un `Decimal<18, 2>` con `1.005` no se redondea a `1.01`.

use crate::esquema::Campo;
use ore_arrow::Columna;
use ore_base::tipos::{Fisico, Valor};

/// El físico y el tipo de OOS de una columna de Salesforce.
///
/// - `double`, `currency` y `percent` son **decimales exactos** cuando
///   `describe` dice su precisión y escala: es lo que Salesforce guarda.
/// - `datetime` es un **instante** (UTC); `date`, una fecha.
/// - Lo demás (texto, ids, picklists, `time`, `anyType`…) va como texto.
pub fn fisico(c: &Campo) -> (Fisico, String) {
    match c.tipo.as_str() {
        "int" | "long" => (Fisico::Entero, "Integer".into()),
        "double" | "currency" | "percent" => {
            if c.precision > 0 && c.precision <= 38 && c.escala <= c.precision {
                (
                    Fisico::Decimal {
                        precision: c.precision as u8,
                        escala: c.escala as u8,
                    },
                    format!("Decimal<{}, {}>", c.precision, c.escala),
                )
            } else {
                (Fisico::Real, "Float".into())
            }
        }
        "boolean" => (Fisico::Logico, "Boolean".into()),
        "date" => (Fisico::Fecha, "Date".into()),
        "datetime" => (Fisico::Instante, "DateTimeTz".into()),
        _ => (Fisico::Texto, "String".into()),
    }
}

pub fn columna(nombre: &str, c: &Campo) -> Columna {
    let (f, oos) = fisico(c);
    Columna::new(nombre, f, oos)
}

/// Días desde 1970-01-01 de una fecha del calendario gregoriano (Hinnant).
fn dias(a: i64, m: i64, d: i64) -> i64 {
    let a = if m <= 2 { a - 1 } else { a };
    let era = a.div_euclid(400);
    let yoe = a - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn bisiesto(a: i64) -> bool {
    (a % 4 == 0 && a % 100 != 0) || a % 400 == 0
}

fn num(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `AAAA-MM-DD` → días desde la época.
pub fn fecha(t: &str) -> Result<i32, String> {
    let malo = || format!("`{t}` no es una fecha `AAAA-MM-DD`");
    let mut p = t.split('-');
    let (a, m, d) = (p.next(), p.next(), p.next());
    let (Some(a), Some(m), Some(d), None) = (a, m, d, p.next()) else {
        return Err(malo());
    };
    if a.len() != 4 || m.len() != 2 || d.len() != 2 {
        return Err(malo());
    }
    let (a, m, d) = (
        num(a).ok_or_else(malo)?,
        num(m).ok_or_else(malo)?,
        num(d).ok_or_else(malo)?,
    );
    let max = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if bisiesto(a) => 29,
        2 => 28,
        _ => return Err(malo()),
    };
    if d < 1 || d > max {
        return Err(malo());
    }
    i32::try_from(dias(a, m, d)).map_err(|_| malo())
}

/// `2026-10-31T20:39:17.000+0000` o `…000Z` → microsegundos UTC desde la época.
pub fn instante(t: &str) -> Result<i64, String> {
    let malo = || format!("`{t}` no es un instante `AAAA-MM-DDThh:mm:ss.fffZ`");
    let (f, h) = t.split_once('T').ok_or_else(malo)?;
    let dia = i64::from(fecha(f)?);
    let b = h.as_bytes();
    if b.len() < 8 || b[2] != b':' || b[5] != b':' {
        return Err(malo());
    }
    let (hh, mm, ss) = (
        num(&h[0..2]).ok_or_else(malo)?,
        num(&h[3..5]).ok_or_else(malo)?,
        num(&h[6..8]).ok_or_else(malo)?,
    );
    if hh > 23 || mm > 59 || ss > 60 {
        return Err(malo());
    }
    let mut resto = &h[8..];
    let mut micros = 0i64;
    if let Some(r) = resto.strip_prefix('.') {
        let n = r.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return Err(malo());
        }
        let (frac, sigue) = r.split_at(n);
        // Microsegundos: más de seis cifras solo se admiten si son ceros.
        if frac.len() > 6 && frac[6..].bytes().any(|b| b != b'0') {
            return Err(format!(
                "`{t}`: la fracción de segundo no cabe en microsegundos"
            ));
        }
        let seis = format!("{:0<6}", &frac[..frac.len().min(6)]);
        micros = seis.parse().map_err(|_| malo())?;
        resto = sigue;
    }
    let desfase = match resto {
        "Z" => 0,
        z => {
            let signo = match z.as_bytes().first() {
                Some(b'+') => 1,
                Some(b'-') => -1,
                _ => return Err(malo()),
            };
            let d = z[1..].replace(':', "");
            if d.len() != 4 {
                return Err(malo());
            }
            signo * (num(&d[0..2]).ok_or_else(malo)? * 60 + num(&d[2..4]).ok_or_else(malo)?)
        }
    };
    Ok(((dia * 86_400 + hh * 3_600 + mm * 60 + ss - desfase * 60) * 1_000_000) + micros)
}

/// `20.0`, `-1.5e2`, `12` → el entero que, dividido por 10^`escala`, es el
/// valor. **Exacto**: lo que no cabe en `escala` cifras decimales (salvo
/// ceros) o en `precision` cifras no se redondea, se rechaza.
pub fn decimal(t: &str, precision: u8, escala: u8) -> Result<i128, String> {
    let malo = || format!("`{t}` no es un decimal");
    let (neg, s) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (mant, exp) = match s.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().map_err(|_| malo())?),
        None => (s, 0),
    };
    let (ent, frac) = mant.split_once('.').unwrap_or((mant, ""));
    if (ent.is_empty() && frac.is_empty())
        || !ent.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
    {
        return Err(malo());
    }
    let digitos = format!("{ent}{frac}");
    // valor × 10^escala = dígitos × 10^e
    let e = exp - frac.len() as i32 + i32::from(escala);
    let no_cabe = || format!("`{t}` no cabe en un `Decimal<{precision}, {escala}>` sin redondear");
    let n: i128 = if e >= 0 {
        let base: i128 = digitos.parse().map_err(|_| no_cabe())?;
        let pot = 10i128.checked_pow(e as u32).ok_or_else(no_cabe)?;
        base.checked_mul(pot).ok_or_else(no_cabe)?
    } else {
        let quitar = (-e) as usize;
        if quitar > digitos.len() {
            if digitos.bytes().all(|b| b == b'0') {
                0
            } else {
                return Err(no_cabe());
            }
        } else {
            let (queda, cola) = digitos.split_at(digitos.len() - quitar);
            if cola.bytes().any(|b| b != b'0') {
                return Err(no_cabe());
            }
            if queda.is_empty() {
                0
            } else {
                queda.parse().map_err(|_| no_cabe())?
            }
        }
    };
    if n >= 10i128.pow(u32::from(precision)) {
        return Err(no_cabe());
    }
    Ok(if neg { -n } else { n })
}

/// (año, mes, día) de unos días desde la época (la inversa de [`dias`]).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Un instante (microsegundos UTC) como `2026-10-31T20:39:17Z`: **el testigo**
/// de una lectura por cursor (ADR 0083 §1.8). Es ordinal como texto (UTC, ancho
/// fijo) y es lo que SOQL admite en un `WHERE`. Trunca a segundos: es la
/// precisión de `SystemModstamp` (X0 D1).
pub fn texto_de_instante(us: i64) -> String {
    let s = us.div_euclid(1_000_000);
    let (y, m, d) = civil(s.div_euclid(86_400));
    let r = s.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        r / 3600,
        r % 3600 / 60,
        r % 60
    )
}

/// La cabecera `Date` de HTTP (`Sat, 31 Oct 2026 20:44:54 GMT`) en microsegundos
/// UTC. **Es la hora de la org** (X0 E): el testigo no depende del reloj de quien
/// lee, que puede ir adelantado o atrasado respecto a `SystemModstamp`.
pub fn instante_de_http(t: &str) -> Result<i64, String> {
    let malo = || format!("`{t}` no es una fecha HTTP `Sat, 31 Oct 2026 20:44:54 GMT`");
    let resto = t.split_once(',').map_or(t, |(_, r)| r);
    let mut p = resto.split_whitespace();
    let (d, mes, a, h) = (p.next(), p.next(), p.next(), p.next());
    let (Some(d), Some(mes), Some(a), Some(h)) = (d, mes, a, h) else {
        return Err(malo());
    };
    let m = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|x| x.eq_ignore_ascii_case(mes))
    .ok_or_else(malo)? as i64
        + 1;
    let hms: Vec<&str> = h.split(':').collect();
    let [hh, mm, ss] = hms[..] else {
        return Err(malo());
    };
    let dia = dias(num(a).ok_or_else(malo)?, m, num(d).ok_or_else(malo)?);
    let seg = dia * 86_400
        + num(hh).ok_or_else(malo)? * 3_600
        + num(mm).ok_or_else(malo)? * 60
        + num(ss).ok_or_else(malo)?;
    Ok(seg * 1_000_000)
}

/// El instante `t` (`2026-10-31T20:39:17Z`, como lo da [`texto_de_instante`] o
/// un `SystemModstamp`) retrasado `segundos`: **la relectura solapada** de una
/// lectura de cambios. Re-entregar el borde es idempotente con clave (spec
/// v1alpha8 `02-view` §5.4) y es lo que cubre las filas que comparten segundo
/// (X0 D1) y las transacciones largas que confirman tarde.
pub fn retrasar(t: &str, segundos: u64) -> Result<String, String> {
    let us = instante(t)?;
    let resta =
        i64::try_from(segundos).map_err(|_| "la relectura es enorme".to_string())? * 1_000_000;
    Ok(texto_de_instante(us.saturating_sub(resta)))
}

/// El valor de OOS de una celda de texto. `None` es el nulo.
pub fn convertir(c: &Campo, t: Option<&str>) -> Result<Option<Valor>, String> {
    let Some(t) = t else { return Ok(None) };
    Ok(Some(match fisico(c).0 {
        Fisico::Entero => {
            // Salesforce a veces dice `10.0` de un entero.
            let t = t.strip_suffix(".0").unwrap_or(t);
            Valor::Entero(t.parse().map_err(|_| format!("`{t}` no es un entero"))?)
        }
        Fisico::Real => Valor::Real(
            t.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(|| format!("`{t}` no es un número"))?,
        ),
        Fisico::Decimal { precision, escala } => Valor::Decimal(decimal(t, precision, escala)?),
        Fisico::Logico => match t {
            "true" => Valor::Logico(true),
            "false" => Valor::Logico(false),
            _ => return Err(format!("`{t}` no es `true` ni `false`")),
        },
        Fisico::Fecha => Valor::Fecha(fecha(t)?),
        Fisico::Instante => Valor::Instante(instante(t)?),
        _ => Valor::Texto(t.to_string()),
    }))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::esquema::pruebas::cuenta;

    fn campo(n: &str) -> Campo {
        cuenta().campo(n).unwrap().clone()
    }

    #[test]
    fn las_fechas_son_dias_desde_la_epoca() {
        assert_eq!(fecha("1970-01-01"), Ok(0));
        assert_eq!(fecha("1970-01-02"), Ok(1));
        assert_eq!(fecha("2000-03-01"), Ok(11_017));
        assert_eq!(fecha("1969-12-31"), Ok(-1));
        // 2026-12-31 (la CloseDate de la siembra del X0).
        assert_eq!(fecha("2026-12-31"), Ok(20_818));
        assert!(fecha("2023-02-29").is_err(), "2023 no es bisiesto");
        assert_eq!(fecha("2024-02-29"), Ok(19_782));
        assert!(fecha("2026-13-01").is_err());
        assert!(fecha("26-1-1").is_err());
    }

    /// X0 C10: REST dice `+0000` y Bulk dice `Z`; son el mismo instante.
    #[test]
    fn los_dos_formatos_del_instante_dan_lo_mismo() {
        let rest = instante("2026-10-31T20:39:17.000+0000").unwrap();
        let bulk = instante("2026-10-31T20:39:17.000Z").unwrap();
        assert_eq!(rest, bulk);
        assert_eq!(instante("1970-01-01T00:00:00.000Z"), Ok(0));
        assert_eq!(instante("1970-01-01T00:00:01.5Z"), Ok(1_500_000));
        // Con desfase: las 02:00 en +02:00 son las 00:00 UTC.
        assert_eq!(
            instante("1970-01-01T02:00:00+02:00"),
            instante("1970-01-01T00:00:00Z")
        );
        assert_eq!(rest, 1_793_479_157_000_000);
    }

    #[test]
    fn un_instante_que_no_cabe_en_microsegundos_se_rechaza() {
        assert!(
            instante("2026-10-31T20:39:17.1234567Z")
                .unwrap_err()
                .contains("microsegundos")
        );
        assert_eq!(
            instante("2026-10-31T20:39:17.1234560Z"),
            instante("2026-10-31T20:39:17.123456Z")
        );
        assert!(instante("ayer").is_err());
        assert!(instante("2026-10-31T25:00:00Z").is_err());
    }

    /// X0 C10: `20.0` por una fórmula y `10` por un entero, en un `Decimal<18, 0>`.
    #[test]
    fn el_decimal_es_exacto_y_no_redondea() {
        assert_eq!(decimal("20.0", 18, 0), Ok(20));
        assert_eq!(decimal("10", 18, 0), Ok(10));
        assert_eq!(decimal("-1.5", 18, 2), Ok(-150));
        assert_eq!(decimal("1.005", 18, 3), Ok(1005));
        assert_eq!(decimal("1.5e2", 18, 0), Ok(150));
        assert_eq!(decimal("0.00", 18, 0), Ok(0));
        // El redondeo no existe: 1.005 no cabe en dos decimales.
        assert!(
            decimal("1.005", 18, 2)
                .unwrap_err()
                .contains("sin redondear")
        );
        assert!(decimal("1e30", 18, 0).is_err(), "no cabe en 18 cifras");
        assert!(decimal("abc", 18, 0).is_err());
        assert!(decimal("", 18, 0).is_err());
    }

    /// El testigo es texto, ordinal y sin pérdida: lo que sale se vuelve a leer.
    #[test]
    fn un_instante_va_y_viene_por_el_texto() {
        assert_eq!(texto_de_instante(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            texto_de_instante(1_793_479_157_000_000),
            "2026-10-31T20:39:17Z"
        );
        assert_eq!(texto_de_instante(-1_000_000), "1969-12-31T23:59:59Z");
        // Los microsegundos se truncan a segundos (la precisión de SystemModstamp).
        assert_eq!(
            texto_de_instante(1_793_479_157_999_999),
            "2026-10-31T20:39:17Z"
        );
        for t in [
            "2000-02-29T23:59:59Z",
            "2026-12-31T00:00:00Z",
            "1999-03-01T12:00:00Z",
        ] {
            assert_eq!(texto_de_instante(instante(t).unwrap()), t);
        }
        // Es ordinal como texto: comparar cadenas es comparar instantes.
        assert!(texto_de_instante(1_000_000_000_000) < texto_de_instante(1_793_479_157_000_000));
    }

    /// La hora de la org sale de la cabecera `Date` (X0 E: «2026-10-31T20:44:54Z»).
    #[test]
    fn la_hora_de_la_org_sale_de_la_cabecera_date() {
        assert_eq!(
            instante_de_http("Sat, 31 Oct 2026 20:44:54 GMT").map(texto_de_instante),
            Ok("2026-10-31T20:44:54Z".to_string())
        );
        assert_eq!(instante_de_http("Thu, 01 Jan 1970 00:00:00 GMT"), Ok(0));
        assert!(instante_de_http("ayer").is_err());
        assert!(instante_de_http("Sat, 31 Foo 2026 20:44:54 GMT").is_err());
    }

    #[test]
    fn la_relectura_retrasa_el_inicio_sin_romper_el_formato() {
        assert_eq!(
            retrasar("2026-10-31T20:39:17Z", 600).unwrap(),
            "2026-10-31T20:29:17Z"
        );
        assert_eq!(
            retrasar("2026-11-01T00:00:05Z", 10).unwrap(),
            "2026-10-31T23:59:55Z"
        );
        assert_eq!(
            retrasar("2026-10-31T20:39:17.000+0000", 0).unwrap(),
            "2026-10-31T20:39:17Z"
        );
        assert!(retrasar("ayer", 1).is_err());
    }

    #[test]
    fn los_tipos_de_oos_de_cada_campo() {
        assert_eq!(fisico(&campo("NumberOfEmployees")).1, "Integer");
        assert_eq!(fisico(&campo("AnnualRevenue")).1, "Decimal<18, 0>");
        assert_eq!(fisico(&campo("X0_Formula__c")).1, "Decimal<18, 0>");
        assert_eq!(fisico(&campo("IsDeleted")).1, "Boolean");
        assert_eq!(fisico(&campo("SystemModstamp")).1, "DateTimeTz");
        assert_eq!(fisico(&campo("Name")).1, "String");
        assert_eq!(fisico(&campo("Id")).1, "String");
    }

    #[test]
    fn convertir_una_celda_por_su_tipo() {
        let c = |n: &str, t: &str| convertir(&campo(n), Some(t));
        assert_eq!(c("NumberOfEmployees", "10"), Ok(Some(Valor::Entero(10))));
        assert_eq!(c("NumberOfEmployees", "10.0"), Ok(Some(Valor::Entero(10))));
        assert_eq!(c("X0_Formula__c", "20.0"), Ok(Some(Valor::Decimal(20))));
        assert_eq!(c("IsDeleted", "false"), Ok(Some(Valor::Logico(false))));
        assert_eq!(
            c("Name", "NA"),
            Ok(Some(Valor::Texto("NA".into()))),
            "`NA` es un nombre, no un nulo"
        );
        assert_eq!(convertir(&campo("Name"), None), Ok(None));
        assert!(c("NumberOfEmployees", "diez").is_err());
        assert!(c("IsDeleted", "quizas").is_err());
        assert!(c("SystemModstamp", "ayer").is_err());
    }
}
