//! Forma canónica — la familia `OOS6xxx`.
//!
//! Un solo código, y es el que justifica que `parse.rs` conserve el **estilo**
//! de cada escalar en lugar de deserializar. Sin esa decisión, tomada en el
//! primer commit del núcleo, esta comprobación sería imposible: el dato que hace
//! falta —si el autor escribió comillas— lo tira cualquier deserializador antes
//! de que nadie pueda mirarlo.
//!
//! # Por qué un decimal sin comillas es un error
//!
//! `68400.50` como número JSON se representa en coma flotante binaria de doble
//! precisión. La serialización canónica de RFC 8785 fija el algoritmo de salida,
//! pero **no puede recuperar los dígitos que el parseo ya perdió**. El cero
//! final se va, y con él la escala que `Money<EUR, 2>` declara.
//!
//! La consecuencia cae directamente sobre **G1**: dos implementaciones con
//! parseadores YAML distintos producirían bytes distintos para el mismo
//! documento, y con ellos digests distintos. La identidad determinista se caería
//! por un céntimo.
//!
//! Un decimal entrecomillado no tiene ese problema: es una cadena, y una cadena
//! sobrevive intacta a cualquier parser.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::parse::{Node, Style};
use std::path::Path;

/// ¿Es este texto un decimal con parte fraccionaria?
///
/// Deliberadamente **no** se pregunta si el valor es *inexacto* en binario. Esa
/// regla sería más permisiva y peor: obligaría a cada implementación a coincidir
/// exactamente en qué decimales pierden precisión, que es justo la divergencia
/// que esta comprobación existe para impedir. Y dejaría pasar `68400.50`, cuyo
/// valor es exacto y cuyo cero final se pierde igual.
///
/// La regla uniforme es que **un decimal escrito sin comillas no tiene forma
/// canónica**: lo que sobreviva depende del parser. El arreglo es una comilla.
fn es_decimal(raw: &str) -> bool {
    let cuerpo = raw.strip_prefix(['-', '+']).unwrap_or(raw);
    // Un exponente no cambia el argumento, pero lo separa del punto.
    let mantisa = cuerpo.split_once(['e', 'E']).map_or(cuerpo, |(m, _)| m);
    let Some((ent, frac)) = mantisa.split_once('.') else {
        return false;
    };
    !frac.is_empty()
        && !ent.is_empty()
        && ent.bytes().all(|b| b.is_ascii_digit())
        && frac.bytes().all(|b| b.is_ascii_digit())
}

/// Recorre los **valores** del documento. Las claves quedan fuera: ahí no viven
/// importes, y una clave numérica sería otro problema distinto.
fn recorrer(n: &Node, ruta: &str, out: &mut Vec<(String, String, crate::diag::Pos)>) {
    match n {
        Node::Scalar { raw, style, pos } => {
            if *style == Style::Plain && es_decimal(raw) {
                out.push((ruta.to_string(), raw.clone(), *pos));
            }
        }
        Node::Mapping { entries, .. } => {
            for (k, v) in entries {
                let clave = k.as_str().unwrap_or("?");
                let hijo = if ruta.is_empty() {
                    clave.to_string()
                } else {
                    format!("{ruta}.{clave}")
                };
                recorrer(v, &hijo, out);
            }
        }
        Node::Sequence { items, .. } => {
            for (i, v) in items.iter().enumerate() {
                recorrer(v, &format!("{ruta}[{i}]"), out);
            }
        }
    }
}

/// El sujeto que un humano reconoce. Dentro de una entidad, una ruta que pasa
/// por `spec.properties.<nombre>` **es** una propiedad cualificada; decir
/// `hr.Employee.baseSalary` en lugar de `spec.properties.baseSalary.examples`
/// es la diferencia entre nombrar la cosa y describir dónde estaba.
fn sujeto(root: &Node, kind: Kind, ruta: &str) -> String {
    let dentro_de_propiedad = || -> Option<String> {
        (kind == Kind::Entity).then_some(())?;
        let nombre = ruta.strip_prefix("spec.properties.")?.split('.').next()?;
        let m = root.get("metadata")?.1;
        let n = m.get("name")?.1.as_str()?;
        Some(match m.get("namespace").and_then(|(_, v)| v.as_str()) {
            Some(ns) => format!("{ns}.{n}.{nombre}"),
            None => format!("{n}.{nombre}"),
        })
    };
    dentro_de_propiedad().unwrap_or_else(|| ruta.to_string())
}

pub fn check(file: &Path, root: &Node, kind: Kind) -> Vec<Diagnostic> {
    let mut hallazgos = Vec::new();
    recorrer(root, "", &mut hallazgos);

    hallazgos
        .into_iter()
        .map(|(ruta, raw, pos)| {
            Diagnostic::new(
                Code::Oos6003,
                file,
                format!(
                    "`{}`: el decimal `{raw}` va sin comillas",
                    sujeto(root, kind, &ruta)
                ),
            )
            .at(pos)
            .help(format!(
                "escríbelo como cadena: `\"{raw}\"`. Sin comillas es un número \
                 JSON, y un número JSON viaja en coma flotante binaria: lo que \
                 sobreviva a la ida y vuelta depende del parser. RFC 8785 fija \
                 cómo se serializa el resultado, pero no puede devolver los \
                 dígitos que el parseo ya perdió"
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconoce_decimales_y_deja_pasar_lo_demas() {
        assert!(es_decimal("68400.50"));
        assert!(es_decimal("-0.1"));
        assert!(es_decimal("1.0e3"));
        // Enteros: exactos hasta 2^53, y sin dígitos que perder.
        assert!(!es_decimal("2"));
        assert!(!es_decimal("-17"));
        // Semver, direcciones, identificadores: no son números.
        assert!(!es_decimal("1.0.0"));
        assert!(!es_decimal("v1.2"));
        assert!(!es_decimal("90d"));
        assert!(!es_decimal(".5"));
    }

    #[test]
    fn solo_dispara_sin_comillas() {
        let root = crate::parse::parse("a: 68400.50\nb: \"68400.50\"\n").unwrap();
        let d = check(Path::new("x.yaml"), &root, Kind::Entity);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("68400.50"));
    }

    #[test]
    fn nombra_la_propiedad_cualificada() {
        let y = "metadata:\n  name: Employee\n  namespace: hr\nspec:\n  properties:\n    \
                 baseSalary:\n      examples:\n        values: [68400.50]\n";
        let root = crate::parse::parse(y).unwrap();
        let d = check(Path::new("x.yaml"), &root, Kind::Entity);
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message.contains("hr.Employee.baseSalary"),
            "{}",
            d[0].message
        );
    }
}
