//! Propiedades derivadas — `expression` promovida de prosa a CEL.
//!
//! # Por qué esto es una fase y no una comprobación de esquema
//!
//! Que una propiedad con `expression` declare `derivedFrom` **sí** es de
//! esquema, y sale como `OOS1004`. Lo que no cabe en un esquema es la regla que
//! justifica que exista esta fase:
//!
//! > Toda propiedad que la expresión lee **DEBE** estar declarada en
//! > `derivedFrom` (`OOS4015`).
//!
//! Cruza dos campos de la misma propiedad, y ningún esquema puede mirar dos
//! campos a la vez.
//!
//! # Por qué la dirección de la comprobación es esa y no la contraria
//!
//! La pregunta obvia es por qué no derivar `derivedFrom` de la expresión, y
//! v1alpha1 ya la contestó en el propio esquema: *«un análisis de contaminación
//! sólido no puede depender de parsear cadenas de expresión»*. Así que
//! `derivedFrom` es normativo —es lo que propaga— y la expresión **se contrasta
//! contra él**.
//!
//! De ahí sale la propiedad que hace aceptable este análisis: **es conservador y
//! solo puede apretar**. Lo que encuentra tiene que estar declarado; lo que se
//! le escape no afloja nada, porque la propagación no depende de él. Un
//! analizador incompleto produce menos errores, nunca una etiqueta más baja —
//! que es la única dirección en la que fallar es aceptable.
//!
//! # Lo que esta fase no hace
//!
//! **No evalúa.** El compilador no calcula el valor de una propiedad derivada,
//! ni aquí ni después: exige datos, es L2, y rompería la pureza de la
//! compilación. Una propiedad derivada sigue necesitando binding.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::link::{Loaded, Package};
use crate::parse::Node;

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let props: Vec<&str> = e
            .section("properties")
            .map(|n| n.entries())
            .unwrap_or(&[])
            .iter()
            .filter_map(|(k, _)| k.as_str())
            .collect();

        for (k, v) in e.section("properties").map(|n| n.entries()).unwrap_or(&[]) {
            let Some(nombre) = k.as_str() else { continue };
            expresion(pkg, e, &qn, nombre, v, &props, &mut out);
        }
    }
    out
}

// ── `expression` · OOS1004 · OOS2005 · OOS4015 ──────────────────────────────

fn expresion(
    pkg: &Package,
    e: &Loaded,
    qn: &str,
    nombre: &str,
    def: &Node,
    props: &[&str],
    out: &mut Vec<Diagnostic>,
) {
    let Some((_, nodo)) = def.get("expression") else {
        return;
    };
    let Some(texto) = nodo.as_str() else { return };

    // Una computación sin procedencia declarada. Es de forma —el esquema lo
    // expresa— y por eso no lleva código propio.
    let Some((_, from)) = def.get("derivedFrom") else {
        out.push(
            Diagnostic::new(
                Code::Oos1004,
                &e.path,
                format!("`{qn}.{nombre}` declara `expression` y no `derivedFrom`"),
            )
            .at(nodo.pos())
            .help(
                "`derivedFrom` es lo que propaga las etiquetas; la expresión solo dice cómo. \
                 Una computación sin procedencia declarada deja la propiedad derivada sin \
                 clasificar, y eso no lo ve nadie",
            ),
        );
        return;
    };

    let declaradas: Vec<String> = from
        .items()
        .iter()
        .filter_map(|i| i.as_str().map(|s| crate::normalize::qualify(s, Some(qn))))
        .collect();

    for leida in lee(texto, props) {
        let q = format!("{qn}.{leida}");
        if declaradas.iter().any(|d| *d == q) {
            continue;
        }
        // Que la propiedad exista lo garantiza `props`: solo se extraen
        // identificadores que son propiedades de esta entidad. Una referencia a
        // algo inexistente no llega aquí — la ve `link` y es `OOS2005`.
        let _ = pkg;
        out.push(
            Diagnostic::new(
                Code::Oos4015,
                &e.path,
                format!("`{qn}.{nombre}` lee `{leida}` y no la declara en `derivedFrom`"),
            )
            .at(nodo.pos())
            .help(format!(
                "añade `{leida}` a `derivedFrom`. La etiqueta de una propiedad derivada se \
                 computa por `join` sobre lo que `derivedFrom` declara: si la expresión lee \
                 algo que no está ahí, la clasificación resultante queda POR DEBAJO de la que \
                 corresponde, y a partir de ese punto el dato fluye a sitios donde no debería. \
                 Es el fallo de `OOS4001` con la etiqueta escrita en la línea de al lado"
            )),
        );
    }
}

/// Qué propiedades de la entidad lee una expresión.
///
/// **No es un analizador de CEL, y no pretende serlo.** Extrae identificadores
/// y se queda con los que son propiedades de esta entidad. Un nombre de función
/// o una variable de otro ámbito no casan y se ignoran.
///
/// La incompletitud es aceptable en esta dirección y solo en esta: lo que se le
/// escape produce un error de menos, nunca una etiqueta más baja.
fn lee(expr: &str, props: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if !(c.is_ascii_alphabetic() || c == '_') {
            i += 1;
            continue;
        }
        let inicio = i;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if c.is_ascii_alphanumeric() || c == '_' {
                i += 1;
            } else {
                break;
            }
        }
        // Un identificador precedido de `.` es un campo de otra cosa, no una
        // propiedad de esta entidad.
        let cualificado = inicio > 0 && bytes[inicio - 1] == b'.';
        let ident = &expr[inicio..i];
        if !cualificado && props.contains(&ident) && !out.iter().any(|x| x == ident) {
            out.push(ident.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lo que la expresión lee, sin ser un analizador de CEL.
    #[test]
    fn extrae_las_propiedades_que_la_expresion_lee() {
        let props = ["baseSalary", "bonus", "department"];
        assert_eq!(lee("baseSalary + bonus", &props), ["baseSalary", "bonus"]);
        // Una función no es una propiedad, y un campo de otro ámbito tampoco.
        assert_eq!(lee("max(baseSalary, 0)", &props), ["baseSalary"]);
        assert!(lee("subject.department", &props).is_empty());
        // Sin repetidos: el mismo nombre dos veces es una lectura.
        assert_eq!(lee("bonus + bonus", &props), ["bonus"]);
    }

    /// La incompletitud tiene que caer siempre del lado seguro: lo que no se
    /// reconoce produce un error de menos, jamás una etiqueta más baja.
    #[test]
    fn lo_que_no_reconoce_no_afloja_nada() {
        let props = ["salary"];
        // `salario` no es una propiedad declarada: no se inventa una lectura.
        assert!(lee("salario * 2", &props).is_empty());
    }
}
