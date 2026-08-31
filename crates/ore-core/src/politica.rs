//! Las etiquetas que una política menciona tienen que existir.
//!
//! # El hueco, y su simetría
//!
//! `sync.rs` ya vigila una dirección: que el esquema Cedar comprometido conozca
//! **cada nivel que declara un retículo**, y su ayuda dice exactamente por qué —
//! *«una política que los mencione no fallará: dejará de casar con nada, y el
//! dato quedará sin gobernar en silencio»*.
//!
//! La dirección contraria no la vigilaba nadie. Una política que escribe
//! `Label::"gdpr.sensitivity:higth"` —con la errata— **valida limpia**, no casa
//! con nada, y el dato que iba a gobernar queda sin gobernar sin que salte nada:
//!
//! > **Una política que no gobierna tiene exactamente el mismo aspecto que una
//! > que gobierna.**
//!
//! El razonamiento estaba escrito y se aplicaba a una de las dos direcciones.
//!
//! # Por qué esto no necesita un evaluador
//!
//! `cedar.rs` ya lee las etiquetas que una política menciona —lo añadió v1alpha3
//! para responder *«¿hay una política sobre esta propiedad?»*—. Comprobar que
//! existen es comparar dos conjuntos de cadenas: ni se evalúa, ni se resuelven
//! jerarquías, ni se enlaza `cedar-policy`. El ADR 0003 sigue en pie.
//!
//! # Por qué el código es `OOS2005` y no uno nuevo
//!
//! Porque es exactamente eso: una **referencia que no resuelve**. El caso
//! `invalid/relation-target-not-found` ya dice que el mismo código cubre *«las
//! demás referencias a entidad o propiedad»*, y una etiqueta es una referencia a
//! un nivel de un retículo. Inflar la familia por simetría con una tabla es lo
//! contrario de lo que P7 pide.
//!
//! # Lo que NO es un error, y conviene decirlo
//!
//! Una política que menciona una etiqueta **declarada** y que hoy no lleva
//! ninguna propiedad **no es un defecto**: es la característica. `Property in
//! [Label, EntityType]` existe para que **una entidad nueva quede gobernada el
//! día que se etiqueta**, sin tocar la política. Marcar eso como error prohibiría
//! escribir la política antes que el dato, que es el orden correcto.
//!
//! Por eso lo que alcanza cada política se **informa**, no se diagnostica.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::flow;
use crate::link::Package;
use std::collections::{BTreeMap, BTreeSet};

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let declaradas = niveles(pkg);
    let mut out = Vec::new();

    for (ruta, texto) in &pkg.cedar {
        for p in crate::cedar::read(texto) {
            for etiqueta in &p.labels {
                if declaradas.contains(etiqueta) {
                    continue;
                }
                out.push(
                    Diagnostic::new(
                        Code::Oos2005,
                        ruta,
                        format!(
                            "`{}` menciona `{etiqueta}`, que ningún retículo declara",
                            p.id
                        ),
                    )
                    .help(if declaradas.is_empty() {
                        "el paquete no declara ningún retículo, así que no hay ninguna \
                         etiqueta que mencionar. Una política que apunta a una \
                         clasificación inexistente no falla: deja de casar con nada, y el \
                         dato queda sin gobernar en silencio"
                            .to_string()
                    } else {
                        format!(
                            "una política que apunta a una clasificación inexistente no \
                             falla: deja de casar con nada, y el dato queda sin gobernar en \
                             silencio. Declaradas: {}",
                            lista(&declaradas, etiqueta)
                        )
                    }),
                );
            }
        }
    }
    out
}

/// Todas las etiquetas que los retículos del paquete declaran, en la forma en
/// que la proyección a Cedar las emite: `<retículo>:<nivel>`.
fn niveles(pkg: &Package) -> BTreeSet<String> {
    flow::lattices(pkg)
        .values()
        .flat_map(|l| {
            l.levels
                .iter()
                .map(|n| format!("{}:{}", l.qname, n))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Las declaradas, empezando por las del mismo retículo que la errónea: si
/// alguien escribió `gdpr.sensitivity:higth`, lo que le sirve son los niveles de
/// `gdpr.sensitivity` y no los de un retículo que no ha tocado.
fn lista(declaradas: &BTreeSet<String>, fallida: &str) -> String {
    let familia = fallida.split_once(':').map(|(f, _)| f).unwrap_or("");
    let mut cerca: Vec<&str> = declaradas
        .iter()
        .filter(|d| d.starts_with(&format!("{familia}:")))
        .map(String::as_str)
        .collect();
    if cerca.is_empty() {
        cerca = declaradas.iter().map(String::as_str).collect();
    }
    cerca
        .iter()
        .map(|c| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(" · ")
}

// ── El acuse de recibo ──────────────────────────────────────────────────────

/// Qué propiedades alcanza cada política, hoy.
///
/// No es un diagnóstico: es lo que convierte escribir Cedar de un acto a ciegas
/// en uno con acuse de recibo. Quien acaba de etiquetar una propiedad quiere
/// ver que la política que escribió ayer ya la alcanza — y quien escribe una
/// política nueva quiere ver que alcanza lo que creía.
pub fn alcance(pkg: &Package) -> BTreeMap<String, Vec<String>> {
    // Las EFECTIVAS, no las declaradas: las heredadas de la entidad y las
    // computadas por propagación son las dos que nadie escribió, y por tanto las
    // que más falta hace ver gobernadas.
    let efectivas = flow::efectivas(pkg, &flow::lattices(pkg));
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (_, texto) in &pkg.cedar {
        for p in crate::cedar::read(texto) {
            let alcanzadas: Vec<String> = efectivas
                .iter()
                .filter(|(_, etiquetas)| {
                    // La comparación es EXACTA, y no por orden: `cedar_schema`
                    // emite `Label` sin jerarquía, así que en el esquema que
                    // nosotros generamos dos niveles no están relacionados. Una
                    // política sobre `high` no alcanza a un recurso `critical`,
                    // y decir que sí sería acreditar cobertura que Cedar no da.
                    etiquetas
                        .iter()
                        .any(|(ret, nivel)| p.labels.contains(&format!("{ret}:{nivel}")))
                })
                .map(|(prop, _)| prop.clone())
                .collect();
            out.insert(p.id.clone(), alcanzadas);
        }
    }
    out
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_sugerencia_se_queda_en_el_reticulo_de_la_errata() {
        let d: BTreeSet<String> = [
            "gdpr.sensitivity:low",
            "gdpr.sensitivity:high",
            "oos.maturity:DRAFT",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let s = lista(&d, "gdpr.sensitivity:higth");
        assert!(s.contains("gdpr.sensitivity:high"), "{s}");
        // Ofrecer niveles de un retículo que nadie tocó es ruido.
        assert!(!s.contains("oos.maturity"), "{s}");
    }

    /// Y si la errata no tiene retículo reconocible, se ofrece todo: es peor
    /// callar que decir de más.
    #[test]
    fn sin_familia_reconocible_se_ofrece_todo() {
        let d: BTreeSet<String> = ["a:1", "b:2"].iter().map(|s| s.to_string()).collect();
        let s = lista(&d, "loquesea");
        assert!(s.contains("a:1") && s.contains("b:2"), "{s}");
    }
}
