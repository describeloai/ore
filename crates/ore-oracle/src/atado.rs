//! **Los parámetros, atados con su tipo.**
//!
//! `ore-sql` pone `:1`, `:2`… y devuelve los valores como texto canónico de OOS (`2026-10-03T23:59:59`,
//! `2099-12-31T23:59:59.999999Z`, `false`). Oracle convierte un parámetro de texto según el `NLS` de la sesión,
//! y ese formato (con `T` y `Z`) no es el suyo: `ORA-01861`. Fijar un `NLS` que lo admita no basta (la `Z` no
//! existe en ninguna máscara de zona) y dependería de cómo se haya abierto la sesión.
//!
//! Lo que sí se puede es **no pasar por el texto**: se sabe la columna con la que se compara cada parámetro
//! ([`ore_sql::Consulta::columnas_de_parametros`]) y su tipo en el catálogo, así que una fecha se ata como
//! [`OracleTimestamp`], un instante como un [`OracleTimestamp`] con zona y un lógico como `bool`. Lo demás
//! (números, textos) va como texto: Oracle coacciona un `VARCHAR2` a `NUMBER` sin ambigüedad.

use crate::tipos::Tipo;
use crate::transporte::Tabla;
use oracledb::{OracleTimestamp, ToDbValue};
use ore_base::tipos::{Fisico, Valor};
use ore_driver::Fallo;

/// Los parámetros de la consulta, atados según el tipo de la columna con que se comparan.
pub fn atar(
    tabla: &Tabla,
    parametros: &[String],
    columnas: &[String],
) -> Result<Vec<Box<dyn ToDbValue>>, Fallo> {
    parametros
        .iter()
        .zip(columnas)
        .map(|(valor, columna)| atar_uno(tabla, valor, columna))
        .collect()
}

fn atar_uno(tabla: &Tabla, valor: &str, columna: &str) -> Result<Box<dyn ToDbValue>, Fallo> {
    let tipo = tabla.columna(columna).map(|c| &c.tipo);
    let no_es = |que: &str| {
        Fallo::operador(format!(
            "`{valor}` no es {que}, y es lo que se compara con `{columna}`"
        ))
    };
    Ok(match tipo {
        Some(Tipo::Fecha) => match Fisico::Fecha.analizar(valor) {
            Some(Valor::Fecha(d)) => {
                let (a, m, dia) = civil(i64::from(d));
                Box::new(OracleTimestamp::new_date(a, m, dia))
            }
            _ => return Err(no_es("una fecha (`2026-10-25`)")),
        },
        Some(Tipo::FechaHora) => match Fisico::FechaHora.analizar(valor) {
            Some(Valor::FechaHora(us)) => Box::new(desde_micros(us)),
            _ => return Err(no_es("una fecha y hora (`2026-10-25 13:14:15`)")),
        },
        Some(Tipo::Instante) => match Fisico::Instante.analizar(valor) {
            // El desfase 0 es UTC: el instante ya viene normalizado.
            Some(Valor::Instante(us)) => Box::new(desde_micros(us)),
            _ => return Err(no_es("un instante (`2026-10-25T13:14:15Z`)")),
        },
        // Un `RAW` se compara por sus bytes: el valor llega en base64 (el texto de `Binary`) y se ata como `RAW`.
        Some(Tipo::Binario) => match Fisico::Binario.analizar(valor) {
            Some(Valor::Bytes(b)) => Box::new(b),
            _ => return Err(no_es("base64 (los bytes de un `Binary`)")),
        },
        Some(Tipo::Logico) => match valor {
            "true" => Box::new(true),
            "false" => Box::new(false),
            _ => return Err(no_es("`true` o `false`")),
        },
        _ => Box::new(valor.to_string()),
    })
}

/// Los campos de un instante dado en microsegundos desde 1970-01-01T00:00.
fn desde_micros(us: i64) -> OracleTimestamp {
    const POR_DIA: i64 = 86_400_000_000;
    let (dias, resto) = (us.div_euclid(POR_DIA), us.rem_euclid(POR_DIA));
    let (a, m, d) = civil(dias);
    let (seg, micro) = (resto / 1_000_000, resto % 1_000_000);
    OracleTimestamp::new_timestamp(
        a,
        m,
        d,
        (seg / 3600) as u8,
        ((seg / 60) % 60) as u8,
        (seg % 60) as u8,
        (micro * 1000) as u32,
    )
}

/// `(año, mes, día)` de un número de días desde 1970-01-01 (el algoritmo del calendario gregoriano proléptico
/// de Howard Hinnant).
fn civil(dias: i64) -> (i16, u8, u8) {
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y as i16, m as u8, d as u8)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::transporte::Columna;

    fn tabla() -> Tabla {
        let c = |n: &str, t: Tipo| Columna {
            nombre: n.into(),
            tipo: t,
            tipo_oracle: String::new(),
            precision: None,
            escala: None,
            anulable: true,
            descripcion: None,
        };
        Tabla {
            esquema: "E".into(),
            nombre: "T".into(),
            columnas: vec![
                c("N", Tipo::Entero),
                c("D", Tipo::Fecha),
                c("H", Tipo::FechaHora),
                c("I", Tipo::Instante),
                c("L", Tipo::Logico),
                c("R", Tipo::Binario),
            ],
            clave: vec![],
            unicas: vec![],
        }
    }

    #[test]
    fn el_calendario_de_dias_a_fecha() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_751), (2026, 10, 25));
        assert_eq!(civil(-1), (1969, 12, 31));
        // El 29 de febrero de un año bisiesto de siglo.
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(-25_567), (1900, 1, 1));
    }

    #[test]
    fn los_microsegundos_a_campos() {
        let t = desde_micros(1_792_934_055_123_456);
        assert_eq!(
            (
                t.year(),
                t.month(),
                t.day(),
                t.hour(),
                t.minute(),
                t.second()
            ),
            (2026, 10, 25, 13, 14, 15)
        );
        assert_eq!(t.nanoseconds(), 123_456_000);
        // Antes de 1970, con fracción.
        let t = desde_micros(-1);
        assert_eq!(
            (
                t.year(),
                t.month(),
                t.day(),
                t.hour(),
                t.minute(),
                t.second(),
                t.nanoseconds()
            ),
            (1969, 12, 31, 23, 59, 59, 999_999_000)
        );
    }

    #[test]
    fn cada_columna_ata_su_tipo() {
        let t = tabla();
        let p = |v: &str, c: &str| atar(&t, &[v.to_string()], &[c.to_string()]);
        // El formato ISO de OOS (con `T` y `Z`) vale, que era lo que Oracle no entendía como texto.
        assert!(p("2026-10-03T23:59:59", "H").is_ok());
        assert!(p("2099-12-31T23:59:59.999999Z", "I").is_ok());
        assert!(p("2000-02-29", "D").is_ok());
        assert!(p("false", "L").is_ok());
        assert!(p("3q2+7w==", "R").is_ok());
        assert!(p("-9223372036854775808", "N").is_ok());
        // Un valor que no es del tipo de su columna es un defecto de quien pidió.
        for (v, c) in [
            ("ayer", "H"),
            ("2026-10-03", "I"),
            ("2026-13-01", "D"),
            ("quizas", "L"),
            ("no es base64!", "R"),
        ] {
            let e = p(v, c).err().unwrap_or_else(|| panic!("{v} en {c}"));
            assert_eq!(e.codigo, ore_driver::Codigo::Operador, "{v}");
        }
        // Una columna que la tabla no tiene va como texto: ya lo habrá rechazado la proyección.
        assert!(p("x", "OTRA").is_ok());
    }
}
