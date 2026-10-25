//! `OOS2055` — **una coordenada, un artefacto** (v1alpha34 `01-artifact` §4).
//!
//! La coordenada de un `Artifact` es lo que Maven, pip o uv piden; la
//! plataforma tiene que saber a qué documento corresponde para decidir quién
//! puede leerlo y de dónde se sirve. Dos con la misma serían dos respuestas a
//! la misma petición, y ganaría la que se leyera antes. Que se llamen distinto
//! en el catálogo no lo arregla: el nombre es de la plataforma, la coordenada
//! del ecosistema. Se compara como la comparan las herramientas: tal cual en
//! Maven, normalizada en PyPI (PEP 503).
//!
//! Va junto a la identidad, antes del enlazado, por lo mismo que `OOS2035`:
//! es una identidad repetida, y lo que la nombre resolvería mal.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{Kind, coordenada_normalizada};
use crate::link::Package;
use std::collections::BTreeMap;

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    // (format, coordenada normalizada) → los artefactos que la declaran
    let mut por_coordenada: BTreeMap<(String, String), Vec<&crate::link::Loaded>> = BTreeMap::new();
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Artifact) {
        let texto = |k: &str| d.section(k).and_then(|n| n.as_str()).map(str::to_string);
        // Sin formato o sin coordenada ya es `OOS1004`, en su documento.
        let (Some(format), Some(coordenada)) = (texto("format"), texto("coordinate")) else {
            continue;
        };
        let clave = coordenada_normalizada(&format, &coordenada);
        por_coordenada.entry((format, clave)).or_default().push(d);
    }
    let mut out = Vec::new();
    for ((format, coordenada), docs) in por_coordenada {
        if docs.len() < 2 {
            continue;
        }
        let quienes: Vec<String> = docs.iter().filter_map(|d| d.qname()).collect();
        for d in &docs {
            out.push(
                Diagnostic::new(
                    Code::Oos2055,
                    &d.path,
                    format!(
                        "la coordenada `{coordenada}` de `{format}` la declaran {} artefactos: {}",
                        docs.len(),
                        quienes.join(" · ")
                    ),
                )
                .at(d.root.pos())
                .help(
                    "la coordenada es lo que la herramienta pide, y tiene que llevar a un solo \
                     artefacto para saber quién puede leerlo y de dónde se sirve. Deja uno: las \
                     versiones de los dos se publican en el mismo",
                ),
            );
        }
    }
    out
}
