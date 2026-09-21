//! `ore datasets` — **los datasets del árbol, por sus punteros** (W3.6b,
//! [0031 §10](../../../docs/decisions/0031-el-puesto.md)).
//!
//! Todo es un dataset: la copia de una vista materializada (`copias/<p>_<v>.json`)
//! y la salida de un `write()` (`datasets/<p>_<t>.json`) son la misma cosa —una
//! tabla Iceberg en el bucket, con historia— y el árbol es el catálogo. Este
//! verbo trabaja **sobre los punteros y nada más**: no compila el árbol entero,
//! no abre ningún origen, y lo único que toca del bucket lo toca por
//! `ore-store-<r2|gcs>` (`ore` no abre un socket).
//!
//! | | qué | quién lo llama |
//! |---|---|---|
//! | `ore datasets .` | los punteros, con su estado | `GET /datasets` |
//! | `--ficha p.x` | el puntero y la historia de la tabla (`ore-store historia`) | `GET /datasets/{ns}/{n}` |
//! | `--recoger [--edad 7d]` | expirar lo superado, retirar lo que nadie nombra, mover los punteros | el CronJob de mantenimiento |
//! | `--confirmar p.t --metadata-location …` | **el swap**: el puntero de un dataset del lago, con su `Table` | `POST /datasets/{ns}/{n}/confirmar`, por quien no puede empujar |
//! | `--commit [--tabla p.t] --peticion …` | **el commit del catálogo REST** (W3.6c, 0031 §11): `requirements` + `updates` de un `updateTable` o un `commitTransaction`, aplicados por `ore-store aplicar`, la clave de operación cotejada, la `Table` que nace o evoluciona, y el puntero | `POST /v1/namespaces/{ns}/tables/{t}` y `POST /v1/transactions/commit` |
//! | `--crear p.t --peticion …` | la tabla nace de un `createTable` (sin `stage-create`): su `metadata.json` v0, su `Table` y su puntero | `POST /v1/namespaces/{ns}/tables` |
//! | `--esbozar p.t --peticion …` | `stage-create`: los metadatos que la tabla tendría, sin escribir nada | idem, con `stage-create: true` |
//! | `--retencion p.t --edad 30d [--minimo 3]` | la retención declarada en la tabla (`history.expire.*`), que `--recoger` obedece | quien gobierna el dataset |
//! | `--cargar p.t [--prestar]` | el `LoadTableResult` de la spec REST, tal cual: el puntero, el `metadata.json` del bucket y, con `--prestar`, la credencial acotada a la tabla (`ore-store prestar`) | `GET /v1/namespaces/{ns}/tables/{t}` |
//!
//! # El commit, y lo que decide `ore` (0031 §11 ①④⑥)
//!
//! El cuerpo del cliente —PyIceberg, DuckDB, o el agente del puesto tras
//! `ore-store escribir`— **pasa tal cual** a `ore-store aplicar` (el JSON de
//! `ore` no modela `null`, y un `assert-ref-snapshot-id` de una tabla recién
//! nacida lo lleva). Aquí se decide lo que es del catálogo: el paquete existe;
//! la `Table` es del lago o no existe todavía (una `View` o una `Table` de otra
//! fuente no se escribe); **el puntero vigente es la base** contra la que se
//! validan los requisitos (el CAS semántico: un `assert-ref-snapshot-id` que no
//! cuadra es código 75 con `actual`); **la clave de operación** del snapshot
//! (`ore.operacion`) se coteja con la ancestría y, si ya está, se contesta con
//! lo que hay sin tocar nada; la tabla que nace recibe la retención por defecto
//! si no la trae; la `Table` nace con las columnas de OOS traducidas del
//! esquema de Iceberg (o las actualiza cuando el esquema evolucionó, si el
//! documento es de los nuestros); y el puntero se mueve. Un `commitTransaction`
//! de N tablas aplica las N antes de mover ningún puntero: o se mueven todos en
//! el commit del árbol que `ore-serve` hace después, o ninguno (lo que se
//! escribió y no se apuntó lo retira `--recoger` como huérfano).
//!
//! # El swap, y por qué lo hace `ore` y no `ore-serve`
//!
//! Quien escribe un dataset desde un puesto (`write()`, W3.6c) puede escribir en
//! el bucket con la identidad del pod, pero **no puede empujar al árbol**: el
//! testigo de la forja es de `ore-serve`. Así que le pide a `ore-serve` que
//! confirme, y `ore-serve` clona, corre esto, y empuja. La decisión —¿el
//! puntero sigue donde el escritor lo dejó (`esperado`)?, ¿el `metadata.json`
//! está en el bucket?, ¿la `Table` del lago existe o nace ahora con estas
//! columnas?— vive aquí, donde se puede probar sin servidor. El
//! compare-and-set tiene dos caras y las dos se dicen: la semántica (el puntero
//! ya no es el esperado → **código 75**, y quién lo movió está en el árbol) y
//! la de la forja (el empujón que no avanza en línea recta → 409, en
//! `ore-serve`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ore_core::json::Json;
use ore_core::parse::Node;

use crate::lector;
use crate::materializar::{campo_de, programa_del_almacen};

/// Código de salida del CAS perdido: `EX_TEMPFAIL`, «vuelve a intentarlo
/// sobre lo que hay ahora». `ore-serve` lo traduce a 409.
pub const ADELANTADO: u8 = 75;

pub struct Opciones<'a> {
    pub json: bool,
    pub ficha: Option<&'a str>,
    pub recoger: bool,
    /// Cuánta historia conserva `--recoger`: `7d`, `12h`, `30m`, `0`.
    pub edad: Option<&'a str>,
    pub seco: bool,
    pub confirmar: Option<&'a str>,
    pub metadata_location: Option<&'a str>,
    pub esperado: Option<&'a str>,
    pub snapshot: Option<&'a str>,
    pub filas: Option<i64>,
    /// Las columnas de la `Table` del lago, como JSON `{"col": "Tipo", …}`.
    pub columnas: Option<&'a str>,
    pub sujeto: Option<&'a str>,
    /// Dónde viven los punteros de las copias; sin él, `<árbol>/copias`.
    pub informe: Option<&'a Path>,
    /// `--commit`: el commit del catálogo REST (ver la cabecera).
    pub commit: bool,
    /// Con `--commit`: la tabla, cuando el cuerpo no trae `identifier`.
    pub tabla: Option<&'a str>,
    /// `--crear p.t`: la tabla nace de un `createTable`.
    pub crear: Option<&'a str>,
    /// `--esbozar p.t`: `stage-create`, sin escribir nada.
    pub esbozar: Option<&'a str>,
    /// `--retencion p.t`: la retención declarada en la tabla (con `--edad` y `--minimo`).
    pub retencion: Option<&'a str>,
    /// Con `--retencion`: cuántos snapshots se conservan como mínimo.
    pub minimo: Option<i64>,
    /// El cuerpo de la petición: JSON, `@fichero` o `-` (stdin).
    pub peticion: Option<&'a str>,
    /// La retención de una tabla que nace y no la trae (`7d`); sin ella, no se declara.
    pub retencion_defecto: Option<&'a str>,
    /// `--cargar p.t`: el `LoadTableResult` de la tabla.
    pub cargar: Option<&'a str>,
    /// Con `--cargar`/`--esbozar`: la credencial acotada a la tabla, prestada.
    pub prestar: bool,
}

pub fn datasets(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let r = if let Some(n) = op.confirmar {
        confirmar(path, n, op)
    } else if op.commit {
        commit(path, op)
    } else if let Some(n) = op.crear {
        crear(path, n, op)
    } else if let Some(n) = op.esbozar {
        esbozar(path, n, op)
    } else if let Some(n) = op.cargar {
        cargar(path, n, op)
    } else if let Some(n) = op.retencion {
        retencion(path, n, op)
    } else if let Some(n) = op.ficha {
        ficha(path, n, op)
    } else if op.recoger {
        recoger(path, op)
    } else {
        listar(path, op)
    };
    match r {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err((codigo, m)) => {
            // Un mensaje vacío es un fallo que ya se dijo (el CAS perdido
            // imprime su JSON con `actual` antes de devolver el código).
            if !m.is_empty() {
                if op.json {
                    println!("{}", Json::obj([("error", Json::s(&m))]).jcs());
                }
                eprintln!("error: {m}");
            }
            std::process::ExitCode::from(codigo)
        }
    }
}

type Fallo = (u8, String);

/// Un puntero leído del árbol: de qué clase, cómo se llama, y lo que dice.
pub(crate) struct Puntero {
    /// `copia` (una View materializada) o `dataset` (una Table del lago).
    pub clase: &'static str,
    /// `<paquete>.<nombre>`, reconstruido del fichero.
    pub nombre: String,
    pub ruta: PathBuf,
    pub nodo: Node,
}

impl Puntero {
    pub fn campo(&self, k: &str) -> Option<String> {
        campo_de(&self.nodo, k)
    }
    /// El nombre del dataset en el bucket: lo que el puntero diga (uno migrado
    /// sigue en `copias/p_v`: los bytes no se mueven), o `datasets/p_n`.
    pub fn dataset(&self) -> String {
        self.campo("dataset")
            .unwrap_or_else(|| format!("datasets/{}", self.nombre.replace('.', "_")))
    }
    pub fn como_json(&self) -> Json {
        let mut m = match Json::de_node(&self.nodo) {
            Json::Obj(m) => m,
            _ => Default::default(),
        };
        m.insert("clase".into(), Json::s(self.clase));
        m.insert("nombre".into(), Json::s(&self.nombre));
        m.insert("dataset".into(), Json::s(self.dataset()));
        Json::Obj(m)
    }
}

/// Los punteros, de **una** carpeta (0033: un kind, un puntero). El nombre
/// `<p>.<x>` sale del campo `nombre`, `tabla` o `vista` del puntero y, si no lo
/// trae, del nombre del fichero (`<p>_<x>.json`: la primera `_` separa el
/// paquete, que no lleva ninguna). La clase la dice el documento del árbol:
/// **mantenido** si `packages/<p>/datasets/<x>.yaml` lleva `from`, **escrito**
/// si no; sin documento, escrito (nació de un `write()` y aún no se declaró).
pub(crate) fn punteros(path: &Path, dir: &Path) -> Vec<Puntero> {
    let mut out = Vec::new();
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut rutas: Vec<PathBuf> = entradas
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    rutas.sort();
    for ruta in rutas {
        let Ok(t) = std::fs::read_to_string(&ruta) else {
            continue;
        };
        let Ok(nodo) = ore_core::parse::parse(&t) else {
            continue;
        };
        let nombre = ["nombre", "tabla", "vista"]
            .into_iter()
            .find_map(|k| campo_de(&nodo, k))
            .unwrap_or_else(|| {
                let stem = ruta
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                match stem.split_once('_') {
                    Some((p, x)) => format!("{p}.{x}"),
                    None => stem.to_string(),
                }
            });
        let clase = match nombre.split_once('.') {
            Some((ns, n)) => {
                let doc = path
                    .join("packages")
                    .join(ns)
                    .join("datasets")
                    .join(format!("{n}.yaml"));
                match std::fs::read_to_string(&doc)
                    .ok()
                    .and_then(|t| ore_core::parse::parse(&t).ok())
                {
                    Some(d) if d.get("spec").is_some_and(|(_, s)| s.get("from").is_some()) => {
                        "mantenido"
                    }
                    _ => "escrito",
                }
            }
            None => "escrito",
        };
        out.push(Puntero {
            clase,
            nombre,
            ruta,
            nodo,
        });
    }
    out
}

fn dir_copias(path: &Path, op: &Opciones) -> PathBuf {
    op.informe
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.join("datasets"))
}

fn listar(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    let ps = punteros(path, &dir_copias(path, op));
    if op.json {
        println!(
            "{}",
            Json::obj([(
                "datasets",
                Json::Arr(ps.iter().map(Puntero::como_json).collect())
            )])
            .jcs()
        );
        return Ok(());
    }
    if ps.is_empty() {
        println!("sin datasets · ningún puntero en `datasets/`");
        return Ok(());
    }
    for p in &ps {
        println!(
            "{:<8} {:<40} {:<9} {:>10} filas · {}",
            p.clase,
            p.nombre,
            p.campo("estado").unwrap_or_default(),
            p.campo("filas").unwrap_or_else(|| "?".into()),
            p.campo("metadata_location")
                .or_else(|| p.campo("clave").map(|c| format!("{c} (sobre heredado)")))
                .unwrap_or_else(|| "sin dataset".into())
        );
    }
    Ok(())
}

/// El almacén, delegado: una petición de una línea, una línea de vuelta.
fn almacen(verbo: &str, peticion: &Json) -> Result<Node, Fallo> {
    let programa = programa_del_almacen().map_err(|e| (78, e))?;
    let salida =
        lector::ejecutar(&programa, &[verbo.to_string()], Some(&peticion.jcs())).map_err(|f| {
            let mut s = f.mensaje;
            for l in f.ayuda {
                s.push('\n');
                s.push_str(&l);
            }
            (69, s)
        })?;
    ore_core::parse::parse(&salida).map_err(|e| {
        (
            69,
            format!("lo que devolvió `{programa} {verbo}` no analiza: {e:?}"),
        )
    })
}

fn ficha(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let ps = punteros(path, &dir_copias(path, op));
    let Some(p) = ps.iter().find(|p| p.nombre == nombre) else {
        return Err((
            65,
            format!("no hay ningún dataset `{nombre}` en `datasets/`"),
        ));
    };
    let mut m = match p.como_json() {
        Json::Obj(m) => m,
        _ => Default::default(),
    };
    if let Some(ml) = p.campo("metadata_location") {
        let h = almacen(
            "historia",
            &Json::obj([
                ("dataset", Json::s(p.dataset())),
                ("metadata_location", Json::s(&ml)),
            ]),
        )?;
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

/// `7d`, `12h`, `30m`, `45s` o `0` → milisegundos.
pub(crate) fn edad_ms(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let (n, mult) = match s.chars().last() {
        Some('d') => (&s[..s.len() - 1], 86_400_000),
        Some('h') => (&s[..s.len() - 1], 3_600_000),
        Some('m') => (&s[..s.len() - 1], 60_000),
        Some('s') => (&s[..s.len() - 1], 1_000),
        _ => (s, 1_000),
    };
    n.parse::<i64>()
        .map(|v| v * mult)
        .map_err(|_| format!("`{s}` no es una edad: `7d`, `12h`, `30m`, `45s` o `0`"))
}

/// **El mantenimiento**: por cada puntero con dataset, expirar lo superado
/// (más viejo que `--edad`) y retirar lo que ningún snapshot nombra; si expiró
/// algo hay un `metadata.json` nuevo y **el puntero se mueve**. Y al final lo
/// que ningún puntero reclama: datasets sin puntero y sobres heredados que
/// ningún puntero nombra ya.
fn recoger(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    let edad = match op.edad {
        Some(e) => Some(edad_ms(e).map_err(|m| (64, m))?),
        None => None,
    };
    let ps = punteros(path, &dir_copias(path, op));
    let mut movidos = 0usize;
    let mut expirados = 0i64;
    let mut ficheros = 0i64;
    let mut datasets = Vec::new();
    let mut claves = Vec::new();
    let mut lineas = Vec::new();
    for p in &ps {
        datasets.push(Json::s(p.dataset()));
        if let Some(c) = p.campo("clave") {
            claves.push(Json::s(c));
        }
        let Some(ml) = p.campo("metadata_location") else {
            continue;
        };
        let mut pet = vec![
            ("dataset", Json::s(p.dataset())),
            ("metadata_location", Json::s(&ml)),
        ];
        if let Some(e) = edad {
            pet.push(("edad_ms", Json::s(e.to_string())));
        }
        let g = almacen(
            if op.seco { "recoger-seco" } else { "recoger" },
            &Json::obj(pet),
        )?;
        let n = |k: &str| {
            campo_de(&g, k)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0)
        };
        expirados += n("expirados");
        ficheros += n("ficheros");
        let nueva = campo_de(&g, "metadata_location").unwrap_or_default();
        let movido = !op.seco && !nueva.is_empty() && nueva != ml;
        if movido {
            let mut m = match Json::de_node(&p.nodo) {
                Json::Obj(m) => m,
                _ => Default::default(),
            };
            m.insert("metadata_location".into(), Json::s(&nueva));
            std::fs::write(&p.ruta, Json::Obj(m).pretty() + "\n").map_err(|e| {
                (
                    73,
                    format!("no se pudo escribir `{}`: {e}", p.ruta.display()),
                )
            })?;
            movidos += 1;
        }
        lineas.push(Json::obj([
            ("nombre", Json::s(&p.nombre)),
            ("expirados", Json::Int(n("expirados"))),
            ("ficheros", Json::Int(n("ficheros"))),
            ("movido", Json::Bool(movido)),
        ]));
    }
    let h = almacen(
        "recoger-huerfanas",
        &Json::obj([
            ("datasets", Json::Arr(datasets)),
            ("claves", Json::Arr(claves)),
            ("seco", Json::Bool(op.seco)),
        ]),
    )?;
    let hn = |k: &str| {
        campo_de(&h, k)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let resumen = Json::obj([
        ("datasets", Json::Int(ps.len() as i64)),
        ("expirados", Json::Int(expirados)),
        ("ficheros", Json::Int(ficheros)),
        ("punteros_movidos", Json::Int(movidos as i64)),
        ("huerfanos", Json::Int(hn("huerfanos"))),
        ("heredados", Json::Int(hn("heredados"))),
        ("seco", Json::Bool(op.seco)),
        // -1: sin `--edad`; rige la retención de cada tabla (0031 §11 ⑥)
        ("edad_ms", Json::Int(edad.unwrap_or(-1))),
        ("por_dataset", Json::Arr(lineas)),
    ]);
    if op.json {
        println!("{}", resumen.jcs());
    } else {
        println!(
            "{}{} datasets · {expirados} snapshot(s) expirado(s) · {ficheros} fichero(s) que nadie nombraba · {movidos} puntero(s) movido(s) · huérfanos: {} dataset(s) y {} objeto(s) heredado(s)",
            if op.seco { "en seco · " } else { "" },
            ps.len(),
            hn("huerfanos"),
            hn("heredados")
        );
    }
    Ok(())
}

// 0033: el lago ya no es un `datasource` del manifiesto. Un dataset es nuestro
// y sus caras se saben; lo que `LAGO_URL` dice —la raíz del bucket— lo sabe
// todo pod, y no hay ninguna `Table` que lo nombre.

fn bien(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

/// `<paquete>.<tabla>` → (`paquete`, `tabla`), o por qué no.
fn partes(nombre: &str) -> Result<(&str, &str), Fallo> {
    let Some((ns, tabla)) = nombre.split_once('.') else {
        return Err((64, format!("`{nombre}` no es `<paquete>.<tabla>`")));
    };
    if !bien(ns) || !bien(tabla) {
        return Err((
            64,
            format!("`{nombre}`: paquete y tabla llevan letras, dígitos y `_`"),
        ));
    }
    Ok((ns, tabla))
}

/// La marca de los documentos que este verbo escribe: sólo esos se regeneran
/// cuando el esquema de la tabla evoluciona; uno escrito a mano se respeta.
const MARCA: &str = "# Un dataset escrito (0033)";

/// Qué hay del documento del dataset: `Ok(None)` si no existe, `Ok(Some(texto))`
/// si es un dataset escrito, `Err` si con ese nombre hay otra cosa.
fn documento_del_dataset(path: &Path, ns: &str, tabla: &str) -> Result<Option<String>, Fallo> {
    let pkg = path.join("packages").join(ns);
    // Una View o una Table con ese nombre: una consulta no se escribe, y a lo
    // que es de otro no se le escribe.
    if pkg.join("views").join(format!("{tabla}.yaml")).is_file() {
        return Err((
            65,
            format!(
                "`{ns}.{tabla}` es una View: una consulta no se escribe; escribe en un dataset"
            ),
        ));
    }
    if pkg.join("tables").join(format!("{tabla}.yaml")).is_file() {
        return Err((
            65,
            format!(
                "`{ns}.{tabla}` es una Table: apunta a lo que es de otro, y a eso no se escribe; escribe en un dataset"
            ),
        ));
    }
    let doc = pkg.join("datasets").join(format!("{tabla}.yaml"));
    let Some(t) = std::fs::read_to_string(&doc).ok() else {
        return Ok(None);
    };
    let n = ore_core::parse::parse(&t)
        .map_err(|e| (65, format!("`{}` no analiza: {e:?}", doc.display())))?;
    if n.get("spec").is_some_and(|(_, s)| s.get("from").is_some()) {
        return Err((
            65,
            format!(
                "`{ns}.{tabla}` es un dataset mantenido: lo cumple el sistema desde `from`, y no se escribe por debajo"
            ),
        ));
    }
    Ok(Some(t))
}

/// El `owner` de un paquete (`packages/<ns>/package.yaml`), o `team:<ns>`.
fn dueno_del_paquete(path: &Path, ns: &str) -> String {
    std::fs::read_to_string(path.join("packages").join(ns).join("package.yaml"))
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok())
        .and_then(|n| {
            n.get("spec")
                .and_then(|(_, s)| s.get("owner"))
                .and_then(|(_, o)| o.as_str().map(String::from))
        })
        .unwrap_or_else(|| format!("team:{ns}"))
}

/// Las columnas que un documento del lago declara (`spec.columns`).
fn columnas_del_documento(texto: &str) -> BTreeMap<String, String> {
    ore_core::parse::parse(texto)
        .ok()
        .and_then(|n| {
            n.get("spec")
                .and_then(|(_, s)| s.get("columns"))
                .map(|(_, c)| {
                    c.entries()
                        .iter()
                        .filter_map(|(k, v)| {
                            Some((
                                k.as_str()?.to_string(),
                                v.get("type")
                                    .and_then(|(_, t)| t.as_str())
                                    .unwrap_or("String")
                                    .to_string(),
                            ))
                        })
                        .collect()
                })
        })
        .unwrap_or_default()
}

/// **El dataset escrito nace, o sigue el esquema de la tabla.** Sin documento:
/// nace con `columnas`. Con documento nuestro y otras columnas: se regenera
/// (el esquema evolucionó con una escritura). Con documento ajeno: se deja, y
/// se dice si difiere. Compila sólo lo que este documento dice; si no compila,
/// se revierte. Devuelve `(nueva, regenerada)`.
fn asegurar_dataset(
    path: &Path,
    ns: &str,
    tabla: &str,
    columnas: &BTreeMap<String, String>,
    clave: Option<&[String]>,
) -> Result<(bool, bool), Fallo> {
    let nombre = format!("{ns}.{tabla}");
    if columnas.is_empty() {
        return Err((64, format!("el dataset `{nombre}` no tiene columnas")));
    }
    for (c, t) in columnas {
        if !bien(c) {
            return Err((64, format!("la columna `{c}` no es un identificador")));
        }
        if ore_core::types::parse_type(t).is_err() {
            return Err((
                64,
                format!("la columna `{c}` tiene un tipo que OOS no conoce: `{t}`"),
            ));
        }
    }
    let doc = path
        .join("packages")
        .join(ns)
        .join("datasets")
        .join(format!("{tabla}.yaml"));
    let texto_previo = documento_del_dataset(path, ns, tabla)?;
    // Lo que `changes` tiene que decir (0033: QUÉ ESCRITURAS ADMITE): `upsert`
    // con su clave si la escritura fue un upsert (y entonces una Entity puede
    // respaldarse de este dataset: OOS2021 no lo permite de uno que «solo
    // anexa»), y si no, lo que diga.
    let cambios = clave
        .filter(|c| !c.is_empty())
        .map(|c| format!("  changes: {{ mode: upsert, key: [{}] }}", c.join(", ")));
    let (nueva, regenerar) = match &texto_previo {
        None => (true, true),
        Some(t) => (
            false,
            t.contains(MARCA)
                && (columnas_del_documento(t) != *columnas
                    || cambios.as_ref().is_some_and(|c| !t.contains(c.trim()))),
        ),
    };
    if !regenerar {
        return Ok((false, false));
    }
    // Con documento: se edita, para no perder lo que alguien le añadió a mano
    // (medido: `labels` y `description` se perdían al evolucionar el esquema).
    // Sin documento, o si el de antes no se deja editar: desde cero.
    let mut s = match texto_previo
        .as_deref()
        .and_then(|t| seguir_esquema(t, columnas))
    {
        Some(s) => s,
        None => documento_nuevo(&dueno_del_paquete(path, ns), ns, tabla, columnas),
    };
    if let Some(c) = &cambios {
        s = con_cambios(&s, c);
    }
    if let Some(padre) = doc.parent() {
        std::fs::create_dir_all(padre)
            .map_err(|e| (73, format!("no se pudo crear `{}`: {e}", padre.display())))?;
    }
    std::fs::write(&doc, s)
        .map_err(|e| (73, format!("no se pudo escribir `{}`: {e}", doc.display())))?;
    // ¿Compila lo que se escribió? Sólo los diagnósticos de ESTE documento:
    // un paquete vecino roto no es de esta escritura.
    let malos: Vec<String> = ore_core::validate_package(path)
        .into_iter()
        .filter(|d| d.file == doc)
        .map(|d| d.render(path))
        .collect();
    if !malos.is_empty() {
        match &texto_previo {
            Some(t) => {
                let _ = std::fs::write(&doc, t);
            }
            None => {
                let _ = std::fs::remove_file(&doc);
            }
        }
        return Err((
            65,
            format!(
                "el dataset `{nombre}` no compila con esas columnas:\n{}",
                malos.join("\n")
            ),
        ));
    }
    Ok((nueva, !nueva))
}

/// El dataset escrito desde cero: lo que `write()` sabe de él.
fn documento_nuevo(
    owner: &str,
    ns: &str,
    tabla: &str,
    columnas: &BTreeMap<String, String>,
) -> String {
    let mut s = format!(
        "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: {{ name: {tabla}, namespace: {ns} }}\n{MARCA}: lo escribió `write()` desde un puesto, y este\n# documento sigue el esquema de la tabla Iceberg (nació con la primera escritura\n# y sus columnas siguen al esquema cuando evoluciona; lo demás que se le\n# añada se conserva). Su puntero es `datasets/{ns}_{tabla}.json`; su\n# historia, los snapshots de la tabla; su linaje, la procedencia del puntero.\nspec:\n  owner: {owner}\n  columns:\n"
    );
    for (c, t) in columnas {
        s.push_str(&format!("    {c}: {{ type: {t} }}\n"));
    }
    s.push_str("  changes: { mode: append }\n");
    s
}

/// **El documento sigue el esquema sin perder lo demás.** Se edita el texto
/// por posiciones (las que el analizador da): a una columna que cambió de
/// tipo se le cambia sólo el tipo, una columna nueva se añade al final del
/// bloque `columns`, y una que ya no está se quita con sus líneas. Lo que no
/// es `columns` —`metadata.description`, `labels` en una columna que sigue,
/// `reads`, comentarios— queda como estaba. `None` si el documento no tiene
/// la forma esperada (y entonces se escribe desde cero).
fn seguir_esquema(texto: &str, columnas: &BTreeMap<String, String>) -> Option<String> {
    let n = ore_core::parse::parse(texto).ok()?;
    let (_, spec) = n.get("spec")?;
    let (clave_cols, cols) = spec.get("columns")?;
    let mut lineas: Vec<String> = texto.lines().map(String::from).collect();
    // Dónde acaba el bloque de columnas (1-based, exclusivo): la entrada de
    // `spec` que sigue a `columns`, o el final del documento.
    let fin = spec
        .entries()
        .iter()
        .map(|(k, _)| k.pos().line)
        .filter(|l| *l > clave_cols.pos().line)
        .min()
        .unwrap_or(lineas.len() + 1);
    // Las columnas del documento, en el orden del texto: nombre, línea de la
    // clave, y dónde está su `type` (línea, columna, largo) si lo tiene.
    struct Col {
        nombre: String,
        linea: usize,
        tipo: Option<(usize, usize, usize)>,
        vacia: bool,
    }
    let mut doc: Vec<Col> = cols
        .entries()
        .iter()
        .filter_map(|(k, v)| {
            Some(Col {
                nombre: k.as_str()?.to_string(),
                linea: k.pos().line,
                tipo: v.get("type").and_then(|(_, t)| {
                    Some((t.pos().line, t.pos().col, t.as_str()?.chars().count()))
                }),
                vacia: v.entries().is_empty(),
            })
        })
        .collect();
    doc.sort_by_key(|c| c.linea);
    if doc
        .iter()
        .any(|c| c.linea < clave_cols.pos().line || c.linea >= fin)
    {
        return None;
    }
    let sangria = doc
        .first()
        .map(|c| c.linea)
        .and_then(|l| lineas.get(l - 1))
        .map(|l| l.len() - l.trim_start().len())
        .unwrap_or(4);
    // Cada edición nombra líneas 1-based; se aplican de abajo arriba para que
    // las de arriba sigan valiendo.
    enum Edicion {
        Quitar(usize, usize),
        Linea(usize, String),
        Tipo(usize, usize, usize, String),
        Insertar(usize, Vec<String>),
    }
    let mut ediciones = Vec::new();
    for (i, c) in doc.iter().enumerate() {
        let siguiente = doc.get(i + 1).map(|d| d.linea).unwrap_or(fin);
        match columnas.get(&c.nombre) {
            None => ediciones.push(Edicion::Quitar(c.linea, siguiente)),
            Some(t) => match c.tipo {
                Some((l, col, largo)) => ediciones.push(Edicion::Tipo(l, col, largo, t.clone())),
                None if c.vacia => ediciones.push(Edicion::Linea(
                    c.linea,
                    format!("{}{}: {{ type: {t} }}", " ".repeat(sangria), c.nombre),
                )),
                // Un mapa con otras claves y sin `type`: se le pone la suya.
                None => {
                    let s2 = lineas
                        .get(c.linea)
                        .map(|l| l.len() - l.trim_start().len())
                        .unwrap_or(sangria + 2);
                    ediciones.push(Edicion::Insertar(
                        c.linea + 1,
                        vec![format!("{}type: {t}", " ".repeat(s2))],
                    ));
                }
            },
        }
    }
    let nuevas: Vec<String> = columnas
        .iter()
        .filter(|(c, _)| !doc.iter().any(|d| &d.nombre == *c))
        .map(|(c, t)| format!("{}{c}: {{ type: {t} }}", " ".repeat(sangria)))
        .collect();
    if !nuevas.is_empty() {
        ediciones.push(Edicion::Insertar(fin, nuevas));
    }
    let linea_de = |e: &Edicion| match e {
        Edicion::Quitar(l, _)
        | Edicion::Linea(l, _)
        | Edicion::Tipo(l, _, _, _)
        | Edicion::Insertar(l, _) => *l,
    };
    ediciones.sort_by_key(|e| std::cmp::Reverse(linea_de(e)));
    for e in ediciones {
        match e {
            Edicion::Quitar(desde, hasta) => {
                lineas.drain(desde - 1..(hasta - 1).min(lineas.len()));
            }
            Edicion::Linea(l, s) => lineas[l - 1] = s,
            Edicion::Tipo(l, col, largo, t) => {
                let linea = &lineas[l - 1];
                let antes: String = linea.chars().take(col - 1).collect();
                let despues: String = linea.chars().skip(col - 1 + largo).collect();
                lineas[l - 1] = format!("{antes}{t}{despues}");
            }
            Edicion::Insertar(l, vs) => {
                let en = (l - 1).min(lineas.len());
                for (i, v) in vs.into_iter().enumerate() {
                    lineas.insert(en + i, v);
                }
            }
        }
    }
    let mut out = lineas.join("\n");
    out.push('\n');
    Some(out)
}

/// La línea `changes:` de `spec`, sustituida (con el bloque que tuviera
/// debajo, si iba en varias líneas); si no la hay, se añade al final de `spec`.
fn con_cambios(texto: &str, linea: &str) -> String {
    let mut lineas: Vec<String> = texto.lines().map(String::from).collect();
    if let Some(i) = lineas.iter().position(|l| l.starts_with("  changes:")) {
        let mut fin = i + 1;
        while fin < lineas.len()
            && (lineas[fin].trim().is_empty()
                || lineas[fin].len() - lineas[fin].trim_start().len() > 2)
        {
            fin += 1;
        }
        lineas.splice(i..fin, [linea.to_string()]);
    } else {
        lineas.push(linea.to_string());
    }
    let mut out = lineas.join("\n");
    out.push('\n');
    out
}

/// El puntero de un dataset, leído del árbol (`datasets/<ns>_<t>.json`).
fn puntero_del_lago(path: &Path, ns: &str, tabla: &str) -> (PathBuf, Option<Node>) {
    let ruta = path.join("datasets").join(format!("{ns}_{tabla}.json"));
    let previo = std::fs::read_to_string(&ruta)
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok());
    (ruta, previo)
}

/// Escribe el puntero: lo que había, con el estado nuevo encima.
fn escribir_puntero(
    ruta: &Path,
    previo: Option<&Node>,
    campos: Vec<(&str, Json)>,
) -> Result<(), Fallo> {
    if let Some(padre) = ruta.parent() {
        std::fs::create_dir_all(padre)
            .map_err(|e| (73, format!("no se pudo crear `{}`: {e}", padre.display())))?;
    }
    let mut m: BTreeMap<String, Json> = match previo.map(Json::de_node) {
        Some(Json::Obj(m)) => m,
        _ => Default::default(),
    };
    for (k, v) in campos {
        m.insert(k.into(), v);
    }
    m.remove("motivo");
    std::fs::write(ruta, Json::Obj(m).pretty() + "\n")
        .map_err(|e| (73, format!("no se pudo escribir `{}`: {e}", ruta.display())))
}

/// El cuerpo de `--peticion`: JSON tal cual, `@fichero`, o `-` por stdin.
fn cuerpo_de(op: &Opciones) -> Result<String, Fallo> {
    let p = op.peticion.filter(|s| !s.trim().is_empty()).ok_or((
        64,
        "falta `--peticion`: el cuerpo (JSON, `@fichero` o `-`)".to_string(),
    ))?;
    let texto = if p == "-" {
        let mut t = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut t)
            .map_err(|e| (66, format!("no se pudo leer la petición de stdin: {e}")))?;
        t
    } else if let Some(f) = p.strip_prefix('@') {
        std::fs::read_to_string(f).map_err(|e| (66, format!("no se pudo leer `{f}`: {e}")))?
    } else {
        p.to_string()
    };
    if ore_core::parse::parse(&texto).is_err() {
        return Err((64, "la petición no es JSON".into()));
    }
    Ok(texto.trim().to_string())
}

/// `ore-store <verbo>` con una petición ya escrita (no se reanaliza) y la
/// respuesta tal cual.
fn almacen_crudo(verbo: &str, peticion: &str) -> Result<String, Fallo> {
    let programa = programa_del_almacen().map_err(|e| (78, e))?;
    lector::ejecutar(&programa, &[verbo.to_string()], Some(peticion)).map_err(|f| {
        let mut s = f.mensaje;
        for l in f.ayuda {
            s.push('\n');
            s.push_str(&l);
        }
        (69, s)
    })
}

/// Un texto como literal JSON.
fn lit(s: &str) -> String {
    Json::s(s).jcs()
}

/// Las propiedades de retención por defecto, como JSON, si `--retencion-defecto`.
fn retencion_defecto(op: &Opciones) -> Result<String, Fallo> {
    match op.retencion_defecto {
        Some(e) => {
            let ms = edad_ms(e).map_err(|m| (64, m))?;
            Ok(format!(
                "{{\"history.expire.max-snapshot-age-ms\":\"{ms}\",\"history.expire.min-snapshots-to-keep\":\"1\"}}"
            ))
        }
        None => Ok("{}".into()),
    }
}

/// Lo que un cambio (un `updateTable`, o una entrada de `table-changes`) dice
/// de sí mismo: a qué tabla va, si crea, y su clave de operación.
struct Cambio<'a> {
    nodo: &'a Node,
    indice: Option<usize>,
}

impl Cambio<'_> {
    fn identificador(&self) -> Option<String> {
        let id = self.nodo.get("identifier").map(|(_, v)| v)?;
        let ns = id
            .get("namespace")
            .map(|(_, v)| v.items())
            .and_then(|i| i.last())
            .and_then(|n| n.as_str())?;
        let n = id.get("name").and_then(|(_, v)| v.as_str())?;
        Some(format!("{ns}.{n}"))
    }
    fn crea(&self) -> bool {
        self.nodo
            .get("requirements")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .any(|r| r.get("type").and_then(|(_, v)| v.as_str()) == Some("assert-create"))
    }
    fn operacion(&self) -> Option<String> {
        self.nodo
            .get("updates")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter(|u| u.get("action").and_then(|(_, v)| v.as_str()) == Some("add-snapshot"))
            .filter_map(|u| {
                u.get("snapshot")
                    .and_then(|(_, s)| s.get("summary"))
                    .and_then(|(_, m)| m.get("ore.operacion"))
                    .and_then(|(_, v)| v.as_str())
                    .filter(|c| !c.is_empty())
                    .map(String::from)
            })
            .next_back()
    }
    /// La procedencia (`ore.procedencia` en el resumen del snapshot, JSON):
    /// de qué salió lo escrito, según quien lo escribió (W3.7 ③).
    fn procedencia(&self) -> Option<Json> {
        self.nodo
            .get("updates")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter(|u| u.get("action").and_then(|(_, v)| v.as_str()) == Some("add-snapshot"))
            .filter_map(|u| {
                let m = u.get("snapshot").and_then(|(_, s)| s.get("summary"))?.1;
                let p = m.get("ore.procedencia").and_then(|(_, v)| v.as_str())?;
                let n = ore_core::parse::parse(p).ok()?;
                match Json::de_node(&n) {
                    j @ Json::Obj(_) => Some(j),
                    _ => None,
                }
            })
            .next_back()
    }
    /// La clave del upsert (`ore.clave` en el resumen del snapshot que lo hizo).
    fn clave(&self) -> Option<Vec<String>> {
        self.nodo
            .get("updates")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter(|u| u.get("action").and_then(|(_, v)| v.as_str()) == Some("add-snapshot"))
            .filter_map(|u| {
                let m = u.get("snapshot").and_then(|(_, s)| s.get("summary"))?.1;
                if m.get("ore.modo").and_then(|(_, v)| v.as_str()) != Some("upsert") {
                    return None;
                }
                let c = m.get("ore.clave").and_then(|(_, v)| v.as_str())?;
                Some(
                    c.split(',')
                        .filter(|x| !x.is_empty())
                        .map(String::from)
                        .collect(),
                )
            })
            .next_back()
    }
}

/// Lo que `ore-store aplicar` contestó, ya como JSON del núcleo más lo que
/// hace falta para el puntero.
struct Aplicado {
    metadata_location: String,
    snapshot: String,
    filas: i64,
    uuid: String,
    operacion: String,
    columnas_oos: BTreeMap<String, String>,
}

fn aplicado_de(n: &Node) -> Aplicado {
    Aplicado {
        metadata_location: campo_de(n, "metadata_location").unwrap_or_default(),
        snapshot: campo_de(n, "snapshot").unwrap_or_default(),
        filas: campo_de(n, "filas")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        uuid: campo_de(n, "uuid").unwrap_or_default(),
        operacion: campo_de(n, "operacion").unwrap_or_default(),
        columnas_oos: n
            .get("columnas_oos")
            .map(|(_, c)| {
                c.entries()
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// El CAS perdido, dicho con `actual` y con código 75.
fn adelantado(op: &Opciones, tabla: &str, actual_ml: &str, actual_snap: &str, m: String) -> Fallo {
    if op.json {
        println!(
            "{}",
            Json::obj([
                (
                    "actual",
                    Json::obj([
                        ("metadata_location", Json::s(actual_ml)),
                        ("snapshot", Json::s(actual_snap)),
                    ]),
                ),
                ("error", Json::s(&m)),
                ("tabla", Json::s(tabla)),
            ])
            .jcs()
        );
    }
    eprintln!("error: {m}");
    (ADELANTADO, String::new())
}

/// **El commit del catálogo REST** (ver la cabecera). Uno o varios cambios;
/// se aplican todos antes de mover ningún puntero.
fn commit(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    let texto = cuerpo_de(op)?;
    let n = ore_core::parse::parse(&texto)
        .map_err(|e| (64, format!("la petición no analiza: {e:?}")))?;
    let cambios: Vec<Cambio> = match n.get("table-changes") {
        Some((_, t)) => t
            .items()
            .iter()
            .enumerate()
            .map(|(i, c)| Cambio {
                nodo: c,
                indice: Some(i),
            })
            .collect(),
        None => vec![Cambio {
            nodo: &n,
            indice: None,
        }],
    };
    if cambios.is_empty() {
        return Err((64, "la petición no trae ningún cambio".into()));
    }
    let defecto = retencion_defecto(op)?;

    // ── primero, todos: ¿a qué tabla, existe, cuál es la base, ya se hizo? ──
    struct Plan<'a> {
        cambio: Cambio<'a>,
        nombre: String,
        ns: String,
        tabla: String,
        ruta: PathBuf,
        previo: Option<Node>,
        base: String,
        repetida: bool,
    }
    let mut planes = Vec::new();
    for c in cambios {
        let nombre = c
            .identificador()
            .or_else(|| op.tabla.map(String::from))
            .ok_or((
                64,
                "el cambio no trae `identifier` y no se dio `--tabla <paquete>.<tabla>`"
                    .to_string(),
            ))?;
        let (ns, tabla) = {
            let (a, b) = partes(&nombre)?;
            (a.to_string(), b.to_string())
        };
        let (ns, tabla) = (ns.as_str(), tabla.as_str());
        if !path
            .join("packages")
            .join(ns)
            .join("package.yaml")
            .is_file()
        {
            return Err((65, format!("no hay ningún paquete `{ns}` en el árbol")));
        }
        documento_del_dataset(path, ns, tabla)?;
        let (ruta, previo) = puntero_del_lago(path, ns, tabla);
        let base = previo
            .as_ref()
            .and_then(|p| campo_de(p, "metadata_location"))
            .unwrap_or_default();
        let snap = previo
            .as_ref()
            .and_then(|p| campo_de(p, "snapshot"))
            .unwrap_or_default();
        if c.crea() && !base.is_empty() {
            return Err(adelantado(
                op,
                &nombre,
                &base,
                &snap,
                format!("`{nombre}` ya existe (`{base}`) y el cambio dice `assert-create`"),
            ));
        }
        if !c.crea() && base.is_empty() {
            return Err((
                65,
                format!("no hay ningún dataset `{nombre}`: la tabla no existe todavía"),
            ));
        }
        // La clave de operación en la ancestría: la misma escritura otra vez
        // (la celda reejecutada, el reintento tras un 5xx) no deja snapshot.
        let repetida = match (c.operacion(), base.is_empty()) {
            (Some(clave), false) => {
                let h = almacen(
                    "historia",
                    &Json::obj([
                        ("dataset", Json::s(format!("datasets/{ns}_{tabla}"))),
                        ("metadata_location", Json::s(&base)),
                    ]),
                )?;
                h.get("snapshots")
                    .map(|(_, v)| v.items())
                    .unwrap_or(&[])
                    .iter()
                    .any(|s| campo_de(s, "idempotencia").as_deref() == Some(clave.as_str()))
            }
            _ => false,
        };
        planes.push(Plan {
            cambio: c,
            nombre,
            ns: ns.to_string(),
            tabla: tabla.to_string(),
            ruta,
            previo,
            base,
            repetida,
        });
    }

    // ── después, aplicar cada uno (los `metadata.json`), sin mover nada ─────
    let mut aplicados: Vec<Option<Aplicado>> = Vec::new();
    for p in &planes {
        if p.repetida {
            aplicados.push(None);
            continue;
        }
        let pet = format!(
            "{{\"dataset\":{},\"metadata_location\":{},\"peticion\":{texto},\"cambio\":{},\"retencion_defecto\":{defecto}}}",
            lit(&format!("datasets/{}_{}", p.ns, p.tabla)),
            lit(&p.base),
            p.cambio
                .indice
                .map(|i| i.to_string())
                .unwrap_or_else(|| "null".into()),
        );
        let salida = match almacen_crudo("aplicar", &pet) {
            Ok(s) => s,
            Err((_, m)) if m.contains("conflicto") => {
                let snap = p
                    .previo
                    .as_ref()
                    .and_then(|x| campo_de(x, "snapshot"))
                    .unwrap_or_default();
                return Err(adelantado(
                    op,
                    &p.nombre,
                    &p.base,
                    &snap,
                    format!(
                        "el puntero de `{}` ya no es la base del cambio: alguien escribió mientras tanto ({}). Hay que volver a leer y escribir sobre lo que hay ahora",
                        p.nombre,
                        m.trim_start_matches("error: ")
                    ),
                ));
            }
            Err(e) => return Err(e),
        };
        let n = ore_core::parse::parse(&salida).map_err(|e| {
            (
                69,
                format!("lo que devolvió `ore-store aplicar` no analiza: {e:?}"),
            )
        })?;
        aplicados.push(Some(aplicado_de(&n)));
    }

    // ── y al final, la `Table` y el puntero de cada uno ─────────────────────
    let mut lineas = Vec::new();
    for (p, a) in planes.iter().zip(aplicados) {
        let Some(a) = a else {
            // la operación ya estaba: lo que hay, sin tocar nada
            let snap = p
                .previo
                .as_ref()
                .and_then(|x| campo_de(x, "snapshot"))
                .unwrap_or_default();
            let filas = p
                .previo
                .as_ref()
                .and_then(|x| campo_de(x, "filas"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            lineas.push(Json::obj([
                ("dataset", Json::s(format!("datasets/{}_{}", p.ns, p.tabla))),
                ("filas", Json::Int(filas)),
                ("metadata_location", Json::s(&p.base)),
                (
                    "operacion",
                    Json::s(p.cambio.operacion().unwrap_or_default()),
                ),
                ("puntero_nuevo", Json::Bool(false)),
                ("repetida", Json::Bool(true)),
                ("snapshot", Json::s(snap)),
                ("tabla", Json::s(&p.nombre)),
                ("tabla_nueva", Json::Bool(false)),
                ("tabla_regenerada", Json::Bool(false)),
            ]));
            continue;
        };
        let clave = p.cambio.clave();
        let (tabla_nueva, regenerada) =
            asegurar_dataset(path, &p.ns, &p.tabla, &a.columnas_oos, clave.as_deref())?;
        let mut campos = vec![
            ("estado", Json::s("copiada")),
            ("tabla", Json::s(&p.nombre)),
            ("dataset", Json::s(format!("datasets/{}_{}", p.ns, p.tabla))),
            ("metadata_location", Json::s(&a.metadata_location)),
            ("snapshot", Json::s(&a.snapshot)),
            ("filas", Json::Int(a.filas)),
            ("uuid", Json::s(&a.uuid)),
            ("operacion", Json::s(&a.operacion)),
        ];
        if let Some(s) = op.sujeto {
            campos.push(("escrito_por", Json::s(s)));
        }
        // La procedencia de ESTE snapshot; una escritura sin ella deja la de
        // antes, que sigue siendo lo último que alguien dijo del dataset.
        if let Some(pr) = p.cambio.procedencia() {
            campos.push(("procedencia", pr));
        }
        escribir_puntero(&p.ruta, p.previo.as_ref(), campos)?;
        lineas.push(Json::obj([
            ("dataset", Json::s(format!("datasets/{}_{}", p.ns, p.tabla))),
            ("filas", Json::Int(a.filas)),
            ("metadata_location", Json::s(&a.metadata_location)),
            ("operacion", Json::s(&a.operacion)),
            ("puntero_nuevo", Json::Bool(p.previo.is_none())),
            ("repetida", Json::Bool(false)),
            ("snapshot", Json::s(&a.snapshot)),
            ("tabla", Json::s(&p.nombre)),
            ("tabla_nueva", Json::Bool(tabla_nueva)),
            ("tabla_regenerada", Json::Bool(regenerada)),
        ]));
    }
    let j = Json::obj([("tablas", Json::Arr(lineas.clone()))]);
    if op.json {
        println!("{}", j.jcs());
    } else {
        for l in &lineas {
            let g = |k: &str| match l {
                Json::Obj(m) => match m.get(k) {
                    Some(Json::Str(s)) => s.clone(),
                    Some(Json::Bool(b)) => b.to_string(),
                    Some(Json::Int(i)) => i.to_string(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            println!(
                "{} · {}{} · {} filas\n  {}",
                g("tabla"),
                if g("repetida") == "true" {
                    "ya estaba (misma operación)"
                } else if g("puntero_nuevo") == "true" {
                    "puntero nuevo"
                } else {
                    "puntero movido"
                },
                if g("tabla_nueva") == "true" {
                    " · el dataset nace"
                } else if g("tabla_regenerada") == "true" {
                    " · el dataset sigue el esquema nuevo"
                } else {
                    ""
                },
                g("filas"),
                g("metadata_location")
            );
        }
    }
    Ok(())
}

/// **La tabla nace de un `createTable`** (sin `stage-create`): el `metadata.json`
/// v0 por `ore-store aplicar` con `crear`, la `Table` con sus columnas, el
/// puntero. Es lo que un `POST /v1/namespaces/{ns}/tables` hace.
fn crear(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let (ns, tabla) = partes(nombre)?;
    let texto = cuerpo_de(op)?;
    if !path
        .join("packages")
        .join(ns)
        .join("package.yaml")
        .is_file()
    {
        return Err((65, format!("no hay ningún paquete `{ns}` en el árbol")));
    }
    documento_del_dataset(path, ns, tabla)?;
    let (ruta, previo) = puntero_del_lago(path, ns, tabla);
    if let Some(p) = &previo
        && let Some(base) = campo_de(p, "metadata_location")
    {
        let snap = campo_de(p, "snapshot").unwrap_or_default();
        return Err(adelantado(
            op,
            nombre,
            &base,
            &snap,
            format!("`{nombre}` ya existe: `{base}`"),
        ));
    }
    let defecto = retencion_defecto(op)?;
    let pet = format!(
        "{{\"dataset\":{},\"crear\":true,\"peticion\":{texto},\"retencion_defecto\":{defecto}}}",
        lit(&format!("datasets/{ns}_{tabla}"))
    );
    let salida = almacen_crudo("aplicar", &pet)?;
    let n = ore_core::parse::parse(&salida).map_err(|e| {
        (
            69,
            format!("lo que devolvió `ore-store aplicar` no analiza: {e:?}"),
        )
    })?;
    let a = aplicado_de(&n);
    let (tabla_nueva, _) = asegurar_dataset(path, ns, tabla, &a.columnas_oos, None)?;
    let mut campos = vec![
        ("estado", Json::s("copiada")),
        ("tabla", Json::s(nombre)),
        ("dataset", Json::s(format!("datasets/{ns}_{tabla}"))),
        ("metadata_location", Json::s(&a.metadata_location)),
        ("snapshot", Json::s(&a.snapshot)),
        ("filas", Json::Int(a.filas)),
        ("uuid", Json::s(&a.uuid)),
    ];
    if let Some(s) = op.sujeto {
        campos.push(("escrito_por", Json::s(s)));
    }
    escribir_puntero(&ruta, previo.as_ref(), campos)?;
    let j = Json::obj([
        ("dataset", Json::s(format!("datasets/{ns}_{tabla}"))),
        ("metadata_location", Json::s(&a.metadata_location)),
        ("puntero_nuevo", Json::Bool(true)),
        ("tabla", Json::s(nombre)),
        ("tabla_nueva", Json::Bool(tabla_nueva)),
        ("uuid", Json::s(&a.uuid)),
    ]);
    if op.json {
        println!("{}", j.jcs());
    } else {
        println!(
            "{nombre} · nace{}\n  {}",
            if tabla_nueva {
                " · el dataset nace"
            } else {
                ""
            },
            a.metadata_location
        );
    }
    Ok(())
}

/// La credencial prestada para `datasets/<ns>_<tabla>`, como los dos campos
/// del `LoadTableResult` (`config` y `storage-credentials`), ya escritos.
fn prestamo(ns: &str, tabla: &str, op: &Opciones) -> Result<String, Fallo> {
    if !op.prestar {
        return Ok("\"config\":{}".into());
    }
    let salida = almacen_crudo(
        "prestar",
        &format!("{{\"dataset\":{}}}", lit(&format!("datasets/{ns}_{tabla}"))),
    )?;
    let n = ore_core::parse::parse(&salida).map_err(|e| {
        (
            69,
            format!("lo que devolvió `ore-store prestar` no analiza: {e:?}"),
        )
    })?;
    let config = n
        .get("config")
        .map(|(_, c)| Json::de_node(c).jcs())
        .unwrap_or_else(|| "{}".into());
    let prefijo = campo_de(&n, "prefijo").unwrap_or_default();
    Ok(format!(
        "\"config\":{config},\"storage-credentials\":[{{\"prefix\":{},\"config\":{config}}}]",
        lit(&prefijo)
    ))
}

/// **`loadTable`**: el `LoadTableResult` de la spec REST, tal cual —el
/// `metadata.json` no se reanaliza: lleva `null` y números que el JSON de
/// `ore` no modela—, con la credencial prestada si se pide.
fn cargar(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let (ns, tabla) = partes(nombre)?;
    documento_del_dataset(path, ns, tabla)?;
    let (_, previo) = puntero_del_lago(path, ns, tabla);
    let ml = previo
        .as_ref()
        .and_then(|p| campo_de(p, "metadata_location"))
        .ok_or((65, format!("no hay ningún dataset `{nombre}`")))?;
    let metadatos = almacen_crudo(
        "metadatos",
        &format!("{{\"metadata_location\":{}}}", lit(&ml)),
    )?;
    let cred = prestamo(ns, tabla, op)?;
    println!(
        "{{\"metadata-location\":{},\"metadata\":{},{cred}}}",
        lit(&ml),
        metadatos.trim()
    );
    Ok(())
}

/// **`stage-create`**: los metadatos que la tabla tendría, para que el cliente
/// escriba sus ficheros; nace en el commit con `assert-create`. No toca el
/// árbol ni el bucket; la respuesta de `ore-store` se imprime tal cual.
fn esbozar(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let (ns, tabla) = partes(nombre)?;
    let texto = cuerpo_de(op)?;
    if !path
        .join("packages")
        .join(ns)
        .join("package.yaml")
        .is_file()
    {
        return Err((65, format!("no hay ningún paquete `{ns}` en el árbol")));
    }
    documento_del_dataset(path, ns, tabla)?;
    let (_, previo) = puntero_del_lago(path, ns, tabla);
    if let Some(p) = &previo
        && let Some(base) = campo_de(p, "metadata_location")
    {
        let snap = campo_de(p, "snapshot").unwrap_or_default();
        return Err(adelantado(
            op,
            nombre,
            &base,
            &snap,
            format!("`{nombre}` ya existe: `{base}`"),
        ));
    }
    let pet = format!(
        "{{\"dataset\":{},\"peticion\":{texto}}}",
        lit(&format!("datasets/{ns}_{tabla}"))
    );
    let salida = almacen_crudo("esbozar", &pet)?;
    let salida = salida.trim();
    // `{"metadata":…,"ubicacion":…,"uuid":…}` → el `LoadTableResult` sin
    // `metadata-location` (no hay: nace en el commit), con la credencial.
    let fin = salida.rfind(",\"ubicacion\":").ok_or((
        69,
        "lo que devolvió `ore-store esbozar` no tiene la forma esperada".to_string(),
    ))?;
    let cred = prestamo(ns, tabla, op)?;
    println!("{},{cred}}}", &salida[..fin]);
    Ok(())
}

/// **La retención declarada en la tabla**: `history.expire.*` como propiedades,
/// en un commit (`set-properties` con `assert-table-uuid`), y el puntero se
/// mueve. Es lo que `--recoger` obedece.
fn retencion(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let (ns, tabla) = partes(nombre)?;
    let edad = op
        .edad
        .ok_or((
            64,
            "falta `--edad`: cuánta historia se conserva (`30d`, `0`)".to_string(),
        ))
        .and_then(|e| edad_ms(e).map_err(|m| (64, m)))?;
    let minimo = op.minimo.unwrap_or(1).max(1);
    let (ruta, previo) = puntero_del_lago(path, ns, tabla);
    let base = previo
        .as_ref()
        .and_then(|p| campo_de(p, "metadata_location"))
        .ok_or((65, format!("no hay ningún dataset `{nombre}`")))?;
    let uuid = previo
        .as_ref()
        .and_then(|p| campo_de(p, "uuid"))
        .map(|u| {
            format!(
                ",\"requirements\":[{{\"type\":\"assert-table-uuid\",\"uuid\":{}}}]",
                lit(&u)
            )
        })
        .unwrap_or_default();
    let pet = format!(
        "{{\"dataset\":{},\"metadata_location\":{},\"updates\":[{{\"action\":\"set-properties\",\"updates\":{{\"history.expire.max-snapshot-age-ms\":\"{edad}\",\"history.expire.min-snapshots-to-keep\":\"{minimo}\"}}}}]{uuid}}}",
        lit(&format!("datasets/{ns}_{tabla}")),
        lit(&base),
    );
    let salida = almacen_crudo("aplicar", &pet)?;
    let n = ore_core::parse::parse(&salida).map_err(|e| {
        (
            69,
            format!("lo que devolvió `ore-store aplicar` no analiza: {e:?}"),
        )
    })?;
    let a = aplicado_de(&n);
    escribir_puntero(
        &ruta,
        previo.as_ref(),
        vec![("metadata_location", Json::s(&a.metadata_location))],
    )?;
    let j = Json::obj([
        ("edad_ms", Json::Int(edad)),
        ("metadata_location", Json::s(&a.metadata_location)),
        ("minimo", Json::Int(minimo)),
        ("tabla", Json::s(nombre)),
    ]);
    if op.json {
        println!("{}", j.jcs());
    } else {
        println!(
            "{nombre} · retención: {edad} ms, mínimo {minimo} snapshot(s)\n  {}",
            a.metadata_location
        );
    }
    Ok(())
}

/// **El swap.** Ver la cabecera del módulo.
fn confirmar(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let (ns, tabla) = partes(nombre)?;
    let ml = op.metadata_location.filter(|s| !s.is_empty()).ok_or((
        64,
        "falta `--metadata-location`: el `metadata.json` que el escritor dejó en el bucket"
            .to_string(),
    ))?;
    if !path
        .join("packages")
        .join(ns)
        .join("package.yaml")
        .is_file()
    {
        return Err((65, format!("no hay ningún paquete `{ns}` en el árbol")));
    }

    // ── ¿está en el bucket lo que se apunta? ────────────────────────────────
    let b = almacen("buscar", &Json::obj([("metadata_location", Json::s(ml))]))?;
    if campo_de(&b, "existe").as_deref() != Some("true") {
        return Err((
            65,
            format!("`{ml}` no está en el bucket: no se apunta lo que no existe"),
        ));
    }

    // ── el CAS semántico: el puntero sigue donde el escritor lo dejó ────────
    let (ruta, previo) = puntero_del_lago(path, ns, tabla);
    let actual = previo
        .as_ref()
        .and_then(|p| campo_de(p, "metadata_location"))
        .unwrap_or_default();
    let esperado = op.esperado.unwrap_or_default();
    if actual != esperado {
        let m = if actual.is_empty() {
            format!("el dataset `{nombre}` no tiene puntero todavía y se esperaba `{esperado}`")
        } else if esperado.is_empty() {
            format!(
                "el dataset `{nombre}` ya tiene puntero (`{actual}`) y no se dijo sobre cuál se construyó (`--esperado`)"
            )
        } else {
            format!(
                "el puntero de `{nombre}` ya no es el esperado: alguien lo movió a `{actual}` mientras se escribía. Nada se escribió; hay que volver a leer y escribir sobre lo que hay ahora"
            )
        };
        if op.json {
            println!(
                "{}",
                Json::obj([
                    ("actual", Json::s(&actual)),
                    ("error", Json::s(&m)),
                    ("esperado", Json::s(esperado)),
                ])
                .jcs()
            );
        }
        eprintln!("error: {m}");
        return Err((ADELANTADO, String::new()));
    }
    if actual == ml {
        // El mismo puntero: nada que mover. Idempotente, como el push.
        return salida(op, nombre, ns, tabla, ml, false, false);
    }

    // ── la fuente y el documento ────────────────────────────────────────────
    let columnas: Option<BTreeMap<String, String>> = match op.columnas {
        Some(c) => {
            let n = ore_core::parse::parse(c).map_err(|e| {
                (
                    64,
                    format!("`--columnas` no es un JSON de columna → tipo: {e:?}"),
                )
            })?;
            let m: BTreeMap<String, String> = n
                .entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect();
            if m.is_empty() {
                return Err((64, "`--columnas` está vacío".into()));
            }
            Some(m)
        }
        None => None,
    };
    let hay_doc = documento_del_dataset(path, ns, tabla)?.is_some();
    let tabla_nueva = match (&columnas, hay_doc) {
        (None, false) => {
            return Err((
                65,
                format!(
                    "el dataset `{nombre}` no existe y no se dieron `--columnas`: el documento nace con el esquema de la primera escritura"
                ),
            ));
        }
        (None, true) => false,
        (Some(cols), _) => asegurar_dataset(path, ns, tabla, cols, None)?.0,
    };

    // ── el puntero ──────────────────────────────────────────────────────────
    let mut campos = vec![
        ("estado", Json::s("copiada")),
        ("tabla", Json::s(nombre)),
        ("dataset", Json::s(format!("datasets/{ns}_{tabla}"))),
        ("metadata_location", Json::s(ml)),
        ("snapshot", Json::s(op.snapshot.unwrap_or_default())),
    ];
    if let Some(f) = op.filas {
        campos.push(("filas", Json::Int(f)));
    }
    if let Some(s) = op.sujeto {
        campos.push(("escrito_por", Json::s(s)));
    }
    escribir_puntero(&ruta, previo.as_ref(), campos)?;
    salida(op, nombre, ns, tabla, ml, previo.is_none(), tabla_nueva)
}

#[allow(clippy::too_many_arguments)]
fn salida(
    op: &Opciones,
    nombre: &str,
    ns: &str,
    tabla: &str,
    ml: &str,
    puntero_nuevo: bool,
    tabla_nueva: bool,
) -> Result<(), Fallo> {
    let j = Json::obj([
        ("dataset", Json::s(format!("datasets/{ns}_{tabla}"))),
        ("metadata_location", Json::s(ml)),
        ("puntero_nuevo", Json::Bool(puntero_nuevo)),
        ("tabla", Json::s(nombre)),
        ("tabla_nueva", Json::Bool(tabla_nueva)),
    ]);
    if op.json {
        println!("{}", j.jcs());
    } else {
        println!(
            "{nombre} · {}{}\n  {ml}",
            if puntero_nuevo {
                "puntero nuevo"
            } else {
                "puntero movido"
            },
            if tabla_nueva {
                " · el dataset nace"
            } else {
                ""
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn el_documento_sigue_el_esquema_sin_perder_lo_demas() {
        use super::seguir_esquema;
        use std::collections::BTreeMap;
        let doc = "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: salida, namespace: ventas, description: \"lo que ana escribió\" }\n# Un dataset escrito (0033): lo escribió `write()`\nspec:\n  owner: team:ventas\n  columns:\n    cuando: { type: DateTimeTz }\n    id: { type: Integer }\n    pais: {}\n    total: { type: Decimal, labels: { gdpr.sensitivity: high } }\n    vieja:\n      type: String\n      labels: { gdpr.sensitivity: low }\n  history: { maxAge: 7d }\n  changes: { mode: append }\n";
        let cols: BTreeMap<String, String> = [
            ("cuando", "DateTimeTz"),
            ("id", "String"),     // cambia de tipo
            ("pais", "String"),   // tenía `{}`
            ("total", "Decimal"), // sigue, con sus labels
            ("nota", "String"),   // nueva
                                  // `vieja` ya no está
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let s = seguir_esquema(doc, &cols).unwrap();
        assert_eq!(
            s,
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: salida, namespace: ventas, description: \"lo que ana escribió\" }\n# Un dataset escrito (0033): lo escribió `write()`\nspec:\n  owner: team:ventas\n  columns:\n    cuando: { type: DateTimeTz }\n    id: { type: String }\n    pais: { type: String }\n    total: { type: Decimal, labels: { gdpr.sensitivity: high } }\n    nota: { type: String }\n  history: { maxAge: 7d }\n  changes: { mode: append }\n"
        );
        // lo que compila: las columnas del resultado son exactamente las pedidas
        assert_eq!(super::columnas_del_documento(&s), cols);
        // un documento sin `columns` no se edita: desde cero
        assert!(seguir_esquema("kind: Dataset\nspec: { owner: team:ventas }\n", &cols).is_none());
        // `changes` con la clave del upsert, en el sitio de la línea de antes
        let c = super::con_cambios(&s, "  changes: { mode: upsert, key: [id] }");
        assert!(
            c.ends_with("  history: { maxAge: 7d }\n  changes: { mode: upsert, key: [id] }\n"),
            "{c}"
        );
        assert_eq!(
            super::con_cambios(
                "spec:\n  changes:\n    mode: append\n  history: {}\n",
                "  changes: { mode: upsert, key: [a] }"
            ),
            "spec:\n  changes: { mode: upsert, key: [a] }\n  history: {}\n"
        );
        // un mapa con otras claves y sin `type` recibe el suyo
        let s = seguir_esquema(
            "spec:\n  columns:\n    a:\n      labels: { x: y }\n",
            &[("a".to_string(), "Integer".to_string())]
                .into_iter()
                .collect(),
        )
        .unwrap();
        assert_eq!(
            s,
            "spec:\n  columns:\n    a:\n      type: Integer\n      labels: { x: y }\n"
        );
    }

    use super::*;

    /// Lo que un cambio del catálogo REST dice de sí mismo, y las columnas
    /// de un documento del lago.
    #[test]
    fn el_cambio_dice_a_que_tabla_va_si_crea_y_su_operacion() {
        let n = ore_core::parse::parse(
            r#"{"identifier":{"namespace":["ventas"],"name":"salida"},"requirements":[{"type":"assert-create"}],"updates":[{"action":"add-schema"},{"action":"add-snapshot","snapshot":{"summary":{"operation":"append","ore.operacion":"op-7"}}}]}"#,
        )
        .unwrap();
        let c = Cambio {
            nodo: &n,
            indice: None,
        };
        assert_eq!(c.identificador().as_deref(), Some("ventas.salida"));
        assert!(c.crea());
        assert_eq!(c.operacion().as_deref(), Some("op-7"));
        let n = ore_core::parse::parse(
            r#"{"requirements":[{"type":"assert-table-uuid","uuid":"x"}],"updates":[]}"#,
        )
        .unwrap();
        let c = Cambio {
            nodo: &n,
            indice: Some(2),
        };
        assert_eq!(c.identificador(), None);
        assert!(!c.crea());
        assert_eq!(c.operacion(), None);
        let cols = columnas_del_documento(
            "kind: Dataset
spec:
  owner: team:ventas
  columns:
    id: { type: Integer }
    pais: {}
",
        );
        assert_eq!(cols.get("id").map(String::as_str), Some("Integer"));
        assert_eq!(cols.get("pais").map(String::as_str), Some("String"));
    }

    #[test]
    fn la_edad_se_lee_en_dias_horas_minutos_y_segundos() {
        assert_eq!(edad_ms("7d").unwrap(), 7 * 86_400_000);
        assert_eq!(edad_ms("12h").unwrap(), 12 * 3_600_000);
        assert_eq!(edad_ms("30m").unwrap(), 30 * 60_000);
        assert_eq!(edad_ms("45s").unwrap(), 45_000);
        assert_eq!(edad_ms("0").unwrap(), 0);
        assert!(edad_ms("una semana").is_err());
    }

    /// Los punteros de una carpeta, con el nombre del campo o del fichero, y
    /// la clase del documento del árbol: mantenido con `from`, escrito sin él.
    #[test]
    fn los_punteros_se_leen_de_una_carpeta_y_la_clase_del_documento() {
        let d = std::env::temp_dir().join(format!("ore-datasets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("datasets")).unwrap();
        std::fs::create_dir_all(d.join("packages/ventas/datasets")).unwrap();
        // Un puntero migrado: dice `vista`, `tabla` y dónde siguen sus bytes.
        std::fs::write(
            d.join("datasets/ventas_pedidos.json"),
            "{\"estado\":\"copiada\",\"vista\":\"ventas.pedidos\",\"tabla\":\"ventas.pedidos\",\"dataset\":\"copias/ventas_pedidos\",\"metadata_location\":\"gs://b/x\",\"filas\":5}",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/ventas/datasets/pedidos.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.orders }\n",
        )
        .unwrap();
        // Uno de un `write()`: sin documento todavía.
        std::fs::write(
            d.join("datasets/ventas_salida.json"),
            "{\"estado\":\"copiada\",\"metadata_location\":\"gs://b/y\"}",
        )
        .unwrap();
        let ps = punteros(&d, &d.join("datasets"));
        assert_eq!(ps.len(), 2);
        assert_eq!(
            (ps[0].clase, ps[0].nombre.as_str()),
            ("mantenido", "ventas.pedidos")
        );
        assert_eq!(
            ps[0].dataset(),
            "copias/ventas_pedidos",
            "los bytes no se mueven"
        );
        assert_eq!(
            (ps[1].clase, ps[1].nombre.as_str()),
            ("escrito", "ventas.salida")
        );
        assert_eq!(
            ps[1].dataset(),
            "datasets/ventas_salida",
            "del nombre del fichero"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// El documento de un dataset escrito nace como `kind: Dataset` con el
    /// dueño del paquete, y sigue el esquema sin perder lo demás.
    #[test]
    fn el_dataset_escrito_nace_y_sigue_el_esquema() {
        let d = std::env::temp_dir().join(format!("ore-escrito-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("packages/ventas")).unwrap();
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: x, version: 0.1.0 }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/ventas/package.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 0.1.0, status: active, domain: ventas }\nspec: { owner: team:ventas }\n",
        )
        .unwrap();
        let cols: BTreeMap<String, String> = [("id", "Integer"), ("pais", "String")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let (nueva, regen) = asegurar_dataset(&d, "ventas", "salida", &cols, None).unwrap();
        assert!(nueva && !regen);
        let t = std::fs::read_to_string(d.join("packages/ventas/datasets/salida.yaml")).unwrap();
        assert!(
            t.contains("kind: Dataset") && t.contains("owner: team:ventas"),
            "{t}"
        );
        assert!(
            t.contains("changes: { mode: append }") && !t.contains("datasource"),
            "{t}"
        );
        assert!(ore_core::validate_package(&d).is_empty());
        // Un upsert por `pais`: `changes` dice lo que admite.
        let clave = vec!["pais".to_string()];
        let (nueva, regen) = asegurar_dataset(&d, "ventas", "salida", &cols, Some(&clave)).unwrap();
        assert!(!nueva && regen);
        let t = std::fs::read_to_string(d.join("packages/ventas/datasets/salida.yaml")).unwrap();
        assert!(t.contains("changes: { mode: upsert, key: [pais] }"), "{t}");
        // Con una Table del mismo nombre no se escribe.
        std::fs::create_dir_all(d.join("packages/ventas/tables")).unwrap();
        std::fs::write(
            d.join("packages/ventas/tables/orders.yaml"),
            "kind: Table\n",
        )
        .unwrap();
        let e = asegurar_dataset(&d, "ventas", "orders", &cols, None).unwrap_err();
        assert!(e.1.contains("es una Table"), "{}", e.1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
