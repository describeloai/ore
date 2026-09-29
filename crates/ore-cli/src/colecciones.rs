//! `ore collections` — **las colecciones del árbol, como activos** (0046
//! E8·1d): lo que `ore datasets` es para un dataset.
//!
//! | | qué | quién lo llama |
//! |---|---|---|
//! | `ore collections .` | cada `MediaCollection`: su forma (virtual, mantenida, escrita), su origen, su medio y sus formatos, y el estado de su puntero (transacción, ítems actuales, retirados y perdidos) | `GET /colecciones` |
//! | `--ficha b.s.n` | lo mismo de una, y **la historia de sus transacciones** (los snapshots de su manifiesto, `ore-store historia`) | `GET /colecciones/{b}/{s}/{n}` |
//! | `--items b.s.n [--estado …] [--desde N] [--limite N]` | sus ítems, por estado (`actual` por defecto; `retirado`, `perdido`, `todos`), en orden de camino, paginados | `GET /colecciones/{b}/{s}/{n}/items` |
//! | `--recoger [--seco] [--gracia 2h]` | **el mantenimiento** (E8·3): la retención de cada colección (lo retirado más viejo que su `retention` sale del manifiesto, y el puntero se mueve) y después la **recogida de blobs** del inquilino: lo que ninguna fila de ningún manifiesto nombra, y nadie tocó en la gracia, se va. Si un manifiesto no se puede leer, no se recoge nada | el CronJob de mantenimiento |
//! | `--cotejar b.s.n [--muestra N]` | **que lo que el manifiesto de una mantenida dice esté en el lago** (E8·2d): cada blob que una fila nombra, con su tamaño; y N de ellos, bajados y vueltos a hashear. Sale con 1 si algo está roto, y dice qué ítem | el mantenimiento, o quien quiera saberlo |
//!
//! El puntero vive con los de los datasets (`datasets/<b>/<s>/<n>.json`, con
//! `kind: MediaCollection`): comparten el espacio de nombres del schema. El
//! manifiesto es una tabla del lago, y se lee por `ore-store` (`ore` no abre
//! un socket).

use crate::lector;
use crate::materializar::programa_del_almacen;
use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::link::Loaded;
use ore_core::parse::Node;
use std::path::{Path, PathBuf};

pub struct Opciones<'a> {
    pub json: bool,
    pub ficha: Option<&'a str>,
    pub items: Option<&'a str>,
    pub estado: Option<&'a str>,
    pub desde: usize,
    pub limite: usize,
    pub informe: Option<&'a Path>,
    pub cotejar: Option<&'a str>,
    pub muestra: usize,
    pub recoger: bool,
    pub seco: bool,
    pub gracia: Option<&'a str>,
}

type Fallo = (u8, String);

pub fn colecciones(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let hecho = if op.recoger {
        recoger(path, op)
    } else if let Some(n) = op.cotejar {
        cotejar(path, n, op)
    } else if let Some(n) = op.items {
        items(path, n, op)
    } else if let Some(n) = op.ficha {
        ficha(path, n, op)
    } else {
        listar(path, op)
    };
    match hecho {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err((c, m)) => {
            eprintln!("error: {m}");
            std::process::ExitCode::from(c)
        }
    }
}

fn dir_punteros(path: &Path, op: &Opciones) -> PathBuf {
    op.informe
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.join("datasets"))
}

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// La forma de una colección: `virtual`, `mantenida` (copia los bytes) o
/// `escrita` (sin `from`: la llena el código).
fn forma(d: &Loaded) -> &'static str {
    match (d.section("from").is_some(), crate::coleccion::es_virtual(d)) {
        (true, true) => "virtual",
        (true, false) => "mantenida",
        (false, _) => "escrita",
    }
}

/// Lo que se dice de una colección: el documento y el resumen de su puntero.
fn resumen(d: &Loaded, puntero: Option<&Node>) -> Json {
    let mut m = std::collections::BTreeMap::new();
    let qn = d.qname().unwrap_or_default();
    m.insert("nombre".into(), Json::s(&qn));
    m.insert("forma".into(), Json::s(forma(d)));
    for k in ["media", "owner"] {
        if let Some(v) = d.section(k).and_then(|v| v.as_str()) {
            m.insert(k.into(), Json::s(v));
        }
    }
    if let Some(f) = d.section("formats") {
        m.insert(
            "formats".into(),
            Json::Arr(
                f.items()
                    .iter()
                    .filter_map(|x| x.as_str().map(Json::s))
                    .collect(),
            ),
        );
    }
    if let Some(o) = d
        .section("from")
        .and_then(|f| f.get("objectTable"))
        .and_then(|(_, v)| v.as_str())
    {
        m.insert("objectTable".into(), Json::s(o));
    }
    if let Some(r) = d.section("retention").and_then(|v| v.as_str()) {
        m.insert("retention".into(), Json::s(r));
    }
    let p = match puntero {
        None => Json::Crudo("null".into()),
        Some(p) => {
            let mut q = std::collections::BTreeMap::new();
            if let Json::Obj(todo) = Json::de_node(p) {
                for k in [
                    "estado",
                    "motivo",
                    "transaccion",
                    "items",
                    "cambios",
                    "metadata_location",
                    "snapshot",
                    "testigo",
                ] {
                    if let Some(v) = todo.get(k) {
                        q.insert(k.to_string(), v.clone());
                    }
                }
            }
            Json::Obj(q)
        }
    };
    m.insert("puntero".into(), p);
    Json::Obj(m)
}

/// Las colecciones del árbol. Se carga el paquete —el documento dice la forma
/// y el origen—, sin exigir que compile: el catálogo enseña lo que hay.
fn todas(path: &Path) -> Vec<Loaded> {
    let (pkg, _) = ore_core::validate::cargar_paquete(path);
    pkg.docs
        .into_iter()
        .filter(|d| d.kind == Kind::MediaCollection)
        .collect()
}

fn una(path: &Path, nombre: &str) -> Result<Loaded, Fallo> {
    let q = ore_core::normalize::a_corto(nombre).into_owned();
    todas(path)
        .into_iter()
        .find(|d| d.qname().as_deref() == Some(q.as_str()))
        .ok_or_else(|| {
            (
                65,
                format!("no hay ninguna colección `{nombre}` en el árbol"),
            )
        })
}

fn listar(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    let dir = dir_punteros(path, op);
    let cs: Vec<Json> = todas(path)
        .iter()
        .map(|d| {
            let p = d
                .qname()
                .and_then(|q| ore_core::punteros::leer_en(&dir, &q).map(|(_, n)| n));
            resumen(d, p.as_ref())
        })
        .collect();
    if op.json {
        println!("{}", Json::obj([("colecciones", Json::Arr(cs))]).jcs());
        return Ok(());
    }
    if cs.is_empty() {
        println!("sin colecciones · ninguna `MediaCollection` en el árbol");
    }
    for c in &cs {
        let Json::Obj(m) = c else { continue };
        let s = |k: &str| match m.get(k) {
            Some(Json::Str(v)) => v.clone(),
            _ => "-".into(),
        };
        let p = match m.get("puntero") {
            Some(Json::Obj(p)) => p.clone(),
            _ => Default::default(),
        };
        let n = |k: &str| match p.get("items") {
            Some(Json::Obj(i)) => match i.get(k) {
                Some(Json::Int(v)) => v.to_string(),
                _ => "0".into(),
            },
            _ => "-".into(),
        };
        println!(
            "{:<9} {:<40} {:<9} {:<12} {:>7} actuales · {} retirados · {} perdidos",
            s("forma"),
            s("nombre"),
            s("media"),
            match p.get("estado") {
                Some(Json::Str(e)) => e.clone(),
                _ => "sin puntero".into(),
            },
            n("actuales"),
            n("retirados"),
            n("perdidos")
        );
    }
    Ok(())
}

/// El almacén, con una petición de una línea.
fn almacen(verbo: &str, peticion: &Json) -> Result<String, Fallo> {
    let programa = programa_del_almacen().map_err(|e| (78, e))?;
    lector::ejecutar(&programa, &[verbo.to_string()], Some(&peticion.jcs())).map_err(|f| {
        let mut s = f.mensaje;
        for l in f.ayuda {
            s.push('\n');
            s.push_str(&l);
        }
        (69, s)
    })
}

/// El puntero de una colección, y el `(dataset, metadata_location)` de su
/// manifiesto si ya tiene transacción.
fn puntero_de(path: &Path, qn: &str, op: &Opciones) -> (Option<Node>, Option<(String, String)>) {
    let p = ore_core::punteros::leer_en(&dir_punteros(path, op), qn).map(|(_, n)| n);
    let lago = p.as_ref().and_then(|n| {
        let ml = campo(n, "metadata_location")?;
        let ds = campo(n, "dataset").or_else(|| ore_core::punteros::dataset_de_ubicacion(&ml))?;
        Some((ds, ml))
    });
    (p, lago)
}

fn ficha(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let d = una(path, nombre)?;
    let qn = d.qname().unwrap_or_default();
    let (p, lago) = puntero_de(path, &qn, op);
    let mut m = match resumen(&d, p.as_ref()) {
        Json::Obj(m) => m,
        _ => Default::default(),
    };
    // La historia de sus transacciones: los snapshots de su manifiesto.
    if let Some((ds, ml)) = lago {
        let h = almacen(
            "historia",
            &Json::obj([("dataset", Json::s(ds)), ("metadata_location", Json::s(ml))]),
        )?;
        let h = ore_core::parse::parse(h.trim())
            .map_err(|e| (69, format!("la historia no analiza: {e:?}")))?;
        if let Json::Obj(hm) = Json::de_node(&h) {
            for (k, v) in hm {
                if k != "metadata_location" {
                    m.insert(k, v);
                }
            }
        }
    }
    let j = Json::Obj(m);
    if op.json {
        println!("{}", j.jcs());
    } else {
        println!("{}", j.pretty());
    }
    Ok(())
}

fn items(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let d = una(path, nombre)?;
    let qn = d.qname().unwrap_or_default();
    let estado = op.estado.unwrap_or("actual");
    if !["actual", "retirado", "perdido", "todos"].contains(&estado) {
        return Err((
            64,
            format!("`--estado {estado}`: es `actual`, `retirado`, `perdido` o `todos`"),
        ));
    }
    let (_, lago) = puntero_de(path, &qn, op);
    // La página la hace el almacén: filtrar y ordenar 100.000 filas aquí,
    // leídas en texto, eran 4 s (medido, E8·1d).
    let (mut total, mut pagina) = (0usize, Vec::new());
    if let Some((ds, ml)) = lago {
        let mut p = vec![
            ("dataset", Json::s(ds)),
            ("metadata_location", Json::s(ml)),
            (
                "orden",
                Json::Arr(vec![Json::s("camino"), Json::s("version")]),
            ),
            ("desde", Json::s(op.desde.to_string())),
            ("limite", Json::s(op.limite.to_string())),
        ];
        if estado != "todos" {
            p.push(("filtro", Json::obj([("estado", Json::s(estado))])));
        }
        let texto = almacen("pagina", &Json::obj(p))?;
        let mut lineas = texto.lines();
        total = lineas
            .next()
            .and_then(|l| ore_core::parse::parse(l.trim()).ok())
            .and_then(|n| campo(&n, "total"))
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        pagina = lineas
            .filter_map(|l| ore_core::parse::parse(l.trim()).ok())
            .map(|n| Json::de_node(&n))
            .collect();
    }
    let j = Json::obj([
        ("coleccion", Json::s(&qn)),
        ("estado", Json::s(estado)),
        ("total", Json::Int(total as i64)),
        ("desde", Json::Int(op.desde as i64)),
        ("limite", Json::Int(op.limite as i64)),
        ("items", Json::Arr(pagina)),
    ]);
    if op.json {
        println!("{}", j.jcs());
    } else {
        println!("{}", j.pretty());
    }
    Ok(())
}

/// **El cotejo** (E8·2d): cada blob que el manifiesto nombra —de lo actual y
/// de lo retirado, que también se sirve—, contra el lago.
fn cotejar(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let d = una(path, nombre)?;
    let qn = d.qname().unwrap_or_default();
    if crate::coleccion::es_virtual(&d) {
        if op.json {
            println!(
                "{}",
                Json::obj([("coleccion", Json::s(&qn)), ("virtual", Json::Bool(true))]).jcs()
            );
        } else {
            println!("{qn} · virtual: sus bytes están en el origen, no hay blobs que cotejar");
        }
        return Ok(());
    }
    let (_, lago) = puntero_de(path, &qn, op);
    let (ds, ml) = lago.ok_or_else(|| {
        (
            65,
            format!("`{qn}` no tiene transacción todavía: no hay manifiesto que cotejar"),
        )
    })?;
    let texto = almacen(
        "leer",
        &Json::obj([
            ("dataset", Json::s(&ds)),
            ("metadata_location", Json::s(&ml)),
        ]),
    )?;
    // blob → (tamaño, los ítems que lo nombran)
    let mut blobs: std::collections::BTreeMap<String, (String, Vec<String>)> = Default::default();
    let mut sin_blob: Vec<String> = Vec::new();
    let mut filas = 0usize;
    for l in texto.lines() {
        let Ok(n) = ore_core::parse::parse(l.trim()) else {
            continue;
        };
        let Some(camino) = campo(&n, "camino") else {
            continue;
        };
        filas += 1;
        let item = format!(
            "{camino} (versión {}, {})",
            campo(&n, "version").unwrap_or_default(),
            campo(&n, "estado").unwrap_or_default()
        );
        match campo(&n, "blob") {
            Some(b) => blobs
                .entry(b)
                .or_insert_with(|| (campo(&n, "tamano").unwrap_or_default(), Vec::new()))
                .1
                .push(item),
            None => sin_blob.push(item),
        }
    }
    let r = almacen(
        "blobs-cotejar",
        &Json::obj([
            (
                "blobs",
                Json::Arr(
                    blobs
                        .iter()
                        .map(|(b, (t, _))| Json::Arr(vec![Json::s(b), Json::s(t)]))
                        .collect(),
                ),
            ),
            ("muestra", Json::s(op.muestra.to_string())),
        ]),
    )?;
    let r = ore_core::parse::parse(r.trim())
        .map_err(|e| (69, format!("el cotejo no analiza: {e:?}")))?;
    let rotos: Vec<(String, String, Vec<String>)> = r
        .get("rotos")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| {
            let b = campo(x, "blob")?;
            let items = blobs.get(&b).map(|(_, i)| i.clone()).unwrap_or_default();
            Some((b, campo(x, "motivo").unwrap_or_default(), items))
        })
        .collect();
    let n = |k: &str| {
        campo(&r, k)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    if op.json {
        let j = Json::obj([
            ("coleccion", Json::s(&qn)),
            ("filas", Json::Int(filas as i64)),
            ("blobs", Json::Int(n("cotejados"))),
            ("bien", Json::Int(n("bien"))),
            ("releidos", Json::Int(n("releidos"))),
            (
                "rotos",
                Json::Arr(
                    rotos
                        .iter()
                        .map(|(b, m, i)| {
                            Json::obj([
                                ("blob", Json::s(b)),
                                ("motivo", Json::s(m)),
                                ("items", Json::Arr(i.iter().map(Json::s).collect())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "sin_blob",
                Json::Arr(sin_blob.iter().map(Json::s).collect()),
            ),
        ]);
        println!("{}", j.jcs());
    } else {
        println!(
            "{qn} · {filas} filas, {} blobs: {} bien, {} vueltos a hashear",
            n("cotejados"),
            n("bien"),
            n("releidos")
        );
        for (b, m, i) in &rotos {
            println!("  roto · {b} · {m} · {}", i.join(", "));
        }
        for i in &sin_blob {
            println!("  roto · sin blob · {i}");
        }
    }
    let malos = rotos.len() + sin_blob.len();
    if malos > 0 {
        return Err((
            1,
            format!(
                "`{qn}`: {malos} blobs o ítems rotos: el manifiesto dice algo que el lago no tiene"
            ),
        ));
    }
    Ok(())
}

/// **El mantenimiento de las colecciones** (E8·3): la retención de cada una y
/// la recogida de los blobs del inquilino. Los vivos son los que nombra
/// **cualquier** fila de **cualquier** manifiesto —de una colección con
/// documento o de una cuyo documento ya se fue y aún tiene puntero—; si uno
/// no se puede leer, la lista no está entera y no se recoge nada.
fn recoger(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    let gracia = match op.gracia {
        Some(g) => crate::datasets::edad_ms(g).map_err(|m| (64, m))?,
        None => 2 * 3_600_000,
    };
    let ahora = crate::coleccion::ahora_ms();
    let docs = todas(path);
    let dir = dir_punteros(path, op);
    let mut vivos: std::collections::BTreeSet<String> = Default::default();
    let mut lineas: Vec<Json> = Vec::new();
    let mut ilegibles: Vec<String> = Vec::new();
    let mut movidos = 0usize;
    for p in crate::datasets::punteros(path, &dir)
        .into_iter()
        .filter(|p| p.es_coleccion())
    {
        let d = docs
            .iter()
            .find(|d| d.qname().as_deref() == Some(p.nombre.as_str()));
        match crate::coleccion::caducar(d, &p.nombre, &p.nodo, ahora, op.seco) {
            Err(e) => {
                ilegibles.push(format!("{}: {e}", p.nombre));
                lineas.push(Json::obj([
                    ("coleccion", Json::s(&p.nombre)),
                    ("error", Json::s(&e)),
                ]));
            }
            Ok(c) => {
                vivos.extend(
                    c.filas
                        .iter()
                        .filter(|f| !f.blob.is_empty())
                        .map(|f| f.blob.clone()),
                );
                if let Some(nuevo) = &c.puntero {
                    std::fs::write(&p.ruta, nuevo.pretty() + "\n").map_err(|e| {
                        (
                            73,
                            format!("no se pudo escribir `{}`: {e}", p.ruta.display()),
                        )
                    })?;
                    movidos += 1;
                }
                lineas.push(Json::obj([
                    ("coleccion", Json::s(&p.nombre)),
                    ("linea", Json::s(&c.linea)),
                    ("movido", Json::Bool(c.puntero.is_some())),
                ]));
            }
        }
    }
    let blobs = if ilegibles.is_empty() {
        let r = almacen(
            "blobs-recoger",
            &Json::obj([
                ("vivos", Json::Arr(vivos.iter().map(Json::s).collect())),
                ("gracia_ms", Json::s(gracia.to_string())),
                ("seco", Json::Bool(op.seco)),
            ]),
        )?;
        Some(
            ore_core::parse::parse(r.trim())
                .map_err(|e| (69, format!("la recogida no analiza: {e:?}")))?,
        )
    } else {
        None
    };
    let n = |k: &str| {
        blobs
            .as_ref()
            .and_then(|b| campo(b, k))
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    if op.json {
        println!(
            "{}",
            Json::obj([
                ("colecciones", Json::Arr(lineas)),
                ("punteros_movidos", Json::Int(movidos as i64)),
                ("seco", Json::Bool(op.seco)),
                ("gracia_ms", Json::Int(gracia)),
                (
                    "blobs",
                    match &blobs {
                        Some(b) => Json::de_node(b),
                        None => Json::Crudo("null".into()),
                    }
                ),
            ])
            .jcs()
        );
    } else {
        for l in &lineas {
            if let Json::Obj(m) = l {
                let s = |k: &str| match m.get(k) {
                    Some(Json::Str(v)) => v.clone(),
                    _ => String::new(),
                };
                let texto = if s("error").is_empty() {
                    s("linea")
                } else {
                    format!("no se pudo leer · {}", s("error"))
                };
                println!("{} · {texto}", s("coleccion"));
            }
        }
        if blobs.is_some() {
            println!(
                "{}blobs: {} en el lago, {} vivos, {} en su gracia, {} recogidos ({} KB) · {} huellas del índice recogidas · {movidos} puntero(s) movido(s)",
                if op.seco { "en seco · " } else { "" },
                n("blobs"),
                n("vivos"),
                n("en_gracia"),
                n("recogidos"),
                n("bytes") / 1024,
                n("huellas_recogidas")
            );
        }
    }
    if !ilegibles.is_empty() {
        return Err((
            69,
            format!(
                "no se recoge ningún blob: {} manifiesto(s) no se pudieron leer y la lista de vivos no está entera ({})",
                ilegibles.len(),
                ilegibles.join("; ")
            ),
        ));
    }
    Ok(())
}
