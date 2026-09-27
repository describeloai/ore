//! **`ore migrate punteros`** — el puntero pasa a la fuente (ADR 0045 P4).
//!
//! Un árbol de antes de P3′ tiene la `Table` de cada objeto **en cada base**
//! que lo lee (`<objeto>_t`), escrita con las respuestas de esa base. Después
//! vive una vez, en `packages/<fuente>/`, y las bases la nombran.
//!
//! Por fuente con paquete y catálogo, y con al menos una base:
//!
//! | antes | después |
//! |---|---|
//! | `clave/*` y `tipo/*` en `discover.answers.json` de cada base | en el de la fuente; en un choque manda la fuente, y se dice |
//! | `Table <base>.<schema>.<objeto>_t` con `datasource: <fuente>` | fuera; la escribe la fuente (`ore source induce`) desde su catálogo |
//! | `from: { table: … }` y la consulta de una `View` SQL que la leían | reapuntados a `<fuente>.<schema>.<objeto>` |
//!
//! ⛔ **Las bases se reapuntan, no se re-inducen**: re-inducir una base vieja
//!   reescribiría sus vistas y renombraría lo que las nombra (una base de antes
//!   de 0038 P5 pasaría a la carpeta de su schema).
//!
//! Se ensaya en una copia, y **no se escribe nada** si el árbol da un
//! diagnóstico que antes no daba, si una Table llevaba una etiqueta que la de la
//! fuente no lleva, o si un objeto no está en el catálogo de la fuente. Lo que
//! cambia sin romper —la cara `D` que P1′ corrigió, un diagnóstico de una Table
//! que ya no se cuenta dos veces— se dice.

use crate::fuente_inducida;
use crate::migrar::Opciones;
use ore_core::document::Kind;
use ore_core::link::Loaded;
use ore_core::parse::Node;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Informe {
    fuentes: Vec<String>,
    bases: usize,
    movidas: usize,
    escritas: usize,
    reapuntados: usize,
    respuestas: usize,
    avisos: Vec<String>,
}

pub fn migrar(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let Some(raiz) = crate::raiz_del_repositorio(path) else {
        eprintln!(
            "ore migrate punteros · `{}` no está en un repositorio",
            path.display()
        );
        return std::process::ExitCode::from(66);
    };
    let tmp = std::env::temp_dir().join(format!(
        "ore-migrate-punteros-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let ensayo = crate::migrar_v14::copiar_arbol(&raiz, &tmp).and_then(|()| aplicar(&tmp));
    let cotejo = ensayo.map(|i| (i, cotejar(&diagnosticos(&raiz), &diagnosticos(&tmp))));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut informe = match cotejo {
        Err(e) => {
            eprintln!("ore migrate punteros · no se migra, y no se escribe nada:\n  · {e}");
            return std::process::ExitCode::from(65);
        }
        Ok((_, (problemas, _))) if !problemas.is_empty() => {
            eprintln!("ore migrate punteros · no se migra, y no se escribe nada:");
            for p in problemas {
                eprintln!("  · {p}");
            }
            return std::process::ExitCode::from(65);
        }
        Ok((i, (_, avisos))) => {
            let mut i = i;
            i.avisos.extend(avisos);
            i
        }
    };
    if !op.seco {
        match aplicar(&raiz) {
            Ok(i) => {
                let avisos = std::mem::take(&mut informe.avisos);
                informe = i;
                informe.avisos = avisos;
            }
            Err(e) => {
                // El ensayo pasó: esto es el disco, no la migración.
                eprintln!("ore migrate punteros · el ensayo pasó y el árbol no: {e}");
                return std::process::ExitCode::from(74);
            }
        }
    }
    if informe.fuentes.is_empty() {
        println!(
            "ore migrate punteros · nada que mover: ninguna base tiene punteros de una fuente con paquete"
        );
        return std::process::ExitCode::SUCCESS;
    }
    println!(
        "ore migrate punteros{} · {} · {} base(s) · {} puntero(s) fuera de las bases · {} escrito(s) en la fuente · {} documento(s) reapuntado(s) · {} respuesta(s) del objeto a la fuente · mismos diagnósticos",
        if op.seco {
            " (en seco: no se ha escrito nada)"
        } else {
            ""
        },
        informe.fuentes.join(", "),
        informe.bases,
        informe.movidas,
        informe.escritas,
        informe.reapuntados,
        informe.respuestas,
    );
    for a in &informe.avisos {
        println!("  · {a}");
    }
    std::process::ExitCode::SUCCESS
}

/// Los diagnósticos del árbol, por código, con el fichero de cada uno.
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

/// Uno más es un problema; uno menos, un aviso: la Table que dos bases tenían
/// dos veces y la fuente tiene una da su diagnóstico una vez.
fn cotejar(
    antes: &BTreeMap<String, Vec<String>>,
    despues: &BTreeMap<String, Vec<String>>,
) -> (Vec<String>, Vec<String>) {
    let (mut problemas, mut avisos) = (Vec::new(), Vec::new());
    for c in antes.keys().chain(despues.keys()).collect::<BTreeSet<_>>() {
        let (a, d) = (
            antes.get(c).map(Vec::len).unwrap_or(0),
            despues.get(c).map(Vec::len).unwrap_or(0),
        );
        if d > a {
            problemas.push(format!(
                "daría {c} ({a} antes, {d} después: {})",
                despues[c].join(", ")
            ));
        } else if a > d {
            avisos.push(format!(
                "{c}: {a} antes, {d} después (lo daba un puntero repetido o una base que ya no lo tiene)"
            ));
        }
    }
    (problemas, avisos)
}

fn texto(n: Option<(&Node, &Node)>) -> Option<String> {
    n.and_then(|(_, v)| v.as_str().map(String::from))
}

/// Un nodo en una línea, con las claves ordenadas: para comparar lo que dicen
/// dos documentos, no cómo lo escribieron.
fn plano(n: &Node) -> String {
    match n {
        Node::Scalar { raw, .. } => raw.clone(),
        Node::Mapping { entries, .. } => {
            let mut e: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{}: {}", plano(k), plano(v)))
                .collect();
            e.sort();
            format!("{{{}}}", e.join(", "))
        }
        Node::Sequence { items, .. } => {
            format!(
                "[{}]",
                items.iter().map(plano).collect::<Vec<_>>().join(", ")
            )
        }
    }
}

/// Las etiquetas de una Table: las suyas y las de cada columna. Es lo que un
/// humano pudo añadir a un puntero inducido, y lo que heredan quienes lo leen.
fn etiquetas(raiz: &Node) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some((_, m)) = raiz.get("metadata").and_then(|(_, m)| m.get("labels")) {
        for (k, v) in m.entries() {
            out.insert(format!("metadata.labels.{} = {}", plano(k), plano(v)));
        }
    }
    let Some((_, spec)) = raiz.get("spec") else {
        return out;
    };
    if let Some((_, l)) = spec.get("labels") {
        for (k, v) in l.entries() {
            out.insert(format!("spec.labels.{} = {}", plano(k), plano(v)));
        }
    }
    if let Some((_, cols)) = spec.get("columns") {
        for (c, v) in cols.entries() {
            if let Some((_, l)) = v.get("labels") {
                for (k, x) in l.entries() {
                    out.insert(format!("{}.labels.{} = {}", plano(c), plano(k), plano(x)));
                }
            }
        }
    }
    out
}

/// La migración, sobre `raiz`. Sin comprobar diagnósticos: eso lo hace quien
/// la ensaya.
fn aplicar(raiz: &Path) -> Result<Informe, String> {
    let (pkg, diags) = ore_core::validate::cargar_paquete(raiz);
    if !diags.is_empty() {
        return Err(format!(
            "el árbol no carga ({} diagnósticos de forma). El primero: {}",
            diags.len(),
            diags[0].render(raiz)
        ));
    }
    let paquetes = raiz.join("packages");
    let mut nombres: Vec<String> = std::fs::read_dir(&paquetes)
        .map_err(|e| format!("{}: {e}", paquetes.display()))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    nombres.sort();
    let mut informe = Informe::default();
    let fallo = |f: crate::revision::Fallo| f.mensaje;

    for fuente in &nombres {
        let Some(dir_f) = fuente_inducida::dir(raiz, fuente) else {
            continue;
        };
        let Some(refs) = fuente_inducida::referencias(raiz, fuente) else {
            continue;
        };
        let bases: Vec<PathBuf> = nombres
            .iter()
            .map(|n| paquetes.join(n))
            .filter(|d| {
                crate::alcance::del_paquete(d)
                    .ok()
                    .flatten()
                    .is_some_and(|a| a.fuente() == fuente)
            })
            .collect();

        // ── los punteros de las bases ──────────────────────────────────────
        let de_una_base = |d: &Loaded| bases.iter().any(|b| d.path.starts_with(b));
        let mut mapa: BTreeMap<String, String> = BTreeMap::new();
        let mut viejas: Vec<(String, String, Node)> = Vec::new();
        let mut borrar: Vec<PathBuf> = Vec::new();
        for t in pkg
            .docs
            .iter()
            .filter(|d| d.kind == Kind::Table && de_una_base(d))
        {
            let spec = t.root.get("spec").map(|(_, s)| s);
            if spec.and_then(|s| texto(s.get("datasource"))).as_deref() != Some(fuente.as_str()) {
                continue;
            }
            let objeto = spec
                .and_then(|s| texto(s.get("object")))
                .unwrap_or_default();
            let Some(nuevo) = refs.get(&objeto) else {
                return Err(format!(
                    "`{}` lee `{objeto}`, que no está en el catálogo de `{fuente}`: sin él la fuente no tiene de dónde escribir su puntero",
                    t.path.strip_prefix(raiz).unwrap_or(&t.path).display()
                ));
            };
            let qn = t.qname().unwrap_or_default();
            mapa.insert(qn, nuevo.clone());
            viejas.push((objeto, nuevo.clone(), t.root.clone()));
            borrar.push(t.path.clone());
        }
        if borrar.is_empty() {
            continue;
        }
        informe.fuentes.push(fuente.clone());
        informe.bases += bases.len();

        // ── lo que es del objeto, a la fuente ──────────────────────────────
        let mut de_f = crate::revision::acumuladas(&dir_f).map_err(fallo)?;
        for b in &bases {
            let (del_objeto, resto) = crate::revision::acumuladas(b).map_err(fallo)?.partir();
            if del_objeto.is_empty() {
                continue;
            }
            for id in del_objeto.discrepa_de(&de_f) {
                informe.avisos.push(format!(
                    "`{id}`: `{}` la tenía contestada distinto de la fuente; vale la de la fuente",
                    b.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            informe.respuestas += del_objeto.len();
            let mut juntas = del_objeto;
            juntas.fundir(de_f);
            de_f = juntas;
            std::fs::write(b.join("discover.answers.json"), resto.json().pretty())
                .map_err(|e| format!("{}: {e}", b.display()))?;
        }
        if !de_f.is_empty() {
            std::fs::write(dir_f.join("discover.answers.json"), de_f.json().pretty())
                .map_err(|e| format!("{}: {e}", dir_f.display()))?;
        }

        // ── quien los leía, reapuntado ─────────────────────────────────────
        for d in pkg.docs.iter().filter(|d| !borrar.contains(&d.path)) {
            let campos: Vec<_> = ore_core::exporta::referencias(d)
                .into_iter()
                .filter(|r| r.kind == Kind::Table && mapa.contains_key(&r.destino))
                .collect();
            let mut sitios: Vec<_> = ore_core::servir::nombrados(&pkg, d)
                .into_iter()
                .filter(|n| {
                    n.doc.kind == Kind::Table
                        && n.doc.qname().is_some_and(|q| mapa.contains_key(&q))
                })
                .collect();
            if campos.is_empty() && sitios.is_empty() {
                continue;
            }
            let t = std::fs::read_to_string(&d.path)
                .map_err(|e| format!("{}: {e}", d.path.display()))?;
            let mut lineas: Vec<String> = t.lines().map(String::from).collect();
            // De atrás adelante, por si dos caen en la misma línea.
            let mut campos = campos;
            campos.sort_by_key(|r| std::cmp::Reverse((r.pos.line, r.pos.col)));
            for r in &campos {
                let (i, l) = crate::paquete::sustituir_en(&lineas, r.pos, &mapa[&r.destino])
                    .ok_or_else(|| {
                        format!(
                            "no se pudo reapuntar `{}` en `{}`",
                            r.clase,
                            d.path.display()
                        )
                    })?;
                lineas[i] = l;
            }
            let mut nuevo = lineas.join("\n");
            if t.ends_with('\n') {
                nuevo.push('\n');
            }
            if !sitios.is_empty() {
                let mut sql = d
                    .section("sql")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                sitios.sort_by_key(|n| std::cmp::Reverse(n.rango.start));
                for n in sitios {
                    let a = &mapa[&n.doc.qname().unwrap_or_default()];
                    let escrito = if n.citado {
                        a.split('.')
                            .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
                            .collect::<Vec<_>>()
                            .join(".")
                    } else {
                        a.clone()
                    };
                    sql.replace_range(n.rango, &escrito);
                }
                nuevo = crate::paquete::con_sql(&nuevo, &sql).ok_or_else(|| {
                    format!("no se pudo reapuntar la consulta de `{}`", d.path.display())
                })?;
            }
            std::fs::write(&d.path, nuevo).map_err(|e| format!("{}: {e}", d.path.display()))?;
            informe.reapuntados += 1;
        }

        // ── fuera de las bases, y la fuente los escribe ────────────────────
        for p in &borrar {
            std::fs::remove_file(p).map_err(|e| format!("{}: {e}", p.display()))?;
            if let Some(dir) = p.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        informe.movidas += borrar.len();
        let i = fuente_inducida::inducir(raiz, fuente)
            .map_err(fallo)?
            .ok_or_else(|| format!("`{fuente}` dejó de ser una fuente"))?;
        informe.escritas += i.escritas;

        // ── lo que un humano pudo añadir a un puntero no se pierde ─────────
        let (pkg2, _) = ore_core::validate::cargar_paquete(raiz);
        let mut caras: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut tipos: Vec<String> = Vec::new();
        let mut con_tipos: BTreeSet<&str> = BTreeSet::new();
        for (objeto, nuevo, vieja) in &viejas {
            let Some(t) = pkg2.table(nuevo) else {
                return Err(format!(
                    "la fuente no escribió el puntero de `{objeto}` ({nuevo})"
                ));
            };
            let perdidas: Vec<String> = etiquetas(vieja)
                .difference(&etiquetas(&t.root))
                .cloned()
                .collect();
            if !perdidas.is_empty() {
                return Err(format!(
                    "el puntero de `{objeto}` llevaba etiquetas que el de la fuente no lleva: {}",
                    perdidas.join("; ")
                ));
            }
            // Los tipos: un puntero de antes de 0032 §3 no llevaba el del
            // conector, y la vista que lo lee servía `String`. Ahora lo lleva:
            // cambia el esquema servido y el plan de su copia, que se rehace.
            let tipos_de = |n: &Node| -> BTreeMap<String, String> {
                n.get("spec")
                    .and_then(|(_, s)| s.get("columns"))
                    .map(|(_, c)| {
                        c.entries()
                            .iter()
                            .map(|(k, v)| {
                                (plano(k), texto(v.get("type")).unwrap_or_else(|| "—".into()))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let (ta, td) = (tipos_de(vieja), tipos_de(&t.root));
            for (col, a) in &ta {
                let d = td.get(col).map(String::as_str).unwrap_or("(no está)");
                // Sin tipo se servía `String`: eso no cambia nada.
                let servido = if a == "—" { "String" } else { a.as_str() };
                if servido != d {
                    con_tipos.insert(objeto.as_str());
                    tipos.push(format!("{objeto}.{col}: {a} → {d}"));
                }
            }
            // La cara `D`, resumida: el modo, y si lleva clave.
            let cara = |n: &Node| {
                let c = n.get("spec").and_then(|(_, s)| s.get("changes"));
                let modo = c
                    .and_then(|(_, c)| texto(c.get("mode")))
                    .unwrap_or_else(|| "?".into());
                let clave = c.is_some_and(|(_, c)| c.get("key").is_some());
                format!("{modo}{}", if clave { " + key" } else { "" })
            };
            let (a, d) = (cara(vieja), cara(&t.root));
            if a != d {
                caras
                    .entry(format!("`changes` {a} → {d}"))
                    .or_default()
                    .insert(objeto.clone());
            }
        }
        if !tipos.is_empty() {
            tipos.sort();
            tipos.dedup();
            informe.avisos.push(format!(
                "tipos: {} columna(s) de {} puntero(s) de `{fuente}` ganan el tipo del conector ({}{}): cambia el esquema que sirven sus vistas, y la copia que las lee se rehace entera una vez",
                tipos.len(),
                con_tipos.len(),
                tipos.iter().take(3).cloned().collect::<Vec<_>>().join("; "),
                if tipos.len() > 3 { "; …" } else { "" }
            ));
        }
        for (cambio, objetos) in caras {
            let muestra: Vec<&str> = objetos.iter().take(3).map(String::as_str).collect();
            informe.avisos.push(format!(
                "{cambio} en {} puntero(s) de `{fuente}` ({}{}): la cara del origen y la clave si se sabe (P1′)",
                objetos.len(),
                muestra.join(", "),
                if objetos.len() > 3 { ", …" } else { "" }
            ));
        }
    }
    Ok(informe)
}
