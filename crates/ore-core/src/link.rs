//! Enlazado — la familia `OOS2xxx`.
//!
//! Es la fase donde el paquete deja de ser una colección de ficheros y pasa a
//! ser un grafo. Casi todo lo que se comprueba aquí **no está en ningún
//! documento por separado**: que un binding cubra la clave primaria de su
//! entidad exige leer los dos, y ninguno de ellos contiene el error.
//!
//! Es también donde se ve por qué la validación de esquema es la mitad fácil.
//! `entity.schema.json` sabe que `target` casa el patrón de un nombre
//! cualificado; **resolver ese nombre exige el paquete entero**.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::parse::Node;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Un documento ya analizado y despachado.
pub struct Loaded {
    pub path: PathBuf,
    pub kind: Kind,
    pub root: Node,
}

impl Loaded {
    /// Las secciones de `OntologyConfig` cuelgan de la raíz; las de los demás,
    /// de `spec`.
    pub fn section(&self, key: &str) -> Option<&Node> {
        if self.kind.sections_at_root() {
            self.root.get(key).map(|(_, v)| v)
        } else {
            self.root
                .get("spec")
                .and_then(|(_, s)| s.get(key))
                .map(|(_, v)| v)
        }
    }

    pub fn meta(&self, key: &str) -> Option<&Node> {
        self.root
            .get("metadata")
            .and_then(|(_, m)| m.get(key))
            .map(|(_, v)| v)
    }

    /// La versión que **este documento** declara.
    ///
    /// Hacía falta desde que una regla puede ser de una versión y no de las
    /// anteriores. El `apiVersion` es por documento —es lo que hace que la
    /// migración no roce— así que quien comprueba una regla acotada tiene que
    /// poder preguntárselo al documento, y no al paquete.
    ///
    /// `None` si no la declara o no se entiende: el despacho ya lo habría
    /// rechazado con `OOS1002`, así que eso solo pasa sobre algo que nadie
    /// validó.
    pub fn version(&self) -> Option<crate::document::ApiVersion> {
        self.root
            .get("apiVersion")
            .and_then(|(_, v)| v.as_str())
            .and_then(crate::document::ApiVersion::parse)
    }

    /// `<namespace>.<name>`, o solo `<name>` si el documento no lleva espacio de
    /// nombres.
    pub fn qname(&self) -> Option<String> {
        let name = self.meta("name")?.as_str()?;
        Some(match self.meta("namespace").and_then(|n| n.as_str()) {
            Some(ns) => format!("{ns}.{name}"),
            None => name.to_string(),
        })
    }
}

/// El sitio de cada miembro del workspace: donde esta su `package.yaml`.
///
/// Los miembros son **ubicaciones**, y no es una convencion de este motor: el
/// esquema de `OntologyConfig` declara `workspace.members` como una lista de
/// patrones de ruta con `packages/*` por defecto, y dice que *«el valor por
/// defecto lo aplica el COMPILADOR al normalizar»*. Se derivan de donde esta
/// cada `package.yaml` en vez de expandir el patron porque el resultado es el
/// mismo y una disposicion no estandar —que es justo para lo que existe
/// declarar `members`— sigue funcionando sin leerla.
///
/// Y un `.oob` sale de aqui como cualquier otro miembro: sus documentos entran
/// con la ruta `<el .oob>/<identidad>`, asi que el fichero ES su directorio.
/// Un paquete importado se atribuye solo.
pub fn miembros(pkg: &Package) -> Vec<PathBuf> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|d| d.path.parent().map(Path::to_path_buf))
        .collect()
}

/// El miembro al que pertenece un documento: el mas largo que es prefijo de su
/// ruta.
///
/// El mas largo y no el primero, porque un miembro puede estar dentro de otro y
/// entonces manda el de dentro.
pub fn miembro_de<'a>(miembros: &'a [PathBuf], doc: &Path) -> Option<&'a Path> {
    miembros
        .iter()
        .filter(|m| doc.starts_with(m))
        .max_by_key(|m| m.components().count())
        .map(PathBuf::as_path)
}

/// Lo que un paquete PUBLICA, que no es todo lo que tiene cargado.
///
/// El manifiesto es del **workspace** —lleva las fuentes fisicas y las
/// dependencias de quien compila— y el lock es de quien consume; ninguno de los
/// dos es del paquete. Vive aqui y no en quien empaqueta porque los tres que lo
/// necesitan tienen que estar de acuerdo: el que escribe un `.oob`, el que
/// resuelve un lock y el que verifica que lo que hay es lo que el lock nombra.
/// Con dos definiciones, el mismo paquete tendria dos digests — y eso ya paso.
pub fn publicables(pkg: &Package) -> Package {
    Package {
        root: pkg.root.clone(),
        docs: pkg
            .docs
            .iter()
            .filter(|d| d.kind != Kind::OntologyConfig)
            .filter(|d| !crate::normalize::es_lock(d))
            .map(|d| Loaded {
                path: d.path.clone(),
                kind: d.kind,
                root: d.root.clone(),
            })
            .collect(),
        cedar: Vec::new(),
        generated: Vec::new(),
        sobres: Vec::new(),
    }
}

/// Un paquete cargado: todos sus documentos, ya despachados.
pub struct Package {
    pub root: PathBuf,
    pub docs: Vec<Loaded>,
    /// Las políticas Cedar, como texto. No son YAML y no pueden ser un `Node`;
    /// OOS no define un lenguaje de autorización — las políticas **son** Cedar.
    pub cedar: Vec<(PathBuf, String)>,
    /// Los artefactos **generados** que el repositorio compromete: hoy el
    /// esquema Cedar. Se comprometen para que el tooling funcione sin compilar,
    /// y por eso pueden quedar obsoletos — `OOS2013`.
    pub generated: Vec<(PathBuf, String)>,
    /// Los **sobres** de los `.oob` del arbol: la ruta del fichero y lo que
    /// dice de si mismo.
    ///
    /// El cargador los guarda y no los interpreta, que es la division que
    /// `abrir_oob` deja escrita: cargar es leer, y decidir que se puede usar es
    /// de `sync`. Sin esto la firma habria tenido que comprobarse en el
    /// cargador, y un cargador que decide que se usa es exactamente la clase de
    /// decision que este proyecto pone donde se pueda revisar.
    pub sobres: Vec<(PathBuf, Node)>,
}

impl Package {
    pub(crate) fn of(&self, kind: Kind) -> impl Iterator<Item = &Loaded> {
        self.docs.iter().filter(move |d| d.kind == kind)
    }

    /// Todas las entidades del paquete.
    pub fn entities(&self) -> impl Iterator<Item = &Loaded> {
        self.of(Kind::Entity)
    }

    /// El `RequestPolicy` del paquete, si lo hay.
    ///
    /// Accesor con nombre y no `of(Kind::…)` publico: **como mucho hay uno**, y
    /// devolver un iterador insinuaria que puede haber varios — dos fronteras de
    /// confianza sin nada que dijera cual manda.
    pub fn request_policy(&self) -> Option<&Loaded> {
        self.of(Kind::RequestPolicy).next()
    }

    pub fn entity(&self, qname: &str) -> Option<&Loaded> {
        self.of(Kind::Entity)
            .find(|d| d.qname().as_deref() == Some(qname))
    }

    /// Resuelve una referencia **tal como la escribió el autor**: la forma
    /// corta es legal dentro del mismo espacio de nombres (N1).
    pub fn resolve_entity(&self, referencia: &str, desde: &Loaded) -> Option<&Loaded> {
        let ns = desde.meta("namespace").and_then(|n| n.as_str());
        self.entity(&crate::normalize::qualify(referencia, ns))
    }
}

/// Nombres de las propiedades declaradas por una entidad.
fn properties(e: &Loaded) -> BTreeSet<String> {
    e.section("properties")
        .map(|p| {
            p.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub fn link(pkg: &Package) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    package_metadata(pkg, &mut d);
    dependencies(pkg, &mut d);
    datasources(pkg, &mut d);
    entities(pkg, &mut d);
    bindings(pkg, &mut d);
    // Las vistas y `backedBy`: la fuente declarada, la cadena que resuelve y no
    // se muerde, y la clave expuesta. Viven en su modulo porque la cadena es
    // una operacion —componer renombres— que `flow` y el ejecutor tambien
    // necesitan, y con dos copias divergirian.
    crate::vistas::comprobar(pkg, &mut d);
    // OOS2014 vive aparte porque no mira UN binding: mira los que comparten
    // objeto y decide si reparten las filas o se pisan.
    crate::selector::comprobar(pkg, &mut d);
    secrets(pkg, &mut d);
    d
}

// ── OOS2007 · OOS2008 · OOS2009 ─────────────────────────────────────────────

const ESTADOS_ODCS: &[&str] = &["proposed", "draft", "active", "deprecated", "retired"];

fn package_metadata(pkg: &Package, out: &mut Vec<Diagnostic>) {
    // El dueño de la superficie de seguridad, con el mismo vocabulario y el
    // mismo código: `owner` es `owner` lo declare quien lo declare, y dos formas
    // de escribir un handle acabarían con dos maneras de resolverlo contra
    // CODEOWNERS.
    for cp in pkg.of(Kind::ConduitPolicy) {
        if let Some(v) = cp.section("owner") {
            let s = v.as_str().unwrap_or("");
            if !es_handle(s) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2009,
                        &cp.path,
                        format!("`owner: {s}` no es un handle"),
                    )
                    .at(v.pos())
                    .help(
                        "usa `team:<handle>` o `user:<handle>`: es lo que se alinea con                          CODEOWNERS, que es quien hace cumplir la revisión",
                    ),
                );
            }
        }
    }

    for p in pkg.of(Kind::Package) {
        if let Some(v) = p.meta("version")
            && !es_semver(v.as_str().unwrap_or(""))
        {
            out.push(
                Diagnostic::new(
                    Code::Oos2007,
                    &p.path,
                    format!("`{}` no es semver 2.0.0", v.as_str().unwrap_or("")),
                )
                .at(v.pos())
                .help(
                    "sin semver no se puede escribir `^2.1` en una dependencia, \
                         ni ordenar, ni comprobar que la versión corresponde a los cambios",
                ),
            );
        }
        if let Some(v) = p.meta("status") {
            let s = v.as_str().unwrap_or("");
            if !ESTADOS_ODCS.contains(&s) {
                let mut diag = Diagnostic::new(
                    Code::Oos2008,
                    &p.path,
                    format!("`status: {s}` no pertenece al vocabulario de ODCS"),
                )
                .at(v.pos());
                diag = if s.eq_ignore_ascii_case("stable") {
                    diag.help(
                        "`STABLE` existe, pero en el retículo `oos.maturity`, que es otra cosa. \
                         `status` usa el vocabulario de ODCS: proposed · draft · active · \
                         deprecated · retired, y el nivel de madurez se deriva de él",
                    )
                } else {
                    diag.help(format!("admitidos: {}", ESTADOS_ODCS.join(" · ")))
                };
                out.push(diag);
            }
        }
        match p.section("owner") {
            None => {
                out.push(Diagnostic::new(Code::Oos2009, &p.path, "falta `owner`").at(p.root.pos()))
            }
            Some(v) => {
                let s = v.as_str().unwrap_or("");
                if !es_handle(s) {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2009,
                            &p.path,
                            format!("`owner: {s}` no es un handle"),
                        )
                        .at(v.pos())
                        .help(
                            "usa `team:<handle>` o `user:<handle>`: es lo que se alinea con \
                             CODEOWNERS, que es quien hace cumplir la revisión. Un nombre \
                             libre no se puede resolver contra el control de versiones",
                        ),
                    );
                }
            }
        }
    }
}

fn es_semver(s: &str) -> bool {
    let core = s.split(['-', '+']).next().unwrap_or("");
    let partes: Vec<&str> = core.split('.').collect();
    partes.len() == 3
        && partes.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}

fn es_handle(s: &str) -> bool {
    let Some((tipo, h)) = s.split_once(':') else {
        return false;
    };
    matches!(tipo, "team" | "user")
        && h.starts_with(|c: char| c.is_ascii_lowercase())
        && h.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ── OOS2002 · OOS2003 ───────────────────────────────────────────────────────

fn dependencias_de(d: &Loaded) -> Vec<(&Node, String)> {
    d.section("dependencies")
        .map(|n| {
            n.items()
                .iter()
                .filter_map(|it| {
                    it.get("package")
                        .map(|(_, v)| (it, v.as_str().unwrap_or("").to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn dependencies(pkg: &Package, out: &mut Vec<Diagnostic>) {
    // OOS2003 · duplicado por CLAVE SEMÁNTICA. `uniqueItems` del esquema no lo
    // ve: los dos elementos son objetos literalmente distintos, y saber que la
    // clave del conjunto es el nombre del paquete es conocimiento del dominio.
    for d in &pkg.docs {
        let mut vistos: BTreeMap<String, usize> = BTreeMap::new();
        for (nodo, nombre) in dependencias_de(d) {
            if let Some(&antes) = vistos.get(&nombre) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2003,
                        &d.path,
                        format!("`{nombre}` declarado dos veces en `dependencies`"),
                    )
                    .at(nodo.pos())
                    .help(format!(
                        "la primera está en la línea {antes}. Un paquete se importa una vez: \
                         importar es transferir autoridad, y dos rangos distintos no dicen \
                         a cuál te acoges"
                    )),
                );
            } else {
                vistos.insert(nombre, nodo.pos().line);
            }
        }
    }

    // OOS2002 · ciclo. El grafo transitivo vive en el lock: sin él, detectarlo
    // exigiría red y la compilación dejaría de ser hermética.
    let Some(lock) = pkg
        .docs
        .iter()
        .find(|d| d.path.file_name().is_some_and(|n| n == "ontology.lock"))
    else {
        return;
    };
    let mut grafo: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some((_, ps)) = lock.root.get("packages") {
        for p in ps.items() {
            let Some(nombre) = p.get("name").and_then(|(_, v)| v.as_str()) else {
                continue;
            };
            let deps = p
                .get("dependencies")
                .map(|(_, d)| {
                    d.items()
                        .iter()
                        .filter_map(|x| x.get("name").and_then(|(_, v)| v.as_str()))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            grafo.insert(nombre.to_string(), deps);
        }
    }
    if let Some(ciclo) = buscar_ciclo(&grafo) {
        out.push(
            Diagnostic::new(
                Code::Oos2002,
                &lock.path,
                format!("ciclo en el grafo de dependencias: {}", ciclo.join(" → ")),
            )
            .at(lock.root.pos())
            .help(
                "importar es transferir autoridad; un ciclo significa que dos paquetes se \
                 delegan la decisión el uno al otro sin que nadie la tome",
            ),
        );
    }
}

fn buscar_ciclo(grafo: &BTreeMap<String, Vec<String>>) -> Option<Vec<String>> {
    fn visitar(
        n: &str,
        grafo: &BTreeMap<String, Vec<String>>,
        pila: &mut Vec<String>,
        hecho: &mut BTreeSet<String>,
    ) -> Option<Vec<String>> {
        if let Some(i) = pila.iter().position(|x| x == n) {
            let mut c = pila[i..].to_vec();
            c.push(n.to_string());
            return Some(c);
        }
        if hecho.contains(n) {
            return None;
        }
        pila.push(n.to_string());
        for v in grafo.get(n).into_iter().flatten() {
            if let Some(c) = visitar(v, grafo, pila, hecho) {
                return Some(c);
            }
        }
        pila.pop();
        hecho.insert(n.to_string());
        None
    }
    let mut hecho = BTreeSet::new();
    for n in grafo.keys() {
        if let Some(c) = visitar(n, grafo, &mut Vec::new(), &mut hecho) {
            return Some(c);
        }
    }
    None
}

// ── OOS2004 ─────────────────────────────────────────────────────────────────

fn datasources(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let declarados: BTreeSet<String> = pkg
        .of(Kind::OntologyConfig)
        .filter_map(|c| c.section("datasources"))
        .flat_map(|n| n.items())
        .filter_map(|it| {
            it.get("name")
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        })
        .collect();

    for b in pkg.of(Kind::Binding) {
        let Some(v) = b.section("datasourceRef") else {
            continue;
        };
        let r = v.as_str().unwrap_or("");
        if !declarados.contains(r) {
            out.push(
                Diagnostic::new(
                    Code::Oos2004,
                    &b.path,
                    format!("`datasourceRef: {r}` no está declarado en el manifiesto raíz"),
                )
                .at(v.pos())
                .help(if declarados.is_empty() {
                    "el manifiesto no declara ningún datasource".to_string()
                } else {
                    format!(
                        "declarados: {}",
                        declarados.iter().cloned().collect::<Vec<_>>().join(" · ")
                    )
                }),
            );
        }
    }
}

// ── OOS2005 · OOS2006 · OOS2010 ─────────────────────────────────────────────

fn entities(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for e in pkg.of(Kind::Entity) {
        let props = properties(e);
        let qn = e.qname().unwrap_or_default();

        // OOS2010 · identidad
        let nature = e.section("nature").and_then(|n| n.as_str()).unwrap_or("");
        let falta = match nature {
            "entity" => e
                .section("primaryKey")
                .is_none()
                .then_some(("primaryKey", "entity")),
            "event" => e
                .section("timeKey")
                .is_none()
                .then_some(("timeKey", "event")),
            _ => None,
        };
        if let Some((campo, nat)) = falta {
            out.push(
                Diagnostic::new(
                    Code::Oos2010,
                    &e.path,
                    format!("`{qn}` declara `nature: {nat}` y no tiene `{campo}`"),
                )
                .at(e.root.pos())
                .help(if nat == "entity" {
                    "sin identidad no hay índice de topología, ni recurso identificable en \
                     una política, ni forma de responder a una solicitud de acceso. Si los \
                     registros no tienen identidad estable —un log, un tema de Kafka— \
                     probablemente sea `nature: event` con `timeKey`"
                } else {
                    "un `event` se sitúa en el tiempo aunque no se identifique"
                }),
            );
        }

        // OOS2006 · nombres reservados
        let reservados: BTreeMap<String, &Node> = e
            .section("reserved")
            .map(|r| {
                r.items()
                    .iter()
                    .filter_map(|it| {
                        it.get("name")
                            .map(|(_, v)| (v.as_str().unwrap_or("").to_string(), v))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(ps) = e.section("properties") {
            for (k, _) in ps.entries() {
                let Some(n) = k.as_str() else { continue };
                if let Some(decl) = reservados.get(n) {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2006,
                            &e.path,
                            format!("`{n}` está reservado y no puede reutilizarse"),
                        )
                        .at(k.pos())
                        .help(format!(
                            "declarado en `reserved` en la línea {}. Reutilizar un nombre \
                             retirado hace que una consulta antigua devuelva una cifra \
                             correcta para la pregunta equivocada",
                            decl.pos().line
                        )),
                    );
                }
            }
        }

        // OOS2005 · referencias
        for campo in ["primaryKey", "timeKey", "uniqueKeys"] {
            let Some(n) = e.section(campo) else { continue };
            let refs: Vec<&Node> = match n {
                Node::Scalar { .. } => vec![n],
                Node::Sequence { items, .. } => items
                    .iter()
                    .flat_map(|i| match i {
                        Node::Sequence { items, .. } => items.iter().collect::<Vec<_>>(),
                        otro => vec![otro],
                    })
                    .collect(),
                _ => vec![],
            };
            for r in refs {
                let Some(p) = r.as_str() else { continue };
                if !props.contains(p) {
                    out.push(referencia_rota(&e.path, r, &format!("{qn}.{p}"), campo));
                }
            }
        }

        // `derivedFrom` referencia propiedades por nombre cualificado. Resolverlas
        // aquí es prerrequisito de OOS3004: no se comprueba la unidad de algo que
        // no existe.
        if let Some(ps) = e.section("properties") {
            for (_, pv) in ps.entries() {
                let Some((_, from)) = pv.get("derivedFrom") else {
                    continue;
                };
                for r in from.items() {
                    let Some(q) = r.as_str() else { continue };
                    let Some((ent, prop)) = q.rsplit_once('.') else {
                        out.push(referencia_rota(&e.path, r, q, "derivedFrom"));
                        continue;
                    };
                    let existe = if ent == qn {
                        props.contains(prop)
                    } else {
                        pkg.entity(ent)
                            .map(|o| properties(o).contains(prop))
                            .unwrap_or(false)
                    };
                    if !existe {
                        out.push(referencia_rota(&e.path, r, q, "derivedFrom"));
                    }
                }
            }
        }

        if let Some(rels) = e.section("relations") {
            for (rk, rv) in rels.entries() {
                let rn = rk.as_str().unwrap_or("");
                if let Some((_, t)) = rv.get("target") {
                    let target = t.as_str().unwrap_or("");
                    if pkg.resolve_entity(target, e).is_none() {
                        out.push(referencia_rota(
                            &e.path,
                            t,
                            target,
                            &format!("relations.{rn}.target"),
                        ));
                    }
                }
                // `toKey` nombra propiedades del DESTINO, no locales: se
                // resuelve contra la otra entidad o no se resuelve contra nada.
                if let Some((_, tk)) = rv.get("toKey")
                    && let Some((_, t)) = rv.get("target")
                    && let Some(otra) = pkg.resolve_entity(t.as_str().unwrap_or(""), e)
                {
                    let suyas = properties(otra);
                    let dqn = otra.qname().unwrap_or_default();
                    for nodo in tk.items() {
                        let k = nodo.as_str().unwrap_or("");
                        if !suyas.contains(k) {
                            out.push(referencia_rota(
                                &e.path,
                                nodo,
                                &format!("{dqn}.{k}"),
                                &format!("relations.{rn}.toKey"),
                            ));
                        }
                    }
                }
                if let Some((_, v)) = rv.get("via") {
                    for nodo in v.items() {
                        let via = nodo.as_str().unwrap_or("");
                        if !props.contains(via) {
                            out.push(referencia_rota(
                                &e.path,
                                nodo,
                                &format!("{qn}.{via}"),
                                &format!("relations.{rn}.via"),
                            ));
                        }
                    }
                }
            }
        }
    }
}

fn referencia_rota(path: &Path, nodo: &Node, referencia: &str, campo: &str) -> Diagnostic {
    Diagnostic::new(Code::Oos2005, path, format!("`{referencia}` no existe"))
        .at(nodo.pos())
        .help(format!(
            "`{campo}` está bien formado como nombre, pero no resuelve. Resolver un nombre \
             exige el paquete entero, no el documento: es lo que un esquema JSON no alcanza"
        ))
}

// ── OOS2011 ─────────────────────────────────────────────────────────────────

fn bindings(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for b in pkg.of(Kind::Binding) {
        let Some(t) = b.section("targetEntity") else {
            continue;
        };
        let target = t.as_str().unwrap_or("");
        let Some(e) = pkg.resolve_entity(target, b) else {
            out.push(referencia_rota(&b.path, t, target, "targetEntity"));
            continue;
        };

        let mapeadas: BTreeSet<String> = b
            .section("properties")
            .map(|p| {
                p.entries()
                    .iter()
                    .filter_map(|(k, _)| k.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mut exigidas: Vec<String> = e
            .section("primaryKey")
            .map(|k| {
                k.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Y las propiedades de cada `via`. El esquema del binding ya lo decia
        // —*«las relaciones no se mapean: el enlace fisico sale de mapear la
        // propiedad `via`»*— y nadie lo comprobaba: un binding que mapeaba la
        // clave y ninguna propiedad del enlace validaba limpio, dejando una
        // relacion sin columna fisica por ningun lado. Tiene exactamente el
        // mismo aspecto que una relacion enlazable.
        if let Some(rels) = e.section("relations") {
            for (_, rv) in rels.entries() {
                if let Some((_, v)) = rv.get("via") {
                    exigidas.extend(
                        v.items()
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from)),
                    );
                }
            }
        }

        // Y lo que se REPLICA. Una copia necesita una columna de la que copiar,
        // igual que una clave o un enlace: `payload.properties` fuera del mapeo
        // es una replica sin origen. Validaba limpio, incluso nombrando una
        // propiedad que la entidad no tiene.
        if let Some(pl) = b
            .section("materialization")
            .and_then(|m| m.get("payload").map(|(_, v)| v))
            .and_then(|pl| pl.get("properties").map(|(_, v)| v))
        {
            exigidas.extend(
                pl.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from)),
            );
        }
        exigidas.sort();
        exigidas.dedup();

        // OOS2015 · un filtro que el origen EXIGE y el binding no mapea deja la fuente
        // inconsultable: el motor no tiene con que construirlo. Compila y no sirve.
        for f in b
            .section("capabilities")
            .and_then(|c| c.get("requiredFilters").map(|(_, v)| v.items()))
            .unwrap_or(&[])
        {
            let Some(prop) = f.as_str() else { continue };
            if !mapeadas.contains(prop) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2015,
                        &b.path,
                        format!("`{target}` exige filtrar por `{prop}`, que el mapeo no cubre"),
                    )
                    .at(f.pos())
                    .help(
                        "`requiredFilters` dice que el origen NO acepta una consulta sin ese \
                         filtro. Sin la propiedad en el mapeo el motor no tiene con qué \
                         construirlo, así que este binding compila y no se puede consultar \
                         nunca",
                    ),
                );
            }
        }

        // OOS2015 · y el otro origen del MISMO defecto. Un ambito de fila
        // (`v1alpha3/02-ruleset` §4.2) recorta por una columna, y si el binding
        // no la mapea el motor tampoco tiene con que construir ese filtro.
        //
        // Que un requisito de la FUENTE y un requisito de la POLITICA produzcan
        // el mismo estado no es una coincidencia: son el mismo defecto. Y este
        // pesa mas, porque `05-ejecutor` §3 obliga a rechazar el plan antes que
        // servirlo sin recorte — un binding asi deja la entidad inconsultable
        // para toda politica que nombre ese ambito.
        let qn = e.qname().unwrap_or_default();
        for r in pkg.of(Kind::Ruleset) {
            for s in r.section("scopes").map(|n| n.items()).unwrap_or(&[]) {
                let Some((_, v)) = s.get("property") else {
                    continue;
                };
                let Some(prop) = v.as_str() else { continue };
                let Some(corta) = prop.strip_prefix(&format!("{qn}.")) else {
                    continue;
                };
                // Si la propiedad NO EXISTE, el defecto es la referencia rota y
                // lo dice `OOS2005` desde `governance`. Decir aqui «el mapeo no
                // la cubre» seria adelantar la consecuencia al error real, y
                // `99-errors` §2.1 es explicito: gana el codigo especifico.
                //
                // Es la segunda vez que esta figura aparece —`politica::check`
                // se adelantaba a `OOS2001` por lo mismo—, y las dos veces la
                // encontro un caso, no una lectura.
                let existe = e
                    .section("properties")
                    .map(|p| p.get(corta).is_some())
                    .unwrap_or(false);
                if existe && !mapeadas.contains(corta) {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2015,
                            &b.path,
                            format!(
                                "un ámbito de fila recorta `{target}` por `{corta}`, que el \
                                 mapeo no cubre"
                            ),
                        )
                        .help(
                            "el recorte por filas se empuja al origen como filtro, y sin la \
                             propiedad en el mapeo no hay con qué construirlo. El ejecutor no \
                             puede servir sin el recorte —serían filas que nadie autorizó—, así \
                             que rechazaría el plan: este binding compila y no se puede \
                             consultar",
                        ),
                    );
                }
            }
        }

        let faltan: Vec<&String> = exigidas.iter().filter(|k| !mapeadas.contains(*k)).collect();
        if !faltan.is_empty() {
            out.push(
                Diagnostic::new(
                    Code::Oos2011,
                    &b.path,
                    format!(
                        "el mapeo de `{target}` no cubre lo que necesita columna: falta{} {}",
                        if faltan.len() == 1 { "" } else { "n" },
                        faltan
                            .iter()
                            .map(|s| format!("`{s}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
                .at(b
                    .section("properties")
                    .map(|p| p.pos())
                    .unwrap_or(b.root.pos()))
                .help(
                    "un binding sin clave produce filas, no instancias: no hay índice de \
                     topología, ni recurso identificable, ni forma de volver a unirlo con \
                     los demás bindings de la misma entidad. Y una propiedad de `via` sin \
                     mapear deja la relacion sin columna fisica: el enlace se \
                     declara y no se puede recorrer. Y una replica sin columna no tiene de donde copiar",
                ),
            );
        }
    }
}

// ── OOS2012 ─────────────────────────────────────────────────────────────────

/// Heurística **determinista** —lo que exige el invariante III— y deliberadamente
/// **no completa**: no puede demostrar que un documento no contiene secretos.
/// Atrapa el descuido común, que es donde ocurren casi todas las filtraciones de
/// este tipo: pegar la cadena de conexión entera donde iba otra cosa.
fn parece_credencial(s: &str) -> bool {
    let Some(i) = s.find("://") else { return false };
    let resto = &s[i + 3..];
    let host = resto.split('/').next().unwrap_or(resto);
    match host.split_once('@') {
        Some((cred, _)) => cred.contains(':') && !cred.is_empty(),
        None => false,
    }
}

fn secrets(pkg: &Package, out: &mut Vec<Diagnostic>) {
    fn recorrer(n: &Node, path: &Path, ruta: &str, out: &mut Vec<Diagnostic>) {
        match n {
            Node::Scalar { raw, pos, .. } if parece_credencial(raw) => out.push(
                Diagnostic::new(
                    Code::Oos2012,
                    path,
                    format!("`{ruta}` contiene lo que parece una credencial"),
                )
                .at(*pos)
                .help(
                    "un repositorio ontológico está pensado para ser publicable. Declara el \
                     nombre de la variable de entorno con `connectionEnv`, nunca su valor",
                ),
            ),
            Node::Mapping { entries, .. } => {
                for (k, v) in entries {
                    let sub = match k.as_str() {
                        Some(name) if ruta.is_empty() => name.to_string(),
                        Some(name) => format!("{ruta}.{name}"),
                        None => ruta.to_string(),
                    };
                    recorrer(v, path, &sub, out);
                }
            }
            Node::Sequence { items, .. } => {
                for it in items {
                    recorrer(it, path, ruta, out);
                }
            }
            _ => {}
        }
    }
    for d in &pkg.docs {
        recorrer(&d.root, &d.path, "", out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_estricto() {
        assert!(es_semver("1.0.0"));
        assert!(es_semver("2.4.0-rc.1"));
        assert!(!es_semver("1.0")); // el caso de conformidad
        assert!(!es_semver("01.0.0")); // cero a la izquierda
        assert!(!es_semver("v1.0.0"));
    }

    #[test]
    fn handles_de_propiedad() {
        assert!(es_handle("team:people-platform"));
        assert!(es_handle("user:daustin"));
        assert!(!es_handle("People Platform")); // el caso de conformidad
        assert!(!es_handle("people-platform"));
        assert!(!es_handle("group:x"));
    }

    #[test]
    fn heuristica_de_credencial() {
        assert!(parece_credencial(
            "postgres://acme:hunter2@db.internal:5432/crm"
        ));
        // Sin credencial embebida no es un secreto, es una URL.
        assert!(!parece_credencial(
            "https://registry.oos.dev/regulatory/gdpr"
        ));
        assert!(!parece_credencial("public.tb_employee"));
        assert!(!parece_credencial("postgres://db.internal:5432/crm"));
    }

    #[test]
    fn detecta_ciclos() {
        let g = BTreeMap::from([
            ("a".to_string(), vec!["b".to_string()]),
            ("b".to_string(), vec!["a".to_string()]),
        ]);
        assert!(buscar_ciclo(&g).is_some());
        let g = BTreeMap::from([
            ("a".to_string(), vec!["b".to_string()]),
            ("b".to_string(), vec![]),
        ]);
        assert!(buscar_ciclo(&g).is_none());
    }
}
