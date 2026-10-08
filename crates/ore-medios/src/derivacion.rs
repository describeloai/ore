//! **El linaje de un `commit`** (ADR 0049 B9, `docs/media.md` §2 «`put`
//! derivado»): lo que `apply()` manda al confirmar para que una colección
//! escrita sepa de qué ítem sale cada fichero.
//!
//! ```json
//! { "derivations": [ { "source": {uri, digest}, "derivation": {…},
//!                      "state": "files" | "empty" | "error",
//!                      "files": [ {path, anchor} ], "error": {type, message} } ],
//!   "retire_sources": [identidad…], "retire": [path…] }
//! ```
//!
//! Aquí se lee y se coteja; las filas las escribe `escritura.rs` al sellar.
//! ⭐ **Una entrada reemplaza a la anterior del mismo origen, entera**: es lo
//! que deja a `apply()` no llevar la cuenta de qué páginas daba un contrato
//! antes —el sello retira las que ya no da—.

use crate::indice::Linaje;
use ore_core::json::Json;
use ore_core::parse::{Node, Style};
use std::collections::{BTreeMap, BTreeSet};

/// Qué dio un origen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Estado {
    /// Esos ficheros: `(camino, ancla en JSON)`.
    Ficheros(Vec<(String, Option<String>)>),
    /// Ninguno: una marca `vacio`.
    Vacio,
    /// Falló: una marca `error`, con `{type, message}` en JSON.
    Error(String),
}

/// Una entrada: un origen y lo que dio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrada {
    pub linaje: Linaje,
    pub estado: Estado,
}

/// El linaje entero de un `commit`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Linajes {
    pub entradas: Vec<Entrada>,
    /// Los orígenes que ya no están: se retiran sus ficheros y su marca.
    pub retirar_origenes: Vec<String>,
    /// Caminos sueltos (`Transaction.delete`).
    pub retirar: Vec<String>,
}

impl Linajes {
    pub fn vacio(&self) -> bool {
        self.entradas.is_empty() && self.retirar_origenes.is_empty() && self.retirar.is_empty()
    }

    /// El linaje de cada camino subido que nombra una entrada.
    pub fn de_camino(&self) -> BTreeMap<&str, (&Linaje, Option<&str>)> {
        let mut m = BTreeMap::new();
        for e in &self.entradas {
            if let Estado::Ficheros(fs) = &e.estado {
                for (c, a) in fs {
                    m.insert(c.as_str(), (&e.linaje, a.as_deref()));
                }
            }
        }
        m
    }
}

/// **Un nodo a JSON, sin perder lo que `Json::de_node` pierde**: un `null`
/// sin comillas es nulo (no el texto `"null"`) y un número con decimales es
/// un número (un `bbox` lleva `0.25`).
pub fn a_json(n: &Node) -> Json {
    match n {
        Node::Mapping { entries, .. } => Json::Obj(
            entries
                .iter()
                .filter_map(|(k, v)| k.as_str().map(|k| (k.to_string(), a_json(v))))
                .collect(),
        ),
        Node::Sequence { items, .. } => Json::Arr(items.iter().map(a_json).collect()),
        Node::Scalar {
            raw,
            style: Style::Plain,
            ..
        } => match raw.as_str() {
            "null" | "~" => Json::Crudo("null".into()),
            "true" => Json::Bool(true),
            "false" => Json::Bool(false),
            _ if raw.parse::<i64>().is_ok() => Json::Int(raw.parse().unwrap_or_default()),
            _ if raw.parse::<f64>().is_ok_and(f64::is_finite) => Json::Crudo(raw.clone()),
            _ => Json::s(raw),
        },
        Node::Scalar { raw, .. } => Json::s(raw),
    }
}

fn nulo(n: Option<&Node>) -> bool {
    match n {
        None => true,
        Some(Node::Scalar {
            raw,
            style: Style::Plain,
            ..
        }) => raw == "null" || raw == "~",
        _ => false,
    }
}

fn campo<'a>(n: &'a Node, k: &str) -> Option<&'a Node> {
    n.get(k).map(|(_, v)| v)
}

fn texto<'a>(n: &'a Node, k: &str) -> Option<&'a str> {
    campo(n, k)
        .filter(|v| !nulo(Some(v)))
        .and_then(Node::as_str)
        .filter(|s| !s.is_empty())
}

/// Un JSON canónico, o nada si el campo falta o es nulo.
fn json_de(n: &Node, k: &str) -> Option<String> {
    let v = campo(n, k)?;
    (!nulo(Some(v))).then(|| a_json(v).jcs())
}

/// Una lista de textos.
fn textos(n: &Node, k: &str) -> Result<Vec<String>, String> {
    match campo(n, k) {
        None => Ok(Vec::new()),
        Some(v) if nulo(Some(v)) => Ok(Vec::new()),
        Some(v) => v
            .items()
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("`{k}` es una lista de textos"))
            })
            .collect(),
    }
}

/// La identidad de un origen (v1alpha17 `01` §3.1): su `digest`; sin él, su
/// `uri` fijada.
pub fn identidad(uri: &str, digest: Option<&str>) -> String {
    digest.map_or_else(|| uri.to_string(), str::to_string)
}

/// **Lee** el linaje del cuerpo de `confirmar`. Sin ninguna de sus claves es
/// un `commit` de siempre.
pub fn leer(n: &Node) -> Result<Linajes, String> {
    let mut l = Linajes {
        retirar_origenes: textos(n, "retire_sources")?,
        retirar: textos(n, "retire")?,
        ..Linajes::default()
    };
    let Some(ds) = campo(n, "derivations").filter(|v| !nulo(Some(v))) else {
        return Ok(l);
    };
    for (i, d) in ds.items().iter().enumerate() {
        let donde = |m: &str| format!("`derivations[{i}]`: {m}");
        let fuente = campo(d, "source").ok_or_else(|| donde("falta `source`"))?;
        let uri = texto(fuente, "uri").ok_or_else(|| donde("`source` sin `uri`"))?;
        let digest = texto(fuente, "digest");
        if let Some(dg) = digest
            && !(dg.starts_with("sha256:") && dg.len() == 71)
        {
            return Err(donde("`source.digest` es `sha256:<64 hex>`"));
        }
        let derivacion = json_de(d, "derivation");
        let clave_ok = campo(d, "derivation")
            .and_then(|v| texto(v, "key"))
            .is_some();
        if !clave_ok {
            return Err(donde("`derivation.key` es obligatoria"));
        }
        let ficheros = match campo(d, "files") {
            None => Vec::new(),
            Some(v) if nulo(Some(v)) => Vec::new(),
            Some(v) => v
                .items()
                .iter()
                .map(|f| {
                    let c = texto(f, "path").ok_or_else(|| donde("un fichero sin `path`"))?;
                    Ok((c.to_string(), json_de(f, "anchor")))
                })
                .collect::<Result<Vec<_>, String>>()?,
        };
        let estado = match texto(d, "state") {
            Some("files") if ficheros.is_empty() => {
                return Err(donde(
                    "`state: files` sin ficheros: sin ficheros es `empty`",
                ));
            }
            Some("files") => Estado::Ficheros(ficheros),
            Some(e @ ("empty" | "error")) if !ficheros.is_empty() => {
                return Err(donde(&format!("`state: {e}` no lleva ficheros")));
            }
            Some("empty") => Estado::Vacio,
            Some("error") => Estado::Error(
                json_de(d, "error").ok_or_else(|| donde("`state: error` sin `error`"))?,
            ),
            _ => return Err(donde("`state` es `files`, `empty` o `error`")),
        };
        l.entradas.push(Entrada {
            linaje: Linaje {
                origen: identidad(uri, digest),
                uri: uri.to_string(),
                digest: digest.map(str::to_string),
                ancla: None,
                derivacion,
            },
            estado,
        });
    }
    Ok(l)
}

/// **Coteja** el linaje con lo subido en la transacción. Lo que no cuadra es
/// `media/derivacion` y no se confirma nada.
pub fn cotejar<T>(l: &Linajes, subidos: &BTreeMap<String, T>) -> Result<(), String> {
    let mut origenes = BTreeSet::new();
    let mut de_quien: BTreeMap<&str, &str> = BTreeMap::new();
    let retirar: BTreeSet<&str> = l.retirar.iter().map(String::as_str).collect();
    for e in &l.entradas {
        if !origenes.insert(e.linaje.origen.as_str()) {
            return Err(format!(
                "el origen `{}` está dos veces en `derivations`",
                e.linaje.uri
            ));
        }
        if l.retirar_origenes.contains(&e.linaje.origen) {
            return Err(format!(
                "el origen `{}` está en `derivations` y en `retire_sources`",
                e.linaje.uri
            ));
        }
        let Estado::Ficheros(fs) = &e.estado else {
            continue;
        };
        for (c, _) in fs {
            if !subidos.contains_key(c) {
                return Err(format!(
                    "`{c}` no se subió en esta transacción: el linaje sólo nombra lo que se sube"
                ));
            }
            if let Some(otro) = de_quien.insert(c, &e.linaje.uri) {
                return Err(format!(
                    "`{c}` sale de dos orígenes: `{otro}` y `{}`",
                    e.linaje.uri
                ));
            }
            if retirar.contains(c.as_str()) {
                return Err(format!("`{c}` está en `files` y en `retire`"));
            }
        }
    }
    for c in &l.retirar {
        if subidos.contains_key(c) {
            return Err(format!("`{c}` se sube y se retira en la misma transacción"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn n(t: &str) -> Node {
        ore_core::parse::parse(t).unwrap()
    }

    const D: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn un_null_es_nulo_y_un_decimal_un_numero() {
        let j = a_json(&n(r#"{"a":null,"b":0.25,"c":"null","d":3,"e":[1,true]}"#)).jcs();
        assert_eq!(j, r#"{"a":null,"b":0.25,"c":"null","d":3,"e":[1,true]}"#);
    }

    #[test]
    fn se_lee_y_se_coteja() {
        let cuerpo = format!(
            r#"{{"derivations":[
                {{"source":{{"uri":"ore://l.a.c/a.pdf?v=1","digest":"{D}"}},"derivation":{{"key":"k1","fn":"f"}},
                  "state":"files","files":[{{"path":"a.pdf/p1.png","anchor":{{"kind":"page","page":1}}}},{{"path":"a.pdf/p2.png","anchor":null}}]}},
                {{"source":{{"uri":"ore://l.a.c/b.png?v=1","digest":null}},"derivation":{{"key":"k2"}},"state":"empty"}},
                {{"source":{{"uri":"ore://l.a.c/c.pdf?v=1"}},"derivation":{{"key":"k3"}},"state":"error","error":{{"type":"ValueError","message":"no"}}}}],
              "retire_sources":["ore://l.a.c/viejo.pdf?v=1"],"retire":["suelto.txt"]}}"#
        );
        let l = leer(&n(&cuerpo)).unwrap();
        assert_eq!(l.entradas.len(), 3);
        assert_eq!(
            l.entradas[0].linaje.origen, D,
            "con digest, la identidad es el digest"
        );
        assert_eq!(
            l.entradas[1].linaje.origen, "ore://l.a.c/b.png?v=1",
            "sin él, la uri"
        );
        assert_eq!(
            l.entradas[0].estado,
            Estado::Ficheros(vec![
                (
                    "a.pdf/p1.png".into(),
                    Some(r#"{"kind":"page","page":1}"#.into())
                ),
                ("a.pdf/p2.png".into(), None),
            ])
        );
        assert_eq!(l.entradas[1].estado, Estado::Vacio);
        assert_eq!(
            l.entradas[2].estado,
            Estado::Error(r#"{"message":"no","type":"ValueError"}"#.into())
        );
        let subidos: BTreeMap<String, ()> =
            [("a.pdf/p1.png".into(), ()), ("a.pdf/p2.png".into(), ())].into();
        assert_eq!(cotejar(&l, &subidos), Ok(()));
        // Un fichero que no se subió.
        let solo_uno: BTreeMap<String, ()> = [("a.pdf/p1.png".into(), ())].into();
        assert!(cotejar(&l, &solo_uno).unwrap_err().contains("no se subió"));
    }

    #[test]
    fn lo_que_no_cuadra() {
        let e = |cuerpo: &str| leer(&n(cuerpo)).unwrap_err();
        assert!(e(r#"{"derivations":[{"source":{"uri":"u"},"derivation":{"key":"k"},"state":"files","files":[]}]}"#)
            .contains("sin ficheros es `empty`"));
        assert!(e(r#"{"derivations":[{"source":{"uri":"u"},"derivation":{"key":"k"},"state":"empty","files":[{"path":"x"}]}]}"#)
            .contains("no lleva ficheros"));
        assert!(
            e(r#"{"derivations":[{"source":{"uri":"u"},"state":"empty"}]}"#)
                .contains("`derivation.key`")
        );
        assert!(e(r#"{"derivations":[{"source":{"uri":"u","digest":"md5:x"},"derivation":{"key":"k"},"state":"empty"}]}"#)
            .contains("sha256"));
        assert!(e(r#"{"derivations":[{"source":{"uri":"u"},"derivation":{"key":"k"},"state":"error"}]}"#)
            .contains("sin `error`"));
        assert!(e(r#"{"derivations":[{"source":{"uri":"u"},"derivation":{"key":"k"},"state":"raro"}]}"#)
            .contains("`state`"));

        let subidos: BTreeMap<String, ()> = [("x".into(), ())].into();
        let c = |cuerpo: &str| cotejar(&leer(&n(cuerpo)).unwrap(), &subidos).unwrap_err();
        assert!(c(r#"{"derivations":[
            {"source":{"uri":"u1"},"derivation":{"key":"k"},"state":"files","files":[{"path":"x"}]},
            {"source":{"uri":"u2"},"derivation":{"key":"k"},"state":"files","files":[{"path":"x"}]}]}"#)
            .contains("dos orígenes"));
        assert!(
            c(r#"{"derivations":[
            {"source":{"uri":"u1"},"derivation":{"key":"k"},"state":"empty"},
            {"source":{"uri":"u1"},"derivation":{"key":"k"},"state":"empty"}]}"#)
            .contains("dos veces")
        );
        assert!(c(r#"{"derivations":[{"source":{"uri":"u1"},"derivation":{"key":"k"},"state":"files","files":[{"path":"x"}]}],"retire":["x"]}"#)
            .contains("`files` y en `retire`"));
        assert!(c(r#"{"retire":["x"]}"#).contains("se sube y se retira"));
        assert!(c(r#"{"derivations":[{"source":{"uri":"u1"},"derivation":{"key":"k"},"state":"empty"}],"retire_sources":["u1"]}"#)
            .contains("`retire_sources`"));
    }
}
