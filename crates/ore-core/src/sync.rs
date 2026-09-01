//! Artefactos generados desincronizados — `OOS2013`.
//!
//! # Dos artefactos derivados que sí se comprometen a Git
//!
//! El principio **P2** dice que lo derivable no se declara. El esquema Cedar y
//! `ontology.lock` no se declaran: **se generan**. Y aun así se comprometen al
//! repositorio, por la misma razón que un `package-lock.json` — para que el
//! tooling de Cedar y el resolutor de dependencias funcionen sin compilar.
//!
//! El precio de esa comodidad es que pueden quedar obsoletos, y este código es
//! lo que lo cobra.
//!
//! # Por qué fallar aquí importa más de lo que parece
//!
//! Un esquema Cedar generado antes de que el retículo tuviera `critical` hace
//! que
//!
//! ```text
//! resource in Label::"gdpr.sensitivity:critical"
//! ```
//!
//! **no case con nada**. La política no da error: simplemente deja de
//! aplicarse. El dato más sensible del paquete queda sin gobernar, en silencio,
//! y todos los tableros siguen en verde.
//!
//! Es el modo de fallo peor posible —silencioso y en la dirección insegura— y
//! es exactamente contra lo que existe la denegación por defecto.
//!
//! # Qué se compara, y qué no
//!
//! **No** se comparan bytes. `emit/cedar-schema-structure` establece que dos
//! implementaciones pueden formatear el esquema distinto y ser ambas correctas,
//! así que exigir el texto exacto convertiría una diferencia de formato en un
//! fallo de conformidad.
//!
//! Lo que se comprueba es **presencia**: cada etiqueta que el paquete declara
//! tiene que estar en el artefacto, y cada dependencia declarada tiene que estar
//! resuelta en el lock. Es la propiedad de la que depende la garantía, y la
//! única que sobrevive a que alguien reindente el fichero.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::flow;
use crate::link::{Loaded, Package};
use crate::normalize;
use std::collections::{BTreeMap, BTreeSet};

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    esquema_cedar(pkg, &mut out);
    lock(pkg, &mut out);
    coincide_con_el_lock(pkg, &mut out);
    el_rango_se_cumple(pkg, &mut out);
    out
}

/// Lo que el manifiesto **pide** y lo que el lock **resolvió** tienen que ser
/// compatibles.
///
/// `OOS2013` comprobaba que cada dependencia declarada estuviera en el lock, y
/// `coincide_con_el_lock` que el árbol fuera lo que el lock dice. Nadie comparaba
/// las dos puntas: con `^0.2` declarado y `0.1.0` resuelto, **el build salía
/// verde** — el manifiesto pedía una cosa, el lock resolvía otra, y la
/// clasificación que gobernaba era la vieja.
///
/// Es el mismo síntoma de siempre —dos documentos generados y declarados que
/// discrepan— así que es el mismo código.
fn el_rango_se_cumple(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let Some(l) = pkg.docs.iter().find(|d| normalize::es_lock(d)) else {
        return;
    };
    let resueltas = versiones(l);
    if resueltas.is_empty() {
        return;
    }
    for d in &pkg.docs {
        if !matches!(d.kind, Kind::OntologyConfig | Kind::Package) {
            continue;
        }
        for it in d.section("dependencies").map(|s| s.items()).unwrap_or(&[]) {
            let (Some((k, nombre)), Some(rango)) = (
                it.get("package"),
                it.get("version").and_then(|(_, v)| v.as_str()),
            ) else {
                continue;
            };
            let Some(nombre) = nombre.as_str() else {
                continue;
            };
            let Some(version) = resueltas.get(nombre) else {
                continue; // ausente del lock: es `OOS2013` por la otra puerta
            };
            if satisface(version, rango) {
                continue;
            }
            out.push(
                Diagnostic::new(
                    Code::Oos2013,
                    &d.path,
                    format!("`{nombre}` se pide en `{rango}` y el lock resuelve `{version}`"),
                )
                .at(k.pos())
                .help(
                    "el manifiesto pide una cosa y el lock resolvió otra, así que lo que \
                     gobierna no es lo que alguien declaró. `ore lock` lo vuelve a resolver, \
                     y si la versión que hace falta no está, lo dirá en vez de dejarlo pasar",
                ),
            );
        }
    }
}

/// `nombre → versión` del lock.
fn versiones(l: &crate::link::Loaded) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some((_, ps)) = l.root.get("packages") {
        for p in ps.items() {
            if let (Some(n), Some(v)) = (
                p.get("name").and_then(|(_, v)| v.as_str()),
                p.get("version").and_then(|(_, v)| v.as_str()),
            ) {
                out.insert(n.to_string(), v.to_string());
            }
        }
    }
    out
}

/// `^`, `~` y la versión exacta, con `0.x` estricto.
///
/// Un rango que no se sabe leer **se da por bueno aquí**, y es la decisión
/// contraria a la de `ore lock`: allí se está ESCRIBIENDO una resolución y
/// aceptar lo que no se comprueba la afirmaría; aquí se está leyendo una que ya
/// existe, y rechazarla por no saber leer el rango rompería un paquete válido
/// por una carencia nuestra.
fn satisface(version: &str, rango: &str) -> bool {
    fn partes(v: &str) -> Option<(u64, u64, u64)> {
        let limpio = v.split(['-', '+']).next()?;
        let mut it = limpio.split('.');
        Some((
            it.next()?.parse().ok()?,
            it.next().unwrap_or("0").parse().ok()?,
            it.next().unwrap_or("0").parse().ok()?,
        ))
    }
    let r = rango.trim();
    let Some(v) = partes(version) else {
        return true;
    };
    let (op, resto) = match r.strip_prefix('^') {
        Some(x) => ('^', x),
        None => match r.strip_prefix('~') {
            Some(x) => ('~', x),
            None => ('=', r),
        },
    };
    let Some(p) = partes(resto) else { return true };
    let techo = match op {
        '^' if p.0 == 0 => (0, p.1 + 1, 0),
        '^' => (p.0 + 1, 0, 0),
        '~' => (p.0, p.1 + 1, 0),
        _ => return v == p,
    };
    v >= p && v < techo
}

/// **`usar(P) ⟹ digest(P) ∈ lock`** — la regla de v1alpha6.
///
/// Un paquete que está en el árbol y que el lock nombra tiene que ser **el que
/// el lock nombra**. No es una comprobación de higiene: es lo único que hace que
/// no importe de dónde vino. Un registro que sirviera otra cosa, un `.oob`
/// editado a mano o un vendorizado que alguien tocó producen otro digest, y
/// aquí se paran.
///
/// Sin esto, «el registro no es de confianza» sería una frase: lo que la
/// convierte en una propiedad es que **alguien compare**.
fn coincide_con_el_lock(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let Some(l) = pkg.docs.iter().find(|d| normalize::es_lock(d)) else {
        return;
    };
    let esperados = digests(l);
    if esperados.is_empty() {
        return;
    }
    let miembros = crate::link::miembros(pkg);
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Package) {
        let (Some(nombre), Some(sitio)) =
            (d.meta("name").and_then(|n| n.as_str()), d.path.parent())
        else {
            continue;
        };
        let Some(esperado) = esperados.get(nombre) else {
            continue;
        };
        let suyos = crate::link::publicables(&solo(pkg, &miembros, sitio));
        let real = crate::digest::package(&suyos);
        if real == *esperado {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos2013,
                &d.path,
                format!("`{nombre}` no es lo que `ontology.lock` dice que es"),
            )
            .at(d.root.pos())
            .help(format!(
                "el lock lo fija en `{esperado}` y lo que hay digiere `{real}`. Un paquete \
                 se identifica por lo que CONTIENE y no por de dónde vino, que es lo que \
                 permite que su origen no tenga que ser de confianza — y lo que obliga a \
                 parar aquí. Si el cambio es deliberado, `ore lock` lo vuelve a fijar y el \
                 nuevo digest se revisa en un pull request"
            )),
        );
    }
}

/// `nombre → digest` del lock.
fn digests(l: &crate::link::Loaded) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some((_, ps)) = l.root.get("packages") {
        for p in ps.items() {
            if let (Some(n), Some(d)) = (
                p.get("name").and_then(|(_, v)| v.as_str()),
                p.get("digest").and_then(|(_, v)| v.as_str()),
            ) {
                out.insert(n.to_string(), d.to_string());
            }
        }
    }
    out
}

/// Los documentos de un miembro. Se filtra el árbol ya cargado en vez de
/// releerlo: un miembro **puede no ser un directorio** — un paquete importado es
/// un `.oob`, y sus documentos entran con la ruta `<el .oob>/<identidad>`.
fn solo(pkg: &Package, miembros: &[std::path::PathBuf], sitio: &std::path::Path) -> Package {
    Package {
        root: pkg.root.clone(),
        docs: pkg
            .docs
            .iter()
            .filter(|d| crate::link::miembro_de(miembros, &d.path) == Some(sitio))
            .map(|d| crate::link::Loaded {
                path: d.path.clone(),
                kind: d.kind,
                root: d.root.clone(),
            })
            .collect(),
        cedar: Vec::new(),
        generated: Vec::new(),
    }
}

// ── El esquema Cedar ────────────────────────────────────────────────────────

fn esquema_cedar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let etiquetas: Vec<String> = flow::lattices(pkg)
        .values()
        .flat_map(|l| {
            l.levels
                .iter()
                .map(|n| format!("{}:{}", l.qname, n))
                .collect::<Vec<_>>()
        })
        .collect();

    for (path, texto) in &pkg.generated {
        let faltan: Vec<&String> = etiquetas.iter().filter(|e| !texto.contains(*e)).collect();
        if faltan.is_empty() {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos2013,
                path,
                format!(
                    "el esquema Cedar comprometido no conoce {}",
                    lista(faltan.iter().map(|s| s.as_str()))
                ),
            )
            .help(
                "el retículo declara niveles que este artefacto no tiene. Una política que \
                 los mencione no fallará: dejará de casar con nada, y el dato quedará sin \
                 gobernar en silencio. Regenéralo con `ore export . --format cedarschema`",
            ),
        );
    }
}

// ── El lock ─────────────────────────────────────────────────────────────────

/// Los paquetes que el lock declara resueltos.
fn resueltos(l: &Loaded) -> BTreeSet<String> {
    l.root
        .get("packages")
        .map(|(_, p)| {
            p.items()
                .iter()
                .filter_map(|i| Some(i.get("name")?.1.as_str()?.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Las dependencias que el paquete declara, con dónde las declara.
fn declaradas(pkg: &Package) -> Vec<(String, crate::diag::Pos, std::path::PathBuf)> {
    pkg.docs
        .iter()
        .filter(|d| !normalize::es_lock(d))
        .flat_map(|d| {
            let seccion = match d.kind {
                Kind::OntologyConfig | Kind::Package => d.section("dependencies"),
                _ => None,
            };
            seccion
                .map(|s| s.items())
                .unwrap_or(&[])
                .iter()
                .filter_map(|i| {
                    let (k, v) = i.get("package")?;
                    Some((v.as_str()?.to_string(), k.pos(), d.path.clone()))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn lock(pkg: &Package, out: &mut Vec<Diagnostic>) {
    // Sin lock no hay desincronización: hay un paquete cuyas dependencias
    // todavía no se han resuelto, y eso **no es un error**. `01-package` §3.1 lo
    // permite en voz alta —«la resolución PUEDE no estar implementada en
    // v1alpha1, pero el campo DEBE existir en la gramática para que activarla
    // después no sea un cambio rompedor»—, así que declarar sin resolver es un
    // estado legítimo y no un descuido. Lo que se comprueba aquí es lo otro: un
    // lock que existe y quedó atrás.
    let Some(l) = pkg.docs.iter().find(|d| normalize::es_lock(d)) else {
        return;
    };
    let resueltos = resueltos(l);

    for (nombre, pos, origen) in declaradas(pkg) {
        if resueltos.contains(&nombre) {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos2013,
                &origen,
                format!("`{nombre}` está declarada y el lock no la resuelve"),
            )
            .at(pos)
            .help(
                "`ontology.lock` es un artefacto generado, y este quedó atrás. Sin la \
                 entrada, la clasificación y las políticas que ese paquete aporta no entran \
                 en la compilación — y el digest del bundle describiría un artefacto que \
                 nadie ha construido. `ore lock` lo reescribe: resuelve contra el árbol, \
                 así que la coordenada tiene que ser el nombre de un paquete vendorizado \
                 como miembro del workspace",
            ),
        );
    }
}

fn lista<'a>(xs: impl Iterator<Item = &'a str>) -> String {
    let v: Vec<&str> = xs.collect();
    match v.split_last() {
        None => String::new(),
        Some((ultimo, [])) => format!("`{ultimo}`"),
        Some((ultimo, resto)) => format!(
            "{} ni `{ultimo}`",
            resto
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
