//! `ore invoke` — **una `Function` de lectura sobre la copia** (ADR 0029 ③, F4a).
//!
//! Las tres mitades del Job, encadenadas por este proceso sin abrir un socket:
//!
//! | mitad | quién | cómo |
//! |---|---|---|
//! | **traer** | `ore-store-<r2\|gcs> leer` | la copia de `over`, por el nombre que su informe (`copias/<paquete>_<vista>.json`) dejó en el árbol |
//! | **invocar** | `ore-invoke` | una llamada por fila a la puerta con el token de la celda (`MODELO_TOKEN`), la respuesta ya en la forma de `output` |
//! | **devolver** | `ore-store-<tipo> sellar` + `--informe DIR` | las filas con su `output` selladas en el bucket del inquilino, y un informe en el árbol con los números y una muestra |
//!
//! # Dónde aterriza lo que devuelve, y por qué ahí
//!
//! **Los datos al bucket, los números al árbol**, como la copia. El resultado
//! es un artefacto más del almacén —el mismo sobre, el mismo nombre por digest—
//! cuya cabecera dice de qué copia salió (`testigo.valor` = la clave de la copia
//! leída), bajo qué función (`conducto` = `function:<ns>.<f>`) y con qué
//! esquema (el de la copia más `output`). Dos corridas de la misma función
//! sobre la misma copia con la misma respuesta del modelo son **el mismo
//! artefacto**, y la segunda no sube un byte. El informe
//! (`resultados/<ns>_<f>_<corrida>.json`) es lo que `ore-serve` y la consola
//! pueden leer sin alcanzar el almacén: filas, aciertos, errores, tokens,
//! milisegundos y las cinco primeras respuestas.
//!
//! # Lo que este verbo no hace (todavía)
//!
//! `effects` (una función de lectura no propone: F4a paso 4), `reads` además
//! de `over` (llegan con el contexto del modelo), `runtime: wasm` (F4b), y
//! decidir quién puede invocarla (Cedar, en `ore-serve` al encolar).

use crate::lector;
use crate::materializar::{Puntero, paquete_del_fichero, programa_del_almacen};
use ore_core::json::Json;
use ore_core::link::Loaded;
use std::collections::BTreeMap;
use std::path::Path;

pub struct Opciones<'a> {
    pub funcion: &'a str,
    pub puerta: Option<&'a str>,
    pub modelo: Option<&'a str>,
    pub informe: Option<&'a Path>,
    pub limite: Option<usize>,
    pub concurrencia: usize,
    pub seco: bool,
}

pub fn invocar(path: &Path, op: &Opciones) -> std::process::ExitCode {
    match correr(path, op) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err((codigo, mensaje)) => {
            eprintln!("error: {mensaje}");
            std::process::ExitCode::from(codigo)
        }
    }
}

type Fallo = (u8, String);

fn correr(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    // ── ① El árbol, y que compile lo que esta función toca ──────────────────
    if !path.is_dir() {
        return Err((
            66,
            format!("`{}` no es un directorio de paquete", path.display()),
        ));
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(path);
    let f = pkg
        .docs
        .iter()
        .find(|d| {
            d.kind == ore_core::document::Kind::Function && d.qname().as_deref() == Some(op.funcion)
        })
        .ok_or_else(|| {
            (
                65,
                format!("no hay ninguna `Function` `{}` en el árbol", op.funcion),
            )
        })?;
    let mio = paquete_del_fichero(path, &f.path);
    for d in ore_core::validate_package(path) {
        if d.code == ore_core::Code::Oos2013 {
            continue;
        }
        let suyo = paquete_del_fichero(path, &d.file);
        if suyo.is_none() || suyo == mio {
            return Err((
                65,
                format!(
                    "el árbol no compila donde esta función vive:\n{}",
                    d.render(path)
                ),
            ));
        }
    }

    // ── ② La función: runtime, modelo, over, prompt, output ─────────────────
    let texto = |k: &str| f.section(k).and_then(|n| n.as_str()).map(String::from);
    let runtime = texto("runtime").unwrap_or_default();
    if runtime != "model" {
        return Err((
            65,
            format!(
                "`{}` es `runtime: {runtime}`: `ore invoke` sólo sabe de `runtime: model` (F4a); wasm es F4b",
                op.funcion
            ),
        ));
    }
    if f.section("effects").is_some() {
        return Err((
            65,
            format!(
                "`{}` declara `effects`: una función que propone es el paso 4 de F4a, no éste",
                op.funcion
            ),
        ));
    }
    let modelo_doc = texto("model").ok_or((65, "la función no dice `model`".to_string()))?;
    let over = texto("over").ok_or((
        65,
        "la función no dice `over`: sin filas no hay sobre qué invocar".to_string(),
    ))?;
    let prompt = texto("prompt").ok_or((65, "la función no lleva `prompt`".to_string()))?;
    let output: BTreeMap<String, String> = f
        .section("output")
        .map(|o| {
            o.entries()
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
        .unwrap_or_default();

    // ── ③ La copia de `over`: declarada, hecha, y con nombre ────────────────
    let vista: &Loaded = pkg
        .docs
        .iter()
        .find(|d| {
            d.kind == ore_core::document::Kind::View && d.qname().as_deref() == Some(over.as_str())
        })
        .ok_or_else(|| (65, format!("`over: {over}` no es una vista del árbol")))?;
    if vista.section("materialized").is_none() {
        return Err((
            65,
            format!(
                "`{over}` no declara copia: una función lee la copia, nunca el origen (0029 ③)"
            ),
        ));
    }
    let puntero =
        Puntero::hecho(path, &over).map_err(|e| (65, format!("la copia de `{over}`: {e}")))?;
    let clave_copia = puntero.nombre().to_string();

    // ── ④ La puerta y el id servido ─────────────────────────────────────────
    let nombre_modelo = modelo_doc
        .strip_prefix("modelo/")
        .unwrap_or(&modelo_doc)
        .to_string();
    let puerta = op
        .puerta
        .map(String::from)
        .or_else(|| std::env::var("MODELO_URL").ok().filter(|s| !s.is_empty()))
        .ok_or((78, "no sé dónde está la puerta: `--puerta` o `MODELO_URL` (lo que `GET /modelos/{n}` devuelve como `url`)".to_string()))?;
    let id = op
        .modelo
        .map(String::from)
        .or_else(|| std::env::var("MODELO_ID").ok().filter(|s| !s.is_empty()))
        .ok_or((
            78,
            "no sé qué id sirve el modelo: `--modelo` o `MODELO_ID` (`GET /modelos/{n}` → `model`)"
                .to_string(),
        ))?;

    println!("{}", op.funcion);
    println!("  over `{over}` · copia {clave_copia}");
    println!("  modelo `{modelo_doc}` → `{id}` por {puerta}");

    // ── traer ───────────────────────────────────────────────────────────────
    let programa = programa_del_almacen().map_err(|e| (78, e))?;
    let leido = lector::ejecutar(&programa, &["leer".into()], Some(&puntero.peticion_leer()))
        .map_err(|e| (69, con_ayuda(e)))?;
    let mut lineas = leido.lines().filter(|l| !l.trim().is_empty());
    let cabecera = lineas
        .next()
        .ok_or((69, format!("`{programa} leer` no devolvió la cabecera")))?;
    let cab = ore_core::parse::parse(cabecera)
        .map_err(|e| (69, format!("la cabecera de la copia no analiza: {e:?}")))?;
    let mut esquema: BTreeMap<String, String> = cab
        .get("esquema")
        .map(|(_, e)| {
            e.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    for (k, t) in &output {
        if esquema.contains_key(k) {
            return Err((
                65,
                format!(
                    "`output.{k}` se llama como un campo de `{over}`: el resultado no podría llevar los dos"
                ),
            ));
        }
        esquema.insert(k.clone(), t.clone());
    }
    let filas: Vec<&str> = match op.limite {
        Some(n) => lineas.take(n).collect(),
        None => lineas.collect(),
    };
    println!(
        "  {} fila(s) de la copia{}",
        filas.len(),
        op.limite
            .map(|n| format!(" (límite {n})"))
            .unwrap_or_default()
    );
    if op.seco {
        println!("  seco · no se llama al modelo ni se sella nada");
        return Ok(());
    }

    // ── invocar ─────────────────────────────────────────────────────────────
    let mut entrada = Json::obj([
        ("concurrencia", Json::Int(op.concurrencia.max(1) as i64)),
        ("modelo", Json::s(&id)),
        (
            "output",
            Json::Obj(
                output
                    .iter()
                    .map(|(k, t)| (k.clone(), Json::s(t)))
                    .collect(),
            ),
        ),
        ("prompt", Json::s(&prompt)),
        ("puerta", Json::s(&puerta)),
    ])
    .jcs();
    for l in &filas {
        entrada.push('\n');
        entrada.push_str(l);
    }
    let t0 = std::time::Instant::now();
    let salida =
        lector::ejecutar("ore-invoke", &[], Some(&entrada)).map_err(|e| (69, con_ayuda(e)))?;
    let total_ms = t0.elapsed().as_millis() as i64;

    let mut buenas: Vec<String> = Vec::new();
    let mut muestra: Vec<Json> = Vec::new();
    let mut errores: Vec<String> = Vec::new();
    let (mut tok_in, mut tok_out, mut suma_ms) = (0i64, 0i64, 0i64);
    for l in salida.lines().filter(|l| !l.trim().is_empty()) {
        let n = ore_core::parse::parse(l).map_err(|e| {
            (
                69,
                format!("lo que devolvió `ore-invoke` no analiza: {e:?}\n{l}"),
            )
        })?;
        if let Some((_, e)) = n.get("error") {
            errores.push(e.as_str().unwrap_or("?").to_string());
            continue;
        }
        let entero = |a: &str, b: Option<&str>| -> i64 {
            let nodo = match b {
                Some(b) => n.get(a).and_then(|(_, x)| x.get(b)).map(|(_, v)| v),
                None => n.get(a).map(|(_, v)| v),
            };
            nodo.and_then(|v| v.as_str())
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        tok_in += entero("tokens", Some("entrada"));
        tok_out += entero("tokens", Some("salida"));
        suma_ms += entero("ms", None);
        let mut fila: BTreeMap<String, Json> = BTreeMap::new();
        if let Some((_, f)) = n.get("fila") {
            for (k, v) in f.entries() {
                if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                    fila.insert(k.to_string(), Json::s(v));
                }
            }
        }
        let mut out_j: BTreeMap<String, Json> = BTreeMap::new();
        if let Some((_, o)) = n.get("output") {
            for (k, v) in o.entries() {
                if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                    out_j.insert(k.to_string(), Json::s(v));
                }
            }
        }
        if muestra.len() < 5 {
            muestra.push(Json::obj([
                ("fila", Json::Obj(fila.clone())),
                ("output", Json::Obj(out_j.clone())),
            ]));
        }
        fila.extend(out_j);
        buenas.push(Json::Obj(fila).jcs());
    }
    let media = if buenas.is_empty() {
        0
    } else {
        suma_ms / buenas.len() as i64
    };
    println!(
        "  {} ok · {} error(es) · {tok_in}+{tok_out} tokens · {:.1} s ({media} ms por fila)",
        buenas.len(),
        errores.len(),
        total_ms as f64 / 1000.0
    );
    for e in errores.iter().take(3) {
        println!("    ✗ {}", e.lines().next().unwrap_or(""));
    }
    if buenas.is_empty() {
        return Err((
            69,
            "el modelo no contestó ninguna fila: nada que sellar".to_string(),
        ));
    }

    // ── devolver ────────────────────────────────────────────────────────────
    //
    // El resultado es un dataset (0031 §10): `resultados/<p>_<f>`, con su
    // puntero en `<informe>/<p>_<f>.json` y una corrida por snapshot. Si el
    // puntero está, la corrida de hoy sobrescribe la anterior y la historia
    // se queda; si no, el dataset nace.
    let dataset_resultado = format!("resultados/{}", op.funcion.replace('.', "_"));
    let puntero_resultado = op
        .informe
        .map(|d| d.join(format!("{}.json", op.funcion.replace('.', "_"))));
    let base_resultado: Option<String> = puntero_resultado
        .as_ref()
        .and_then(|r| std::fs::read_to_string(r).ok())
        .and_then(|t| ore_core::parse::parse(&t).ok())
        .and_then(|n| {
            n.get("metadata_location")
                .and_then(|(_, v)| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
        });
    let plan = ore_core::digest::de_bytes(
        Json::obj([
            ("copia", Json::s(&clave_copia)),
            ("funcion", Json::s(op.funcion)),
            ("modelo", Json::s(&id)),
            ("prompt", Json::s(&prompt)),
        ])
        .jcs()
        .as_bytes(),
    );
    let cabecera_resultado = Json::obj([
        ("bundle", Json::s(ore_core::digest::bundle(&pkg))),
        ("clave", Json::Arr(Vec::new())),
        ("conducto", Json::s(format!("function:{}", op.funcion))),
        (
            "esquema",
            Json::Obj(
                esquema
                    .iter()
                    .map(|(k, t)| (k.clone(), Json::s(t)))
                    .collect(),
            ),
        ),
        ("plan", Json::s(&plan)),
        (
            "testigo",
            Json::obj([
                ("modo", Json::s("snapshot")),
                ("valor", Json::s(&clave_copia)),
            ]),
        ),
    ])
    .jcs();
    let mut para_sellar = cabecera_resultado.replacen(
        '{',
        &format!(
            "{{\"dataset\":\"{dataset_resultado}\",\"fundir\":false,{}",
            base_resultado
                .as_ref()
                .map(|b| format!("\"base\":\"{b}\","))
                .unwrap_or_default()
        ),
        1,
    );
    for b in &buenas {
        para_sellar.push('\n');
        para_sellar.push_str(b);
    }
    let sellado = lector::ejecutar(&programa, &["sellar".into()], Some(&para_sellar))
        .map_err(|e| (69, con_ayuda(e)))?;
    let s = ore_core::parse::parse(&sellado).map_err(|e| {
        (
            69,
            format!("lo que devolvió `{programa} sellar` no analiza: {e:?}"),
        )
    })?;
    let campo = |k: &str| {
        s.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    println!(
        "  sellado · {} · {} filas · {} bytes · {}",
        campo("metadata_location"),
        campo("filas"),
        campo("bytes"),
        match campo("operacion").as_str() {
            "creada" => "el dataset nace",
            _ => "snapshot nuevo sobre la corrida anterior",
        }
    );

    if let Some(dir) = op.informe {
        let cuando = ahora_utc();
        let corrida = cuando.replace(['-', ':'], "");
        let informe = Json::obj([
            ("copia", puntero.como_json()),
            ("cuando", Json::s(&cuando)),
            ("errores", Json::Int(errores.len() as i64)),
            (
                "errores_muestra",
                Json::Arr(
                    errores
                        .iter()
                        .take(3)
                        .map(|e| Json::s(e.lines().next().unwrap_or("")))
                        .collect(),
                ),
            ),
            (
                "estado",
                Json::s(if errores.is_empty() { "ok" } else { "parcial" }),
            ),
            ("filas", Json::Int(filas.len() as i64)),
            ("funcion", Json::s(op.funcion)),
            (
                "modelo",
                Json::obj([
                    ("id", Json::s(&id)),
                    ("nombre", Json::s(&nombre_modelo)),
                    ("puerta", Json::s(&puerta)),
                ]),
            ),
            (
                "ms",
                Json::obj([
                    ("por_fila", Json::Int(media)),
                    ("total", Json::Int(total_ms)),
                ]),
            ),
            ("muestra", Json::Arr(muestra)),
            ("ok", Json::Int(buenas.len() as i64)),
            (
                "resultado",
                Json::obj([
                    ("bytes", Json::Int(campo("bytes").parse().unwrap_or(0))),
                    ("dataset", Json::s(&dataset_resultado)),
                    ("metadata_location", Json::s(campo("metadata_location"))),
                    ("operacion", Json::s(campo("operacion"))),
                    ("plan", Json::s(&plan)),
                    ("snapshot", Json::s(campo("snapshot"))),
                ]),
            ),
            (
                "tokens",
                Json::obj([
                    ("entrada", Json::Int(tok_in)),
                    ("salida", Json::Int(tok_out)),
                ]),
            ),
        ]);
        std::fs::create_dir_all(dir)
            .map_err(|e| (73, format!("no se pudo crear `{}`: {e}", dir.display())))?;
        let ruta = dir.join(format!("{}_{corrida}.json", op.funcion.replace('.', "_")));
        std::fs::write(&ruta, informe.pretty() + "\n")
            .map_err(|e| (73, format!("no se pudo escribir `{}`: {e}", ruta.display())))?;
        println!("  informe · {}", ruta.display());
        // Y el puntero del dataset de resultados: lo que la siguiente corrida
        // sobrescribe, y lo que un lector encuentra sin buscar la corrida.
        if let Some(r) = &puntero_resultado {
            let p = Json::obj([
                ("estado", Json::s("copiada")),
                ("dataset", Json::s(&dataset_resultado)),
                ("metadata_location", Json::s(campo("metadata_location"))),
                ("snapshot", Json::s(campo("snapshot"))),
                ("filas", Json::Int(campo("filas").parse().unwrap_or(0))),
                ("funcion", Json::s(op.funcion)),
                ("corrida", Json::s(&corrida)),
                ("plan", Json::s(&plan)),
            ]);
            std::fs::write(r, p.pretty() + "\n")
                .map_err(|e| (73, format!("no se pudo escribir `{}`: {e}", r.display())))?;
        }
    }
    Ok(())
}

fn con_ayuda(f: lector::Fallo) -> String {
    let mut s = f.mensaje;
    for l in f.ayuda {
        s.push('\n');
        s.push_str(&l);
    }
    s
}

/// `AAAA-MM-DDTHH:MM:SSZ` sin una crate de fechas: los segundos desde la época
/// a civil, con el algoritmo de Howard Hinnant.
fn ahora_utc() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (dias, resto) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        resto / 3600,
        (resto % 3600) / 60,
        resto % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_fecha_es_iso_y_utc() {
        let f = ahora_utc();
        assert_eq!(f.len(), 20, "{f}");
        assert!(f.starts_with("20") && f.ends_with('Z'), "{f}");
    }
}
