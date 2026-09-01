//! El tiempo, leído sin biblioteca de fechas.
//!
//! Dos funciones y una resta. Están aquí y no donde se usan porque ya se usan en
//! **dos** sitios —el estado degradado de una respuesta y el veredicto de la
//! caché— y dos implementaciones de *«¿esto está rancio?»* que se contestan
//! distinto es exactamente la clase de divergencia que no tiene aspecto de
//! fallo: las dos devuelven un dato.
//!
//! # Por qué no entra `chrono`
//!
//! Por lo mismo que el evaluador de Cedar se quedó fuera: **el reloj es la
//! evidencia, no el argumento**. Aquí no se lee la hora — se recibe. Lo que
//! hace falta es convertir un ISO-8601 en un entero y una duración en segundos,
//! y eso son cuarenta líneas de aritmética civil. Traer una biblioteca de fechas
//! para eso metería en `ore` la crate que `dependencias.rs` veta desde el
//! principio.

/// Un instante ISO-8601 en UTC —`AAAA-MM-DDTHH:MM:SSZ`— a segundos de época.
///
/// El algoritmo de días es el civil estándar (Howard Hinnant): sirve para
/// cualquier año y no tiene tabla de meses que mantener.
pub fn epoca(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |a: usize, z: usize| iso.get(a..z)?.parse::<i64>().ok();
    let (y, m, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi, s) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let dias = era * 146_097 + doe - 719_468;
    Some(dias * 86_400 + h * 3_600 + mi * 60 + s)
}

/// `30m`, `2h`, `7d` → segundos. El vocabulario es cerrado a propósito: un
/// `freshnessSLA` con una unidad que nadie sabe leer se interpretaría como cero
/// o como infinito, y las dos lecturas son peligrosas en direcciones opuestas.
pub fn duracion(s: &str) -> Option<i64> {
    let (n, u) = s.split_at(s.len().checked_sub(1)?);
    let n: i64 = n.parse().ok()?;
    Some(match u {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3_600,
        "d" => n * 86_400,
        _ => return None,
    })
}

/// Cuántos segundos se ha pasado del SLA, si se ha pasado.
///
/// `None` cubre dos casos que **no** son lo mismo y aquí se juntan a propósito:
/// que esté dentro del SLA, y que falte alguno de los tres datos. Quien pregunta
/// tiene los tres o no los tiene, y el sitio para decidir qué se hace sin ellos
/// es el que sabe qué se estaba preguntando — no esta resta.
pub fn retraso(marca: &str, instante: &str, sla: &str) -> Option<i64> {
    let (m, t, d) = (epoca(marca)?, epoca(instante)?, duracion(sla)?);
    (t > m + d).then_some(t - m - d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_epoca_y_la_duracion_se_leen_sin_biblioteca() {
        assert_eq!(epoca("1970-01-01T00:00:00Z"), Some(0));
        // Contrastado contra `datetime` de Python, no contra la propia funcion:
        // comprobar un algoritmo con su propia salida no comprueba nada.
        assert_eq!(epoca("2026-08-31T12:00:00Z"), Some(1_788_177_600));
        // Un bisiesto, que es donde falla la aritmetica escrita a ojo.
        assert_eq!(
            epoca("2024-03-01T00:00:00Z").unwrap() - epoca("2024-02-29T00:00:00Z").unwrap(),
            86_400
        );
        assert_eq!(duracion("30m"), Some(1_800));
        assert_eq!(duracion("2h"), Some(7_200));
        // Una unidad que nadie sabe leer no vale cero: no vale.
        assert_eq!(duracion("30x"), None);
    }

    /// El retraso es una resta, y lo que importa es que no dispare **dentro**
    /// del SLA: un aviso de rancio sobre dato fresco entrena a ignorarlo.
    #[test]
    fn dentro_del_sla_no_hay_retraso() {
        assert_eq!(
            retraso("2026-08-31T10:00:00Z", "2026-08-31T10:59:00Z", "1h"),
            None
        );
        assert_eq!(
            retraso("2026-08-31T10:00:00Z", "2026-08-31T11:00:30Z", "1h"),
            Some(30)
        );
    }

    /// Y un dato que falta no se cuenta como fresco por accidente: devuelve
    /// `None` como el caso bueno, y por eso quien pregunta tiene que haber
    /// decidido antes qué hace sin los tres.
    #[test]
    fn sin_los_tres_datos_no_hay_resta() {
        assert_eq!(retraso("ayer", "2026-08-31T10:00:00Z", "1h"), None);
        assert_eq!(
            retraso("2026-08-31T10:00:00Z", "2026-08-31T10:00:00Z", ""),
            None
        );
    }
}
