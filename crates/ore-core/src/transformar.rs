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
use crate::diag::{Diagnostic, Pos};
use crate::document::Kind;
use crate::link::{Loaded, Package, cualificar};
use crate::parse::Node;
use crate::promover::{carpeta_del_paquete, ficheros_con, paquetes_publicables};
use crate::sql_del_arbol::guion::{Sentencia, guion};
use ore_code::Derivacion;
use ore_code::lineas::Lineas;
use ore_code::transform::Produccion;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Cómo se arregla un documento que no es el del código.
const DERIVAR: &str = "the document is derived from the code: regenerate it from the `@transform` \
                       or the SQL statement (commit the code), or change the code";

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
    /// Dónde está cada transform en el `.sql`, por su ordinal.
    pub sitios: BTreeMap<usize, Sitio>,
}

/// 0055 · **Dónde dice el código lo que un transform declara**: la línea del
/// decorador o de la sentencia, y la de cada entrada y la salida si se saben.
/// Lo que resuelve (`OOS2018`, `OOS2037`, `OOS2046`, `OOS2047`, `OOS2019`)
/// se dice aquí, en el código que se escribe, y no en el documento derivado.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sitio {
    pub pos: Option<Pos>,
    /// Cada entrada, en forma corta, con su sitio.
    pub inputs: Vec<(String, Pos)>,
    pub output: Option<Pos>,
}

/// Por qué un `.sql` no se analiza: el primer motivo, y dónde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoSeAnaliza {
    pub mensaje: String,
    pub pos: Option<Pos>,
}

/// Un `.sql` → sus transforms: uno por cada `create or replace dataset … as
/// select`, `insert into … select` e `insert or replace into … select`. Lo que
/// no escribe —un `select`, una vista, lo que crea algo vacío— cuenta para el
/// ordinal y no es transform. `Err` con el primer motivo si no se analiza.
pub fn derivar_sql(texto: &str, ruta: &str) -> Result<Guion, NoSeAnaliza> {
    let trozos = guion(texto).map_err(|fs| {
        fs.into_iter()
            .next()
            .map(|f| NoSeAnaliza {
                mensaje: f.mensaje,
                pos: f.pos,
            })
            .unwrap_or(NoSeAnaliza {
                mensaje: String::new(),
                pos: None,
            })
    })?;
    let mut sitios = BTreeMap::new();
    for (i, t) in trozos.iter().enumerate() {
        let Sentencia::Unidad(u) = &t.sentencia else {
            continue;
        };
        let Some(e) = u.escribe.as_ref() else {
            continue;
        };
        sitios.insert(
            i + 1,
            Sitio {
                pos: t.pos,
                inputs: u
                    .lee
                    .iter()
                    .filter_map(|n| Some((n.referencia(), n.pos?)))
                    .collect(),
                output: e.destino.pos.or(t.pos),
            },
        );
    }
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
        sitios,
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
    Sql(Result<Guion, NoSeAnaliza>),
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
    motivo: &NoSeAnaliza,
    out: &mut Vec<Diagnostic>,
    dichos: &mut BTreeSet<PathBuf>,
) {
    if dichos.insert(fichero.to_path_buf()) {
        let mut d = Diagnostic::new(
            Code::Oos2043,
            fichero,
            format!(
                "this file does not parse, so its transforms are not derived: {}",
                motivo.mensaje
            ),
        )
        .help(
            "a `.sql` gives one transform per statement that writes: `create or replace \
             dataset … as select`, `insert into … select`, `insert or replace into … select`. \
             Its documents stay as they were until it parses",
        );
        if let Some(p) = motivo.pos {
            d = d.at(p);
        }
        out.push(d);
    }
}

/// Si un `.py` **dice que tiene transforms** aunque no se lea entero: un
/// `@transform` que se derivó, uno que no está en el nivel superior, o una
/// línea que empieza por `@` y nombra `transform` (un decorador que el
/// analizador perdió al recuperarse de un error de sintaxis). La línea del
/// primero, si se sabe.
pub fn declara_transforms(fuente: &str, d: &Derivacion) -> Option<Option<usize>> {
    if let Some(x) = d.transforms.first() {
        let (l, _) = Lineas::new(fuente).de(x.sitios.decorador);
        return Some(Some(l as usize));
    }
    if let Some(a) = d.avisos.iter().find(|a| a.mensaje.contains("`@transform`")) {
        let (l, _) = Lineas::new(fuente).de(a.rango);
        return Some(Some(l as usize));
    }
    fuente
        .lines()
        .position(|l| {
            let t = l.trim_start();
            t.starts_with('@') && t.contains("transform")
        })
        .map(|i| Some(i + 1))
}

/// `OOS2043` por un `.py` con transforms que no se derivan **enteros**: no es
/// Python del puesto (un error de sintaxis, con su línea), o un `@transform`
/// decora un `def` que no está en el nivel superior. Una vez por fichero.
/// `true` si lo es: entonces sus documentos se quedan como estaban.
pub(crate) fn py_roto(
    fichero: &Path,
    fuente: &str,
    d: &Derivacion,
    out: &mut Vec<Diagnostic>,
    dichos: &mut BTreeSet<PathBuf>,
) -> bool {
    let lineas = Lineas::new(fuente);
    let diag = if let Some(f) = d.sintaxis.first().or(d.version.first()) {
        let (mayor, menor) = ore_code::python::PYTHON_DEL_PUESTO;
        let otros = d.sintaxis.len() + d.version.len() - 1;
        let mut x = Diagnostic::new(
            Code::Oos2043,
            fichero,
            format!(
                "this file is not Python the session ({mayor}.{menor}) runs, so its transforms are \
                 not derived: {}{}",
                f.mensaje,
                if otros > 0 {
                    format!(" (and {otros} more)")
                } else {
                    String::new()
                }
            ),
        )
        .at(crate::promover::pos(&lineas, f.rango))
        .help(
            "fix the syntax error: until the file parses, its `Transform` documents stay as they \
             were and nothing new is derived",
        );
        if let Some(a) = &f.ayuda {
            x = x.help(a.clone());
        }
        x
    } else if let Some(a) = d.avisos.iter().find(|a| a.mensaje.contains("`@transform`")) {
        Diagnostic::new(Code::Oos2043, fichero, a.mensaje.clone())
            .at(crate::promover::pos(&lineas, a.rango))
            .help(a.ayuda.clone().unwrap_or_default())
    } else {
        return false;
    };
    if dichos.insert(fichero.to_path_buf()) {
        out.push(diag);
    }
    true
}

/// `OOS2043`: cada razón por la que un `@transform` no se deriva, en su sitio.
fn no_se_deriva_t(
    fichero: &Path,
    fuente: &str,
    nombre: &str,
    fallos: &[ore_code::Fallo],
    out: &mut Vec<Diagnostic>,
) {
    let l = Lineas::new(fuente);
    for x in fallos {
        let mut d = Diagnostic::new(
            Code::Oos2043,
            fichero,
            format!("the transform `{nombre}` is not derived: {}", x.mensaje),
        )
        .at(crate::promover::pos(&l, x.rango));
        if let Some(a) = &x.ayuda {
            d = d.help(a.clone());
        }
        out.push(d);
    }
}

/// El sitio de un transform del código, si se lee.
fn sitio_de(l: &Leido, clave: &str) -> Option<Sitio> {
    match l {
        Leido::Python { fuente, d } => {
            let x = d.transforms.iter().rev().find(|x| x.nombre == clave)?;
            let lineas = Lineas::new(fuente);
            let p = |r| crate::promover::pos(&lineas, r);
            Some(Sitio {
                pos: Some(p(x.sitios.decorador)),
                inputs: x
                    .sitios
                    .inputs
                    .iter()
                    .map(|(n, r)| (n.clone(), p(*r)))
                    .collect(),
                output: x.sitios.output.map(p),
            })
        }
        Leido::Sql(Ok(g)) => g.sitios.get(&clave.parse().ok()?).cloned(),
        Leido::Sql(Err(_)) => None,
    }
}

/// Un diagnóstico de lo que resuelve, del documento derivado **al código**:
/// el fichero del código y, por el nodo que señalaba, el argumento —la
/// entrada, la salida— o el decorador (la sentencia, en SQL).
fn al_codigo(d: &mut Diagnostic, t: &Loaded, fichero: &Path, sitio: &Sitio, raiz: &Path) {
    let corto = |x: &str| crate::normalize::a_corto(x).into_owned();
    let salida = t.section("output").map(Node::pos);
    let pos = if d.pos.is_some() && d.pos == salida {
        sitio.output.or(sitio.pos)
    } else if let Some((e, _)) = entradas(t)
        .into_iter()
        .find(|(_, n)| Some(n.pos()) == d.pos)
    {
        sitio
            .inputs
            .iter()
            .find(|(k, _)| corto(k) == corto(&e))
            .map(|(_, p)| *p)
            .or(sitio.pos)
    } else {
        sitio.pos
    };
    let doc = t
        .path
        .strip_prefix(raiz)
        .unwrap_or(&t.path)
        .to_string_lossy()
        .replace('\\', "/");
    d.file = fichero.to_path_buf();
    d.pos = pos;
    d.help = Some(match d.help.take() {
        Some(h) => format!("{h} (derived document: `{doc}`)"),
        None => format!("derived document: `{doc}`"),
    });
}

/// Cómo se nombra un transform en un mensaje: su `entrypoint` (lo que la
/// persona escribió), no el nombre del documento derivado.
fn quien(t: &Loaded) -> String {
    t.section("entrypoint")
        .and_then(Node::as_str)
        .map(str::to_string)
        .or_else(|| t.qname())
        .unwrap_or_default()
}

pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let mut leidos: BTreeMap<PathBuf, Option<Leido>> = BTreeMap::new();
    let mut nombrados: BTreeSet<(PathBuf, String)> = BTreeSet::new();
    let mut rotos: BTreeSet<PathBuf> = BTreeSet::new();
    let mut coherentes: Vec<(&Loaded, PathBuf, String)> = Vec::new();

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
                    format!("`entrypoint: {texto}`: `{ruta}` is not in the package"),
                )
                .at(nodo.pos())
                .help(
                    "the document is about code that is not there. Write the file, or fix the \
                     path: it is relative to the package folder (the one with `package.yaml`)",
                ),
            );
            continue;
        };
        nombrados.insert((fichero.clone(), clave.clone()));

        let produccion = match l {
            Leido::Python { fuente, d } => {
                // ── OOS2043 · es Python del puesto ───────────────────────
                // Antes que el `def`: un error de sintaxis puede llevarse el
                // `def` o su decorador, y lo que hay que decir es el error.
                if py_roto(&fichero, fuente, d, out, &mut rotos) {
                    continue;
                }
                // ── OOS2042 · el `def` está ──────────────────────────────
                let Some(def) = d.defs.iter().rev().find(|x| x.nombre == clave) else {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2042,
                            &t.path,
                            format!("`{ruta}` has no top-level `def {clave}(…)`"),
                        )
                        .at(nodo.pos())
                        .help(
                            "the `entrypoint` names a module-level `def`: not a method of a \
                             class, nor a function inside another one",
                        ),
                    );
                    continue;
                };
                // ── OOS2013 · el `def` es un transform ───────────────────
                if !def.transformada {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2013,
                            &t.path,
                            format!(
                                "`{ruta}:{clave}` has no `@transform`, and this document says it \
                                 is a transform"
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "a transform is marked in the code —`from ore import transform` and \
                             `@transform(inputs=[…], output=…)` on the `def`— and the document \
                             comes from there",
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
                        no_se_deriva_t(&fichero, fuente, &clave, fallos, out);
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
                                "`entrypoint: {texto}`: `{ruta}` has {} statement{}",
                                g.sentencias,
                                if g.sentencias == 1 { "" } else { "s" }
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "`<path>.sql:<n>` names the `n`-th statement of the file, from 1, \
                             counting those that do not write too",
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
                                "statement {n} of `{ruta}` does not write data, and this document \
                                 says it is a transform"
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "a SQL transform is a statement that writes: `create or replace \
                             dataset … as select`, `insert into … select` or `insert or replace \
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
            coherentes.push((t, fichero.clone(), clave.clone()));
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
                    // Un fichero que dice tener transforms y no se lee entero es
                    // OOS2043 aunque ningún documento lo nombre: si no, el
                    // decorador que el analizador perdió no diría nada.
                    if declara_transforms(fuente, d).is_some()
                        && py_roto(&f, fuente, d, out, &mut rotos)
                    {
                        continue;
                    }
                    let sin_documento: Vec<_> = d
                        .transforms
                        .iter()
                        .filter(|x| !nombrados.contains(&(f.clone(), x.nombre.clone())))
                        .collect();
                    if sin_documento.is_empty() {
                        continue;
                    }
                    let lineas = Lineas::new(fuente);
                    for x in sin_documento {
                        match &x.resultado {
                            Err(fallos) => no_se_deriva_t(&f, fuente, &x.nombre, fallos, out),
                            Ok(p) => out.push(
                                Diagnostic::new(
                                    Code::Oos2013,
                                    &f,
                                    format!(
                                        "the `@transform` `{}` has no document: no `Transform` \
                                         of the package has `entrypoint: {}`",
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
                        let mut d = Diagnostic::new(
                            Code::Oos2013,
                            &f,
                            format!(
                                "statement {n} writes `{}` and has no document: no `Transform` \
                                 of the package has `entrypoint: {}`",
                                p.output, p.entrypoint
                            ),
                        )
                        .help(DERIVAR);
                        if let Some(pos) = g.sitios.get(n).and_then(|x| x.pos) {
                            d = d.at(pos);
                        }
                        out.push(d);
                    }
                }
            }
        }
    }

    // ── lo que resuelve, dicho en el código ──────────────────────────────
    let antes = out.len();
    let docs: Vec<&Loaded> = coherentes.iter().map(|(t, _, _)| *t).collect();
    resolver(pkg, &docs, out);
    for d in &mut out[antes..] {
        let Some((t, fichero, clave)) = coherentes.iter().find(|(t, _, _)| t.path == d.file) else {
            continue;
        };
        let Some(sitio) = leidos
            .get(fichero)
            .and_then(Option::as_ref)
            .and_then(|l| sitio_de(l, clave))
        else {
            continue;
        };
        al_codigo(d, t, fichero, &sitio, &pkg.root);
    }
}

// ── OOS2013 · el documento es el que el código da ───────────────────────────

/// Cada diferencia entre el documento y lo que el código da, en su sitio.
/// `true` si no hay ninguna.
fn coherencia(t: &Loaded, p: &Produccion, out: &mut Vec<Diagnostic>) -> bool {
    let antes = out.len();
    let qn = quien(t);
    let raiz = t.root.pos();
    let mut dif = |nodo: Option<&Node>, que: String| {
        out.push(
            Diagnostic::new(
                Code::Oos2013,
                &t.path,
                format!("the document of `{qn}` is not the one its code gives: {que}"),
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
                "it is named `{nombre}`, and its output gives `{}` (`.` as `__`)",
                p.nombre()
            ),
        );
    }
    let descripcion = t.meta("description").and_then(Node::as_str);
    if descripcion != p.descripcion.as_deref() {
        dif(
            t.meta("description"),
            match (descripcion, p.descripcion.as_deref()) {
                (Some(d), Some(c)) => format!("`description` says `{d}` and the code, `{c}`"),
                (Some(d), None) => format!("`description: {d}` is not in the code"),
                _ => format!(
                    "`description: {}` is missing: the first line of the docstring",
                    p.descripcion.as_deref().unwrap_or_default()
                ),
            },
        );
    }
    let runtime = t.section("runtime").and_then(Node::as_str).unwrap_or("");
    if runtime != p.runtime {
        dif(
            t.section("runtime"),
            format!("`runtime: {runtime}`, and the code is `{}`", p.runtime),
        );
    }
    let output = t.section("output").and_then(Node::as_str).unwrap_or("");
    if corto(output) != corto(&p.output) {
        dif(
            t.section("output"),
            format!("`output: {output}`, and the code writes `{}`", p.output),
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
        dif(
            nodo,
            format!("`inputs` has `{x}`, and the code does not read it"),
        );
    }
    for x in cod.difference(&doc) {
        dif(nodo, format!("`inputs` lacks `{x}`, which the code reads"));
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

/// 0055 B1 · **Una salida, aunque esté por nacer, nombra una base y un schema
/// que existen**: la primera escritura registra el dataset, pero no inventa su
/// base ni su schema —un build que escribe `ventsa.resumen` no puede crear la
/// base `ventsa`—. La base es un `Package` del árbol (`OOS2018`, la referencia
/// a lo que no hay); el schema, `default` o uno que esa base declara con un
/// `kind: Schema` (`OOS2037`). Y no es una base foránea, que no tiene datos
/// propios (`OOS2049`, ORE 0057 B4·6). `true` si dijo algo.
fn base_y_schema_de_la_salida(
    pkg: &Package,
    t: &Loaded,
    s: &str,
    out: &mut Vec<Diagnostic>,
) -> bool {
    let partes: Vec<&str> = s.split('.').collect();
    let (base, schema) = match partes.as_slice() {
        [b, _] => (*b, crate::normalize::SCHEMA_POR_DEFECTO),
        [b, sc, _] => (*b, *sc),
        _ => return false, // la forma la dice el despacho
    };
    let qn = quien(t);
    let nodo = t.section("output").map(Node::pos).unwrap_or(t.root.pos());
    let Some(p) = pkg
        .of(Kind::Package)
        .find(|p| p.meta("name").and_then(Node::as_str) == Some(base))
    else {
        out.push(
            Diagnostic::new(
                Code::Oos2018,
                &t.path,
                format!("`{qn}` writes `{s}`, and there is no database `{base}` in the tree"),
            )
            .at(nodo)
            .help(
                "the output of a transform may not exist yet, but its database must: create it \
                 (`create database`) or fix the name in the code",
            ),
        );
        return true;
    };
    // 0057 B4·6 · Una base foránea no tiene datos propios (v1alpha27 `01` §4):
    // lo que un transform escribe va a una base estándar, y se dice al
    // commitear, no al construir.
    if crate::foranea::es_foranea(p) {
        out.push(
            Diagnostic::new(
                Code::Oos2049,
                &t.path,
                format!(
                    "`{qn}` writes `{s}`, and `{base}` is a foreign database: it exposes its source and is not written to"
                ),
            )
            .at(nodo)
            .help(
                "write the output to a standard database (`create database`), and read the foreign database as an input",
            ),
        );
        return true;
    }
    if schema == crate::normalize::SCHEMA_POR_DEFECTO {
        return false;
    }
    let declarado = pkg.of(Kind::Schema).any(|x| {
        x.meta("name").and_then(Node::as_str) == Some(schema)
            && x.meta("namespace").and_then(Node::as_str) == Some(base)
    });
    if declarado {
        return false;
    }
    out.push(
        Diagnostic::new(
            Code::Oos2037,
            &t.path,
            format!("`{qn}` writes `{s}`, and the database `{base}` does not declare the schema `{schema}`"),
        )
        .at(nodo)
        .help(
            "the output of a transform may not exist yet, but its schema must: declare it \
             (`create schema`) or write to `default`",
        ),
    );
    true
}

fn resolver(pkg: &Package, ts: &[&Loaded], out: &mut Vec<Diagnostic>) {
    // Lo que producen los transforms del árbol, aunque esté por nacer: una
    // entrada puede ser la salida de otro (§6), y así se encadena un pipeline
    // antes de su primer build.
    let producido: BTreeSet<String> = pkg.of(Kind::Transform).filter_map(salida).collect();

    for t in ts {
        let qn = quien(t);
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
                        format!("`{qn}` reads `{e}`, which is nothing in the tree"),
                    )
                    .at(nodo.pos())
                    .help(
                        "an input is a `Table`, a `View`, a `Dataset` or a `MediaCollection`, or \
                         the output of another transform. Fix the name in the code",
                    ),
                );
            }
        }
        // ── OOS2018 · OOS2037 · la salida vive en una base y un schema ───
        let Some(s) = salida(t) else { continue };
        if base_y_schema_de_la_salida(pkg, t, &s, out) {
            continue;
        }
        // ── OOS2046 · la salida es algo que el código escribe ────────────
        let que = if let Some(d) = pkg.dataset(&s) {
            crate::vistas::es_mantenido(d)
                .then_some("a maintained `Dataset`: the system fills it from its `from`, not code")
        } else if let Some(c) = pkg.collection(&s) {
            c.section("from").is_some().then_some(
                "a `MediaCollection` with `from`: its files come from its origin, not code",
            )
        } else if pkg.table(&s).is_some() {
            Some("a `Table`: a pointer to an object of an origin, with no bytes of its own")
        } else if pkg.view(&s).is_some() {
            Some("a `View`: a question, with no bytes")
        } else if pkg.object_table(&s).is_some() {
            Some("an `ObjectTable`: the listing of an origin")
        } else {
            None // por nacer: la primera escritura la registra
        };
        if let Some(que) = que {
            out.push(
                Diagnostic::new(
                    Code::Oos2046,
                    &t.path,
                    format!("`{qn}` writes `{s}`, which is {que}"),
                )
                .at(t.section("output").map(Node::pos).unwrap_or(t.root.pos()))
                .help(
                    "the output of a transform is a written `Dataset` (with `columns`, without \
                     `from`) or a written `MediaCollection`, or a name that does not exist yet",
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
                        "`{s}` has {} producers: {}",
                        productores.len(),
                        donde.join(" · ")
                    ),
                )
                .at(t.section("output").map(Node::pos).unwrap_or(t.root.pos()))
                .help(
                    "one output, one producer: a transform is named by what it writes. Let only \
                     one write it, or each write its own",
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
        let (Some(s), qn) = (salida(t), quien(t)) else {
            continue;
        };
        let es = entradas(t);
        if let Some((_, nodo)) = es.iter().find(|(e, _)| *e == s) {
            out.push(
                Diagnostic::new(
                    Code::Oos2019,
                    &t.path,
                    format!("`{qn}` reads `{s}`, which is what it writes"),
                )
                .at(nodo.pos())
                .help(
                    "the output is not an input: reading what it wrote itself —an incremental— \
                     is not declared as an input",
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
                    format!("`{qn}` reads `{e}`, which comes from `{s}`, which is what it writes"),
                )
                .at(nodo.pos())
                .help(
                    "the graph of transforms —and maintained datasets— loops back on itself: no \
                     build order satisfies it. Break the cycle",
                ),
            );
        }
    }
}

// ── el editor (0055 P1) ─────────────────────────────────────────────────────

/// 0055 P1 · **Un transform del texto del editor**, sin guardar: lo que el
/// desplegable de Preview enseña y lo que su techo acota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelEditor {
    /// Python: el nombre del `def`. SQL: el ordinal de la sentencia.
    pub clave: String,
    /// La línea del decorador (o de la sentencia), desde 1.
    pub linea: Option<usize>,
    /// En forma corta, en su orden y sin repetir.
    pub inputs: Vec<String>,
    /// En forma corta.
    pub output: String,
    pub descripcion: Option<String>,
}

/// 0055 P1 · **Los transforms de un texto sin guardar**: la derivación del
/// commit sobre el editor —sin leer el árbol, sin escribir, sin ejecutar—, y
/// sus `OOS2043` con **el mismo texto y la misma línea** que la puerta del
/// commit (`py_roto`, `no_se_deriva_t`, `sql_roto`). Un `@transform` que no
/// se deriva no está en la lista: está en los diagnósticos. `fichero` da la
/// extensión y el `file` de los diagnósticos.
pub fn del_editor(fichero: &Path, fuente: &str) -> (Vec<DelEditor>, Vec<Diagnostic>) {
    let corto = |x: &str| crate::normalize::a_corto(x).into_owned();
    let base = fichero
        .file_name()
        .map(|b| b.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut dichos = BTreeSet::new();
    let mut lista = Vec::new();
    if base.ends_with(".sql") {
        match derivar_sql(fuente, &base) {
            Err(motivo) => sql_roto(fichero, &motivo, &mut out, &mut dichos),
            Ok(g) => {
                for (n, p) in &g.transforms {
                    lista.push(DelEditor {
                        clave: n.to_string(),
                        linea: g.sitios.get(n).and_then(|s| s.pos).map(|p| p.line),
                        inputs: p.inputs.iter().map(|i| corto(i)).collect(),
                        output: corto(&p.output),
                        descripcion: None,
                    });
                }
            }
        }
        return (lista, out);
    }
    let d = ore_code::python::derivar(fuente, &base);
    if py_roto(fichero, fuente, &d, &mut out, &mut dichos) {
        return (lista, out);
    }
    let lineas = Lineas::new(fuente);
    for x in &d.transforms {
        match &x.resultado {
            Err(fallos) => no_se_deriva_t(fichero, fuente, &x.nombre, fallos, &mut out),
            Ok(p) => {
                // Dos `def` con el mismo nombre: vale el último, como en Python.
                lista.retain(|t: &DelEditor| t.clave != x.nombre);
                lista.push(DelEditor {
                    clave: x.nombre.clone(),
                    linea: Some(crate::promover::pos(&lineas, x.sitios.decorador).line),
                    inputs: p.inputs.iter().map(|i| corto(i)).collect(),
                    output: corto(&p.output),
                    descripcion: p.descripcion.clone(),
                });
            }
        }
    }
    (lista, out)
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

    /// 0055 B1: la salida por nacer nombra una base que hay (`OOS2018`) y un
    /// schema que esa base declara (`OOS2037`); `default` existe sin más.
    #[test]
    fn la_salida_por_nacer_exige_su_base_y_su_schema() {
        let pkg = Package {
            root: PathBuf::from("."),
            docs: vec![
                doc(
                    "packages/ventas/package.yaml",
                    Kind::Package,
                    "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
                     metadata: { name: ventas, version: 1.0.0, status: active, domain: v }\n\
                     spec: { owner: team:v }\n",
                ),
                doc(
                    "packages/ventas/curado/schema.yaml",
                    Kind::Schema,
                    "apiVersion: oos.dev/v1alpha13\nkind: Schema\n\
                     metadata: { name: curado, namespace: ventas }\n",
                ),
                doc(
                    "packages/vivo/package.yaml",
                    Kind::Package,
                    "apiVersion: oos.dev/v1alpha27\nkind: Package\n\
                     metadata: { name: vivo, version: 1.0.0, status: active, domain: v }\n\
                     spec:\n  owner: team:v\n  foreign: { datasource: erp, include: [ventas] }\n",
                ),
            ],
            cedar: Vec::new(),
            generated: Vec::new(),
            sobres: Vec::new(),
        };
        let t = |output: &str| {
            doc(
                "packages/ventas/etl/pipeline/x.yaml",
                Kind::Transform,
                &format!(
                    "apiVersion: oos.dev/v1alpha25\nkind: Transform\n\
                     metadata: {{ name: x, namespace: ventas }}\n\
                     spec:\n  runtime: python\n  entrypoint: etl/t.py:x\n  \
                     inputs: []\n  output: {output}\n"
                ),
            )
        };
        let codigos = |output: &str| {
            let d = t(output);
            let s = salida(&d).unwrap();
            let mut out = Vec::new();
            base_y_schema_de_la_salida(&pkg, &d, &s, &mut out);
            out.into_iter().map(|d| d.code).collect::<Vec<_>>()
        };
        assert!(codigos("ventas.resumen").is_empty());
        assert!(codigos("ventas.default.resumen").is_empty());
        assert!(codigos("ventas.curado.resumen").is_empty());
        assert_eq!(codigos("ventas.crudo.resumen"), vec![Code::Oos2037]);
        assert_eq!(codigos("ventsa.resumen"), vec![Code::Oos2018]);
        assert_eq!(codigos("ventsa.curado.resumen"), vec![Code::Oos2018]);
        // 0057 B4·6: en una foreign database no se escribe, tampoco desde un build.
        assert_eq!(codigos("vivo.ventas.resumen"), vec![Code::Oos2049]);
        assert_eq!(codigos("vivo.resumen"), vec![Code::Oos2049]);
    }

    #[test]
    fn un_sql_que_no_se_analiza_no_da_transforms() {
        assert!(derivar_sql("create table x.y as select 1", "a.sql").is_err());
        assert!(derivar_sql("select 'sin cerrar", "a.sql").is_err());
    }

    // ── 0055 · lo roto se dice y no se borra; lo que resuelve, en el código ──

    const BUENO: &str = "from ore import transform, over, write\n\
                         \n\
                         \n\
                         def ayuda():\n\
                         \x20   return 1\n\
                         \n\
                         \n\
                         @transform(inputs=[\"ventas.clientes\"], output=\"ventas.limpios\")\n\
                         def clientes_limpios():\n\
                         \x20   return write(\"ventas.limpios\", over(\"ventas.clientes\"))\n";

    /// El caso medido en vivo: el decorador, sangrado dentro del `def` de
    /// antes tras su `return`, y el `def` en el nivel superior (pyright:
    /// «Expected function or class declaration after decorator»).
    const SANGRADO: &str = "from ore import transform, over, write\n\
                            \n\
                            \n\
                            def ayuda():\n\
                            \x20   return 1\n\
                            \x20   @transform(inputs=[\"ventas.clientes\"], output=\"ventas.limpios\")\n\
                            def clientes_limpios():\n\
                            \x20   return write(\"ventas.limpios\", over(\"ventas.clientes\"))\n";

    /// Python válido, con el `@transform` sobre un `def` anidado.
    const ANIDADO: &str = "from ore import transform, over, write\n\
                           \n\
                           \n\
                           def ayuda():\n\
                           \x20   @transform(inputs=[\"ventas.clientes\"], output=\"ventas.limpios\")\n\
                           \x20   def clientes_limpios():\n\
                           \x20       return write(\"ventas.limpios\", over(\"ventas.clientes\"))\n\
                           \x20   return clientes_limpios\n";

    fn arbol(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("transformar-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("packages/ventas");
        std::fs::create_dir_all(p.join("etl")).unwrap();
        std::fs::create_dir_all(p.join("datasets")).unwrap();
        std::fs::create_dir_all(p.join("curado")).unwrap();
        std::fs::write(
            p.join("package.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
             metadata: { name: ventas, version: 1.0.0, status: active, domain: v }\n\
             spec: { owner: team:v }\n",
        )
        .unwrap();
        std::fs::write(
            p.join("datasets/clientes.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\n\
             metadata: { name: clientes, namespace: ventas }\n\
             spec:\n  owner: team:v\n  changes: { mode: append }\n  columns:\n    id: { type: Integer }\n",
        )
        .unwrap();
        d
    }

    fn escribir(d: &Path, rel: &str, texto: &str) {
        std::fs::write(d.join("packages/ventas").join(rel), texto).unwrap();
    }

    fn generar(d: &Path) -> crate::generar::Plan {
        let (pkg, _) = crate::validate::cargar_paquete(d);
        let plan = crate::generar::plan_de_transforms(&pkg, None, Some("user:ana"));
        crate::generar::aplicar(&plan).unwrap();
        plan
    }

    fn diagnosticos(d: &Path) -> Vec<Diagnostic> {
        let (pkg, _) = crate::validate::cargar_paquete(d);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        out
    }

    fn rel(d: &Path, x: &Diagnostic) -> String {
        x.file
            .strip_prefix(d)
            .unwrap_or(&x.file)
            .to_string_lossy()
            .replace('\\', "/")
    }

    #[test]
    fn un_fichero_que_no_se_analiza_es_oos2043_y_su_documento_se_queda() {
        let d = arbol("sangrado");
        escribir(&d, "etl/limpios.py", BUENO);
        generar(&d);
        let doc = d.join("packages/ventas/etl/pipeline/ventas.limpios.yaml");
        assert!(doc.is_file());
        assert!(diagnosticos(&d).is_empty(), "{:?}", diagnosticos(&d));

        escribir(&d, "etl/limpios.py", SANGRADO);
        let plan = generar(&d);
        // (b) el documento se queda: nada que borrar.
        assert!(
            plan.cambios
                .iter()
                .all(|c| c.accion != crate::generar::Accion::Borrar),
            "{:?}",
            plan.cambios
        );
        assert!(doc.is_file());
        // (a) y la puerta lo dice, en el fichero y su línea, en inglés.
        let ds = diagnosticos(&d);
        let x = ds
            .iter()
            .find(|x| x.code == Code::Oos2043)
            .unwrap_or_else(|| panic!("{ds:?}"));
        assert_eq!(rel(&d, x), "packages/ventas/etl/limpios.py");
        assert!(x.pos.is_some_and(|p| (6..=7).contains(&p.line)), "{x:?}");
        assert!(
            x.message.starts_with("this file is not Python"),
            "{}",
            x.message
        );
        assert_eq!(
            ds.iter().filter(|x| x.code == Code::Oos2043).count(),
            1,
            "{ds:?}"
        );

        // Sin documento que lo nombre (un fichero nuevo), también.
        let _ = std::fs::remove_file(&doc);
        escribir(&d, "etl/otro.py", SANGRADO);
        let ds = diagnosticos(&d);
        assert!(
            ds.iter()
                .any(|x| x.code == Code::Oos2043 && rel(&d, x) == "packages/ventas/etl/otro.py"),
            "{ds:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn un_transform_anidado_es_oos2043_y_no_se_ignora() {
        let d = arbol("anidado");
        escribir(&d, "etl/limpios.py", BUENO);
        generar(&d);
        let doc = d.join("packages/ventas/etl/pipeline/ventas.limpios.yaml");
        escribir(&d, "etl/limpios.py", ANIDADO);
        let plan = generar(&d);
        assert!(doc.is_file(), "{:?}", plan.cambios);
        let ds = diagnosticos(&d);
        let x = ds
            .iter()
            .find(|x| x.code == Code::Oos2043)
            .unwrap_or_else(|| panic!("{ds:?}"));
        assert_eq!(rel(&d, x), "packages/ventas/etl/limpios.py");
        assert!(x.message.contains("not a top-level def"), "{}", x.message);
        assert_eq!(x.pos.map(|p| p.line), Some(5), "{x:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Lo que resuelve se dice en el código: el argumento de la entrada o de
    /// la salida del `@transform`, o el nombre en la sentencia SQL.
    #[test]
    fn lo_que_no_resuelve_se_dice_en_el_codigo() {
        let d = arbol("al-codigo");
        escribir(
            &d,
            "etl/limpios.py",
            &BUENO
                .replace(
                    "inputs=[\"ventas.clientes\"]",
                    "inputs=[\n    \"ventas.clientes\",\n    \"ventas.nadie\",\n]",
                )
                .replace(
                    "output=\"ventas.limpios\"",
                    "output=\"ventas.crudo.limpios\"",
                )
                .replace("write(\"ventas.limpios\"", "write(\"ventas.crudo.limpios\""),
        );
        escribir(
            &d,
            "etl/carga.sql",
            "select 1;\n\ncreate or replace dataset ventas.resumen as\nselect *\nfrom ventas.ninguno;\n",
        );
        generar(&d);
        let ds = diagnosticos(&d);
        let de = |code: Code, fichero: &str| {
            ds.iter()
                .find(|x| x.code == code && rel(&d, x) == fichero)
                .unwrap_or_else(|| panic!("{code:?} {fichero}: {ds:?}"))
        };
        let x = de(Code::Oos2018, "packages/ventas/etl/limpios.py");
        assert_eq!(x.pos.map(|p| p.line), Some(10), "{x:?}");
        assert_eq!(
            x.message,
            "`etl/limpios.py:clientes_limpios` reads `ventas.nadie`, which is nothing in the tree"
        );
        assert!(
            x.help
                .as_deref()
                .unwrap_or("")
                .contains("pipeline/ventas.crudo.limpios.yaml"),
            "{x:?}"
        );
        let x = de(Code::Oos2037, "packages/ventas/etl/limpios.py");
        assert_eq!(x.pos.map(|p| p.line), Some(11), "{x:?}");
        let x = de(Code::Oos2018, "packages/ventas/etl/carga.sql");
        assert_eq!(x.pos.map(|p| p.line), Some(5), "{x:?}");
        assert!(x.message.contains("`ventas.ninguno`"), "{}", x.message);
        // Ninguno en el YAML derivado.
        assert!(ds.iter().all(|x| !rel(&d, x).ends_with(".yaml")), "{ds:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0055 P1 · El editor da los transforms del texto sin guardar y, si no se
    /// lee, el mismo `OOS2043` —texto y línea— que la puerta del commit.
    #[test]
    fn el_editor_lista_sus_transforms_y_dice_lo_mismo_que_la_puerta() {
        let dos = format!(
            "{BUENO}\n\n@transform(inputs=[\"ventas.default.clientes\", \"ventas.pedidos\"], \
             output=\"ventas.default.otros\")\ndef otros():\n    \"\"\"Los otros.\"\"\"\n    return 1\n"
        );
        let (ts, ds) = del_editor(Path::new("packages/ventas/etl/limpios.py"), &dos);
        assert!(ds.is_empty(), "{ds:?}");
        let resumen: Vec<(&str, Option<usize>, Vec<&str>, &str)> = ts
            .iter()
            .map(|t| {
                (
                    t.clave.as_str(),
                    t.linea,
                    t.inputs.iter().map(String::as_str).collect(),
                    t.output.as_str(),
                )
            })
            .collect();
        assert_eq!(
            resumen,
            [
                (
                    "clientes_limpios",
                    Some(8),
                    vec!["ventas.clientes"],
                    "ventas.limpios"
                ),
                (
                    "otros",
                    Some(13),
                    vec!["ventas.clientes", "ventas.pedidos"],
                    "ventas.otros"
                ),
            ]
        );
        assert_eq!(ts[1].descripcion.as_deref(), Some("Los otros."));

        // Lo que no se lee: el diagnóstico de la puerta, igual.
        let d = arbol("editor");
        for (fuente, ruta) in [(SANGRADO, "etl/limpios.py"), (ANIDADO, "etl/anidado.py")] {
            escribir(&d, ruta, fuente);
            let fichero = d.join("packages/ventas").join(ruta);
            let puerta: Vec<(String, Option<usize>)> = diagnosticos(&d)
                .into_iter()
                .filter(|x| x.code == Code::Oos2043 && x.file == fichero)
                .map(|x| (x.message, x.pos.map(|p| p.line)))
                .collect();
            let (ts, ds) = del_editor(&fichero, fuente);
            assert!(ts.is_empty(), "{ts:?}");
            assert!(ds.iter().all(|x| x.code == Code::Oos2043), "{ds:?}");
            let editor: Vec<(String, Option<usize>)> = ds
                .into_iter()
                .map(|x| (x.message, x.pos.map(|p| p.line)))
                .collect();
            assert!(!puerta.is_empty(), "{ruta}");
            assert_eq!(editor, puerta, "{ruta}");
        }
        let _ = std::fs::remove_dir_all(&d);

        // SQL: una sentencia que escribe, por su ordinal.
        let (ts, ds) = del_editor(
            Path::new("c.sql"),
            "select 1;\ncreate or replace dataset ventas.resumen as select * from ventas.clientes;\n",
        );
        assert!(ds.is_empty(), "{ds:?}");
        assert_eq!(ts.len(), 1);
        assert_eq!(
            (ts[0].clave.as_str(), ts[0].output.as_str()),
            ("2", "ventas.resumen")
        );
        let (ts, ds) = del_editor(Path::new("c.sql"), "create or replace dataset (;\n");
        assert!(ts.is_empty());
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].code, Code::Oos2043);
    }
}
