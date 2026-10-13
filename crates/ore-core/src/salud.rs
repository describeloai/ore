//! **La salud de un activo** — OOS v1alpha30 (`spec/v1alpha30/01-la-salud.md`),
//! ADR 0063 S2.
//!
//! Dos preguntas que se contestaban en cinco sitios, y de cinco maneras:
//!
//! - **¿Hay algo que leer?** ([`legible`]). Hasta S2, sólo un puntero `copiada`
//!   o `al-dia`. Uno en `error` conservaba la versión buena de antes, pero
//!   nadie la servía: SQL y Python decían «la copia no está hecha» (0063 M5).
//!   La spec (§3.5, §4) obliga a lo contrario: **un refresco que falla no apaga
//!   lo último bueno**.
//! - **¿En qué estado está, y de cuándo es?** ([`bloque`]). El bloque de salud
//!   de §6, calculado y no guardado (0063 D-B): con lo que dice el puntero, la
//!   promesa de frescura y la hora, que **llega de fuera**. Aquí no hay reloj,
//!   como en [`crate::frescura`].
//!
//! # Lo que el puntero dice (lo escribe la copia, `ore materialize`)
//!
//! | campo | qué es |
//! |---|---|
//! | `datos_de` | `dataAsOf`: cuándo se leyó el origen para la versión que se sirve, tomado **antes** de leer |
//! | `comprobado` | `checkedAt`: la última pasada **posterior** que vio, con una medida, que el origen no había cambiado |
//! | `intento` | `{cuando, resultado, motivo?}`: la última pasada. `publicada`, `sin-cambios` (una medida lo respaldó), `sin-comprobar` (dijo «ya está» sin medida), `error`, `retenida` |
//!
//! Un puntero de antes de S2 no trae ninguno de los tres: su `dataAsOf` es
//! desconocido, y lo dice (`unverified`). **No se rellena con la hora del
//! commit**: es la de publicar, y la spec lo prohíbe (§3.4; 0063 M1).

use crate::frescura::{duracion, epoca};
use crate::json::Json;
use crate::parse::Node;

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// **Lo que se lee de un puntero**: el `metadata_location` de su versión
/// vigente o, mientras quede alguno, la `clave` de un sobre heredado. Con
/// `copiada`, `al-dia` **y `error`**: un refresco fallido no mueve la versión
/// vigente, que sigue siendo la última buena (v1alpha30 §4).
///
/// `None` si no hay ninguna versión: nunca se copió, o se copió y falló sin
/// haber tenido antes una buena.
pub fn legible(n: &Node) -> Option<String> {
    let estado = campo(n, "estado").unwrap_or_default();
    if !matches!(estado.as_str(), "copiada" | "al-dia" | "error") {
        return None;
    }
    campo(n, "metadata_location").or_else(|| campo(n, "clave"))
}

/// Si lo que se sirve es **la última buena** porque el último intento no
/// publicó (v1alpha30 §4).
pub fn sirve_la_ultima_buena(n: &Node) -> bool {
    legible(n).is_some() && campo(n, "estado").as_deref() == Some("error")
}

/// Un motivo de §6.
fn motivo(code: &str, since: Option<&str>, message: Option<&str>) -> Json {
    let mut m = vec![("code", Json::s(code))];
    // `since` es obligatorio en el esquema: sin hora conocida, la del cálculo
    // la pone quien llama (`bloque` recibe `ahora`).
    m.push(("since", Json::s(since.unwrap_or_default())));
    if let Some(t) = message.filter(|t| !t.is_empty()) {
        m.push(("message", Json::s(t)));
    }
    Json::obj(m)
}

fn nulo() -> Json {
    Json::Crudo("null".into())
}

/// **El bloque de salud** (v1alpha30 §6) de un activo materializado.
///
/// - `puntero`: el del activo, si hay.
/// - `promesa`: su `freshness` (v1alpha12), en el vocabulario de
///   [`crate::frescura::duracion`] (`30m`, `2h`, `7d`).
/// - `ahora`: segundos de época; la hora llega de fuera.
/// - `capas`: `(vigilancia, perfilado)`, las capas 1 y 2 encendidas (§5).
///
/// El estado es el peor de sus motivos (§2.1): `degraded` > `warning` >
/// `unknown` > `healthy`.
pub fn bloque(
    puntero: Option<&Node>,
    promesa: Option<&str>,
    ahora: i64,
    capas: (bool, bool),
) -> Json {
    let hoy = crate::frescura::iso(ahora);
    let mut motivos: Vec<(u8, Json)> = Vec::new();
    // El orden de §2.1, como rango: cuanto más alto, peor.
    const DEGRADED: u8 = 3;
    const WARNING: u8 = 2;
    const UNKNOWN: u8 = 1;

    let datos_de = puntero.and_then(|p| campo(p, "datos_de"));
    let comprobado = puntero.and_then(|p| campo(p, "comprobado"));
    let intento = puntero.and_then(|p| p.get("intento").map(|(_, i)| i.clone()));
    let intento_cuando = intento.as_ref().and_then(|i| campo(i, "cuando"));
    let intento_resultado = intento.as_ref().and_then(|i| campo(i, "resultado"));
    let hay_version = puntero.and_then(legible).is_some();
    let ultima_buena = puntero.is_some_and(sirve_la_ultima_buena);
    let estado = puntero.and_then(|p| campo(p, "estado")).unwrap_or_default();

    // ── El último intento ───────────────────────────────────────────────────
    if estado == "error" {
        let desde = intento_cuando.clone().unwrap_or_else(|| hoy.clone());
        let razon = puntero.and_then(|p| campo(p, "motivo"));
        let code = if intento_resultado.as_deref() == Some("retenida") {
            "refresh-held"
        } else {
            "refresh-failed"
        };
        motivos.push((DEGRADED, motivo(code, Some(&desde), razon.as_deref())));
    }
    if !hay_version {
        motivos.push((
            UNKNOWN,
            motivo(
                "never-refreshed",
                Some(intento_cuando.as_deref().unwrap_or(&hoy)),
                None,
            ),
        ));
    }

    // ── La frescura (§1) ────────────────────────────────────────────────────
    let t_datos = datos_de.as_deref().and_then(epoca);
    let t_comprobado = comprobado.as_deref().and_then(epoca);
    // Una comprobación sólo cuenta si es POSTERIOR a los datos: la de la misma
    // pasada que copió no dice nada de lo que pasó después.
    let t_comprobado = t_comprobado.filter(|c| t_datos.is_some_and(|d| *c > d));
    let retraso = t_datos.map(|d| (ahora - d).max(0));
    // La edad, acotada: el origen no había cambiado en `comprobado`, así que su
    // primer cambio no reflejado es posterior. Sin comprobación, no se sabe.
    let edad = t_comprobado.map(|c| (ahora - c).max(0));
    let limite = promesa.and_then(duracion);
    if hay_version {
        match (limite, edad.or(retraso)) {
            (Some(l), Some(e)) if e > l => motivos.push((
                DEGRADED,
                motivo(
                    "freshness-exceeded",
                    Some(&crate::frescura::iso(
                        t_comprobado.or(t_datos).unwrap_or(ahora) + l,
                    )),
                    None,
                ),
            )),
            (Some(_), Some(_)) => {}
            // Sin promesa, o sin hora: sólo una comprobación posterior respalda
            // algo. Sin ella, `unknown` (§3.2: nunca fresco sin medida).
            _ if t_comprobado.is_some() => {}
            _ => motivos.push((
                UNKNOWN,
                motivo(
                    "unverified",
                    Some(datos_de.as_deref().unwrap_or(&hoy)),
                    Some(if datos_de.is_none() {
                        "the time this data was read from its source is not known"
                    } else {
                        "nothing has checked whether the source changed since this data was read"
                    }),
                ),
            )),
        }
    }

    let peor = motivos.iter().map(|(r, _)| *r).max().unwrap_or(0);
    let state = match peor {
        DEGRADED => "degraded",
        WARNING => "warning",
        UNKNOWN => "unknown",
        _ => "healthy",
    };
    let mut freshness = vec![("lagSeconds", retraso.map(Json::Int).unwrap_or_else(nulo))];
    freshness.push(("ageSeconds", edad.map(Json::Int).unwrap_or_else(nulo)));
    if let Some(p) = promesa {
        freshness.push(("promise", Json::s(p)));
    }
    let mut b = vec![
        ("state", Json::s(state)),
        ("dataAsOf", datos_de.map(Json::s).unwrap_or_else(nulo)),
        (
            "checkedAt",
            t_comprobado
                .and(comprobado)
                .map(Json::s)
                .unwrap_or_else(nulo),
        ),
        ("sourceChangedAt", nulo()),
        ("freshness", Json::obj(freshness)),
        ("servingLastGood", Json::Bool(ultima_buena)),
        (
            "reasons",
            Json::Arr(motivos.into_iter().map(|(_, m)| m).collect()),
        ),
        (
            "layers",
            Json::obj([
                ("monitoring", Json::Bool(capas.0)),
                ("profiling", Json::Bool(capas.1)),
            ]),
        ),
    ];
    if let (Some(at), Some(r)) = (intento_cuando, intento_resultado) {
        let outcome = match r.as_str() {
            "publicada" => Some("published"),
            "sin-cambios" => Some("unchanged"),
            "error" => Some("failed"),
            "retenida" => Some("held"),
            // «Ya está» sin medida no es un refresco: no se cuenta como uno.
            _ => None,
        };
        if let Some(o) = outcome {
            b.push((
                "lastRefresh",
                Json::obj([("at", Json::s(at)), ("outcome", Json::s(o))]),
            ));
        }
    }
    Json::obj(b)
}

/// El estado, sin el resto del bloque: lo que se cuenta y se pinta.
pub fn estado(bloque: &Json) -> &str {
    match bloque {
        Json::Obj(m) => match m.get("state") {
            Some(Json::Str(s)) => s.as_str(),
            _ => "unknown",
        },
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(texto: &str) -> Node {
        crate::parse::parse(texto).expect("puntero de prueba")
    }

    const AHORA: &str = "2026-10-13T16:00:00Z";
    fn ahora() -> i64 {
        epoca(AHORA).unwrap()
    }
    fn motivos(b: &Json) -> Vec<String> {
        match b {
            Json::Obj(m) => match m.get("reasons") {
                Some(Json::Arr(a)) => a
                    .iter()
                    .filter_map(|r| match r {
                        Json::Obj(r) => match r.get("code") {
                            Some(Json::Str(c)) => Some(c.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            },
            _ => vec![],
        }
    }

    /// 0063 M5: la pasada falló por una columna renombrada y el puntero
    /// conserva la versión buena. Antes: «la copia no está hecha». Ahora se lee,
    /// `degraded`, con su motivo y su hora.
    #[test]
    fn un_refresco_fallido_no_apaga_lo_ultimo_bueno() {
        let n = p(
            r#"{"estado":"error","metadata_location":"gs://b/t/metadata/3.json",
            "datos_de":"2026-10-13T13:37:02Z","motivo":"the source has no column `cliente`",
            "intento":{"cuando":"2026-10-13T13:44:51Z","resultado":"error"}}"#,
        );
        assert_eq!(legible(&n).as_deref(), Some("gs://b/t/metadata/3.json"));
        assert!(sirve_la_ultima_buena(&n));
        let b = bloque(Some(&n), None, ahora(), (false, false));
        assert_eq!(estado(&b), "degraded");
        assert_eq!(motivos(&b), ["refresh-failed", "unverified"]);
        let t = b.pretty();
        assert!(t.contains("\"servingLastGood\": true"), "{t}");
        assert!(t.contains("\"outcome\": \"failed\""), "{t}");
        assert!(t.contains("2026-10-13T13:37:02Z"), "{t}");
    }

    /// Un error sin versión anterior no tiene nada que servir.
    #[test]
    fn un_error_sin_version_no_se_lee() {
        let n = p(r#"{"estado":"error","motivo":"el tipo `vector` no se sabe leer"}"#);
        assert_eq!(legible(&n), None);
        let b = bloque(Some(&n), None, ahora(), (false, false));
        assert_eq!(estado(&b), "degraded");
        assert_eq!(motivos(&b), ["refresh-failed", "never-refreshed"]);
    }

    /// 0063 M6: «al-dia» sin haber mirado no es sano. Sin promesa y sin una
    /// comprobación posterior a los datos, `unknown`.
    #[test]
    fn al_dia_sin_comprobar_es_desconocido() {
        let n = p(r#"{"estado":"al-dia","metadata_location":"gs://b/t/m.json",
            "datos_de":"2026-10-02T21:27:00Z",
            "intento":{"cuando":"2026-10-13T09:08:06Z","resultado":"sin-comprobar"}}"#);
        let b = bloque(Some(&n), None, ahora(), (false, false));
        assert_eq!(estado(&b), "unknown");
        assert_eq!(motivos(&b), ["unverified"]);
        // Y no cuenta como refresco.
        assert!(!b.pretty().contains("lastRefresh"));
    }

    /// Un puntero de antes de S2: sin `datos_de`, la hora no se sabe, y no se
    /// rellena con la del commit (0063 M1).
    #[test]
    fn un_puntero_de_antes_no_inventa_su_hora() {
        let n = p(r#"{"estado":"al-dia","metadata_location":"gs://b/t/m.json"}"#);
        let b = bloque(Some(&n), Some("1h"), ahora(), (false, false));
        assert_eq!(estado(&b), "unknown");
        assert!(b.pretty().contains("\"dataAsOf\": null"));
    }

    /// Con una comprobación posterior a los datos, sin promesa: sano. La
    /// comprobación de la misma pasada que copió no cuenta.
    #[test]
    fn una_comprobacion_posterior_respalda() {
        let misma = p(
            r#"{"estado":"copiada","metadata_location":"m","datos_de":"2026-10-13T15:00:00Z",
            "comprobado":"2026-10-13T15:00:00Z"}"#,
        );
        assert_eq!(
            estado(&bloque(Some(&misma), None, ahora(), (false, false))),
            "unknown"
        );
        let despues = p(
            r#"{"estado":"al-dia","metadata_location":"m","datos_de":"2026-10-13T15:00:00Z",
            "comprobado":"2026-10-13T15:50:00Z",
            "intento":{"cuando":"2026-10-13T15:50:00Z","resultado":"sin-cambios"}}"#,
        );
        let b = bloque(Some(&despues), None, ahora(), (false, false));
        assert_eq!(estado(&b), "healthy");
        assert!(motivos(&b).is_empty());
        assert!(b.pretty().contains("\"ageSeconds\": 600"));
        assert!(b.pretty().contains("\"outcome\": \"unchanged\""));
    }

    /// Con promesa, el retraso basta para cumplirla (la edad nunca lo supera),
    /// y pasarse la rompe aunque nadie haya refrescado.
    #[test]
    fn la_promesa_se_juzga_con_el_reloj() {
        let n =
            p(r#"{"estado":"copiada","metadata_location":"m","datos_de":"2026-10-13T15:30:00Z"}"#);
        assert_eq!(
            estado(&bloque(Some(&n), Some("1h"), ahora(), (false, false))),
            "healthy"
        );
        let b = bloque(Some(&n), Some("15m"), ahora(), (false, false));
        assert_eq!(estado(&b), "degraded");
        assert_eq!(motivos(&b), ["freshness-exceeded"]);
        assert!(
            b.pretty().contains("2026-10-13T15:45:00Z"),
            "desde que venció"
        );
    }

    #[test]
    fn sin_puntero_nunca_se_refresco() {
        let b = bloque(None, None, ahora(), (true, false));
        assert_eq!(estado(&b), "unknown");
        assert_eq!(motivos(&b), ["never-refreshed"]);
        assert!(b.pretty().contains("\"monitoring\": true"));
    }
}
