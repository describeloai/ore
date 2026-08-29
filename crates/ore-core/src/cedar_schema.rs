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
    tipos.insert("Label".into(), entidad(&[]));
    tipos.insert("Role".into(), entidad(&[]));

    let mut nombres: Vec<String> = Vec::new();
    for e in pkg.entities() {
        let Some(qn) = e.qname() else { continue };
        let corto = qn.rsplit('.').next().unwrap_or(&qn).to_string();

        // Autorreferencia → jerarquía. Se mira el destino de cada relación: si
        // apunta a la propia entidad, esa entidad es miembro de sí misma.
        let auto = e
            .section("relations")
            .map(|rs| {
                rs.entries().iter().any(|(_, rv)| {
                    rv.get("target")
                        .and_then(|(_, t)| t.as_str())
                        .is_some_and(|t| {
                            crate::normalize::qualify(
                                t,
                                e.meta("namespace").and_then(|n| n.as_str()),
                            ) == qn
                        })
                })
            })
            .unwrap_or(false);

        // Autorreferencia: la entidad es miembro de sí misma, y eso **es** la
        // jerarquía. `manager in Employee` sale de aquí y no de un sistema aparte.
        let padres = if auto {
            vec![corto.clone()]
        } else {
            Vec::new()
        };
        tipos.insert(corto.clone(), miembro_de(&padres));
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
                        ("principalTypes", Json::Arr(vec![Json::s("Role")])),
                        (
                            "resourceTypes",
                            Json::Arr(recursos.iter().map(|r| Json::s(r.as_str())).collect()),
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
            ("x-oos-labels", Json::Arr(etiquetas)),
        ]),
    )])
}
