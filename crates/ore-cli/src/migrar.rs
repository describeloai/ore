//! **`ore migrate v1alpha12`** — un árbol de antes pasa a después (ORE 0033, «Lo
//! construido»): lo que se tiene deja de ser dos disfraces y pasa a ser
//! `kind: Dataset`. Mecánico y sin opinión; con `--seco` dice qué haría y no
//! toca nada, que es la medida sobre un árbol real antes de aplicarlo.
//!
//! | antes | después |
//! |---|---|
//! | `View v` con `materialized` | `Dataset v` con el plan de `v` dentro (`from`, `fields`, `where`, `groupBy`, `having`, `freshness`); la `View` se va, salvo que una `Function`, una `Action` o un `TrainedModel` la nombren como vista: entonces queda como **la pregunta sobre el dataset** (`from: {dataset: v}`, campos identidad) |
//! | `Table t` con `datasource: lago` | `Dataset t` escrito: `columns` (sin `physicalType`) y `changes: {mode, key?}`; el `owner` es el del paquete; `datasource: lago` sale del manifiesto si nadie más lo usa |
//! | `from: {view: v}` / `from: {table: t}` a lo que pasó a ser dataset | `from: {dataset: …}`, y el documento sube a v1alpha12 |
//! | `copias/<p>_<v>.json` | `datasets/<p>_<v>.json` (el contenido no cambia) |
//!
//! Lo que se pierde, dicho: los **comentarios** de los documentos que se
//! reescriben (se emiten de nuevo desde el nodo), y `moved`/`reserved` de una
//! vista que se va (el nombre sobrevive en el dataset; la disciplina de
//! renombrado de campos no tiene sitio en él y se avisa).
//!
//! El criterio de hecho no es de aquí: es que el árbol migrado compile con los
//! mismos diagnósticos o menos, y que `GET /datasets` liste lo mismo.

use ore_core::document::Kind;
use ore_core::link::{Loaded, Package};
use ore_core::parse::{Node, Style};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct Opciones {
    /// Dice qué haría y no escribe nada.
    pub seco: bool,
}

/// Un cambio del plan: qué fichero, qué pasa y por qué.
pub(crate) struct Cambio {
    pub(crate) que: String,
    pub(crate) fichero: PathBuf,
    /// El texto nuevo, si el fichero se escribe; `None` si se borra o se mueve.
    pub(crate) texto: Option<String>,
    /// A dónde se mueve, si se mueve.
    pub(crate) a: Option<PathBuf>,
}

/// El plan de una versión: los cambios, los avisos y los diagnósticos de antes.
pub(crate) type Plan = Result<(Vec<Cambio>, Vec<String>, usize), String>;

pub fn migrar(path: &Path, op: &Opciones) -> std::process::ExitCode {
    ejecutar(
        path,
        op,
        plan(path),
        "nada que migrar · el árbol ya está en v1alpha12 (ninguna `View` con `materialized`, ninguna `Table` con `datasource: lago`, ningún puntero en `copias/` ni fuera de su sitio en `datasets/`)",
        |cambios, antes| {
            let (n_datasets, n_borrados, n_movidos, n_reescritos) = (
                cambios.iter().filter(|c| c.que == "dataset").count(),
                cambios.iter().filter(|c| c.que == "se va").count(),
                cambios.iter().filter(|c| c.que == "puntero").count(),
                cambios.iter().filter(|c| c.que == "reescrito").count(),
            );
            format!(
                "{n_datasets} datasets nuevos · {n_reescritos} documentos reescritos · {n_borrados} que se van · {n_movidos} punteros movidos · antes: {antes} diagnósticos"
            )
        },
    )
}

/// Enseña el plan y, sin `--seco`, lo aplica y dice qué da el árbol después.
pub(crate) fn ejecutar(
    path: &Path,
    op: &Opciones,
    plan: Plan,
    nada: &str,
    resumen: impl Fn(&[Cambio], usize) -> String,
) -> std::process::ExitCode {
    match plan {
        Err(e) => {
            eprintln!("ore migrate · {e}");
            std::process::ExitCode::from(65)
        }
        Ok((cambios, avisos, antes)) => {
            if cambios.is_empty() {
                println!("{nada}");
                return std::process::ExitCode::SUCCESS;
            }
            for c in &cambios {
                println!("  {:<10} {}", c.que, c.fichero.display());
                if let Some(a) = &c.a {
                    println!("             → {}", a.display());
                }
            }
            for a in &avisos {
                println!("  aviso      {a}");
            }
            println!();
            println!("{}", resumen(&cambios, antes));
            if op.seco {
                println!("(--seco: nada escrito)");
                return std::process::ExitCode::SUCCESS;
            }
            if let Err(e) = aplicar(path, &cambios) {
                eprintln!("ore migrate · {e}");
                return std::process::ExitCode::from(74);
            }
            let despues = ore_core::validate::validate_package(path);
            println!("después: {} diagnósticos", despues.len());
            for d in despues.iter().take(12) {
                println!("  {}", d.render(path));
            }
            std::process::ExitCode::SUCCESS
        }
    }
}

/// El plan entero, sin tocar el disco.
pub(crate) fn plan(raiz: &Path) -> Plan {
    let (pkg, diags) = ore_core::validate::cargar_paquete(raiz);
    if !diags.is_empty() {
        return Err(format!(
            "el árbol no carga ({} diagnósticos de forma): migra sobre un árbol que compile. El primero: {}",
            diags.len(),
            diags[0].render(raiz)
        ));
    }
    let antes = ore_core::validate::validate_package(raiz).len();
    let mut cambios: Vec<Cambio> = Vec::new();
    let mut avisos: Vec<String> = Vec::new();

    let copias: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::View && d.section("materialized").is_some())
        .collect();
    let tablas_lago: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| {
            d.kind == Kind::Table
                && d.section("datasource").and_then(|x| x.as_str()) == Some("lago")
        })
        .collect();

    // Los nombres que pasan a ser dataset: las copias y las tablas del lago.
    // Por separado, porque una tabla y una vista pueden llamarse igual: un
    // `from.table` que nombra la tabla de una vista copiada lee la tabla, no
    // la copia.
    let a_dataset = ADataset {
        vistas: copias.iter().filter_map(|d| d.qname()).collect(),
        tablas: tablas_lago.iter().filter_map(|d| d.qname()).collect(),
    };
    // Las vistas que alguien nombra COMO vista y no pueden irse: `over` y
    // `reads` de una Function, `over` de una Action, `trainedFrom`.
    let nombradas_como_vista: BTreeSet<String> = nombradas_como_vista(&pkg);

    let dueno_por_paquete = duenos(&pkg);

    // ── 1 · cada copia es un dataset con su plan ────────────────────────────
    for v in &copias {
        let qn = v.qname().unwrap_or_default();
        let (ns, nombre) = (meta(v, "namespace"), meta(v, "name"));
        let spec = v
            .root
            .get("spec")
            .map(|(_, s)| s)
            .ok_or("una vista sin spec")?;
        let mut ds_spec: Vec<(String, Node)> = Vec::new();
        for k in [
            "owner",
            "from",
            "fields",
            "where",
            "groupBy",
            "having",
            "freshness",
        ] {
            if let Some((_, n)) = spec.get(k) {
                ds_spec.push((k.to_string(), n.clone()));
            }
        }
        if spec.get("moved").is_some() || spec.get("reserved").is_some() {
            avisos.push(format!(
                "`{qn}`: `moved`/`reserved` no tienen sitio en un dataset y se pierden si la vista se va"
            ));
        }
        let mut meta_ds: Vec<(String, Node)> = vec![
            ("name".into(), esc(&nombre)),
            ("namespace".into(), esc(&ns)),
        ];
        if let Some(d) = v.meta("description") {
            meta_ds.push(("description".into(), d.clone()));
        }
        let dataset = documento("Dataset", meta_ds, ds_spec);
        let carpeta = carpeta_hermana(&v.path, "datasets");
        cambios.push(Cambio {
            que: "dataset".into(),
            fichero: carpeta.join(format!("{nombre}.yaml")),
            texto: Some(emitir(&dataset)),
            a: None,
        });
        if nombradas_como_vista.contains(&qn) {
            // La pregunta sobre el dataset: identidad sobre lo que expone.
            let expuestos: Vec<String> = ore_core::vistas::expone(v).into_keys().collect();
            let fields = Node::Mapping {
                entries: expuestos.iter().map(|c| (esc(c), esc(c))).collect(),
                pos: POS,
            };
            let mut vs: Vec<(String, Node)> = Vec::new();
            if let Some((_, o)) = spec.get("owner") {
                vs.push(("owner".into(), o.clone()));
            }
            vs.push((
                "from".into(),
                Node::Mapping {
                    entries: vec![(esc("dataset"), esc(&qn))],
                    pos: POS,
                },
            ));
            vs.push(("fields".into(), fields));
            for k in ["moved", "reserved"] {
                if let Some((_, n)) = spec.get(k) {
                    vs.push((k.to_string(), n.clone()));
                }
            }
            let mut mv: Vec<(String, Node)> = vec![
                ("name".into(), esc(&nombre)),
                ("namespace".into(), esc(&ns)),
            ];
            for k in ["labels", "description"] {
                if let Some(n) = v.meta(k) {
                    mv.push((k.to_string(), n.clone()));
                }
            }
            cambios.push(Cambio {
                que: "reescrito".into(),
                fichero: v.path.clone(),
                texto: Some(emitir(&documento("View", mv, vs))),
                a: None,
            });
            avisos.push(format!(
                "`{qn}`: una Function, Action o TrainedModel la nombra como vista, así que queda como la pregunta sobre su dataset"
            ));
        } else {
            cambios.push(Cambio {
                que: "se va".into(),
                fichero: v.path.clone(),
                texto: None,
                a: None,
            });
        }
    }

    // ── 2 · cada tabla del lago es un dataset escrito ───────────────────────
    for t in &tablas_lago {
        let (ns, nombre) = (meta(t, "namespace"), meta(t, "name"));
        let spec = t
            .root
            .get("spec")
            .map(|(_, s)| s)
            .ok_or("una tabla sin spec")?;
        let owner = dueno_por_paquete
            .get(&ns)
            .cloned()
            .unwrap_or_else(|| format!("team:{ns}"));
        // Las columnas, sin `physicalType`: el físico es Iceberg y no se cita.
        let columnas = spec
            .get("columns")
            .map(|(_, c)| Node::Mapping {
                entries: c
                    .entries()
                    .iter()
                    .map(|(k, v)| {
                        let entries: Vec<(Node, Node)> = v
                            .entries()
                            .iter()
                            .filter(|(kk, _)| kk.as_str() != Some("physicalType"))
                            .cloned()
                            .collect();
                        (k.clone(), Node::Mapping { entries, pos: POS })
                    })
                    .collect(),
                pos: POS,
            })
            .unwrap_or(Node::Mapping {
                entries: vec![],
                pos: POS,
            });
        let (modo, clave) = spec
            .get("changes")
            .map(|(_, c)| {
                (
                    c.get("mode")
                        .and_then(|(_, m)| m.as_str())
                        .unwrap_or("append")
                        .to_string(),
                    c.get("key").map(|(_, k)| k.clone()),
                )
            })
            .unwrap_or(("append".into(), None));
        // `retract` y `none` son de un origen que espeja; lo escrito admite
        // altas o fusión por clave. Con clave, fusión; sin ella, altas.
        let modo = match (modo.as_str(), &clave) {
            ("upsert", Some(_)) => "upsert",
            (_, Some(_)) => "upsert",
            _ => "append",
        };
        let mut ch: Vec<(Node, Node)> = vec![(esc("mode"), esc(modo))];
        if modo == "upsert"
            && let Some(k) = clave
        {
            ch.push((esc("key"), k));
        }
        let ds_spec = vec![
            ("owner".into(), esc(&owner)),
            ("columns".into(), columnas),
            (
                "changes".into(),
                Node::Mapping {
                    entries: ch,
                    pos: POS,
                },
            ),
        ];
        let mut meta_ds: Vec<(String, Node)> = vec![
            ("name".into(), esc(&nombre)),
            ("namespace".into(), esc(&ns)),
        ];
        if let Some(d) = t.meta("description") {
            meta_ds.push(("description".into(), d.clone()));
        }
        let carpeta = carpeta_hermana(&t.path, "datasets");
        cambios.push(Cambio {
            que: "dataset".into(),
            fichero: carpeta.join(format!("{nombre}.yaml")),
            texto: Some(emitir(&documento("Dataset", meta_ds, ds_spec))),
            a: None,
        });
        cambios.push(Cambio {
            que: "se va".into(),
            fichero: t.path.clone(),
            texto: None,
            a: None,
        });
    }

    // ── 3 · quien leía lo que pasó a ser dataset, lo lee como dataset ───────
    //
    // Sobre las vistas que quedan (no las copias, que ya se reescribieron) y
    // sobre los datasets nuevos, cuyo `from` puede nombrar a otra copia.
    let ya: BTreeSet<PathBuf> = cambios.iter().map(|c| c.fichero.clone()).collect();
    for v in pkg.docs.iter().filter(|d| d.kind == Kind::View) {
        if ya.contains(&v.path) {
            continue;
        }
        if let Some(nuevo) = reapuntar(v, &a_dataset) {
            cambios.push(Cambio {
                que: "reescrito".into(),
                fichero: v.path.clone(),
                texto: Some(nuevo),
                a: None,
            });
        }
    }
    for c in cambios.iter_mut().filter(|c| c.que == "dataset") {
        if let Some(t) = &c.texto
            && let Ok(n) = ore_core::parse::parse(t)
            && let Some(nuevo) = reapuntar_nodo(&n, &a_dataset)
        {
            c.texto = Some(nuevo);
        }
    }

    // ── 4 · el datasource `lago` sale del manifiesto si nadie lo usa ────────
    let usan_lago = pkg.docs.iter().any(|d| {
        (d.kind == Kind::Table
            && d.section("datasource").and_then(|x| x.as_str()) == Some("lago")
            && !tablas_lago.iter().any(|t| t.path == d.path))
            || (d.kind == Kind::View
                && d.section("from")
                    .and_then(|f| f.get("datasource"))
                    .and_then(|(_, x)| x.as_str())
                    == Some("lago"))
    });
    if !usan_lago {
        for c in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
            if let Some(texto) = sin_datasource_lago(raiz, c) {
                cambios.push(Cambio {
                    que: "reescrito".into(),
                    fichero: c.path.clone(),
                    texto: Some(texto),
                    a: None,
                });
            }
        }
    }

    // ── 5 · los punteros: `copias/` y los de antes de `datasets/` a su sitio ─
    //
    // 0038 P2: el puntero de `<p>.<n>` vive en `datasets/<p>/default/<n>.json`
    // (`ore_core::punteros`). Los de `copias/<p>_<v>.json` y los de antes en la
    // raíz de `datasets/` (`<p>_<n>.json`) se mueven ahí. El puntero de una
    // copia dice `vista`; el de `datasets/`, `tabla`; y sin `dataset` derivaba
    // el prefijo del bucket de su carpeta y su nombre de fichero: se escribe,
    // porque los bytes se quedan donde están (`copias/<p>_<v>`,
    // `datasets/<p>_<n>`) y el fichero ya no lo dice.
    for carpeta in ["copias", ore_core::punteros::CARPETA] {
        let dir = raiz.join(carpeta);
        let Ok(es) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut ficheros: Vec<PathBuf> = es
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
            .collect();
        ficheros.sort();
        for f in ficheros {
            let nombre = f.file_name().map(|n| n.to_os_string()).unwrap_or_default();
            let stem = f
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let Some(nodo) = std::fs::read_to_string(&f)
                .ok()
                .and_then(|t| ore_core::parse::parse(&t).ok())
            else {
                avisos.push(format!("{carpeta}/{stem}.json no se lee: se queda"));
                continue;
            };
            let Some(qn) = ore_core::punteros::clave_de(Path::new(&nombre), Some(&nodo)) else {
                avisos.push(format!("{carpeta}/{stem}.json no dice de qué es: se queda"));
                continue;
            };
            let Some(a) = ore_core::punteros::ruta(&qn) else {
                avisos.push(format!(
                    "{carpeta}/{stem}.json es de `{qn}`, que no es un nombre: se queda"
                ));
                continue;
            };
            if carpeta != "copias" && raiz.join(&a).is_file() {
                avisos.push(format!(
                    "{carpeta}/{stem}.json y {a} son de `{qn}`: manda el de su sitio, y el de antes se va"
                ));
                cambios.push(Cambio {
                    que: "se va".into(),
                    fichero: PathBuf::from(carpeta).join(&nombre),
                    texto: None,
                    a: None,
                });
                continue;
            }
            let ore_core::json::Json::Obj(mut m) = ore_core::json::Json::de_node(&nodo) else {
                continue;
            };
            if let Some(v) = m.get("vista").cloned()
                && !m.contains_key("tabla")
            {
                m.insert("tabla".into(), v);
            }
            if !m.contains_key("dataset") {
                m.insert(
                    "dataset".into(),
                    ore_core::json::Json::s(format!("{carpeta}/{stem}")),
                );
            }
            cambios.push(Cambio {
                que: "puntero".into(),
                fichero: PathBuf::from(carpeta).join(&nombre),
                texto: Some(ore_core::json::Json::Obj(m).pretty() + "\n"),
                a: Some(PathBuf::from(a)),
            });
        }
    }

    // Las rutas, relativas a la raíz: el cargador las da absolutas, y el
    // informe (y `git mv`) las quieren como el árbol las ve.
    for c in cambios.iter_mut() {
        if let Ok(r) = c.fichero.strip_prefix(raiz) {
            c.fichero = r.to_path_buf();
        }
    }
    Ok((cambios, avisos, antes))
}

pub(crate) fn aplicar(raiz: &Path, cambios: &[Cambio]) -> Result<(), String> {
    let en_git = raiz.join(".git").exists();
    for c in cambios {
        let abs = raiz.join(&c.fichero);
        match (&c.texto, &c.a) {
            // Movido y reescrito: se quita el viejo y se escribe el nuevo.
            (Some(t), Some(a)) => {
                let destino = raiz.join(a);
                if let Some(p) = destino.parent() {
                    std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
                }
                let quitado = en_git
                    && std::process::Command::new("git")
                        .args(["rm", "-q", "-f"])
                        .arg(&c.fichero)
                        .current_dir(raiz)
                        .status()
                        .is_ok_and(|s| s.success());
                if !quitado && abs.exists() {
                    std::fs::remove_file(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
                }
                std::fs::write(&destino, t).map_err(|e| format!("{}: {e}", destino.display()))?;
                if en_git {
                    let _ = std::process::Command::new("git")
                        .args(["add"])
                        .arg(a)
                        .current_dir(raiz)
                        .status();
                }
            }
            (Some(t), None) => {
                if let Some(p) = abs.parent() {
                    std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
                }
                std::fs::write(&abs, t).map_err(|e| format!("{}: {e}", abs.display()))?;
            }
            (None, Some(a)) => {
                let destino = raiz.join(a);
                if let Some(p) = destino.parent() {
                    std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
                }
                if en_git {
                    let s = std::process::Command::new("git")
                        .args(["mv", "-f"])
                        .arg(&c.fichero)
                        .arg(a)
                        .current_dir(raiz)
                        .status()
                        .map_err(|e| format!("git mv: {e}"))?;
                    if !s.success() {
                        std::fs::rename(&abs, &destino)
                            .map_err(|e| format!("{}: {e}", abs.display()))?;
                    }
                } else {
                    std::fs::rename(&abs, &destino)
                        .map_err(|e| format!("{}: {e}", abs.display()))?;
                }
            }
            (None, None) => {
                let borrado = en_git
                    && std::process::Command::new("git")
                        .args(["rm", "-q", "-f"])
                        .arg(&c.fichero)
                        .current_dir(raiz)
                        .status()
                        .is_ok_and(|s| s.success());
                if !borrado && abs.exists() {
                    std::fs::remove_file(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
                }
            }
        }
    }
    Ok(())
}

// ── lo que se mira del paquete ─────────────────────────────────────────────

fn nombradas_como_vista(pkg: &Package) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for d in &pkg.docs {
        let ns = d.meta("namespace").and_then(|n| n.as_str());
        let mut nombra = |n: &Node| {
            if let Some(s) = n.as_str() {
                out.insert(ore_core::normalize::qualify(s, ns));
            }
        };
        match d.kind {
            Kind::Function | Kind::Action => {
                if let Some(o) = d.section("over") {
                    nombra(o);
                }
                for r in d.section("reads").map(|r| r.items()).unwrap_or(&[]) {
                    nombra(r);
                }
            }
            Kind::TrainedModel => {
                for r in d.section("trainedFrom").map(|r| r.items()).unwrap_or(&[]) {
                    nombra(r);
                }
            }
            _ => {}
        }
    }
    out
}

/// El `owner` de cada paquete por su nombre (que es el `namespace` de sus
/// documentos): lo que hereda un dataset escrito, que no tenía dueño.
fn duenos(pkg: &Package) -> BTreeMap<String, String> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|p| {
            Some((
                p.meta("name")?.as_str()?.to_string(),
                p.section("owner")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn meta(d: &Loaded, k: &str) -> String {
    d.meta(k).and_then(|n| n.as_str()).unwrap_or("").to_string()
}

/// `views/x.yaml` → `datasets/`; un documento fuera de una carpeta con nombre
/// de kind gana la carpeta al lado.
fn carpeta_hermana(path: &Path, nombre: &str) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    let es_de_kind = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| matches!(n, "views" | "tables" | "entities" | "functions" | "models"));
    if es_de_kind {
        dir.parent().unwrap_or(Path::new("")).join(nombre)
    } else {
        dir.join(nombre)
    }
}

/// `from: {view: x}` o `from: {table: x}` donde `x` pasó a ser dataset →
/// `from: {dataset: x}`, y el documento sube a v1alpha12. `None` si no toca.
/// Lo que pasa a ser dataset, por lo que era.
struct ADataset {
    vistas: BTreeSet<String>,
    tablas: BTreeSet<String>,
}

fn reapuntar(v: &Loaded, a_dataset: &ADataset) -> Option<String> {
    reapuntar_nodo(&v.root, a_dataset)
}

fn reapuntar_nodo(root: &Node, a_dataset: &ADataset) -> Option<String> {
    let ns = root
        .get("metadata")
        .and_then(|(_, m)| m.get("namespace"))
        .and_then(|(_, n)| n.as_str());
    let (_, spec) = root.get("spec")?;
    let (_, from) = spec.get("from")?;
    let (clave, ref_) = from
        .get("view")
        .map(|(_, n)| ("view", n))
        .or_else(|| from.get("table").map(|(_, n)| ("table", n)))?;
    let qn = ore_core::normalize::qualify(ref_.as_str()?, ns);
    let de = if clave == "view" {
        &a_dataset.vistas
    } else {
        &a_dataset.tablas
    };
    if !de.contains(&qn) {
        return None;
    }
    let mut nuevo = root.clone();
    // apiVersion → v1alpha12
    if let Node::Mapping { entries, .. } = &mut nuevo {
        for (k, v) in entries.iter_mut() {
            if k.as_str() == Some("apiVersion") {
                *v = esc("oos.dev/v1alpha12");
            }
            if k.as_str() == Some("spec")
                && let Node::Mapping { entries: se, .. } = v
            {
                for (sk, sv) in se.iter_mut() {
                    if sk.as_str() == Some("from") {
                        *sv = Node::Mapping {
                            entries: vec![(esc("dataset"), esc(&qn))],
                            pos: POS,
                        };
                    }
                }
            }
        }
    }
    Some(emitir(&nuevo))
}

/// Quita `- { name: lago, … }` de `datasources` del manifiesto, por líneas: el
/// manifiesto lleva comentarios que un documento reemitido perdería.
fn sin_datasource_lago(raiz: &Path, c: &Loaded) -> Option<String> {
    let (_, ds) = c.root.get("datasources")?;
    let items = ds.items();
    let i = items
        .iter()
        .position(|it| it.get("name").and_then(|(_, n)| n.as_str()) == Some("lago"))?;
    let texto = std::fs::read_to_string(raiz.join(&c.path)).ok()?;
    let lineas: Vec<&str> = texto.lines().collect();
    let desde = items[i].pos().line; // 1-based
    let hasta = items
        .get(i + 1)
        .map(|n| n.pos().line)
        .or_else(|| {
            // La siguiente clave de la raíz después de `datasources`.
            c.root
                .entries()
                .iter()
                .map(|(k, _)| k.pos().line)
                .filter(|l| *l > desde)
                .min()
        })
        .unwrap_or(lineas.len() + 1);
    // La línea del ítem empieza en el `- `; si el ítem es en bloque, `pos` es
    // la del primer campo y el `- ` está en la misma línea.
    // Y los comentarios pegados encima del item: hablaban de el.
    let mut desde = desde;
    while desde >= 2 && lineas[desde - 2].trim_start().starts_with('#') {
        desde -= 1;
    }
    let mut out: Vec<&str> = Vec::new();
    for (n, l) in lineas.iter().enumerate() {
        let ln = n + 1;
        if ln >= desde && ln < hasta {
            continue;
        }
        out.push(l);
    }
    let mut s = out.join("\n");
    if texto.ends_with('\n') {
        s.push('\n');
    }
    Some(s)
}

// ── emitir YAML desde un nodo ──────────────────────────────────────────────

pub(crate) const POS: ore_core::diag::Pos = ore_core::diag::Pos { line: 0, col: 0 };

pub(crate) fn esc(s: &str) -> Node {
    Node::Scalar {
        raw: s.to_string(),
        style: Style::Plain,
        pos: POS,
    }
}

fn documento(kind: &str, metadata: Vec<(String, Node)>, spec: Vec<(String, Node)>) -> Node {
    documento_en("oos.dev/v1alpha12", kind, metadata, spec)
}

/// Un documento de `version` con su `metadata` y su `spec`, en ese orden.
pub(crate) fn documento_en(
    version: &str,
    kind: &str,
    metadata: Vec<(String, Node)>,
    spec: Vec<(String, Node)>,
) -> Node {
    let m = |v: Vec<(String, Node)>| Node::Mapping {
        entries: v.into_iter().map(|(k, n)| (esc(&k), n)).collect(),
        pos: POS,
    };
    Node::Mapping {
        entries: vec![
            (esc("apiVersion"), esc(version)),
            (esc("kind"), esc(kind)),
            (esc("metadata"), m(metadata)),
            (esc("spec"), m(spec)),
        ],
        pos: POS,
    }
}

/// YAML en bloque, con las secuencias de escalares en flujo. Lo justo para un
/// documento OOS: mapas, secuencias y escalares.
pub(crate) fn emitir(n: &Node) -> String {
    let mut out = String::new();
    emitir_en(n, 0, &mut out);
    out
}

fn escalar(raw: &str, style: Style) -> String {
    let necesita = raw.is_empty()
        || style == Style::Quoted
        || raw.contains(": ")
        || raw.contains(" #")
        || raw.ends_with(':')
        || raw.starts_with([
            '[', '{', '"', '\'', '*', '&', '!', '|', '>', '%', '@', '`', '-', '?', ':', ',',
        ])
        || raw.contains('\n')
        || matches!(raw, "true" | "false" | "null" | "~" | "yes" | "no")
        || raw.parse::<f64>().is_ok();
    if necesita {
        format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        raw.to_string()
    }
}

/// Un texto de una sola línea (y su salto final), partido en líneas de hasta
/// 80 por los espacios: plegado (`>`), YAML las vuelve a juntar con un espacio
/// y el valor es el mismo. `None` si tiene más líneas, o espacios que plegar
/// cambiaría (dobles, al principio o al final).
fn plegado(raw: &str) -> Option<Vec<String>> {
    let una = raw.strip_suffix('\n')?;
    if una.contains('\n')
        || una.contains("  ")
        || una.starts_with(' ')
        || una.ends_with(' ')
        || una.contains('\t')
    {
        return None;
    }
    let mut lineas: Vec<String> = Vec::new();
    for palabra in una.split(' ') {
        match lineas.last_mut() {
            Some(l) if l.chars().count() + 1 + palabra.chars().count() <= 80 => {
                l.push(' ');
                l.push_str(palabra);
            }
            _ => lineas.push(palabra.to_string()),
        }
    }
    Some(lineas)
}

fn es_escalar(n: &Node) -> bool {
    matches!(n, Node::Scalar { .. })
}

fn emitir_en(n: &Node, sangria: usize, out: &mut String) {
    let pad = " ".repeat(sangria);
    match n {
        Node::Mapping { entries, .. } => {
            for (k, v) in entries {
                let clave = k.as_str().unwrap_or("");
                match v {
                    // Un texto de varias líneas —la consulta de una vista— en
                    // bloque literal, como lo escribiría una persona; uno de
                    // una línea que venía plegado (`>`), plegado otra vez.
                    Node::Scalar {
                        raw,
                        style: Style::Block,
                        ..
                    } if raw.ends_with('\n') && !raw.trim().is_empty() => match plegado(raw) {
                        Some(lineas) => {
                            out.push_str(&format!("{pad}{clave}: >\n"));
                            for l in lineas {
                                out.push_str(&format!("{pad}  {l}\n"));
                            }
                        }
                        None => {
                            out.push_str(&format!("{pad}{clave}: |\n"));
                            for l in raw.lines() {
                                if l.is_empty() {
                                    out.push('\n');
                                } else {
                                    out.push_str(&format!("{pad}  {l}\n"));
                                }
                            }
                        }
                    },
                    Node::Scalar { raw, style, .. } => {
                        out.push_str(&format!("{pad}{clave}: {}\n", escalar(raw, *style)));
                    }
                    Node::Sequence { items, .. } if items.iter().all(es_escalar) => {
                        let xs: Vec<String> = items
                            .iter()
                            .map(|i| match i {
                                Node::Scalar { raw, style, .. } => escalar(raw, *style),
                                _ => unreachable!(),
                            })
                            .collect();
                        out.push_str(&format!("{pad}{clave}: [{}]\n", xs.join(", ")));
                    }
                    Node::Mapping { entries: e, .. } if e.is_empty() => {
                        out.push_str(&format!("{pad}{clave}: {{}}\n"));
                    }
                    _ => {
                        out.push_str(&format!("{pad}{clave}:\n"));
                        emitir_en(v, sangria + 2, out);
                    }
                }
            }
        }
        Node::Sequence { items, .. } => {
            for i in items {
                match i {
                    Node::Scalar { raw, style, .. } => {
                        out.push_str(&format!("{pad}- {}\n", escalar(raw, *style)));
                    }
                    Node::Mapping { .. } => {
                        // `- clave: valor` con el resto sangrado dos más.
                        let mut cuerpo = String::new();
                        emitir_en(i, sangria + 2, &mut cuerpo);
                        let mut primera = true;
                        for l in cuerpo.lines() {
                            if primera {
                                out.push_str(&format!("{pad}- {}\n", l.trim_start()));
                                primera = false;
                            } else {
                                out.push_str(l);
                                out.push('\n');
                            }
                        }
                    }
                    Node::Sequence { .. } => {
                        out.push_str(&format!("{pad}-\n"));
                        emitir_en(i, sangria + 2, out);
                    }
                }
            }
        }
        Node::Scalar { raw, style, .. } => {
            out.push_str(&format!("{pad}{}\n", escalar(raw, *style)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arbol(ficheros: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ore-migrate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (rel, t) in ficheros {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        dir
    }

    const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n# la fuente\ndatasources:\n  - { name: erp, type: postgres, connectionEnv: ERP_URL }\n  - { name: lago, type: lago, connectionEnv: LAGO_URL }\n";
    const PAQUETE: &str = "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: hr, version: 1.0.0, status: active, domain: people }\nspec: { owner: team:data }\n";
    const CONDUCTO: &str = "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: hr }\nspec:\n  owner: team:security\n  conduits:\n    materialization.payload:\n      gdpr.sensitivity: high\n";
    const LATTICE: &str = "apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n";
    const TABLA: &str = "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: employees, namespace: hr }\nspec:\n  datasource: erp\n  object: \"public.employees\"\n  columns:\n    employee_id: { physicalType: \"varchar(16)\", type: String }\n    country: { physicalType: \"char(2)\", type: String }\n  reads: { predicatePushdown: [eq], fullScan: cheap }\n  changes: { mode: retract, witness: log }\n";
    const COPIA: &str = "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  from: { table: hr.employees }\n  freshness: 15m\n  fields:\n    id: employee_id\n    pais: country\n  where:\n    country: [ES, PT]\n  materialized: { datasource: lago, table: \"cache.hr_empleados\" }\n";
    const ENCIMA: &str = "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: iberia, namespace: hr }\nspec:\n  owner: team:hr\n  from: { view: empleados }\n  fields:\n    id: id\n";
    const ENTIDAD: &str = "apiVersion: oos.dev/v1alpha8\nkind: Entity\nmetadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  primaryKey: [id]\n  backedBy: empleados\n  properties:\n    id: { type: String }\n    pais: { type: String }\n";
    const LAGO: &str = "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: resumen, namespace: hr }\n# la escribió write()\nspec:\n  datasource: lago\n  object: \"hr_resumen\"\n  columns:\n    pais: { type: String }\n    n: { type: Integer }\n  reads: { fullScan: cheap }\n  changes: { mode: upsert, key: [pais], witness: snapshot }\n";

    /// 0038 P2: los punteros de antes de `datasets/` van a su sitio, con el
    /// nombre de sus bytes escrito; `a_b.c` y `a.b_c` ya no chocan; y si el de
    /// su sitio ya está, manda ese y el de antes se va.
    #[test]
    fn los_punteros_de_antes_van_a_su_sitio() {
        let paquete = |n: &str| {
            format!(
                "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {n}, version: 1.0.0, status: active, domain: d }}\nspec: {{ owner: team:data }}\n"
            )
        };
        let (pa, pab) = (paquete("a"), paquete("a_b"));
        let dir = arbol(&[
            ("packages/a/package.yaml", &pa),
            ("packages/a_b/package.yaml", &pab),
            (
                "datasets/a_b_c.json",
                r#"{"tabla":"a_b.c","metadata_location":"m1"}"#,
            ),
            (
                "datasets/a_x.json",
                r#"{"estado":"copiada","dataset":"copias/a_x"}"#,
            ),
            ("datasets/a_y.json", r#"{"tabla":"a.y"}"#),
            (
                "datasets/a/default/y.json",
                r#"{"tabla":"a.y","dataset":"catalogo/a/default/y"}"#,
            ),
        ]);
        let (cambios, _, _) = plan(&dir).unwrap();
        aplicar(&dir, &cambios).unwrap();
        let lee = |r: &str| {
            let t = std::fs::read_to_string(dir.join(r)).unwrap_or_else(|_| panic!("{r} no está"));
            let n = ore_core::parse::parse(&t).unwrap();
            n.get("dataset")
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        };
        assert_eq!(
            lee("datasets/a_b/default/c.json").as_deref(),
            Some("datasets/a_b_c")
        );
        assert_eq!(
            lee("datasets/a/default/x.json").as_deref(),
            Some("copias/a_x")
        );
        assert_eq!(
            lee("datasets/a/default/y.json").as_deref(),
            Some("catalogo/a/default/y")
        );
        for viejo in ["a_b_c", "a_x", "a_y"] {
            assert!(
                !dir.join(format!("datasets/{viejo}.json")).exists(),
                "{viejo}"
            );
        }
        let (otra, _, _) = plan(&dir).unwrap();
        assert!(otra.is_empty(), "migrar dos veces no hace nada");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn la_copia_es_un_dataset_y_la_tabla_del_lago_otro() {
        let dir = arbol(&[
            ("ontology.config.yaml", CONFIG),
            ("packages/hr/package.yaml", PAQUETE),
            ("conduits.yaml", CONDUCTO),
            ("lattices/s.yaml", LATTICE),
            ("packages/hr/tables/employees.yaml", TABLA),
            ("packages/hr/tables/resumen.yaml", LAGO),
            ("packages/hr/views/empleados.yaml", COPIA),
            ("packages/hr/views/iberia.yaml", ENCIMA),
            ("packages/hr/entities/Employee.yaml", ENTIDAD),
            (
                "copias/hr_empleados.json",
                "{\"snapshot\": 1, \"vista\": \"hr.empleados\"}",
            ),
        ]);
        let antes = ore_core::validate::validate_package(&dir);
        assert!(antes.is_empty(), "{antes:?}");

        let (cambios, _avisos, _) = plan(&dir).unwrap();
        let que: Vec<String> = cambios
            .iter()
            .map(|c| {
                format!(
                    "{} {}",
                    c.que,
                    c.fichero.display().to_string().replace('\\', "/")
                )
            })
            .collect();
        assert!(
            que.contains(&"dataset packages/hr/datasets/empleados.yaml".to_string()),
            "{que:?}"
        );
        assert!(
            que.contains(&"se va packages/hr/views/empleados.yaml".to_string()),
            "{que:?}"
        );
        assert!(
            que.contains(&"dataset packages/hr/datasets/resumen.yaml".to_string()),
            "{que:?}"
        );
        assert!(
            que.contains(&"se va packages/hr/tables/resumen.yaml".to_string()),
            "{que:?}"
        );
        assert!(
            que.contains(&"reescrito packages/hr/views/iberia.yaml".to_string()),
            "{que:?}"
        );
        assert!(
            que.contains(&"reescrito ontology.config.yaml".to_string()),
            "{que:?}"
        );
        assert!(
            que.contains(&"puntero copias/hr_empleados.json".to_string()),
            "{que:?}"
        );

        aplicar(&dir, &cambios).unwrap();
        let ds = std::fs::read_to_string(dir.join("packages/hr/datasets/empleados.yaml")).unwrap();
        assert!(ds.contains("kind: Dataset"), "{ds}");
        assert!(ds.contains("from:\n    table: hr.employees"), "{ds}");
        assert!(ds.contains("freshness: 15m"), "{ds}");
        assert!(ds.contains("where:\n    country: [ES, PT]"), "{ds}");
        assert!(!ds.contains("materialized"), "{ds}");
        let esc = std::fs::read_to_string(dir.join("packages/hr/datasets/resumen.yaml")).unwrap();
        assert!(esc.contains("owner: team:data"), "{esc}");
        assert!(
            esc.contains("changes:\n    mode: upsert\n    key: [pais]"),
            "{esc}"
        );
        assert!(
            !esc.contains("physicalType") && !esc.contains("datasource"),
            "{esc}"
        );
        let ib = std::fs::read_to_string(dir.join("packages/hr/views/iberia.yaml")).unwrap();
        assert!(ib.contains("apiVersion: oos.dev/v1alpha12"), "{ib}");
        assert!(ib.contains("from:\n    dataset: hr.empleados"), "{ib}");
        let cfg = std::fs::read_to_string(dir.join("ontology.config.yaml")).unwrap();
        assert!(
            !cfg.contains("lago") && cfg.contains("# la fuente"),
            "{cfg}"
        );
        // 0038 P2: el puntero va a su sitio y dice dónde siguen los bytes.
        assert!(dir.join("datasets/hr/default/empleados.json").exists());
        assert!(!dir.join("copias/hr_empleados.json").exists());
        assert!(!dir.join("datasets/hr_empleados.json").exists());
        let pj = ore_core::parse::parse(
            &std::fs::read_to_string(dir.join("datasets/hr/default/empleados.json")).unwrap(),
        )
        .unwrap();
        let campo = |k: &str| pj.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        assert_eq!(campo("tabla").as_deref(), Some("hr.empleados"));
        assert_eq!(campo("dataset").as_deref(), Some("copias/hr_empleados"));

        let despues = ore_core::validate::validate_package(&dir);
        assert!(despues.is_empty(), "{despues:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn una_vista_que_una_funcion_nombra_queda_como_la_pregunta_sobre_su_dataset() {
        let funcion = "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: contar, namespace: hr }\nspec:\n  runtime: wasm\n  entrypoint: contar.wasm\n  over: empleados\n  input: {}\n  output: { type: Integer }\n";
        let dir = arbol(&[
            ("ontology.config.yaml", CONFIG),
            ("packages/hr/package.yaml", PAQUETE),
            ("conduits.yaml", CONDUCTO),
            ("lattices/s.yaml", LATTICE),
            ("packages/hr/tables/employees.yaml", TABLA),
            ("packages/hr/views/empleados.yaml", COPIA),
            ("packages/hr/functions/contar.yaml", funcion),
        ]);
        let (cambios, avisos, _) = plan(&dir).unwrap();
        assert!(
            avisos.iter().any(|a| a.contains("queda como la pregunta")),
            "{avisos:?}"
        );
        let v = cambios
            .iter()
            .find(|c| c.que == "reescrito" && c.fichero.ends_with("empleados.yaml"))
            .expect("la vista se reescribe");
        let t = v.texto.as_ref().unwrap();
        assert!(t.contains("from:\n    dataset: hr.empleados"), "{t}");
        assert!(t.contains("fields:\n    id: id\n    pais: pais"), "{t}");
        assert!(!t.contains("where") && !t.contains("freshness"), "{t}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn el_emisor_entrecomilla_lo_que_hace_falta() {
        let n = ore_core::parse::parse(
            "a: \"x: y\"\nb: 15m\nc: [ES, PT]\nd:\n  - { k: v }\ne: \"true\"\n",
        )
        .unwrap();
        let t = emitir(&n);
        assert_eq!(
            t,
            "a: \"x: y\"\nb: 15m\nc: [ES, PT]\nd:\n  - k: v\ne: \"true\"\n"
        );
    }
}
