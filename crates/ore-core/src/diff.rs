//! Compatibilidad entre dos versiones de un paquete — la familia `OOS5xxx`.
//!
//! # Las dos direcciones
//!
//! En una librería, añadir capacidad es seguro y quitarla rompe. En una
//! ontología gobernada hay **dos direcciones opuestas y ambas rompen**:
//!
//! ```text
//!     ◀── restringir ──────────────────── relajar ──▶
//!     rompe al CONSUMIDOR                 rompe la GOBERNANZA
//!     (deja de poder leer)                (concede acceso en silencio)
//! ```
//!
//! Relajar una política no rompe a ningún consumidor y es el cambio más
//! peligroso que existe aquí: nadie recibe un error, simplemente más gente ve
//! más cosas. Un versionado que solo mirase al consumidor lo dejaría pasar como
//! retrocompatible.
//!
//! Por eso un cambio no tiene *un* veredicto: se evalúa contra los cuatro ejes
//! por separado, y **puede ser compatible en uno y rompedor en otro**. Elevar la
//! etiqueta de una propiedad mejora la gobernanza (`POLICY` compatible) y rompe
//! a quien la leía (`CONSUMER` rompedor). Sin ejes separados habría que elegir
//! uno y mentir en el otro.
//!
//! # La consecuencia
//!
//! De la clasificación sale el salto de versión exigido, y `OOS5021` falla
//! cuando el declarado no llega. **La versión deja de ser una afirmación y pasa
//! a ser una comprobación.**

use crate::cedar::{self, Effect};
use crate::code::Code;
use crate::flow::{self, Lattice};
use crate::json::Json;
use crate::link::{Loaded, Package};
use crate::parse::Node;
use std::collections::{BTreeMap, BTreeSet};

// ── Ejes y veredictos ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    Consumer,
    Policy,
    Index,
    Package,
}

impl Axis {
    pub const ALL: &'static [Axis] = &[Axis::Consumer, Axis::Policy, Axis::Index, Axis::Package];

    pub const fn as_str(self) -> &'static str {
        match self {
            Axis::Consumer => "CONSUMER",
            Axis::Policy => "POLICY",
            Axis::Index => "INDEX",
            Axis::Package => "PACKAGE",
        }
    }

    /// Los ejes que obligan a subir la versión **mayor**. `INDEX` no está: un
    /// índice que hay que reconstruir no rompe a nadie, solo cuesta.
    const fn fuerza_mayor(self) -> bool {
        matches!(self, Axis::Consumer | Axis::Policy | Axis::Package)
    }
}

#[derive(Debug, Clone)]
pub struct Change {
    pub code: Code,
    pub axis: Axis,
    campos: Vec<(&'static str, Json)>,
}

impl Change {
    fn new(code: Code, axis: Axis) -> Self {
        Change {
            code,
            axis,
            campos: Vec::new(),
        }
    }

    fn with(mut self, k: &'static str, v: Json) -> Self {
        self.campos.push((k, v));
        self
    }

    fn sujeto(self, s: impl Into<String>) -> Self {
        self.with("subject", Json::s(s))
    }

    fn de_a(self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.with("from", Json::s(from)).with("to", Json::s(to))
    }

    fn json(&self) -> Json {
        let mut m: BTreeMap<String, Json> = BTreeMap::new();
        m.insert("code".into(), Json::s(self.code.as_str()));
        m.insert("axis".into(), Json::s(self.axis.as_str()));
        for (k, v) in &self.campos {
            m.insert((*k).to_string(), v.clone());
        }
        Json::Obj(m)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    const fn as_str(self) -> &'static str {
        match self {
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        }
    }
}

pub struct Report {
    pub changes: Vec<Change>,
    pub required_bump: Bump,
}

impl Report {
    /// El eje está roto si tiene algún cambio. Se emiten los cuatro siempre:
    /// «compatible en POLICY» es información, no ausencia de información.
    pub fn verdicts(&self) -> BTreeMap<&'static str, &'static str> {
        Axis::ALL
            .iter()
            .map(|a| {
                let roto = self.changes.iter().any(|c| c.axis == *a);
                (a.as_str(), if roto { "breaking" } else { "compatible" })
            })
            .collect()
    }

    pub fn json(&self) -> Json {
        Json::obj([
            (
                "changes",
                Json::Arr(self.changes.iter().map(Change::json).collect()),
            ),
            (
                "verdicts",
                Json::Obj(
                    self.verdicts()
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), Json::s(v)))
                        .collect(),
                ),
            ),
            ("requiredBump", Json::s(self.required_bump.as_str())),
        ])
    }
}

// ── Semver ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
struct Version(u64, u64, u64);

impl Version {
    fn parse(s: &str) -> Option<Version> {
        let nucleo = s.split(['-', '+']).next()?;
        let mut it = nucleo.split('.');
        let mut n = || it.next()?.parse::<u64>().ok();
        let v = Version(n()?, n()?, n()?);
        it.next().is_none().then_some(v)
    }

    /// La versión mínima que este salto exige a partir de la anterior.
    fn tras(self, bump: Bump) -> Version {
        match bump {
            Bump::Major => Version(self.0 + 1, 0, 0),
            Bump::Minor => Version(self.0, self.1 + 1, 0),
            Bump::Patch => Version(self.0, self.1, self.2 + 1),
        }
    }

    fn texto(self) -> String {
        format!("{}.{}.{}", self.0, self.1, self.2)
    }
}

// ── La forma comparable de un paquete ───────────────────────────────────────
//
// Comparar dos árboles YAML directamente confundiría reordenar con cambiar. Lo
// que se compara es la forma: identidades y valores, sin orden ni presentación.

#[derive(Default)]
struct Prop {
    ty: String,
    enum_values: Option<Vec<String>>,
    labels: BTreeMap<String, String>,
    required: bool,
}

#[derive(Default)]
struct Rel {
    target: String,
    cardinality: String,
    required: bool,
}

#[derive(Default)]
struct Ent {
    labels: BTreeMap<String, String>,
    primary_key: Vec<String>,
    props: BTreeMap<String, Prop>,
    relations: BTreeMap<String, Rel>,
    /// Nombres antiguos declarados en `moved` o `reserved`: un nombre anunciado
    /// no desaparece en silencio.
    anunciados: BTreeSet<String>,
}

#[derive(Default)]
struct Bind {
    source: String,
    mode: String,
}

#[derive(Default)]
struct Shape {
    version: Version,
    notice_period: Option<String>,
    entities: BTreeMap<String, Ent>,
    /// conducto → retículo → nivel
    conduits: BTreeMap<String, BTreeMap<String, String>>,
    /// entidad destino → binding
    bindings: BTreeMap<String, Bind>,
    policies: BTreeMap<String, cedar::Policy>,
    lattices: BTreeMap<String, Lattice>,
    /// Propiedad → clases de gobierno que la cubren **de hecho**.
    ///
    /// No son las reglas: es su efecto. Comparar sintaxis diría que un
    /// `Ruleset` cambió; comparar esto dice **qué propiedad se quedó sin
    /// gobierno**, que es la pregunta que el eje `POLICY` hace.
    gobernadas: BTreeMap<String, BTreeSet<&'static str>>,
    /// Retículo → nivel → clases exigidas desde ese nivel.
    exigencias: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// Lo que exige cada **concepto**, que es el tercer origen de exigencia y
    /// se rompe igual que los otros dos: quitarle una naturaleza a un concepto
    /// publicado desgobierna, sin tocarlos, a todos los paquetes que lo mapean.
    exigencias_de_concepto: BTreeMap<String, Vec<String>>,
    /// Los conceptos, **como propiedades**.
    ///
    /// No es una comodidad de implementación: es la tesis de `01-significado`
    /// §3 escrita en un tipo. Un `Property` declara `type`, `labels` y `enum`
    /// —exactamente lo que declara una propiedad— y por eso sus cambios son los
    /// mismos cambios y se clasifican con los mismos códigos, sin inventar
    /// ninguno.
    conceptos: BTreeMap<String, Prop>,
    /// Las formas: `interfaz` → los conceptos que exige.
    interfaces: BTreeMap<String, Vec<String>>,
    /// v1alpha2 · la superficie de escritura gobernada.
    funciones: BTreeMap<String, Funcion>,
    /// v1alpha2 · `resolución` → `estrategia` → su umbral.
    umbrales: BTreeMap<String, BTreeMap<String, String>>,
    /// v1alpha3 · lo que un `Ruleset` **dice**, no solo lo que cubre.
    ///
    /// `gobernadas` ya veía su **efecto** —qué naturaleza cubre cada
    /// propiedad—, y con eso una regla que desaparece salta. Lo que no se veía
    /// es una regla que **sigue ahí y ha dejado de significar algo**: la
    /// cobertura no cambia, así que `OOS8001` sigue satisfecho y `OOS5023` no
    /// dice nada. Es el modo de fallo que `01-gobierno` §6.2 describe —*una
    /// política que permite todo cubre igual que una que no permite nada*— y
    /// entre dos versiones **sí** es computable: no hace falta saber si un
    /// umbral es el correcto, solo que se ha aflojado.
    reglas: BTreeMap<String, Regla>,
}

#[derive(Default)]
struct Funcion {
    input: Prop,
    output: Prop,
    preconditions: BTreeSet<String>,
    endorsements: BTreeSet<String>,
}

#[derive(Default)]
struct Regla {
    /// `id` → sus cotas, por operador.
    assertions: BTreeMap<String, BTreeMap<String, String>>,
    /// `id` → `minGroupSize`, cuando lo declara.
    masks: BTreeMap<String, Option<String>>,
    duties: BTreeSet<String>,
}

/// Los operadores cuyo **aflojamiento tiene dirección definida**.
///
/// `mustBe` y `mustNotBe` no están, y su ausencia es la decisión difícil de
/// esta comparación: son igualdades, no cotas. Pasar de `mustBe 0` a
/// `mustBe 999` no es *más flojo*, es **otra cosa**, y llamarlo relajación
/// sería inventar una dirección que el operador no tiene.
const COTAS: &[(&str, bool)] = &[
    // (operador, aflojar es AUMENTAR el valor)
    ("mustBeLessThan", true),
    ("mustBeLessOrEqualTo", true),
    ("mustBeGreaterThan", false),
    ("mustBeGreaterOrEqualTo", false),
];

fn cadena(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
}

fn etiquetas(n: &Node) -> BTreeMap<String, String> {
    n.get("labels")
        .map(|(_, l)| {
            l.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn lista(n: &Node) -> Vec<String> {
    n.items()
        .iter()
        .filter_map(|i| i.as_str().map(String::from))
        .collect()
}

fn shape(pkg: &Package) -> Shape {
    let lat = flow::lattices(pkg);
    let mut s = Shape {
        exigencias: lat
            .iter()
            .map(|(q, l)| (q.clone(), l.requires_governance.clone()))
            .filter(|(_, r)| !r.is_empty())
            .collect(),
        exigencias_de_concepto: crate::significado::conceptos(pkg)
            .into_iter()
            .map(|(q, c)| (q, c.requiere))
            .filter(|(_, r)| !r.is_empty())
            .collect(),
        gobernadas: crate::governance::cobertura_efectiva(pkg),
        lattices: lat,
        ..Default::default()
    };

    for d in &pkg.docs {
        match d.kind {
            crate::document::Kind::Function => {
                if let Some(qn) = d.qname() {
                    let ids = |sec: &str, campos: &[&str]| -> BTreeSet<String> {
                        d.section(sec)
                            .map(|n| n.items())
                            .unwrap_or(&[])
                            .iter()
                            .filter_map(|i| {
                                let partes: Vec<String> =
                                    campos.iter().filter_map(|c| cadena(i, c)).collect();
                                (!partes.is_empty()).then(|| partes.join("/"))
                            })
                            .collect()
                    };
                    let tipo = |sec: &str| {
                        d.section(sec)
                            .map(|n| Prop {
                                ty: n
                                    .get("type")
                                    .and_then(|(_, v)| v.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                                ..Default::default()
                            })
                            .unwrap_or_default()
                    };
                    s.funciones.insert(
                        qn,
                        Funcion {
                            input: tipo("input"),
                            output: tipo("output"),
                            preconditions: ids("preconditions", &["id"]),
                            // El endoso se identifica por QUIÉN y QUÉ atesta:
                            // cambiar el endosante de una atestación es perder
                            // la que había y ganar otra.
                            endorsements: ids("endorsements", &["endorser", "attestation"]),
                        },
                    );
                }
            }
            crate::document::Kind::Resolution => {
                if let Some(qn) = d.qname() {
                    let mut us = BTreeMap::new();
                    for (i, e) in d
                        .section("strategies")
                        .map(|n| n.items())
                        .unwrap_or(&[])
                        .iter()
                        .enumerate()
                    {
                        // Por `id` cuando lo hay; si no, por posición — que en
                        // una SECUENCIA es una identidad legítima.
                        let id = cadena(e, "id").unwrap_or_else(|| i.to_string());
                        if let Some(t) = cadena(e, "threshold") {
                            us.insert(id, t);
                        }
                    }
                    // Se registra aunque no haya umbrales: si no, una
                    // `Resolution` determinista que DESAPARECE sería
                    // indistinguible de una que nunca estuvo.
                    s.umbrales.insert(qn, us);
                }
            }
            crate::document::Kind::Ruleset => {
                if let Some(qn) = d.qname() {
                    let mut r = Regla::default();
                    for a in d.section("assertions").map(|n| n.items()).unwrap_or(&[]) {
                        let Some(id) = cadena(a, "id") else { continue };
                        let cotas = COTAS
                            .iter()
                            .filter_map(|(op, _)| cadena(a, op).map(|v| (op.to_string(), v)))
                            .collect();
                        r.assertions.insert(id, cotas);
                    }
                    for m in d.section("masks").map(|n| n.items()).unwrap_or(&[]) {
                        let Some(id) = cadena(m, "id") else { continue };
                        r.masks.insert(id, cadena(m, "minGroupSize"));
                    }
                    r.duties = d
                        .section("duties")
                        .map(|n| n.items())
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|x| cadena(x, "call"))
                        .collect();
                    s.reglas.insert(qn, r);
                }
            }
            crate::document::Kind::Property => {
                if let Some(qn) = d.qname()
                    && let Some(spec) = d.root.get("spec").map(|(_, v)| v)
                {
                    s.conceptos.insert(
                        qn,
                        Prop {
                            ty: cadena(spec, "type").unwrap_or_default(),
                            enum_values: spec.get("enum").map(|(_, n)| lista(n)),
                            labels: etiquetas(spec),
                            required: false,
                        },
                    );
                }
            }
            crate::document::Kind::Interface => {
                if let Some(qn) = d.qname() {
                    s.interfaces
                        .insert(qn, d.section("requires").map(lista).unwrap_or_default());
                }
            }
            crate::document::Kind::Package => {
                if let Some(v) = d.meta("version").and_then(|n| n.as_str())
                    && let Some(v) = Version::parse(v)
                {
                    s.version = v;
                }
                s.notice_period = d
                    .section("sla")
                    .and_then(|sla| sla.get("breakingChangePolicy").map(|(_, p)| p.clone()))
                    .and_then(|p| cadena(&p, "noticePeriod"));
            }
            crate::document::Kind::Entity => {
                let Some(qn) = d.qname() else { continue };
                s.entities.insert(qn, entidad(d));
            }
            crate::document::Kind::Binding => {
                let Some(t) = d.section("targetEntity").and_then(|n| n.as_str()) else {
                    continue;
                };
                s.bindings.insert(
                    t.to_string(),
                    Bind {
                        source: d
                            .section("source")
                            .and_then(|n| n.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        mode: d
                            .section("materialization")
                            .and_then(|m| cadena(m, "mode"))
                            .unwrap_or_default(),
                    },
                );
            }
            crate::document::Kind::ConduitPolicy => {
                for (k, v) in d
                    .section("conduits")
                    .map(|n| n.entries())
                    .unwrap_or_default()
                {
                    let Some(nombre) = k.as_str() else { continue };
                    s.conduits.insert(
                        nombre.to_string(),
                        v.entries()
                            .iter()
                            .filter_map(|(lk, lv)| {
                                Some((lk.as_str()?.to_string(), lv.as_str()?.to_string()))
                            })
                            .collect(),
                    );
                }
            }
            _ => {}
        }
    }

    for (_, texto) in &pkg.cedar {
        for p in cedar::read(texto) {
            s.policies.insert(p.id.clone(), p);
        }
    }
    s
}

fn entidad(d: &Loaded) -> Ent {
    let mut e = Ent {
        labels: d
            .root
            .get("metadata")
            .map(|(_, m)| etiquetas(m))
            .unwrap_or_default(),
        primary_key: d.section("primaryKey").map(lista).unwrap_or_default(),
        ..Default::default()
    };

    for (k, v) in d.section("properties").map(|n| n.entries()).unwrap_or(&[]) {
        let Some(nombre) = k.as_str() else { continue };
        e.props.insert(
            nombre.to_string(),
            Prop {
                ty: cadena(v, "type").unwrap_or_default(),
                enum_values: v.get("enum").map(|(_, n)| lista(n)),
                labels: etiquetas(v),
                required: cadena(v, "required").as_deref() == Some("true"),
            },
        );
    }

    for (k, v) in d.section("relations").map(|n| n.entries()).unwrap_or(&[]) {
        let Some(nombre) = k.as_str() else { continue };
        e.relations.insert(
            nombre.to_string(),
            Rel {
                target: cadena(v, "target").unwrap_or_default(),
                cardinality: cadena(v, "cardinality").unwrap_or_default(),
                required: cadena(v, "required").as_deref() == Some("true"),
            },
        );
    }

    for m in d.section("moved").map(|n| n.items()).unwrap_or(&[]) {
        if let Some(f) = cadena(m, "from") {
            e.anunciados.insert(f);
        }
    }
    e.anunciados
        .extend(d.section("reserved").map(lista).unwrap_or_default());
    e
}

// ── La comparación ──────────────────────────────────────────────────────────

pub fn diff(antes: &Package, despues: &Package) -> Report {
    let a = shape(antes);
    let b = shape(despues);
    let mut changes = Vec::new();

    entidades(&a, &b, &mut changes);
    significado(&a, &b, &mut changes);
    efectos_y_reglas(&a, &b, &mut changes);
    conductos(&a, &b, &mut changes);
    materializacion(&a, &b, &mut changes);
    politicas(&a, &b, &mut changes);
    gobierno(&a, &b, &mut changes);

    // El salto exigido sale de los ejes, y se calcula ANTES de mirar la versión
    // declarada: si dependiera de ella, comprobarla sería circular.
    let required_bump = if changes.iter().any(|c| c.axis.fuerza_mayor()) {
        Bump::Major
    } else if !changes.is_empty() || superficie(&a, &b) {
        Bump::Minor
    } else {
        Bump::Patch
    };

    sla(&a, &b, &mut changes);
    // `OOS5022` sí es rompedor en PACKAGE, así que puede elevar el salto.
    let required_bump = if changes.iter().any(|c| c.axis.fuerza_mayor()) {
        Bump::Major
    } else {
        required_bump
    };

    version(&a, &b, required_bump, &mut changes);

    Report {
        changes,
        required_bump,
    }
}

/// ¿Ha cambiado la superficie declarada, sin romper nada?
///
/// Se comparan los **nombres**, no cuántos hay. Un renombrado anunciado en
/// `moved` retira uno y añade otro: el recuento no se mueve y la superficie sí.
/// Contar habría clasificado como parche un cambio que obliga a todo consumidor
/// a tocar su código.
fn superficie(a: &Shape, b: &Shape) -> bool {
    let nombres = |s: &Shape| -> BTreeSet<String> {
        s.entities
            .iter()
            .flat_map(|(q, e)| {
                e.props
                    .keys()
                    .map(move |p| format!("{q}.{p}"))
                    .chain(
                        e.relations
                            .keys()
                            .map(move |r| format!("{q}.relations.{r}")),
                    )
                    .chain(std::iter::once(q.clone()))
            })
            .collect()
    };
    nombres(a) != nombres(b)
}

fn nivel(lat: &BTreeMap<String, Lattice>, reticulo: &str, nombre: &str) -> Option<usize> {
    lat.get(reticulo)?.levels.iter().position(|l| l == nombre)
}

/// `Function`, `Resolution` y `Ruleset` — la estación 10 para v1alpha2 y
/// v1alpha3, **sin un solo código nuevo**.
///
/// Que no haga falta ninguno es el resultado, no la restricción: cada cambio de
/// estos tres documentos tiene el mismo **síntoma** que uno que ya estaba
/// clasificado, y este registro emite un código por síntoma y no por causa.
///
/// - algo con nombre propio del que otros dependen deja de existir → `OOS5007`
/// - un tipo cambia → `OOS5002` / `OOS5010`, por la misma función que una propiedad
/// - un contrato existente pasa a exigir más → `OOS5025`
/// - una etiqueta baja → `OOS5011`. La integridad de una función **se computa**
///   de sus endosos (`02-function` §6): perder uno la baja
/// - un parámetro de seguridad se afloja → `OOS5016`
///
/// El último es el que decide si esto valía la pena. `OOS5016` existía para
/// *«`minGroupSize` de `aggregate` reducido»*, y el proyecto **ya había
/// decidido** que aflojar un parámetro de seguridad es un cambio rompedor —
/// pero solo lo implementó del lado de Cedar. El mismo aflojamiento en el
/// umbral de una fusión probabilística o en la cota de una aserción era
/// invisible. Mismo riesgo, dos tratamientos, según en qué documento estuviera
/// escrito.
fn efectos_y_reglas(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    for (qn, antes) in &a.funciones {
        let Some(despues) = b.funciones.get(qn) else {
            out.push(Change::new(Code::Oos5007, Axis::Consumer).sujeto(qn));
            continue;
        };
        tipos(&format!("{qn}.input"), &antes.input, &despues.input, out);
        tipos(&format!("{qn}.output"), &antes.output, &despues.output, out);

        // OOS5025 · exigir más. Una llamada que era legal deja de serlo, y el
        // que la hace vive en otro paquete.
        let nuevas: Vec<&String> = despues
            .preconditions
            .difference(&antes.preconditions)
            .collect();
        if !nuevas.is_empty() {
            out.push(Change::new(Code::Oos5025, Axis::Consumer).sujeto(qn).with(
                "nowRequires",
                Json::Arr(nuevas.iter().map(|p| Json::s(p.as_str())).collect()),
            ));
        }

        // OOS5011 · la integridad de una función SE COMPUTA de sus endosos.
        // Perder uno la baja, y es una etiqueta rebajada como cualquier otra.
        let perdidos: Vec<&String> = antes
            .endorsements
            .difference(&despues.endorsements)
            .collect();
        if !perdidos.is_empty() {
            out.push(Change::new(Code::Oos5011, Axis::Policy).sujeto(qn).with(
                "lostEndorsements",
                Json::Arr(perdidos.iter().map(|e| Json::s(e.as_str())).collect()),
            ));
        }
    }

    // OOS5016 · el umbral de una fusión probabilística. Bajarlo no cambia un
    // esquema: cambia **qué registros son la misma persona**. Se funden más, y
    // aguas abajo un registro de cliente puede absorber los datos de otro.
    for (qn, antes) in &a.umbrales {
        let Some(despues) = b.umbrales.get(qn) else {
            // La `Resolution` entera. Quien dependía de que dos registros se
            // fundieran deja de tenerlo, y sin aviso.
            out.push(Change::new(Code::Oos5007, Axis::Consumer).sujeto(qn));
            continue;
        };
        for (id, u) in antes {
            let Some(ahora) = despues.get(id) else {
                continue;
            };
            if menor(ahora, u) {
                out.push(
                    Change::new(Code::Oos5016, Axis::Policy)
                        .sujeto(format!("{qn}.{id}"))
                        .de_a(u, ahora),
                );
            }
        }
    }

    for (qn, antes) in &a.reglas {
        // Retirar el `Ruleset` ENTERO no se reporta aquí, y no es un olvido: ya
        // lo dice su **efecto** —`OOS5023`, una propiedad pierde una clase de
        // gobierno que tenía—, y `diff/weaken-coverage` lo fija así desde
        // v1alpha3. Añadirlo daría dos códigos para un cambio, en el mismo eje.
        //
        // Lo que faltaba es más fino y por eso no se veía: **una pieza que
        // desaparece de dentro de una regla que se queda.** Si un hermano de la
        // misma naturaleza sobrevive, la cobertura no cambia y `OOS5023` calla.
        let Some(despues) = b.reglas.get(qn) else {
            continue;
        };
        for id in antes.assertions.keys() {
            if !despues.assertions.contains_key(id) {
                out.push(Change::new(Code::Oos5007, Axis::Policy).sujeto(format!("{qn}.{id}")));
            }
        }
        for id in antes.masks.keys() {
            if !despues.masks.contains_key(id) {
                out.push(Change::new(Code::Oos5007, Axis::Policy).sujeto(format!("{qn}.{id}")));
            }
        }
        for d in antes.duties.difference(&despues.duties) {
            out.push(Change::new(Code::Oos5007, Axis::Policy).sujeto(format!("{qn}.{d}")));
        }

        // OOS5016 · una cota que se afloja. Solo los operadores con dirección
        // definida: `mustBe` y `mustNotBe` son igualdades y cambiarlas no es
        // relajar, es decir otra cosa.
        for (id, cotas) in &antes.assertions {
            let Some(ahora) = despues.assertions.get(id) else {
                continue;
            };
            for (op, aflojar_es_subir) in COTAS {
                let (Some(v0), Some(v1)) = (cotas.get(*op), ahora.get(*op)) else {
                    continue;
                };
                let aflojado = if *aflojar_es_subir {
                    menor(v0, v1)
                } else {
                    menor(v1, v0)
                };
                if aflojado {
                    out.push(
                        Change::new(Code::Oos5016, Axis::Policy)
                            .sujeto(format!("{qn}.{id}.{op}"))
                            .de_a(v0, v1),
                    );
                }
            }
        }

        // OOS5016 · literal: el `minGroupSize` de una máscara, que es para lo
        // que este código se escribió.
        for (id, antes_min) in &antes.masks {
            let (Some(v0), Some(Some(v1))) = (antes_min.as_ref(), despues.masks.get(id)) else {
                continue;
            };
            if menor(v1, v0) {
                out.push(
                    Change::new(Code::Oos5016, Axis::Policy)
                        .sujeto(format!("{qn}.{id}"))
                        .de_a(v0, v1),
                );
            }
        }
    }
}

/// Comparación numérica de dos valores escritos como cadena.
///
/// Cadena y no número porque **es la forma canónica**: un decimal sin comillas
/// no tiene representación estable (`OOS6003`). Se comparan como números
/// cuando los dos lo son, y si alguno no lo es no se afirma nada — de los dos
/// errores posibles, callar es el reversible.
fn menor(x: &str, y: &str) -> bool {
    match (x.parse::<f64>(), y.parse::<f64>()) {
        (Ok(a), Ok(b)) => a < b,
        _ => false,
    }
}

/// Conceptos y formas — la estación 10 para v1alpha4.
///
/// **Un código nuevo de cinco**, y esa proporción es el resultado: un concepto
/// es una propiedad un piso más arriba, así que retirarlo, cambiarle el tipo,
/// estrechar su `enum` o mover su clasificación son los cambios de siempre y
/// pasan por las mismas dos funciones que una propiedad de una entidad.
///
/// Antes de esto, un concepto que cambiaba de tipo, **rebajaba su clasificación
/// de `high` a `low`** y otro que desaparecía se clasificaban juntos como
/// `patch`. Rebajar la clasificación es lo que `OOS4012` impide dentro de un
/// paquete; entre dos versiones no lo veía nadie.
fn significado(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    for (qn, antes) in &a.conceptos {
        let Some(despues) = b.conceptos.get(qn) else {
            // OOS5007 · lo que otros nombran ha dejado de existir. Mismo código
            // que una entidad retirada, y por el mismo síntoma: todo `is` que
            // lo nombre queda colgando, en paquetes que no se han tocado.
            out.push(Change::new(Code::Oos5007, Axis::Consumer).sujeto(qn));
            continue;
        };
        tipos(qn, antes, despues, out);
        etiquetas_de(qn, &antes.labels, &despues.labels, &b.lattices, out);
    }

    for (qn, antes) in &a.interfaces {
        let Some(despues) = b.interfaces.get(qn) else {
            out.push(Change::new(Code::Oos5007, Axis::Consumer).sujeto(qn));
            continue;
        };
        // OOS5025 · exigir más. Quien declaraba implementarla deja de
        // satisfacerla, y eso no es un aviso: es que **no compila**.
        //
        // Exigir MENOS no aparece aquí, y la asimetría es deliberada: al
        // encoger `requires`, más formas la subsumen —`J.requires ⊆ I.requires`
        // se cumple para más `I`— luego una regla que apunte a ella alcanza
        // más. Agrandar lo gobernado es la dirección segura.
        let nuevos: Vec<&String> = despues.iter().filter(|c| !antes.contains(c)).collect();
        if !nuevos.is_empty() {
            out.push(Change::new(Code::Oos5025, Axis::Consumer).sujeto(qn).with(
                "nowRequires",
                Json::Arr(nuevos.iter().map(|c| Json::s(c.as_str())).collect()),
            ));
        }
    }
}

fn entidades(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    for (qn, antes) in &a.entities {
        let Some(despues) = b.entities.get(qn) else {
            // OOS5007 · la entidad ya no está.
            out.push(Change::new(Code::Oos5007, Axis::Consumer).sujeto(qn));
            continue;
        };

        etiquetas_de(qn, &antes.labels, &despues.labels, &b.lattices, out);

        // OOS5006 + OOS5018 · la misma causa en dos ejes: los consumidores
        // pierden la identidad con la que referenciaban, y el índice hay que
        // reconstruirlo. Ninguno de los dos veredictos implica al otro.
        if antes.primary_key != despues.primary_key {
            out.push(Change::new(Code::Oos5006, Axis::Consumer).sujeto(qn));
            out.push(Change::new(Code::Oos5018, Axis::Index).sujeto(qn));
        }

        for (nombre, p) in &antes.props {
            let sujeto = format!("{qn}.{nombre}");
            let Some(q) = despues.props.get(nombre) else {
                // OOS5001 · un nombre anunciado en `moved` o `reserved` no
                // desaparece en silencio: desaparece con instrucciones.
                if !despues.anunciados.contains(nombre) {
                    out.push(Change::new(Code::Oos5001, Axis::Consumer).sujeto(&sujeto));
                }
                continue;
            };

            tipos(&sujeto, p, q, out);
            if !p.required && q.required {
                out.push(Change::new(Code::Oos5003, Axis::Consumer).sujeto(&sujeto));
            }
            etiquetas_de(&sujeto, &p.labels, &q.labels, &b.lattices, out);
        }

        for (nombre, r) in &antes.relations {
            let sujeto = format!("{qn}.relations.{nombre}");
            let Some(s) = despues.relations.get(nombre) else {
                out.push(Change::new(Code::Oos5007, Axis::Consumer).sujeto(&sujeto));
                continue;
            };
            // OOS5003 · endurecer cardinalidad. `0..n → 1..n` deja fuera a
            // quien tenía filas sin relacionar.
            if (!r.required && s.required) || r.cardinality != s.cardinality {
                out.push(Change::new(Code::Oos5003, Axis::Consumer).sujeto(&sujeto));
            }
            if r.target != s.target {
                out.push(
                    Change::new(Code::Oos5002, Axis::Consumer)
                        .sujeto(&sujeto)
                        .de_a(&r.target, &s.target),
                );
            }
        }
    }
}

/// El tipo: primero lo paramétrico, que es lo específico.
fn tipos(sujeto: &str, p: &Prop, q: &Prop, out: &mut Vec<Change>) {
    if p.ty != q.ty {
        let base = |t: &str| t.split('<').next().unwrap_or(t).trim().to_string();
        // OOS5010 · misma base, distintos parámetros: `Money<EUR,2> →
        // Money<USD,2>` no es un tipo nuevo, es el mismo tipo mintiendo. El
        // valor 68400.50 sigue cabiendo y significa otra cosa.
        let code = if base(&p.ty) == base(&q.ty) && p.ty.contains('<') {
            Code::Oos5010
        } else {
            Code::Oos5002
        };
        out.push(
            Change::new(code, Axis::Consumer)
                .sujeto(sujeto)
                .de_a(&p.ty, &q.ty),
        );
        return;
    }

    // OOS5002 · retirar valores de un enum. Añadirlos no rompe a quien lee.
    if let (Some(antes), Some(despues)) = (&p.enum_values, &q.enum_values) {
        let retirados: Vec<&String> = antes.iter().filter(|v| !despues.contains(v)).collect();
        if !retirados.is_empty() {
            out.push(
                Change::new(Code::Oos5002, Axis::Consumer)
                    .sujeto(sujeto)
                    .with(
                        "removed",
                        Json::Arr(retirados.into_iter().map(Json::s).collect()),
                    ),
            );
        }
    } else if p.enum_values.is_none() && q.enum_values.is_some() {
        // De abierto a cerrado: el dominio se estrecha.
        out.push(Change::new(Code::Oos5002, Axis::Consumer).sujeto(sujeto));
    }
}

/// **El caso que demuestra que los ejes no son académicos.**
///
/// Elevar una etiqueta y rebajarla son el mismo cambio con el signo cambiado, y
/// cada uno rompe un eje distinto. Elevar mejora la gobernanza y rompe al
/// consumidor; rebajar no rompe a ningún consumidor y concede acceso en
/// silencio.
fn etiquetas_de(
    sujeto: &str,
    antes: &BTreeMap<String, String>,
    despues: &BTreeMap<String, String>,
    lat: &BTreeMap<String, Lattice>,
    out: &mut Vec<Change>,
) {
    for (reticulo, nivel_antes) in antes {
        let Some(nivel_despues) = despues.get(reticulo) else {
            continue;
        };
        if nivel_antes == nivel_despues {
            continue;
        }
        let (Some(i), Some(j)) = (
            nivel(lat, reticulo, nivel_antes),
            nivel(lat, reticulo, nivel_despues),
        ) else {
            continue;
        };
        let de = format!("{reticulo}:{nivel_antes}");
        let a = format!("{reticulo}:{nivel_despues}");

        // OOS5008 · la madurez tiene su propio código porque su lectura es
        // distinta. En el retículo, `STABLE` es el FONDO —lo que puede servirse
        // a cualquiera— y `DEPRECATED` el techo, así que volver a `DRAFT` o a
        // `REVIEWED` es SUBIR: restringir algo que ya se había prometido. Eso
        // es lo que rompe.
        //
        // Deprecar también sube, y no es lo mismo: es una salida ordenada que
        // anuncia el fin de la promesa en vez de retirarla sin aviso, así que
        // se excluye explícitamente.
        if reticulo == "oos.maturity" {
            if nivel_antes == "STABLE" && nivel_despues != "DEPRECATED" && j > i {
                out.push(
                    Change::new(Code::Oos5008, Axis::Consumer)
                        .sujeto(sujeto)
                        .de_a(&de, &a),
                );
            }
            continue;
        }

        let code = if j > i { Code::Oos5009 } else { Code::Oos5011 };
        let axis = if j > i { Axis::Consumer } else { Axis::Policy };
        out.push(Change::new(code, axis).sujeto(sujeto).de_a(&de, &a));
    }
}

fn conductos(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    for (nombre, antes) in &a.conduits {
        let Some(despues) = b.conduits.get(nombre) else {
            continue;
        };
        for (reticulo, nivel_antes) in antes {
            let Some(nivel_despues) = despues.get(reticulo) else {
                continue;
            };
            let (Some(i), Some(j)) = (
                nivel(&b.lattices, reticulo, nivel_antes),
                nivel(&b.lattices, reticulo, nivel_despues),
            ) else {
                continue;
            };
            // Las dos direcciones rompen, y por motivos opuestos
            // (`91-versioning` §4). Elevar deja pasar lo que antes no pasaba:
            // conceder acceso en silencio. Rebajar retira lo que sí pasaba, y
            // eso se lo encuentra un consumidor.
            let (code, axis) = match j.cmp(&i) {
                std::cmp::Ordering::Greater => (Code::Oos5012, Axis::Policy),
                std::cmp::Ordering::Less => (Code::Oos5026, Axis::Consumer),
                std::cmp::Ordering::Equal => continue,
            };
            out.push(
                Change::new(code, axis)
                    .with("conduit", Json::s(nombre))
                    .de_a(
                        format!("{reticulo}:{nivel_antes}"),
                        format!("{reticulo}:{nivel_despues}"),
                    ),
            );
        }
    }
}

fn materializacion(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    for (entidad, antes) in &a.bindings {
        let Some(despues) = b.bindings.get(entidad) else {
            continue;
        };
        // OOS5020 · cambiar el modo. No rompe a nadie, pero decide si el índice
        // existe: callarlo dejaría un índice fantasma sirviendo lecturas.
        if antes.mode != despues.mode {
            out.push(
                Change::new(Code::Oos5020, Axis::Index)
                    .sujeto(entidad)
                    .de_a(&antes.mode, &despues.mode),
            );
        }
        if antes.source != despues.source {
            out.push(
                Change::new(Code::Oos5019, Axis::Index)
                    .sujeto(entidad)
                    .de_a(&antes.source, &despues.source),
            );
        }
    }
}

/// El eje `POLICY` sobre el plano de gobierno.
///
/// Dos códigos y **ninguno mira la sintaxis de una regla**: `OOS5023` compara
/// el efecto —qué clase de gobierno tiene cada propiedad— y `OOS5024`, la
/// exigencia. Seis cambios distintos producen el primero, y eso es deliberado:
/// un código por síntoma, no por causa.
///
/// # Solo la dirección que debilita, y por qué
///
/// Endurecer el gobierno **no** se registra. No es una omisión: endurecerlo
/// rompe la compilación de quien lo endurece, en su propia rama, de forma
/// ruidosa. Lo que esta familia existe para cazar es lo **silencioso**, y
/// quitar gobierno lo es — el paquete sigue compilando, y nadie se entera de
/// que una columna con PII dejó de exigir una política.
fn gobierno(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    // OOS5023 · lo que una propiedad tenía cubierto y ha dejado de tener.
    for (prop, antes) in &a.gobernadas {
        let vacio = BTreeSet::new();
        let despues = b.gobernadas.get(prop).unwrap_or(&vacio);
        let perdidas: Vec<&str> = antes.difference(despues).copied().collect();
        if perdidas.is_empty() {
            continue;
        }
        out.push(
            Change::new(Code::Oos5023, Axis::Policy)
                .with("property", Json::s(prop))
                .with(
                    "lost",
                    Json::Arr(perdidas.iter().map(|n| Json::s(*n)).collect()),
                ),
        );
    }

    // OOS5024 · lo que la clasificación exigía y ha dejado de exigir. Es el
    // cambio de una línea en un retículo importado que desgobierna un paquete
    // entero sin tocarlo.
    for (ret, antes) in &a.exigencias {
        let vacio = BTreeMap::new();
        let despues = b.exigencias.get(ret).unwrap_or(&vacio);
        for (nivel, exigidas) in antes {
            let ahora = despues.get(nivel).cloned().unwrap_or_default();
            let perdidas: Vec<&String> = exigidas.iter().filter(|n| !ahora.contains(n)).collect();
            if perdidas.is_empty() {
                continue;
            }
            out.push(
                Change::new(Code::Oos5024, Axis::Policy)
                    .with("lattice", Json::s(ret))
                    .with("level", Json::s(nivel))
                    .with(
                        "noLongerRequired",
                        Json::Arr(perdidas.iter().map(|n| Json::s(n.as_str())).collect()),
                    ),
            );
        }
    }

    // Y lo mismo desde el tercer origen. Mismo código porque es el mismo
    // síntoma —la clasificación exige menos que antes— y este registro emite
    // **un código por síntoma, no por causa**.
    for (concepto, antes) in &a.exigencias_de_concepto {
        let vacio = Vec::new();
        let ahora = b.exigencias_de_concepto.get(concepto).unwrap_or(&vacio);
        let perdidas: Vec<&String> = antes.iter().filter(|n| !ahora.contains(n)).collect();
        if perdidas.is_empty() {
            continue;
        }
        out.push(
            Change::new(Code::Oos5024, Axis::Policy)
                .with("concept", Json::s(concepto))
                .with(
                    "noLongerRequired",
                    Json::Arr(perdidas.iter().map(|n| Json::s(n.as_str())).collect()),
                ),
        );
    }
}

fn politicas(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    for (id, antes) in &a.policies {
        let Some(despues) = b.policies.get(id) else {
            // OOS5014 · un `forbid` de Cedar gana siempre sobre cualquier
            // permit. Quitarlo no cambia una regla: retira un techo.
            if antes.effect == Effect::Forbid {
                out.push(Change::new(Code::Oos5014, Axis::Policy).with("policy", Json::s(id)));
            }
            continue;
        };

        // Las condiciones sobre `context.purpose` se comparan como conjunto más
        // abajo, no como texto: `== "x"` y `in ["x"]` dicen lo mismo.
        let perdidas: Vec<&String> = antes
            .conditions
            .iter()
            .filter(|c| !c.contains("context.purpose") && !despues.conditions.contains(*c))
            .collect();
        if let Some(c) = perdidas.first() {
            // OOS5013 · un permit con una condición menos alcanza a más gente.
            // OOS5014 · un forbid con una condición menos prohíbe menos.
            let code = match antes.effect {
                Effect::Permit => Code::Oos5013,
                Effect::Forbid => Code::Oos5014,
            };
            out.push(
                Change::new(code, Axis::Policy)
                    .with("policy", Json::s(id))
                    .with("removedCondition", Json::s(*c)),
            );
        }

        // OOS5015 · ampliar las finalidades. El dato es el mismo; lo que cambia
        // es para qué se puede usar, y eso es exactamente lo que un régimen de
        // finalidad limitada existe para impedir.
        let anadidas: Vec<&String> = despues
            .purposes
            .iter()
            .filter(|p| !antes.purposes.contains(*p))
            .collect();
        if !anadidas.is_empty() {
            out.push(
                Change::new(Code::Oos5015, Axis::Policy)
                    .with("policy", Json::s(id))
                    .with(
                        "added",
                        Json::Arr(anadidas.into_iter().map(Json::s).collect()),
                    ),
            );
        }

        // OOS5016 · bajar el umbral de k-anonimato. Con `minGroupSize: 2`, el
        // agregado de un grupo de dos personas y el conocimiento de una revela
        // la otra.
        if let (Some(i), Some(j)) = (antes.min_group_size(), despues.min_group_size())
            && j < i
        {
            out.push(
                Change::new(Code::Oos5016, Axis::Policy)
                    .with("policy", Json::s(id))
                    .with("from", Json::Int(i as i64))
                    .with("to", Json::Int(j as i64)),
            );
        }

        // OOS5017 · añadir un desclasificador donde no lo había. Un
        // desclasificador es una vía de escape autorizada; abrir la primera en
        // una política es un cambio de naturaleza, no de grado.
        let previos = antes.declassifiers();
        if previos.is_empty() {
            for d in despues.declassifiers() {
                out.push(
                    Change::new(Code::Oos5017, Axis::Policy)
                        .with("policy", Json::s(id))
                        .with("declassifier", Json::s(d)),
                );
            }
        }
    }
}

/// `noticePeriod` es el **único campo del SLA que se tipa** — todo lo demás
/// viaja sin interpretar — precisamente porque es el único que el compilador
/// hace cumplir.
///
/// La comprobación es hermética: no consulta ningún reloj. Lo que exige es que
/// el aviso estuviera **en el repositorio anterior** — la entidad marcada
/// `DEPRECATED`, o el nombre anunciado en `moved` / `reserved`. Un aviso que no
/// está escrito no es un aviso.
fn sla(a: &Shape, b: &Shape, out: &mut Vec<Change>) {
    let Some(periodo) = b.notice_period.clone().or_else(|| a.notice_period.clone()) else {
        return;
    };

    let mut avisados = BTreeSet::new();
    for c in out.iter() {
        if c.axis != Axis::Consumer && c.axis != Axis::Policy {
            continue;
        }
        let Some(Json::Str(sujeto)) = c
            .campos
            .iter()
            .find(|(k, _)| *k == "subject")
            .map(|(_, v)| v)
        else {
            continue;
        };
        // La entidad del sujeto, sea la entidad misma o el dueño de la propiedad.
        let entidad = a
            .entities
            .get(sujeto)
            .map(|_| sujeto.clone())
            .or_else(|| {
                let (e, _) = sujeto.rsplit_once('.')?;
                a.entities.contains_key(e).then(|| e.to_string())
            })
            .unwrap_or_else(|| sujeto.clone());

        let deprecada = a
            .entities
            .get(&entidad)
            .and_then(|e| e.labels.get("oos.maturity"))
            .is_some_and(|m| m == "DEPRECATED");
        if !deprecada {
            avisados.insert(sujeto.clone());
        }
    }

    for sujeto in avisados {
        out.push(
            Change::new(Code::Oos5022, Axis::Package)
                .sujeto(sujeto)
                .with("noticePeriod", Json::s(&periodo)),
        );
    }
}

/// **La versión deja de ser una afirmación y pasa a ser una comprobación.**
///
/// Subir más de lo exigido es legítimo — nadie se rompe por un mayor de más.
/// Subir menos es afirmar una compatibilidad que el compilador acaba de
/// desmentir.
fn version(a: &Shape, b: &Shape, exigido: Bump, out: &mut Vec<Change>) {
    let minimo = a.version.tras(exigido);
    if b.version < minimo {
        out.push(
            Change::new(Code::Oos5021, Axis::Package)
                .with("declared", Json::s(b.version.texto()))
                .with("required", Json::s(minimo.texto())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_salto_exigido_sale_de_la_version_anterior() {
        let v = Version::parse("1.4.0").unwrap();
        assert_eq!(v.tras(Bump::Major).texto(), "2.0.0");
        assert_eq!(v.tras(Bump::Minor).texto(), "1.5.0");
        assert!(Version::parse("1.4").is_none());
        assert!(Version::parse("1.4.0-rc.1").is_some());
    }

    #[test]
    fn index_no_fuerza_un_mayor() {
        assert!(!Axis::Index.fuerza_mayor());
        assert!(Axis::Consumer.fuerza_mayor());
        assert!(Axis::Policy.fuerza_mayor());
    }
}
