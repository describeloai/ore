//! v1alpha27 — **la base foránea** (ORE 0057, OOS v1alpha27 `01`).
//!
//! Un `Package` con `spec.foreign: { datasource, include }` **expone**, con su
//! nombre, las `Table` y los `ObjectTable` de una fuente: `<base>.<schema>.<n>`
//! **es** el documento de la fuente —el mismo, con dos nombres—, no una copia
//! ni una vista. Aquí:
//!
//! - **la resolución** ([`expuesto`]): de un nombre expuesto al documento de la
//!   fuente. [`Package::table`] y [`Package::object_table`] la usan cuando el
//!   nombre no es de ningún documento, y con ellas todo lo que resuelve;
//! - **lo que la gramática pide** ([`check`]), antes del enlazado:
//!   - la forma de `spec.foreign` (`OOS1004`) y su fuente declarada (`OOS2004`);
//!   - que la base sólo contenga `Schema` y vistas sin copia (`OOS2049`);
//!   - que cada nombre expuesto sea único (`OOS2050`).
//!
//! Lo que la base no expone no resuelve, y un nombre suyo que no es nada es
//! `OOS2018`, como cualquier otro. Leer lo expuesto es leer la tabla en vivo
//! (v1alpha24), y con el interruptor apagado es `OOS2051` (`reparto`).

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package};
use crate::parse::Node;
use std::collections::BTreeMap;

/// Una base foránea del árbol: su `Package` y lo que declara.
pub struct Foranea<'a> {
    /// El nombre del paquete: el primer trozo de lo que expone.
    pub nombre: &'a str,
    pub paquete: &'a Loaded,
    /// `foreign.datasource`.
    pub datasource: &'a str,
    /// `foreign.include`: schemas (`ventas`) u objetos (`ventas.clientes`).
    pub include: Vec<&'a str>,
}

impl Foranea<'_> {
    /// Si `include` alcanza al objeto `schema.nombre`.
    fn incluye(&self, schema: &str, nombre: &str) -> bool {
        self.include.iter().any(|i| match i.split_once('.') {
            None => *i == schema,
            Some((s, n)) => s == schema && n == nombre,
        })
    }
}

/// Si el paquete es una base foránea: tiene `spec.foreign`.
pub fn es_foranea(p: &Loaded) -> bool {
    p.kind == Kind::Package && p.section("foreign").is_some()
}

/// Las bases foráneas del árbol.
pub fn foraneas(pkg: &Package) -> Vec<Foranea<'_>> {
    pkg.of(Kind::Package)
        .filter_map(|p| {
            let f = p.section("foreign")?;
            Some(Foranea {
                nombre: p.meta("name")?.as_str()?,
                paquete: p,
                datasource: f.get("datasource").and_then(|(_, v)| v.as_str())?,
                include: f
                    .get("include")
                    .map(|(_, v)| v.items().iter().filter_map(Node::as_str).collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// La base foránea de este nombre, si lo es.
pub fn foranea<'a>(pkg: &'a Package, nombre: &str) -> Option<Foranea<'a>> {
    foraneas(pkg).into_iter().find(|f| f.nombre == nombre)
}

/// El schema que un documento **declara** (`metadata.schema`): sin él no se
/// expone (v1alpha27 `01` §3.2). No es [`Loaded::schema`], que da `default`.
fn schema_declarado(d: &Loaded) -> Option<&str> {
    d.meta("schema").and_then(Node::as_str)
}

/// Los documentos de la fuente que esta base podría exponer por su schema y
/// nombre: `Table` u `ObjectTable` de su `datasource`, con schema declarado.
fn de_la_fuente<'a>(pkg: &'a Package, f: &Foranea<'_>) -> impl Iterator<Item = &'a Loaded> {
    let ds = f.datasource.to_string();
    pkg.docs.iter().filter(move |d| {
        matches!(d.kind, Kind::Table | Kind::ObjectTable)
            && d.section("datasource").and_then(Node::as_str) == Some(ds.as_str())
            && schema_declarado(d).is_some()
    })
}

/// Lo que una base expone, por su nombre expuesto (forma corta): lo que su
/// `include` alcanza, sin `exports` (v1alpha28 `01` §4: el árbol es un catálogo).
pub fn expuestos<'a>(pkg: &'a Package, f: &Foranea<'_>) -> Vec<(String, &'a Loaded)> {
    de_la_fuente(pkg, f)
        .filter_map(|d| {
            let s = schema_declarado(d)?;
            let n = d.meta("name").and_then(Node::as_str)?;
            f.incluye(s, n)
                .then(|| (crate::normalize::corto(f.nombre, s, n), d))
        })
        .collect()
}

/// **La resolución** (v1alpha27 `01` §3): el documento de la fuente que el
/// nombre expuesto `qname` (corto o completo) **es**, si alguna base foránea
/// lo expone. Con dos candidatos no resuelve: es `OOS2050`, y elegir uno
/// sería resolver a la primera que aparezca.
pub fn expuesto<'a>(pkg: &'a Package, qname: &str) -> Option<&'a Loaded> {
    let completo = crate::normalize::completo(qname);
    let mut partes = completo.split('.');
    let (Some(b), Some(s), Some(n), None) =
        (partes.next(), partes.next(), partes.next(), partes.next())
    else {
        return None;
    };
    let f = foranea(pkg, b)?;
    if !f.incluye(s, n) {
        return None;
    }
    let mut hay = de_la_fuente(pkg, &f).filter(|d| {
        schema_declarado(d) == Some(s) && d.meta("name").and_then(Node::as_str) == Some(n)
    });
    let uno = hay.next()?;
    hay.next().is_none().then_some(uno)
}

/// v1alpha27 `01` §2–§5, antes del enlazado.
pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let declarados = crate::vistas::datasources_declarados(pkg);

    for p in pkg.of(Kind::Package).filter(|p| es_foranea(p)) {
        let Some(nodo) = p.section("foreign") else {
            continue;
        };
        let nombre = p.meta("name").and_then(Node::as_str).unwrap_or_default();

        // ── OOS1004 · la forma ───────────────────────────────────────────────
        let forma = |m: String| {
            Diagnostic::new(Code::Oos1004, &p.path, m)
                .at(nodo.pos())
                .help(
                    "`foreign: { datasource: <fuente>, include: [<schema> | <schema>.<objeto>, …] }` \
                     (v1alpha27 `01` §2)",
                )
        };
        let ds = nodo.get("datasource").map(|(_, v)| v);
        if ds.and_then(Node::as_str).is_none_or(str::is_empty) {
            out.push(forma(format!(
                "la base foránea `{nombre}` no dice `foreign.datasource`"
            )));
            continue;
        }
        let include = nodo.get("include").map(|(_, v)| v);
        let entradas: Vec<&Node> = include
            .map(|v| v.items().iter().collect())
            .unwrap_or_default();
        if entradas.is_empty() {
            out.push(forma(format!(
                "la base foránea `{nombre}` no dice qué expone: `foreign.include` vacío o ausente"
            )));
            continue;
        }
        let ident = |s: &str| {
            s.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        let mut vistas_entradas = std::collections::BTreeSet::new();
        for e in &entradas {
            let t = e.as_str().unwrap_or("");
            let partes: Vec<&str> = t.split('.').collect();
            if partes.is_empty() || partes.len() > 2 || !partes.iter().all(|x| ident(x)) {
                out.push(
                    Diagnostic::new(
                        Code::Oos1004,
                        &p.path,
                        format!("`foreign.include` de `{nombre}`: `{t}` no es `<schema>` ni `<schema>.<objeto>`"),
                    )
                    .at(e.pos()),
                );
            } else if !vistas_entradas.insert(t) {
                out.push(
                    Diagnostic::new(
                        Code::Oos1004,
                        &p.path,
                        format!("`foreign.include` de `{nombre}` repite `{t}`"),
                    )
                    .at(e.pos()),
                );
            }
        }

        // ── OOS2004 · la fuente, declarada ───────────────────────────────────
        if let Some(ds) = ds
            && !declarados.contains(ds.as_str().unwrap_or(""))
        {
            out.push(crate::vistas::no_declarado(
                p,
                ds,
                "foreign.datasource",
                &declarados,
            ));
        }
    }
    if !out.is_empty() {
        return out;
    }

    for f in foraneas(pkg) {
        // ── OOS2049 · sólo `Schema` y vistas sin copia ───────────────────────
        for d in pkg.docs.iter().filter(|d| {
            d.kind != Kind::Package && d.meta("namespace").and_then(Node::as_str) == Some(f.nombre)
        }) {
            let cabe = d.kind == Kind::Schema
                || (d.kind == Kind::View && d.section("materialized").is_none());
            if !cabe {
                out.push(
                    Diagnostic::new(
                        Code::Oos2049,
                        &d.path,
                        format!(
                            "`{}` es una base foránea y no puede tener {}",
                            f.nombre,
                            if d.kind == Kind::View {
                                "una vista con copia (`materialized`)".to_string()
                            } else {
                                format!("un `{}`", d.kind.as_str())
                            }
                        ),
                    )
                    .at(d
                        .meta("name")
                        .map(Node::pos)
                        .unwrap_or_else(|| d.root.pos()))
                    .help(
                        "una base foránea sólo tiene `Schema` y vistas sin copia: lo del origen \
                         lo expone, no lo declara ni lo copia. Copiar es crear un `Dataset` en \
                         una base estándar (v1alpha27 `01` §4)",
                    ),
                );
            }
        }

        // ── OOS2050 · un nombre expuesto, una cosa ───────────────────────────
        let mut por_nombre: BTreeMap<String, Vec<&Loaded>> = BTreeMap::new();
        for (n, d) in expuestos(pkg, &f) {
            por_nombre.entry(n).or_default().push(d);
        }
        for (n, ds) in &por_nombre {
            if ds.len() > 1 {
                out.push(
                    Diagnostic::new(
                        Code::Oos2050,
                        &f.paquete.path,
                        format!(
                            "`{n}` lo exponen dos objetos de `{}`: {}",
                            f.datasource,
                            ds.iter()
                                .filter_map(|d| d.qname())
                                .collect::<Vec<_>>()
                                .join(" y ")
                        ),
                    )
                    .at(f.paquete.root.pos())
                    .help("un nombre expuesto es un documento: que la fuente tenga uno solo con ese schema y nombre"),
                );
            }
            if let Some(v) = pkg.view(n) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2050,
                        &v.path,
                        format!("la vista `{n}` tiene el nombre de lo que `{}` expone", f.nombre),
                    )
                    .at(v.meta("name").map(Node::pos).unwrap_or_else(|| v.root.pos()))
                    .help(format!(
                        "`{n}` ya es la tabla de la fuente: dale otro nombre a la vista, o ponla en otro schema"
                    )),
                );
            }
        }
    }
    out
}
