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
}

pub fn datasets(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let r = if let Some(n) = op.confirmar {
        confirmar(path, n, op)
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
    /// El nombre del dataset en el bucket (`copias/p_v`, `datasets/p_t`).
    pub fn dataset(&self) -> String {
        self.campo("dataset").unwrap_or_else(|| {
            format!(
                "{}/{}",
                if self.clase == "copia" {
                    "copias"
                } else {
                    "datasets"
                },
                self.nombre.replace('.', "_")
            )
        })
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

/// Los punteros de las dos clases. El nombre `<p>.<x>` sale del campo `vista`
/// o `tabla` del puntero y, si no lo trae, del nombre del fichero
/// (`<p>_<x>.json`: la primera `_` separa el paquete, que no lleva ninguna).
pub(crate) fn punteros(path: &Path, copias: &Path) -> Vec<Puntero> {
    let mut out = Vec::new();
    for (clase, dir, campo) in [
        ("copia", copias.to_path_buf(), "vista"),
        ("dataset", path.join("datasets"), "tabla"),
    ] {
        let Ok(entradas) = std::fs::read_dir(&dir) else {
            continue;
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
            let nombre = campo_de(&nodo, campo).unwrap_or_else(|| {
                let stem = ruta
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                match stem.split_once('_') {
                    Some((p, x)) => format!("{p}.{x}"),
                    None => stem.to_string(),
                }
            });
            out.push(Puntero {
                clase,
                nombre,
                ruta,
                nodo,
            });
        }
    }
    out
}

fn dir_copias(path: &Path, op: &Opciones) -> PathBuf {
    op.informe
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.join("copias"))
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
        println!("sin datasets · ningún puntero en `copias/` ni en `datasets/`");
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
            format!("no hay ningún dataset `{nombre}`: ni en `copias/` ni en `datasets/`"),
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
        ("edad_ms", Json::Int(edad.unwrap_or(0))),
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

/// El bloque de `datasources` que declara el lago del inquilino. `connectionEnv`
/// es obligatorio en el esquema y aquí no guarda ningún secreto: la «conexión»
/// del lago es la raíz de su bucket (`gs://<bucket>`), que ya sabe todo pod.
pub(crate) const LAGO: &str = "  # El lago del inquilino (0031 §10): donde viven los datasets — la copia de\n  # cada vista materializada y la salida de cada `write()` — como tablas\n  # Iceberg. No es un secreto: `LAGO_URL` es la raíz del bucket de la celda.\n  - name: lago\n    type: lago\n    connectionEnv: LAGO_URL\n";

/// Declara el `datasource: lago` en `ontology.config.yaml` si no está. `Ok(true)`
/// si lo escribió.
pub(crate) fn asegurar_lago(path: &Path) -> Result<bool, String> {
    let ruta = path.join("ontology.config.yaml");
    let texto = std::fs::read_to_string(&ruta)
        .map_err(|e| format!("no se pudo leer `{}`: {e}", ruta.display()))?;
    let n = ore_core::parse::parse(&texto)
        .map_err(|e| format!("`ontology.config.yaml` no analiza: {e:?}"))?;
    let hay = n
        .get("datasources")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .any(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some("lago"));
    if hay {
        return Ok(false);
    }
    let nuevo = crate::fuente::insertar(&texto, LAGO)?;
    std::fs::write(&ruta, nuevo)
        .map_err(|e| format!("no se pudo escribir `{}`: {e}", ruta.display()))?;
    Ok(true)
}

/// **El swap.** Ver la cabecera del módulo.
fn confirmar(path: &Path, nombre: &str, op: &Opciones) -> Result<(), Fallo> {
    let Some((ns, tabla)) = nombre.split_once('.') else {
        return Err((64, format!("`{nombre}` no es `<paquete>.<tabla>`")));
    };
    let bien = |s: &str| {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    };
    if !bien(ns) || !bien(tabla) {
        return Err((
            64,
            format!("`{nombre}`: paquete y tabla llevan letras, dígitos y `_`"),
        ));
    }
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
    let dir = path.join("datasets");
    let ruta = dir.join(format!("{ns}_{tabla}.json"));
    let previo = std::fs::read_to_string(&ruta)
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok());
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
        return salida(op, nombre, ns, tabla, ml, false, false, false);
    }

    // ── la fuente y el documento ────────────────────────────────────────────
    let lago_nuevo = asegurar_lago(path).map_err(|m| (65, m))?;
    let doc = path
        .join("packages")
        .join(ns)
        .join("tables")
        .join(format!("{tabla}.yaml"));
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
            for (c, t) in &m {
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
            Some(m)
        }
        None => None,
    };
    let tabla_nueva = !doc.is_file();
    let texto_previo = std::fs::read_to_string(&doc).ok();
    if let Some(t) = &texto_previo {
        let n = ore_core::parse::parse(t)
            .map_err(|e| (65, format!("`{}` no analiza: {e:?}", doc.display())))?;
        let ds = n
            .get("spec")
            .and_then(|(_, s)| s.get("datasource"))
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("");
        if ds != "lago" {
            return Err((
                65,
                format!(
                    "`{nombre}` es una Table de `{ds}`, no del lago: un dataset no puede apuntar a una tabla de otra fuente"
                ),
            ));
        }
    }
    let escribir_doc = match (&columnas, tabla_nueva) {
        (None, true) => {
            return Err((
                65,
                format!(
                    "la Table `{nombre}` no existe y no se dieron `--columnas`: el documento del lago nace con el esquema de la primera escritura"
                ),
            ));
        }
        (None, false) => false,
        (Some(_), _) => true,
    };
    if escribir_doc {
        let cols = columnas.as_ref().expect("columnas");
        let mut s = format!(
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: {{ name: {tabla}, namespace: {ns} }}\n# Una tabla del lago (0031 §10): la escribió `write()` desde un puesto, y este\n# documento nació con el esquema de esa primera escritura. Su puntero es\n# `datasets/{ns}_{tabla}.json`; su historia, los snapshots de la tabla Iceberg.\nspec:\n  datasource: lago\n  object: \"{ns}_{tabla}\"\n  columns:\n"
        );
        for (c, t) in cols {
            s.push_str(&format!("    {c}: {{ type: {t} }}\n"));
        }
        s.push_str(
            "  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
        );
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
                    "la Table `{nombre}` no compila con esas columnas:\n{}",
                    malos.join("\n")
                ),
            ));
        }
    }

    // ── el puntero ──────────────────────────────────────────────────────────
    std::fs::create_dir_all(&dir)
        .map_err(|e| (73, format!("no se pudo crear `{}`: {e}", dir.display())))?;
    let mut m: BTreeMap<String, Json> = match previo.as_ref().map(Json::de_node) {
        Some(Json::Obj(m)) => m,
        _ => Default::default(),
    };
    m.insert("estado".into(), Json::s("copiada"));
    m.insert("tabla".into(), Json::s(nombre));
    m.insert("dataset".into(), Json::s(format!("datasets/{ns}_{tabla}")));
    m.insert("metadata_location".into(), Json::s(ml));
    m.insert("snapshot".into(), Json::s(op.snapshot.unwrap_or_default()));
    if let Some(f) = op.filas {
        m.insert("filas".into(), Json::Int(f));
    }
    if let Some(s) = op.sujeto {
        m.insert("escrito_por".into(), Json::s(s));
    }
    m.remove("motivo");
    std::fs::write(&ruta, Json::Obj(m).pretty() + "\n")
        .map_err(|e| (73, format!("no se pudo escribir `{}`: {e}", ruta.display())))?;
    salida(
        op,
        nombre,
        ns,
        tabla,
        ml,
        previo.is_none(),
        tabla_nueva,
        lago_nuevo,
    )
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
    lago_nuevo: bool,
) -> Result<(), Fallo> {
    let j = Json::obj([
        ("dataset", Json::s(format!("datasets/{ns}_{tabla}"))),
        ("lago_declarado", Json::Bool(lago_nuevo)),
        ("metadata_location", Json::s(ml)),
        ("puntero_nuevo", Json::Bool(puntero_nuevo)),
        ("tabla", Json::s(nombre)),
        ("tabla_nueva", Json::Bool(tabla_nueva)),
    ]);
    if op.json {
        println!("{}", j.jcs());
    } else {
        println!(
            "{nombre} · {}{}{}\n  {ml}",
            if puntero_nuevo {
                "puntero nuevo"
            } else {
                "puntero movido"
            },
            if tabla_nueva {
                " · la Table del lago nace"
            } else {
                ""
            },
            if lago_nuevo {
                " · `datasource: lago` declarado"
            } else {
                ""
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_edad_se_lee_en_dias_horas_minutos_y_segundos() {
        assert_eq!(edad_ms("7d").unwrap(), 7 * 86_400_000);
        assert_eq!(edad_ms("12h").unwrap(), 12 * 3_600_000);
        assert_eq!(edad_ms("30m").unwrap(), 30 * 60_000);
        assert_eq!(edad_ms("45s").unwrap(), 45_000);
        assert_eq!(edad_ms("0").unwrap(), 0);
        assert!(edad_ms("una semana").is_err());
    }

    /// Los punteros de las dos clases, con el nombre del campo o del fichero.
    #[test]
    fn los_punteros_se_leen_de_las_dos_carpetas() {
        let d = std::env::temp_dir().join(format!("ore-datasets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("copias")).unwrap();
        std::fs::create_dir_all(d.join("datasets")).unwrap();
        std::fs::write(
            d.join("copias/ventas_pedidos.json"),
            "{\"estado\":\"copiada\",\"vista\":\"ventas.pedidos\",\"metadata_location\":\"gs://b/x\",\"filas\":5}",
        )
        .unwrap();
        std::fs::write(
            d.join("datasets/ventas_salida.json"),
            "{\"estado\":\"copiada\",\"metadata_location\":\"gs://b/y\"}",
        )
        .unwrap();
        let ps = punteros(&d, &d.join("copias"));
        assert_eq!(ps.len(), 2);
        assert_eq!(
            (ps[0].clase, ps[0].nombre.as_str()),
            ("copia", "ventas.pedidos")
        );
        assert_eq!(ps[0].dataset(), "copias/ventas_pedidos");
        assert_eq!(
            (ps[1].clase, ps[1].nombre.as_str()),
            ("dataset", "ventas.salida")
        );
        assert_eq!(
            ps[1].dataset(),
            "datasets/ventas_salida",
            "del nombre del fichero"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `asegurar_lago` escribe el bloque una vez y no dos.
    #[test]
    fn el_lago_se_declara_una_vez() {
        let d = std::env::temp_dir().join(format!("ore-lago-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: x, version: 0.1.0 }\n",
        )
        .unwrap();
        assert!(asegurar_lago(&d).unwrap());
        let t = std::fs::read_to_string(d.join("ontology.config.yaml")).unwrap();
        assert!(
            t.contains("name: lago") && t.contains("connectionEnv: LAGO_URL"),
            "{t}"
        );
        assert!(!asegurar_lago(&d).unwrap(), "ya estaba");
        assert_eq!(
            t,
            std::fs::read_to_string(d.join("ontology.config.yaml")).unwrap()
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
