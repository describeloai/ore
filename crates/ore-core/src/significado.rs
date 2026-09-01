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
use std::path::{Path, PathBuf};

/// Un concepto: lo que una propiedad hereda al escribir `is`.
#[derive(Debug, Clone, Default)]
pub struct Concepto {
    pub tipo: Option<String>,
    /// Qué clase de regla exige de quien lo lleve, **categóricamente**.
    ///
    /// Un retículo exige por nivel; un concepto, por ser lo que es. La
    /// regulación clasifica así —el artículo 9 del RGPD enumera categorías y
    /// sus obligaciones se activan en cuanto el dato cae en una, con
    /// independencia de lo sensible que sea en ese contexto—, y sin esto la
    /// exigencia depende de que alguien acertara a etiquetar.
    pub requiere: Vec<String>,
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
        let requiere = d
            .section("requiresGovernance")
            .map(|n| {
                n.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        out.insert(
            q,
            Concepto {
                tipo,
                labels,
                requiere,
            },
        );
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
    let mut out: BTreeMap<String, BTreeSet<String>> =
        exige.keys().map(|i| (i.clone(), BTreeSet::new())).collect();

    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let Some(ps) = e.section("properties") else {
            continue;
        };
        for i in e.section("implements").map(|n| n.items()).unwrap_or(&[]) {
            let Some(declarada) = i.as_str() else {
                continue;
            };
            let Some(suyo) = exige.get(declarada) else {
                continue;
            };
            // La entidad declara `I`; cuenta para toda forma `J` que `I`
            // subsuma, y `J = I` es el caso trivial de la misma regla.
            for (objetivo, pide) in exige.iter().filter(|(_, p)| subsume(suyo, p)) {
                let Some(acc) = out.get_mut(objetivo) else {
                    continue;
                };
                for (k, v) in ps.entries() {
                    let (Some(n), Some(c)) = (k.as_str(), mapeo(v)) else {
                        continue;
                    };
                    if pide.contains(&c) {
                        acc.insert(format!("{qn}.{n}"));
                    }
                }
            }
        }
    }
    out
}

/// `I ⊑ J` — la forma que exige `suyo` satisface también la que exige `otro`.
///
/// **Es una inclusión de conjuntos, y por eso no hay campo `extends`.** Si
/// `J.requires ⊆ I.requires`, toda entidad que satisface `I` satisface `J`:
/// no es una declaración, es un teorema sobre dos documentos, y declararlo
/// sería un segundo sitio donde decirlo con la posibilidad de contradecirlo
/// (**P2**).
///
/// No es una analogía tomada de los lenguajes de programación: es lo que hace
/// la propia disciplina. En OWL, una clase con condiciones **necesarias y
/// suficientes** es una *clase definida* y un razonador computa su lugar en la
/// jerarquía; la asertada se reserva para las primitivas, cuya pertenencia no
/// se puede calcular. Un `Interface` es una clase definida por construcción.
///
/// Y por eso esto no puede fallar y no aparece en ningún registro de errores:
/// **lo derivable no se declara, luego no se puede escribir mal.**
fn subsume(suyo: &[String], otro: &[String]) -> bool {
    otro.iter().all(|c| suyo.contains(c))
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
/// El directorio de cada miembro del workspace: el de su `package.yaml`.
///
/// Los miembros son **directorios**, y no es una convención de este motor: el
/// esquema de `OntologyConfig` declara `workspace.members` como una lista de
/// patrones de ruta con `packages/*` por defecto, y dice que *«el valor por
/// defecto lo aplica el COMPILADOR al normalizar»*. Se derivan de dónde está
/// cada `package.yaml` en vez de expandir el patrón porque el resultado es el
/// mismo y una disposición no estándar —que es justo para lo que existe declarar
/// `members`— sigue funcionando sin leerla.
fn miembros(pkg: &Package) -> Vec<PathBuf> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|d| d.path.parent().map(Path::to_path_buf))
        .collect()
}

/// El miembro que declara un documento: el más largo que es prefijo de su ruta.
///
/// El más largo y no el primero, porque un miembro puede estar dentro de otro y
/// entonces el de dentro es el que manda.
fn miembro_de<'a>(miembros: &'a [PathBuf], doc: &Path) -> Option<&'a Path> {
    miembros
        .iter()
        .filter(|m| doc.starts_with(m))
        .max_by_key(|m| m.components().count())
        .map(PathBuf::as_path)
}

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
    //
    // Y se busca en el WORKSPACE entero, que es la unidad que se compila
    // (`02-ruleset` §2.5). Quien habla un concepto no tiene por qué estar en el
    // paquete que lo publica: **ese es el caso normal.**
    let mut hablados: BTreeSet<String> = mapeos(pkg).into_values().collect();
    for (_, requiere) in exigencias(pkg) {
        hablados.extend(requiere);
    }

    // La excepción de `02-property` §6.1 es de un **paquete** sin entidades, y
    // un workspace no es un paquete: tiene miembros. Evaluarla sobre el árbol
    // entero convertía la forma de publicar vocabulario en un error — un
    // paquete de quince conceptos vendorizado junto a uno que habla dos dejaba
    // trece `OOS9004`, y la ayuda del diagnóstico recomendaba exactamente lo
    // que ya se había hecho. Se midió con el primer vocabulario de verdad.
    let miembros = miembros(pkg);
    let con_entidades: BTreeSet<&Path> = pkg
        .entities()
        .filter_map(|e| miembro_de(&miembros, &e.path))
        .collect();

    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Property) {
        let Some(q) = d.qname() else { continue };
        if !conceptos.contains_key(&q) || hablados.contains(&q) {
            continue;
        }
        // Un concepto de un miembro que no modela nada es vocabulario
        // publicado: quien lo importa es quien lo habla. Uno que no está en
        // ningún miembro —suelto en la raíz del workspace— se juzga contra el
        // workspace, que es el único paquete al que pertenece.
        if let Some(m) = miembro_de(&miembros, &d.path)
            && !con_entidades.contains(m)
        {
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
