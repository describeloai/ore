//! **Las codificaciones**: como se guardan en HBase los bytes de un valor, y como se leen (ficha 0072, §7.5).
//!
//! HBase no tiene tipos: una celda son bytes. Un `Integer` puede ser un `int` de Java (4 bytes), un `long` (8) o un
//! texto, y adivinarlo es corromper. La tabla lo dice —en el `physicalType` de la columna, que el motor manda en
//! `codificaciones`— con uno de estos nombres, **cerrados**. Todos son los de la biblioteca de Java
//! (`org.apache.hadoop.hbase.util.Bytes`), que es con lo que escribe casi todo el mundo:
//!
//! | nombre | bytes | tipos de OOS que puede dar |
//! |---|---|---|
//! | `utf8` | texto | `String` |
//! | `bytes` | los que haya | `Opaque` (en base64) |
//! | `texto` | el valor escrito (`"42"`, `"2026-01-01"`) | cualquiera de los de abajo |
//! | `i8be` `i16be` `i32be` `i64be` | `Bytes.toBytes(byte/short/int/long)`: complemento a dos, big-endian | `Integer` |
//! | `u32be` `u64be` | sin signo | `Integer` |
//! | `f32be` `f64be` | IEEE 754, `Bytes.toBytes(float/double)` | `Float` |
//! | `bool` | `Bytes.toBytes(boolean)`: `0xFF` o `0x00` (cualquier distinto de cero es verdadero) | `Boolean` |
//! | `epoch-ms` | `Bytes.toBytes(long)` de milisegundos desde la época | `Date`, `DateTime`, `DateTimeTz` |
//! | `decimal` | `Bytes.toBytes(BigDecimal)`: la escala (4 bytes) y el valor sin escala (`BigInteger.toByteArray`) | `Decimal<p, s>` |
//!
//! **Un valor que no cabe en su tipo es un error que dice cual**: ni se redondea ni se convierte en nulo (0032 §4).

use crate::lote::{base64, de_base64};
use ore_base::tipos::{Fisico, Valor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codificacion {
    Utf8,
    Bytes,
    Texto,
    I8Be,
    I16Be,
    I32Be,
    I64Be,
    U32Be,
    U64Be,
    F32Be,
    F64Be,
    Bool,
    EpochMs,
    Decimal,
}

pub const TODAS: &[(&str, Codificacion)] = &[
    ("utf8", Codificacion::Utf8),
    ("bytes", Codificacion::Bytes),
    ("texto", Codificacion::Texto),
    ("i8be", Codificacion::I8Be),
    ("i16be", Codificacion::I16Be),
    ("i32be", Codificacion::I32Be),
    ("i64be", Codificacion::I64Be),
    ("u32be", Codificacion::U32Be),
    ("u64be", Codificacion::U64Be),
    ("f32be", Codificacion::F32Be),
    ("f64be", Codificacion::F64Be),
    ("bool", Codificacion::Bool),
    ("epoch-ms", Codificacion::EpochMs),
    ("decimal", Codificacion::Decimal),
];

impl Codificacion {
    /// El nombre tal como lo escribe la tabla.
    pub fn nombre(self) -> &'static str {
        TODAS
            .iter()
            .find(|(_, c)| *c == self)
            .map_or("?", |(n, _)| n)
    }

    /// Por su nombre; mayusculas y minusculas valen igual.
    pub fn analizar(nombre: &str) -> Option<Codificacion> {
        let n = nombre.trim().to_ascii_lowercase();
        TODAS.iter().find(|(x, _)| *x == n).map(|(_, c)| *c)
    }

    /// Los nombres, para un mensaje de error.
    pub fn lista() -> String {
        TODAS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
    }

    /// ¿Puede esta codificacion dar este fisico? (`opaco`: el tipo es `Opaque`, que como fisico es texto.)
    pub fn sirve_para(self, f: &Fisico, opaco: bool) -> bool {
        use Codificacion::*;
        match (f, opaco) {
            (Fisico::Texto, true) => self == Bytes,
            (Fisico::Texto, false) => self == Utf8,
            (Fisico::Entero, _) => {
                matches!(self, I8Be | I16Be | I32Be | I64Be | U32Be | U64Be | Texto)
            }
            (Fisico::Real, _) => matches!(self, F32Be | F64Be | Texto),
            (Fisico::Logico, _) => matches!(self, Bool | Texto),
            (Fisico::Decimal { .. }, _) => matches!(self, Decimal | Texto),
            (Fisico::Fecha | Fisico::FechaHora | Fisico::Instante, _) => {
                matches!(self, EpochMs | Texto)
            }
            // `Time` no tiene codificacion binaria acordada; lo demas no es un escalar.
            (Fisico::Hora, _) => self == Texto,
        }
    }

    /// El ancho fijo en bytes, si lo tiene.
    pub fn ancho(self) -> Option<usize> {
        use Codificacion::*;
        match self {
            I8Be | Bool => Some(1),
            I16Be => Some(2),
            I32Be | U32Be | F32Be => Some(4),
            I64Be | U64Be | F64Be | EpochMs => Some(8),
            Utf8 | Bytes | Texto | Decimal => None,
        }
    }
}

const MS_POR_DIA: i64 = 86_400_000;

fn ancho_exacto(b: &[u8], n: usize, cod: Codificacion) -> Result<(), String> {
    if b.len() == n {
        Ok(())
    } else {
        Err(format!(
            "`{}` espera {n} byte(s) y la celda tiene {}",
            cod.nombre(),
            b.len()
        ))
    }
}

/// Los bytes de un entero con signo, big-endian, como los lee `BigInteger(byte[])`.
fn entero_con_signo(b: &[u8]) -> i128 {
    let negativo = b.first().is_some_and(|x| x & 0x80 != 0);
    let mut v: i128 = if negativo { -1 } else { 0 };
    for x in b {
        v = (v << 8) | i128::from(*x);
    }
    v
}

/// `BigInteger.toByteArray()`: el menor numero de bytes que conserva el signo.
fn a_bytes_minimos(v: i128) -> Vec<u8> {
    let todos = v.to_be_bytes();
    let relleno = if v < 0 { 0xFF } else { 0x00 };
    let mut i = 0;
    while i + 1 < todos.len() && todos[i] == relleno && (todos[i + 1] & 0x80 != 0) == (v < 0) {
        i += 1;
    }
    todos[i..].to_vec()
}

fn pot10(n: u32) -> Option<i128> {
    10i128.checked_pow(n)
}

/// **Un `BigDecimal` de Java** llevado a la escala de la columna. Si tiene mas decimales de los que la escala
/// admite, solo cabe si los que sobran son ceros: nada se redondea.
fn decimal_de_java(b: &[u8], precision: u8, escala: u8) -> Result<i128, String> {
    if b.len() < 5 {
        return Err(format!(
            "un BigDecimal son 4 bytes de escala y al menos uno de valor; la celda tiene {}",
            b.len()
        ));
    }
    if b.len() > 4 + 16 {
        return Err(format!(
            "el valor sin escala ocupa {} bytes y no cabe en 128 bits",
            b.len() - 4
        ));
    }
    let escala_celda = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let sin_escala = entero_con_signo(&b[4..]);
    let dif = i64::from(escala) - i64::from(escala_celda);
    let valor = if dif >= 0 {
        let f = pot10(u32::try_from(dif).map_err(|_| "escala fuera de rango".to_string())?)
            .ok_or("el valor no cabe en 128 bits")?;
        sin_escala
            .checked_mul(f)
            .ok_or("el valor no cabe en 128 bits")?
    } else {
        let f = pot10(u32::try_from(-dif).map_err(|_| "escala fuera de rango".to_string())?)
            .ok_or("la escala de la celda es absurda")?;
        if sin_escala % f != 0 {
            return Err(format!(
                "tiene mas decimales ({escala_celda}) que la escala de la columna ({escala}) y los que sobran no son ceros: no se redondea"
            ));
        }
        sin_escala / f
    };
    let limite = pot10(u32::from(precision)).ok_or("precision fuera de rango")?;
    if valor.abs() >= limite {
        return Err(format!("pasa de los {precision} digitos de la columna"));
    }
    Ok(valor)
}

/// **Los bytes de una celda, como un valor de OOS** en la unidad en que Arrow lo guarda.
pub fn decodificar(cod: Codificacion, fisico: &Fisico, b: &[u8]) -> Result<Valor, String> {
    use Codificacion::*;
    Ok(match cod {
        Utf8 => Valor::Texto(
            std::str::from_utf8(b)
                .map_err(|e| format!("los bytes no son UTF-8 (falla en el byte {}): la columna debe ser `Opaque` con `bytes`", e.valid_up_to()))?
                .to_string(),
        ),
        Bytes => Valor::Texto(base64(b)),
        Texto => {
            let t = std::str::from_utf8(b).map_err(|_| "el texto no es UTF-8".to_string())?;
            fisico
                .analizar(t)
                .ok_or_else(|| format!("`{t}` no es un valor de {} escrito como texto", fisico.nombre()))?
        }
        I8Be => {
            ancho_exacto(b, 1, cod)?;
            Valor::Entero(i64::from(b[0] as i8))
        }
        I16Be => {
            ancho_exacto(b, 2, cod)?;
            Valor::Entero(i64::from(i16::from_be_bytes([b[0], b[1]])))
        }
        I32Be => {
            ancho_exacto(b, 4, cod)?;
            Valor::Entero(i64::from(i32::from_be_bytes([b[0], b[1], b[2], b[3]])))
        }
        I64Be => {
            ancho_exacto(b, 8, cod)?;
            Valor::Entero(i64::from_be_bytes(b.try_into().expect("8 bytes")))
        }
        U32Be => {
            ancho_exacto(b, 4, cod)?;
            Valor::Entero(i64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])))
        }
        U64Be => {
            ancho_exacto(b, 8, cod)?;
            let n = u64::from_be_bytes(b.try_into().expect("8 bytes"));
            Valor::Entero(i64::try_from(n).map_err(|_| format!("{n} no cabe en un Integer (64 bits con signo)"))?)
        }
        F32Be => {
            ancho_exacto(b, 4, cod)?;
            Valor::Real(f64::from(f32::from_be_bytes([b[0], b[1], b[2], b[3]])))
        }
        F64Be => {
            ancho_exacto(b, 8, cod)?;
            Valor::Real(f64::from_be_bytes(b.try_into().expect("8 bytes")))
        }
        Bool => {
            ancho_exacto(b, 1, cod)?;
            Valor::Logico(b[0] != 0)
        }
        EpochMs => {
            ancho_exacto(b, 8, cod)?;
            let ms = i64::from_be_bytes(b.try_into().expect("8 bytes"));
            match fisico {
                Fisico::Fecha => Valor::Fecha(
                    i32::try_from(ms.div_euclid(MS_POR_DIA)).map_err(|_| format!("{ms} ms no cabe en una fecha"))?,
                ),
                Fisico::FechaHora => Valor::FechaHora(
                    ms.checked_mul(1000).ok_or_else(|| format!("{ms} ms desborda los microsegundos de 64 bits"))?,
                ),
                Fisico::Instante => Valor::Instante(
                    ms.checked_mul(1000).ok_or_else(|| format!("{ms} ms desborda los microsegundos de 64 bits"))?,
                ),
                otro => return Err(format!("`epoch-ms` no da {}", otro.nombre())),
            }
        }
        Decimal => match fisico {
            Fisico::Decimal { precision, escala } => Valor::Decimal(decimal_de_java(b, *precision, *escala)?),
            otro => return Err(format!("`decimal` no da {}", otro.nombre())),
        },
    })
}

/// **Un valor de OOS, como bytes** de la codificacion. Lo usan la clave (un rango sobre una clave tipada se
/// convierte en bytes), la siembra del kit y las pruebas. `Err` si el valor no cabe o no se puede representar.
pub fn codificar(cod: Codificacion, fisico: &Fisico, v: &Valor) -> Result<Vec<u8>, String> {
    use Codificacion::*;
    let no_cabe = |n: &str| format!("el valor no cabe en `{n}`");
    Ok(match (cod, v) {
        (Utf8, Valor::Texto(s)) => s.as_bytes().to_vec(),
        (Bytes, Valor::Texto(s)) => de_base64(s).ok_or_else(|| format!("`{s}` no es base64"))?,
        (Texto, v) => v.texto(fisico).into_bytes(),
        (I8Be, Valor::Entero(n)) => i8::try_from(*n)
            .map_err(|_| no_cabe("i8be"))?
            .to_be_bytes()
            .to_vec(),
        (I16Be, Valor::Entero(n)) => i16::try_from(*n)
            .map_err(|_| no_cabe("i16be"))?
            .to_be_bytes()
            .to_vec(),
        (I32Be, Valor::Entero(n)) => i32::try_from(*n)
            .map_err(|_| no_cabe("i32be"))?
            .to_be_bytes()
            .to_vec(),
        (I64Be, Valor::Entero(n)) => n.to_be_bytes().to_vec(),
        (U32Be, Valor::Entero(n)) => u32::try_from(*n)
            .map_err(|_| no_cabe("u32be"))?
            .to_be_bytes()
            .to_vec(),
        (U64Be, Valor::Entero(n)) => u64::try_from(*n)
            .map_err(|_| no_cabe("u64be"))?
            .to_be_bytes()
            .to_vec(),
        (F32Be, Valor::Real(x)) => (*x as f32).to_be_bytes().to_vec(),
        (F64Be, Valor::Real(x)) => x.to_be_bytes().to_vec(),
        (Bool, Valor::Logico(b)) => vec![if *b { 0xFF } else { 0x00 }],
        (EpochMs, Valor::Fecha(d)) => (i64::from(*d) * MS_POR_DIA).to_be_bytes().to_vec(),
        (EpochMs, Valor::FechaHora(us) | Valor::Instante(us)) => {
            if us % 1000 != 0 {
                return Err("tiene microsegundos y `epoch-ms` solo guarda milisegundos".into());
            }
            (us / 1000).to_be_bytes().to_vec()
        }
        (Decimal, Valor::Decimal(n)) => {
            let Fisico::Decimal { escala, .. } = fisico else {
                return Err("`decimal` da un Decimal".into());
            };
            let mut b = i32::from(*escala).to_be_bytes().to_vec();
            b.extend(a_bytes_minimos(*n));
            b
        }
        _ => return Err(format!("`{}` no escribe ese valor", cod.nombre())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(p: u8, s: u8) -> Fisico {
        Fisico::Decimal {
            precision: p,
            escala: s,
        }
    }

    /// Lo que `Bytes.toBytes` de Java produce (la biblioteca es la referencia; el laboratorio lo comprueba con la
    /// propia JVM en `tests/laboratorio.rs`).
    #[test]
    fn los_enteros_son_complemento_a_dos_big_endian() {
        let e = |c, b: &[u8]| decodificar(c, &Fisico::Entero, b).unwrap();
        assert_eq!(e(Codificacion::I32Be, &[0, 0, 0, 42]), Valor::Entero(42));
        assert_eq!(e(Codificacion::I32Be, &[0xFF; 4]), Valor::Entero(-1));
        assert_eq!(e(Codificacion::I16Be, &[0x01, 0x02]), Valor::Entero(258));
        assert_eq!(e(Codificacion::I8Be, &[0x80]), Valor::Entero(-128));
        assert_eq!(
            e(Codificacion::I64Be, &[0, 0, 0, 0, 0, 0, 0, 1]),
            Valor::Entero(1)
        );
        assert_eq!(
            e(Codificacion::I64Be, &i64::MIN.to_be_bytes()),
            Valor::Entero(i64::MIN)
        );
        assert_eq!(
            e(Codificacion::U32Be, &[0xFF; 4]),
            Valor::Entero(4_294_967_295)
        );
        assert_eq!(
            e(Codificacion::U64Be, &i64::MAX.to_be_bytes()),
            Valor::Entero(i64::MAX)
        );
    }

    #[test]
    fn un_u64_que_pasa_de_int64_no_cabe_y_un_ancho_equivocado_se_dice() {
        let e = decodificar(Codificacion::U64Be, &Fisico::Entero, &[0xFF; 8]).unwrap_err();
        assert!(e.contains("no cabe"), "{e}");
        let e = decodificar(Codificacion::I32Be, &Fisico::Entero, &[0, 0, 42]).unwrap_err();
        assert!(
            e.contains("i32be") && e.contains("4 byte") && e.contains("tiene 3"),
            "{e}"
        );
    }

    #[test]
    fn los_reales_son_ieee_big_endian() {
        let r = |c, b: &[u8]| decodificar(c, &Fisico::Real, b).unwrap();
        assert_eq!(
            r(Codificacion::F64Be, &[0x3F, 0xF0, 0, 0, 0, 0, 0, 0]),
            Valor::Real(1.0)
        );
        assert_eq!(
            r(Codificacion::F32Be, &[0x3F, 0x80, 0, 0]),
            Valor::Real(1.0)
        );
        assert_eq!(
            r(Codificacion::F64Be, &(-2.5f64).to_be_bytes()),
            Valor::Real(-2.5)
        );
        let Valor::Real(x) = r(Codificacion::F64Be, &f64::NAN.to_be_bytes()) else {
            panic!()
        };
        assert!(x.is_nan());
    }

    #[test]
    fn un_booleano_de_java_es_ff_o_00_y_cualquier_distinto_de_cero_es_verdadero() {
        let b = |x: u8| decodificar(Codificacion::Bool, &Fisico::Logico, &[x]).unwrap();
        assert_eq!(
            (b(0xFF), b(0x00), b(0x01)),
            (
                Valor::Logico(true),
                Valor::Logico(false),
                Valor::Logico(true)
            )
        );
        assert!(decodificar(Codificacion::Bool, &Fisico::Logico, &[]).is_err());
    }

    #[test]
    fn epoch_ms_da_dias_hora_de_pared_o_instante_segun_el_tipo() {
        let ms = 1_700_000_000_123i64.to_be_bytes();
        assert_eq!(
            decodificar(Codificacion::EpochMs, &Fisico::Instante, &ms).unwrap(),
            Valor::Instante(1_700_000_000_123_000)
        );
        assert_eq!(
            decodificar(Codificacion::EpochMs, &Fisico::FechaHora, &ms).unwrap(),
            Valor::FechaHora(1_700_000_000_123_000)
        );
        assert_eq!(
            decodificar(Codificacion::EpochMs, &Fisico::Fecha, &ms).unwrap(),
            Valor::Fecha(19_675)
        );
        // antes de la epoca: el dia es el que contiene el instante
        assert_eq!(
            decodificar(
                Codificacion::EpochMs,
                &Fisico::Fecha,
                &(-1i64).to_be_bytes()
            )
            .unwrap(),
            Valor::Fecha(-1)
        );
        assert!(
            decodificar(
                Codificacion::EpochMs,
                &Fisico::FechaHora,
                &i64::MAX.to_be_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn un_bigdecimal_de_java_se_lleva_a_la_escala_de_la_columna() {
        // BigDecimal("12.34"): escala 2, 1234 = 0x04D2
        let b = [0, 0, 0, 2, 0x04, 0xD2];
        assert_eq!(
            decodificar(Codificacion::Decimal, &dec(12, 2), &b).unwrap(),
            Valor::Decimal(1234)
        );
        // a una escala mayor se rellena con ceros: no pierde nada
        assert_eq!(
            decodificar(Codificacion::Decimal, &dec(12, 4), &b).unwrap(),
            Valor::Decimal(123_400)
        );
        // BigDecimal("-0.5"): escala 1, -5 = 0xFB
        assert_eq!(
            decodificar(Codificacion::Decimal, &dec(12, 2), &[0, 0, 0, 1, 0xFB]).unwrap(),
            Valor::Decimal(-50)
        );
        // BigDecimal("0"): escala 0, 0
        assert_eq!(
            decodificar(Codificacion::Decimal, &dec(12, 2), &[0, 0, 0, 0, 0]).unwrap(),
            Valor::Decimal(0)
        );
        // BigDecimal("1.500") a escala 2: el cero de mas no cambia el valor
        assert_eq!(
            decodificar(
                Codificacion::Decimal,
                &dec(12, 2),
                &[0, 0, 0, 3, 0x05, 0xDC]
            )
            .unwrap(),
            Valor::Decimal(150)
        );
    }

    #[test]
    fn un_decimal_con_mas_decimales_que_la_escala_no_se_redondea_ni_pasa_de_la_precision() {
        // BigDecimal("12.345") a escala 2
        let e = decodificar(
            Codificacion::Decimal,
            &dec(12, 2),
            &[0, 0, 0, 3, 0x30, 0x39],
        )
        .unwrap_err();
        assert!(e.contains("no se redondea"), "{e}");
        // 123 con precision 2
        let e = decodificar(Codificacion::Decimal, &dec(2, 0), &[0, 0, 0, 0, 123]).unwrap_err();
        assert!(e.contains("digitos"), "{e}");
        assert!(decodificar(Codificacion::Decimal, &dec(12, 2), &[0, 0, 0]).is_err());
    }

    #[test]
    fn biginteger_to_byte_array_es_el_minimo_con_signo() {
        for (n, esperado) in [
            (0i128, vec![0x00]),
            (1, vec![0x01]),
            (127, vec![0x7F]),
            (128, vec![0x00, 0x80]),
            (255, vec![0x00, 0xFF]),
            (256, vec![0x01, 0x00]),
            (-1, vec![0xFF]),
            (-128, vec![0x80]),
            (-129, vec![0xFF, 0x7F]),
            (-256, vec![0xFF, 0x00]),
        ] {
            assert_eq!(a_bytes_minimos(n), esperado, "{n}");
            assert_eq!(entero_con_signo(&esperado), n, "{n}");
        }
    }

    #[test]
    fn el_texto_se_analiza_con_la_forma_canonica_del_tipo() {
        assert_eq!(
            decodificar(Codificacion::Texto, &Fisico::Entero, b"42").unwrap(),
            Valor::Entero(42)
        );
        assert_eq!(
            decodificar(Codificacion::Texto, &Fisico::Logico, b"true").unwrap(),
            Valor::Logico(true)
        );
        assert_eq!(
            decodificar(Codificacion::Texto, &Fisico::Fecha, b"2026-01-31").unwrap(),
            Valor::Fecha(20_484)
        );
        let e = decodificar(Codificacion::Texto, &Fisico::Entero, b"cuarenta").unwrap_err();
        assert!(e.contains("cuarenta"), "{e}");
        // ni una cifra con espacios ni un `+` se arreglan
        assert!(decodificar(Codificacion::Texto, &Fisico::Entero, b" 42").is_err());
    }

    #[test]
    fn utf8_y_bytes() {
        assert_eq!(
            decodificar(Codificacion::Utf8, &Fisico::Texto, "canción".as_bytes()).unwrap(),
            Valor::Texto("canción".into())
        );
        let e = decodificar(Codificacion::Utf8, &Fisico::Texto, &[0xFF, 0xFE]).unwrap_err();
        assert!(e.contains("Opaque"), "{e}");
        assert_eq!(
            decodificar(Codificacion::Bytes, &Fisico::Texto, &[0, 0, 0, 42]).unwrap(),
            Valor::Texto("AAAAKg==".into())
        );
    }

    #[test]
    fn codificar_y_decodificar_son_inversas() {
        let casos: Vec<(Codificacion, Fisico, Valor)> = vec![
            (Codificacion::I8Be, Fisico::Entero, Valor::Entero(-7)),
            (Codificacion::I16Be, Fisico::Entero, Valor::Entero(30_000)),
            (
                Codificacion::I32Be,
                Fisico::Entero,
                Valor::Entero(i64::from(i32::MIN)),
            ),
            (Codificacion::I64Be, Fisico::Entero, Valor::Entero(i64::MAX)),
            (
                Codificacion::U32Be,
                Fisico::Entero,
                Valor::Entero(4_000_000_000),
            ),
            (Codificacion::U64Be, Fisico::Entero, Valor::Entero(i64::MAX)),
            (Codificacion::F32Be, Fisico::Real, Valor::Real(0.5)),
            (Codificacion::F64Be, Fisico::Real, Valor::Real(-1e300)),
            (Codificacion::Bool, Fisico::Logico, Valor::Logico(false)),
            (
                Codificacion::EpochMs,
                Fisico::Instante,
                Valor::Instante(1_700_000_000_123_000),
            ),
            (
                Codificacion::EpochMs,
                Fisico::FechaHora,
                Valor::FechaHora(-1_000),
            ),
            (Codificacion::EpochMs, Fisico::Fecha, Valor::Fecha(-3)),
            (Codificacion::Decimal, dec(12, 2), Valor::Decimal(-123_456)),
            (
                Codificacion::Decimal,
                dec(38, 10),
                Valor::Decimal(i128::from(i64::MAX) * 3),
            ),
            (Codificacion::Texto, Fisico::Entero, Valor::Entero(-99)),
            (
                Codificacion::Utf8,
                Fisico::Texto,
                Valor::Texto("hola".into()),
            ),
            (
                Codificacion::Bytes,
                Fisico::Texto,
                Valor::Texto("AAAAKg==".into()),
            ),
        ];
        for (c, f, v) in casos {
            let b = codificar(c, &f, &v).unwrap_or_else(|e| panic!("{c:?}: {e}"));
            assert_eq!(decodificar(c, &f, &b).unwrap(), v, "{c:?}");
        }
    }

    #[test]
    fn codificar_lo_que_no_cabe_es_un_error() {
        assert!(codificar(Codificacion::I8Be, &Fisico::Entero, &Valor::Entero(128)).is_err());
        assert!(codificar(Codificacion::U32Be, &Fisico::Entero, &Valor::Entero(-1)).is_err());
        let e = codificar(
            Codificacion::EpochMs,
            &Fisico::Instante,
            &Valor::Instante(1_500),
        )
        .unwrap_err();
        assert!(e.contains("microsegundos"), "{e}");
        assert!(
            codificar(
                Codificacion::I32Be,
                &Fisico::Entero,
                &Valor::Texto("x".into())
            )
            .is_err()
        );
    }

    #[test]
    fn los_nombres_son_cerrados_y_la_matriz_dice_que_tipo_sirve_con_cual() {
        assert_eq!(Codificacion::analizar(" I32BE "), Some(Codificacion::I32Be));
        assert_eq!(
            Codificacion::analizar("epoch-ms"),
            Some(Codificacion::EpochMs)
        );
        assert_eq!(Codificacion::analizar("numeric(10,2)"), None);
        assert_eq!(TODAS.len(), 14);
        for (n, c) in TODAS {
            assert_eq!(c.nombre(), *n);
        }
        assert!(Codificacion::I32Be.sirve_para(&Fisico::Entero, false));
        assert!(!Codificacion::I32Be.sirve_para(&Fisico::Real, false));
        assert!(
            !Codificacion::Utf8.sirve_para(&Fisico::Texto, true),
            "Opaque es bytes, no utf8"
        );
        assert!(Codificacion::Bytes.sirve_para(&Fisico::Texto, true));
        assert!(Codificacion::EpochMs.sirve_para(&Fisico::Fecha, false));
        assert!(!Codificacion::Decimal.sirve_para(&Fisico::Entero, false));
    }
}
