//! v1alpha33 — **el enriquecimiento** (ORE 0068 C4, OOS v1alpha33 `01-enrich`):
//! un `Transform` con `runtime: enrich`, cuyo código es una declaración —un
//! `.enrich.yaml`— que dice qué capas, con qué relación, qué se toma y cómo se
//! reparte. Como el de un `.sql`, su documento **se deriva** del fichero y se
//! coteja con él ([`crate::transformar`]); lo de aquí es lo propio:
//!
//! 1. leer el fichero: un documento YAML por transform, con la forma de §2, o
//!    sus fallos (`OOS2043`);
//! 2. la [`Produccion`] que da cada uno, de la que sale el documento derivado;
//! 3. lo que se comprueba sin ejecutar sobre el árbol (§4): que las columnas
//!    están (`OOS2054`), un solo CRS, metros sólo donde se mide en metros y un
//!    método que casa (`OOS3009`);
//! 4. el esquema de la salida (§5), que se sabe antes de construir.
//!
//! Lo que se comprueba sobre las columnas de un dataset sólo se comprueba si se
//! saben: la salida por nacer de otro transform no dice nada hasta su build.

use crate::code::Code;
use crate::diag::{Diagnostic, Pos};
use crate::link::{Loaded, Package, cualificar};
use crate::parse::{Node, Style, parse_stream};
use crate::types::Type;
use ore_code::transform::Produccion;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// La terminación de un fichero de enriquecimiento (§2).
pub const EXTENSION: &str = ".enrich.yaml";

/// Si una ruta es un `.enrich.yaml`: código, no un documento del árbol.
pub fn es_fuente(ruta: &Path) -> bool {
    ruta.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(EXTENSION))
}

/// `<ruta>.enrich.yaml:<n>` → `(ruta, n)`, con la misma forma que el de un
/// `.sql` (v1alpha25 `01` §4): relativa con `/`, sin `..`; `n` desde 1.
pub fn entrypoint(s: &str) -> Option<(&str, usize)> {
    let (ruta, n) = s.rsplit_once(':')?;
    let ruta_ok = !ruta.starts_with('/')
        && !ruta.contains('\\')
        && !ruta.contains(':')
        && ruta.ends_with(EXTENSION)
        && ruta.len() > EXTENSION.len()
        && ruta.split('/').all(|t| !t.is_empty() && t != "..");
    let n_ok =
        n.starts_with(|c: char| ('1'..='9').contains(&c)) && n.chars().all(|c| c.is_ascii_digit());
    if !(ruta_ok && n_ok) {
        return None;
    }
    Some((ruta, n.parse().ok()?))
}

// ── Lo que dice el fichero ──────────────────────────────────────────────────

/// La relación del objetivo con cada rasgo de una capa (§2).
#[derive(Debug, Clone, PartialEq)]
pub enum Relacion {
    Within,
    Intersects,
    Contains,
    /// Los `k` más cercanos a menos de `max` metros.
    Nearest { k: u32, max: f64 },
}

/// Cómo se reparte lo que se toma de una columna (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metodo {
    Value,
    Sum,
    Avg,
    Min,
    Max,
    Extensive,
    Intensive,
    Majority,
}

impl Metodo {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "value" => Metodo::Value,
            "sum" => Metodo::Sum,
            "avg" => Metodo::Avg,
            "min" => Metodo::Min,
            "max" => Metodo::Max,
            "extensive" => Metodo::Extensive,
            "intensive" => Metodo::Intensive,
            "majority" => Metodo::Majority,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Metodo::Value => "value",
            Metodo::Sum => "sum",
            Metodo::Avg => "avg",
            Metodo::Min => "min",
            Metodo::Max => "max",
            Metodo::Extensive => "extensive",
            Metodo::Intensive => "intensive",
            Metodo::Majority => "majority",
        }
    }

    /// Si pide un número.
    fn numerico(self) -> bool {
        matches!(
            self,
            Metodo::Sum | Metodo::Avg | Metodo::Extensive | Metodo::Intensive
        )
    }

    /// Si reparte por el área de la intersección.
    fn por_area(self) -> bool {
        matches!(self, Metodo::Extensive | Metodo::Intensive)
    }
}

/// Qué se toma.
#[derive(Debug, Clone, PartialEq)]
pub enum Que {
    Columna {
        columna: String,
        pos: Pos,
        metodo: Metodo,
    },
    Contar,
    Distancia,
}

/// Un `take`: lo que se toma y cómo se llama en la salida.
#[derive(Debug, Clone, PartialEq)]
pub struct Toma {
    pub como: String,
    pub pos: Pos,
    pub que: Que,
}

/// Un dataset nombrado —el objetivo o una capa— y su columna geo, si se dice.
#[derive(Debug, Clone, PartialEq)]
pub struct Lectura {
    pub dataset: String,
    pub pos: Pos,
    pub geometry: Option<(String, Pos)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capa {
    pub lectura: Lectura,
    pub relacion: Relacion,
    pub around: Option<f64>,
    pub take: Vec<Toma>,
}

/// Un documento del `.enrich.yaml`: un transform.
#[derive(Debug, Clone, PartialEq)]
pub struct Enriquecimiento {
    pub pos: Pos,
    pub descripcion: Option<String>,
    pub output: String,
    pub pos_output: Pos,
    pub objetivo: Lectura,
    pub capas: Vec<Capa>,
}

/// Un motivo por el que un documento no se deriva, y dónde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallo {
    pub mensaje: String,
    pub pos: Pos,
}

/// Un `.enrich.yaml` leído: cada documento, el suyo o sus fallos, por su
/// ordinal desde 1. `Err` si no es YAML.
pub type Fichero = Result<BTreeMap<usize, Result<Enriquecimiento, Vec<Fallo>>>, Fallo>;

/// Lee un `.enrich.yaml` (§2).
pub fn leer(texto: &str) -> Fichero {
    let docs = parse_stream(texto).map_err(|e| Fallo {
        mensaje: e.message,
        pos: e.pos,
    })?;
    Ok(docs
        .iter()
        .enumerate()
        .map(|(i, d)| (i + 1, documento(d)))
        .collect())
}

/// Lo que se lee de un mapa: cada clave conocida una vez, y un fallo por cada
/// una que no es de aquí.
struct Mapa<'a> {
    nodo: &'a Node,
    fallos: &'a mut Vec<Fallo>,
}

impl<'a> Mapa<'a> {
    fn de(nodo: &'a Node, donde: &str, claves: &[&str], fallos: &'a mut Vec<Fallo>) -> Option<Self> {
        if !matches!(nodo, Node::Mapping { .. }) {
            fallos.push(Fallo {
                mensaje: format!("{donde} is not a mapping"),
                pos: nodo.pos(),
            });
            return None;
        }
        for (k, _) in nodo.entries() {
            let k_txt = k.as_str().unwrap_or("");
            if !claves.contains(&k_txt) {
                fallos.push(Fallo {
                    mensaje: format!(
                        "`{k_txt}` is not a key of {donde}: {}",
                        claves
                            .iter()
                            .map(|c| format!("`{c}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    pos: k.pos(),
                });
            }
        }
        Some(Mapa { nodo, fallos })
    }

    fn opcional(&self, clave: &str) -> Option<&'a Node> {
        self.nodo.get(clave).map(|(_, v)| v)
    }

    fn obligatoria(&mut self, clave: &str, donde: &str) -> Option<&'a Node> {
        let v = self.opcional(clave);
        if v.is_none() {
            self.fallos.push(Fallo {
                mensaje: format!("{donde} has no `{clave}`"),
                pos: self.nodo.pos(),
            });
        }
        v
    }

    /// Una cadena no vacía.
    fn texto(&mut self, v: &Node, clave: &str) -> Option<String> {
        match v {
            Node::Scalar { raw, .. } if !raw.trim().is_empty() => Some(raw.clone()),
            _ => {
                self.fallos.push(Fallo {
                    mensaje: format!("`{clave}` is not a name"),
                    pos: v.pos(),
                });
                None
            }
        }
    }

    /// Un número de metros mayor que 0.
    fn metros(&mut self, v: &Node, clave: &str) -> Option<f64> {
        let n = match v {
            Node::Scalar {
                raw,
                style: Style::Plain,
                ..
            } => raw.parse::<f64>().ok().filter(|x| x.is_finite() && *x > 0.0),
            _ => None,
        };
        if n.is_none() {
            self.fallos.push(Fallo {
                mensaje: format!("`{clave}` is not a number of meters greater than 0"),
                pos: v.pos(),
            });
        }
        n
    }
}

fn lectura(nodo: &Node, donde: &str, fallos: &mut Vec<Fallo>) -> Option<Lectura> {
    let mut m = Mapa::de(nodo, donde, &["dataset", "geometry"], fallos)?;
    let d = m.obligatoria("dataset", donde)?;
    let dataset = m.texto(d, "dataset")?;
    let geometry = match m.opcional("geometry") {
        Some(g) => Some((m.texto(g, "geometry")?, g.pos())),
        None => None,
    };
    Some(Lectura {
        dataset,
        pos: d.pos(),
        geometry,
    })
}

fn relacion(v: &Node, fallos: &mut Vec<Fallo>) -> Option<Relacion> {
    const AYUDA: &str = "`within`, `intersects`, `contains` or `{ nearest: k, maxDistance: m }`";
    match v {
        Node::Scalar { raw, .. } => match raw.as_str() {
            "within" => Some(Relacion::Within),
            "intersects" => Some(Relacion::Intersects),
            "contains" => Some(Relacion::Contains),
            otro => {
                fallos.push(Fallo {
                    mensaje: format!("`relation: {otro}` is not a relation: {AYUDA}"),
                    pos: v.pos(),
                });
                None
            }
        },
        Node::Mapping { .. } => {
            let mut m = Mapa::de(v, "`relation`", &["nearest", "maxDistance"], fallos)?;
            let k = m.obligatoria("nearest", "`relation`");
            let max = m.obligatoria("maxDistance", "`relation`");
            let k = k.and_then(|k| {
                let n = match k {
                    Node::Scalar {
                        raw,
                        style: Style::Plain,
                        ..
                    } => raw.parse::<u32>().ok().filter(|n| *n >= 1),
                    _ => None,
                };
                if n.is_none() {
                    m.fallos.push(Fallo {
                        mensaje: "`nearest` is not a whole number of 1 or more".into(),
                        pos: k.pos(),
                    });
                }
                n
            });
            let max = max.and_then(|x| m.metros(x, "maxDistance"));
            Some(Relacion::Nearest { k: k?, max: max? })
        }
        _ => {
            fallos.push(Fallo {
                mensaje: format!("`relation` is not a relation: {AYUDA}"),
                pos: v.pos(),
            });
            None
        }
    }
}

fn toma(nodo: &Node, cercano: bool, fallos: &mut Vec<Fallo>) -> Option<Toma> {
    let mut m = Mapa::de(
        nodo,
        "a `take`",
        &["as", "column", "count", "distance", "method"],
        fallos,
    )?;
    let como = m
        .obligatoria("as", "a `take`")
        .and_then(|v| m.texto(v, "as"));
    let columna = m.opcional("column");
    let contar = m.opcional("count");
    let distancia = m.opcional("distance");
    let metodo = m.opcional("method");
    let cuantas = [columna, contar, distancia].iter().flatten().count();
    if cuantas != 1 {
        m.fallos.push(Fallo {
            mensaje: "a `take` takes exactly one of `column`, `count: true` or `distance: true`"
                .into(),
            pos: nodo.pos(),
        });
        return None;
    }
    let verdad = |v: &Node| matches!(v, Node::Scalar { raw, style: Style::Plain, .. } if raw == "true");
    let que = if let Some(c) = columna {
        let columna = m.texto(c, "column")?;
        let metodo = match metodo {
            None => Metodo::Value,
            Some(x) => match x.as_str().and_then(Metodo::parse) {
                Some(me) => me,
                None => {
                    m.fallos.push(Fallo {
                        mensaje: format!(
                            "`method: {}` is not a method: `value`, `sum`, `avg`, `min`, `max`, \
                             `extensive`, `intensive` or `majority`",
                            x.as_str().unwrap_or("…")
                        ),
                        pos: x.pos(),
                    });
                    return None;
                }
            },
        };
        if cercano && metodo.por_area() {
            m.fallos.push(Fallo {
                mensaje: format!(
                    "`method: {}` shares by area, and `nearest` has no area: use `value`, `sum` or \
                     `avg`",
                    metodo.as_str()
                ),
                pos: metodo_pos(nodo),
            });
            return None;
        }
        Que::Columna {
            columna,
            pos: c.pos(),
            metodo,
        }
    } else {
        if let Some(x) = metodo {
            m.fallos.push(Fallo {
                mensaje: "`method` goes only with `column`".into(),
                pos: x.pos(),
            });
            return None;
        }
        let (v, que, clave) = match (contar, distancia) {
            (Some(v), _) => (v, Que::Contar, "count"),
            (_, Some(v)) => (v, Que::Distancia, "distance"),
            _ => unreachable!("exactamente una"),
        };
        if !verdad(v) {
            m.fallos.push(Fallo {
                mensaje: format!("`{clave}` is written `{clave}: true`"),
                pos: v.pos(),
            });
            return None;
        }
        if que == Que::Distancia && !cercano {
            m.fallos.push(Fallo {
                mensaje: "`distance: true` goes only with `relation: { nearest: … }`".into(),
                pos: v.pos(),
            });
            return None;
        }
        que
    };
    Some(Toma {
        como: como?,
        pos: nodo.pos(),
        que,
    })
}

fn metodo_pos(take: &Node) -> Pos {
    take.get("method").map_or(take.pos(), |(_, v)| v.pos())
}

fn capa(nodo: &Node, fallos: &mut Vec<Fallo>) -> Option<Capa> {
    let mut m = Mapa::de(
        nodo,
        "a layer",
        &["dataset", "geometry", "relation", "around", "take"],
        fallos,
    )?;
    let d = m.obligatoria("dataset", "a layer");
    let dataset = d.and_then(|d| m.texto(d, "dataset"));
    let geometry = match m.opcional("geometry") {
        Some(g) => m.texto(g, "geometry").map(|t| Some((t, g.pos()))),
        None => Some(None),
    };
    let rel = m
        .obligatoria("relation", "a layer")
        .and_then(|r| relacion(r, m.fallos));
    let around = match m.opcional("around") {
        Some(a) => m.metros(a, "around").map(Some),
        None => Some(None),
    };
    let take = m.obligatoria("take", "a layer");
    let rel = rel?;
    let cercano = matches!(rel, Relacion::Nearest { .. });
    let around = around?;
    if cercano && around.is_some() {
        let p = nodo.get("around").map_or(nodo.pos(), |(k, _)| k.pos());
        m.fallos.push(Fallo {
            mensaje: "`around` with `nearest`: the nearest is measured from the geometry itself"
                .into(),
            pos: p,
        });
        return None;
    }
    let take = take?;
    if !matches!(take, Node::Sequence { .. }) || take.items().is_empty() {
        m.fallos.push(Fallo {
            mensaje: "`take` is not a list of what is taken, or it is empty".into(),
            pos: take.pos(),
        });
        return None;
    }
    let mut tomas = Vec::new();
    let mut bien = true;
    for t in take.items() {
        match toma(t, cercano, m.fallos) {
            Some(x) => tomas.push(x),
            None => bien = false,
        }
    }
    if !bien {
        return None;
    }
    Some(Capa {
        lectura: Lectura {
            dataset: dataset?,
            pos: d?.pos(),
            geometry: geometry?,
        },
        relacion: rel,
        around,
        take: tomas,
    })
}

/// Un documento del fichero (§2), o todo lo que impide derivarlo.
fn documento(raiz: &Node) -> Result<Enriquecimiento, Vec<Fallo>> {
    let mut fallos = Vec::new();
    let e = (|| {
        let mut m = Mapa::de(
            raiz,
            "an enrichment",
            &["description", "output", "target", "layers"],
            &mut fallos,
        )?;
        let descripcion = match m.opcional("description") {
            Some(d) => Some(m.texto(d, "description")?),
            None => None,
        };
        let o = m.obligatoria("output", "an enrichment");
        let output = o.and_then(|o| m.texto(o, "output"));
        let t = m.obligatoria("target", "an enrichment");
        let objetivo = t.and_then(|t| lectura(t, "`target`", m.fallos));
        let l = m.obligatoria("layers", "an enrichment")?;
        if !matches!(l, Node::Sequence { .. }) || l.items().is_empty() {
            m.fallos.push(Fallo {
                mensaje: "`layers` is not a list of layers, or it is empty".into(),
                pos: l.pos(),
            });
            return None;
        }
        let mut capas = Vec::new();
        let mut bien = true;
        for x in l.items() {
            match capa(x, m.fallos) {
                Some(c) => capas.push(c),
                None => bien = false,
            }
        }
        let e = Enriquecimiento {
            pos: raiz.pos(),
            descripcion,
            output: output?,
            pos_output: o?.pos(),
            objetivo: objetivo?,
            capas,
        };
        bien.then_some(e)
    })();
    // Dos `take` con el mismo `as`: dos columnas de la salida con un nombre.
    if let Some(e) = &e {
        let mut vistos = BTreeSet::new();
        for t in e.capas.iter().flat_map(|c| &c.take) {
            if !vistos.insert(t.como.as_str()) {
                fallos.push(Fallo {
                    mensaje: format!("two `take` are named `as: {}`", t.como),
                    pos: t.pos,
                });
            }
        }
    }
    match e {
        Some(e) if fallos.is_empty() => Ok(e),
        _ => {
            if fallos.is_empty() {
                fallos.push(Fallo {
                    mensaje: "it does not have the shape of an enrichment".into(),
                    pos: raiz.pos(),
                });
            }
            Err(fallos)
        }
    }
}

impl Enriquecimiento {
    /// Lo que declara (§6): el objetivo y luego las capas, sin repetir.
    pub fn produccion(&self, ruta: &str, n: usize) -> Produccion {
        let mut inputs: Vec<String> = Vec::new();
        for d in std::iter::once(&self.objetivo.dataset)
            .chain(self.capas.iter().map(|c| &c.lectura.dataset))
        {
            if !inputs.contains(d) {
                inputs.push(d.clone());
            }
        }
        Produccion {
            runtime: "enrich",
            entrypoint: format!("{ruta}:{n}"),
            descripcion: self.descripcion.clone(),
            inputs,
            output: self.output.clone(),
        }
    }

    /// Dónde dice el código cada entrada y la salida.
    pub fn sitio(&self) -> crate::transformar::Sitio {
        let mut inputs = vec![(self.objetivo.dataset.clone(), self.objetivo.pos)];
        for c in &self.capas {
            if !inputs.iter().any(|(d, _)| *d == c.lectura.dataset) {
                inputs.push((c.lectura.dataset.clone(), c.lectura.pos));
            }
        }
        crate::transformar::Sitio {
            pos: Some(self.pos),
            inputs,
            output: Some(self.pos_output),
        }
    }
}

// ── Lo que se comprueba sobre el árbol (§4) ─────────────────────────────────

/// Lo que se sabe de un dataset nombrado: sus columnas y sus tipos, si se
/// saben. `None` si no resuelve a nada con columnas (por nacer, o una entrada
/// que no resuelve, que dice `OOS2018`).
struct Conocido {
    columnas: BTreeSet<String>,
    tipos: BTreeMap<String, Type>,
}

fn conocido(pkg: &Package, nombre: &str, desde: &Loaded) -> Option<Conocido> {
    let q = cualificar(nombre, desde);
    let d = pkg
        .table(&q)
        .or_else(|| pkg.dataset(&q))
        .or_else(|| pkg.view(&q))
        .or_else(|| pkg.object_table(&q))
        .or_else(|| pkg.collection(&q))?;
    let columnas = crate::vistas::columnas_que_expone(pkg, d);
    if columnas.is_empty() {
        return None;
    }
    Some(Conocido {
        columnas,
        tipos: crate::vistas::tipos_de_columnas(d),
    })
}

/// El CRS de un tipo geo: `Geography` es `OGC:CRS84`.
fn crs(t: &Type) -> Option<&str> {
    match t {
        Type::Geometry { crs, .. } => Some(crs),
        Type::Geography { .. } => Some(crate::types::CRS_DEFECTO),
        _ => None,
    }
}

/// §4.4: los CRS que miden en grados. Toda `Geography` también.
const GEOGRAFICOS: &[&str] = &[
    "OGC:CRS84",
    "OGC:CRS83",
    "OGC:CRS27",
    "EPSG:4326",
    "EPSG:4258",
    "EPSG:4269",
    "EPSG:4267",
    "EPSG:4230",
    "EPSG:4283",
    "EPSG:4674",
    "EPSG:4979",
];

fn geografico(t: &Type) -> bool {
    match t {
        Type::Geography { .. } => true,
        Type::Geometry { crs, .. } => GEOGRAFICOS.contains(&crs.as_str()),
        _ => false,
    }
}

fn poligonal(t: &Type) -> bool {
    match t {
        Type::Geometry { subtipo, .. } => {
            let s = subtipo.split_whitespace().next().unwrap_or("");
            s == "Polygon" || s == "MultiPolygon"
        }
        _ => false,
    }
}

fn numero(t: &Type) -> bool {
    match t {
        Type::Scalar(s) => matches!(s.as_str(), "Integer" | "Float" | "Decimal"),
        Type::Decimal { .. } => true,
        _ => false,
    }
}

/// La columna geo de una lectura y su tipo, si se sabe: la nombrada, o la
/// única. Lo que no casa es `OOS2054`.
fn geo_de(
    l: &Lectura,
    k: &Conocido,
    fichero: &Path,
    out: &mut Vec<Diagnostic>,
) -> Option<(String, Type)> {
    let geos: Vec<(&String, &Type)> = k.tipos.iter().filter(|(_, t)| t.es_geo()).collect();
    match &l.geometry {
        Some((g, pos)) => {
            if !k.columnas.contains(g) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2054,
                        fichero,
                        format!("`{}` has no column `{g}`", l.dataset),
                    )
                    .at(*pos)
                    .help(columnas_help(k)),
                );
                return None;
            }
            match k.tipos.get(g) {
                Some(t) if t.es_geo() => Some((g.clone(), t.clone())),
                Some(t) => {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2054,
                            fichero,
                            format!(
                                "`{}.{g}` is `{t}`, not a geometry: `geometry` names a \
                                 `Geometry<…>` or `Geography<…>` column",
                                l.dataset
                            ),
                        )
                        .at(*pos),
                    );
                    None
                }
                None => None, // sin tipo: no se sabe
            }
        }
        None => match geos.as_slice() {
            [(g, t)] => Some(((*g).clone(), (*t).clone())),
            [] if k.tipos.is_empty() => None, // sin tipos: no se sabe
            [] => {
                out.push(
                    Diagnostic::new(
                        Code::Oos2054,
                        fichero,
                        format!("`{}` has no geometry column to enrich by", l.dataset),
                    )
                    .at(l.pos)
                    .help("a geometry column is `Geometry<…>` or `Geography<…>`"),
                );
                None
            }
            varias => {
                let n: Vec<String> = varias.iter().map(|(g, _)| format!("`{g}`")).collect();
                out.push(
                    Diagnostic::new(
                        Code::Oos2054,
                        fichero,
                        format!(
                            "`{}` has {} geometry columns ({}): say which with `geometry:`",
                            l.dataset,
                            varias.len(),
                            n.join(", ")
                        ),
                    )
                    .at(l.pos),
                );
                None
            }
        },
    }
}

fn columnas_help(k: &Conocido) -> String {
    let c: Vec<String> = k.columnas.iter().map(|c| format!("`{c}`")).collect();
    format!("its columns: {}", c.join(", "))
}

/// §4, sobre un transform que es el que su código da: `desde` es su documento
/// (para cualificar los nombres) y `fichero`, el `.enrich.yaml`, donde se dice
/// todo.
pub fn comprobar(
    pkg: &Package,
    desde: &Loaded,
    e: &Enriquecimiento,
    fichero: &Path,
    out: &mut Vec<Diagnostic>,
) {
    let objetivo = conocido(pkg, &e.objetivo.dataset, desde);
    let geo_obj = objetivo
        .as_ref()
        .and_then(|k| geo_de(&e.objetivo, k, fichero, out));

    // Un `as` no repite una columna del objetivo (§4.2).
    if let Some(k) = &objetivo {
        for t in e.capas.iter().flat_map(|c| &c.take) {
            if k.columnas.contains(&t.como) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2054,
                        fichero,
                        format!(
                            "`as: {}` repeats a column of `{}`: the output has both",
                            t.como, e.objetivo.dataset
                        ),
                    )
                    .at(t.pos)
                    .help("give the taken column another name"),
                );
            }
        }
    }

    let objetivo_con_area = |c: &Capa| c.around.is_some() || geo_obj.as_ref().is_some_and(|(_, t)| poligonal(t));

    for c in &e.capas {
        let k = conocido(pkg, &c.lectura.dataset, desde);
        let geo_capa = k.as_ref().and_then(|k| geo_de(&c.lectura, k, fichero, out));

        // ── OOS3009 · un solo CRS ────────────────────────────────────────
        if let (Some((go, to)), Some((gc, tc))) = (&geo_obj, &geo_capa)
            && crs(to) != crs(tc)
        {
            out.push(
                Diagnostic::new(
                    Code::Oos3009,
                    fichero,
                    format!(
                        "`{}.{gc}` is in `{}` and `{}.{go}` in `{}`: an enrichment compares \
                         geometries in one CRS",
                        c.lectura.dataset,
                        crs(tc).unwrap_or_default(),
                        e.objetivo.dataset,
                        crs(to).unwrap_or_default()
                    ),
                )
                .at(c.lectura.pos)
                .help("reproject one of them first, in a view (`ST_Transform`): it is explicit"),
            );
        }

        // ── OOS3009 · metros, sólo donde se mide en metros ───────────────
        let en_grados = geo_obj
            .as_ref()
            .map(|(_, t)| t)
            .or(geo_capa.as_ref().map(|(_, t)| t))
            .filter(|t| geografico(t));
        if let Some(t) = en_grados {
            let mut metricas: Vec<(String, Pos)> = Vec::new();
            if let Some(a) = c.around {
                metricas.push((format!("`around: {a}`"), c.lectura.pos));
            }
            if let Relacion::Nearest { max, .. } = c.relacion {
                metricas.push((format!("`maxDistance: {max}`"), c.lectura.pos));
            }
            for x in &c.take {
                match &x.que {
                    Que::Distancia => metricas.push(("`distance: true`".into(), x.pos)),
                    Que::Columna { metodo, .. } if metodo.por_area() => {
                        metricas.push((format!("`method: {}`", metodo.as_str()), x.pos))
                    }
                    _ => {}
                }
            }
            for (que, pos) in metricas {
                out.push(
                    Diagnostic::new(
                        Code::Oos3009,
                        fichero,
                        format!(
                            "{que} measures in meters, and `{}` is in `{}`, which measures in degrees",
                            c.lectura.dataset,
                            crs(t).unwrap_or_default()
                        ),
                    )
                    .at(pos)
                    .help(
                        "reproject first, in a view, to a CRS in meters (for Spain, \
                         `EPSG:25830`): it is explicit",
                    ),
                );
            }
        }

        // ── OOS2054 · OOS3009 · cada columna que se toma ─────────────────
        for x in &c.take {
            let Que::Columna {
                columna,
                pos,
                metodo,
            } = &x.que
            else {
                continue;
            };
            if let Some(k) = &k {
                if !k.columnas.contains(columna) {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2054,
                            fichero,
                            format!("`{}` has no column `{columna}`", c.lectura.dataset),
                        )
                        .at(*pos)
                        .help(columnas_help(k)),
                    );
                    continue;
                }
                if metodo.numerico()
                    && let Some(t) = k.tipos.get(columna)
                    && !numero(t)
                {
                    out.push(
                        Diagnostic::new(
                            Code::Oos3009,
                            fichero,
                            format!(
                                "`method: {}` adds numbers, and `{}.{columna}` is `{t}`",
                                metodo.as_str(),
                                c.lectura.dataset
                            ),
                        )
                        .at(*pos)
                        .help("`value`, `min`, `max` or `majority` take any column"),
                    );
                }
            }
            if metodo.por_area() {
                let capa_ok = geo_capa.as_ref().is_none_or(|(_, t)| poligonal(t));
                let obj_ok = geo_obj.is_none() || objetivo_con_area(c);
                if !capa_ok {
                    out.push(
                        Diagnostic::new(
                            Code::Oos3009,
                            fichero,
                            format!(
                                "`method: {}` shares by area, and `{}` is not a layer of polygons",
                                metodo.as_str(),
                                c.lectura.dataset
                            ),
                        )
                        .at(*pos)
                        .help("over points or lines, what adds up is `sum` (or `avg`)"),
                    );
                } else if !obj_ok {
                    out.push(
                        Diagnostic::new(
                            Code::Oos3009,
                            fichero,
                            format!(
                                "`method: {}` shares by area, and `{}` has none: it is not \
                                 polygons and the layer has no `around`",
                                metodo.as_str(),
                                e.objetivo.dataset
                            ),
                        )
                        .at(*pos)
                        .help("give the layer an `around` (meters): the area around each row"),
                    );
                }
            }
        }
    }
}

/// §5 · **El esquema de la salida**, sin ejecutar: las columnas del objetivo
/// y una por `take`. `None` si las del objetivo no se saben; una columna cuyo
/// tipo no se sabe, sin tipo (`None`).
pub fn esquema_de_salida(
    pkg: &Package,
    desde: &Loaded,
    e: &Enriquecimiento,
) -> Option<Vec<(String, Option<Type>)>> {
    let objetivo = conocido(pkg, &e.objetivo.dataset, desde)?;
    let mut out: Vec<(String, Option<Type>)> = objetivo
        .columnas
        .iter()
        .map(|c| (c.clone(), objetivo.tipos.get(c).cloned()))
        .collect();
    let float = || Some(Type::Scalar("Float".into()));
    for c in &e.capas {
        let k = conocido(pkg, &c.lectura.dataset, desde);
        let tipo_de = |col: &str| k.as_ref().and_then(|k| k.tipos.get(col).cloned());
        for t in &c.take {
            let ty = match &t.que {
                Que::Contar => Some(Type::Scalar("Integer".into())),
                Que::Distancia => float(),
                Que::Columna {
                    columna, metodo, ..
                } => match metodo {
                    Metodo::Extensive | Metodo::Intensive | Metodo::Avg => float(),
                    Metodo::Sum => match tipo_de(columna) {
                        Some(Type::Scalar(s)) if s == "Integer" => Some(Type::Scalar(s)),
                        Some(_) => float(),
                        None => None,
                    },
                    Metodo::Value | Metodo::Majority | Metodo::Min | Metodo::Max => {
                        tipo_de(columna)
                    }
                },
            };
            out.push((t.como.clone(), ty));
        }
    }
    Some(out)
}

// ── El cómputo: el SQL de cada motor (C4·2 DuckDB, C4·3 Spark) ──────────────
//
// Medido en victor con los límites de un puesto Lightweight (2 CPU, 4 GiB,
// DuckDB 1.5.6, previo de C4·2): toda relación cae en `SPATIAL_JOIN` (el
// R-tree de DuckDB), también `ST_DWithin`; el más cercano como *join* a
// `maxDistance` con `arg_min` es 2,3× más rápido que un `lateral` con `limit`.
// Por eso, por capa, un CTE de pares (`pN`) y uno de agregados (`cN`), y la
// salida es el objetivo entero con un LEFT JOIN de cada uno (D-C4-2). En Spark
// la misma forma: Sedona hace de cada *join* espacial un `RangeJoin` o un
// `DistanceJoin`, y el lado pequeño lo difunde `over()` (C3·a).
//
// Las dos sentencias las lee antes el guion de ORE (`sqlparser`, dialecto de
// DuckDB, que acepta las comillas invertidas de Spark) y la de Spark, además,
// `consulta_para_spark`: lo escrito aquí es lo que entienden los dos.

/// El motor para el que se escribe la sentencia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialecto {
    DuckDb,
    Spark,
}

impl Dialecto {
    /// Un identificador: entre comillas dobles en DuckDB, invertidas en Spark
    /// (donde las dobles son una cadena).
    fn id(self, s: &str) -> String {
        match self {
            Dialecto::DuckDb => format!("\"{}\"", s.replace('"', "\"\"")),
            Dialecto::Spark => format!("`{}`", s.replace('`', "``")),
        }
    }

    /// El número de cada fila del objetivo. En Spark, el de C3·b: un
    /// `row_number()` sin partición lo metería todo en una tarea.
    fn numero_de_fila(self) -> &'static str {
        match self {
            Dialecto::DuckDb => "row_number() over ()",
            Dialecto::Spark => "monotonically_increasing_id()",
        }
    }

    /// El valor de `v` en la fila de menor `clave`.
    fn arg_min(self, v: &str, clave: &str) -> String {
        match self {
            Dialecto::DuckDb => format!("arg_min({v}, {clave})"),
            Dialecto::Spark => format!("min_by({v}, {clave})"),
        }
    }

    /// El valor de `v` con más `peso` y, si empatan, el menor `v`.
    fn el_de_mas(self, v: &str, peso: &str) -> String {
        match self {
            Dialecto::DuckDb => format!("first({v} order by {peso} desc, {v})"),
            Dialecto::Spark => format!("min_by({v}, named_struct('w', -{peso}, 'v', {v}))"),
        }
    }

    fn wkb(self, g: &str) -> String {
        match self {
            Dialecto::DuckDb => format!("ST_AsWKB({g})"),
            Dialecto::Spark => format!("ST_AsBinary({g})"),
        }
    }
}

/// Un número de metros, como literal SQL.
fn num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// La columna geo de una lectura y su tipo, si se sabe; sin diagnósticos (los
/// dio la puerta). `Err` si no se puede saber cuál es.
fn geometria(
    pkg: &Package,
    desde: &Loaded,
    l: &Lectura,
) -> Result<(String, Option<Type>), String> {
    let k = conocido(pkg, &l.dataset, desde);
    if let Some(k) = &k
        && let Some((g, t)) = geo_de(l, k, Path::new(""), &mut Vec::new())
    {
        return Ok((g, Some(t)));
    }
    match &l.geometry {
        Some((g, _)) => Ok((g.clone(), k.and_then(|k| k.tipos.get(g).cloned()))),
        None => Err(format!(
            "`{}`: its geometry column is not known before it is built; say it with `geometry:`",
            l.dataset
        )),
    }
}

/// Las columnas del objetivo **en su orden**: las de `columns`, como se
/// declaran; si no, las que expone (ordenadas por nombre).
fn columnas_en_orden(pkg: &Package, desde: &Loaded, nombre: &str) -> Option<Vec<String>> {
    let q = cualificar(nombre, desde);
    let d = pkg.table(&q).or_else(|| pkg.dataset(&q));
    if let Some(cs) = d.and_then(|d| d.section("columns")) {
        let v: Vec<String> = cs
            .entries()
            .iter()
            .filter_map(|(k, _)| k.as_str().map(str::to_string))
            .collect();
        if !v.is_empty() {
            return Some(v);
        }
    }
    conocido(pkg, nombre, desde).map(|k| k.columnas.into_iter().collect())
}

/// **El SQL de DuckDB de un enriquecimiento** (C4·2): ver [`sql`].
pub fn sql_duckdb(pkg: &Package, desde: &Loaded, e: &Enriquecimiento) -> Result<String, String> {
    sql(pkg, desde, e, Dialecto::DuckDb)
}

/// **El de Spark con Sedona** (C4·3): ver [`sql`].
pub fn sql_spark(pkg: &Package, desde: &Loaded, e: &Enriquecimiento) -> Result<String, String> {
    sql(pkg, desde, e, Dialecto::Spark)
}

/// **La sentencia de un enriquecimiento**: `create or replace dataset … as …`
/// del guion de ORE, que se construye como la de un `.sql` (`runtime: sql`) en
/// el motor de su repositorio. Lee los nombres tal como el fichero los escribe.
pub fn sql(
    pkg: &Package,
    desde: &Loaded,
    e: &Enriquecimiento,
    d: Dialecto,
) -> Result<String, String> {
    let (g_obj, t_obj) = geometria(pkg, desde, &e.objetivo)?;
    let obj_poligonal = t_obj.as_ref().is_some_and(poligonal);
    let mut ctes = vec![format!(
        "t as (select *, {} __ore_k from {})",
        d.numero_de_fila(),
        e.objetivo.dataset
    )];
    // Las del objetivo: en DuckDB, `t.* exclude`; en Spark, una a una.
    let mut columnas: Vec<String> = match d {
        Dialecto::DuckDb => vec!["t.* exclude (__ore_k)".into()],
        Dialecto::Spark => columnas_en_orden(pkg, desde, &e.objetivo.dataset)
            .ok_or_else(|| {
                format!(
                    "`{}`: its columns are not known before it is built, and Spark names them one \
                     by one",
                    e.objetivo.dataset
                )
            })?
            .iter()
            .map(|c| format!("t.{}", d.id(c)))
            .collect(),
    };
    let mut joins: Vec<String> = Vec::new();
    for (i, c) in e.capas.iter().enumerate() {
        let n = i + 1;
        let (g_capa, t_capa) = geometria(pkg, desde, &c.lectura)?;
        let capa_poligonal = t_capa.as_ref().is_none_or(poligonal);
        let por_area = c.take.iter().any(|x| match &x.que {
            Que::Columna { metodo, .. } => {
                metodo.por_area() || matches!(metodo, Metodo::Value | Metodo::Majority)
            }
            _ => false,
        });
        let gt = format!("t.{}", d.id(&g_obj));
        let gl = format!("l.{}", d.id(&g_capa));
        // El área del objetivo: la suya, o la de `around`.
        let (desde_t, area_t, pred) = match (&c.relacion, c.around) {
            (Relacion::Nearest { max, .. }, _) => (
                "t".to_string(),
                None,
                format!("ST_DWithin({gt}, {gl}, {})", num(*max)),
            ),
            (r, Some(a)) if por_area || !matches!(r, Relacion::Intersects) => {
                let b = "t.__ore_b".to_string();
                let p = match r {
                    Relacion::Within => format!("ST_Within({b}, {gl})"),
                    Relacion::Contains => format!("ST_Contains({b}, {gl})"),
                    _ => format!("ST_Intersects({b}, {gl})"),
                };
                (
                    format!(
                        "(select __ore_k, ST_Buffer({}, {}) __ore_b from t) t",
                        d.id(&g_obj),
                        num(a)
                    ),
                    Some(b),
                    p,
                )
            }
            (_, Some(a)) => (
                "t".to_string(),
                None,
                format!("ST_DWithin({gt}, {gl}, {})", num(a)),
            ),
            (r, None) => {
                let p = match r {
                    Relacion::Within => format!("ST_Within({gt}, {gl})"),
                    Relacion::Contains => format!("ST_Contains({gt}, {gl})"),
                    _ => format!("ST_Intersects({gt}, {gl})"),
                };
                ("t".to_string(), obj_poligonal.then(|| gt.clone()), p)
            }
        };
        // Lo que cada par lleva: las columnas que se toman, el solape y la
        // distancia con su rango.
        let mut lleva: Vec<String> = vec!["t.__ore_k".into()];
        // En el orden en que se toman: el alias `__ore_c_<i>` es su posición
        // (el cotejo con PostGIS lo destapó con un conjunto ordenado).
        let mut vistas: Vec<&str> = Vec::new();
        for x in &c.take {
            if let Que::Columna { columna, .. } = &x.que
                && !vistas.contains(&columna.as_str())
            {
                vistas.push(columna);
                lleva.push(format!("l.{} __ore_c_{}", d.id(columna), vistas.len()));
            }
        }
        let alias = |col: &str| {
            let i = vistas.iter().position(|v| *v == col).unwrap_or(0) + 1;
            format!("__ore_c_{i}")
        };
        let solape = match (&area_t, capa_poligonal) {
            (Some(a), true) => Some(format!("ST_Area(ST_Intersection({a}, {gl}))")),
            _ => None,
        };
        if let Some(s) = &solape {
            lleva.push(format!("{s} __ore_a"));
            lleva.push(format!("ST_Area({gl}) __ore_al"));
        }
        let cercano = match c.relacion {
            Relacion::Nearest { k, .. } => Some(k),
            _ => None,
        };
        // El más cercano: con k = 1, `arg_min` sobre los pares a `maxDistance`
        // (lo medido); con k > 1, el rango de cada par y sólo los k primeros.
        let uno = cercano == Some(1);
        if cercano.is_some() {
            lleva.push(format!("ST_Distance({gt}, {gl}) __ore_d"));
        }
        if cercano.is_some() && !uno {
            lleva.push(format!(
                "row_number() over (partition by t.__ore_k order by ST_Distance({gt}, {gl}), {}) \
                 __ore_r",
                d.wkb(&gl)
            ));
        }
        let pares = format!(
            "select {} from {desde_t} join {} l on {pred}",
            lleva.join(", "),
            c.lectura.dataset
        );
        // Sólo los k primeros: sin `qualify`, que Spark no tiene.
        let pares = match cercano.filter(|_| !uno) {
            Some(k) => format!("select * from ({pares}) where __ore_r <= {k}"),
            None => pares,
        };
        // `majority`: el peso de cada categoría, con una ventana sobre los mismos
        // pares (C4·3b). Antes era otro CTE que releía `pN`, y Spark no guarda
        // un CTE: hacía el *join* espacial dos veces (~200 s de 592 con 10 M).
        let mut pesos: Vec<String> = Vec::new();
        if !uno {
            let peso = if solape.is_some() { "__ore_a" } else { "1" };
            for x in &c.take {
                if let Que::Columna {
                    columna,
                    metodo: Metodo::Majority,
                    ..
                } = &x.que
                {
                    let v = alias(columna);
                    let w = format!("__ore_w{}", &v["__ore_c".len()..]);
                    if !pesos.iter().any(|p| p.ends_with(&w)) {
                        pesos.push(format!(
                            "sum({peso}) over (partition by __ore_k, {v}) {w}"
                        ));
                    }
                }
            }
        }
        ctes.push(if pesos.is_empty() {
            format!("p{n} as ({pares})")
        } else {
            format!("p{n} as (select *, {} from ({pares}) q)", pesos.join(", "))
        });

        // Los agregados, por fila del objetivo: un solo CTE, que lee `pN` una vez.
        let mut aggs: Vec<String> = vec!["__ore_k".into()];
        for x in &c.take {
            let como = d.id(&x.como);
            let expr = match &x.que {
                Que::Contar => {
                    columnas.push(format!("coalesce(c{n}.{como}, 0) {como}"));
                    aggs.push(if uno {
                        format!("least(count(*), 1) {como}")
                    } else {
                        format!("count(*) {como}")
                    });
                    continue;
                }
                Que::Distancia => "min(__ore_d)".to_string(),
                Que::Columna {
                    columna, metodo, ..
                } => {
                    let v = alias(columna);
                    match metodo {
                        // Con k = 1, todo lo que se toma es del más cercano.
                        Metodo::Avg if uno => {
                            format!("cast({} as double)", d.arg_min(&v, "__ore_d"))
                        }
                        _ if uno => d.arg_min(&v, "__ore_d"),
                        Metodo::Value if cercano.is_some() => d.arg_min(&v, "__ore_r"),
                        Metodo::Value if solape.is_some() => d.el_de_mas(&v, "__ore_a"),
                        Metodo::Value => format!("min({v})"),
                        Metodo::Sum => format!("sum({v})"),
                        Metodo::Avg => format!("avg({v})"),
                        Metodo::Min => format!("min({v})"),
                        Metodo::Max => format!("max({v})"),
                        Metodo::Extensive => format!("sum({v} * __ore_a / nullif(__ore_al, 0))"),
                        Metodo::Intensive => {
                            format!("sum({v} * __ore_a) / nullif(sum(__ore_a), 0)")
                        }
                        Metodo::Majority => {
                            d.el_de_mas(&v, &format!("__ore_w{}", &v["__ore_c".len()..]))
                        }
                    }
                }
            };
            columnas.push(format!("c{n}.{como}"));
            aggs.push(format!("{expr} {como}"));
        }
        ctes.push(format!(
            "c{n} as (select {} from p{n} group by 1)",
            aggs.join(", ")
        ));
        joins.push(format!("left join c{n} using (__ore_k)"));
    }
    Ok(format!(
        "create or replace dataset {} as\nwith {}\nselect {}\nfrom t {}",
        e.output,
        ctes.join(",\n"),
        columnas.join(", "),
        joins.join(" ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASICO: &str = r#"description: Cada cliente.
output: ventas.clientes_enriquecidos
target: { dataset: ventas.clientes, geometry: ubicacion }
layers:
  - dataset: ventas.municipios
    relation: within
    take:
      - { as: municipio, column: nombre }
  - dataset: ventas.competidores
    relation: intersects
    around: 500
    take:
      - { as: competidores_500m, count: true }
  - dataset: ventas.competidores
    relation: { nearest: 1, maxDistance: 5000 }
    take:
      - { as: dist, distance: true }
"#;

    fn uno(t: &str) -> Result<Enriquecimiento, Vec<Fallo>> {
        leer(t).expect("yaml").remove(&1).expect("un documento")
    }

    /// §5, sobre el caso de conformidad: el esquema se sabe sin ejecutar.
    #[test]
    fn el_esquema_de_la_salida_se_sabe_sin_ejecutar() {
        let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/oos/conformance/v1alpha33/valid/an-enrich-transform/input");
        let (pkg, diags) = crate::validate::cargar_paquete(&raiz);
        assert!(diags.is_empty(), "{diags:?}");
        let t = pkg
            .of(crate::document::Kind::Transform)
            .next()
            .expect("el transform");
        let f = std::fs::read_to_string(
            raiz.join("packages/ventas/etl/clientes.enrich.yaml"),
        )
        .unwrap();
        let e = leer(&f).unwrap().remove(&1).unwrap().unwrap();
        let s: Vec<(String, String)> = esquema_de_salida(&pkg, t, &e)
            .expect("el objetivo tiene columnas")
            .into_iter()
            .map(|(c, t)| (c, t.map(|t| t.to_string()).unwrap_or_default()))
            .collect();
        let s: Vec<(&str, &str)> = s.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        assert_eq!(
            s,
            [
                ("cliente_id", "String"),
                ("ubicacion", "Geometry<Point, EPSG:25830>"),
                ("municipio", "String"),
                ("competidores_500m", "Integer"),
                ("ventas_500m", "Integer"),
                ("competidor_cercano", "String"),
                ("dist_competidor", "Float"),
            ]
        );
    }

    /// C4·2 · el SQL de DuckDB del caso de conformidad: el guion de ORE lo lee
    /// como una sentencia que escribe la salida y lee el objetivo y las capas.
    #[test]
    fn el_sql_de_duckdb_es_una_sentencia_del_guion() {
        let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/oos/conformance/v1alpha33/valid/an-enrich-transform/input");
        let (pkg, _) = crate::validate::cargar_paquete(&raiz);
        let t = pkg.of(crate::document::Kind::Transform).next().unwrap();
        let f = std::fs::read_to_string(raiz.join("packages/ventas/etl/clientes.enrich.yaml")).unwrap();
        let e = leer(&f).unwrap().remove(&1).unwrap().unwrap();
        let sql = sql_duckdb(&pkg, t, &e).expect("se genera");
        println!("{sql}");
        let trozos = crate::sql_del_arbol::guion::guion(&sql).unwrap_or_else(|f| panic!("{f:?}\n{sql}"));
        let crate::sql_del_arbol::guion::Sentencia::Unidad(u) = &trozos[0].sentencia else {
            panic!("no es una unidad: {sql}");
        };
        let escribe = u.escribe.as_ref().expect("escribe");
        assert_eq!(escribe.destino.referencia(), "ventas.clientes_enriquecidos");
        let lee: BTreeSet<String> = u.lee.iter().map(|n| n.referencia()).collect();
        assert_eq!(
            lee,
            ["ventas.clientes", "ventas.competidores", "ventas.municipios"]
                .map(String::from)
                .into()
        );
        // Lo medido: el más cercano con k = 1 es `arg_min`, sin ventana.
        assert!(sql.contains("arg_min(__ore_c_1, __ore_d)") && !sql.contains("qualify"), "{sql}");
        assert!(sql.contains("ST_DWithin(t.\"ubicacion\", l.\"geom\", 500)"), "{sql}");
    }

    /// C4·3 · el de Spark del mismo caso: lo leen el guion y
    /// `consulta_para_spark`, numera con `monotonically_increasing_id`, nombra
    /// las columnas del objetivo una a una y no lleva nada de DuckDB.
    #[test]
    fn el_sql_de_spark_lo_leen_el_guion_y_consulta_para_spark() {
        let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/oos/conformance/v1alpha33/valid/an-enrich-transform/input");
        let (pkg, _) = crate::validate::cargar_paquete(&raiz);
        let t = pkg.of(crate::document::Kind::Transform).next().unwrap();
        let f = std::fs::read_to_string(raiz.join("packages/ventas/etl/clientes.enrich.yaml")).unwrap();
        let e = leer(&f).unwrap().remove(&1).unwrap().unwrap();
        let s = sql_spark(&pkg, t, &e).expect("se genera");
        crate::sql_del_arbol::guion::guion(&s).unwrap_or_else(|f| panic!("{f:?}\n{s}"));
        let p = crate::sql_del_arbol::consulta_para_spark(s.split_once(" as\n").unwrap().1)
            .unwrap_or_else(|f| panic!("{f}\n{s}"));
        assert_eq!(p.vistas.len(), 3, "{:?}", p.vistas);
        assert!(s.contains("monotonically_increasing_id() __ore_k"), "{s}");
        assert!(s.contains("select t.`cliente_id`, t.`ubicacion`, c1.`municipio`"), "{s}");
        assert!(s.contains("min_by(__ore_c_1, __ore_d)"), "{s}");
        for duckdb in ["exclude", "qualify", "arg_min", "first(", "\"", "::"] {
            assert!(!s.contains(duckdb), "`{duckdb}` en el SQL de Spark:\n{s}");
        }
    }

    /// C4·3b · cada CTE de pares se lee **una vez**: Spark no guarda un CTE, y
    /// uno que se leyera dos veces haría su *join* espacial dos veces (medido:
    /// ~200 s de 592 con 10 M, por `majority`). El árbol del cotejo lo tiene.
    #[test]
    fn cada_cte_de_pares_se_lee_una_vez() {
        let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../pruebas-de-fuego/geo/c4/cotejo/arbol");
        let (pkg, _) = crate::validate::cargar_paquete(&raiz);
        let t = pkg.of(crate::document::Kind::Transform).next().unwrap();
        let f = std::fs::read_to_string(raiz.join("packages/ventas/etl/cotejo.enrich.yaml")).unwrap();
        let e = leer(&f).unwrap().remove(&1).unwrap().unwrap();
        for d in [Dialecto::DuckDb, Dialecto::Spark] {
            let s = sql(&pkg, t, &e, d).unwrap();
            for n in 1..=e.capas.len() {
                assert_eq!(s.matches(&format!("from p{n} ")).count(), 1, "p{n} en {d:?}:\n{s}");
            }
            assert!(s.contains("over (partition by __ore_k, __ore_c_3) __ore_w_3"), "{s}");
        }
    }

    /// El SQL de DuckDB de `ENRICH_ARBOL` + `ENRICH_FICHERO` (documento 1), en
    /// `ENRICH_SALIDA`: lo usa el cotejo con PostGIS
    /// (`pruebas-de-fuego/geo/c4/cotejo`). El árbol, sin diagnósticos.
    #[test]
    #[ignore = "lo corre pruebas-de-fuego/geo/c4/cotejo/cotejar.sh"]
    fn escribe_el_sql_de_un_arbol() {
        let raiz = std::path::PathBuf::from(std::env::var("ENRICH_ARBOL").unwrap());
        let (pkg, mut diags) = crate::validate::cargar_paquete(&raiz);
        crate::transformar::comprobar(&pkg, &mut diags);
        let errores: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_str().starts_with("OOS2") || d.code.as_str().starts_with("OOS3"))
            .collect();
        assert!(errores.is_empty(), "{errores:#?}");
        let t = pkg.of(crate::document::Kind::Transform).next().unwrap();
        let f = std::fs::read_to_string(raiz.join(std::env::var("ENRICH_FICHERO").unwrap())).unwrap();
        let e = leer(&f).unwrap().remove(&1).unwrap().unwrap();
        let salida = std::path::PathBuf::from(std::env::var("ENRICH_SALIDA").unwrap());
        for (d, nombre) in [(Dialecto::DuckDb, "enrich.sql"), (Dialecto::Spark, "enrich-spark.sql")] {
            let s = sql(&pkg, t, &e, d).expect("se genera");
            crate::sql_del_arbol::guion::guion(&s).unwrap_or_else(|f| panic!("{f:?}\n{s}"));
            if d == Dialecto::Spark {
                // Lo que lee la sentencia de Spark también lo reescribe `consulta_para_spark`.
                let consulta = s.split_once(" as\n").unwrap().1;
                crate::sql_del_arbol::consulta_para_spark(consulta).unwrap_or_else(|f| panic!("{f}\n{s}"));
            }
            std::fs::write(salida.join(nombre), &s).unwrap();
        }
    }

    #[test]
    fn el_entrypoint_tiene_la_forma_del_de_un_sql() {
        assert_eq!(entrypoint("etl/c.enrich.yaml:2"), Some(("etl/c.enrich.yaml", 2)));
        assert_eq!(entrypoint("etl/c.yaml:1"), None);
        assert_eq!(entrypoint("etl/c.enrich.yaml:0"), None);
        assert_eq!(entrypoint("../c.enrich.yaml:1"), None);
        assert_eq!(entrypoint(".enrich.yaml:1"), None);
        assert!(es_fuente(Path::new("a/b/c.enrich.yaml")));
        assert!(!es_fuente(Path::new("a/b/c.yaml")));
    }

    #[test]
    fn un_documento_da_su_produccion() {
        let e = uno(BASICO).unwrap();
        let p = e.produccion("etl/clientes.enrich.yaml", 1);
        assert_eq!(p.runtime, "enrich");
        assert_eq!(p.entrypoint, "etl/clientes.enrich.yaml:1");
        assert_eq!(p.descripcion.as_deref(), Some("Cada cliente."));
        assert_eq!(
            p.inputs,
            ["ventas.clientes", "ventas.municipios", "ventas.competidores"]
        );
        assert_eq!(p.output, "ventas.clientes_enriquecidos");
        assert_eq!(e.capas[2].relacion, Relacion::Nearest { k: 1, max: 5000.0 });
        assert_eq!(e.capas[1].around, Some(500.0));
        let doc = ore_code::transform::documento(&p, "ventas");
        assert!(doc.starts_with("# derivado por ore desde etl/clientes.enrich.yaml:1"), "{doc}");
        assert!(doc.contains("apiVersion: oos.dev/v1alpha33\n"), "{doc}");
        assert!(doc.contains("  runtime: enrich\n"), "{doc}");
    }

    #[test]
    fn varios_documentos_por_su_ordinal() {
        let f = leer(&format!("{BASICO}---\n{}", BASICO.replace("clientes_enriquecidos", "otro")))
            .unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[&2].as_ref().unwrap().output, "ventas.otro");
    }

    fn mensajes(t: &str) -> String {
        uno(t)
            .unwrap_err()
            .into_iter()
            .map(|f| f.mensaje)
            .collect::<Vec<_>>()
            .join(" | ")
    }

    #[test]
    fn lo_que_no_tiene_la_forma_no_se_deriva() {
        let m = mensajes(&BASICO.replace("{ as: municipio, column: nombre }", "{ column: nombre }"));
        assert!(m.contains("a `take` has no `as`"), "{m}");
        let m = mensajes(&BASICO.replace("relation: within", "relation: cerca"));
        assert!(m.contains("`relation: cerca` is not a relation"), "{m}");
        let m = mensajes(&BASICO.replace("{ as: dist, distance: true }", "{ as: dist, column: x, count: true }"));
        assert!(m.contains("exactly one"), "{m}");
        let m = mensajes(&BASICO.replace("around: 500", "around: -3"));
        assert!(m.contains("greater than 0"), "{m}");
        let m = mensajes(&BASICO.replace("{ as: competidores_500m, count: true }", "{ as: competidores_500m, distance: true }"));
        assert!(m.contains("only with `relation: { nearest"), "{m}");
        let m = mensajes(&BASICO.replace("as: dist,", "as: municipio,"));
        assert!(m.contains("two `take` are named `as: municipio`"), "{m}");
        let m = mensajes(&BASICO.replace("description:", "schedule: diario\ndescription:"));
        assert!(m.contains("`schedule` is not a key"), "{m}");
        let m = mensajes(&BASICO.replace("{ nearest: 1, maxDistance: 5000 }", "{ nearest: 1 }"));
        assert!(m.contains("has no `maxDistance`"), "{m}");
        let m = mensajes(&BASICO.replace("{ as: municipio, column: nombre }", "{ as: municipio, column: nombre, method: media }"));
        assert!(m.contains("`method: media` is not a method"), "{m}");
        let m = mensajes(&BASICO.replace("{ as: dist, distance: true }", "{ as: x, column: v, method: extensive }"));
        assert!(m.contains("`nearest` has no area"), "{m}");
        assert!(leer("a: [").is_err());
    }
}
