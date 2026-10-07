//! Lo público de un paquete — `OOS2027`.
//!
//! # Dentro de un árbol no hay frontera (v1alpha28, ORE 0059)
//!
//! Un árbol es **un catálogo**, como un *metastore* de Unity: sus bases se leen
//! por su nombre, y quién lee qué lo decide el acceso al servir (0047 A8), no
//! el compilador. `OOS2028` —una referencia que cruza a un paquete que no la
//! exporta— ya no se aplica entre miembros de un árbol, **sea cual sea la
//! versión** de su config: la regla solo quitaba errores, así que no hay árbol
//! que deje de compilar. `exports` queda como la superficie de un paquete
//! publicado hacia otro árbol (`dependencies`), que se resuelve por el lock y
//! el registro y no se ve desde aquí. Lo de abajo cuenta por qué nació.
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
use std::collections::BTreeMap;
use std::path::Path;

/// Una referencia salida de un documento: a qué apunta y dónde se escribió.
pub struct Ref<'a> {
    /// El nombre tal como lo escribió el autor, ya cualificado.
    pub destino: String,
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
    pub kind: Kind,
    /// Cómo se llama esta clase de referencia, para el diagnóstico.
    pub clase: &'static str,
    pub pos: crate::diag::Pos,
    pub desde: &'a Loaded,
}

/// El namespace del documento que escribe, para resolver la forma corta (N1).
fn ns(d: &Loaded) -> Option<&str> {
    d.meta("namespace").and_then(|n| n.as_str())
}

/// La entidad de una referencia a propiedad: `hr.Employee.baseSalary` es una
/// referencia a `hr.Employee`. Quien se acopla, se acopla a la entidad.
///
/// La propiedad es **siempre el ultimo segmento** —el campo (`derivedFrom`,
/// `writes`) dice que nombra una propiedad—, y lo de delante es la entidad en
/// una, dos o tres partes (v1alpha13 01 §5). Antes se adivinaba por el numero
/// de puntos, y con tres niveles `hr.rrhh.Employee` se habria cortado a
/// `hr.rrhh`; y `Employee.baseSalary`, sin paquete, no se cortaba.
fn sin_propiedad(r: &str) -> &str {
    r.rsplit_once('.').map(|(izq, _)| izq).unwrap_or(r)
}

/// Todas las referencias que un documento escribe a otro documento.
///
/// Una sola función, y esa es la mitad del valor: hoy cada regla resuelve sus
/// propios nombres en su propio módulo, así que no había un sitio donde
/// preguntar *«¿a qué apunta este documento?»*. Ahora lo hay.
pub fn referencias(d: &Loaded) -> Vec<Ref<'_>> {
    let mut out = Vec::new();
    let n = ns(d);
    let mut push = |r: &str, kind: Kind, clase: &'static str, pos| {
        out.push(Ref {
            // v1alpha13: lo del catalogo, con el schema de quien escribe.
            destino: if kind.con_schema() {
                crate::link::cualificar(r, d)
            } else {
                qualify(r, n)
            },
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
                // v1alpha16: `Media<x>` apunta a una coleccion, y quien la
                // referencia desde otro paquete se acopla a ella.
                if let Some((_, v)) = cuerpo.get("type")
                    && let Ok(crate::types::Type::Media(c)) =
                        crate::types::parse_type(v.as_str().unwrap_or(""))
                {
                    push(&c, Kind::MediaCollection, "Media", v.pos());
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
        // v1alpha12: el dataset mantenido sale de lo mismo que una vista, y
        // los dos pueden salir de un dataset.
        Kind::View | Kind::Dataset => {
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
                if let Some((_, v)) = from.get("dataset")
                    && let Some(s) = v.as_str()
                {
                    push(s, Kind::Dataset, "from.dataset", v.pos());
                }
            }
        }
        // v1alpha16: la coleccion mantenida sale del `ObjectTable` de la
        // fuente, que vive en otro paquete (0045): cruza, y tiene que estar
        // exportado.
        Kind::MediaCollection => {
            if let Some((_, v)) = d.section("from").and_then(|f| f.get("objectTable"))
                && let Some(s) = v.as_str()
            {
                push(s, Kind::ObjectTable, "from.objectTable", v.pos());
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
        Kind::Function | Kind::Action => {
            // `effects` de la funcion, `sets` de la accion: la misma superficie.
            for seccion in ["effects", "sets"] {
                for e in d.section(seccion).map(|e| e.items()).unwrap_or(&[]) {
                    if let Some((_, v)) = e.get("writes")
                        && let Some(s) = v.as_str()
                    {
                        push(sin_propiedad(s), Kind::Entity, "writes", v.pos());
                    }
                }
            }
            // v1alpha10: lo que lee cruza la frontera igual que lo que escribe.
            if let Some(v) = d.section("over")
                && let Some(s) = v.as_str()
            {
                push(s, Kind::View, "over", v.pos());
            }
            for r in d.section("reads").map(|r| r.items()).unwrap_or(&[]) {
                if let Some(s) = r.as_str() {
                    push(s, Kind::View, "reads", r.pos());
                }
            }
            if let Some(v) = d.section("call")
                && let Some(s) = v.as_str()
            {
                push(s, Kind::Function, "call", v.pos());
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

pub fn comprobar(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let miembros = crate::link::miembros(pkg);

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
            let qn =
                crate::normalize::qualify_catalogo(s, ns(p), crate::normalize::SCHEMA_POR_DEFECTO);
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

    out
}
