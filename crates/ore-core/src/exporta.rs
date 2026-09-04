//! Lo público de un paquete — `OOS2027` y `OOS2028`.
//!
//! # Por qué existe un campo, y por qué no es la lista de lo que hay dentro
//!
//! [`ontologia-como-repositorio`](../../../docs/ontologia-como-repositorio.md)
//! §6.3 pedía que el manifiesto dijera **de qué se compone** el paquete. Al
//! medirlo salió que eso ya lo dice el directorio, y que redeclararlo sería
//! declarar lo derivable —P2—. Lo que **no** está escrito en ninguna parte del
//! árbol es otra cosa: *«esto lo expongo a propósito»*.
//!
//! Se comprobó con tres reglas candidatas sobre el corpus, y la que parecía
//! buena —*público = lo que nadie del paquete tira*— publicaba **catorce vistas
//! de casos `invalid/`**: desde dentro del paquete, una vista publicada y una
//! muerta se ven igual. No es que derivar la lista sea caro; es que **la
//! información no existe**.
//!
//! Así que es visibilidad y no membresía, y por eso se llama `exports` —como el
//! `module-info` de Java y el `package.json` de Node— y no `views`, que es lo
//! que Cognite lista en su *data model* y sí es membresía.
//!
//! # El defecto es CERRADO, y no es una elección de gusto
//!
//! Ausente significa **nada**, no *todo*. Es P4, la misma frase con la que este
//! motor rechaza un conducto no listado —*«omitirlo no es dejarlo abierto, es
//! cerrarlo»*— y la misma con la que `reads` ausente es una negativa. Java
//! exporta nada sin `exports`, Rust es privado por defecto y dbt es
//! `protected`. El único con defecto abierto es Node, y lo dice: es
//! compatibilidad hacia atrás.
//!
//! Y sale gratis: en el corpus entero **ninguna referencia cruza** la frontera
//! de un paquete dentro del mismo árbol.
//!
//! # Qué mira, y qué no
//!
//! Solo cuando **los dos extremos pertenecen a un miembro y los miembros
//! difieren**. Un documento de la raíz —el retículo, la política de conductos—
//! no es de ningún miembro y gobierna a todos: `pack` ya lo hace viajar con
//! cada `.oob`, y pedirle que se exporte sería pedirle permiso a nadie.
//!
//! Un árbol de un solo miembro no ejerce esto ni una vez, que son 294 de los
//! 298 del corpus.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package};
use crate::normalize::qualify;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Una referencia salida de un documento: a qué apunta y dónde se escribió.
struct Ref<'a> {
    /// El nombre tal como lo escribió el autor, ya cualificado.
    destino: String,
    /// Qué clase de documento espera encontrar. Sirve para dar con **el dueño**
    /// —dos documentos de kinds distintos pueden llamarse igual y vivir en
    /// miembros distintos, y sin el kind se preguntaría por el permiso del
    /// paquete equivocado—.
    ///
    /// Lo que **no** hace es discriminar la autorización: `exports` es una
    /// lista de nombres, así que exportar `hr.cosa` exporta lo que se llame
    /// así. Es deliberado: pedir `View:hr.cosa` en el manifiesto sería un
    /// vocabulario nuevo para un choque que ningún documento del corpus
    /// provoca.
    kind: Kind,
    /// Cómo se llama esta clase de referencia, para el diagnóstico.
    clase: &'static str,
    pos: crate::diag::Pos,
    desde: &'a Loaded,
}

/// El namespace del documento que escribe, para resolver la forma corta (N1).
fn ns(d: &Loaded) -> Option<&str> {
    d.meta("namespace").and_then(|n| n.as_str())
}

/// La entidad de una referencia a propiedad: `hr.Employee.baseSalary` es una
/// referencia a `hr.Employee`. Quien se acopla, se acopla a la entidad.
fn sin_propiedad(r: &str) -> &str {
    match r.rsplit_once('.') {
        Some((izq, _)) if izq.contains('.') => izq,
        _ => r,
    }
}

/// Todas las referencias que un documento escribe a otro documento.
///
/// Una sola función, y esa es la mitad del valor: hoy cada regla resuelve sus
/// propios nombres en su propio módulo, así que no había un sitio donde
/// preguntar *«¿a qué apunta este documento?»*. Ahora lo hay.
fn referencias(d: &Loaded) -> Vec<Ref<'_>> {
    let mut out = Vec::new();
    let n = ns(d);
    let mut push = |r: &str, kind: Kind, clase: &'static str, pos| {
        out.push(Ref {
            destino: qualify(r, n),
            kind,
            clase,
            pos,
            desde: d,
        });
    };

    match d.kind {
        Kind::Entity => {
            if let Some(v) = d.section("backedBy")
                && let Some(s) = v.as_str()
            {
                push(s, Kind::View, "backedBy", v.pos());
            }
            if let Some(v) = d.section("implements") {
                for i in v.items() {
                    if let Some(s) = i.as_str() {
                        push(s, Kind::Interface, "implements", i.pos());
                    }
                }
            }
            for (_, cuerpo) in d.section("properties").map(|p| p.entries()).unwrap_or(&[]) {
                if let Some((_, v)) = cuerpo.get("is")
                    && let Some(s) = v.as_str()
                {
                    push(s, Kind::Concept, "is", v.pos());
                }
                if let Some((_, v)) = cuerpo.get("derivedFrom") {
                    for i in v.items() {
                        if let Some(s) = i.as_str() {
                            push(sin_propiedad(s), Kind::Entity, "derivedFrom", i.pos());
                        }
                    }
                }
            }
            for (_, rel) in d.section("relations").map(|r| r.entries()).unwrap_or(&[]) {
                if let Some((_, v)) = rel.get("target")
                    && let Some(s) = v.as_str()
                {
                    push(s, Kind::Entity, "relations.target", v.pos());
                }
            }
        }
        Kind::View => {
            if let Some(from) = d.section("from") {
                if let Some((_, v)) = from.get("view")
                    && let Some(s) = v.as_str()
                {
                    push(s, Kind::View, "from.view", v.pos());
                }
                if let Some((_, v)) = from.get("table")
                    && let Some(s) = v.as_str()
                {
                    push(s, Kind::Table, "from.table", v.pos());
                }
            }
        }
        Kind::Interface => {
            if let Some(v) = d.section("requires") {
                for i in v.items() {
                    if let Some(s) = i.as_str() {
                        push(s, Kind::Concept, "requires", i.pos());
                    }
                }
            }
        }
        Kind::Binding => {
            if let Some(v) = d.section("targetEntity")
                && let Some(s) = v.as_str()
            {
                push(s, Kind::Entity, "targetEntity", v.pos());
            }
        }
        Kind::Function => {
            for e in d.section("effects").map(|e| e.items()).unwrap_or(&[]) {
                if let Some((_, v)) = e.get("writes")
                    && let Some(s) = v.as_str()
                {
                    push(sin_propiedad(s), Kind::Entity, "effects.writes", v.pos());
                }
            }
        }
        Kind::Resolution => {
            if let Some(v) = d.section("entity")
                && let Some(s) = v.as_str()
            {
                push(s, Kind::Entity, "entity", v.pos());
            }
        }
        _ => {}
    }
    out
}

/// Lo que cada miembro exporta, ya cualificado con el namespace de su
/// manifiesto si lo tuviera.
fn exportado(pkg: &Package, miembros: &[PathBuf]) -> BTreeMap<PathBuf, BTreeSet<String>> {
    let mut out: BTreeMap<PathBuf, BTreeSet<String>> = miembros
        .iter()
        .map(|m| (m.clone(), BTreeSet::new()))
        .collect();
    for p in pkg.of(Kind::Package) {
        let Some(sitio) = p.path.parent() else {
            continue;
        };
        let Some(v) = p.section("exports") else {
            continue;
        };
        let e = out.entry(sitio.to_path_buf()).or_default();
        for i in v.items() {
            if let Some(s) = i.as_str() {
                e.insert(qualify(s, ns(p)));
            }
        }
    }
    out
}

pub fn comprobar(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let miembros = crate::link::miembros(pkg);
    let exports = exportado(pkg, &miembros);

    // El índice: nombre cualificado y kind -> el miembro que lo contiene. Con
    // el kind dentro de la clave a propósito: una tabla y una vista pueden
    // llamarse igual, y confundirlas exportaría una por la otra.
    //
    // El kind entra como su `as_str()` y no como el `enum`: darle `Ord` a
    // `Kind` para ordenar un mapa de aquí sería pedirle a un tipo compartido
    // que cargue con una necesidad de un módulo.
    let mut donde: BTreeMap<(String, &'static str), &Path> = BTreeMap::new();
    for d in &pkg.docs {
        let (Some(qn), Some(m)) = (d.qname(), crate::link::miembro_de(&miembros, &d.path)) else {
            continue;
        };
        donde.insert((qn, d.kind.as_str()), m);
    }

    // ── OOS2027 · la lista nombra algo que el paquete no tiene ──────────────
    for p in pkg.of(Kind::Package) {
        let (Some(sitio), Some(v)) = (p.path.parent(), p.section("exports")) else {
            continue;
        };
        for i in v.items() {
            let Some(s) = i.as_str() else { continue };
            let qn = qualify(s, ns(p));
            let suyo = donde
                .iter()
                .find(|((n, _), m)| *n == qn && **m == sitio)
                .is_some();
            if suyo {
                continue;
            }
            let en_otro = donde.keys().find(|(n, _)| *n == qn).is_some();
            out.push(
                Diagnostic::new(
                    Code::Oos2027,
                    &p.path,
                    format!("`exports` nombra `{qn}`, que este paquete no contiene"),
                )
                .at(i.pos())
                .help(if en_otro {
                    "existe, pero es de otro paquete. Un paquete solo puede exportar lo suyo: \
                     lo de la dependencia lo exporta ella, y aquí se declara `dependencies`"
                } else {
                    "no hay ningún documento con ese nombre en el árbol. Revisa la errata, o \
                     quita el nombre de la lista: exportar lo que no existe no es un aviso, \
                     es una promesa que nadie puede cumplir"
                }),
            );
        }
    }

    // ── OOS2028 · una referencia cruza a un paquete que no la exporta ───────
    //
    // **Solo de v1alpha8**, y con el argumento textual de `OOS2022`: un
    // documento anterior declaro su version, y esa version no tenia una
    // superficie publica que respetar. Aplicarsela cambiaria lo que significa
    // un documento ya escrito, y el invariante que esta linea de trabajo
    // sostiene es que NO CAMBIA UN SOLO RESULTADO de v1alpha1 a v1alpha7.
    //
    // Se midio sin la puerta: caian dos casos de `conformance/v1alpha4` —
    // `valid/concept-from-another-package` y
    // `valid/vocabulary-member-has-no-entities`—, que son exactamente los dos
    // unicos cruces del corpus y son los dos el mismo: un paquete de
    // vocabulario del que otros toman autoridad. Con la puerta, cero.
    //
    // Decide la version del documento que ESCRIBE la referencia, no la del
    // que la recibe: quien se acoplo lo hizo bajo unas reglas, y son las
    // suyas las que valen.
    for d in &pkg.docs {
        if d.version()
            .is_none_or(|v| v < crate::document::ApiVersion::V1Alpha8)
        {
            continue;
        }
        let Some(mio) = crate::link::miembro_de(&miembros, &d.path) else {
            continue;
        };
        for r in referencias(d) {
            let Some(suyo) = donde.get(&(r.destino.clone(), r.kind.as_str())) else {
                continue; // no resuelve: lo dice quien comprueba esa referencia
            };
            if *suyo == mio {
                continue;
            }
            if exports.get(*suyo).is_some_and(|e| e.contains(&r.destino)) {
                continue;
            }
            let dueno = suyo.file_name().unwrap_or_default().to_string_lossy();
            out.push(
                Diagnostic::new(
                    Code::Oos2028,
                    &r.desde.path,
                    format!(
                        "`{}: {}` cruza a `{dueno}`, que no lo exporta",
                        r.clase, r.destino
                    ),
                )
                .at(r.pos)
                .help(format!(
                    "existe, y por eso esto no es `OOS2018`: está en otro paquete y ese \
                     paquete no lo hace público. Añádelo a `exports` de `{dueno}`, o deja de \
                     nombrarlo. Un `exports` ausente no significa «todo»: significa nada"
                )),
            );
        }
    }

    out
}
