//! Un valor JSON y su escritura.
//!
//! No hay `serde` aquí, y no es ascetismo: la salida de `ore` es un **artefacto
//! normativo**, no la serialización incidental de unas estructuras internas. El
//! día que `digest/` exija RFC 8785 (`90-canonical-form`), la ordenación de
//! claves y la serialización de números tendrán que ser exactamente las que
//! dice el estándar, no las que decida una librería por defecto.
//!
//! Escribirlo aquí cuesta ochenta líneas y deja ese punto bajo control.

use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Str(String),
    Int(i64),
    Bool(bool),
    Arr(Vec<Json>),
    /// `BTreeMap`: las claves salen ordenadas siempre. Es gratis aquí y es
    /// obligatorio en la forma canónica.
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn obj(pares: impl IntoIterator<Item = (&'static str, Json)>) -> Json {
        Json::Obj(pares.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn s(v: impl Into<String>) -> Json {
        Json::Str(v.into())
    }

    /// JSON indentado, para que lo lea una persona en una revisión.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, nivel: usize) {
        let sangria = |n: usize| "  ".repeat(n);
        match self {
            Json::Str(s) => escribir_cadena(out, s),
            Json::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Arr(items) => {
                out.push_str("[\n");
                for (i, v) in items.iter().enumerate() {
                    out.push_str(&sangria(nivel + 1));
                    v.write(out, nivel + 1);
                    out.push_str(if i + 1 == items.len() { "\n" } else { ",\n" });
                }
                out.push_str(&sangria(nivel));
                out.push(']');
            }
            Json::Obj(m) if m.is_empty() => out.push_str("{}"),
            Json::Obj(m) => {
                out.push_str("{\n");
                for (i, (k, v)) in m.iter().enumerate() {
                    out.push_str(&sangria(nivel + 1));
                    escribir_cadena(out, k);
                    out.push_str(": ");
                    v.write(out, nivel + 1);
                    out.push_str(if i + 1 == m.len() { "\n" } else { ",\n" });
                }
                out.push_str(&sangria(nivel));
                out.push('}');
            }
        }
    }
}

fn escribir_cadena(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordena_las_claves() {
        let j = Json::obj([("zeta", Json::Int(1)), ("alfa", Json::Int(2))]);
        assert!(j.pretty().find("alfa").unwrap() < j.pretty().find("zeta").unwrap());
    }

    #[test]
    fn escapa_lo_que_debe() {
        assert_eq!(Json::s("a\"b\\c\n").pretty(), r#""a\"b\\c\n""#);
    }
}
