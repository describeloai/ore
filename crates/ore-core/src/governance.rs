//! Gobierno — la familia `OOS8xxx`.
//!
//! Ni [`flow`](crate::flow) ni [`effect`](crate::effect): el tercero. Aquel
//! gobierna **lo que se puede saber**, el otro **lo que se puede causar**, y
//! este **qué debe sostenerse y quién responde**.
//!
//! > `∀x . L(x) ⊒ n ⟹ ∃r . x ∈ objetivo(r)`
//!
//! # Lo que hace distinta a esta fase
//!
//! Las otras dos razonan documento a documento: una etiqueta mal puesta, una
//! función que no alcanza su destino. Esta razona sobre el paquete **entero**,
//! y su error central —`OOS8001`— no señala una línea equivocada sino **una
//! línea que nadie escribió**. Es el `OOS4001` de este plano: no hay diff donde
//! mirarlo, no lo encuentra un `grep` y no lo encuentra una revisión de código.
//!
//! Por eso corre la última. Calcular una ausencia sobre etiquetas que todavía
//! podrían estar mal produciría el peor diagnóstico posible: señalar algo que
//! no existe por culpa de algo que sí.
//!
//! # La pieza que lo hace gratis
//!
//! No hay lenguaje de objetivos. Un retículo declarado para **comparar** dos
//! elementos —`L ⊑ C`— nombra, leído en la otra dirección, un **conjunto**:
//! `{x : L(x) ⊒ n}`. Todo este módulo es esa lectura, y es decidible al
//! compilar por la misma razón que lo es la regla de flujo.
//!
//! # Lo que esta fase todavía no hace
//!
//! `OOS8001` demuestra que **existe** una regla, no que sea **la adecuada**:
//! una política que permite todo cubre igual que una que no permite nada. Aquí
//! se cierran los tres huecos baratos —lo ilegible, lo que no puede fallar, lo
//! que no puede fallar al compilar— y no el caro. Se dice para que nadie lo
//! deduzca de que los casos pasan.

use crate::code::Code;
use crate::diag::{Diagnostic, Pos};
use crate::document::Kind;
use crate::flow::{self, Axis, Lattice};
use crate::link::{Loaded, Package};
use crate::normalize;
use std::collections::{BTreeMap, BTreeSet};

/// Los desclasificadores admisibles **como máscara**.
///
/// Subconjunto del vocabulario cerrado de `04-flow` §5, y falta uno a
/// propósito: `promote` **sube** por un retículo de ciclo de vida y una máscara
/// tiene que bajar. Excluirlo no es retirarlo — sigue siendo un desclasificador
/// para su propio uso.
const MASCARAS: &[&str] = &["mask", "tokenize", "redact", "aggregate"];

/// Un objetivo: retículo → nivel mínimo. La conjunción de sus entradas.
type Objetivo = BTreeMap<String, String>;

/// Etiquetas efectivas: `entidad.propiedad` → retículo → nivel.
type Props = BTreeMap<String, BTreeMap<String, String>>;

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let reglas: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Ruleset)
        .collect();
    let lat = flow::lattices(pkg);
    let mut out = Vec::new();

    // 1 · Que los objetivos apunten a algo que existe y del eje correcto.
    //     Antes que nada: seleccionar sobre un retículo que no está o sobre el
    //     eje equivocado no produce una selección mala — produce una vacía, y
    //     el error saldría como `OOS8002`, que es el diagnóstico equivocado.
    for r in &reglas {
        objetivos_validos(r, &lat, &mut out);
    }
    if !out.is_empty() {
        return out;
    }

    // 2 · La selección, de la que depende todo lo demás.
    let props = flow::efectivas(pkg, &lat);
    let mut sel: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &reglas {
        let mut union = BTreeSet::new();
        for (obj, pos) in objetivos(r) {
            let casan = selecciona(&obj, &props, &lat);
            if casan.is_empty() {
                out.push(vacio(r, &obj, pos));
            }
            union.extend(casan);
        }
        sel.insert(r.qname().unwrap_or_default(), union);
    }
    if !out.is_empty() {
        return out;
    }

    // 3 · Las tres piezas, cada una contra su objetivo.
    let fuentes = fuentes_por_entidad(pkg);
    for r in &reglas {
        let q = r.qname().unwrap_or_default();
        let seleccionadas = sel.get(&q).cloned().unwrap_or_default();
        mascaras(r, &lat, &mut out);
        aserciones(r, &seleccionadas, &fuentes, &mut out);
        deberes(pkg, r, &mut out);
    }
    if !out.is_empty() {
        return out;
    }

    // 4 · Y la cobertura, que es una diferencia de conjuntos.
    cobertura(pkg, &lat, &props, &reglas, &sel, &mut out);
    out
}

// ── Los objetivos ───────────────────────────────────────────────────────────

/// Lee `spec.targets`, con la posición de cada uno para poder señalarlo.
fn objetivos(r: &Loaded) -> Vec<(Objetivo, Pos)> {
    r.section("targets")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|t| {
            let (_, mapa) = t.get("atLeast")?;
            let obj: Objetivo = mapa
                .entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect();
            Some((obj, mapa.pos()))
        })
        .collect()
}

/// `OOS4003` · `OOS8006` — el retículo existe, el nivel existe, y el eje es el
/// que gobierna.
fn objetivos_validos(r: &Loaded, lat: &BTreeMap<String, Lattice>, out: &mut Vec<Diagnostic>) {
    for (obj, pos) in objetivos(r) {
        for (ret, nivel) in &obj {
            let Some(l) = lat.get(ret) else {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &r.path,
                        format!("`{ret}` no es un retículo declarado"),
                    )
                    .at(pos)
                    .help(
                        "un objetivo se escribe sobre un retículo que ya existe: no hay \
                         lenguaje de objetivos, hay el orden del retículo leído al revés",
                    ),
                );
                continue;
            };
            if l.index(nivel).is_none() {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &r.path,
                        format!("`{nivel}` no es un nivel de `{ret}`"),
                    )
                    .at(pos)
                    .help(format!("los niveles son: {}", l.levels.join(", "))),
                );
                continue;
            }
            if l.axis == Axis::Integrity {
                out.push(
                    Diagnostic::new(
                        Code::Oos8006,
                        &r.path,
                        format!("`{ret}` es de eje `integrity` y un objetivo no puede apuntarlo"),
                    )
                    .at(pos)
                    .help(
                        "la monotonía del gobierno corre al revés en ese eje: en \
                         confidencialidad se gobierna hacia arriba —más sensible, más \
                         gobierno— y en integridad se gobernaría hacia abajo —menos fiable, \
                         más gobierno—, así que `atLeast` selecciona justo lo contrario de lo \
                         que hace falta. Y antes de añadir `atMost` hay una pregunta: el \
                         remedio natural de la baja integridad es un endoso, que es asunto de \
                         `Function` y no de un `Ruleset`",
                    ),
                );
            }
        }
    }
}

/// Las propiedades que casan con un objetivo. Y dentro del mapa, con la
/// conjunción: la propiedad debe satisfacer **todas** las entradas.
fn selecciona(obj: &Objetivo, props: &Props, lat: &BTreeMap<String, Lattice>) -> BTreeSet<String> {
    props
        .iter()
        .filter(|(_, etiquetas)| {
            obj.iter().all(|(ret, piso)| {
                etiquetas
                    .get(ret)
                    .and_then(|nivel| lat.get(ret)?.ge(nivel, piso))
                    .unwrap_or(false)
            })
        })
        .map(|(q, _)| q.clone())
        .collect()
}

/// `OOS8002` — un objetivo que no casa con nada.
fn vacio(r: &Loaded, obj: &Objetivo, pos: Pos) -> Diagnostic {
    let escrito = obj
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    Diagnostic::new(
        Code::Oos8002,
        &r.path,
        format!("el objetivo `{escrito}` no casa con ninguna propiedad"),
    )
    .at(pos)
    .help(
        "casi nunca significa «no hay nada así»: significa que un nivel está mal escrito, o \
         que el retículo cambió y la regla se quedó apuntando a un nombre que ya no existe. \
         Y una regla que no gobierna nada tiene exactamente el mismo aspecto que una que \
         funciona — es el único fallo de este documento que no produce ningún síntoma",
    )
}

// ── Las máscaras · OOS8003 ──────────────────────────────────────────────────

fn mascaras(r: &Loaded, lat: &BTreeMap<String, Lattice>, out: &mut Vec<Diagnostic>) {
    let objs = objetivos(r);
    for m in r.section("masks").map(|n| n.items()).unwrap_or(&[]) {
        let Some(nombre) = m.get("declassifier").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        if !MASCARAS.contains(&nombre) {
            out.push(
                Diagnostic::new(
                    Code::Oos4006,
                    &r.path,
                    format!("`{nombre}` no es un desclasificador admisible como máscara"),
                )
                .at(m.pos())
                .help(format!(
                    "son: {}. `promote` queda fuera porque SUBE por un retículo de ciclo de \
                     vida, y una máscara tiene que bajar",
                    MASCARAS.join(", ")
                )),
            );
            continue;
        }

        // `to` sobre un `redact` es un campo derivable —redactar deja el valor
        // en el ínfimo del retículo, siempre— y el error es de forma, no de
        // gobierno: `OOS1004`. Es la lección de `OOS7010`, y por eso `OOS8003`
        // se quedó con una sola causa.
        let to = m.get("to").map(|(_, v)| v);
        match (nombre, to) {
            ("redact", Some(v)) => {
                out.push(
                    Diagnostic::new(Code::Oos1004, &r.path, "`redact` no admite `to`")
                        .at(v.pos())
                        .help(
                            "redactar hace desaparecer el valor, luego su salida es siempre el \
                             ínfimo del retículo. Un campo derivable no es declarable (P2)",
                        ),
                );
                continue;
            }
            ("redact", None) => continue,
            (_, None) => {
                out.push(
                    Diagnostic::new(
                        Code::Oos1004,
                        &r.path,
                        format!("la máscara `{nombre}` no declara `to`"),
                    )
                    .at(m.pos())
                    .help(
                        "declarar el nivel resultante es lo que hace comprobable el descenso: \
                         sin él la máscara es una función opaca, que es exactamente lo que \
                         hace inauditables a las de un catálogo",
                    ),
                );
                continue;
            }
            _ => {}
        }
        let Some(destino) = to else { continue };

        for (ret, nivel) in destino
            .entries()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str()?, v.as_str()?)))
        {
            let Some(l) = lat.get(ret) else {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &r.path,
                        format!("`{ret}` no es un retículo declarado"),
                    )
                    .at(destino.pos()),
                );
                continue;
            };
            let Some(salida) = l.index(nivel) else {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &r.path,
                        format!("`{nivel}` no es un nivel de `{ret}`"),
                    )
                    .at(destino.pos()),
                );
                continue;
            };

            // El suelo más bajo que algún objetivo impone sobre este retículo.
            // Si ninguno lo acota, el descenso no es demostrable, que a efectos
            // de gobierno es lo mismo que no bajar.
            let pisos: Vec<usize> = objs
                .iter()
                .filter_map(|(o, _)| o.get(ret))
                .filter_map(|p| l.index(p))
                .collect();
            let Some(&piso) = pisos.iter().min() else {
                out.push(
                    Diagnostic::new(
                        Code::Oos8003,
                        &r.path,
                        format!(
                            "la máscara declara bajar `{ret}` y ningún objetivo de este \
                             documento acota ese retículo"
                        ),
                    )
                    .at(destino.pos())
                    .help(
                        "el descenso se comprueba contra el suelo del objetivo. Sin suelo no \
                         hay nada contra qué compararlo, y una máscara que no baja \
                         demostrablemente no es una salvaguarda",
                    ),
                );
                continue;
            };
            if salida >= piso {
                out.push(
                    Diagnostic::new(
                        Code::Oos8003,
                        &r.path,
                        format!(
                            "`{nombre}` produce `{ret}: {nivel}` y el objetivo ya selecciona \
                             desde `{}`",
                            l.levels[piso]
                        ),
                    )
                    .at(destino.pos())
                    .help(
                        "un desclasificador que no baja no es una salvaguarda: es teatro con \
                         coste de cómputo. La comprobación es local —dos niveles declarados— \
                         porque ninguna propiedad seleccionada puede estar por debajo del \
                         suelo que la seleccionó",
                    ),
                );
            }
        }
    }
}

// ── Las aserciones · OOS8005 ────────────────────────────────────────────────

/// Entidad cualificada → las fuentes físicas a las que se enlaza.
fn fuentes_por_entidad(pkg: &Package) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for b in pkg.docs.iter().filter(|d| d.kind == Kind::Binding) {
        let Some(e) = b.section("targetEntity").and_then(|n| n.as_str()) else {
            continue;
        };
        let Some(ds) = b.section("datasourceRef").and_then(|n| n.as_str()) else {
            continue;
        };
        out.entry(normalize::qualify(
            e,
            b.meta("namespace").and_then(|n| n.as_str()),
        ))
        .or_default()
        .insert(ds.to_string());
    }
    out
}

fn aserciones(
    r: &Loaded,
    seleccionadas: &BTreeSet<String>,
    fuentes: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Diagnostic>,
) {
    for a in r.section("assertions").map(|n| n.items()).unwrap_or(&[]) {
        if a.get("type").and_then(|(_, v)| v.as_str()) != Some("sql") {
            continue;
        }
        let mut usadas: BTreeSet<&String> = BTreeSet::new();
        for p in seleccionadas {
            let Some((entidad, _)) = p.rsplit_once('.') else {
                continue;
            };
            if let Some(ds) = fuentes.get(entidad) {
                usadas.extend(ds);
            }
        }
        if usadas.len() <= 1 {
            continue;
        }
        let id = a
            .get("id")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("<sin id>");
        out.push(
            Diagnostic::new(
                Code::Oos8005,
                &r.path,
                format!(
                    "la aserción `sql` `{id}` apunta a propiedades de {} fuentes: {}",
                    usadas.len(),
                    usadas
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .at(a.pos())
            .help(
                "una regla `sql` está atada a un dialecto y el dialecto solo se conoce donde \
                 se declara la fuente: no es portable entre fuentes, y la limitación es de \
                 ODCS, no nuestra. La misma regla escrita como `library` no tiene el problema \
                 porque no tiene dialecto",
            ),
        );
    }
}

// ── Los deberes · OOS2001 ───────────────────────────────────────────────────

fn deberes(pkg: &Package, r: &Loaded, out: &mut Vec<Diagnostic>) {
    let ns = r.meta("namespace").and_then(|n| n.as_str());
    for d in r.section("duties").map(|n| n.items()).unwrap_or(&[]) {
        let Some((_, nodo)) = d.get("call") else {
            continue;
        };
        let Some(nombre) = nodo.as_str() else {
            continue;
        };
        let q = normalize::qualify(nombre, ns);
        let resuelve = pkg
            .docs
            .iter()
            .filter(|x| x.kind == Kind::Function)
            .any(|f| f.qname().as_deref() == Some(q.as_str()));
        if resuelve {
            continue;
        }
        let otro = pkg
            .docs
            .iter()
            .find(|x| x.qname().as_deref() == Some(q.as_str()))
            .map(|x| x.kind.as_str());
        out.push(
            Diagnostic::new(
                Code::Oos2001,
                &r.path,
                match otro {
                    Some(k) => format!("`{q}` es un `{k}`, no una `Function`"),
                    None => format!("`{q}` no resuelve a ninguna `Function`"),
                },
            )
            .at(nodo.pos())
            .help(
                "un deber DEBE nombrar una `Function`, y esa es la única restricción que hace \
                 falta desde el primer día: XACML murió de obligaciones que nombraban deberes \
                 que ningún runtime sabía ejecutar. Una referencia a una función declarada \
                 trae su integridad computada, sus precondiciones y su endoso; un deber en \
                 prosa no trae nada",
            ),
        );
    }
}

// ── La cobertura · OOS8001 ──────────────────────────────────────────────────

/// ¿Cuenta este `Ruleset` para la cobertura de lo que selecciona?
///
/// > Solo cuenta lo que el compilador **puede leer** y lo que **puede fallar**.
///
/// Una regla, tres consecuencias: un aviso no cuenta porque es, por definición,
/// «lo vimos y no paramos nada»; `text` y `custom` no cuentan porque se
/// transportan sin interpretar; y un deber no cuenta porque su incumplimiento
/// es un hecho temporal, no algo que pueda fallar al compilar.
///
/// Una máscara sí: el compilador la lee, `OOS8003` la puede rechazar, y una
/// propiedad enmascarada está gobernada.
fn cuenta(r: &Loaded) -> bool {
    let hay_mascara = !r
        .section("masks")
        .map(|n| n.items())
        .unwrap_or(&[])
        .is_empty();
    let hay_asercion = r
        .section("assertions")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .any(|a| {
            let tipo = a
                .get("type")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("library");
            let severidad = a
                .get("severity")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("error");
            asercion_cuenta(tipo, severidad)
        });
    hay_mascara || hay_asercion
}

/// El criterio, aislado para poder probarlo: **legible y capaz de fallar**.
///
/// `text` y `custom` se transportan sin interpretar, así que el compilador no
/// sabe qué afirman. Y un aviso no puede fallar. Sin esta función, `OOS8001` se
/// satisface con una aserción que no para nada y la cobertura pasa a medir que
/// alguien escribió un fichero — que es el modo de fallo que aparece en cuanto
/// alguien tiene prisa por poner verde una compilación.
fn asercion_cuenta(tipo: &str, severidad: &str) -> bool {
    matches!(tipo, "library" | "sql") && severidad == "error"
}

fn cobertura(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    props: &Props,
    reglas: &[&Loaded],
    sel: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Diagnostic>,
) {
    let cubren: Vec<&BTreeSet<String>> = reglas
        .iter()
        .filter(|r| cuenta(r))
        .filter_map(|r| sel.get(&r.qname().unwrap_or_default()))
        .collect();

    for (prop, etiquetas) in props {
        for (ret, nivel) in etiquetas {
            let Some(l) = lat.get(ret) else { continue };
            let Some(piso) = l.requires_governance.as_deref() else {
                continue;
            };
            if l.ge(nivel, piso) != Some(true) {
                continue;
            }
            if cubren.iter().any(|s| s.contains(prop)) {
                continue;
            }
            let (fichero, pos) = donde(pkg, prop);
            let mut d = Diagnostic::new(
                Code::Oos8001,
                fichero,
                format!("`{prop}` está clasificada `{ret}: {nivel}` y ninguna regla la cubre"),
            )
            .help(format!(
                "`{ret}` declara `requiresGovernance: {piso}`, así que todo lo clasificado ahí \
                 o por encima tiene que estar cubierto. Añade un `Ruleset` cuyo objetivo lo \
                 alcance —un suelo más bajo también sirve, porque el gobierno es monótono—. Y \
                 ojo con la salida barata: una aserción `severity: warning` NO cuenta, porque \
                 un aviso no descarga la obligación de gobernar",
            ));
            if let Some(p) = pos {
                d = d.at(p);
            }
            out.push(d);
        }
    }
}

/// Dónde señalar una propiedad. El fichero de su entidad, y la propiedad si se
/// encuentra — que es lo más cerca que se puede estar de un defecto que no está
/// escrito en ninguna parte.
fn donde(pkg: &Package, prop: &str) -> (std::path::PathBuf, Option<Pos>) {
    let Some((entidad, nombre)) = prop.rsplit_once('.') else {
        return (pkg.root.clone(), None);
    };
    let Some(e) = pkg.entity(entidad) else {
        return (pkg.root.clone(), None);
    };
    let pos = e
        .section("properties")
        .and_then(|n| n.get(nombre))
        .map(|(k, _)| k.pos());
    (e.path.clone(), pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reticulo(qname: &str, niveles: &[&str]) -> (String, Lattice) {
        (
            qname.to_string(),
            Lattice {
                qname: qname.to_string(),
                levels: niveles.iter().map(|s| s.to_string()).collect(),
                axis: Axis::Confidentiality,
                requires_governance: None,
            },
        )
    }

    fn props(entradas: &[(&str, &[(&str, &str)])]) -> Props {
        entradas
            .iter()
            .map(|(p, ets)| {
                (
                    p.to_string(),
                    ets.iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    /// La propiedad de la que sale que solo haga falta un operador: si una
    /// regla se aplica a `high`, **tiene** que aplicarse a `critical`. Una que
    /// dejara de aplicarse cuando el dato es más sensible sería un defecto.
    #[test]
    fn el_gobierno_es_monotono() {
        let lat = BTreeMap::from([reticulo(
            "gdpr.sensitivity",
            &["none", "low", "medium", "high", "critical"],
        )]);
        let p = props(&[
            ("E.baja", &[("gdpr.sensitivity", "low")]),
            ("E.alta", &[("gdpr.sensitivity", "high")]),
            ("E.critica", &[("gdpr.sensitivity", "critical")]),
        ]);
        let obj = Objetivo::from([("gdpr.sensitivity".into(), "medium".into())]);
        let sel = selecciona(&obj, &p, &lat);
        assert!(sel.contains("E.alta") && sel.contains("E.critica"));
        assert!(!sel.contains("E.baja"));
    }

    /// Dentro de un objetivo, las entradas se conjugan. Es la semántica de
    /// `matchLabels`, y la razón de que sea un mapa y no una lista: una lista
    /// admitiría dos suelos para el mismo retículo, que no significa nada.
    #[test]
    fn el_objetivo_conjuga_sus_entradas() {
        let lat = BTreeMap::from([
            reticulo("gdpr.sensitivity", &["none", "medium", "high"]),
            reticulo("acme.residency", &["global", "eu_only"]),
        ]);
        let p = props(&[
            (
                "E.ambas",
                &[("gdpr.sensitivity", "high"), ("acme.residency", "eu_only")],
            ),
            ("E.solo_una", &[("gdpr.sensitivity", "high")]),
        ]);
        let obj = Objetivo::from([
            ("gdpr.sensitivity".into(), "high".into()),
            ("acme.residency".into(), "eu_only".into()),
        ]);
        let sel = selecciona(&obj, &p, &lat);
        assert!(sel.contains("E.ambas"));
        assert!(!sel.contains("E.solo_una"));
    }

    /// Una etiqueta que no pertenece al retículo NO selecciona. `ge` devuelve
    /// `None` y no `false` a propósito, y aquí se comprueba que el `None` se
    /// trate como «no casa» en vez de propagarse como un acierto.
    #[test]
    fn un_nivel_desconocido_no_selecciona() {
        let lat = BTreeMap::from([reticulo("gdpr.sensitivity", &["none", "high"])]);
        let p = props(&[("E.p", &[("gdpr.sensitivity", "inventado")])]);
        let obj = Objetivo::from([("gdpr.sensitivity".into(), "none".into())]);
        assert!(selecciona(&obj, &p, &lat).is_empty());
    }

    /// La regla que impide que la cobertura se vuelva decorativa. Un solo
    /// carácter —`warning` en vez de `error`— separa gobernar de aparentarlo.
    #[test]
    fn un_aviso_no_descarga_la_obligacion() {
        assert!(asercion_cuenta("library", "error"));
        assert!(asercion_cuenta("sql", "error"));
        assert!(!asercion_cuenta("library", "warning"));
        // Se transportan sin interpretar: el compilador no sabe qué afirman.
        assert!(!asercion_cuenta("text", "error"));
        assert!(!asercion_cuenta("custom", "error"));
    }
}
