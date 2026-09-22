//! **El índice de assets** (0034 ⑤): el árbol compilado, proyectado a ítems con
//! sus relaciones y sus capas. Es lo que el catálogo de la consola lee, y nada
//! más: una representación, un sitio, a una cabeza.
//!
//! Una función pura del árbol (y de sus punteros) a un JSON:
//!
//! ```text
//! indice(pkg, punteros, cabeza) → { cabeza, rama, generado, paquetes[], items{ref → ítem} }
//! ```
//!
//! # Lo que es un ítem (0034 ②)
//!
//! Un documento de `packages/<p>/…` con kind `Dataset`, `Table`, `View`,
//! `Entity`, `Interface`, `Concept`, `Function`, `Action` o `TrainedModel`; y
//! los de la raíz sin paquete (`Model`, los `Concept` importados). `Package`,
//! `OntologyConfig`, `Lattice`, `ConduitPolicy`, `Ruleset`, `RequestPolicy` y
//! `Binding` no son ítems: alimentan las capas.
//!
//! Se dirige como **`kind:namespace.name`** (`dataset:ventas.pedidos`,
//! `model:v2-lite`), que es la clave de `items` y lo que una relación nombra.
//!
//! # Lo derivado, y de dónde
//!
//! | campo | de dónde |
//! |---|---|
//! | `paquete`, `carpeta` | la ruta: `packages/<p>/<carpeta>/<fichero>`, quitando las carpetas del kind (`tables/`, `views/`…) estén donde estén: los árboles de hoy caen enteros en `""` («sin clasificar») y `espana/views/x.yaml` es el schema `espana` |
//! | `define` | lo que tiene plan: `from` (la ref), `identidad` (sin `where`/`groupBy`/`having` y expone lo de abajo con sus nombres), qué claves usa, `freshness`; un escrito: `columns`, `changes` |
//! | `expone` | lo que sale con su tipo: [`vistas::expone_en`] + las `columns` de la raíz |
//! | `detalle` | la `Table`: `object`, `datasource`, `reads`, `changes`, `columns`, y **`vistaInducida`** (0034 ⑤ 5: la View identidad que el inductor dejó sobre ella —`__` en el fichero— es un detalle de la tabla y **no un ítem**) |
//! | `puntero` | el resumen de `datasets/<p>_<n>.json`, si lo hay |
//! | `relaciones` | tipadas y **en las dos direcciones**, de lo que el documento dice: `from` → `sale_de`/`produce`; `backedBy` → `respaldada_por`/`respalda`; `over`/`reads` → `lee`/`leido_por`; `effects.writes` → `escribe`/`escrito_por`; `trainedFrom` → `sale_de`/`produce`; `implements` → `satisface`/`satisfecha_por`; `is` (el concepto de una propiedad) → `nombra`/`nombrado_por`; `model` → `usa`/`usado_por`. Una ref que no resuelve va con `rota: true`: el índice enseña lo que hay, no lo arregla |
//! | `acceso` | del plano de datos: `clasificacion` (las labels del documento; en una Entity, las efectivas de sus propiedades; en lo que lee una tabla, las de las columnas que usa) y `conductos` (si `materialization.payload` compila para lo que copia, por [`flow::check`]). La concesión de `ore-iam` **no** entra: es del plano de control |
//! | `version` | `null` aquí: la pone quien tiene la forja (ore-serve), por fichero |
use crate::document::Kind;
use crate::json::Json;
use crate::link::{Loaded, Package};
use crate::parse::Node;
use crate::vistas::{self, Fuente};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A qué cabeza del árbol responde el índice.
#[derive(Debug, Clone, Default)]
pub struct Cabeza {
    pub cabeza: Option<String>,
    pub rama: Option<String>,
    pub generado: Option<String>,
}

const CARPETAS_DE_KIND: &[&str] = &[
    "tables",
    "views",
    "datasets",
    "entities",
    "functions",
    "actions",
    "models",
    "interfaces",
    "concepts",
];

fn es_item(k: Kind) -> bool {
    matches!(
        k,
        Kind::Dataset
            | Kind::Table
            | Kind::View
            | Kind::Entity
            | Kind::Interface
            | Kind::Concept
            | Kind::Function
            | Kind::Action
            | Kind::TrainedModel
            | Kind::Model
    )
}

fn kind_en_ref(k: Kind) -> &'static str {
    match k {
        Kind::Dataset => "dataset",
        Kind::Table => "table",
        Kind::View => "view",
        Kind::Entity => "entity",
        Kind::Interface => "interface",
        Kind::Concept => "concept",
        Kind::Function => "function",
        Kind::Action => "action",
        Kind::TrainedModel => "trainedmodel",
        Kind::Model => "model",
        _ => "otro",
    }
}

fn kind_nombre(k: Kind) -> &'static str {
    match k {
        Kind::Dataset => "Dataset",
        Kind::Table => "Table",
        Kind::View => "View",
        Kind::Entity => "Entity",
        Kind::Interface => "Interface",
        Kind::Concept => "Concept",
        Kind::Function => "Function",
        Kind::Action => "Action",
        Kind::TrainedModel => "TrainedModel",
        Kind::Model => "Model",
        _ => "?",
    }
}

/// `kind:namespace.name`, la dirección de un ítem.
pub fn ref_de(kind: Kind, ns: Option<&str>, name: &str) -> String {
    match ns {
        Some(ns) if !ns.is_empty() => format!("{}:{ns}.{name}", kind_en_ref(kind)),
        _ => format!("{}:{name}", kind_en_ref(kind)),
    }
}

/// `p.x` o `x` (con el namespace de quien enlaza) → la ref con el kind que
/// quien enlaza espera.
fn ref_qn(kind: Kind, texto: &str, ns: Option<&str>) -> String {
    match texto.split_once('.') {
        Some((p, n)) => ref_de(kind, Some(p), n),
        None => ref_de(kind, ns, texto),
    }
}

fn meta_str(d: &Loaded, k: &str) -> Option<String> {
    d.meta(k).and_then(|n| n.as_str()).map(str::to_string)
}

fn spec_str(d: &Loaded, k: &str) -> Option<String> {
    d.section(k).and_then(|n| n.as_str()).map(str::to_string)
}

fn labels_de(n: &Node) -> BTreeMap<String, Json> {
    n.get("labels")
        .map(|(_, l)| {
            l.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), Json::s(v.as_str()?))))
                .collect()
        })
        .unwrap_or_default()
}

/// `packages/<p>/<carpeta…>/<fichero>` → (paquete, carpeta).
fn paquete_y_carpeta(pkg: &Package, d: &Loaded) -> (Option<String>, String) {
    let rel = d.path.strip_prefix(&pkg.root).unwrap_or(&d.path);
    let partes: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if partes.len() < 3 || partes[0] != "packages" {
        return (None, String::new());
    }
    // Las carpetas del kind (`views/`, `tables/`…) no cuentan, estén donde
    // estén: `espana/views/x.yaml` es el schema `espana`.
    let entre: Vec<&str> = partes[2..partes.len() - 1]
        .iter()
        .map(String::as_str)
        .filter(|c| !CARPETAS_DE_KIND.contains(c))
        .collect();
    (Some(partes[1].clone()), entre.join("/"))
}

fn ruta_de(pkg: &Package, d: &Loaded) -> String {
    d.path
        .strip_prefix(&pkg.root)
        .unwrap_or(&d.path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Los tipos de las columnas de una `Table`: nombre → `type` (y `physicalType`).
fn tipos_de_tabla(t: &Loaded) -> BTreeMap<String, (Option<String>, Option<String>)> {
    t.section("columns")
        .map(|c| {
            c.entries()
                .iter()
                .filter_map(|(k, v)| {
                    let tipo = v
                        .get("type")
                        .and_then(|(_, x)| x.as_str())
                        .map(str::to_string);
                    let fisico = v
                        .get("physicalType")
                        .and_then(|(_, x)| x.as_str())
                        .map(str::to_string);
                    Some((k.as_str()?.to_string(), (tipo, fisico)))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn tabla_fisica<'a>(pkg: &'a Package, datasource: &str, objeto: &str) -> Option<&'a Loaded> {
    pkg.tables().find(|t| {
        spec_str(t, "datasource").as_deref() == Some(datasource)
            && spec_str(t, "object").as_deref() == Some(objeto)
    })
}

/// Sin `where`/`groupBy`/`having`, y expone lo de abajo con sus nombres.
fn es_identidad(pkg: &Package, d: &Loaded) -> bool {
    if ["where", "groupBy", "having"]
        .iter()
        .any(|k| d.section(k).is_some())
    {
        return false;
    }
    let expone = vistas::expone_en(pkg, d);
    if expone.iter().any(|(a, b)| a != b) {
        return false;
    }
    let abajo: BTreeSet<String> = match vistas::fuente(d) {
        Some(Fuente::Tabla(qn)) => pkg.table(&qn).map(vistas::columnas).unwrap_or_default(),
        Some(Fuente::Vista(qn)) => pkg
            .view(&qn)
            .map(|v| vistas::expone_en(pkg, v).into_keys().collect())
            .unwrap_or_default(),
        Some(Fuente::Dataset(qn)) => pkg
            .dataset(&qn)
            .map(|v| vistas::expone_en(pkg, v).into_keys().collect())
            .unwrap_or_default(),
        _ => return false,
    };
    !abajo.is_empty() && expone.keys().cloned().collect::<BTreeSet<_>>() == abajo
}

/// La View identidad que el inductor dejó sobre una tabla: `__` en el
/// fichero, `from: {table}` a ella, identidad. Es un detalle de la tabla.
fn vista_inducida_de<'a>(pkg: &'a Package, t: &Loaded) -> Option<&'a Loaded> {
    let tqn = t.qname()?;
    pkg.of(Kind::View).find(|v| {
        matches!(vistas::fuente(v), Some(Fuente::Tabla(ref q)) if *q == tqn)
            && v.path
                .file_name()
                .is_some_and(|f| f.to_string_lossy().contains("__"))
            && es_identidad(pkg, v)
    })
}

fn define_de(pkg: &Package, d: &Loaded) -> Option<Json> {
    if d.kind == Kind::Dataset && vistas::es_escrito(d) {
        let mut m = vec![("columns", Json::Int(vistas::columnas(d).len() as i64))];
        if let Some(c) = d.section("changes") {
            m.push(("changes", Json::de_node(c)));
        }
        return Some(Json::obj(m));
    }
    if !matches!(d.kind, Kind::View | Kind::Dataset) {
        return None;
    }
    let ns = meta_str(d, "namespace");
    let from = match vistas::fuente(d)? {
        Fuente::Tabla(qn) => ref_qn(Kind::Table, &qn, ns.as_deref()),
        Fuente::Vista(qn) => ref_qn(Kind::View, &qn, ns.as_deref()),
        Fuente::Dataset(qn) => ref_qn(Kind::Dataset, &qn, ns.as_deref()),
        Fuente::Datasource { datasource, objeto } => format!("datasource:{datasource}/{objeto}"),
    };
    let campos = d
        .section("fields")
        .map(|f| f.entries().len() as i64)
        .unwrap_or_else(|| vistas::expone_en(pkg, d).len() as i64);
    let mut m = vec![
        ("from", Json::s(from)),
        ("identidad", Json::Bool(es_identidad(pkg, d))),
        ("fields", Json::Int(campos)),
        ("where", Json::Bool(d.section("where").is_some())),
        ("groupBy", Json::Bool(d.section("groupBy").is_some())),
        ("having", Json::Bool(d.section("having").is_some())),
    ];
    if let Some(f) = spec_str(d, "freshness") {
        m.push(("freshness", Json::s(f)));
    }
    Some(Json::obj(m))
}

fn expone_de(pkg: &Package, d: &Loaded) -> Json {
    let columna = |name: &str, tipo: Option<String>| {
        let mut m = vec![("name", Json::s(name))];
        if let Some(t) = tipo {
            m.push(("type", Json::s(t)));
        }
        Json::obj(m)
    };
    match d.kind {
        Kind::Table => {
            let tipos = tipos_de_tabla(d);
            Json::Arr(
                tipos
                    .into_iter()
                    .map(|(n, (t, f))| {
                        let mut m = vec![("name", Json::s(&n))];
                        if let Some(t) = t {
                            m.push(("type", Json::s(t)));
                        }
                        if let Some(f) = f {
                            m.push(("physicalType", Json::s(f)));
                        }
                        Json::obj(m)
                    })
                    .collect(),
            )
        }
        Kind::View | Kind::Dataset => {
            // El tipo de cada campo: el de su columna en la raíz.
            let tipos_raiz = vistas::raiz(pkg, d)
                .ok()
                .and_then(|r| {
                    let t = tabla_fisica(pkg, &r.datasource, &r.objeto)?;
                    let tipos = tipos_de_tabla(t);
                    Some(
                        r.columnas
                            .into_iter()
                            .filter_map(|(campo, fisica)| {
                                Some((campo, tipos.get(&fisica)?.0.clone()?))
                            })
                            .collect::<BTreeMap<_, _>>(),
                    )
                })
                .unwrap_or_default();
            let escrito = if vistas::es_escrito(d) {
                tipos_de_tabla(d)
            } else {
                BTreeMap::new()
            };
            Json::Arr(
                vistas::expone_en(pkg, d)
                    .into_keys()
                    .map(|c| {
                        let tipo = tipos_raiz
                            .get(&c)
                            .cloned()
                            .or_else(|| escrito.get(&c).and_then(|x| x.0.clone()));
                        columna(&c, tipo)
                    })
                    .collect(),
            )
        }
        Kind::Entity => Json::Arr(
            d.section("properties")
                .map(|p| {
                    p.entries()
                        .iter()
                        .filter_map(|(k, v)| {
                            let tipo = v
                                .get("type")
                                .and_then(|(_, x)| x.as_str())
                                .map(str::to_string);
                            Some(columna(k.as_str()?, tipo))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        _ => Json::Arr(Vec::new()),
    }
}

fn detalle_de(pkg: &Package, d: &Loaded) -> Option<Json> {
    if d.kind != Kind::Table {
        return None;
    }
    let mut m: Vec<(&'static str, Json)> = Vec::new();
    for k in ["object", "datasource", "profile"] {
        if let Some(v) = spec_str(d, k) {
            m.push((
                match k {
                    "object" => "object",
                    "datasource" => "datasource",
                    _ => "profile",
                },
                Json::s(v),
            ));
        }
    }
    for k in ["reads", "changes"] {
        if let Some(n) = d.section(k) {
            m.push((
                if k == "reads" { "reads" } else { "changes" },
                Json::de_node(n),
            ));
        }
    }
    if let Some(v) = vista_inducida_de(pkg, d) {
        m.push((
            "vistaInducida",
            Json::s(ref_de(
                Kind::View,
                meta_str(v, "namespace").as_deref(),
                &meta_str(v, "name").unwrap_or_default(),
            )),
        ));
    }
    Some(Json::obj(m))
}

fn puntero_de(punteros: &BTreeMap<String, Json>, d: &Loaded) -> Json {
    let clave = format!(
        "{}_{}",
        meta_str(d, "namespace").unwrap_or_default(),
        meta_str(d, "name").unwrap_or_default()
    );
    let Some(Json::Obj(p)) = punteros.get(&clave) else {
        return Json::Crudo("null".into());
    };
    let mut m: BTreeMap<String, Json> = BTreeMap::new();
    for k in [
        "estado",
        "motivo",
        "filas",
        "snapshot",
        "metadata_location",
        "cuando",
        "escrito_por",
    ] {
        if let Some(v) = p.get(k) {
            m.insert(k.to_string(), v.clone());
        }
    }
    if let Some(v) = p.get("dataset") {
        m.insert("ubicacion".into(), v.clone());
    }
    if let Some(Json::Obj(pr)) = p.get("procedencia")
        && let Some(l) = pr.get("leidas")
    {
        m.insert("leidas".into(), l.clone());
    }
    Json::Obj(m)
}

/// Una arista, con su inversa.
struct Arista {
    de: String,
    tipo: &'static str,
    a: String,
    inverso: &'static str,
}

fn aristas_de(pkg: &Package, d: &Loaded, punteros: &BTreeMap<String, Json>) -> Vec<Arista> {
    let ns = meta_str(d, "namespace");
    let ns = ns.as_deref();
    let yo = ref_de(d.kind, ns, &meta_str(d, "name").unwrap_or_default());
    let mut out = Vec::new();
    let mut a = |tipo: &'static str, inverso: &'static str, destino: String| {
        out.push(Arista {
            de: yo.clone(),
            tipo,
            a: destino,
            inverso,
        });
    };
    match d.kind {
        Kind::View | Kind::Dataset => {
            match vistas::fuente(d) {
                Some(Fuente::Tabla(qn)) => a("sale_de", "produce", ref_qn(Kind::Table, &qn, ns)),
                Some(Fuente::Vista(qn)) => a("sale_de", "produce", ref_qn(Kind::View, &qn, ns)),
                Some(Fuente::Dataset(qn)) => {
                    a("sale_de", "produce", ref_qn(Kind::Dataset, &qn, ns))
                }
                _ => {}
            }
            // Un escrito: de dónde salió lo lee su puntero (procedencia.leidas).
            if d.kind == Kind::Dataset && vistas::es_escrito(d) {
                let clave = format!(
                    "{}_{}",
                    ns.unwrap_or_default(),
                    meta_str(d, "name").unwrap_or_default()
                );
                if let Some(Json::Obj(p)) = punteros.get(&clave)
                    && let Some(Json::Obj(pr)) = p.get("procedencia")
                    && let Some(Json::Arr(leidas)) = pr.get("leidas")
                {
                    for l in leidas {
                        if let Json::Str(qn) = l {
                            let destino = if pkg.dataset(qn).is_some() {
                                ref_qn(Kind::Dataset, qn, ns)
                            } else {
                                ref_qn(Kind::View, qn, ns)
                            };
                            a("sale_de", "produce", destino);
                        }
                    }
                }
            }
        }
        Kind::Entity => {
            if let Some(b) = spec_str(d, "backedBy") {
                let destino = match vistas::respaldo(pkg, d) {
                    Some(r) => ref_de(
                        r.kind,
                        meta_str(r, "namespace").as_deref(),
                        &meta_str(r, "name").unwrap_or_default(),
                    ),
                    None => ref_qn(Kind::View, &b, ns),
                };
                a("respaldada_por", "respalda", destino);
            }
            if let Some(im) = d.section("implements") {
                for i in im.items() {
                    if let Some(s) = i.as_str() {
                        a(
                            "satisface",
                            "satisfecha_por",
                            ref_qn(Kind::Interface, s, ns),
                        );
                    }
                }
            }
            // `is: gdpr.personalEmail`: el nombre es mío, el significado es suyo.
            if let Some(props) = d.section("properties") {
                for (_, v) in props.entries() {
                    if let Some((_, c)) = v.get("is")
                        && let Some(s) = c.as_str()
                    {
                        a("nombra", "nombrado_por", ref_qn(Kind::Concept, s, ns));
                    }
                }
            }
        }
        Kind::Function | Kind::Action => {
            if let Some(o) = spec_str(d, "over") {
                a("lee", "leido_por", ref_qn(Kind::View, &o, ns));
            }
            if let Some(r) = d.section("reads") {
                for i in r.items() {
                    if let Some(s) = i.as_str() {
                        a("lee", "leido_por", ref_qn(Kind::View, s, ns));
                    }
                }
            }
            // Una Function escribe por `effects[].writes`; una Action, por `sets[].writes`.
            for seccion in ["effects", "sets"] {
                if let Some(e) = d.section(seccion) {
                    for i in e.items() {
                        if let Some((_, w)) = i.get("writes")
                            && let Some(s) = w.as_str()
                        {
                            // `ns.Entidad.propiedad` → la entidad
                            let ent = s.rsplit_once('.').map(|(e, _)| e).unwrap_or(s);
                            a("escribe", "escrito_por", ref_qn(Kind::Entity, ent, ns));
                        }
                    }
                }
            }
            if let Some(m) = spec_str(d, "model") {
                let n = m.rsplit('/').next().unwrap_or(&m);
                a("usa", "usado_por", ref_de(Kind::Model, None, n));
            }
        }
        Kind::TrainedModel => {
            // `trainedFrom: [ventas.pedidos]`: una lista (o un nombre).
            let de: Vec<String> = match d.section("trainedFrom") {
                Some(n) if n.as_str().is_some() => vec![n.as_str().unwrap_or_default().to_string()],
                Some(n) => n
                    .items()
                    .iter()
                    .filter_map(|i| i.as_str().map(str::to_string))
                    .collect(),
                None => Vec::new(),
            };
            for t in de {
                let destino = if pkg.dataset(&t).is_some() {
                    ref_qn(Kind::Dataset, &t, ns)
                } else {
                    ref_qn(Kind::View, &t, ns)
                };
                a("sale_de", "produce", destino);
            }
        }
        _ => {}
    }
    out
}

/// La clasificación del ítem: retículo → nivel más alto que lleva.
fn clasificacion_de(
    pkg: &Package,
    d: &Loaded,
    lat: &BTreeMap<String, crate::flow::Lattice>,
    efectivas: &BTreeMap<String, BTreeMap<String, String>>,
    con_origen: &BTreeMap<String, crate::flow::EntityLabels>,
) -> BTreeMap<String, Json> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut sube = |eje: &str, nivel: &str| {
        let mas_alto = match (out.get(eje), lat.get(eje)) {
            (Some(actual), Some(l)) => l.index(nivel) > l.index(actual),
            (None, _) => true,
            (Some(_), None) => false,
        };
        if mas_alto {
            out.insert(eje.to_string(), nivel.to_string());
        }
    };
    // Las del documento.
    if let Some(m) = d.meta("labels") {
        for (k, v) in m.entries() {
            if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                sube(k, v);
            }
        }
    }
    match d.kind {
        Kind::Entity => {
            let qn = d.qname().unwrap_or_default();
            for (prop, ls) in efectivas {
                if prop.rsplit_once('.').map(|(e, _)| e) == Some(qn.as_str()) {
                    for (eje, nivel) in ls {
                        sube(eje, nivel);
                    }
                }
            }
        }
        Kind::Table => {
            if let Some(c) = d.section("columns") {
                for (_, v) in c.entries() {
                    for (eje, nivel) in labels_de(v) {
                        if let Json::Str(n) = nivel {
                            sube(&eje, &n);
                        }
                    }
                }
            }
        }
        Kind::View | Kind::Dataset => {
            // Lo que lleva su carga: las columnas de la raíz que usa, lo que
            // el datasource etiqueta y lo que las entidades de su cadena
            // declaran (`flow::carga_de`, las mismas vías que el conducto de
            // la copia y el de la lectura desde un puesto: W3.7 gobierno ②).
            for (eje, nivel) in crate::flow::clasificacion_de_carga(
                lat,
                &crate::flow::carga_de(pkg, lat, con_origen, d),
            ) {
                sube(&eje, &nivel);
            }
        }
        _ => {}
    }
    out.into_iter().map(|(k, v)| (k, Json::s(v))).collect()
}

/// El índice de assets de un árbol compilado.
///
/// `punteros`: `datasets/<p>_<n>.json` ya leídos, por su nombre de fichero sin
/// extensión. Los lee quien llama (ore-serve, el CLI): el núcleo no sabe de
/// ficheros de estado.
pub fn indice(pkg: &Package, punteros: &BTreeMap<String, Json>, cabeza: &Cabeza) -> Json {
    let lat = crate::flow::lattices(pkg);
    let efectivas = crate::flow::efectivas(pkg, &lat);
    let con_origen = crate::flow::efectivas_con_origen(pkg, &lat);
    // Lo que copia y no compila: por fichero, desde el chequeo de flujo.
    let flujo_roto: BTreeSet<std::path::PathBuf> = crate::flow::check(pkg)
        .into_iter()
        .filter(|d| {
            matches!(
                d.code,
                crate::code::Code::Oos4011 | crate::code::Code::Oos4002
            )
        })
        .map(|d| d.file)
        .collect();

    // Los ítems: los documentos con kind de ②, menos las vistas inducidas.
    let inducidas: BTreeSet<std::path::PathBuf> = pkg
        .tables()
        .filter_map(|t| vista_inducida_de(pkg, t))
        .map(|v| v.path.clone())
        .collect();
    let docs: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| es_item(d.kind) && !inducidas.contains(&d.path))
        .collect();

    let mut items: BTreeMap<String, BTreeMap<String, Json>> = BTreeMap::new();
    let mut aristas: Vec<Arista> = Vec::new();
    let mut por_paquete: BTreeMap<String, (BTreeSet<String>, i64)> = BTreeMap::new();

    for d in &docs {
        let ns = meta_str(d, "namespace");
        let name = meta_str(d, "name").unwrap_or_default();
        let r = ref_de(d.kind, ns.as_deref(), &name);
        let (paquete, carpeta) = paquete_y_carpeta(pkg, d);
        if let Some(p) = &paquete {
            let e = por_paquete.entry(p.clone()).or_default();
            e.0.insert(carpeta.clone());
            e.1 += 1;
        }
        let mut it: BTreeMap<String, Json> = BTreeMap::new();
        it.insert("ref".into(), Json::s(&r));
        it.insert("kind".into(), Json::s(kind_nombre(d.kind)));
        it.insert(
            "namespace".into(),
            ns.as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        );
        it.insert("name".into(), Json::s(&name));
        it.insert(
            "displayName".into(),
            meta_str(d, "x-rubix-displayName")
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        );
        it.insert(
            "description".into(),
            meta_str(d, "description")
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        );
        it.insert(
            "owner".into(),
            spec_str(d, "owner")
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        );
        it.insert(
            "labels".into(),
            Json::Obj(
                d.root
                    .get("metadata")
                    .map(|(_, m)| labels_de(m))
                    .unwrap_or_default(),
            ),
        );
        it.insert(
            "paquete".into(),
            paquete
                .as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        );
        it.insert("carpeta".into(), Json::s(&carpeta));
        it.insert("ruta".into(), Json::s(ruta_de(pkg, d)));
        if let Some(def) = define_de(pkg, d) {
            it.insert("define".into(), def);
        }
        it.insert("expone".into(), expone_de(pkg, d));
        if let Some(det) = detalle_de(pkg, d) {
            it.insert("detalle".into(), det);
        }
        if d.kind == Kind::Dataset {
            it.insert("puntero".into(), puntero_de(punteros, d));
        }
        // acceso
        let mut acceso: Vec<(&'static str, Json)> = vec![(
            "clasificacion",
            Json::Obj(clasificacion_de(pkg, d, &lat, &efectivas, &con_origen)),
        )];
        if vistas::es_copia(d) {
            acceso.push((
                "conductos",
                Json::obj([(
                    "materialization.payload",
                    Json::s(if flujo_roto.contains(&d.path) {
                        "no compila"
                    } else {
                        "compila"
                    }),
                )]),
            ));
        }
        it.insert("acceso".into(), Json::obj(acceso));
        it.insert("version".into(), Json::Crudo("null".into()));
        it.insert("relaciones".into(), Json::Arr(Vec::new()));
        items.insert(r, it);
        aristas.extend(aristas_de(pkg, d, punteros));
    }

    // Las relaciones, en las dos direcciones.
    let mut rel: BTreeMap<String, Vec<Json>> = BTreeMap::new();
    for a in &aristas {
        let rota = !items.contains_key(&a.a);
        let mut ida = vec![("tipo", Json::s(a.tipo)), ("ref", Json::s(&a.a))];
        if rota {
            ida.push(("rota", Json::Bool(true)));
        }
        rel.entry(a.de.clone()).or_default().push(Json::obj(ida));
        if !rota {
            rel.entry(a.a.clone()).or_default().push(Json::obj([
                ("tipo", Json::s(a.inverso)),
                ("ref", Json::s(&a.de)),
            ]));
        }
    }
    for (r, lista) in rel {
        if let Some(it) = items.get_mut(&r) {
            it.insert("relaciones".into(), Json::Arr(lista));
        }
    }

    // Los paquetes.
    let mut paquetes: Vec<Json> = Vec::new();
    for p in pkg.docs.iter().filter(|d| d.kind == Kind::Package) {
        let (nombre, _) = paquete_y_carpeta(pkg, p);
        let Some(nombre) = nombre else { continue };
        let dir = pkg.root.join("packages").join(&nombre);
        let (fuente, elegido, clase) = scope_de(&dir);
        let (carpetas, n) = por_paquete.get(&nombre).cloned().unwrap_or_default();
        let mut m = vec![
            ("name", Json::s(&nombre)),
            ("type", Json::s(clase)),
            ("scoped", Json::Bool(elegido)),
            (
                "owner",
                spec_str(p, "owner")
                    .map(Json::s)
                    .unwrap_or(Json::Crudo("null".into())),
            ),
            (
                "carpetas",
                Json::Arr(carpetas.into_iter().map(Json::s).collect()),
            ),
            ("items", Json::Int(n)),
        ];
        if let Some(f) = fuente {
            m.push(("source", Json::s(f)));
        }
        paquetes.push(Json::obj(m));
    }

    Json::obj([
        (
            "cabeza",
            cabeza
                .cabeza
                .as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
        (
            "rama",
            cabeza
                .rama
                .as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
        (
            "generado",
            cabeza
                .generado
                .as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
        ("paquetes", Json::Arr(paquetes)),
        (
            "items",
            Json::Obj(items.into_iter().map(|(k, v)| (k, Json::Obj(v))).collect()),
        ),
    ])
}

/// `discover.scope.json` (elegido) o `discover.catalog.json` (la fuente entera):
/// de dónde sale el paquete y de qué clase es. Lo mismo que `GET /paquetes`.
fn scope_de(dir: &Path) -> (Option<String>, bool, &'static str) {
    let lee = |f: &str| {
        std::fs::read_to_string(dir.join(f))
            .ok()
            .and_then(|t| crate::parse::parse(&t).ok())
    };
    if let Some(n) = lee("discover.scope.json") {
        let fuente = n
            .get("source")
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string);
        let clase = match n.get("type").and_then(|(_, v)| v.as_str()) {
            Some("standard") => "standard",
            _ => "foreign",
        };
        return (fuente, true, clase);
    }
    let fuente = lee("discover.catalog.json").and_then(|n| {
        n.get("source")
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
    });
    (fuente, false, "foreign")
}
