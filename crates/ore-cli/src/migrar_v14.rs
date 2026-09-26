//! **`ore migrate v1alpha14`** — la vista es SQL (ADR 0040 paso 6; OOS
//! v1alpha14 `01-la-vista-es-sql` §7). Cada `View` estructurada pasa a ser su
//! consulta (`linaje::como_sql`, la traducción con la que el núcleo ya la sirve
//! desde el paso 3) con su contrato (`columns`, lo que el plan del motor tipa
//! sobre los tipos de la fuente). Mecánico; con `--seco` dice qué haría.
//!
//! | antes | después |
//! |---|---|
//! | `View v` con `from`/`fields`/`where`/`groupBy`/`having` | `View v` v1alpha14: `dialect: duckdb`, `sql`, `columns` |
//! | una `Table t` que se llama como una vista | `t_t` (su `object` no cambia), y quien la lee por `from`, la lee así: en v1alpha14 un nombre es una sola cosa (`OOS2035`) |
//! | un `Dataset d` que se llama como una vista (lo deja la v1alpha12 cuando una función nombra la vista que se copiaba) | `d_copia`, con su puntero en su sitio nuevo diciendo dónde siguen sus bytes |
//! | un `Dataset d` que copia entera una vista `v` que lee una tabla `t` | `d` lee `t` con la forma de `v` (la copian los drivers, como hoy), y `v` pasa a ser la consulta sobre `d` |
//! | `View` con `materialized`, `Table` con `datasource: lago` | antes, `ore migrate v1alpha12` (se encadena solo) |
//!
//! Va en dos tiempos: primero los nombres, sobre una copia del árbol, y luego
//! las consultas sobre esa copia —con los nombres chocando el plan no tipa: la
//! vista `ventas.clientes` sobre el dataset `ventas.clientes` es un ciclo—.
//!
//! Lo que no se migra solo se dice, y entonces **no se escribe nada**: un árbol
//! a medias no es de ninguna versión. Una vista v1alpha7 (lee el objeto por
//! dentro: su `Table` primero), una copia con forma encima de una vista
//! (decisión D: la copia de una vista es la vista entera), una cadena de vistas
//! hasta una tabla copiada, y una vista que el plan no tipa.
//!
//! Lo que cambia además de la forma, dicho: el digest de la consulta de cada
//! copia (la cabecera de la copia es la consulta servida), así que cada copia
//! por consulta se rehace entera **una vez**; los comentarios de los documentos
//! reescritos; y `freshness` de una vista que no se copiaba, que no significaba
//! nada.

use crate::migrar::{Cambio, POS, documento_en, emitir, esc};
use ore_core::diag::Pos;
use ore_core::document::Kind;
use ore_core::link::{Loaded, Package};
use ore_core::parse::{Node, Style};
use ore_core::vistas::{self, Fuente};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const VERSION: &str = "oos.dev/v1alpha14";

/// Las claves de la forma estructurada: lo que v1alpha14 ya no admite en una vista.
const FORMA: &[&str] = &["from", "fields", "where", "groupBy", "having"];

/// Lo que un `Dataset` mantenido lleva de la forma de la vista que copiaba.
const FORMA_DEL_DATASET: &[&str] = &["fields", "where", "groupBy", "having"];

/// El plan entero, sin tocar el árbol: los cambios, los avisos y cuántos
/// diagnósticos da antes. `Err` con lo que no se migra solo.
pub(crate) fn plan(raiz: &Path) -> crate::migrar::Plan {
    let (pkg, diags) = ore_core::validate::cargar_paquete(raiz);
    if !diags.is_empty() {
        return Err(format!(
            "el árbol no carga ({} diagnósticos de forma): migra sobre un árbol que compile. El primero: {}",
            diags.len(),
            diags[0].render(raiz)
        ));
    }
    let antes = ore_core::validate::validate_package(raiz).len();
    if !pkg
        .docs
        .iter()
        .any(|d| d.kind == Kind::View && !vistas::es_sql(d))
    {
        return Ok((Vec::new(), Vec::new(), antes));
    }
    let mut avisos: Vec<String> = Vec::new();
    let mut problemas: Vec<String> = Vec::new();

    // ── 1 · los nombres ──────────────────────────────────────────────────────
    let mut cambios = nombres(raiz, &pkg, &mut avisos, &mut problemas);
    if !problemas.is_empty() {
        return Err(no_se_migra(&problemas));
    }

    // ── 2 · las consultas, sobre el árbol ya con sus nombres ─────────────────
    let tmp;
    let (sitio, renombrado) = if cambios.is_empty() {
        (raiz, None)
    } else {
        tmp = temporal("nombres");
        copiar_arbol(raiz, &tmp)?;
        crate::migrar::aplicar(&tmp, &cambios)?;
        let (p, d) = ore_core::validate::cargar_paquete(&tmp);
        if !d.is_empty() {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!(
                "con los nombres nuevos el árbol no carga: {}",
                d[0].render(&tmp)
            ));
        }
        (tmp.as_path(), Some(p))
    };
    let pkg2 = renombrado.as_ref().unwrap_or(&pkg);
    let consultas = consultas(sitio, pkg2, &mut avisos, &mut problemas);
    if renombrado.is_some() {
        let _ = std::fs::remove_dir_all(sitio);
    }
    if !problemas.is_empty() {
        return Err(no_se_migra(&problemas));
    }
    // Lo de las consultas manda sobre lo de los nombres en el mismo fichero:
    // se escribió sobre él.
    for c in consultas {
        match cambios
            .iter_mut()
            .find(|x| x.fichero == c.fichero && x.a.is_none())
        {
            Some(x) => {
                x.texto = c.texto;
                x.que = c.que;
            }
            None => cambios.push(c),
        }
    }
    cotejo(raiz, &cambios)?;
    Ok((cambios, avisos, antes))
}

/// **El criterio de hecho**: el árbol migrado da exactamente los mismos
/// diagnósticos que antes, código a código. Uno que desaparece es un error que
/// la migración callaría —la forma de la vista estructurada tenía reglas
/// (`OOS2024`, `OOS2033`…) que sobre una consulta comprueba el motor, no el
/// compilador—; uno que aparece, uno que la migración haría. Se ensaya en una
/// copia; el árbol no se toca.
fn cotejo(raiz: &Path, cambios: &[Cambio]) -> Result<(), String> {
    let antes = diagnosticos(raiz);
    let tmp = temporal("cotejo");
    let despues = copiar_arbol(raiz, &tmp)
        .and_then(|()| crate::migrar::aplicar(&tmp, cambios))
        .map(|()| diagnosticos(&tmp));
    let _ = std::fs::remove_dir_all(&tmp);
    let problemas = comparar(&antes, &despues?);
    if problemas.is_empty() {
        Ok(())
    } else {
        Err(no_se_migra(&problemas))
    }
}

/// Los diagnósticos de un árbol, por código: los ficheros que los dan.
fn diagnosticos(raiz: &Path) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in ore_core::validate::validate_package(raiz) {
        let f = d
            .file
            .strip_prefix(raiz)
            .unwrap_or(&d.file)
            .display()
            .to_string();
        m.entry(d.code.as_str().to_string()).or_default().push(f);
    }
    m
}

fn temporal(para: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ore-migrate-v14-{para}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ))
}

/// Lo que cambia de `antes` a `despues`, código a código.
fn comparar(
    antes: &BTreeMap<String, Vec<String>>,
    despues: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut problemas: Vec<String> = Vec::new();
    for c in antes.keys().chain(despues.keys()).collect::<BTreeSet<_>>() {
        let (a, d) = (
            antes.get(c).map(Vec::len).unwrap_or(0),
            despues.get(c).map(Vec::len).unwrap_or(0),
        );
        if a > d {
            problemas.push(format!(
                "se callaría {c} ({a} antes, {d} después: {}): arréglalo antes de migrar",
                antes[c].join(", ")
            ));
        } else if d > a {
            problemas.push(format!(
                "daría {c} ({a} antes, {d} después: {})",
                despues[c].join(", ")
            ));
        }
    }
    problemas
}

fn no_se_migra(problemas: &[String]) -> String {
    format!(
        "no se migra solo, y no se escribe nada:\n{}",
        problemas
            .iter()
            .map(|p| format!("  · {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// **Un nombre es una sola cosa** (v1alpha14 §3, `OOS2035`). La vista se
/// queda el suyo, que es el que nombran las entidades, las funciones y las
/// acciones. La tabla, a la que sólo nombran quienes la leen, pasa a `<n>_t`
/// (el sufijo de los ejemplos de la spec); su `object` es el mismo y el origen
/// no se entera. El dataset pasa a `<n>_copia` y su puntero se muda con él,
/// diciendo dónde siguen sus bytes, que no se mueven. Las rutas de los cambios,
/// relativas a `raiz`.
fn nombres(
    raiz: &Path,
    pkg: &Package,
    avisos: &mut Vec<String>,
    problemas: &mut Vec<String>,
) -> Vec<Cambio> {
    let mut cambios: Vec<Cambio> = Vec::new();
    for v in pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::View && !vistas::es_sql(d))
    {
        let qn = v.qname().unwrap_or_default();
        if v.section("materialized").is_some() {
            problemas.push(format!(
                "`{qn}` lleva `materialized`: antes, `ore migrate v1alpha12`"
            ));
        }
        if matches!(vistas::fuente(v), Some(Fuente::Datasource { .. })) {
            problemas.push(format!(
                "`{qn}` es v1alpha7 (lee el objeto por dentro): declara su `Table` (v1alpha8) y léela con `from: {{ table }}`"
            ));
        }
    }
    let ocupados: BTreeSet<String> = pkg
        .docs
        .iter()
        .filter(|d| matches!(d.kind, Kind::View | Kind::Table | Kind::Dataset))
        .filter_map(|d| d.qname())
        .collect();
    let de_vista: BTreeSet<String> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::View)
        .filter_map(|d| d.qname())
        .collect();
    let mut ediciones: BTreeMap<PathBuf, Vec<(Pos, String)>> = BTreeMap::new();
    // qn de antes → (qn nuevo, nombre nuevo), por clase.
    let mut tablas: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut datasets: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut tomados = ocupados.clone();
    for d in pkg
        .docs
        .iter()
        .filter(|d| matches!(d.kind, Kind::Table | Kind::Dataset))
    {
        let Some(qn) = d.qname() else { continue };
        if !de_vista.contains(&qn) {
            continue;
        }
        let nombre = d.meta("name").and_then(|n| n.as_str()).unwrap_or_default();
        let base = qn[..qn.len() - nombre.len()].to_string();
        let sufijo = if d.kind == Kind::Table {
            "_t"
        } else {
            "_copia"
        };
        let nuevo = (1..)
            .map(|i| match i {
                1 => format!("{nombre}{sufijo}"),
                _ => format!("{nombre}{sufijo}{i}"),
            })
            .find(|n| !tomados.contains(&format!("{base}{n}")))
            .expect("un nombre libre");
        tomados.insert(format!("{base}{nuevo}"));
        if !editar(&mut ediciones, d, &["metadata", "name"], &nuevo) {
            problemas.push(format!("`{qn}`: no se encuentra su nombre en el fichero"));
            continue;
        }
        let que = if d.kind == Kind::Table {
            "la tabla"
        } else {
            "el dataset"
        };
        avisos.push(format!(
            "`{qn}` se llamaba como una vista: {que} pasa a `{base}{nuevo}`"
        ));
        let destino = if d.kind == Kind::Table {
            &mut tablas
        } else {
            &mut datasets
        };
        destino.insert(qn, (format!("{base}{nuevo}"), nuevo));
    }
    // Quien los lee por `from`, los lee por su nombre nuevo, escrito como
    // estaba (una parte, dos o tres).
    for d in pkg
        .docs
        .iter()
        .filter(|d| matches!(d.kind, Kind::View | Kind::Dataset))
    {
        let (clave, mapa) = match vistas::fuente(d) {
            Some(Fuente::Tabla(t)) => ("table", tablas.get(&t)),
            Some(Fuente::Dataset(x)) => ("dataset", datasets.get(&x)),
            _ => continue,
        };
        let Some((_, nuevo)) = mapa else { continue };
        let escrito = d
            .section("from")
            .and_then(|f| f.get(clave))
            .and_then(|(_, n)| n.as_str())
            .unwrap_or_default();
        let valor = match escrito.rsplit_once('.') {
            Some((antes, _)) => format!("{antes}.{nuevo}"),
            None => nuevo.clone(),
        };
        if !editar(&mut ediciones, d, &["spec", "from", clave], &valor) {
            problemas.push(format!(
                "`{}`: no se encuentra `from.{clave}` en el fichero",
                d.qname().unwrap_or_default()
            ));
        }
    }
    for (fichero, eds) in ediciones {
        let rel = fichero
            .strip_prefix(raiz)
            .map(Path::to_path_buf)
            .unwrap_or(fichero.clone());
        match aplicar_ediciones(&fichero, eds) {
            Some(texto) => cambios.push(Cambio {
                que: "renombra".into(),
                fichero: rel,
                texto: Some(texto),
                a: None,
            }),
            None => problemas.push(format!(
                "{}: el nombre no está donde el analizador dijo",
                rel.display()
            )),
        }
    }
    // El puntero del dataset renombrado se muda, y dice dónde están sus bytes.
    for (viejo, (nuevo, _)) in &datasets {
        let (Some(de), Some(a)) = (
            ore_core::punteros::ruta(viejo),
            ore_core::punteros::ruta(nuevo),
        ) else {
            continue;
        };
        let Some(nodo) = std::fs::read_to_string(raiz.join(&de))
            .ok()
            .and_then(|t| ore_core::parse::parse(&t).ok())
        else {
            continue;
        };
        let bytes = crate::materializar::dataset_de(Some(&nodo), viejo);
        let ore_core::json::Json::Obj(mut m) = ore_core::json::Json::de_node(&nodo) else {
            problemas.push(format!("{de}: el puntero de `{viejo}` no es un objeto"));
            continue;
        };
        m.insert("dataset".into(), ore_core::json::Json::s(bytes));
        for k in ["nombre", "tabla", "vista"] {
            if m.contains_key(k) {
                m.insert(k.into(), ore_core::json::Json::s(nuevo));
            }
        }
        cambios.push(Cambio {
            que: "puntero".into(),
            fichero: PathBuf::from(de),
            texto: Some(ore_core::json::Json::Obj(m).pretty() + "\n"),
            a: Some(PathBuf::from(a)),
        });
    }
    cambios
}

/// Las vistas estructuradas, cada una su consulta; y la copia de una vista que
/// lee una tabla, sobre la tabla. `pkg` ya no tiene nombres que choquen.
fn consultas(
    raiz: &Path,
    pkg: &Package,
    avisos: &mut Vec<String>,
    problemas: &mut Vec<String>,
) -> Vec<Cambio> {
    let mut cambios: Vec<Cambio> = Vec::new();
    let contratos = crate::vista::contratos(pkg);

    // La copia de una consulta se calcula en un puesto, que lee datasets del
    // lago y no orígenes (paso 4c). La que copiaba una vista sobre una tabla
    // la hacían los drivers: se queda así, con la forma de la vista en el
    // dataset (v1alpha14 la mantiene en el mantenido), y la vista pasa a ser
    // la consulta sobre su copia —que es lo que ya se servía—.
    let mut sobre_su_copia: BTreeMap<String, String> = BTreeMap::new();
    for d in pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Dataset && vistas::es_mantenido(d))
    {
        let qd = d.qname().unwrap_or_default();
        let Some(Fuente::Vista(x)) = vistas::fuente(d) else {
            continue;
        };
        let Some(v) = pkg.view(&x).filter(|v| !vistas::es_sql(v)) else {
            continue;
        };
        let propia = FORMA_DEL_DATASET.iter().find(|k| d.section(k).is_some());
        match (propia, lee_una_tabla(pkg, v, &mut Vec::new()), vistas::fuente(v)) {
            (Some(k), None, _) => problemas.push(format!(
                "`{qd}` copia `{x}` con `{k}` encima: la copia de una vista es la vista entera (decisión D); lleva `{k}` a la vista o escribe otra vista con ella, y copia ésa"
            )),
            // Sobre el lago y entera: la consulta se calcula en un puesto.
            (None, None, _) => {}
            // Con forma encima de una vista sobre una tabla: las dos formas,
            // compuestas, sobre la tabla; la vista sigue siendo la suya.
            (Some(k), Some(t), Some(Fuente::Tabla(_))) => match componer(d, v) {
                Some(forma) => {
                    cambios.push(sobre_la_tabla(raiz, d, &t, forma));
                    avisos.push(format!(
                        "`{qd}` copiaba `{x}` con `{k}` encima, y `{x}` lee `{t}`: ahora lee `{t}` con las dos formas compuestas"
                    ));
                }
                None => problemas.push(format!(
                    "`{qd}` copia `{x}` con `{k}` encima, y `{x}` lee una tabla con una forma que no se compone sola (agrupa, o un campo no es una columna): escribe el dataset sobre `{t}` a mano"
                )),
            },
            (Some(k), Some(t), _) => problemas.push(format!(
                "`{qd}` copia `{x}` con `{k}` encima, y `{x}` llega a la tabla `{t}` por otras vistas: escribe el dataset sobre `{t}` a mano"
            )),
            (None, Some(t), Some(Fuente::Tabla(_))) => {
                if let Some(otra) = sobre_su_copia.insert(x.clone(), qd.clone()) {
                    problemas.push(format!(
                        "`{x}` la copian `{otra}` y `{qd}`: deja una copia"
                    ));
                    continue;
                }
                let de_v = v.root.get("spec").map(|(_, s)| s);
                let forma = FORMA_DEL_DATASET
                    .iter()
                    .filter_map(|f| Some((f.to_string(), de_v?.get(f)?.1.clone())))
                    .collect();
                cambios.push(sobre_la_tabla(raiz, d, &t, forma));
                avisos.push(format!(
                    "`{qd}` copiaba `{x}`, que lee `{t}`: ahora lee `{t}` con la forma de `{x}`, y `{x}` es la consulta sobre `{qd}`"
                ));
            }
            (None, Some(t), _) => problemas.push(format!(
                "`{qd}` copia `{x}`, que llega a la tabla `{t}` por otras vistas: copia la de abajo, o escribe el dataset sobre `{t}` a mano"
            )),
        }
    }

    // Una vista sobre una tabla que un dataset copia la contesta hoy `ore ask`
    // desde esa copia (el emparejador compensa filtros y columnas). Una
    // consulta no pasa por él: sobre la tabla ya no se leería. No se decide
    // aquí cómo escribirla sobre la copia.
    for v in pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::View && !vistas::es_sql(d))
    {
        let qn = v.qname().unwrap_or_default();
        let Some(Fuente::Tabla(t)) = vistas::fuente(v) else {
            continue;
        };
        if sobre_su_copia.contains_key(&qn) {
            continue;
        }
        if let Some(d) = pkg.docs.iter().find(|d| {
            d.kind == Kind::Dataset
                && vistas::es_mantenido(d)
                && matches!(vistas::fuente(d), Some(Fuente::Tabla(x)) if x == t)
        }) {
            problemas.push(format!(
                "`{qn}` lee `{t}`, que copia `{}`: hoy se contesta desde esa copia, y como consulta se leería sólo sobre un dataset; escríbela sobre `{}`",
                d.qname().unwrap_or_default(),
                d.qname().unwrap_or_default()
            ));
        }
    }

    let mut sin_tipo: BTreeSet<String> = BTreeSet::new();
    for v in pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::View && !vistas::es_sql(d))
    {
        let qn = v.qname().unwrap_or_default();
        let columnas = match contratos.get(&qn) {
            Some(Ok(c)) => en_su_orden(v, c.clone()),
            Some(Err(e)) => {
                problemas.push(format!("`{qn}`: el plan no la tipa · {e}"));
                continue;
            }
            None => {
                problemas.push(format!("`{qn}`: sin plan"));
                continue;
            }
        };
        let sql = match sobre_su_copia.get(&qn) {
            Some(d) => format!(
                "SELECT {}\nFROM {}",
                columnas
                    .iter()
                    .map(|(c, _)| comillas(c))
                    .collect::<Vec<_>>()
                    .join(", "),
                d.split('.').map(comillas).collect::<Vec<_>>().join(".")
            ),
            None => match ore_core::linaje::como_sql(v) {
                Some(s) => legible(&s),
                None => {
                    problemas.push(format!("`{qn}` no se escribe como consulta"));
                    continue;
                }
            },
        };
        if let Some(Fuente::Tabla(t)) = vistas::fuente(v)
            && pkg
                .table(&t)
                .and_then(|t| t.section("columns"))
                .is_some_and(|c| c.entries().iter().any(|(_, x)| x.get("type").is_none()))
        {
            sin_tipo.insert(t);
        }
        let spec = v.root.get("spec").map(|(_, s)| s);
        let mut s: Vec<(String, Node)> = Vec::new();
        if let Some((_, o)) = spec.and_then(|s| s.get("owner")) {
            s.push(("owner".into(), o.clone()));
        }
        s.push(("dialect".into(), esc("duckdb")));
        s.push((
            "sql".into(),
            Node::Scalar {
                raw: format!("{sql}\n"),
                style: Style::Block,
                pos: POS,
            },
        ));
        s.push((
            "columns".into(),
            Node::Mapping {
                entries: columnas
                    .iter()
                    .map(|(c, t)| {
                        (
                            esc(c),
                            Node::Mapping {
                                entries: vec![(esc("type"), esc(t))],
                                pos: POS,
                            },
                        )
                    })
                    .collect(),
                pos: POS,
            },
        ));
        for k in ["moved", "reserved"] {
            if let Some((_, n)) = spec.and_then(|s| s.get(k)) {
                s.push((k.to_string(), n.clone()));
            }
        }
        for (k, _) in spec.map(|s| s.entries()).unwrap_or(&[]) {
            let Some(k) = k.as_str() else { continue };
            if !FORMA.contains(&k) && !["owner", "moved", "reserved"].contains(&k) {
                avisos.push(format!(
                    "`{qn}`: `{k}` no es de una vista v1alpha14 y se va"
                ));
            }
        }
        cambios.push(Cambio {
            que: "sql".into(),
            fichero: rel(raiz, &v.path),
            texto: Some(emitir(&documento_en(VERSION, "View", metadata_de(v), s))),
            a: None,
        });
    }
    for t in sin_tipo {
        avisos.push(format!(
            "`{t}` tiene columnas sin tipo: el contrato de las vistas que la leen las dice `String`, que es lo único que se afirma de ellas (`ore discover` las tipa)"
        ));
    }
    cambios
}

/// El dataset `d`, leyendo la tabla `t` con `forma` en vez de su `from` y la
/// forma que tuviera; lo demás de su `spec`, tal cual y en su orden.
fn sobre_la_tabla(raiz: &Path, d: &Loaded, t: &str, forma: Vec<(String, Node)>) -> Cambio {
    let mut forma = Some(forma);
    let mut ds: Vec<(String, Node)> = Vec::new();
    for (k, n) in d.root.get("spec").map(|(_, s)| s.entries()).unwrap_or(&[]) {
        let Some(k) = k.as_str() else { continue };
        if FORMA_DEL_DATASET.contains(&k) {
            continue;
        }
        if k != "from" {
            ds.push((k.to_string(), n.clone()));
            continue;
        }
        ds.push((
            "from".into(),
            Node::Mapping {
                entries: vec![(esc("table"), esc(t))],
                pos: POS,
            },
        ));
        ds.extend(forma.take().unwrap_or_default());
    }
    let version = d
        .root
        .get("apiVersion")
        .and_then(|(_, a)| a.as_str())
        .unwrap_or("oos.dev/v1alpha12");
    Cambio {
        que: "copia".into(),
        fichero: rel(raiz, &d.path),
        texto: Some(emitir(&documento_en(
            version,
            "Dataset",
            metadata_de(d),
            ds,
        ))),
        a: None,
    }
}

/// La forma de la copia `d` compuesta con la de la vista `v` que copiaba,
/// sobre la tabla de `v`: cada campo de `d` baja a su columna por los de `v`,
/// y los dos `where` se juntan. Sólo si ninguna agrupa, cada campo de `d` es
/// un campo de `v` y cada campo de `v` una columna; y si las dos filtran la
/// misma columna, no (juntarlas sería decidir su intersección).
fn componer(d: &Loaded, v: &Loaded) -> Option<Vec<(String, Node)>> {
    if [d, v]
        .iter()
        .any(|x| x.section("groupBy").is_some() || x.section("having").is_some())
    {
        return None;
    }
    let de_v = vistas::campos(v);
    let mut campos: Vec<(Node, Node)> = Vec::new();
    for (k, val) in d.section("fields")?.entries() {
        campos.push((k.clone(), esc(de_v.get(val.as_str()?)?)));
    }
    let mut filtros: Vec<(Node, Node)> = v
        .section("where")
        .map(|w| w.entries().to_vec())
        .unwrap_or_default();
    for (k, val) in d.section("where").map(|w| w.entries()).unwrap_or(&[]) {
        let col = de_v.get(k.as_str()?)?;
        if filtros
            .iter()
            .any(|(c, _)| c.as_str() == Some(col.as_str()))
        {
            return None;
        }
        filtros.push((esc(col), val.clone()));
    }
    let mut forma = vec![(
        "fields".to_string(),
        Node::Mapping {
            entries: campos,
            pos: POS,
        },
    )];
    if !filtros.is_empty() {
        forma.push((
            "where".to_string(),
            Node::Mapping {
                entries: filtros,
                pos: POS,
            },
        ));
    }
    Some(forma)
}

fn rel(raiz: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(raiz)
        .map(Path::to_path_buf)
        .unwrap_or(p.to_path_buf())
}

/// El contrato en el orden en que la vista proyecta (el de `fields`): el orden
/// no significa nada (§4), pero se lee como se escribió.
fn en_su_orden(v: &Loaded, mut columnas: Vec<(String, String)>) -> Vec<(String, String)> {
    let orden: Vec<String> = v
        .section("fields")
        .map(|f| {
            f.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    columnas.sort_by_key(|(c, _)| orden.iter().position(|o| o == c).unwrap_or(usize::MAX));
    columnas
}

/// La consulta de `como_sql`, con una columna por línea si el `SELECT` no cabe
/// en 80. Lo servido no cambia: se sirve la consulta analizada y reimpresa, no
/// su texto.
pub(crate) fn legible(sql: &str) -> String {
    let Some((primera, resto)) = sql.split_once('\n') else {
        return sql.to_string();
    };
    let Some(items) = primera.strip_prefix("SELECT ") else {
        return sql.to_string();
    };
    if primera.chars().count() <= 80 {
        return sql.to_string();
    }
    // Las comas de fuera de comillas y paréntesis separan columnas.
    let (mut partes, mut actual, mut comillas, mut hondo) =
        (Vec::new(), String::new(), false, 0i32);
    for c in items.chars() {
        match c {
            '"' => comillas = !comillas,
            '(' if !comillas => hondo += 1,
            ')' if !comillas => hondo -= 1,
            ',' if !comillas && hondo == 0 => {
                partes.push(actual.trim().to_string());
                actual.clear();
                continue;
            }
            _ => {}
        }
        actual.push(c);
    }
    partes.push(actual.trim().to_string());
    format!("SELECT\n  {}\n{resto}", partes.join(",\n  "))
}

/// ¿Llega `v` a una tabla por su cadena de vistas estructuradas? La tabla, o
/// `None` si llega al lago (un dataset). Una vista SQL de por medio cuenta
/// como lago: lo que lee lo comprueba el núcleo.
fn lee_una_tabla(pkg: &Package, v: &Loaded, pila: &mut Vec<String>) -> Option<String> {
    let qn = v.qname()?;
    if pila.contains(&qn) {
        return None;
    }
    pila.push(qn);
    match vistas::fuente(v)? {
        Fuente::Tabla(t) => Some(t),
        Fuente::Vista(x) => {
            let y = pkg.view(&x).filter(|y| !vistas::es_sql(y))?;
            lee_una_tabla(pkg, y, pila)
        }
        Fuente::Dataset(_) | Fuente::Datasource { .. } => None,
    }
}

/// `metadata` tal cual, en su orden.
fn metadata_de(d: &Loaded) -> Vec<(String, Node)> {
    d.root
        .get("metadata")
        .map(|(_, m)| {
            m.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.clone())))
                .collect()
        })
        .unwrap_or_default()
}

fn comillas(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Apunta que el escalar de `ruta` en el fichero de `d` pasa a ser `nuevo`.
/// `false` si no hay tal escalar, o es un bloque.
fn editar(
    ediciones: &mut BTreeMap<PathBuf, Vec<(Pos, String)>>,
    d: &Loaded,
    ruta: &[&str],
    nuevo: &str,
) -> bool {
    let mut n = &d.root;
    for k in ruta {
        match n.get(k) {
            Some((_, x)) => n = x,
            None => return false,
        }
    }
    if !matches!(
        n,
        Node::Scalar {
            style: Style::Plain | Style::Quoted,
            ..
        }
    ) {
        return false;
    }
    ediciones
        .entry(d.path.clone())
        .or_default()
        .push((n.pos(), nuevo.to_string()));
    true
}

/// El texto del fichero con cada escalar cambiado **en su sitio**: el resto
/// —comentarios, estilo, orden— no se toca. `None` si un escalar no está
/// donde el analizador dijo.
fn aplicar_ediciones(fichero: &Path, mut eds: Vec<(Pos, String)>) -> Option<String> {
    let texto = std::fs::read_to_string(fichero).ok()?;
    let doc = ore_core::parse::parse(&texto).ok()?;
    let mut lineas: Vec<String> = texto.split('\n').map(str::to_string).collect();
    // De la última a la primera: una edición no mueve a las de antes.
    eds.sort_by_key(|(p, _)| std::cmp::Reverse((p.line, p.col)));
    for (pos, nuevo) in eds {
        let (raw, style) = escalar_en(&doc, pos)?;
        let linea = lineas.get_mut(pos.line.checked_sub(1)?)?;
        let inicio: usize = linea.chars().take(pos.col - 1).map(char::len_utf8).sum();
        let resto = linea.get(inicio..)?;
        let (viejo, puesto) = match style {
            Style::Plain => (raw, nuevo),
            Style::Quoted => {
                let q = resto.chars().next()?;
                (format!("{q}{raw}{q}"), format!("{q}{nuevo}{q}"))
            }
            Style::Block => return None,
        };
        if !resto.starts_with(&viejo) {
            return None;
        }
        linea.replace_range(inicio..inicio + viejo.len(), &puesto);
    }
    Some(lineas.join("\n"))
}

/// El escalar que empieza en `pos`.
fn escalar_en(n: &Node, pos: Pos) -> Option<(String, Style)> {
    match n {
        Node::Scalar { raw, style, pos: p } if *p == pos => Some((raw.clone(), *style)),
        Node::Scalar { .. } => None,
        Node::Mapping { entries, .. } => entries
            .iter()
            .find_map(|(k, v)| escalar_en(k, pos).or_else(|| escalar_en(v, pos))),
        Node::Sequence { items, .. } => items.iter().find_map(|i| escalar_en(i, pos)),
    }
}

/// `ore migrate v1alpha14`: si al árbol le falta la v1alpha12, primero ésa
/// (con `--seco`, la v1alpha14 se planea sobre una copia del árbol a la que se
/// aplicó la v1alpha12, que es lo que habría); luego la vista es SQL.
pub fn migrar(path: &Path, op: &crate::migrar::Opciones) -> std::process::ExitCode {
    let falta_la_12 = crate::migrar::plan(path).is_ok_and(|(c, _, _)| !c.is_empty());
    if !falta_la_12 {
        return ejecutar(path, op);
    }
    // La cadena entera, ensayada en una copia y cotejada contra el árbol de
    // antes: la v1alpha12 sola acepta menos diagnósticos (una vista escrita que
    // pasa a dataset calla `OOS2024`), y la cadena no puede callar nada.
    let tmp = temporal("cadena");
    let ensayo = copiar_arbol(path, &tmp)
        .and_then(|()| {
            let (c, _, _) = crate::migrar::plan(&tmp)?;
            crate::migrar::aplicar(&tmp, &c)?;
            let (c, _, _) = plan(&tmp)?;
            crate::migrar::aplicar(&tmp, &c)
        })
        .map(|()| comparar(&diagnosticos(path), &diagnosticos(&tmp)));
    let _ = std::fs::remove_dir_all(&tmp);
    match ensayo {
        Ok(p) if p.is_empty() => {}
        Ok(p) => {
            eprintln!(
                "ore migrate · v1alpha12 y luego v1alpha14 · {}",
                no_se_migra(&p)
            );
            return std::process::ExitCode::from(65);
        }
        Err(e) => {
            eprintln!("ore migrate · v1alpha12 y luego v1alpha14 · {e}");
            return std::process::ExitCode::from(65);
        }
    }
    println!("── antes, v1alpha12 ──");
    let r = crate::migrar::migrar(path, op);
    println!();
    println!("── v1alpha14 ──");
    if !op.seco {
        if crate::migrar::plan(path).is_ok_and(|(c, _, _)| !c.is_empty()) {
            eprintln!("ore migrate · la v1alpha12 no quedó aplicada: no sigo");
            return r;
        }
        return ejecutar(path, op);
    }
    let tmp = temporal("seco");
    let hecho = copiar_arbol(path, &tmp).and_then(|()| {
        let (cambios, _, _) = crate::migrar::plan(&tmp)?;
        crate::migrar::aplicar(&tmp, &cambios)
    });
    let salida = match hecho {
        Ok(()) => {
            println!("(sobre el árbol como quedaría en v1alpha12)");
            ejecutar(&tmp, op)
        }
        Err(e) => {
            eprintln!("ore migrate · no se pudo ensayar la v1alpha12 en una copia: {e}");
            std::process::ExitCode::from(74)
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    salida
}

fn ejecutar(path: &Path, op: &crate::migrar::Opciones) -> std::process::ExitCode {
    crate::migrar::ejecutar(
        path,
        op,
        plan(path),
        "nada que migrar · ninguna `View` estructurada: el árbol ya está en v1alpha14",
        |cambios, antes| {
            let n = |q: &str| cambios.iter().filter(|c| c.que == q).count();
            format!(
                "{} vistas a SQL · {} documentos renombrados o reapuntados · {} copias que leen su tabla · {} punteros movidos · antes: {antes} diagnósticos",
                n("sql"),
                n("renombra"),
                n("copia"),
                n("puntero")
            )
        },
    )
}

/// El árbol, sin `.git` ni lo construido, en `a`.
fn copiar_arbol(de: &Path, a: &Path) -> Result<(), String> {
    std::fs::create_dir_all(a).map_err(|e| format!("{}: {e}", a.display()))?;
    for e in std::fs::read_dir(de).map_err(|e| format!("{}: {e}", de.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let nombre = e.file_name();
        if matches!(nombre.to_str(), Some(".git" | "target" | "node_modules")) {
            continue;
        }
        let (origen, destino) = (e.path(), a.join(&nombre));
        if origen.is_dir() {
            copiar_arbol(&origen, &destino)?;
        } else {
            std::fs::copy(&origen, &destino).map_err(|e| format!("{}: {e}", origen.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OP: crate::migrar::Opciones = crate::migrar::Opciones { seco: false };

    /// Un caso de la conformance, copiado donde se puede tocar.
    fn caso(rel: &str) -> PathBuf {
        let de = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        let a = temporal("prueba");
        copiar_arbol(&de, &a).unwrap();
        a
    }

    fn leer(raiz: &Path, rel: &str) -> String {
        std::fs::read_to_string(raiz.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    /// La copia de una vista que lee una tabla: el dataset lee la tabla con la
    /// forma de la vista (la copian los drivers), y la vista es la consulta
    /// sobre su copia, con el contrato tipado por el plan.
    #[test]
    fn la_copia_de_una_vista_sobre_una_tabla_lee_la_tabla() {
        let r = caso("vendor/oos/conformance/v1alpha12/valid/a-dataset-over-a-view/input");
        let (cambios, _, antes) = plan(&r).unwrap();
        assert_eq!(antes, 0);
        crate::migrar::aplicar(&r, &cambios).unwrap();
        let ds = leer(&r, "packages/hr/datasets/iberia_copia.yaml");
        assert!(ds.contains("table: hr.employees"), "{ds}");
        assert!(
            ds.contains("dni: national_id") && ds.contains("country:"),
            "{ds}"
        );
        assert!(ds.contains("freshness: 1h"), "{ds}");
        let v = leer(&r, "packages/hr/views/iberia.yaml");
        assert!(v.contains("apiVersion: oos.dev/v1alpha14"), "{v}");
        assert!(
            v.contains(
                "sql: |\n    SELECT \"id\", \"dni\", \"pais\"\n    FROM \"hr\".\"iberia_copia\"\n"
            ),
            "{v}"
        );
        assert!(v.contains("pais:\n      type: String"), "{v}");
        assert!(ore_core::validate::validate_package(&r).is_empty());
        let _ = std::fs::remove_dir_all(&r);
    }

    /// v1alpha10 → v1alpha12 → v1alpha14. La v1alpha12 deja la vista que una
    /// función nombra como la pregunta sobre su dataset, con el mismo nombre;
    /// y la tabla ya se llamaba así. La vista se queda su nombre; la tabla pasa
    /// a `_t` y el dataset a `_copia`, con su puntero mudado diciendo dónde
    /// siguen sus bytes. Y la función sigue viendo lo que la vista expone
    /// (`OOS7014` sobre el contrato de una vista SQL).
    #[test]
    fn un_nombre_es_una_sola_cosa() {
        let r = caso("vendor/oos/conformance/v1alpha10/valid/a-read-only-function/input");
        let puntero = r.join("datasets/ventas/default/clientes.json");
        std::fs::create_dir_all(puntero.parent().unwrap()).unwrap();
        std::fs::write(
            &puntero,
            "{\"metadata_location\": \"s3://b/ore/v2/datasets/ventas_clientes/metadata/1.json\"}\n",
        )
        .unwrap();
        assert!(migrar(&r, &OP) == std::process::ExitCode::SUCCESS);
        let t = leer(&r, "packages/ventas/tables/clientes.yaml");
        assert!(
            t.contains("metadata: { name: clientes_t, namespace: ventas }"),
            "{t}"
        );
        assert!(t.contains("object: 'public.clientes'"), "{t}");
        let ds = leer(&r, "packages/ventas/datasets/clientes.yaml");
        assert!(
            ds.contains("name: clientes_copia") && ds.contains("table: ventas.clientes_t"),
            "{ds}"
        );
        let v = leer(&r, "packages/ventas/views/clientes.yaml");
        assert!(v.contains("FROM \"ventas\".\"clientes_copia\""), "{v}");
        let p = leer(&r, "datasets/ventas/default/clientes_copia.json");
        assert!(
            p.contains("\"dataset\": \"datasets/ventas_clientes\""),
            "{p}"
        );
        assert!(!puntero.exists());
        let d = ore_core::validate::validate_package(&r);
        assert!(
            d.is_empty(),
            "{:?}",
            d.iter().map(|d| d.render(&r)).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&r);
    }

    /// Una copia con forma encima de una vista que lee una tabla: las dos
    /// formas, compuestas, sobre la tabla. La copia sigue sin llevar el campo
    /// clasificado, que es lo que el caso protege.
    #[test]
    fn la_copia_con_forma_encima_compone_las_dos() {
        let r = caso(
            "vendor/oos/conformance/v1alpha8/valid/a-copy-above-that-carries-nothing-classified/input",
        );
        assert!(migrar(&r, &OP) == std::process::ExitCode::SUCCESS);
        let ds = leer(&r, "datasets/copia.yaml");
        assert!(ds.contains("table: hr.employees"), "{ds}");
        assert!(
            ds.contains("employeeId: employee_id") && ds.contains("pais: country"),
            "{ds}"
        );
        assert!(!ds.contains("national"), "{ds}");
        assert!(ore_core::validate::validate_package(&r).is_empty());
        let _ = std::fs::remove_dir_all(&r);
    }

    /// Lo que no se migra solo no escribe nada: una vista v1alpha7, y una
    /// cadena que callaría un error (la vista escrita sin clave, `OOS2024`,
    /// que la v1alpha12 sola se llevaba al pasarla a dataset).
    #[test]
    fn lo_que_no_se_migra_solo_no_toca_el_arbol() {
        for rel in [
            "casos/con-vista",
            "vendor/oos/conformance/v1alpha8/invalid/written-view-without-a-key/input",
        ] {
            let r = caso(rel);
            let antes = diagnosticos(&r);
            let ficheros = |r: &Path| {
                let mut v: Vec<(PathBuf, String)> = Vec::new();
                fn andar(d: &Path, v: &mut Vec<(PathBuf, String)>) {
                    for e in std::fs::read_dir(d).unwrap().flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            andar(&p, v);
                        } else {
                            v.push((p.clone(), std::fs::read_to_string(&p).unwrap_or_default()));
                        }
                    }
                }
                andar(r, &mut v);
                v.sort();
                v
            };
            let de_antes = ficheros(&r);
            assert!(migrar(&r, &OP) != std::process::ExitCode::SUCCESS, "{rel}");
            assert_eq!(ficheros(&r), de_antes, "{rel}");
            assert_eq!(diagnosticos(&r), antes, "{rel}");
            let _ = std::fs::remove_dir_all(&r);
        }
    }
}
