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
}

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
    let mut s = Shape {
        lattices: flow::lattices(pkg),
        ..Default::default()
    };

    for d in &pkg.docs {
        match d.kind {
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
    conductos(&a, &b, &mut changes);
    materializacion(&a, &b, &mut changes);
    politicas(&a, &b, &mut changes);

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
        // distinta: DEPRECATED está *arriba* del retículo, así que deprecar no
        // es rebajar. Rebajar es retirar una promesa ya hecha.
        if reticulo == "oos.maturity" {
            if nivel_antes == "STABLE" && j < i {
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
            // OOS5012 · elevar la autorización de un conducto deja pasar lo que
            // antes no pasaba. Es la definición de conceder acceso en silencio.
            if j > i {
                out.push(
                    Change::new(Code::Oos5012, Axis::Policy)
                        .with("conduit", Json::s(nombre))
                        .de_a(
                            format!("{reticulo}:{nivel_antes}"),
                            format!("{reticulo}:{nivel_despues}"),
                        ),
                );
            }
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
