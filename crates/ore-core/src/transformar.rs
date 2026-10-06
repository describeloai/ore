//! v1alpha25 — **el transform**: el productor declarado de un dataset escrito
//! (ORE 0055, OOS v1alpha25 `01`). Como la función de v1alpha18, el documento
//! `Transform` **se deriva** del código —un `@transform` de Python, que lee
//! `ore-code`, o una sentencia SQL que escribe, que se lee aquí con el
//! [`guion`]— y se coteja con él. Para cada transform, en este orden:
//!
//! 1. que el `entrypoint` esté: el fichero, el `def` de su nivel superior o la
//!    sentencia `n` (`OOS2042`);
//! 2. que se derive: Python que el puesto entiende y argumentos que se leen
//!    sin ejecutar, o un `.sql` que se analiza (`OOS2043`);
//! 3. que el documento sea **el que el código da**, campo a campo; que el
//!    `def` lleve `@transform` o la sentencia escriba; y que cada transform del
//!    código tenga el suyo (`OOS2013`).
//!
//! Y después, sobre los que son el que el código da (`01` §6): cada entrada
//! resuelve —a lo que se lee o a la salida de otro transform— (`OOS2018`); la
//! salida, si resuelve, es algo que el código escribe (`OOS2046`); una salida,
//! un productor (`OOS2047`); y sin ciclos (`OOS2019`).
//!
//! Dónde vive el documento no es de la regla: se encuentra por su
//! `entrypoint`. Lo que baja por él —la etiqueta de las entradas a la salida—
//! es de `flow` ([`entradas_de`]).
//!
//! [`guion`]: crate::sql_del_arbol::guion::guion

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package, cualificar};
use crate::parse::Node;
use crate::promover::{
    carpeta_del_paquete, ficheros_con, no_se_deriva, paquetes_publicables, roto,
};
use crate::sql_del_arbol::guion::{Sentencia, guion};
use ore_code::Derivacion;
use ore_code::lineas::Lineas;
use ore_code::transform::Produccion;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Cómo se arregla un documento que no es el del código.
const DERIVAR: &str = "el documento se deriva del código: regenéralo desde el `@transform` o la \
                       sentencia SQL, o cambia el código";

/// `<ruta>.sql:<n>` → `(ruta, n)`, o `None` si la forma no vale (`01` §4): la
/// ruta, relativa con `/`, sin `..` ni `/` inicial, que termina en `.sql`; `n`,
/// la sentencia, desde 1 y sin ceros delante.
pub fn entrypoint_sql(s: &str) -> Option<(&str, usize)> {
    let (ruta, n) = s.rsplit_once(':')?;
    let ruta_ok = !ruta.starts_with('/')
        && !ruta.contains('\\')
        && !ruta.contains(':')
        && ruta.ends_with(".sql")
        && ruta.len() > ".sql".len()
        && ruta.split('/').all(|t| !t.is_empty() && t != "..");
    let n_ok =
        n.starts_with(|c: char| ('1'..='9').contains(&c)) && n.chars().all(|c| c.is_ascii_digit());
    if !(ruta_ok && n_ok) {
        return None;
    }
    Some((ruta, n.parse().ok()?))
}

/// Lo que un `.sql` dice de sí mismo (`01` §5.2): cuántas sentencias tiene y
/// el transform de cada una que escribe, con su ordinal desde 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guion {
    pub sentencias: usize,
    pub transforms: Vec<(usize, Produccion)>,
}

/// Un `.sql` → sus transforms: uno por cada `create or replace dataset … as
/// select`, `insert into … select` e `insert or replace into … select`. Lo que
/// no escribe —un `select`, una vista, lo que crea algo vacío— cuenta para el
/// ordinal y no es transform. `Err` con el primer motivo si no se analiza.
pub fn derivar_sql(texto: &str, ruta: &str) -> Result<Guion, String> {
    let trozos =
        guion(texto).map_err(|fs| fs.into_iter().next().map(|f| f.mensaje).unwrap_or_default())?;
    let transforms = trozos
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let Sentencia::Unidad(u) = &t.sentencia else {
                return None;
            };
            let e = u.escribe.as_ref()?;
            let mut inputs: Vec<String> = Vec::new();
            for n in &u.lee {
                let r = n.referencia();
                if !inputs.contains(&r) {
                    inputs.push(r);
                }
            }
            Some((
                i + 1,
                Produccion {
                    runtime: "sql",
                    entrypoint: format!("{ruta}:{}", i + 1),
                    descripcion: None,
                    inputs,
                    output: e.destino.referencia(),
                },
            ))
        })
        .collect();
    Ok(Guion {
        sentencias: trozos.len(),
        transforms,
    })
}

/// Si merece la pena analizar un `.sql` buscando transforms: sin `insert` ni
/// `dataset` no hay sentencia que escriba (un filtro, no una respuesta).
fn puede_escribir(fuente: &str) -> bool {
    let f = fuente.to_ascii_lowercase();
    f.contains("insert") || f.contains("dataset")
}

/// Un fichero de código leído una sola vez, aunque lo nombren varios
/// documentos.
enum Leido {
    Python { fuente: String, d: Derivacion },
    Sql(Result<Guion, String>),
}

fn leer<'a>(
    leidos: &'a mut BTreeMap<PathBuf, Option<Leido>>,
    fichero: &Path,
    ruta: &str,
) -> Option<&'a Leido> {
    leidos
        .entry(fichero.to_path_buf())
        .or_insert_with(|| {
            let fuente = std::fs::read_to_string(fichero).ok()?;
            if ruta.ends_with(".sql") {
                Some(Leido::Sql(derivar_sql(&fuente, ruta)))
            } else if ruta.ends_with(".py") {
                let d = ore_code::python::derivar(&fuente, ruta);
                Some(Leido::Python { fuente, d })
            } else {
                None
            }
        })
        .as_ref()
}

/// `OOS2043` por un `.sql` que no se analiza, una vez por fichero.
fn sql_roto(
    fichero: &Path,
    motivo: &str,
    out: &mut Vec<Diagnostic>,
    dichos: &mut BTreeSet<PathBuf>,
) {
    if dichos.insert(fichero.to_path_buf()) {
        out.push(
            Diagnostic::new(
                Code::Oos2043,
                fichero,
                format!("no se analiza, y sus transforms no se derivan: {motivo}"),
            )
            .help(
                "un `.sql` del paquete da un transform por cada sentencia que escribe: \
                 `create or replace dataset … as select`, `insert into … select`, `insert or \
                 replace into … select`",
            ),
        );
    }
}

/// `OOS2043` por un `.py` que no es Python del puesto, una vez por fichero.
fn py_roto(
    fichero: &Path,
    fuente: &str,
    d: &Derivacion,
    out: &mut Vec<Diagnostic>,
    dichos: &mut BTreeSet<PathBuf>,
) -> bool {
    let Some(diag) = roto(fichero, fuente, d) else {
        return false;
    };
    if dichos.insert(fichero.to_path_buf()) {
        out.push(diag);
    }
    true
}

pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let mut leidos: BTreeMap<PathBuf, Option<Leido>> = BTreeMap::new();
    let mut nombrados: BTreeSet<(PathBuf, String)> = BTreeSet::new();
    let mut rotos: BTreeSet<PathBuf> = BTreeSet::new();
    let mut coherentes: Vec<&Loaded> = Vec::new();

    for t in pkg.of(Kind::Transform) {
        // La forma la comprobó el despacho (`OOS1004`): aquí ya es una.
        let runtime = t.section("runtime").and_then(Node::as_str).unwrap_or("");
        let Some(nodo) = t.section("entrypoint") else {
            continue;
        };
        let texto = nodo.as_str().unwrap_or("");
        let leido = match runtime {
            "python" => crate::promover::entrypoint(texto).map(|(r, n)| (r, n.to_string())),
            "sql" => entrypoint_sql(texto).map(|(r, n)| (r, n.to_string())),
            _ => None,
        };
        let Some((ruta, clave)) = leido else { continue };

        // ── OOS2042 · el fichero está ────────────────────────────────────
        let carpeta = carpeta_del_paquete(&t.path, &pkg.root);
        let fichero = carpeta.join(ruta);
        let Some(l) = leer(&mut leidos, &fichero, ruta) else {
            out.push(
                Diagnostic::new(
                    Code::Oos2042,
                    &t.path,
                    format!("`entrypoint: {texto}`: `{ruta}` no está en el paquete"),
                )
                .at(nodo.pos())
                .help(
                    "el documento es de código que no hay. Escribe el fichero, o corrige la \
                     ruta: es relativa a la carpeta del paquete (la de su `package.yaml`)",
                ),
            );
            continue;
        };
        nombrados.insert((fichero.clone(), clave.clone()));

        let produccion = match l {
            Leido::Python { fuente, d } => {
                // ── OOS2042 · el `def` está ──────────────────────────────
                let Some(def) = d.defs.iter().rev().find(|x| x.nombre == clave) else {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2042,
                            &t.path,
                            format!("`{ruta}` no define `def {clave}(…)` en su nivel superior"),
                        )
                        .at(nodo.pos())
                        .help(
                            "el `entrypoint` nombra un `def` del módulo: no un método de una \
                             clase, ni una función dentro de otra",
                        ),
                    );
                    continue;
                };
                // ── OOS2043 · es Python del puesto ───────────────────────
                if py_roto(&fichero, fuente, d, out, &mut rotos) {
                    continue;
                }
                // ── OOS2013 · el `def` es un transform ───────────────────
                if !def.transformada {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2013,
                            &t.path,
                            format!(
                                "`{ruta}:{clave}` no lleva `@transform`, y este documento dice \
                                 que lo es"
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "un transform se marca en el código —`from ore import transform` y \
                             `@transform(inputs=[…], output=…)` sobre el `def`— y el documento \
                             sale de ahí",
                        ),
                    );
                    continue;
                }
                // ── OOS2043 · se deriva ──────────────────────────────────
                let Some(x) = d.transforms.iter().rev().find(|x| x.nombre == clave) else {
                    continue;
                };
                match &x.resultado {
                    Err(fallos) => {
                        no_se_deriva(&fichero, fuente, &clave, fallos, out);
                        continue;
                    }
                    Ok(p) => p,
                }
            }
            Leido::Sql(Err(motivo)) => {
                sql_roto(&fichero, motivo, out, &mut rotos);
                continue;
            }
            Leido::Sql(Ok(g)) => {
                let n: usize = clave.parse().unwrap_or(0);
                // ── OOS2042 · la sentencia está ──────────────────────────
                if n > g.sentencias {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2042,
                            &t.path,
                            format!(
                                "`entrypoint: {texto}`: `{ruta}` tiene {} sentencia{}",
                                g.sentencias,
                                if g.sentencias == 1 { "" } else { "s" }
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "`<ruta>.sql:<n>` nombra la sentencia `n`-ésima del fichero, desde 1, \
                             contando también las que no escriben",
                        ),
                    );
                    continue;
                }
                // ── OOS2013 · la sentencia escribe ───────────────────────
                let Some((_, p)) = g.transforms.iter().find(|(i, _)| *i == n) else {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2013,
                            &t.path,
                            format!(
                                "la sentencia {n} de `{ruta}` no escribe datos, y este documento \
                                 dice que es un transform"
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "un transform de SQL es una sentencia que escribe: `create or replace \
                             dataset … as select`, `insert into … select` o `insert or replace \
                             into … select`",
                        ),
                    );
                    continue;
                };
                p
            }
        };

        // ── OOS2013 · y es este documento ────────────────────────────────
        if coherencia(t, produccion, out) {
            coherentes.push(t);
        }
    }

    // ── OOS2013 · cada transform del código tiene su documento ───────────
    //
    // Como con `@function`: salvo en un paquete que no puede publicar
    // (`OOS2030`), donde el código es de la sesión, sin más.
    for (carpeta, _) in paquetes_publicables(pkg) {
        let mut ficheros = Vec::new();
        ficheros_con(&carpeta, &[".py", ".sql"], &mut ficheros);
        ficheros.sort();
        for f in ficheros {
            if carpeta_del_paquete(&f, &pkg.root) != carpeta {
                continue; // de un paquete de dentro: lo mira él
            }
            let Ok(rel) = f.strip_prefix(&carpeta) else {
                continue;
            };
            let ruta = rel.to_string_lossy().replace('\\', "/");
            if !leidos.contains_key(&f)
                && !std::fs::read_to_string(&f).is_ok_and(|t| {
                    if ruta.ends_with(".sql") {
                        puede_escribir(&t)
                    } else {
                        ore_code::puede_tener_transforms(&t)
                    }
                })
            {
                continue;
            }
            let Some(l) = leer(&mut leidos, &f, &ruta) else {
                continue;
            };
            match l {
                Leido::Python { fuente, d } => {
                    let sin_documento: Vec<_> = d
                        .transforms
                        .iter()
                        .filter(|x| !nombrados.contains(&(f.clone(), x.nombre.clone())))
                        .collect();
                    if sin_documento.is_empty() || py_roto(&f, fuente, d, out, &mut rotos) {
                        continue;
                    }
                    let lineas = Lineas::new(fuente);
                    for x in sin_documento {
                        match &x.resultado {
                            Err(fallos) => no_se_deriva(&f, fuente, &x.nombre, fallos, out),
                            Ok(p) => out.push(
                                Diagnostic::new(
                                    Code::Oos2013,
                                    &f,
                                    format!(
                                        "`@transform` `{}` sin su documento: ningún `Transform` \
                                         del paquete tiene `entrypoint: {}`",
                                        x.nombre, p.entrypoint
                                    ),
                                )
                                .at(crate::promover::pos(&lineas, x.rango))
                                .help(DERIVAR),
                            ),
                        }
                    }
                }
                Leido::Sql(Err(motivo)) => sql_roto(&f, motivo, out, &mut rotos),
                Leido::Sql(Ok(g)) => {
                    for (n, p) in &g.transforms {
                        if nombrados.contains(&(f.clone(), n.to_string())) {
                            continue;
                        }
                        out.push(
                            Diagnostic::new(
                                Code::Oos2013,
                                &f,
                                format!(
                                    "la sentencia {n} escribe `{}` y no tiene su documento: \
                                     ningún `Transform` del paquete tiene `entrypoint: {}`",
                                    p.output, p.entrypoint
                                ),
                            )
                            .help(DERIVAR),
                        );
                    }
                }
            }
        }
    }

    resolver(pkg, &coherentes, out);
}

// ── OOS2013 · el documento es el que el código da ───────────────────────────

/// Cada diferencia entre el documento y lo que el código da, en su sitio.
/// `true` si no hay ninguna.
fn coherencia(t: &Loaded, p: &Produccion, out: &mut Vec<Diagnostic>) -> bool {
    let antes = out.len();
    let qn = t.qname().unwrap_or_default();
    let raiz = t.root.pos();
    let mut dif = |nodo: Option<&Node>, que: String| {
        out.push(
            Diagnostic::new(
                Code::Oos2013,
                &t.path,
                format!("`{qn}` no es el documento que su código da: {que}"),
            )
            .at(nodo.map(Node::pos).unwrap_or(raiz))
            .help(DERIVAR),
        );
    };
    let corto = |s: &str| crate::normalize::a_corto(s).into_owned();

    let nombre = t.meta("name").and_then(Node::as_str).unwrap_or_default();
    if nombre != p.nombre() {
        dif(
            t.meta("name"),
            format!(
                "se llama `{nombre}` y su salida da `{}` (`.` por `__`)",
                p.nombre()
            ),
        );
    }
    let descripcion = t.meta("description").and_then(Node::as_str);
    if descripcion != p.descripcion.as_deref() {
        dif(
            t.meta("description"),
            match (descripcion, p.descripcion.as_deref()) {
                (Some(d), Some(c)) => format!("`description` dice `{d}` y el código, `{c}`"),
                (Some(d), None) => format!("`description: {d}` no está en el código"),
                _ => format!(
                    "falta `description: {}`, la primera línea de la docstring",
                    p.descripcion.as_deref().unwrap_or_default()
                ),
            },
        );
    }
    let runtime = t.section("runtime").and_then(Node::as_str).unwrap_or("");
    if runtime != p.runtime {
        dif(
            t.section("runtime"),
            format!("`runtime: {runtime}` y el código es `{}`", p.runtime),
        );
    }
    let output = t.section("output").and_then(Node::as_str).unwrap_or("");
    if corto(output) != corto(&p.output) {
        dif(
            t.section("output"),
            format!("`output: {output}` y el código escribe `{}`", p.output),
        );
    }
    // Lo que lee, sin orden: un conjunto de lecturas, como `reads`.
    let nodo = t.section("inputs");
    let doc: BTreeSet<String> = nodo
        .map(Node::items)
        .unwrap_or(&[])
        .iter()
        .filter_map(Node::as_str)
        .map(corto)
        .collect();
    let cod: BTreeSet<String> = p.inputs.iter().map(|x| corto(x)).collect();
    for x in doc.difference(&cod) {
        dif(nodo, format!("`inputs` lleva `{x}` y el código no lo lee"));
    }
    for x in cod.difference(&doc) {
        dif(
            nodo,
            format!("a `inputs` le falta `{x}`, que el código lee"),
        );
    }
    out.len() == antes
}

// ── OOS2018 · OOS2046 · OOS2047 · OOS2019 · lo que resuelve ─────────────────

/// La salida de un transform, en forma corta.
fn salida(t: &Loaded) -> Option<String> {
    Some(cualificar(t.section("output")?.as_str()?, t))
}

/// Sus entradas, en forma corta, con el nodo de cada una.
fn entradas(t: &Loaded) -> Vec<(String, &Node)> {
    t.section("inputs")
        .map(Node::items)
        .unwrap_or(&[])
        .iter()
        .filter_map(|n| Some((cualificar(n.as_str()?, t), n)))
        .collect()
}

fn resolver(pkg: &Package, ts: &[&Loaded], out: &mut Vec<Diagnostic>) {
    // Lo que producen los transforms del árbol, aunque esté por nacer: una
    // entrada puede ser la salida de otro (§6), y así se encadena un pipeline
    // antes de su primer build.
    let producido: BTreeSet<String> = pkg.of(Kind::Transform).filter_map(salida).collect();

    for t in ts {
        let qn = t.qname().unwrap_or_default();
        // ── OOS2018 · cada entrada resuelve ──────────────────────────────
        for (e, nodo) in entradas(t) {
            let resuelve = pkg.table(&e).is_some()
                || pkg.view(&e).is_some()
                || pkg.dataset(&e).is_some()
                || pkg.collection(&e).is_some()
                || producido.contains(&e);
            if !resuelve {
                out.push(
                    Diagnostic::new(
                        Code::Oos2018,
                        &t.path,
                        format!("`{qn}` lee `{e}`, y no es nada del árbol"),
                    )
                    .at(nodo.pos())
                    .help(
                        "una entrada es una `Table`, una `View`, un `Dataset` o una \
                         `MediaCollection`, o la salida de otro transform. Corrige el nombre en el \
                         código y regenera el documento",
                    ),
                );
            }
        }
        // ── OOS2046 · la salida es algo que el código escribe ────────────
        let Some(s) = salida(t) else { continue };
        let que = if let Some(d) = pkg.dataset(&s) {
            crate::vistas::es_mantenido(d).then_some(
                "un `Dataset` mantenido: lo llena el sistema cumpliendo su `from`, no código",
            )
        } else if let Some(c) = pkg.collection(&s) {
            c.section("from").is_some().then_some(
                "una `MediaCollection` con `from`: sus ficheros salen de su origen, no de código",
            )
        } else if pkg.table(&s).is_some() {
            Some("una `Table`: el puntero a un objeto de un origen, que no guarda bytes")
        } else if pkg.view(&s).is_some() {
            Some("una `View`: una pregunta, sin bytes")
        } else if pkg.object_table(&s).is_some() {
            Some("un `ObjectTable`: el listado de un origen")
        } else {
            None // por nacer: la primera escritura la registra
        };
        if let Some(que) = que {
            out.push(
                Diagnostic::new(
                    Code::Oos2046,
                    &t.path,
                    format!("`{qn}` escribe `{s}`, que es {que}"),
                )
                .at(t.section("output").map(Node::pos).unwrap_or(t.root.pos()))
                .help(
                    "la salida de un transform es un `Dataset` escrito (con `columns`, sin \
                     `from`) o una `MediaCollection` escrita, o un nombre que todavía no existe",
                ),
            );
        }
    }

    // ── OOS2047 · una salida, un productor ───────────────────────────────
    let mut por_salida: BTreeMap<String, Vec<&Loaded>> = BTreeMap::new();
    for t in ts {
        if let Some(s) = salida(t) {
            por_salida.entry(s).or_default().push(t);
        }
    }
    for (s, productores) in &por_salida {
        if productores.len() < 2 {
            continue;
        }
        let donde: Vec<String> = productores
            .iter()
            .filter_map(|t| t.section("entrypoint").and_then(Node::as_str))
            .map(str::to_string)
            .collect();
        for t in productores {
            out.push(
                Diagnostic::new(
                    Code::Oos2047,
                    &t.path,
                    format!(
                        "`{s}` tiene {} productores: {}",
                        productores.len(),
                        donde.join(" · ")
                    ),
                )
                .at(t.section("output").map(Node::pos).unwrap_or(t.root.pos()))
                .help(
                    "una salida, un productor: el transform se nombra por lo que escribe. Que \
                     escriba uno solo, o que cada uno escriba lo suyo",
                ),
            );
        }
    }

    // ── OOS2019 · sin ciclos ─────────────────────────────────────────────
    //
    // Las aristas son `inputs → output` de cada transform y `from → dataset`
    // de cada mantenido (§6). Un transform está en un ciclo si desde su salida
    // se vuelve a una de sus entradas.
    let mut aristas: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in ts {
        let Some(s) = salida(t) else { continue };
        for (e, _) in entradas(t) {
            aristas.entry(e).or_default().insert(s.clone());
        }
    }
    for d in pkg.datasets().filter(|d| crate::vistas::es_mantenido(d)) {
        let Some(qn) = d.qname() else { continue };
        for (_, origen) in d.section("from").map(Node::entries).unwrap_or(&[]) {
            if let Some(o) = origen.as_str() {
                aristas
                    .entry(cualificar(o, d))
                    .or_default()
                    .insert(qn.clone());
            }
        }
    }
    for t in ts {
        let (Some(s), qn) = (salida(t), t.qname().unwrap_or_default()) else {
            continue;
        };
        let es = entradas(t);
        if let Some((_, nodo)) = es.iter().find(|(e, _)| *e == s) {
            out.push(
                Diagnostic::new(
                    Code::Oos2019,
                    &t.path,
                    format!("`{qn}` lee `{s}`, que es lo que escribe"),
                )
                .at(nodo.pos())
                .help(
                    "la salida no es una entrada: leer lo que uno mismo escribió —un \
                     incremental— no se declara como entrada",
                ),
            );
            continue;
        }
        let mut vistos: BTreeSet<&str> = BTreeSet::new();
        let mut pila: Vec<&str> = vec![s.as_str()];
        let mut vuelve = None;
        while let Some(n) = pila.pop() {
            if !vistos.insert(n) {
                continue;
            }
            if let Some((e, nodo)) = es.iter().find(|(e, _)| e == n) {
                vuelve = Some((e.clone(), *nodo));
                break;
            }
            pila.extend(aristas.get(n).into_iter().flatten().map(String::as_str));
        }
        if let Some((e, nodo)) = vuelve {
            out.push(
                Diagnostic::new(
                    Code::Oos2019,
                    &t.path,
                    format!("`{qn}` lee `{e}`, que sale de `{s}`, que es lo que escribe"),
                )
                .at(nodo.pos())
                .help(
                    "el grafo de transforms —y de datasets mantenidos— vuelve sobre sí: ningún \
                     orden de construcción lo cumple. Rompe el ciclo",
                ),
            );
        }
    }
}

// ── el flujo ────────────────────────────────────────────────────────────────

/// v1alpha25 `01` §7: lo que lee el transform que escribe `qn` —cada entrada,
/// con el documento desde el que se resuelve—. Vacío si nadie lo produce. Es
/// lo que `flow` suma a un `derivedFrom`: se sabe antes de ejecutar.
pub fn entradas_de<'a>(pkg: &'a Package, qn: &str) -> Vec<(String, &'a Loaded)> {
    let qn = crate::normalize::a_corto(qn);
    pkg.of(Kind::Transform)
        .filter(|t| salida(t).as_deref() == Some(qn.as_ref()))
        .flat_map(|t| entradas(t).into_iter().map(move |(e, _)| (e, t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_entrypoint_de_sql_es_una_ruta_y_una_sentencia() {
        assert_eq!(entrypoint_sql("etl/c.sql:1"), Some(("etl/c.sql", 1)));
        assert_eq!(entrypoint_sql("c.sql:12"), Some(("c.sql", 12)));
        for malo in [
            "c.sql:0",
            "c.sql:01",
            "c.sql:",
            "c.sql",
            "/c.sql:1",
            "../c.sql:1",
            "a/../c.sql:1",
            "c.py:1",
            ".sql:1",
            "c.sql:-1",
            "c.sql:x",
        ] {
            assert_eq!(entrypoint_sql(malo), None, "{malo}");
        }
    }

    #[test]
    fn una_sentencia_que_escribe_es_un_transform_y_el_ordinal_cuenta_todas() {
        let g = derivar_sql(
            "-- lo que no escribe no es un transform\n\
             SELECT count(*) FROM ventas.pedidos;\n\n\
             CREATE OR REPLACE DATASET ventas.por_cliente AS\n\
             SELECT cliente_id, sum(importe) AS total FROM ventas.pedidos GROUP BY cliente_id;\n\n\
             create view ventas.v as select * from ventas.pedidos;\n\
             INSERT INTO ventas.historico\n\
             SELECT p.pedido_id, c.pais FROM ventas.clientes c JOIN ventas.pedidos p USING \
             (cliente_id);\n\
             insert or replace into ventas.default.ultimos select * from ventas.default.pedidos;\n",
            "etl/cargas.sql",
        )
        .expect("se analiza");
        assert_eq!(g.sentencias, 5);
        let resumen: Vec<(usize, &str, Vec<&str>, &str)> = g
            .transforms
            .iter()
            .map(|(n, p)| {
                (
                    *n,
                    p.entrypoint.as_str(),
                    p.inputs.iter().map(String::as_str).collect(),
                    p.output.as_str(),
                )
            })
            .collect();
        assert_eq!(
            resumen,
            vec![
                (
                    2,
                    "etl/cargas.sql:2",
                    vec!["ventas.pedidos"],
                    "ventas.por_cliente"
                ),
                (
                    4,
                    "etl/cargas.sql:4",
                    vec!["ventas.clientes", "ventas.pedidos"],
                    "ventas.historico"
                ),
                // Tres partes en `default`: la forma corta.
                (
                    5,
                    "etl/cargas.sql:5",
                    vec!["ventas.pedidos"],
                    "ventas.ultimos"
                ),
            ]
        );
        assert!(g.transforms.iter().all(|(_, p)| p.runtime == "sql"));
        assert!(g.transforms.iter().all(|(_, p)| p.descripcion.is_none()));
    }

    fn doc(ruta: &str, kind: Kind, texto: &str) -> Loaded {
        Loaded {
            path: PathBuf::from(ruta),
            kind,
            root: crate::parse::parse(texto).expect("analiza"),
        }
    }

    /// §7: la salida lleva lo que llevan sus entradas antes de la primera
    /// escritura, sin `derivedFrom`. Aquí, una columna de una `Table`.
    #[test]
    fn la_etiqueta_baja_por_el_transform() {
        let base = || {
            vec![
                doc(
                    "lattices/s.yaml",
                    Kind::Lattice,
                    "apiVersion: oos.dev/v1alpha1\nkind: Lattice\n\
                     metadata: { name: sensitivity, namespace: acme }\n\
                     spec:\n  levels: [public, internal, confidential]\n",
                ),
                doc(
                    "packages/ventas/tables/pedidos.yaml",
                    Kind::Table,
                    "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
                     metadata: { name: pedidos, namespace: ventas }\n\
                     spec:\n  datasource: erp\n  object: public.pedidos\n  columns:\n    \
                     importe: { physicalType: numeric, labels: { acme.sensitivity: confidential } }\n",
                ),
                doc(
                    "packages/ventas/datasets/resumen.yaml",
                    Kind::Dataset,
                    "apiVersion: oos.dev/v1alpha12\nkind: Dataset\n\
                     metadata: { name: resumen, namespace: ventas }\n\
                     spec:\n  owner: team:ventas\n  columns:\n    total: { type: Decimal }\n",
                ),
            ]
        };
        let paquete = |docs: Vec<Loaded>| Package {
            root: PathBuf::from("."),
            docs,
            cedar: Vec::new(),
            generated: Vec::new(),
            sobres: Vec::new(),
        };
        let sin = paquete(base());
        assert_eq!(
            crate::flow::clasificacion_de(&sin, "ventas.resumen").get("acme.sensitivity"),
            None
        );
        let mut docs = base();
        docs.push(doc(
            "packages/ventas/etl/pipeline/ventas.resumen.yaml",
            Kind::Transform,
            "apiVersion: oos.dev/v1alpha25\nkind: Transform\n\
             metadata: { name: ventas__resumen, namespace: ventas }\n\
             spec:\n  runtime: python\n  entrypoint: etl/t.py:resumen\n  \
             inputs: [ventas.pedidos]\n  output: ventas.resumen\n",
        ));
        let con = paquete(docs);
        assert_eq!(
            entradas_de(&con, "ventas.default.resumen")
                .iter()
                .map(|(e, _)| e.as_str())
                .collect::<Vec<_>>(),
            ["ventas.pedidos"]
        );
        assert_eq!(
            crate::flow::clasificacion_de(&con, "ventas.resumen")
                .get("acme.sensitivity")
                .map(String::as_str),
            Some("confidential")
        );
    }

    #[test]
    fn un_sql_que_no_se_analiza_no_da_transforms() {
        assert!(derivar_sql("create table x.y as select 1", "a.sql").is_err());
        assert!(derivar_sql("select 'sin cerrar", "a.sql").is_err());
    }
}
