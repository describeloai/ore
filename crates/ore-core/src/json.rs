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

    /// **La forma canónica**: RFC 8785 (JCS).
    ///
    /// Sin espacios, claves ordenadas por sus unidades de código UTF-16, y
    /// escapes mínimos. De estos bytes sale el digest, así que aquí es donde
    /// vive **G1**: dos implementaciones conformes producen los mismos bytes o
    /// la identidad determinista no existe.
    ///
    /// Los números: RFC 8785 manda el algoritmo de ECMAScript, que es laborioso
    /// para los dobles. Aquí no hace falta — `OOS6003` prohíbe los decimales sin
    /// comillas, así que **todo número de la forma canónica es un entero** y
    /// todo decimal es una cadena. La regla de §4.1 no era higiene: se paga
    /// sola aquí.
    pub fn jcs(&self) -> String {
        let mut out = String::new();
        self.write_jcs(&mut out);
        out
    }

    fn write_jcs(&self, out: &mut String) {
        match self {
            Json::Str(s) => escribir_cadena(out, s),
            Json::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_jcs(out);
                }
                out.push(']');
            }
            Json::Obj(m) => {
                // `BTreeMap` ordena por bytes UTF-8; RFC 8785 exige unidades de
                // código UTF-16. Coinciden en todo el BMP y difieren por encima
                // de U+FFFF. Nuestras claves son identificadores ASCII, pero
                // depender de eso sería depender de que nadie escriba un emoji
                // en una clave de extensión.
                let mut claves: Vec<&String> = m.keys().collect();
                claves.sort_by_key(|k| k.encode_utf16().collect::<Vec<u16>>());
                out.push('{');
                for (i, k) in claves.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    escribir_cadena(out, k);
                    out.push(':');
                    m[*k].write_jcs(out);
                }
                out.push('}');
            }
        }
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
    fn jcs_ordena_por_utf16_y_no_deja_espacios() {
        let j = Json::obj([
            ("b", Json::Int(1)),
            ("a", Json::Arr(vec![Json::Int(2), Json::Bool(true)])),
        ]);
        assert_eq!(j.jcs(), r#"{"a":[2,true],"b":1}"#);
    }

    /// El orden es por unidades UTF-16, no por puntos de código (RFC 8785
    /// §3.2.3). U+1F600 va DESPUÉS de U+FB33 en UTF-8 y ANTES en UTF-16, porque
    /// su primer sustituto es 0xD83D.
    #[test]
    fn jcs_ordena_los_sustitutos_como_manda_el_estandar() {
        let j = Json::Obj(
            [
                ("\u{1f600}".to_string(), Json::Int(1)),
                ("\u{fb33}".to_string(), Json::Int(2)),
            ]
            .into_iter()
            .collect(),
        );
        let s = j.jcs();
        assert!(
            s.find('\u{1f600}') < s.find('\u{fb33}'),
            "orden UTF-16 incumplido: {s}"
        );
    }

    #[test]
    fn escapa_lo_que_debe() {
        assert_eq!(Json::s("a\"b\\c\n").pretty(), r#""a\"b\\c\n""#);
    }
}
