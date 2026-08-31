//! Proyección de un paquete a **esquema Cedar**.
//!
//! OOS no define un lenguaje de autorización: las políticas son Cedar. Lo que
//! sí define —y hay que certificar— es el **mapeo determinista de un paquete a
//! un esquema Cedar**. Si dos implementaciones proyectaran distinto, las mismas
//! políticas dejarían de validar, y la portabilidad del bundle sería falsa.
//!
//! # Las dos proyecciones que hacen el trabajo
//!
//! **`Property in [Label, <Entidad>]`.** Una propiedad pertenece a la vez a su
//! entidad y a **todas sus etiquetas**. Es lo que permite escribir
//!
//! ```text
//! resource in Label::"gdpr.sensitivity:high"
//! ```
//!
//! en lugar de enumerar propiedades, y por tanto lo que hace que **una entidad
//! nueva quede gobernada el día que se etiqueta**, sin tocar ninguna política.
//! La alternativa —enumerar— convierte cada alta de propiedad en una edición de
//! políticas, y ahí es donde la gobernanza se cae en la práctica.
//!
//! **`<Entidad> in [<Entidad>]`.** Una autorreferencia —`manager` apuntando a la
//! propia entidad— se proyecta como jerarquía de entidades. De ahí sale el ReBAC
//! estilo Zanzibar sin añadir un segundo sistema de autorización.

use crate::flow;
use crate::json::Json;
use crate::link::Package;
use std::collections::{BTreeMap, BTreeSet};

/// El vocabulario de acciones. Cerrado a propósito: una acción que el motor no
/// sabe interpretar es una política que no se puede hacer cumplir.
const ACCIONES: &[&str] = &["read", "aggregate", "export", "invoke"];

fn entidad(memberof: &[&str]) -> Json {
    Json::obj([(
        "memberOfTypes",
        Json::Arr(memberof.iter().map(|s| Json::s(*s)).collect()),
    )])
}

/// ¿Alguna relación de esta entidad apunta a la propia entidad?
///
/// Es lo que se proyecta como jerarquía, y de ahí sale el ReBAC estilo Zanzibar
/// sin añadir un segundo sistema de autorización. Se comparte entre las dos
/// emisiones —JSON y sintaxis nativa— porque proyectar distinto según el
/// formato de salida sería exactamente la divergencia que este módulo evita.
fn autorreferencia(e: &crate::link::Loaded, qn: &str) -> bool {
    e.section("relations")
        .map(|rs| {
            rs.entries().iter().any(|(_, rv)| {
                rv.get("target")
                    .and_then(|(_, t)| t.as_str())
                    .is_some_and(|t| {
                        crate::normalize::qualify(t, e.meta("namespace").and_then(|n| n.as_str()))
                            == qn
                    })
            })
        })
        .unwrap_or(false)
}

/// Una entidad que declara `principal: true` puede ser el SUJETO de una
/// peticion, y sus propiedades son la forma que la capa de identidad tiene que
/// rellenar. Sin esto el principal era `Role` —una bolsa sin atributos— y
/// `principal.department` no era expresable: lo que emitiamos era RBAC.
fn es_principal(e: &crate::link::Loaded) -> bool {
    e.section("principal").and_then(|n| n.as_str()) == Some("true")
}

/// El tipo Cedar de cada propiedad. Solo los escalares que Cedar tiene: lo
/// demas se omite en vez de traducirse a `String`, porque un tipo inventado en
/// el esquema hace que el analizador estatico valide lo que no debe.
fn tipo_cedar(t: &str) -> Option<&'static str> {
    Some(match t {
        "String" | "Date" | "Time" | "DateTime" | "DateTimeTz" | "Opaque" => "String",
        "Integer" => "Long",
        "Boolean" => "Bool",
        _ => return None,
    })
}

fn atributos(e: &crate::link::Loaded) -> BTreeMap<String, Json> {
    let mut out = BTreeMap::new();
    let Some(ps) = e.section("properties") else {
        return out;
    };
    for (k, v) in ps.entries() {
        let (Some(nombre), Some(t)) = (k.as_str(), v.get("type").and_then(|(_, x)| x.as_str()))
        else {
            continue;
        };
        if let Some(ct) = tipo_cedar(t) {
            out.insert(
                nombre.to_string(),
                Json::obj([("type", Json::s(ct)), ("required", Json::Bool(true))]),
            );
        }
    }
    out
}

fn miembro_de(tipos: &[String]) -> Json {
    Json::obj([(
        "memberOfTypes",
        Json::Arr(tipos.iter().map(|t| Json::s(t.as_str())).collect()),
    )])
}

pub fn emit(pkg: &Package) -> Json {
    let lat = flow::lattices(pkg);

    // Cada nivel de cada retículo es un miembro de `Label`. `oos.maturity`
    // aparece aunque el paquete no lo declare: es estándar de la especificación
    // y siempre está activo.
    let etiquetas: Vec<Json> = lat
        .values()
        .flat_map(|l| {
            l.levels
                .iter()
                .map(move |n| Json::s(format!("{}:{}", l.qname, n)))
        })
        .collect();

    let mut tipos: BTreeMap<String, Json> = BTreeMap::new();
    // `Label` es un tipo ENUMERADO, y eso lo dice Cedar: `entity Label enum
    // [...]`. La forma JSON lo perdia y los niveles viajaban en un
    // `x-oos-labels` inventado, que Cedar rechaza — no admite campos
    // desconocidos en el espacio de nombres. Es P7: el anfitrion ya lo tiene.
    tipos.insert(
        "Label".into(),
        Json::obj([("enum", Json::Arr(etiquetas.clone()))]),
    );
    tipos.insert("Role".into(), entidad(&[]));

    let mut nombres: Vec<String> = Vec::new();
    // `Role` se queda para el principal que NO es una persona modelada: un
    // servicio, un trabajo programado. RBAC puro sigue siendo valido.
    let mut principales: Vec<String> = vec!["Role".to_string()];
    for e in pkg.entities() {
        let Some(qn) = e.qname() else { continue };
        let corto = qn.rsplit('.').next().unwrap_or(&qn).to_string();

        // Autorreferencia → jerarquía. Se mira el destino de cada relación: si
        // apunta a la propia entidad, esa entidad es miembro de sí misma.
        let auto = autorreferencia(e, &qn);

        // Autorreferencia: la entidad es miembro de sí misma, y eso **es** la
        // jerarquía. `manager in Employee` sale de aquí y no de un sistema aparte.
        let padres = if auto {
            vec![corto.clone()]
        } else {
            Vec::new()
        };
        let mut t = miembro_de(&padres);
        // Los atributos SOLO en un principal. El recurso se posiciona por
        // PERTENENCIA —`resource in Label::"..."`, `resource in principal`— y no
        // por atributos: describirlo con atributos obligaria al motor a LEER el
        // recurso para autorizar, que es la lectura gobernada que decide el
        // acceso a si misma.
        if es_principal(e) {
            let attrs = atributos(e);
            if let Json::Obj(ref mut m) = t
                && !attrs.is_empty()
            {
                // Un `shape` es un TIPO en el formato JSON de Cedar, no un mapa
                // de atributos: `{ "type": "Record", "attributes": {...} }`. La
                // forma legible no lo exige, asi que las dos representaciones
                // divergian y solo una era valida.
                let mut forma: BTreeMap<String, Json> = BTreeMap::new();
                forma.insert("type".into(), Json::s("Record"));
                forma.insert("attributes".into(), Json::Obj(attrs));
                m.insert("shape".into(), Json::Obj(forma));
            }
            principales.push(corto.clone());
        }
        tipos.insert(corto.clone(), t);
        nombres.push(corto);
    }

    // `Property` pertenece a `Label` y a todos los tipos de entidad. Es la
    // proyección que hace innecesario enumerar propiedades en las políticas.
    let mut padres: Vec<String> = vec!["Label".to_string()];
    padres.extend(nombres.iter().cloned());
    tipos.insert("Property".into(), miembro_de(&padres));

    let recursos: BTreeSet<String> = nombres
        .iter()
        .cloned()
        .chain(std::iter::once("Property".to_string()))
        .collect();
    let acciones: BTreeMap<String, Json> = ACCIONES
        .iter()
        .map(|a| {
            (
                (*a).to_string(),
                Json::obj([(
                    "appliesTo",
                    Json::obj([
                        (
                            "principalTypes",
                            Json::Arr(principales.iter().map(|r| Json::s(r.as_str())).collect()),
                        ),
                        (
                            "resourceTypes",
                            Json::Arr(recursos.iter().map(|r| Json::s(r.as_str())).collect()),
                        ),
                        // `context` cerrado en `purpose`, y OBLIGATORIO. Se leia
                        // para `OOS5015` y no se declaraba en ninguna parte: una
                        // politica que la usara no validaba contra nuestro
                        // propio esquema. Obligatorio porque una peticion sin
                        // finalidad no se puede comprobar contra la limitacion
                        // de finalidad, y lo que no se declara se deniega (P4).
                        (
                            "context",
                            Json::obj([
                                ("type", Json::s("Record")),
                                (
                                    "attributes",
                                    Json::obj([(
                                        "purpose",
                                        Json::obj([
                                            ("type", Json::s("String")),
                                            ("required", Json::Bool(true)),
                                        ]),
                                    )]),
                                ),
                            ]),
                        ),
                    ]),
                )]),
            )
        })
        .collect();

    // El espacio de nombres vacío es el que Cedar usa por defecto, y mantiene el
    // esquema legible: `Label::"gdpr.sensitivity:high"`, no `OOS::Label::"…"`.
    Json::obj([(
        "",
        Json::obj([
            ("entityTypes", Json::Obj(tipos)),
            ("actions", Json::Obj(acciones)),
        ]),
    )])
}

/// El esquema en la **sintaxis nativa de Cedar**, que es la forma en que se
/// compromete al repositorio y la que consume el tooling de Cedar.
///
/// Los niveles se emiten como tipo de entidad enumerado. No es cosmética: con
/// la enumeración, una política que mencione una etiqueta inexistente es un
/// **error de validación de Cedar**; sin ella, `Label::"gdpr.sensitivity:critical"`
/// es una referencia perfectamente legal a una entidad que no existe, y la
/// política deja de aplicarse sin decir nada.
pub fn emit_text(pkg: &Package) -> String {
    let lat = flow::lattices(pkg);
    let mut out = String::new();
    out.push_str("// GENERADO POR ORE - NO EDITAR\n");
    out.push_str("// Proyeccion del paquete segun `00-overview.md` 4.1.\n\n");

    let etiquetas: Vec<String> = lat
        .values()
        .flat_map(|l| {
            l.levels
                .iter()
                .map(|n| format!("\"{}:{}\"", l.qname, n))
                .collect::<Vec<_>>()
        })
        .collect();
    out.push_str(&format!("entity Label enum [{}];\n", etiquetas.join(", ")));
    out.push_str("entity Role;\n");

    let mut nombres: Vec<String> = Vec::new();
    let mut principales: Vec<String> = vec!["Role".to_string()];
    for e in pkg.entities() {
        let Some(qn) = e.qname() else { continue };
        let corto = qn.rsplit('.').next().unwrap_or(&qn).to_string();
        let jerarquia = if autorreferencia(e, &qn) {
            format!(" in [{corto}]")
        } else {
            String::new()
        };
        // Los atributos SOLO en un principal: el recurso se posiciona por
        // pertenencia, no describiendolo con atributos.
        let forma = if es_principal(e) {
            principales.push(corto.clone());
            let attrs = atributos(e);
            let campos: Vec<String> = attrs
                .iter()
                .map(|(k, v)| {
                    let t = match v {
                        Json::Obj(m) => match m.get("type") {
                            Some(Json::Str(x)) => x.as_str(),
                            _ => "String",
                        },
                        _ => "String",
                    };
                    format!("{k}: {t}")
                })
                .collect();
            if campos.is_empty() {
                String::new()
            } else {
                format!(" {{ {} }}", campos.join(", "))
            }
        } else {
            String::new()
        };
        out.push_str(&format!("entity {corto}{jerarquia}{forma};\n"));
        nombres.push(corto);
    }

    let mut padres = vec!["Label".to_string()];
    padres.extend(nombres.iter().cloned());
    out.push_str(&format!("entity Property in [{}];\n\n", padres.join(", ")));

    let mut recursos = nombres.clone();
    recursos.push("Property".to_string());
    for a in ACCIONES {
        out.push_str(&format!(
            "action \"{a}\" appliesTo {{ principal: [{}], resource: [{}], \
             context: {{ purpose: String }} }};\n",
            principales.join(", "),
            recursos.join(", ")
        ));
    }
    out
}
