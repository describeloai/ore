//! Normalización semántica — N1 a N8 de `90-canonical-form`.
//!
//! Convierte el árbol YAML de un documento en el valor JSON del que sale su
//! digest. Se aplica **antes** de serializar, y cada regla existe porque sin
//! ella dos autores que dicen lo mismo producirían identidades distintas.
//!
//! | | Regla | Qué deja de importar |
//! |---|---|---|
//! | N1 | cualificación de nombres | escribir `Employee` o `hr.Employee` |
//! | N2 | defaults materializados | omitir lo que ya era el valor por defecto |
//! | N3 | ausencia, nunca nulo | escribir `null` en vez de no escribir |
//! | N4 | conjuntos ordenados | el orden, **solo** donde no es semántico |
//! | N5 | Unicode NFC | cómo compuso el editor una `ñ` |
//! | N6 | identificadores | *nada* — aquí las mayúsculas sí importan |
//! | N7 | comentarios y formato | todo lo que no es significado |
//! | N8 | lo derivado no se serializa | lo que calculó el compilador |
//!
//! # N8 es la única que se cumple no haciendo nada
//!
//! Las otras siete transforman. N8 **prohíbe**: la etiqueta que `flow` computa
//! para una propiedad derivada, el linaje, el grafo de consumidores —todo eso
//! existe ya en memoria cuando se llega aquí, y sería trivial y tentador
//! inyectarlo. No se inyecta. Son salida del compilador y viven en el bundle,
//! no en el repositorio (principio P2).

use crate::json::Json;
use crate::link::{Loaded, Package};
use crate::parse::{Node, Style};
use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

/// N4 · Los campos lista cuyo orden **no** es semántico.
///
/// El resto son secuencias y conservan su orden. El defecto es deliberado:
/// ordenar algo cuyo orden significa —`primaryKey`, `enum`, los niveles de un
/// retículo, `moved`— corrompe el documento en silencio, mientras que no ordenar
/// un conjunto solo hace que dos escrituras equivalentes no converjan. De los
/// dos errores posibles, este módulo comete el reversible.
const CONJUNTOS: &[&str] = &[
    "predicatePushdown",
    "aggregatePushdown",
    "requiredFilters",
    "datasources",
    "dependencies",
    "reviewers",
    "roles",
    "tags",
    "authoritativeDefinitions",
];

/// N1 · Campos cuyo valor es una **referencia** a otro documento y por tanto se
/// expande al nombre cualificado. Un nombre corto es azúcar del autor, no una
/// identidad distinta.
fn es_referencia(clave: &str) -> bool {
    matches!(clave, "targetEntity" | "target")
}

/// N1 · Expande un nombre corto con el espacio de nombres de quien lo escribe.
///
/// Pública porque **la fase de enlazado tiene que resolver con esta misma
/// regla**. Si `link` exigiera el nombre largo y `normalize` aceptara el corto,
/// habría documentos que compilan y no resuelven, o peor: que resuelven a una
/// cosa y cuyo digest describe otra.
pub fn qualify(nombre: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(ns) if !nombre.contains('.') => format!("{ns}.{nombre}"),
        _ => nombre.to_string(),
    }
}

/// N3 · Las formas en que YAML escribe «nada».
fn es_nulo(raw: &str, style: Style) -> bool {
    style == Style::Plain && matches!(raw, "" | "~" | "null" | "Null" | "NULL")
}

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

struct Ctx {
    /// El espacio de nombres del documento, para N1.
    namespace: Option<String>,
}

fn escalar(raw: &str, style: Style, clave: &str, ctx: &Ctx) -> Json {
    if style != Style::Plain {
        // Entrecomillado es siempre una cadena. Es lo que hace que `"68400.50"`
        // sobreviva intacto: §4.1 y `OOS6003` conspiran para que aquí no haya
        // ninguna decisión que tomar.
        return Json::Str(nfc(raw));
    }
    match raw {
        "true" => return Json::Bool(true),
        "false" => return Json::Bool(false),
        _ => {}
    }
    if let Ok(n) = raw.parse::<i64>() {
        return Json::Int(n);
    }

    // N1 · un nombre sin punto en un campo de referencia es la forma corta.
    if es_referencia(clave) {
        return Json::Str(nfc(&qualify(raw, ctx.namespace.as_deref())));
    }
    Json::Str(nfc(raw))
}

fn valor(n: &Node, clave: &str, ctx: &Ctx) -> Option<Json> {
    match n {
        Node::Scalar { raw, style, .. } => {
            (!es_nulo(raw, *style)).then(|| escalar(raw, *style, clave, ctx))
        }
        Node::Mapping { entries, .. } => {
            let mut m: BTreeMap<String, Json> = BTreeMap::new();
            for (k, v) in entries {
                let Some(nombre) = k.as_str() else { continue };
                // N7 · el comentario ya se descartó al analizar; aquí se
                // descarta el resto del formato, que es todo lo que no es una
                // clave o un valor.
                if let Some(j) = valor(v, nombre, ctx) {
                    m.insert(nfc(nombre), j);
                }
            }
            Some(Json::Obj(m))
        }
        Node::Sequence { items, .. } => {
            let mut xs: Vec<Json> = items.iter().filter_map(|i| valor(i, clave, ctx)).collect();
            // N4 · ordenar por la forma canónica serializada, que es la única
            // ordenación que no depende de cómo esté escrito el elemento.
            if CONJUNTOS.contains(&clave) {
                xs.sort_by_key(|x| x.jcs());
            }
            Some(Json::Arr(xs))
        }
    }
}

/// N2 · Materialización de valores por defecto.
///
/// La forma canónica no contiene valores implícitos: un binding que omite
/// `materialization` y otro que escribe `mode: passthrough` **dicen lo mismo**, y
/// tienen que producir el mismo digest. Omitir es un atajo de escritura, no una
/// afirmación distinta.
fn defaults(kind: crate::document::Kind, doc: &mut Json) {
    use crate::document::Kind;
    let Json::Obj(raiz) = doc else { return };
    let Some(Json::Obj(spec)) = raiz.get_mut("spec") else {
        return;
    };
    if kind == Kind::Binding {
        let mat = spec
            .entry("materialization".to_string())
            .or_insert_with(|| Json::Obj(BTreeMap::new()));
        if let Json::Obj(m) = mat {
            m.entry("mode".to_string())
                .or_insert_with(|| Json::s("passthrough"));
        }
    }
}

/// La identidad de un documento: `kind:nombreCualificado`.
///
/// **Nunca la ruta.** El nombre del fichero es incidental, igual que los
/// comentarios y la indentación: renombrar `Employee.yaml` a `emp.yaml` no
/// cambia lo que el documento dice, y por tanto no debe cambiar su digest ni
/// invalidar una firma (`90-canonical-form` §5.2).
pub fn doc_id(d: &Loaded) -> String {
    format!(
        "{}:{}",
        d.kind.as_str(),
        d.qname().unwrap_or_else(|| "<sin nombre>".into())
    )
}

/// La forma canónica de un documento.
pub fn document(d: &Loaded) -> Json {
    let ctx = Ctx {
        namespace: d
            .meta("namespace")
            .and_then(|n| n.as_str())
            .map(str::to_string),
    };
    let mut j = valor(&d.root, "", &ctx).unwrap_or(Json::Obj(BTreeMap::new()));
    defaults(d.kind, &mut j);
    j
}

/// ¿Es este documento el lock? Es un artefacto generado, no fuente: entra en el
/// digest del **bundle**, no en el del paquete (§5.3).
pub fn es_lock(d: &Loaded) -> bool {
    d.path.file_name().is_some_and(|n| n == "ontology.lock")
}

/// La forma canónica de un paquete: sus documentos fuente por identidad.
///
/// Ordenados por `docId`, que es lo que hace que **mover un paquete de la forma
/// plana a `packages/hr/` no cambie el artefacto**. La estructura de carpetas es
/// una preferencia de quien organiza el repositorio; la ontología es la misma.
pub fn package(pkg: &Package) -> BTreeMap<String, Json> {
    pkg.docs
        .iter()
        .filter(|d| !es_lock(d))
        .map(|d| (doc_id(d), document(d)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Kind;

    fn doc(kind: Kind, yaml: &str) -> Loaded {
        Loaded {
            path: "x.yaml".into(),
            kind,
            root: crate::parse::parse(yaml).unwrap(),
        }
    }

    #[test]
    fn n1_expande_la_forma_corta() {
        let corto = document(&doc(
            Kind::Binding,
            "metadata: { name: erp, namespace: hr }\nspec: { targetEntity: Employee }\n",
        ));
        let largo = document(&doc(
            Kind::Binding,
            "metadata: { name: erp, namespace: hr }\nspec: { targetEntity: hr.Employee }\n",
        ));
        assert_eq!(corto.jcs(), largo.jcs());
    }

    #[test]
    fn n2_materializa_el_modo_por_defecto() {
        let j = document(&doc(Kind::Binding, "spec: { targetEntity: hr.E }\n"));
        assert!(j.jcs().contains(r#""mode":"passthrough""#), "{}", j.jcs());
    }

    #[test]
    fn n3_omite_los_nulos() {
        let j = document(&doc(Kind::Entity, "spec: { a: ~, b: 1 }\n"));
        assert_eq!(j.jcs(), r#"{"spec":{"b":1}}"#);
    }

    /// N4 en sus dos direcciones, que es donde está el contenido de la regla.
    #[test]
    fn n4_ordena_conjuntos_y_respeta_secuencias() {
        let uno = document(&doc(
            Kind::Binding,
            "spec: { capabilities: { predicatePushdown: [range, eq, in] } }\n",
        ));
        let otro = document(&doc(
            Kind::Binding,
            "spec: { capabilities: { predicatePushdown: [in, range, eq] } }\n",
        ));
        assert_eq!(
            uno.jcs(),
            otro.jcs(),
            "un conjunto no debe depender del orden"
        );

        let a = document(&doc(Kind::Entity, "spec: { primaryKey: [a, b] }\n"));
        let b = document(&doc(Kind::Entity, "spec: { primaryKey: [b, a] }\n"));
        assert_ne!(a.jcs(), b.jcs(), "una clave compuesta SÍ depende del orden");
    }

    #[test]
    fn n5_converge_nfc_y_nfd() {
        let compuesto = document(&doc(
            Kind::Entity,
            "metadata: { description: \"compañía\" }\n",
        ));
        let descompuesto = document(&doc(
            Kind::Entity,
            "metadata: { description: \"compan\u{303}i\u{301}a\" }\n",
        ));
        assert_eq!(compuesto.jcs(), descompuesto.jcs());
    }

    #[test]
    fn n6_las_mayusculas_distinguen() {
        let a = document(&doc(Kind::Entity, "spec: { primaryKey: [employeeId] }\n"));
        let b = document(&doc(Kind::Entity, "spec: { primaryKey: [employeeid] }\n"));
        assert_ne!(a.jcs(), b.jcs());
    }

    #[test]
    fn n7_el_formato_no_llega_al_digest() {
        let plano = document(&doc(Kind::Entity, "spec: { a: 1, b: 2 }\n"));
        let bloque = document(&doc(
            Kind::Entity,
            "# un comentario\nspec:\n  a: 1\n  b: 2\n",
        ));
        assert_eq!(plano.jcs(), bloque.jcs());
    }

    /// La identidad vive dentro del documento, no en dónde alguien lo guardó.
    #[test]
    fn la_identidad_no_es_la_ruta() {
        let mut a = doc(
            Kind::Entity,
            "metadata: { name: Employee, namespace: hr }\n",
        );
        let mut b = doc(
            Kind::Entity,
            "metadata: { name: Employee, namespace: hr }\n",
        );
        a.path = "entities/Employee.yaml".into();
        b.path = "packages/hr/entities/emp_v2_final.yaml".into();
        assert_eq!(doc_id(&a), doc_id(&b));
        assert_eq!(doc_id(&a), "Entity:hr.Employee");
    }
}
