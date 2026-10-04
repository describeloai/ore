//! `ore collections` — **las colecciones del árbol, como activos** (0046
//! E8·1d): lo que `ore datasets` es para un dataset.
//!
//! | | qué | quién lo llama |
//! |---|---|---|
//! | `ore collections .` | cada `MediaCollection`: su forma (virtual, mantenida, escrita), su origen, su medio y sus formatos, y el estado de su puntero (transacción, ítems actuales, retirados y perdidos) | `GET /colecciones` |
//! | `--ficha b.s.n` | lo mismo de una, y **la historia de sus transacciones** (los snapshots de su manifiesto, `ore-store historia`) | `GET /colecciones/{b}/{s}/{n}` |
//! | `--items b.s.n [--estado …] [--desde N] [--limite N]` | sus ítems, por estado (`actual` por defecto; `retirado`, `perdido`, `todos`), en orden de camino, paginados | `GET /colecciones/{b}/{s}/{n}/items` |
//! | (en las tres) | si el manifiesto pasa de 500.000 filas, el aviso del **techo** (E8·3c): cada transacción lo reescribe entero, y toca *merge-on-read* | quien mira la colección |
//! | `--recoger [--seco] [--gracia 2h]` | **el mantenimiento** (E8·3): la retención de cada colección (lo retirado más viejo que su `retention` sale del manifiesto, y el puntero se mueve) y después la **recogida de blobs** del inquilino: lo que ninguna fila de ningún manifiesto nombra, y nadie tocó en la gracia, se va. Si un manifiesto no se puede leer, no se recoge nada | el CronJob de mantenimiento |
//! | `--servir b.s.n --huella H [--huella H…] [--ttl 300]` | **servir** (0046 E9·2 y E9·3): de cada huella, el ítem que la lleva —el actual antes que el retirado— y una **URL firmada**: a su blob en el lago (mantenida) o a su versión en el origen (virtual, con la credencial de `connectionEnv`; sin ella sale con 69 y dice `{necesita: {fuente, env}}`), que vive `--ttl` segundos (5 min; entre 30 s y 1 h), con su tipo y su disposición dentro de la firma: `inline` sólo para lo que un navegador enseña sin ejecutar (PDF, imagen de mapa de bits, vídeo, audio); lo demás, descarga. Quién puede lo decide quien llama | `GET /colecciones/{b}/{s}/{n}/items/{huella}`, `POST …/items/resolver` |
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
    /// Con `recoger`: los punteros propios de las demás ramas (0044 C D7a).
    pub reclaman: Option<&'a Path>,
    pub servir: Option<&'a str>,
    pub huellas: &'a [String],
    pub ttl: Option<u64>,
}

type Fallo = (u8, String);

pub fn colecciones(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let hecho = if let Some(n) = op.servir {
        servir(path, n, op)
    } else if op.recoger {
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
                    "techo",
                    "blobs",
                    "no_copiados",
                    "retencion",
                    // 0049 B8·3: si lo sellado es de una virtual, y lo que a
                    // una mantenida le falta en el lago
                    "virtual",
                    "por_copiar",
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
        if let Some(Json::Obj(t)) = p.get("techo")
            && let Some(Json::Str(a)) = t.get("aviso")
        {
            println!("          ⚠ techo · {a}");
        }
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
    // 0049 B8·3: una mantenida copia por el conducto `materialization.payload`;
    // sin él autorizado su copia no se encola (OOS4011), y se dice.
    if forma(&d) == "mantenida" {
        let conducto = std::fs::read_to_string(path.join("conduits.yaml"))
            .is_ok_and(|t| t.contains("materialization.payload"));
        m.insert("conducto".into(), Json::Bool(conducto));
    }
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

/// Cuántas huellas se resuelven de una vez: una página de una lista, no un volcado.
pub const HUELLAS_POR_LOTE: usize = 100;

// `disposicion` y lo que se sirve en línea viven en `ore_core::medios`: los
// usa también `ore-medios` (0049 B2).
pub use ore_core::medios::disposicion;

/// De las filas con una huella, la que se sirve: la actual antes que la
/// retirada, y entre iguales la de menor camino (determinista). Lo perdido no
/// se sirve; de una mantenida, tampoco una fila sin blob.
fn la_que_se_sirve<'a>(filas: &'a [Node], huella: &str, virtual_: bool) -> Option<&'a Node> {
    let rango = |n: &Node| match campo(n, "estado").as_deref() {
        Some("actual") => 0,
        Some("retirado") => 1,
        _ => 2,
    };
    filas
        .iter()
        .filter(|n| {
            campo(n, "huella").as_deref() == Some(huella)
                && matches!(campo(n, "estado").as_deref(), Some("actual" | "retirado"))
                && (virtual_ || campo(n, "blob").is_some())
        })
        .min_by(|a, b| {
            rango(a)
                .cmp(&rango(b))
                .then_with(|| campo(a, "camino").cmp(&campo(b, "camino")))
        })
}

/// Lo que se sirve de una huella: la huella, su fila, su tipo, el modo y la
/// cabecera `Content-Disposition`.
type Servido<'a> = (String, &'a Node, String, &'static str, String);

/// **Servir** (0046 E9·2 y E9·3): de cada huella, su ítem y una URL firmada.
/// Sale con 65 si la colección o su transacción no están, con 66 si su origen
/// no sabe firmar, con 69 —y `{necesita: {fuente, env}}` en la salida— si falta
/// la credencial de una virtual, y con 64 si lo pedido no vale.
fn servir(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let d = una(path, nombre)?;
    let qn = d.qname().unwrap_or_default();
    if op.huellas.is_empty() || op.huellas.len() > HUELLAS_POR_LOTE {
        return Err((
            64,
            format!("de 1 a {HUELLAS_POR_LOTE} huellas por vez (`--huella`)"),
        ));
    }
    if let Some(h) = op
        .huellas
        .iter()
        .find(|h| h.is_empty() || h.len() > 256 || h.chars().any(char::is_control))
    {
        return Err((64, format!("`{h}` no es una huella")));
    }
    let virtual_ = crate::coleccion::es_virtual(&d);
    let (_, lago) = puntero_de(path, &qn, op);
    let (ds, ml) = lago.ok_or_else(|| {
        (
            65,
            format!("`{qn}` no tiene transacción todavía: no hay ítems que servir"),
        )
    })?;
    let texto = almacen(
        "pagina",
        &Json::obj([
            ("dataset", Json::s(&ds)),
            ("metadata_location", Json::s(&ml)),
            (
                "filtro",
                Json::obj([(
                    "huella",
                    Json::Arr(op.huellas.iter().map(Json::s).collect()),
                )]),
            ),
            ("limite", Json::s("100000")),
        ]),
    )?;
    let filas: Vec<Node> = texto
        .lines()
        .skip(1)
        .filter_map(|l| ore_core::parse::parse(l.trim()).ok())
        .collect();
    let mut servidos: Vec<Servido> = Vec::new();
    let mut no_estan: Vec<String> = Vec::new();
    for h in op.huellas {
        if servidos.iter().any(|(x, ..)| x == h) || no_estan.contains(h) {
            continue;
        }
        match la_que_se_sirve(&filas, h, virtual_) {
            None => no_estan.push(h.clone()),
            Some(f) => {
                let tipo = campo(f, "tipo")
                    .or_else(|| {
                        campo(f, "formato")
                            .map(|x| crate::coleccion::tipo_de_formato(&x.to_ascii_lowercase()))
                    })
                    .unwrap_or_else(|| "application/octet-stream".into());
                let camino = campo(f, "camino").unwrap_or_default();
                let nombre = camino.rsplit('/').next().unwrap_or(&camino).to_string();
                let (modo, cabecera) = disposicion(&tipo, &nombre);
                servidos.push((h.clone(), f, tipo, modo, cabecera));
            }
        }
    }
    let (urls, segundos, caduca_ms) = if servidos.is_empty() {
        (Vec::new(), 0, 0)
    } else if virtual_ {
        firmar_en_el_origen(path, &d, &servidos, op.ttl)?
    } else {
        firmar_en_el_lago(&servidos, op.ttl)?
    };
    let firmadas: std::collections::BTreeMap<String, String> =
        servidos.iter().map(|(h, ..)| h.clone()).zip(urls).collect();
    let items: Vec<Json> = servidos
        .iter()
        .map(|(h, f, tipo, modo, _)| {
            let mut m = vec![
                ("huella", Json::s(h)),
                ("url", Json::s(firmadas.get(h).cloned().unwrap_or_default())),
                ("tipo", Json::s(tipo)),
                ("disposicion", Json::s(*modo)),
            ];
            for k in ["blob", "camino", "clave", "version", "estado"] {
                if let Some(v) = campo(f, k) {
                    m.push((k, Json::s(v)));
                }
            }
            m.push((
                "tamano",
                Json::Int(campo(f, "tamano").and_then(|t| t.parse().ok()).unwrap_or(0)),
            ));
            Json::obj(m)
        })
        .collect();
    let j = Json::obj([
        ("coleccion", Json::s(&qn)),
        ("virtual", Json::Bool(virtual_)),
        ("segundos", Json::Int(segundos)),
        ("caduca_ms", Json::Int(caduca_ms)),
        ("items", Json::Arr(items)),
        (
            "no_estan",
            Json::Arr(no_estan.iter().map(Json::s).collect()),
        ),
    ]);
    if op.json {
        println!("{}", j.jcs());
    } else {
        println!("{}", j.pretty());
    }
    Ok(())
}

/// Las URLs de una mantenida: sus blobs, firmados por el lago
/// (`ore-store blob-firmar`), en el orden pedido.
fn firmar_en_el_lago(
    servidos: &[Servido],
    ttl: Option<u64>,
) -> Result<(Vec<String>, i64, i64), Fallo> {
    let mut p = vec![(
        "firmas",
        Json::Arr(
            servidos
                .iter()
                .map(|(_, f, tipo, _, cab)| {
                    Json::obj([
                        ("blob", Json::s(campo(f, "blob").unwrap_or_default())),
                        ("tipo", Json::s(tipo)),
                        ("disposicion", Json::s(cab)),
                    ])
                })
                .collect(),
        ),
    )];
    if let Some(t) = ttl {
        p.push(("segundos", Json::s(t.to_string())));
    }
    let r = almacen("blob-firmar", &Json::obj(p))?;
    let r = ore_core::parse::parse(r.trim())
        .map_err(|e| (69, format!("la firma no analiza: {e:?}")))?;
    let num = |k: &str| campo(&r, k).and_then(|v| v.parse().ok()).unwrap_or(0);
    let (segundos, caduca_ms) = (num("segundos"), num("caduca_ms"));
    // Por posición, no por blob: dos ítems con el mismo blob pueden pedir
    // disposiciones distintas (su nombre).
    let urls: Vec<String> = r
        .get("firmadas")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| campo(x, "url"))
        .collect();
    if urls.len() != servidos.len() {
        return Err((69, "el almacén no firmó todo lo pedido".into()));
    }
    Ok((urls, segundos, caduca_ms))
}

/// El firmante de las URLs de un bucket de S3: sin red (`ore-sigv4`), así que
/// cabe donde un lector no (la imagen de `ore-serve`).
const FIRMANTE_S3: &str = "ore-firmar-s3";

/// Las URLs de una virtual: **del origen**, cada ítem fijado a su versión, con
/// la credencial de su fuente (`connectionEnv`) y firmadas por
/// [`FIRMANTE_S3`], que no puede abrir un socket. Sin la credencial, dice
/// cuál necesita —quien sirve la trae del cofre (E9·3)— y sale con 69.
fn firmar_en_el_origen(
    path: &Path,
    d: &Loaded,
    servidos: &[Servido],
    ttl: Option<u64>,
) -> Result<(Vec<String>, i64, i64), Fallo> {
    let (fuente, tipo, env) = crate::coleccion::fuente_de(path, d).map_err(|m| (65, m))?;
    if tipo != "s3" {
        return Err((
            66,
            format!("servir del origen `{fuente}` ({tipo}) no se sabe todavía: sólo de S3"),
        ));
    }
    let Ok(url) = lector::url(path, &env, &fuente) else {
        println!(
            "{}",
            Json::obj([(
                "necesita",
                Json::obj([("fuente", Json::s(&fuente)), ("env", Json::s(&env))]),
            )])
            .jcs()
        );
        return Err((
            69,
            format!("`{env}` no está definida: sin la credencial de `{fuente}` no se firma"),
        ));
    };
    let mut p = vec![
        ("url", Json::s(url)),
        (
            "items",
            Json::Arr(
                servidos
                    .iter()
                    .map(|(_, f, tipo, _, cab)| {
                        Json::obj([
                            ("clave", Json::s(campo(f, "clave").unwrap_or_default())),
                            ("version", Json::s(campo(f, "version").unwrap_or_default())),
                            ("tipo", Json::s(tipo)),
                            ("disposicion", Json::s(cab)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    if let Some(t) = ttl {
        p.push(("segundos", Json::s(t.to_string())));
    }
    let r = lector::ejecutar(FIRMANTE_S3, &[], Some(&Json::obj(p).jcs()))
        .map_err(|f| (69, f.mensaje))?;
    let r = ore_core::parse::parse(r.trim())
        .map_err(|e| (69, format!("la firma del origen no analiza: {e:?}")))?;
    let segundos: i64 = campo(&r, "segundos")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let urls: Vec<String> = r
        .get("firmadas")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| campo(x, "url"))
        .collect();
    if urls.len() != servidos.len() {
        return Err((69, "el lector no firmó todo lo pedido".into()));
    }
    Ok((
        urls,
        segundos,
        crate::coleccion::ahora_ms() + segundos * 1000,
    ))
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
///
/// ⭐ Y de **cualquier rama** (0044 C D7a): con `--reclaman`, también los de
///   las colecciones que las demás ramas construyeron. Los blobs son del
///   inquilino, no de una rama: uno que sólo nombra una rama se borraría
///   pasada la gracia, como un dataset antes de D1.
fn recoger(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    let ajenos = crate::datasets::Ajenos::de(op.reclaman).map_err(|m| (66, m))?;
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
    // Las de las demás ramas: lo que sus manifiestos nombran, entero (la
    // retención de una rama no corre aquí; lo que ella retiró, ella lo guarda).
    let propios: std::collections::BTreeSet<String> = crate::datasets::punteros(path, &dir)
        .into_iter()
        .filter(|p| p.es_coleccion())
        .filter_map(|p| p.campo("metadata_location"))
        .collect();
    let mut de_otras = 0usize;
    for (dataset, ml) in &ajenos.colecciones {
        if propios.contains(ml) {
            continue;
        }
        match crate::coleccion::manifiesto(dataset, ml) {
            Ok(filas) => {
                de_otras += 1;
                vivos.extend(
                    filas
                        .into_iter()
                        .filter(|f| !f.blob.is_empty())
                        .map(|f| f.blob),
                );
            }
            Err(e) => {
                ilegibles.push(format!("{dataset} (de otra rama): {e}"));
                lineas.push(Json::obj([
                    ("coleccion", Json::s(dataset)),
                    ("rama", Json::s("otra")),
                    ("error", Json::s(&e)),
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
                ("de_otras_ramas", Json::Int(de_otras as i64)),
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
        if op.reclaman.is_some() {
            println!("de las demás ramas: {de_otras} colección(es) con blobs que no son de aquí");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_lo_que_no_ejecuta_va_en_linea() {
        assert_eq!(disposicion("application/pdf", "a.pdf").0, "inline");
        assert_eq!(disposicion("video/mp4", "v.mp4").0, "inline");
        for activo in [
            "text/html",
            "image/svg+xml",
            "application/xml",
            "text/plain",
            "",
        ] {
            assert_eq!(disposicion(activo, "x").0, "attachment", "{activo}");
        }
    }

    #[test]
    fn el_nombre_no_rompe_la_cabecera() {
        let (_, c) = disposicion("application/pdf", "contrato \"ñ\"\r\n.pdf");
        assert_eq!(
            c,
            "inline; filename=\"contrato _____.pdf\"; filename*=UTF-8''contrato%20%22%C3%B1%22%0D%0A.pdf"
        );
    }

    #[test]
    fn se_sirve_la_actual_antes_que_la_retirada() {
        let f = |s: &str| ore_core::parse::parse(s).unwrap();
        let filas = vec![
            f(r#"{"huella":"h","estado":"retirado","camino":"a.pdf","blob":"1"}"#),
            f(r#"{"huella":"h","estado":"actual","camino":"z.pdf","blob":"2"}"#),
            f(r#"{"huella":"h","estado":"actual","camino":"b.pdf","blob":"3"}"#),
            f(r#"{"huella":"g","estado":"retirado","camino":"g.pdf","blob":"4"}"#),
            f(r#"{"huella":"s","estado":"actual","camino":"s.pdf"}"#),
        ];
        let blob = |h| la_que_se_sirve(&filas, h, false).and_then(|n| campo(n, "blob"));
        assert_eq!(blob("h").as_deref(), Some("3"));
        assert_eq!(blob("g").as_deref(), Some("4"));
        assert_eq!(blob("s"), None, "sin blob no se sirve");
        assert_eq!(blob("nada"), None);
        // De una virtual: sin blob, por su versión; lo perdido no se sirve.
        let v = vec![
            f(r#"{"huella":"v","estado":"perdido","camino":"a.pdf","version":"1"}"#),
            f(r#"{"huella":"v","estado":"retirado","camino":"b.pdf","version":"2"}"#),
            f(r#"{"huella":"p","estado":"perdido","camino":"p.pdf","version":"3"}"#),
        ];
        let version = |h| la_que_se_sirve(&v, h, true).and_then(|n| campo(n, "version"));
        assert_eq!(version("v").as_deref(), Some("2"));
        assert_eq!(version("p"), None);
    }
}
