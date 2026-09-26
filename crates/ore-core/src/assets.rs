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
//! | `repositorio` | **en singular** (0035 ⑥): el repositorio donde vive, o `null`. Un proyecto es una lente y se solapa; un repositorio es **el sitio donde se trabaja**, y anidarlos es hondura —se lo queda el más hondo—, no solape |
//! | `carpetas` (del paquete) | **las que existen**, tengan ítems dentro o no (0035 ⑦): la unión de las que los ítems nombran y los directorios que están en el árbol, quitando las del kind. Contarlas por ítems dejaba invisible justo la carpeta que más importa —la recién creada, o la que sólo tiene un repositorio—, y con ella no se puede elegir dónde se guarda lo primero |
//! | `proyectos` | **en plural** (0035 ⑤ 4): qué proyectos NOMBRAN a este ítem, de `proyectos/*/README.md` ([`crate::proyectos`]). Se solapan a propósito: un proyecto es una lente, no una caja, y un ítem puede estar en varias o en ninguna |
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

/// La dirección de un documento: `kind:` + su nombre cualificado en forma
/// corta (v1alpha13: `p.n` en `default`, `p.s.n` en otro schema).
pub fn ref_doc(d: &Loaded) -> String {
    format!("{}:{}", kind_en_ref(d.kind), d.qname().unwrap_or_default())
}

/// `p.x` o `x` (con el namespace de quien enlaza) → la ref con el kind que
/// quien enlaza espera. Si lo nombrado es del catálogo (v1alpha13), una parte
/// es del schema de quien enlaza y tres son completas: su forma corta.
fn ref_qn(kind: Kind, texto: &str, ns: Option<&str>, schema: &str) -> String {
    if kind.con_schema() {
        return format!(
            "{}:{}",
            kind_en_ref(kind),
            crate::normalize::qualify_catalogo(texto, ns, schema)
        );
    }
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

/// Las carpetas que **existen** dentro de un paquete, tengan ítems o no.
///
/// ⭐⭐ El índice las contaba por ítems, y eso dejaba fuera **la carpeta vacía**
///   —que es exactamente la que hace falta para guardar lo PRIMERO—: crearla
///   escribía un commit de verdad y aun así no se podía elegir en ninguna
///   parte. Medido en el árbol de victor (0035 ⑦): `PUT
///   packages/standard_test/mi_carpeta/README.md` daba 201 y salía en
///   `GET /arbol`, y `carpetas` seguía diciendo `[""]`.
///
/// ⛔ Las del kind (`tables/`, `views/`…) no cuentan, igual que en la ruta de
///   un ítem: `espana/views/` es la carpeta `espana`. Y la raíz (`""`) no se
///   añade por estar: la nombran los ítems que caen en ella, como siempre.
fn carpetas_del_paquete(dir: &Path) -> BTreeSet<String> {
    fn anda(dir: &Path, tramo: &[String], hondo: usize, out: &mut BTreeSet<String>) {
        if hondo > 12 {
            return;
        }
        let Ok(entradas) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entradas.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let nombre = e.file_name().to_string_lossy().into_owned();
            if nombre.starts_with('.') {
                continue;
            }
            let mut suyo = tramo.to_vec();
            if !CARPETAS_DE_KIND.contains(&nombre.as_str()) {
                suyo.push(nombre);
                out.insert(suyo.join("/"));
            }
            anda(&e.path(), &suyo, hondo + 1, out);
        }
    }
    let mut out = BTreeSet::new();
    anda(dir, &[], 0, &mut out);
    out
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
    // v1alpha14: una vista SQL define su consulta, en su dialecto, y lo que
    // lee por nombre.
    if vistas::es_sql(d) {
        let lee: Vec<Json> = vistas::lee_directo(pkg, d)
            .into_iter()
            .map(|f| Json::s(ref_doc(f)))
            .collect();
        return Some(Json::obj(vec![
            (
                "dialect",
                Json::s(spec_str(d, "dialect").unwrap_or_default()),
            ),
            ("sql", Json::s(spec_str(d, "sql").unwrap_or_default())),
            ("lee", Json::Arr(lee)),
        ]));
    }
    let ns = meta_str(d, "namespace");
    let sc = d.schema().unwrap_or(crate::normalize::SCHEMA_POR_DEFECTO);
    let from = match vistas::fuente(d)? {
        Fuente::Tabla(qn) => ref_qn(Kind::Table, &qn, ns.as_deref(), sc),
        Fuente::Vista(qn) => ref_qn(Kind::View, &qn, ns.as_deref(), sc),
        Fuente::Dataset(qn) => ref_qn(Kind::Dataset, &qn, ns.as_deref(), sc),
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
            // El contrato de una vista SQL lleva sus tipos, como las `columns`
            // de un escrito.
            let escrito = if vistas::es_escrito(d) || vistas::es_sql(d) {
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
        m.push(("vistaInducida", Json::s(ref_doc(v))));
    }
    Some(Json::obj(m))
}

fn puntero_de(punteros: &BTreeMap<String, Json>, d: &Loaded) -> Json {
    let clave = d.qname().unwrap_or_default();
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
    let sc = d.schema().unwrap_or(crate::normalize::SCHEMA_POR_DEFECTO);
    let yo = ref_doc(d);
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
            if vistas::es_sql(d) {
                for f in vistas::lee_directo(pkg, d) {
                    a("sale_de", "produce", ref_doc(f));
                }
            }
            match vistas::fuente(d) {
                Some(Fuente::Tabla(qn)) => {
                    a("sale_de", "produce", ref_qn(Kind::Table, &qn, ns, sc))
                }
                Some(Fuente::Vista(qn)) => a("sale_de", "produce", ref_qn(Kind::View, &qn, ns, sc)),
                Some(Fuente::Dataset(qn)) => {
                    a("sale_de", "produce", ref_qn(Kind::Dataset, &qn, ns, sc))
                }
                _ => {}
            }
            // Un escrito: de dónde salió lo dice su documento (`derivedFrom`,
            // W3.7 gobierno ③: es por donde baja la clasificación) y, si no lo
            // dice, su puntero (procedencia.leidas: lo escrito antes de que el
            // documento lo llevara). Nunca de sí mismo.
            if d.kind == Kind::Dataset && vistas::es_escrito(d) {
                let propio = d.qname().unwrap_or_default();
                let mut leidos: Vec<String> = d
                    .section("derivedFrom")
                    .map(|df| {
                        df.items()
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if leidos.is_empty() {
                    let clave = d.qname().unwrap_or_default();
                    if let Some(Json::Obj(p)) = punteros.get(&clave)
                        && let Some(Json::Obj(pr)) = p.get("procedencia")
                        && let Some(Json::Arr(leidas)) = pr.get("leidas")
                    {
                        leidos = leidas
                            .iter()
                            .filter_map(|l| match l {
                                Json::Str(s) => Some(s.clone()),
                                _ => None,
                            })
                            .collect();
                    }
                }
                for qn in leidos.iter().filter(|q| **q != propio) {
                    let destino = if pkg.dataset(qn).is_some() {
                        ref_qn(Kind::Dataset, qn, ns, sc)
                    } else {
                        ref_qn(Kind::View, qn, ns, sc)
                    };
                    a("sale_de", "produce", destino);
                }
            }
        }
        Kind::Entity => {
            if let Some(b) = spec_str(d, "backedBy") {
                let destino = match vistas::respaldo(pkg, d) {
                    Some(r) => ref_doc(r),
                    None => ref_qn(Kind::View, &b, ns, sc),
                };
                a("respaldada_por", "respalda", destino);
            }
            if let Some(im) = d.section("implements") {
                for i in im.items() {
                    if let Some(s) = i.as_str() {
                        a(
                            "satisface",
                            "satisfecha_por",
                            ref_qn(Kind::Interface, s, ns, sc),
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
                        a("nombra", "nombrado_por", ref_qn(Kind::Concept, s, ns, sc));
                    }
                }
            }
        }
        Kind::Function | Kind::Action => {
            if let Some(o) = spec_str(d, "over") {
                a("lee", "leido_por", ref_qn(Kind::View, &o, ns, sc));
            }
            if let Some(r) = d.section("reads") {
                for i in r.items() {
                    if let Some(s) = i.as_str() {
                        a("lee", "leido_por", ref_qn(Kind::View, s, ns, sc));
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
                            a("escribe", "escrito_por", ref_qn(Kind::Entity, ent, ns, sc));
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
                    ref_qn(Kind::Dataset, &t, ns, sc)
                } else {
                    ref_qn(Kind::View, &t, ns, sc)
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
/// `punteros`: los de `datasets/` ya leídos, por la forma corta del nombre
/// (`crate::punteros::del_arbol`, 0038 P2). Los lee quien llama (ore-serve, el
/// CLI).
pub fn indice(pkg: &Package, punteros: &BTreeMap<String, Json>, cabeza: &Cabeza) -> Json {
    let proyectos = crate::proyectos::leer(&pkg.root);
    let repositorios = crate::repositorios::leer(&pkg.root);
    let mut de_repositorio: BTreeMap<String, i64> = BTreeMap::new();
    // Qué ítems nombra cada proyecto, y qué proyectos nombran a cada ítem. Lo
    // roto no alcanza nada: un manifiesto que no se entiende no reparte.
    let mut de_proyecto: BTreeMap<String, i64> = BTreeMap::new();
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
        let r = ref_doc(d);
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
        // v1alpha13: el schema que el documento DECLARA (o `default`), que es
        // su nombre; `carpeta` sigue siendo donde está el fichero. `null` en lo
        // que no se ordena en schemas.
        it.insert(
            "schema".into(),
            d.schema()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        );
        let suyos: Vec<Json> = proyectos
            .iter()
            .filter(|p| {
                p.roto.is_none() && p.alcanza(paquete.as_deref().unwrap_or_default(), &carpeta)
            })
            .map(|p| {
                *de_proyecto.entry(p.nombre.clone()).or_default() += 1;
                Json::s(&p.nombre)
            })
            .collect();
        it.insert("proyectos".into(), Json::Arr(suyos));
        // El repositorio donde vive el ítem: el más hondo que lo contiene.
        let suyo = crate::repositorios::de_item(
            &repositorios,
            paquete.as_deref().unwrap_or_default(),
            &carpeta,
        );
        it.insert(
            "repositorio".into(),
            match suyo {
                Some(r) => {
                    *de_repositorio.entry(r.ruta.clone()).or_default() += 1;
                    Json::s(&r.ruta)
                }
                None => Json::Crudo("null".into()),
            },
        );
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

    // Los schemas DECLARADOS de cada paquete (0038 P6d): un schema existe
    // porque un `kind: Schema` lo declara, no por ser una carpeta. `carpetas`
    // sigue diciendo las carpetas —las de Projects y Repositorios (0035/0036)
    // lo son sin ser schemas—; el catálogo pinta `schemas`.
    let mut declarados: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in pkg.docs.iter().filter(|d| d.kind == Kind::Schema) {
        if let (Some(p), Some(n)) = (
            paquete_y_carpeta(pkg, s).0,
            s.meta("name").and_then(|n| n.as_str()),
        ) {
            declarados.entry(p).or_default().insert(n.to_string());
        }
    }

    // Los paquetes.
    let mut paquetes: Vec<Json> = Vec::new();
    for p in pkg.docs.iter().filter(|d| d.kind == Kind::Package) {
        let (nombre, _) = paquete_y_carpeta(pkg, p);
        let Some(nombre) = nombre else { continue };
        let dir = pkg.root.join("packages").join(&nombre);
        let (fuente, elegido, clase) = scope_de(&dir);
        let (mut carpetas, n) = por_paquete.get(&nombre).cloned().unwrap_or_default();
        // ⭐ Y las que están en el árbol sin tener ítems todavía (0035 ⑦).
        carpetas.extend(carpetas_del_paquete(&dir));
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
            // `default` —existe sin declararse— y los declarados, en orden.
            (
                "schemas",
                Json::Arr(
                    std::iter::once(crate::normalize::SCHEMA_POR_DEFECTO.to_string())
                        .chain(declarados.remove(&nombre).unwrap_or_default())
                        .map(Json::s)
                        .collect(),
                ),
            ),
            ("items", Json::Int(n)),
        ];
        if let Some(f) = fuente {
            m.push(("source", Json::s(f)));
        }
        paquetes.push(Json::obj(m));
    }

    // Los proyectos (0035 ①): lo que cada uno nombra y cuántos ítems le tocan.
    let nombres_de_paquete: BTreeSet<String> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|d| paquete_y_carpeta(pkg, d).0)
        .collect();
    let proyectos: Vec<Json> = proyectos
        .iter()
        .map(|p| {
            let mut m = vec![
                ("nombre", Json::s(&p.nombre)),
                (
                    "titulo",
                    p.titulo
                        .as_deref()
                        .map(Json::s)
                        .unwrap_or(Json::Crudo("null".into())),
                ),
                (
                    "descripcion",
                    p.descripcion
                        .as_deref()
                        .map(Json::s)
                        .unwrap_or(Json::Crudo("null".into())),
                ),
                (
                    "contiene",
                    Json::Arr(p.contiene.iter().map(Json::s).collect()),
                ),
                (
                    "items",
                    Json::Int(de_proyecto.get(&p.nombre).copied().unwrap_or(0)),
                ),
                ("ruta", Json::s(&p.ruta)),
                // ⭐⭐ SU SITIO (0035 ⑦.1): el paquete que lleva su nombre, si
                //   está. Es **la raíz del proyecto** —donde nacen sus cosas—,
                //   y va aparte de `contiene` a propósito: lo demás que nombra
                //   es de otros, y ofrecerlo como sitio donde crear fue el
                //   error que ⑦ arregla.
                (
                    "sitio",
                    if nombres_de_paquete.contains(&p.nombre) {
                        Json::s(format!("packages/{}", p.nombre))
                    } else {
                        Json::Crudo("null".into())
                    },
                ),
                ("version", Json::Crudo("null".into())),
            ];
            if let Some(r) = &p.roto {
                m.push(("roto", Json::s(r)));
            }
            Json::obj(m)
        })
        .collect();

    // Las clases del producto (0036 ⑧b): lo que se puede crear, con su versión.
    let clases: Vec<Json> = crate::clases::CLASES
        .iter()
        .map(|c| {
            Json::obj([
                ("id", Json::s(c.id)),
                ("familia", Json::s(c.familia)),
                ("lenguaje", Json::s(c.lenguaje)),
                ("titulo", Json::s(c.titulo)),
                ("descripcion", Json::s(c.descripcion)),
                ("version", Json::Int(c.version)),
                ("escribe", Json::Bool(c.escribe)),
                ("ejecuta", Json::Bool(c.ejecuta)),
            ])
        })
        .collect();

    let familias: Vec<Json> = crate::clases::FAMILIAS
        .iter()
        .map(|f| {
            Json::obj([
                ("id", Json::s(f.id)),
                ("titulo", Json::s(f.titulo)),
                ("descripcion", Json::s(f.descripcion)),
            ])
        })
        .collect();

    // Los repositorios (0035 ⑥): dónde se trabaja, con su clase y su versión.
    let repositorios: Vec<Json> = repositorios
        .iter()
        .map(|r| {
            let mut m = vec![
                ("ruta", Json::s(&r.ruta)),
                (
                    "nombre",
                    r.nombre
                        .as_deref()
                        .map(Json::s)
                        .unwrap_or(Json::Crudo("null".into())),
                ),
                (
                    "plantilla",
                    r.plantilla
                        .as_deref()
                        .map(Json::s)
                        .unwrap_or(Json::Crudo("null".into())),
                ),
                (
                    "plantillaVersion",
                    r.plantilla_version
                        .map(Json::Int)
                        .unwrap_or(Json::Crudo("null".into())),
                ),
                ("paquete", Json::s(&r.paquete)),
                ("carpeta", Json::s(&r.carpeta)),
                (
                    "items",
                    Json::Int(de_repositorio.get(&r.ruta).copied().unwrap_or(0)),
                ),
                ("manifiesto", Json::s(&r.manifiesto)),
                ("version", Json::Crudo("null".into())),
            ];
            // **La versión de la clase** (0036 ⑤): lo que el repositorio dice
            // frente a lo que el producto trae hoy. Es lo que hace verdadera la
            // columna «UPGRADE · Up to date» — y subirla es una propuesta con su
            // diff, no un commit a la brava.
            if let Some(c) = r.plantilla.as_deref().and_then(crate::clases::de) {
                m.push(("plantillaActual", Json::Int(c.version)));
                m.push((
                    "actualizable",
                    Json::Bool(r.plantilla_version.unwrap_or(0) < c.version),
                ));
                m.push(("escribe", Json::Bool(c.escribe)));
                m.push(("ejecuta", Json::Bool(c.ejecuta)));
            } else if r.plantilla.is_some() {
                // Una clase que este producto no conoce: se lista, y se dice.
                // El árbol de un cliente puede venir de una versión posterior.
                m.push(("plantillaActual", Json::Crudo("null".into())));
                m.push(("actualizable", Json::Bool(false)));
            }
            if let Some(x) = &r.roto {
                m.push(("roto", Json::s(x)));
            }
            Json::obj(m)
        })
        .collect();

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
        ("proyectos", Json::Arr(proyectos)),
        ("repositorios", Json::Arr(repositorios)),
        // ⭐ LAS CLASES QUE ESTE PRODUCTO TRAE (0036 ⑧b). No salen del árbol
        //   —son del producto, como `plantillaActual`— y viajan aquí para que
        //   la consola no tenga su propia copia de las cinco tarjetas escrita
        //   a mano: dos descripciones de lo mismo divergen, y la que se quede
        //   vieja ofrecerá algo que el servidor rechazaría.
        ("clases", Json::Arr(clases)),
        // ⭐ Y cómo se agrupan (⑧b): la tarjeta que junta a las que hacen lo
        //   mismo en distintos lenguajes, con su frase — que tampoco la
        //   escribe la consola.
        ("familias", Json::Arr(familias)),
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
    let Some(catalogo) = lee("discover.catalog.json") else {
        // 0039: sin origen —`create standard database`, `ore package new`—,
        // lo que tenga sólo puede vivir en el lago.
        return (None, false, "standard");
    };
    let fuente = catalogo
        .get("source")
        .and_then(|(_, v)| v.as_str())
        .map(str::to_string);
    (fuente, false, "foreign")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Un árbol de mentira que se borra solo.
    struct Arbol(std::path::PathBuf);
    impl Drop for Arbol {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn arbol(caso: &str, ficheros: &[&str]) -> Arbol {
        let d =
            Arbol(std::env::temp_dir().join(format!("ore-assets-{}-{caso}", std::process::id())));
        let _ = std::fs::remove_dir_all(&d.0);
        for ruta in ficheros {
            let p = d.0.join(ruta);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        d
    }

    /// ⭐ Una carpeta **existe por estar**, no por tener ítems: es lo que hace
    ///   que se pueda elegir dónde se guarda lo primero (0035 ⑦).
    #[test]
    fn las_carpetas_que_estan() {
        let d = arbol(
            "carpetas",
            &[
                "packages/hr/package.yaml",
                "packages/hr/views/empleados.yaml",
                "packages/hr/ingesta/README.md",
                "packages/hr/raw/limpio/README.md",
                "packages/hr/espana/views/x.yaml",
                "packages/hr/.oculta/nada.txt",
            ],
        );
        let c = carpetas_del_paquete(&d.0.join("packages").join("hr"));
        assert_eq!(
            c.iter().map(String::as_str).collect::<Vec<_>>(),
            // `views/` es del kind y no cuenta; `espana/views` es `espana`;
            // lo oculto no se lista; y la raíz no se añade por estar.
            vec!["espana", "ingesta", "raw", "raw/limpio"]
        );
    }
}
