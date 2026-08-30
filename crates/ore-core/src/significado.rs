//! Significado — la familia `OOS9xxx`.
//!
//! La cuarta fila, y **la que faltaba debajo de las otras tres**.
//!
//! > `E implements I ⟹ ∀c ∈ I . ∃p ∈ E . is(p) = c`
//!
//! [`flow`](crate::flow) compara etiquetas, [`governance`](crate::governance)
//! exige que estén cubiertas, y **ninguna de las dos comprueba que la
//! clasificación sea consistente** — porque hasta v1alpha4 no había forma de
//! decir que dos propiedades son la misma cosa. Todo v1alpha3 gobierna lo que
//! alguien **acertó a etiquetar**; este módulo es el sustrato de ese acierto.
//!
//! # El concepto no es un módulo, es una tercera herencia
//!
//! Lo que un `is` hace no se implementa aquí: se implementa en `flow`, dentro
//! de `propagar`, y esa brevedad es el resultado. Un `Binding` declara una
//! identidad hacia abajo —esta propiedad es esa columna—; un `is` la declara
//! hacia arriba —esta propiedad es ese concepto—, y la dirección de la herencia
//! ya estaba decidida: se puede elevar, no rebajar (`OOS4012`).
//!
//! **Lo único nuevo es el nivel al que se aplica lo que ya estaba.**
//!
//! # Lo que este módulo no puede ver
//!
//! Una entidad que **declara** implementar y no cumple es un fallo visible.
//! Una columna que **es** un correo personal y que nadie mapeó, no — y ese es
//! el modo de fallo real de un patrimonio sucio. Detectarla exigiría adivinar
//! significado desde un nombre, y `02-entity` ya decidió que un análisis sólido
//! no depende de parsear cadenas.
//!
//! Así que la regla hace lo mismo que `OOS8001`: **convierte en error lo que
//! alguien declaró que importaba.** Tercera vez que la frontera cae en el mismo
//! sitio, y ya no es casualidad — lo declarado es decidible, lo omitido no lo
//! es nunca.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::flow;
use crate::link::Package;
use crate::parse::Node;
use std::collections::{BTreeMap, BTreeSet};

/// Un concepto: lo que una propiedad hereda al escribir `is`.
#[derive(Debug, Clone, Default)]
pub struct Concepto {
    pub tipo: Option<String>,
    /// Lo que el concepto declara del **dato**, no del documento. Sale de
    /// `spec.labels`; `metadata.labels` clasifica el documento y no se hereda.
    pub labels: BTreeMap<String, String>,
}

/// Los conceptos que el paquete puede nombrar.
///
/// Se expone porque [`flow`](crate::flow) los necesita para propagar y **no
/// debe recalcularlos con otro criterio**: dos lecturas del mismo `is` serían
/// dos semánticas, que es justo el fallo que `is` existe para no cometer.
pub fn conceptos(pkg: &Package) -> BTreeMap<String, Concepto> {
    let mut out = BTreeMap::new();
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Property) {
        let Some(q) = d.qname() else { continue };
        let tipo = d
            .section("type")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string());
        let mut labels = BTreeMap::new();
        if let Some(l) = d.section("labels") {
            for (k, v) in l.entries() {
                if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                    labels.insert(k.to_string(), v.to_string());
                }
            }
        }
        out.insert(q, Concepto { tipo, labels });
    }
    out
}

/// El concepto que una propiedad declara ser, si lo declara.
pub fn mapeo(prop: &Node) -> Option<String> {
    prop.get("is")
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string())
}

/// Todos los mapeos del paquete: `entidad.propiedad` → concepto.
///
/// Es el índice que consume un objetivo `implements` de un `Ruleset`.
pub fn mapeos(pkg: &Package) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let Some(ps) = e.section("properties") else {
            continue;
        };
        for (k, v) in ps.entries() {
            if let (Some(n), Some(c)) = (k.as_str(), mapeo(v)) {
                out.insert(format!("{qn}.{n}"), c);
            }
        }
    }
    out
}

/// Qué exige cada interfaz declarada: `interfaz` → conceptos.
pub fn exigencias(pkg: &Package) -> BTreeMap<String, Vec<String>> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Interface)
        .filter_map(|d| {
            Some((
                d.qname()?,
                d.section("requires")
                    .map(|n| n.items())
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|c| c.as_str().map(|s| s.to_string()))
                    .collect(),
            ))
        })
        .collect()
}

/// `interfaz` → las propiedades que la satisfacen, en toda entidad que declare
/// implementarla.
///
/// Es el tercer eje de objetivo de un `Ruleset`, resuelto. **Toda interfaz
/// declarada aparece como clave**, con el conjunto vacío si nadie la
/// implementa: eso es lo que permite distinguir un objetivo que apunta a algo
/// inexistente —`OOS2001`— de uno que apunta a algo real y no casa con nada
/// —`OOS8002`—, que son dos fallos distintos y solo uno es una errata.
///
/// No alcanza a TODA propiedad de esas entidades, y la diferencia es la que
/// evita repetir el error que ya se corrigió en la cobertura por orden de
/// retículo: una regla sobre `Party` habla de nombres legales y correos
/// personales, no de lo que `Customer` tenga además. Acreditar cobertura sobre
/// lo que la interfaz no nombra sería acreditar lo que nadie exigió.
pub fn por_forma(pkg: &Package) -> BTreeMap<String, BTreeSet<String>> {
    let exige = exigencias(pkg);
    let mut out: BTreeMap<String, BTreeSet<String>> = exige
        .keys()
        .map(|i| (i.clone(), BTreeSet::new()))
        .collect();

    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let Some(ps) = e.section("properties") else {
            continue;
        };
        for i in e.section("implements").map(|n| n.items()).unwrap_or(&[]) {
            let Some(nombre) = i.as_str() else { continue };
            let Some(requiere) = exige.get(nombre) else {
                continue;
            };
            let Some(acc) = out.get_mut(nombre) else {
                continue;
            };
            for (k, v) in ps.entries() {
                let (Some(n), Some(c)) = (k.as_str(), mapeo(v)) else {
                    continue;
                };
                if requiere.contains(&c) {
                    acc.insert(format!("{qn}.{n}"));
                }
            }
        }
    }
    out
}

// ── El chequeo completo ─────────────────────────────────────────────────────

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let conceptos = conceptos(pkg);

    // 1 · Un `is` o un `requires` que no resuelve. Antes que nada, porque el
    //     resto compararía contra la nada.
    referencias(pkg, &conceptos, &mut out);
    if !out.is_empty() {
        return out;
    }

    // 2 · Las formas declaradas se satisfacen.
    formas(pkg, &mut out);

    // 3 · Ninguna conjetura fuera de DRAFT.
    conjeturas(pkg, &mut out);

    // 4 · Y ninguna palabra que nadie hable.
    palabras_muertas(pkg, &conceptos, &mut out);

    out
}

// ── OOS2001 · lo que no resuelve ────────────────────────────────────────────

fn referencias(pkg: &Package, conceptos: &BTreeMap<String, Concepto>, out: &mut Vec<Diagnostic>) {
    let interfaces: BTreeSet<String> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Interface)
        .filter_map(|d| d.qname())
        .collect();

    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        if let Some(ps) = e.section("properties") {
            for (k, v) in ps.entries() {
                let (Some(n), Some(c)) = (k.as_str(), mapeo(v)) else {
                    continue;
                };
                if !conceptos.contains_key(&c) {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2001,
                            &e.path,
                            format!("`{qn}.{n}` declara ser `{c}`, que no existe"),
                        )
                        .at(v.pos())
                        .help(
                            "un concepto es un documento `Property`, propio o importado. Si lo \
                             publica una dependencia, comprueba que esté en \
                             `ontology.config.yaml` y fijada en el lock",
                        ),
                    );
                }
            }
        }
        for i in e.section("implements").map(|n| n.items()).unwrap_or(&[]) {
            let Some(nombre) = i.as_str() else { continue };
            if !interfaces.contains(nombre) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2001,
                        &e.path,
                        format!("`{qn}` declara implementar `{nombre}`, que no existe"),
                    )
                    .at(i.pos()),
                );
            }
        }
    }

    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Interface) {
        let qn = d.qname().unwrap_or_default();
        for c in d.section("requires").map(|n| n.items()).unwrap_or(&[]) {
            let Some(nombre) = c.as_str() else { continue };
            if !conceptos.contains_key(nombre) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2001,
                        &d.path,
                        format!("`{qn}` exige `{nombre}`, que no existe"),
                    )
                    .at(c.pos())
                    .help(
                        "una interfaz se expresa en CONCEPTOS, no en nombres de propiedad ni en \
                         otras interfaces: la herencia entre interfaces no está escrita",
                    ),
                );
            }
        }
    }
}

// ── OOS9001 · una forma declarada se satisface ──────────────────────────────

fn formas(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let exige = exigencias(pkg);

    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let mapeados: BTreeSet<String> = e
            .section("properties")
            .map(|ps| ps.entries().iter().filter_map(|(_, v)| mapeo(v)).collect())
            .unwrap_or_default();

        for i in e.section("implements").map(|n| n.items()).unwrap_or(&[]) {
            let Some(nombre) = i.as_str() else { continue };
            let Some(requiere) = exige.get(nombre) else {
                continue;
            };
            let faltan: Vec<&String> = requiere.iter().filter(|c| !mapeados.contains(*c)).collect();
            if faltan.is_empty() {
                continue;
            }
            let lista = faltan
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(
                Diagnostic::new(
                    Code::Oos9001,
                    &e.path,
                    format!("`{qn}` declara implementar `{nombre}` y no la satisface: {lista}"),
                )
                .at(i.pos())
                .help(
                    "la satisfacción se comprueba EN CONCEPTOS, no en nombres: la propiedad \
                     puede llamarse como quiera, pero alguna tiene que declarar `is` sobre cada \
                     concepto que la interfaz exige. Eso es lo que permite que quince \
                     casi-duplicados de quince sistemas implementen la misma forma sin \
                     renombrarse",
                ),
            );
        }
    }
}

// ── OOS9003 · ninguna conjetura fuera de DRAFT ──────────────────────────────

/// La regla leída al revés, que es como hace trabajo: **un documento que no
/// está en `DRAFT` no puede contener una sola conjetura.**
///
/// Es del compilador y no del esquema porque la madurez es **efectiva**: se
/// hereda de la entidad y del `datasource`, y una etiqueta heredada no está
/// escrita en el documento donde vive el `confidence`. Nadie promueve una
/// entidad dejando una propiedad atrás.
fn conjeturas(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let lat = flow::lattices(pkg);
    let efectivas = flow::efectivas(pkg, &lat);

    let ayuda = "`confidence` marca lo que propuso una máquina. Promover exige haber resuelto \
                 cada propuesta una a una: o se confirma y el campo desaparece, o se corrige. \
                 Una importación desde un inductor ajeno que traiga mapeos sin revisar entra \
                 como `DRAFT` o no entra";

    // Ausente NO es `DRAFT`, y la dirección es deliberada: de los dos errores
    // posibles, este es el reversible. Rechazar una conjetura sin marcar cuesta
    // una etiqueta; aceptarla en silencio publica una suposición como si fuera
    // una decisión.
    let dice = |nivel: Option<&str>| match nivel {
        Some(n) => format!("su madurez efectiva es `{n}`"),
        None => "no declara madurez, y ausente no es `DRAFT`".to_string(),
    };

    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let Some(ps) = e.section("properties") else {
            continue;
        };
        for (k, v) in ps.entries() {
            let (Some(n), Some((_, c))) = (k.as_str(), v.get("confidence")) else {
                continue;
            };
            let nivel = efectivas
                .get(&format!("{qn}.{n}"))
                .and_then(|l| l.get("oos.maturity"))
                .map(|s| s.as_str());
            if nivel != Some("DRAFT") {
                out.push(
                    Diagnostic::new(
                        Code::Oos9003,
                        &e.path,
                        format!("`{qn}.{n}` lleva `confidence` y {}", dice(nivel)),
                    )
                    .at(c.pos())
                    .help(ayuda),
                );
            }
        }
    }

    // Y acuñar es una inferencia igual que mapear, así que un concepto cae bajo
    // la misma regla. El mecanismo no distingue las dos, y no debe: son la
    // misma clase de acto, y lo que las separa es la consecuencia.
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Property) {
        let Some(c) = d.section("confidence") else {
            continue;
        };
        let qn = d.qname().unwrap_or_default();
        let nivel = d.meta("labels").and_then(|l| {
            l.entries()
                .iter()
                .find(|(k, _)| k.as_str() == Some("oos.maturity"))
                .and_then(|(_, v)| v.as_str())
        });
        if nivel != Some("DRAFT") {
            out.push(
                Diagnostic::new(
                    Code::Oos9003,
                    &d.path,
                    format!("el concepto `{qn}` lleva `confidence` y {}", dice(nivel)),
                )
                .at(c.pos())
                .help(ayuda),
            );
        }
    }
}

// ── OOS9004 · una palabra que nadie habla ───────────────────────────────────

/// `OOS8002` un piso más arriba, y por el mismo motivo: **una regla que no
/// gobierna nada y un concepto que nadie habla tienen exactamente el mismo
/// aspecto que los que funcionan.**
///
/// No se aplica a un paquete **sin entidades**, que es el caso degenerado —y
/// legítimo— de publicar vocabulario para que otros lo importen.
fn palabras_muertas(
    pkg: &Package,
    conceptos: &BTreeMap<String, Concepto>,
    out: &mut Vec<Diagnostic>,
) {
    if pkg.entities().next().is_none() {
        return;
    }

    // Habla el concepto quien lo mapea **y también quien lo exige**: una
    // interfaz que lo nombra lo ha puesto en circulación. Contarlo como
    // silencio daría dos códigos para una situación —este aquí y `OOS9001` en
    // la entidad que no la satisface—, y este registro emite **un código por
    // síntoma, no por causa**.
    let mut hablados: BTreeSet<String> = mapeos(pkg).into_values().collect();
    for (_, requiere) in exigencias(pkg) {
        hablados.extend(requiere);
    }

    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Property) {
        let Some(q) = d.qname() else { continue };
        if !conceptos.contains_key(&q) || hablados.contains(&q) {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos9004,
                &d.path,
                format!("el concepto `{q}` no lo referencia nada del paquete"),
            )
            .help(
                "un vocabulario crece solo si algo lo habla. Si el concepto es para que otros lo \
                 importen, publícalo en un paquete SIN ENTIDADES: allí la regla no se aplica, \
                 porque esa es exactamente la forma de publicar vocabulario",
            ),
        );
    }
}
