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
        let mut m = motivo(code, Some(&desde), razon.as_deref());
        // §6: `refresh-held` dice qué guarda saltó (0063 S4: `zero-rows`).
        if let (Json::Obj(o), Some(g)) = (&mut m, intento.as_ref().and_then(|i| campo(i, "guarda")))
        {
            o.insert("guard".into(), Json::s(g));
        }
        motivos.push((DEGRADED, m));
    }
    // ── El esquema del origen (0063 S5a) ────────────────────────────────────
    //   Lo que la copia vio distinto de la `Table` declarada, con `columns`.
    if let Some((_, e)) = puntero.and_then(|p| p.get("esquema_origen")) {
        let lista = |k: &str| -> Vec<String> {
            e.get(k)
                .map(|(_, v)| {
                    v.items()
                        .iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let (added, removed, retyped) = (lista("added"), lista("removed"), lista("retyped"));
        if !(added.is_empty() && removed.is_empty() && retyped.is_empty()) {
            let mut partes = Vec::new();
            for (k, v) in [
                ("removed", &removed),
                ("added", &added),
                ("retyped", &retyped),
            ] {
                if !v.is_empty() {
                    partes.push(format!("{k} {}", v.join(", ")));
                }
            }
            let mensaje = format!(
                "the source no longer matches the declared table: {}",
                partes.join("; ")
            );
            let mut m = motivo(
                "source-schema-changed",
                Some(campo(e, "visto").as_deref().unwrap_or(&hoy)),
                Some(&mensaje),
            );
            if let Json::Obj(o) = &mut m {
                let mut c = Vec::new();
                for (k, v) in [("added", added), ("removed", removed), ("retyped", retyped)] {
                    if !v.is_empty() {
                        c.push((k, Json::Arr(v.into_iter().map(Json::s).collect())));
                    }
                }
                o.insert("columns".into(), Json::obj(c));
            }
            motivos.push((DEGRADED, m));
        }
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

// ── 0063 S6a · El linaje (v1alpha30 §2.2) ───────────────────────────────────
//
//   Hasta S6 sólo tenía salud lo que tiene puntero. Lo derivado —una vista que
//   no se copia, lo que se escribe desde algo— callaba aunque leyera de algo
//   degradado, y una tabla en vivo no decía lo que sus copias ya habían visto.
//   Aquí se junta, al servir, con el grafo del índice (`sale_de`).

/// Lo que [`propagar`] sabe de un activo.
#[derive(Debug, Clone, Default)]
pub struct Nodo {
    /// Su propia medida: el [`bloque`] del puntero de una copia o de una
    /// colección, o lo que sus copias vieron de una tabla en vivo
    /// ([`en_vivo`]). `None`: no la tiene (una vista sin copia, una tabla en
    /// vivo de la que nada se sabe).
    pub propio: Option<Json>,
    /// `propio` es lo que las copias vieron de esta tabla en vivo. No vuelve a
    /// bajar a las copias: lo vieron ellas, y ya lo dicen.
    pub de_las_copias: bool,
    /// De qué sale, por su ref del índice (`kind:qname`).
    pub entradas: Vec<String>,
    /// La `freshness` de una vista sin copia: se juzga con el más viejo de lo
    /// que lee.
    pub promesa: Option<String>,
}

/// Lo que una entrada le pasa a quien lee de ella.
#[derive(Debug, Clone)]
enum Entrada {
    /// La ref no es un activo de este índice (`rota`, o de otra celda).
    NoEsta,
    /// En vivo y sin nada sabido, o un ciclo: no aporta ni quita.
    EnVivo,
    Con {
        bloque: Json,
        /// Las raíces degradadas de las que viene (ref → desde cuándo), si
        /// ella no lo está: un activo con una raíz degradada lo dice.
        raices: BTreeMap<String, String>,
        de_las_copias: bool,
    },
}

use std::collections::{BTreeMap, BTreeSet};

/// Cuánto empuja cada motivo (§6): el mismo orden que [`bloque`].
fn rango_de_motivo(code: &str) -> u8 {
    match code {
        "freshness-exceeded"
        | "refresh-failed"
        | "refresh-held"
        | "source-schema-changed"
        | "source-unreachable" => 3,
        "upstream-degraded" | "behind-source" | "guard-warning" => 2,
        _ => 1,
    }
}

fn nombre_de_estado(rango: u8) -> &'static str {
    match rango {
        3 => "degraded",
        2 => "warning",
        1 => "unknown",
        _ => "healthy",
    }
}

fn texto<'a>(b: &'a Json, k: &str) -> Option<&'a str> {
    match b {
        Json::Obj(m) => match m.get(k) {
            Some(Json::Str(s)) => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn motivos_de(b: &Json) -> Vec<Json> {
    match b {
        Json::Obj(m) => match m.get("reasons") {
            Some(Json::Arr(a)) => a.clone(),
            _ => vec![],
        },
        _ => vec![],
    }
}

/// El nombre cualificado de una ref del índice: `asset` lo pide así (§6).
fn cualificado(r: &str) -> &str {
    r.split_once(':').map(|(_, q)| q).unwrap_or(r)
}

/// Desde cuándo está un bloque como está: el `since` más viejo de sus motivos.
fn desde(b: &Json) -> Option<String> {
    motivos_de(b)
        .iter()
        .filter_map(|m| texto(m, "since").map(String::from))
        .filter(|s| !s.is_empty())
        .min()
}

fn aguas_arriba(raiz: &str, desde: &str) -> Json {
    let q = cualificado(raiz);
    Json::obj([
        ("code", Json::s("upstream-degraded")),
        ("since", Json::s(desde)),
        ("asset", Json::s(q)),
        (
            "message",
            Json::s(format!("reads `{q}`, which is degraded")),
        ),
    ])
}

/// **Lo que las copias vieron de una tabla en vivo** (0063 D-S6-1). Una tabla
/// que se lee en vivo no tiene *data as of* hasta que alguien la lee, y en el
/// catálogo va sin estado («Live»). Pero si una copia suya vio que su esquema
/// cambió (S5a) o que el origen no contesta (S6e), la tabla lo dice: es lo
/// mismo que encontrará quien la lea. De cada motivo, el visto más tarde.
pub fn en_vivo(motivos_de_sus_copias: &[Json], capas: (bool, bool)) -> Option<Json> {
    let mut por_codigo: BTreeMap<String, Json> = BTreeMap::new();
    for m in motivos_de_sus_copias {
        let Some(code) = texto(m, "code") else {
            continue;
        };
        if !matches!(code, "source-schema-changed" | "source-unreachable") {
            continue;
        }
        let mas_tarde = por_codigo
            .get(code)
            .is_none_or(|v| texto(v, "since") < texto(m, "since"));
        if mas_tarde {
            por_codigo.insert(code.to_string(), m.clone());
        }
    }
    if por_codigo.is_empty() {
        return None;
    }
    Some(armar(
        "degraded",
        None,
        None,
        None,
        None,
        None,
        por_codigo.into_values().collect(),
        capas,
    ))
}

/// El bloque de §6 de algo sin puntero propio: sin `lastRefresh`, y nunca
/// sirviendo «lo último bueno» (no tiene versión que retener).
#[allow(clippy::too_many_arguments)]
fn armar(
    state: &str,
    datos_de: Option<String>,
    comprobado: Option<String>,
    retraso: Option<i64>,
    edad: Option<i64>,
    promesa: Option<&str>,
    motivos: Vec<Json>,
    capas: (bool, bool),
) -> Json {
    let mut freshness = vec![
        ("lagSeconds", retraso.map(Json::Int).unwrap_or_else(nulo)),
        ("ageSeconds", edad.map(Json::Int).unwrap_or_else(nulo)),
    ];
    if let Some(p) = promesa {
        freshness.push(("promise", Json::s(p)));
    }
    Json::obj([
        ("state", Json::s(state)),
        ("dataAsOf", datos_de.map(Json::s).unwrap_or_else(nulo)),
        ("checkedAt", comprobado.map(Json::s).unwrap_or_else(nulo)),
        ("sourceChangedAt", nulo()),
        ("freshness", Json::obj(freshness)),
        ("servingLastGood", Json::Bool(false)),
        ("reasons", Json::Arr(motivos)),
        (
            "layers",
            Json::obj([
                ("monitoring", Json::Bool(capas.0)),
                ("profiling", Json::Bool(capas.1)),
            ]),
        ),
    ])
}

/// **La salud por el linaje** (v1alpha30 §2.2), de todo el índice a la vez.
///
/// - Quien tiene medida propia la conserva, y si alguna raíz de la que sale
///   está `degraded`, suma `upstream-degraded` con su `asset` y queda, al
///   menos, en `warning`.
/// - Quien no la tiene (una vista sin copia) la toma de lo que lee: su
///   `dataAsOf` es el más viejo de sus entradas, y su `checkedAt`, la
///   comprobación más vieja si todas la tienen. Una entrada `unknown` la deja
///   `unknown` (`unverified`, nombrándola), igual que una que este índice no
///   tiene. Lo que se lee en vivo no envejece nada.
/// - Quien no tiene medida y sólo lee en vivo, sin nada sabido, **no tiene
///   bloque**: es «Live» (0063 D-S6-1), ni `unknown` ni `healthy`.
///
/// Devuelve el bloque de cada ref que lo tiene. Un ciclo no aporta nada.
pub fn propagar(
    nodos: &BTreeMap<String, Nodo>,
    ahora: i64,
    capas: (bool, bool),
) -> BTreeMap<String, Json> {
    let mut memo: BTreeMap<String, Entrada> = BTreeMap::new();
    for r in nodos.keys() {
        visitar(r, nodos, ahora, capas, &mut memo, &mut BTreeSet::new());
    }
    memo.into_iter()
        .filter_map(|(r, e)| match e {
            Entrada::Con { bloque, .. } => Some((r, bloque)),
            _ => None,
        })
        .collect()
}

fn visitar(
    r: &str,
    nodos: &BTreeMap<String, Nodo>,
    ahora: i64,
    capas: (bool, bool),
    memo: &mut BTreeMap<String, Entrada>,
    pila: &mut BTreeSet<String>,
) -> Entrada {
    if let Some(e) = memo.get(r) {
        return e.clone();
    }
    let Some(n) = nodos.get(r) else {
        return Entrada::NoEsta;
    };
    if !pila.insert(r.to_string()) {
        return Entrada::EnVivo;
    }
    let entradas: Vec<(String, Entrada)> = n
        .entradas
        .iter()
        .filter(|e| e.as_str() != r)
        .map(|e| (e.clone(), visitar(e, nodos, ahora, capas, memo, pila)))
        .collect();
    pila.remove(r);
    let e = calcular(n, &entradas, ahora, capas);
    memo.insert(r.to_string(), e.clone());
    e
}

fn calcular(n: &Nodo, entradas: &[(String, Entrada)], ahora: i64, capas: (bool, bool)) -> Entrada {
    // Las raíces degradadas de las que se sale. Lo que las copias vieron de una
    // tabla en vivo no vuelve a una copia: ella lo vio, y ya lo dice.
    let mut raices: BTreeMap<String, String> = BTreeMap::new();
    for (er, e) in entradas {
        let Entrada::Con {
            bloque,
            raices: suyas,
            de_las_copias,
        } = e
        else {
            continue;
        };
        if n.propio.is_some() && !n.de_las_copias && *de_las_copias {
            continue;
        }
        if estado(bloque) == "degraded" {
            let d = desde(bloque).unwrap_or_else(|| crate::frescura::iso(ahora));
            raices.insert(er.clone(), d);
        } else {
            raices.extend(suyas.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }

    // ── Con medida propia: la suya, y lo que viene de arriba ────────────────
    if let Some(b) = &n.propio {
        let mut b = b.clone();
        if !raices.is_empty()
            && let Json::Obj(m) = &mut b
        {
            let mut motivos = match m.remove("reasons") {
                Some(Json::Arr(a)) => a,
                _ => vec![],
            };
            motivos.extend(raices.iter().map(|(r, d)| aguas_arriba(r, d)));
            let peor = motivos
                .iter()
                .filter_map(|x| texto(x, "code"))
                .map(rango_de_motivo)
                .max()
                .unwrap_or(0);
            m.insert("state".into(), Json::s(nombre_de_estado(peor)));
            m.insert("reasons".into(), Json::Arr(motivos));
        }
        return Entrada::Con {
            bloque: b,
            raices,
            de_las_copias: n.de_las_copias,
        };
    }

    // ── Sin medida propia: la de lo que lee ─────────────────────────────────
    let hoy = crate::frescura::iso(ahora);
    let mut motivos: Vec<Json> = Vec::new();
    // Lo que tiene hora: lo copiado. Lo que se lee en vivo no envejece nada.
    let mut con_hora: Vec<(&String, &Json)> = Vec::new();
    let mut alguna = false;
    for (er, e) in entradas {
        match e {
            Entrada::NoEsta => {
                alguna = true;
                let q = cualificado(er);
                motivos.push(motivo(
                    "unverified",
                    Some(&hoy),
                    Some(&format!("reads `{q}`, which this catalog does not have")),
                ));
            }
            Entrada::Con {
                bloque,
                de_las_copias,
                ..
            } => {
                alguna = true;
                if !de_las_copias {
                    con_hora.push((er, bloque));
                }
            }
            Entrada::EnVivo => {}
        }
    }
    if !alguna {
        return Entrada::EnVivo;
    }
    for (er, b) in &con_hora {
        if estado(b) == "unknown" {
            let q = cualificado(er);
            motivos.push(motivo(
                "unverified",
                Some(texto(b, "dataAsOf").unwrap_or(&hoy)),
                Some(&format!("reads `{q}`, whose freshness is not known")),
            ));
        }
    }
    let horas: Option<Vec<&str>> = con_hora.iter().map(|(_, b)| texto(b, "dataAsOf")).collect();
    let datos_de = horas
        .filter(|h| !h.is_empty())
        .and_then(|h| h.into_iter().min_by_key(|t| epoca(t)).map(String::from));
    let comprobaciones: Option<Vec<&str>> = con_hora
        .iter()
        .map(|(_, b)| texto(b, "checkedAt"))
        .collect();
    let comprobado = comprobaciones
        .filter(|h| !h.is_empty())
        .and_then(|h| h.into_iter().min_by_key(|t| epoca(t)).map(String::from));
    let t_datos = datos_de.as_deref().and_then(epoca);
    let t_comprobado = comprobado.as_deref().and_then(epoca);
    let retraso = t_datos.map(|d| (ahora - d).max(0));
    let edad = t_comprobado.map(|c| (ahora - c).max(0));
    if let (Some(l), Some(e)) = (n.promesa.as_deref().and_then(duracion), edad.or(retraso))
        && e > l
    {
        motivos.push(motivo(
            "freshness-exceeded",
            Some(&crate::frescura::iso(
                t_comprobado.or(t_datos).unwrap_or(ahora) + l,
            )),
            None,
        ));
    }
    motivos.extend(raices.iter().map(|(r, d)| aguas_arriba(r, d)));
    let peor = motivos
        .iter()
        .filter_map(|x| texto(x, "code"))
        .map(rango_de_motivo)
        .max()
        .unwrap_or(0);
    let bloque = armar(
        nombre_de_estado(peor),
        datos_de,
        comprobado,
        retraso,
        edad,
        n.promesa.as_deref(),
        motivos,
        capas,
    );
    Entrada::Con {
        bloque,
        raices,
        de_las_copias: false,
    }
}

/// **La suma de un ámbito** (una base, un schema, el árbol): cuántos en cada
/// estado, con lo desconocido aparte (§3.3: nunca como sano). Y
/// `needsAttention` (0063 D-S6-2): lo `degraded`, lo `warning` y lo que nunca
/// se copió —lo único `unknown` que pide que alguien haga algo—. Lo demás
/// `unknown` es lo que no se puede comprobar: va aparte, como *Unverified*.
pub fn sumar<'a>(bloques: impl IntoIterator<Item = &'a Json>) -> Json {
    let (mut h, mut w, mut d, mut u, mut atencion) = (0i64, 0i64, 0i64, 0i64, 0i64);
    for b in bloques {
        match estado(b) {
            "healthy" => h += 1,
            "warning" => {
                w += 1;
                atencion += 1;
            }
            "degraded" => {
                d += 1;
                atencion += 1;
            }
            _ => {
                u += 1;
                if motivos_de(b)
                    .iter()
                    .any(|m| texto(m, "code") == Some("never-refreshed"))
                {
                    atencion += 1;
                }
            }
        }
    }
    Json::obj([
        ("healthy", Json::Int(h)),
        ("warning", Json::Int(w)),
        ("degraded", Json::Int(d)),
        ("unknown", Json::Int(u)),
        ("needsAttention", Json::Int(atencion)),
    ])
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
    fn el_esquema_del_origen_es_un_motivo_con_sus_columnas() {
        let n = p(
            r#"{"estado":"error","motivo":"falta `cliente`","metadata_location":"m",
            "datos_de":"2026-10-13T15:00:00Z",
            "intento":{"cuando":"2026-10-13T16:00:00Z","resultado":"error"},
            "esquema_origen":{"added":["comprador"],"removed":["cliente"],"retyped":[],"visto":"2026-10-13T16:00:00Z"}}"#,
        );
        let b = bloque(Some(&n), None, 1_791_900_000, (false, false));
        assert_eq!(estado(&b), "degraded");
        let t = b.jcs();
        assert!(t.contains(r#""code":"source-schema-changed""#), "{t}");
        assert!(
            t.contains(r#""columns":{"added":["comprador"],"removed":["cliente"]}"#),
            "{t}"
        );
        assert!(t.contains("removed cliente; added comprador"), "{t}");
    }

    #[test]
    fn retenida_dice_su_guarda() {
        let n = p(
            r#"{"estado":"error","motivo":"0 rows","metadata_location":"m",
            "intento":{"cuando":"2026-10-13T16:00:00Z","resultado":"retenida","guarda":"zero-rows"}}"#,
        );
        let t = bloque(Some(&n), None, 1_791_900_000, (false, false)).jcs();
        assert!(
            t.contains(r#""code":"refresh-held""#) && t.contains(r#""guard":"zero-rows""#),
            "{t}"
        );
    }

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

    // ── S6a · el linaje ──────────────────────────────────────────────────────

    fn copia(texto: &str) -> Nodo {
        Nodo {
            propio: Some(bloque(Some(&p(texto)), None, ahora(), (false, false))),
            ..Nodo::default()
        }
    }
    fn lee(de: &[&str]) -> Nodo {
        Nodo {
            entradas: de.iter().map(|s| s.to_string()).collect(),
            ..Nodo::default()
        }
    }
    const SANA: &str = r#"{"estado":"al-dia","metadata_location":"m","datos_de":"2026-10-13T15:00:00Z",
        "comprobado":"2026-10-13T15:50:00Z","intento":{"cuando":"2026-10-13T15:50:00Z","resultado":"sin-cambios"}}"#;
    const ROTA: &str = r#"{"estado":"error","metadata_location":"m","datos_de":"2026-10-13T12:00:00Z",
        "motivo":"falta `cliente`","intento":{"cuando":"2026-10-13T13:44:51Z","resultado":"error"}}"#;

    /// §2.2: lo que lee de una copia degradada está, al menos, en `warning`,
    /// con `upstream-degraded` y el nombre de esa copia. Y su hora es la más
    /// vieja de lo que lee.
    #[test]
    fn una_vista_sobre_una_copia_degradada_lo_dice() {
        let mut n = BTreeMap::new();
        n.insert("dataset:b.s.sana".to_string(), copia(SANA));
        n.insert("dataset:b.s.rota".to_string(), copia(ROTA));
        n.insert(
            "view:b.s.junta".to_string(),
            lee(&["dataset:b.s.sana", "dataset:b.s.rota"]),
        );
        let s = propagar(&n, ahora(), (false, false));
        let v = &s["view:b.s.junta"];
        assert_eq!(estado(v), "warning");
        let t = v.jcs();
        assert!(
            t.contains(r#""asset":"b.s.rota","code":"upstream-degraded""#),
            "{t}"
        );
        assert!(t.contains(r#""dataAsOf":"2026-10-13T12:00:00Z""#), "{t}");
        // Ni lo último bueno ni un refresco: no tiene puntero.
        assert!(t.contains(r#""servingLastGood":false"#) && !t.contains("lastRefresh"));
    }

    /// Una raíz degradada se dice a dos pasos: la vista de una vista también.
    /// Y quien tiene medida propia la conserva y suma la de arriba.
    #[test]
    fn la_raiz_degradada_baja_por_todo_el_camino() {
        let mut n = BTreeMap::new();
        n.insert("dataset:b.s.rota".to_string(), copia(ROTA));
        n.insert("view:b.s.uno".to_string(), lee(&["dataset:b.s.rota"]));
        n.insert("view:b.s.dos".to_string(), lee(&["view:b.s.uno"]));
        let mut escrito = copia(SANA);
        escrito.entradas = vec!["view:b.s.dos".into()];
        n.insert("dataset:b.s.escrito".to_string(), escrito);
        let s = propagar(&n, ahora(), (false, false));
        for r in ["view:b.s.dos", "dataset:b.s.escrito"] {
            assert_eq!(estado(&s[r]), "warning", "{r}");
            assert!(s[r].jcs().contains(r#""asset":"b.s.rota""#), "{r}");
        }
        // La medida propia del escrito sigue ahí.
        assert!(
            s["dataset:b.s.escrito"]
                .jcs()
                .contains("\"checkedAt\":\"2026-10-13T15:50:00Z\"")
        );
    }

    /// Sobre copias sanas, una vista sin copia está sana: sus entradas se
    /// comprobaron. Sobre una sin comprobar, `unknown`, nombrándola.
    #[test]
    fn una_vista_es_tan_sabida_como_lo_que_lee() {
        let mut n = BTreeMap::new();
        n.insert("dataset:b.s.sana".to_string(), copia(SANA));
        n.insert(
            "dataset:b.s.vieja".to_string(),
            copia(
                r#"{"estado":"al-dia","metadata_location":"m","datos_de":"2026-10-02T00:00:00Z"}"#,
            ),
        );
        n.insert("view:b.s.a".to_string(), lee(&["dataset:b.s.sana"]));
        n.insert("view:b.s.b".to_string(), lee(&["dataset:b.s.vieja"]));
        n.insert("view:b.s.c".to_string(), lee(&["dataset:otra.celda"]));
        let s = propagar(&n, ahora(), (false, false));
        assert_eq!(estado(&s["view:b.s.a"]), "healthy");
        assert!(s["view:b.s.a"].jcs().contains(r#""ageSeconds":600"#));
        assert_eq!(estado(&s["view:b.s.b"]), "unknown");
        assert!(
            s["view:b.s.b"]
                .jcs()
                .contains("reads `b.s.vieja`, whose freshness is not known")
        );
        assert_eq!(estado(&s["view:b.s.c"]), "unknown");
        assert!(
            s["view:b.s.c"]
                .jcs()
                .contains("which this catalog does not have")
        );
    }

    /// 0063 D-S6-1: lo que se lee en vivo, sin nada sabido, no tiene bloque.
    /// Con lo que vio una copia suya, sí: y la vista en vivo encima lo hereda,
    /// pero la copia no lo recibe de vuelta.
    #[test]
    fn lo_en_vivo_va_sin_estado_salvo_lo_que_vieron_sus_copias() {
        let mut n = BTreeMap::new();
        n.insert("table:f.s.t".to_string(), Nodo::default());
        n.insert("view:foranea.s.t".to_string(), lee(&["table:f.s.t"]));
        let s = propagar(&n, ahora(), (false, false));
        assert!(!s.contains_key("table:f.s.t") && !s.contains_key("view:foranea.s.t"));

        let deriva = r#"{"estado":"error","metadata_location":"m","datos_de":"2026-10-13T12:00:00Z",
            "intento":{"cuando":"2026-10-13T13:00:00Z","resultado":"error"},
            "esquema_origen":{"added":["comprador"],"removed":["cliente"],"visto":"2026-10-13T13:00:00Z"}}"#;
        let mut c = copia(deriva);
        c.entradas = vec!["table:f.s.t".into()];
        let vistos = motivos_de(c.propio.as_ref().unwrap());
        n.insert("dataset:std.s.t".to_string(), c);
        n.insert(
            "table:f.s.t".to_string(),
            Nodo {
                propio: en_vivo(&vistos, (false, false)),
                de_las_copias: true,
                ..Nodo::default()
            },
        );
        let s = propagar(&n, ahora(), (false, false));
        let t = &s["table:f.s.t"];
        assert_eq!(estado(t), "degraded");
        assert_eq!(motivos(t), ["source-schema-changed"]);
        assert!(t.jcs().contains(r#""dataAsOf":null"#));
        assert_eq!(estado(&s["view:foranea.s.t"]), "warning");
        assert!(!s["dataset:std.s.t"].jcs().contains("upstream-degraded"));
    }

    #[test]
    fn la_suma_cuenta_lo_desconocido_aparte() {
        let nunca = bloque(None, None, ahora(), (false, false));
        let sin_comprobar = bloque(
            Some(&p(r#"{"estado":"al-dia","metadata_location":"m"}"#)),
            None,
            ahora(),
            (false, false),
        );
        let rota = bloque(Some(&p(ROTA)), None, ahora(), (false, false));
        let sana = bloque(Some(&p(SANA)), None, ahora(), (false, false));
        let t = sumar([&nunca, &sin_comprobar, &rota, &sana]).jcs();
        assert_eq!(
            t,
            r#"{"degraded":1,"healthy":1,"needsAttention":2,"unknown":2,"warning":0}"#
        );
    }

    #[test]
    fn un_ciclo_no_cuelga() {
        let mut n = BTreeMap::new();
        n.insert("view:a".to_string(), lee(&["view:b"]));
        n.insert("view:b".to_string(), lee(&["view:a"]));
        let _ = propagar(&n, ahora(), (false, false));
    }

    #[test]
    fn sin_puntero_nunca_se_refresco() {
        let b = bloque(None, None, ahora(), (true, false));
        assert_eq!(estado(&b), "unknown");
        assert_eq!(motivos(&b), ["never-refreshed"]);
        assert!(b.pretty().contains("\"monitoring\": true"));
    }
}
