//! v1alpha18 — **promover**: el código de un repositorio es una función
//! (ORE 0050, sobre 0031 W3.8). Desde G1, **el código es la fuente**: el
//! cliente escribe un `def` con `@function` y sus anotaciones, y el documento
//! `Function` se DERIVA de él (OOS v1alpha18 01 §4) con `ore-code`, que lee el
//! `.py` sin ejecutarlo. Aquí se comprueba, para cada función y en este orden:
//!
//! 1. la forma del documento: `runtime: python` es de v1alpha18, lleva
//!    `entrypoint` `<ruta>.py:<def>` dentro del paquete, y `models` es solo
//!    suyo (`OOS1004`); cada modelo resuelve (`OOS2005`);
//! 2. que el fichero esté y defina ese `def` en su nivel superior, sin `async`
//!    (`OOS2042`);
//! 3. que el `def` se pueda derivar —Python que el puesto entiende, anotado,
//!    con tipos de OOS— (`OOS2043`, señalando el `.py` con línea y columna);
//! 4. que el `def` lleve `@function`, que el documento sea **el que el código
//!    da** campo a campo, y que cada `@function` del paquete tenga el suyo
//!    (`OOS2013`: como el esquema Cedar, un artefacto generado que se quedó
//!    atrás).
//!
//! La precedencia es esa: `OOS2042` antes que `OOS2043` antes que `OOS2013`.
//!
//! # Lo que no se compara
//!
//! Los bytes. El documento se compara después de leerlo, campo a campo:
//! reordenar, reindentar o cambiar las comillas no es un fallo, y un cambio de
//! contrato siempre lo es.

use crate::code::Code;
use crate::diag::{Diagnostic, Pos};
use crate::document::{ApiVersion, Kind};
use crate::link::{Loaded, Package};
use crate::parse::Node;
use ore_code::lineas::Lineas;
use ore_code::{Campo, Derivacion, Fallo, Firma, Rango, Salida, python};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Cómo se arregla un documento que no es el del código (G1d).
pub(crate) const GENERAR: &str =
    "el documento se deriva del código: regenéralo con `ore functions generate`, o cambia el `def`";

/// Lo que un documento puede llevar y el código no da (§4.3). Una función con
/// cualquiera de ellos se escribe como en v1alpha10, sin `@function`.
const NO_SALE_DEL_CODIGO: &[&str] = &[
    "authorization",
    "endorsements",
    "effects",
    "preconditions",
    "idempotency",
    "model",
    "prompt",
    "source",
];

/// Un `.py` leído y derivado una sola vez, aunque lo nombren varios documentos.
struct Leido {
    fuente: String,
    d: Derivacion,
}

pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let mut leidos: BTreeMap<PathBuf, Option<Leido>> = BTreeMap::new();
    let mut nombradas: BTreeSet<(PathBuf, String)> = BTreeSet::new();
    let mut rotos: BTreeSet<PathBuf> = BTreeSet::new();
    for f in pkg.of(Kind::Function) {
        let version = f.version();
        let runtime = f.section("runtime").and_then(|n| n.as_str()).unwrap_or("");
        let de_18 = version.is_some_and(|v| v >= ApiVersion::V1Alpha18);

        // ── OOS1004 · el runtime de la versión ───────────────────────────
        if runtime == "python" && !de_18 {
            out.push(forma(
                f,
                f.section("runtime"),
                "`runtime: python` es de v1alpha18".to_string(),
                "una función de código se declara con `apiVersion: oos.dev/v1alpha18`; hasta \
                 v1alpha17 el runtime era `wasm` o `model`",
            ));
            continue;
        }
        if de_18 && !matches!(runtime, "wasm" | "model" | "python") {
            out.push(forma(
                f,
                f.section("runtime"),
                format!("`runtime: {runtime}` no es un runtime de OOS"),
                "`wasm` (un módulo sin red), `model` (un modelo del árbol) o `python` (un `def` \
                 del paquete)",
            ));
            continue;
        }

        // ── OOS1004 · `models`, solo de una función de código ────────────
        if let Some(m) = f.section("models")
            && runtime != "python"
        {
            out.push(forma(
                f,
                Some(m),
                format!("`models` con `runtime: {runtime}`"),
                "`models` dice qué modelos puede llamar el código de una función `python`. Un \
                 `wasm` no tiene red, y con `runtime: model` el modelo es lo que se ejecuta: \
                 `model`",
            ));
        }
        modelos_usados(pkg, f, out);

        if runtime != "python" {
            continue;
        }

        // ── OOS1004 · el `entrypoint` ────────────────────────────────────
        let Some(nodo) = f.section("entrypoint") else {
            out.push(forma(
                f,
                None,
                format!(
                    "`{}` es `runtime: python` y no dice `entrypoint`",
                    f.qname().unwrap_or_default()
                ),
                "el `entrypoint` es lo que el documento promueve: `<ruta>.py:<def>`, relativo a \
                 la carpeta del paquete",
            ));
            continue;
        };
        let texto = nodo.as_str().unwrap_or("");
        let Some((ruta, nombre)) = entrypoint(texto) else {
            out.push(forma(
                f,
                Some(nodo),
                format!("`entrypoint: {texto}` no es `<ruta>.py:<def>` dentro del paquete"),
                "la ruta es relativa a la carpeta del paquete, con `/`, sin `..` ni `/` inicial, \
                 y termina en `.py`; detrás de `:`, el nombre del `def`. Una función no nombra \
                 código de otro paquete",
            ));
            continue;
        };

        // ── OOS2042 · el fichero y el `def` están ────────────────────────
        let carpeta = carpeta_del_paquete(&f.path, &pkg.root);
        let fichero = carpeta.join(ruta);
        let Some(l) = leer(&mut leidos, &fichero, ruta) else {
            out.push(
                Diagnostic::new(
                    Code::Oos2042,
                    &f.path,
                    format!("`entrypoint: {texto}`: `{ruta}` no está en el paquete"),
                )
                .at(nodo.pos())
                .help(
                    "el documento promueve código que no hay. Escribe el fichero, o corrige la \
                     ruta: es relativa a la carpeta del paquete (la de su `package.yaml`)",
                ),
            );
            continue;
        };
        nombradas.insert((fichero.clone(), nombre.to_string()));
        // El último `def` con ese nombre es el que vale: Python liga el nombre
        // a la última definición.
        let Some(def) = l.d.defs.iter().rev().find(|d| d.nombre == nombre) else {
            out.push(
                Diagnostic::new(
                    Code::Oos2042,
                    &f.path,
                    format!("`{ruta}` no define `def {nombre}(…)` en su nivel superior"),
                )
                .at(nodo.pos())
                .help(
                    "el `entrypoint` nombra una función del módulo: no un método de una clase, \
                     ni una función dentro de otra",
                ),
            );
            continue;
        };
        if def.asincrona {
            out.push(
                Diagnostic::new(
                    Code::Oos2042,
                    &f.path,
                    format!("`{nombre}` en `{ruta}` es un `async def`"),
                )
                .at(nodo.pos())
                .help(
                    "el runtime llama al `def` y espera su valor; una corrutina es otra cosa. \
                     Escríbelo como `def`",
                ),
            );
            continue;
        }

        // ── OOS2043 · el fichero es Python del puesto ────────────────────
        if fichero_roto(&fichero, l, out, &mut rotos) {
            continue;
        }

        // ── OOS2013 · un documento promueve un `def` sin `@function` ─────
        if !def.decorada {
            out.push(
                Diagnostic::new(
                    Code::Oos2013,
                    &f.path,
                    format!("`{ruta}:{nombre}` no lleva `@function`, y este documento lo promueve"),
                )
                .at(nodo.pos())
                .help(
                    "una función de código se marca en el código, `from ore import function` y \
                     `@function` sobre el `def`, y el documento sale de ahí",
                ),
            );
            continue;
        }

        // ── OOS2043 · se deriva; OOS2013 · y es este documento ───────────
        let Some(func) = l.d.funciones.iter().rev().find(|x| x.nombre == nombre) else {
            continue;
        };
        match &func.resultado {
            Err(fallos) => no_se_deriva(&fichero, &l.fuente, nombre, fallos, out),
            Ok(firma) => coherencia(f, firma, out),
        }
    }

    // ── OOS2013 · cada `@function` del paquete tiene su documento ────────────
    //
    // Salvo en un paquete que no puede tener documentos gobernados: uno cuyo
    // nombre no puede ser `namespace` (`OOS2030`). Hoy es un vocabulario
    // importado (`oos.dev`), o el paquete de un proyecto de antes del
    // 2026-10-01 (`test-project`) hasta `ore migrate proyectos`: un proyecto
    // nuevo nace con un identificador y su paquete publica. Ahí un `@function`
    // no tiene dónde publicarse y es código de la sesión, sin más.
    let publicables: Vec<PathBuf> = paquetes_publicables(pkg)
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    for carpeta in publicables {
        let mut pys = Vec::new();
        ficheros_py(&carpeta, &mut pys);
        pys.sort();
        for py in pys {
            if carpeta_del_paquete(&py, &pkg.root) != carpeta {
                continue; // de un paquete de dentro: lo mira él
            }
            let Ok(rel) = py.strip_prefix(&carpeta) else {
                continue;
            };
            let ruta = rel.to_string_lossy().replace('\\', "/");
            if !leidos.contains_key(&py)
                && !std::fs::read_to_string(&py).is_ok_and(|t| python::puede_tener_funciones(&t))
            {
                continue;
            }
            let Some(l) = leer(&mut leidos, &py, &ruta) else {
                continue;
            };
            let sin_documento: Vec<_> =
                l.d.funciones
                    .iter()
                    .filter(|x| !nombradas.contains(&(py.clone(), x.nombre.clone())))
                    .collect();
            if sin_documento.is_empty() || fichero_roto(&py, l, out, &mut rotos) {
                continue;
            }
            let lineas = Lineas::new(&l.fuente);
            for x in sin_documento {
                match &x.resultado {
                    Err(fallos) => no_se_deriva(&py, &l.fuente, &x.nombre, fallos, out),
                    Ok(firma) => out.push(
                        Diagnostic::new(
                            Code::Oos2013,
                            &py,
                            format!(
                                "`@function` `{}` sin su documento: ningún `Function` del paquete \
                                 tiene `entrypoint: {}`",
                                x.nombre, firma.entrypoint
                            ),
                        )
                        .at(pos(&lineas, x.rango))
                        .help(GENERAR),
                    ),
                }
            }
        }
    }
}

fn forma(f: &Loaded, nodo: Option<&Node>, msg: String, ayuda: &str) -> Diagnostic {
    Diagnostic::new(Code::Oos1004, &f.path, msg)
        .at(nodo.map(Node::pos).unwrap_or_else(|| f.root.pos()))
        .help(ayuda.to_string())
}

// ── OOS2005 · cada modelo de `models` resuelve ──────────────────────────────

fn modelos_usados(pkg: &Package, f: &Loaded, out: &mut Vec<Diagnostic>) {
    for nodo in f.section("models").map(|n| n.items()).unwrap_or(&[]) {
        let Some(referencia) = nodo.as_str() else {
            continue;
        };
        if pkg.resolve_model(referencia, f).is_none() {
            out.push(
                crate::link::referencia_rota(&f.path, nodo, referencia, "models").help(format!(
                    "`{referencia}` no resuelve a ningún `kind: Model` (v1alpha15: una parte es \
                     el paquete y el schema de la función; dos, `<paquete>.<nombre>` en \
                     `default`; tres, completo). El código solo puede llamar a lo que el \
                     árbol tiene"
                )),
            );
        }
    }
}

// ── el `entrypoint` ─────────────────────────────────────────────────────────

/// `<ruta>.py:<def>` → `(ruta, def)`, o `None` si la forma no vale: ruta
/// relativa con `/`, sin `..` ni `/` inicial, que termina en `.py`; el `def`,
/// un identificador.
pub fn entrypoint(s: &str) -> Option<(&str, &str)> {
    let (ruta, nombre) = s.rsplit_once(':')?;
    let ident = |x: &str| {
        let mut c = x.chars();
        c.next()
            .is_some_and(|p| p.is_ascii_alphabetic() || p == '_')
            && c.all(|p| p.is_ascii_alphanumeric() || p == '_')
    };
    let ruta_ok = !ruta.is_empty()
        && !ruta.starts_with('/')
        && !ruta.contains('\\')
        && !ruta.contains(':')
        && ruta.ends_with(".py")
        && ruta.split('/').all(|t| !t.is_empty() && t != "..");
    (ruta_ok && ident(nombre)).then_some((ruta, nombre))
}

/// La carpeta del paquete donde vive el documento: la más cercana, subiendo,
/// que tiene `package.yaml`. Sin ninguna dentro del árbol, la raíz (un
/// paquete suelto).
pub(crate) fn carpeta_del_paquete(doc: &Path, raiz: &Path) -> PathBuf {
    let mut d = doc.parent();
    while let Some(c) = d {
        if c.join("package.yaml").is_file() {
            return c.to_path_buf();
        }
        if c == raiz {
            break;
        }
        d = c.parent();
    }
    raiz.to_path_buf()
}

// ── leer el código ──────────────────────────────────────────────────────────

/// Las carpetas de los paquetes que pueden publicar funciones, con su nombre:
/// los que tienen `package.yaml` y un nombre que puede ser `namespace`
/// (`OOS2030`).
pub(crate) fn paquetes_publicables(pkg: &Package) -> Vec<(PathBuf, String)> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|d| {
            let nombre = d.meta("name").and_then(Node::as_str)?;
            let carpeta = d.path.parent()?;
            (crate::pertenencia::puede_ser_namespace(nombre) && carpeta.is_dir())
                .then(|| (carpeta.to_path_buf(), nombre.to_string()))
        })
        .collect()
}

/// Lee y deriva un `.py` una sola vez. `None` si no se puede leer.
fn leer<'a>(
    leidos: &'a mut BTreeMap<PathBuf, Option<Leido>>,
    fichero: &Path,
    ruta: &str,
) -> Option<&'a Leido> {
    leidos
        .entry(fichero.to_path_buf())
        .or_insert_with(|| {
            let fuente = std::fs::read_to_string(fichero).ok()?;
            let d = python::derivar(&fuente, ruta);
            Some(Leido { fuente, d })
        })
        .as_ref()
}

/// Los `.py` de una carpeta, sin entrar en lo que no es del paquete: lo
/// oculto, los entornos y las cachés.
pub(crate) fn ficheros_py(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(es) = std::fs::read_dir(dir) else {
        return;
    };
    for e in es.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with('.')
            || matches!(
                n.as_str(),
                "__pycache__" | "node_modules" | "venv" | "target"
            )
        {
            continue;
        }
        if p.is_dir() {
            ficheros_py(&p, out);
        } else if n.ends_with(".py") {
            out.push(p);
        }
    }
}

fn pos(l: &Lineas, r: Rango) -> Pos {
    let (line, col) = l.de(r);
    Pos {
        line: line as usize,
        col: col as usize,
    }
}

/// `OOS2043` por un fichero que no es Python del puesto —roto, o con sintaxis
/// posterior a la suya—. `None` si lo es.
pub(crate) fn roto(fichero: &Path, fuente: &str, d: &Derivacion) -> Option<Diagnostic> {
    let f = d.sintaxis.first().or(d.version.first())?;
    let (mayor, menor) = python::PYTHON_DEL_PUESTO;
    let otros = d.sintaxis.len() + d.version.len() - 1;
    let mas = if otros > 0 {
        format!(" (y {otros} más)")
    } else {
        String::new()
    };
    let mut diag = Diagnostic::new(
        Code::Oos2043,
        fichero,
        format!(
            "no es Python que el puesto ({mayor}.{menor}) entienda: {}{mas}",
            f.mensaje
        ),
    )
    .at(pos(&Lineas::new(fuente), f.rango));
    if let Some(a) = &f.ayuda {
        diag = diag.help(a.clone());
    }
    Some(diag)
}

/// [`roto`], una sola vez por fichero. `true` si lo es.
fn fichero_roto(
    fichero: &Path,
    l: &Leido,
    out: &mut Vec<Diagnostic>,
    dichos: &mut BTreeSet<PathBuf>,
) -> bool {
    let Some(d) = roto(fichero, &l.fuente, &l.d) else {
        return false;
    };
    if dichos.insert(fichero.to_path_buf()) {
        out.push(d);
    }
    true
}

/// `OOS2043`: cada razón por la que un `@function` no se deriva, en su sitio
/// del `.py`.
pub(crate) fn no_se_deriva(
    fichero: &Path,
    fuente: &str,
    nombre: &str,
    fallos: &[Fallo],
    out: &mut Vec<Diagnostic>,
) {
    let l = Lineas::new(fuente);
    for x in fallos {
        let mut d = Diagnostic::new(
            Code::Oos2043,
            fichero,
            format!("`{nombre}` no se deriva: {}", x.mensaje),
        )
        .at(pos(&l, x.rango));
        if let Some(a) = &x.ayuda {
            d = d.help(a.clone());
        }
        out.push(d);
    }
}

// ── OOS2013 · el documento es el que el código da ───────────────────────────

/// `output` como un valor (§4.7): `{type: T}`, un mapa cuyo `type` no es un
/// objeto. `Some(T)` si lo es; un mapa de campos da `None`.
pub fn salida_valor(output: &Node) -> Option<&str> {
    output.get("type").and_then(|(_, t)| t.as_str())
}

/// Cada diferencia entre el documento y la firma del código, en el sitio del
/// documento donde está.
fn coherencia(f: &Loaded, firma: &Firma, out: &mut Vec<Diagnostic>) {
    let qn = f.qname().unwrap_or_default();
    let raiz = f.root.pos();
    let mut dif = |nodo: Option<&Node>, que: String| {
        out.push(
            Diagnostic::new(
                Code::Oos2013,
                &f.path,
                format!("`{qn}` no es el documento que su código da: {que}"),
            )
            .at(nodo.map(Node::pos).unwrap_or(raiz))
            .help(GENERAR),
        );
    };

    for k in NO_SALE_DEL_CODIGO {
        if let Some(n) = f.section(k) {
            dif(
                Some(n),
                format!(
                    "`{k}` no sale del código; una función con `{k}` se escribe sin `@function`"
                ),
            );
        }
    }
    let nombre = f.meta("name").and_then(Node::as_str).unwrap_or_default();
    if nombre != firma.nombre {
        dif(
            f.meta("name"),
            format!("se llama `{nombre}` y el `def`, `{}`", firma.nombre),
        );
    }
    escalar(
        f.meta("description"),
        firma.descripcion.as_deref(),
        "description",
        &mut dif,
    );
    escalar(f.section("over"), firma.over.as_deref(), "over", &mut dif);
    let limites = f.section("limits");
    escalar(
        limites.and_then(|l| l.get("timeout")).map(|(_, v)| v),
        firma.timeout.as_deref(),
        "limits.timeout",
        &mut dif,
    );
    for (k, v) in limites.map(Node::entries).unwrap_or(&[]) {
        if let Some(k) = k.as_str().filter(|k| *k != "timeout") {
            dif(Some(v), format!("`limits.{k}` no sale del código"));
        }
    }
    conjunto(
        f.section("reads"),
        firma.reads.as_deref(),
        "reads",
        &mut dif,
    );
    conjunto(
        f.section("models"),
        firma.models.as_deref(),
        "models",
        &mut dif,
    );
    campos(f.section("input"), &firma.entrada, "input", &mut dif);
    let output = f.section("output");
    match (&firma.salida, output.and_then(salida_valor)) {
        (Salida::Valor(t), Some(d)) if mismo_tipo(d, &t.to_string()) => {}
        (Salida::Valor(t), _) => dif(
            output,
            format!("el código devuelve un valor, `output: {{type: {t}}}`"),
        ),
        (Salida::Campos(_), Some(d)) => dif(
            output,
            format!("`output` es un valor (`{d}`) y el código devuelve una `@dataclass`"),
        ),
        (Salida::Campos(cs), None) => campos(output, cs, "output", &mut dif),
    }
}

fn mismo_tipo(a: &str, b: &str) -> bool {
    let sin = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    sin(a) == sin(b)
}

fn escalar(
    nodo: Option<&Node>,
    codigo: Option<&str>,
    campo: &str,
    dif: &mut impl FnMut(Option<&Node>, String),
) {
    let doc = nodo.and_then(Node::as_str);
    if doc != codigo {
        let que = match (doc, codigo) {
            (Some(d), Some(c)) => format!("`{campo}` dice `{d}` y el código, `{c}`"),
            (Some(d), None) => format!("`{campo}: {d}` no está en el código"),
            (None, Some(c)) => format!("falta `{campo}: {c}`, que el código dice"),
            (None, None) => return,
        };
        dif(nodo, que);
    }
}

/// `reads` y `models`: el orden no es contrato.
fn conjunto(
    nodo: Option<&Node>,
    codigo: Option<&[String]>,
    campo: &str,
    dif: &mut impl FnMut(Option<&Node>, String),
) {
    let doc: BTreeSet<&str> = nodo
        .map(Node::items)
        .unwrap_or(&[])
        .iter()
        .filter_map(Node::as_str)
        .collect();
    let cod: BTreeSet<&str> = codigo.unwrap_or(&[]).iter().map(String::as_str).collect();
    for x in doc.difference(&cod) {
        dif(nodo, format!("`{campo}` lleva `{x}` y el código no"));
    }
    for x in cod.difference(&doc) {
        dif(
            nodo,
            format!("al `{campo}` le falta `{x}`, que el código dice"),
        );
    }
}

/// `input` o los campos de `output`: nombre, tipo y `required` (ausente es
/// `false`), sin orden.
fn campos(
    nodo: Option<&Node>,
    codigo: &[Campo],
    lado: &str,
    dif: &mut impl FnMut(Option<&Node>, String),
) {
    let doc: BTreeMap<&str, &Node> = nodo
        .map(Node::entries)
        .unwrap_or(&[])
        .iter()
        .filter_map(|(k, v)| Some((k.as_str()?, v)))
        .collect();
    for c in codigo {
        let Some(v) = doc.get(c.nombre.as_str()) else {
            let req = if c.requerido { ", required: true" } else { "" };
            dif(
                nodo,
                format!(
                    "falta `{lado}.{}: {{type: {}{req}}}`, que el código dice",
                    c.nombre, c.tipo
                ),
            );
            continue;
        };
        let tipo = v
            .get("type")
            .and_then(|(_, t)| t.as_str())
            .unwrap_or_default();
        if !mismo_tipo(tipo, &c.tipo.to_string()) {
            dif(
                Some(v),
                format!(
                    "`{lado}.{}` es `{tipo}` y en el código, `{}`",
                    c.nombre, c.tipo
                ),
            );
        }
        let requerido = v
            .get("required")
            .and_then(|(_, r)| r.as_str())
            .is_some_and(|r| r == "true");
        if requerido != c.requerido {
            let (d, k) = if c.requerido {
                (
                    "opcional",
                    "obligatorio: sin valor por defecto y sin `Optional`",
                )
            } else {
                (
                    "obligatorio",
                    "opcional: con valor por defecto, o `Optional`",
                )
            };
            dif(
                Some(v),
                format!(
                    "`{lado}.{}` es {d} en el documento y en el código, {k}",
                    c.nombre
                ),
            );
        }
    }
    for (k, v) in &doc {
        if !codigo.iter().any(|c| c.nombre == *k) {
            dif(Some(*v), format!("`{lado}.{k}` no está en el código"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_entrypoint_es_una_ruta_del_paquete_y_un_def() {
        assert_eq!(
            entrypoint("funciones/r.py:riesgo"),
            Some(("funciones/r.py", "riesgo"))
        );
        assert_eq!(entrypoint("a/b/c.py:_f1"), Some(("a/b/c.py", "_f1")));
        for malo in [
            "/a.py:f",
            "../a.py:f",
            "a/../b.py:f",
            "a.py",
            "a.ts:f",
            "a.py:1f",
            "a.py:",
            "a//b.py:f",
        ] {
            assert_eq!(entrypoint(malo), None, "{malo}");
        }
    }
}
