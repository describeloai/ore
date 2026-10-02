//! Una [`Firma`] → su documento `Function`, con la forma de los casos de
//! conformidad de v1alpha18.
//!
//! **Determinista e idempotente**: la misma firma da los mismos bytes, siempre,
//! en el orden de §4.3. Así un `generar` que no cambia nada no ensucia el
//! árbol, y un diff del documento es un cambio de contrato y nada más.
//!
//! La coherencia (`OOS2013`) **no** compara estos bytes: compara el documento
//! comprometido con este, campo a campo, después de leer los dos. Alguien puede
//! reindentar el fichero sin que eso sea un fallo, como con el esquema Cedar.

use crate::firma::{Campo, Firma, Salida};
use std::fmt::Write;

/// La de una firma que solo usa la tabla de v1alpha18; la de cada una la
/// dice [`Firma::api_version`] (v1alpha20 `01` §6).
pub const API_VERSION: &str = "oos.dev/v1alpha18";

/// La primera línea de un documento generado. Es la marca que permite a
/// `generar` borrar el documento de un `def` que ya no existe sin tocar nunca
/// uno escrito a mano.
pub const MARCA: &str = "# generado por ore desde";

/// Dónde vive el documento en el paquete (§4.8: la herramienta lo pone ahí; un
/// validador lo encuentra por su `entrypoint`).
pub fn ruta_del_documento(f: &Firma) -> String {
    format!("functions/{}.yaml", f.nombre)
}

/// Si un documento lo generó `ore` (empieza por [`MARCA`]).
pub fn es_generado(texto: &str) -> bool {
    texto.trim_start_matches('\u{feff}').starts_with(MARCA)
}

pub fn documento(f: &Firma, paquete: &str) -> String {
    documento_con_dueno(f, paquete, None)
}

/// La de un documento con `owner` (v1alpha21 `01` §4): la más baja que lo
/// describe, porque antes la clave no existía.
pub const API_VERSION_CON_DUENO: &str = "oos.dev/v1alpha21";

/// [`documento`] con quien responde de la función (v1alpha21 `01` §4). `owner` no
/// sale del código: lo da quien crea el documento, y al regenerarlo se conserva
/// el que tenía. Sin él, los mismos bytes de siempre.
pub fn documento_con_dueno(f: &Firma, paquete: &str, owner: Option<&str>) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{MARCA} {} · se edita el def, no este fichero",
        f.entrypoint
    );
    let version = if owner.is_some() {
        API_VERSION_CON_DUENO
    } else {
        f.api_version()
    };
    let _ = writeln!(s, "apiVersion: {version}");
    s.push_str("kind: Function\n");
    match &f.descripcion {
        None => {
            let _ = writeln!(
                s,
                "metadata: {{ name: {}, namespace: {} }}",
                escalar(&f.nombre),
                escalar(paquete)
            );
        }
        Some(d) => {
            s.push_str("metadata:\n");
            let _ = writeln!(s, "  name: {}", escalar(&f.nombre));
            let _ = writeln!(s, "  namespace: {}", escalar(paquete));
            let _ = writeln!(s, "  description: {}", escalar(d));
        }
    }
    s.push_str("spec:\n");
    if let Some(o) = owner {
        let _ = writeln!(s, "  owner: {}", escalar(o));
    }
    s.push_str("  runtime: python\n");
    let _ = writeln!(s, "  entrypoint: {}", escalar(&f.entrypoint));
    if let Some(o) = &f.over {
        let _ = writeln!(s, "  over: {}", escalar(o));
    }
    if let Some(r) = &f.reads {
        let _ = writeln!(s, "  reads: {}", lista(r));
    }
    if let Some(m) = &f.models {
        let _ = writeln!(s, "  models: {}", lista(m));
    }
    if !f.entrada.is_empty() {
        s.push_str("  input:\n");
        campos(&mut s, &f.entrada);
    }
    match &f.salida {
        Salida::Valor(t) => {
            let _ = writeln!(s, "  output: {{ type: {} }}", escalar(&t.to_string()));
        }
        Salida::Campos(cs) if cs.is_empty() => s.push_str("  output: {}\n"),
        Salida::Campos(cs) => {
            s.push_str("  output:\n");
            campos(&mut s, cs);
        }
    }
    if let Some(t) = &f.timeout {
        let _ = writeln!(s, "  limits: {{ timeout: {} }}", escalar(t));
    }
    s
}

fn campos(s: &mut String, cs: &[Campo]) {
    for c in cs {
        let req = if c.requerido { ", required: true" } else { "" };
        let _ = writeln!(
            s,
            "    {}: {{ type: {}{req} }}",
            escalar(&c.nombre),
            escalar(&c.tipo.to_string())
        );
    }
}

fn lista(xs: &[String]) -> String {
    let v: Vec<String> = xs.iter().map(|x| escalar(x)).collect();
    format!("[{}]", v.join(", "))
}

/// Un escalar de YAML que se lee como la cadena que es, también dentro de un
/// `{ … }` o un `[ … ]`: sin comillas si no hay ninguna duda, y si no, entre
/// comillas simples.
pub fn escalar(s: &str) -> String {
    if es_llano(s) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

fn es_llano(s: &str) -> bool {
    const RESERVADAS: &[&str] = &[
        "true", "false", "null", "yes", "no", "on", "off", "y", "n", "~",
    ];
    let Some(primero) = s.chars().next() else {
        return false;
    };
    !(s != s.trim()
        || RESERVADAS.iter().any(|r| s.eq_ignore_ascii_case(r))
        || primero.is_ascii_digit()
        || "-+.?:,[]{}#&*!|>'\"%@`".contains(primero)
        || s.contains(": ")
        || s.contains(" #")
        || s.ends_with(':')
        || s.chars().any(|c| c.is_control() || ",[]{}".contains(c)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firma::Tipo;

    #[test]
    fn un_escalar_dudoso_va_entre_comillas() {
        assert_eq!(escalar("ventas.default.pedidos"), "ventas.default.pedidos");
        assert_eq!(escalar("list<Decimal>"), "list<Decimal>");
        assert_eq!(escalar("funciones/x.py:evaluar"), "funciones/x.py:evaluar");
        assert_eq!(
            escalar("Evalúa una línea y nada más."),
            "Evalúa una línea y nada más."
        );
        assert_eq!(escalar("no"), "'no'");
        assert_eq!(escalar("2m"), "'2m'");
        assert_eq!(escalar("a: b"), "'a: b'");
        assert_eq!(escalar("«f\"{x}\"»"), "'«f\"{x}\"»'");
        assert_eq!(escalar("it's"), "it's");
        assert_eq!(escalar("'x'"), "'''x'''");
        assert_eq!(escalar(""), "''");
    }

    #[test]
    fn el_documento_tiene_la_forma_de_los_casos() {
        let f = Firma {
            nombre: "riesgo".into(),
            entrypoint: "funciones/riesgo.py:riesgo".into(),
            descripcion: None,
            over: Some("ventas.clientes".into()),
            reads: Some(vec!["ventas.pedidos".into()]),
            models: None,
            timeout: None,
            entrada: vec![
                Campo {
                    nombre: "umbral".into(),
                    tipo: Tipo::Decimal,
                    requerido: true,
                },
                Campo {
                    nombre: "moneda".into(),
                    tipo: Tipo::String,
                    requerido: false,
                },
            ],
            salida: Salida::Campos(vec![Campo {
                nombre: "nivel".into(),
                tipo: Tipo::String,
                requerido: true,
            }]),
        };
        assert_eq!(
            documento(&f, "ventas"),
            "# generado por ore desde funciones/riesgo.py:riesgo · se edita el def, no este fichero\n\
             apiVersion: oos.dev/v1alpha18\n\
             kind: Function\n\
             metadata: { name: riesgo, namespace: ventas }\n\
             spec:\n  runtime: python\n  entrypoint: funciones/riesgo.py:riesgo\n  over: ventas.clientes\n  \
             reads: [ventas.pedidos]\n  input:\n    umbral: { type: Decimal, required: true }\n    \
             moneda: { type: String }\n  output:\n    nivel: { type: String, required: true }\n"
        );
        assert!(es_generado(&documento(&f, "ventas")));
        assert_eq!(documento(&f, "ventas"), documento(&f, "ventas"));
    }
}
