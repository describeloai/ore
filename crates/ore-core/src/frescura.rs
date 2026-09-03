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

/// **Si el testigo de una copia sigue dentro de la retencion del origen.**
///
/// `changes.retention` dice *«cuanto guarda el origen su changelog, si se
/// sabe»*, y lleva desde v1alpha8 sin ningun consumidor. Este es el que le
/// faltaba, y contesta la unica pregunta que ese campo permite contestar:
/// **¿puede este refresco ser incremental, o hay que rehacer la copia entera?**
///
/// # Por que devuelve tres cosas y no un booleano
///
/// Porque *«no se sabe»* no es *«si»* ni *«no»*, y tratarlo como cualquiera de
/// los dos es como se cuelan los fallos que no dan sintoma. Un origen sin
/// `retention` declarada **no afirma que guarde para siempre**: afirma que no lo
/// dice, y ahi la decision es de quien programe el refresco.
///
/// # Y por que el instante llega de fuera
///
/// Porque **aqui no hay reloj**, igual que en [`retraso`] y en el veredicto de
/// la cache. El compilador es hermetico por invariante —sin red, sin
/// credenciales, sin reloj— asi que esta funcion no puede saber que dia es.
/// Quien la llama lo sabe o no la puede llamar.
///
/// # Lo que esto NO detecta, y quien lo detecta
///
/// Que el origen haya perdido la posicion de verdad. Esto compara con lo
/// **declarado**, que es una promesa, no un hecho: el changelog puede haberse
/// truncado antes por un `VACUUM`, una restauracion o un cambio de politica.
///
/// La caducidad real **la dice el origen negandose**, que es lo que hacen los
/// cuatro sistemas que el ADR 0016 midio: Debezium vuelve a hacer la
/// instantanea, el *stream* de Snowflake pasa a `STALE`, Delta no arranca el
/// flujo y BigQuery da error. Esta funcion sirve para **anticiparlo** —para eso
/// esta el campo— no para sustituirlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alcance {
    /// El testigo cabe. Sobran estos segundos antes de que deje de caber.
    Dentro { margen: i64 },
    /// El testigo quedo fuera por estos segundos. Un refresco incremental
    /// pedira una posicion que el origen ya no guarda.
    Fuera { pasado: i64 },
    /// Falta algun dato. **No es «dentro»**, y por eso no se colapsa con el:
    /// sin `retention` declarada, o sin testigo poblado, no se afirma nada.
    NoSeSabe,
}

impl Alcance {
    /// Qué hacer. Va aquí y no en quien lo imprime para que la diferencia entre
    /// **refrescar** y **rehacer** salga por la boca de la herramienta.
    pub const fn remedio(&self) -> &'static str {
        match self {
            Alcance::Dentro { .. } => "un refresco incremental vale",
            Alcance::Fuera { .. } => {
                "hay que rehacer la copia entera: el origen ya no guarda esa posicion"
            }
            Alcance::NoSeSabe => "sin retencion declarada o sin testigo poblado no se afirma nada",
        }
    }
}

/// El calculo. `marca` es el testigo de la copia y `retencion` lo que el origen
/// declara guardar; los dos ISO-8601, porque es lo unico que se puede restar.
pub fn alcance(marca: Option<&str>, instante: &str, retencion: Option<&str>) -> Alcance {
    let (Some(m), Some(r)) = (marca, retencion) else {
        return Alcance::NoSeSabe;
    };
    let (Some(m), Some(t), Some(d)) = (epoca(m), epoca(instante), duracion(r)) else {
        return Alcance::NoSeSabe;
    };
    // El limite es el instante menos lo que se guarda: por debajo de ahi, el
    // origen ya no tiene con que servir un incremento.
    let limite = t - d;
    if m >= limite {
        Alcance::Dentro { margen: m - limite }
    } else {
        Alcance::Fuera { pasado: limite - m }
    }
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

    // ── R1 · `changes.retention` deja de ser decorativa ─────────────────────

    const AHORA: &str = "2026-09-03T12:00:00Z";

    /// Un testigo de hace tres días con siete de retención cabe, y sobran
    /// cuatro. El margen se devuelve porque es lo que permite **anticipar**:
    /// con él se sabe cuánto se puede tardar en volver, que es exactamente para
    /// lo que el campo dice existir.
    #[test]
    fn un_testigo_dentro_de_la_retencion_dice_cuanto_margen_queda() {
        assert_eq!(
            alcance(Some("2026-08-31T12:00:00Z"), AHORA, Some("7d")),
            Alcance::Dentro { margen: 4 * 86_400 }
        );
    }

    /// Y uno de hace nueve días, no. Se dice **por cuánto**, porque «caducó» y
    /// «caducó hace dos días» piden cosas distintas a quien programa.
    #[test]
    fn un_testigo_fuera_dice_por_cuanto_y_que_hay_que_rehacerla() {
        let a = alcance(Some("2026-08-25T12:00:00Z"), AHORA, Some("7d"));
        assert_eq!(a, Alcance::Fuera { pasado: 2 * 86_400 });
        assert!(a.remedio().contains("rehacer la copia entera"), "{a:?}");
    }

    /// **Y lo que no se sabe no se afirma.** Sin `retention` declarada el origen
    /// no promete guardar para siempre: promete no decirlo. Colapsarlo con
    /// `Dentro` sería inventar una garantía, y es la clase de fallo que no da
    /// síntoma hasta que el refresco falla contra el origen.
    #[test]
    fn sin_retencion_o_sin_testigo_no_se_afirma_nada() {
        assert_eq!(
            alcance(Some("2026-08-31T12:00:00Z"), AHORA, None),
            Alcance::NoSeSabe
        );
        assert_eq!(alcance(None, AHORA, Some("7d")), Alcance::NoSeSabe);
        // Y una retención que nadie sabe leer tampoco vale cero: no vale.
        assert_eq!(
            alcance(Some("2026-08-31T12:00:00Z"), AHORA, Some("7x")),
            Alcance::NoSeSabe
        );
    }

    /// El borde exacto cabe. Un testigo justo en el límite todavía se puede
    /// servir: la retención dice **cuánto se guarda**, y lo guardado incluye su
    /// extremo. Sin esta prueba, un `>` en vez de un `>=` haría que una copia
    /// perfectamente refrescable se rehiciera entera una vez por ciclo.
    #[test]
    fn el_borde_exacto_todavia_cabe() {
        assert_eq!(
            alcance(Some("2026-08-27T12:00:00Z"), AHORA, Some("7d")),
            Alcance::Dentro { margen: 0 }
        );
    }
}
