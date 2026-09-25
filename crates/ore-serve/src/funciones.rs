//! Las funciones del árbol y su invocación (ADR 0029, F4a·I3):
//! `GET /funciones` · `GET /funciones/{ns}/{n}/resultados` ·
//! `POST /funciones/{ns}/{n}/invocar` — y, desde 0038 P6c, con su schema:
//! `/funciones/{b}/{s}/{n}/…` (las de dos partes son de `default`).
//!
//! # Quién manda
//!
//! Invocar **no escribe el árbol**: escribe la **cola** —un `Job` rendido de
//! `plantilla-invocacion.txt` con la función, la puerta, el id servido y el
//! instante— y Flux lo crea en la celda. El Job es quien lee la copia, llama
//! al modelo y deja el informe en `resultados/` (`49-la-invocacion.yaml`); este
//! proceso sólo decide **si** se encola, y lo decide con lo que puede ver: que
//! el árbol compile donde la función vive, que sea `runtime: model` sin
//! `effects` (una función de lectura: lo demás es el paso 4), que el `Model`
//! que nombra esté y resuelva a un id servido, y que la copia de `over` **esté
//! hecha** (el informe de `copias/` lo dice). Lo que no cumple es 409 o 422
//! con el motivo, y nada se encola.
//!
//! # Lo que no decide todavía
//!
//! Quién puede invocar. Cedar entra cuando la función declare `authorization`
//! (F6/L0); hoy una que lo declare se rechaza con 422 para no fingir que se
//! evaluó.

use crate::cola;
use crate::rutas::{Servidor, de_node, token};
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::{Path, PathBuf};

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
}

/// Una `Function` del árbol, tal como está en `packages/<ns>/functions/` o en
/// `packages/<ns>/<schema>/functions/` (0038).
struct Funcion {
    ruta: PathBuf,
    ns: String,
    /// `metadata.schema`, o `default`.
    schema: String,
    nombre: String,
    spec: Node,
}

impl Funcion {
    /// Su forma corta: `p.f` en `default`, `p.s.f` en otro schema.
    fn qn(&self) -> String {
        ore_core::normalize::corto(&self.ns, &self.schema, &self.nombre)
    }
    fn texto(&self, k: &str) -> Option<String> {
        campo(&self.spec, k)
    }
    fn output(&self) -> Vec<String> {
        self.spec
            .get("output")
            .map(|(_, o)| {
                o.entries()
                    .iter()
                    .filter_map(|(k, _)| k.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn funciones_de(raiz: &Path) -> Vec<Funcion> {
    let mut out = Vec::new();
    let Ok(paquetes) = std::fs::read_dir(raiz.join("packages")) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = paquetes.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for p in dirs {
        // La raíz del paquete y la carpeta de cada schema (0038).
        for ruta in crate::rutas::yamls_del_kind(&p, "functions") {
            let Ok(texto) = std::fs::read_to_string(&ruta) else {
                continue;
            };
            let Ok(n) = parse::parse(&texto) else {
                continue;
            };
            if campo(&n, "kind").as_deref() != Some("Function") {
                continue;
            }
            let Some((_, meta)) = n.get("metadata") else {
                continue;
            };
            let (Some(nombre), Some(ns)) = (campo(meta, "name"), campo(meta, "namespace")) else {
                continue;
            };
            let schema = campo(meta, "schema")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
            let spec = n
                .get("spec")
                .map(|(_, s)| s.clone())
                .unwrap_or(Node::Sequence {
                    items: Vec::new(),
                    pos: n.pos(),
                });
            out.push(Funcion {
                ruta,
                ns,
                schema,
                nombre,
                spec,
            });
        }
    }
    out
}

/// `AAAAMMDDTHHMMSSZ`: lo que va detrás del nombre en el informe de una corrida.
fn es_corrida(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 16
        && b[8] == b'T'
        && b[15] == b'Z'
        && b[..8].iter().chain(&b[9..15]).all(u8::is_ascii_digit)
}

/// Los informes de una función, los más recientes primero: `resultados/<p>_<f>_<corrida>.json`
/// en `default`, `resultados/<p>/<s>/<f>_<corrida>.json` en otro schema
/// (`punteros::resultados_de`).
///
/// ⚠️ Lo que sigue al prefijo tiene que ser una corrida: `ia_f_` es también el
///   principio de los informes de `ia.f_x`, y sin mirarlo se mezclaban.
fn resultados_de(raiz: &Path, qn: &str) -> Vec<Json> {
    let nombre = ore_core::punteros::resultados_de(qn);
    let (sub, base) = nombre.rsplit_once('/').unwrap_or(("", &nombre));
    let prefijo = format!("{base}_");
    let Ok(es) = std::fs::read_dir(raiz.join("resultados").join(sub)) else {
        return Vec::new();
    };
    let mut ficheros: Vec<PathBuf> = es
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|f| f.to_str()).is_some_and(|f| {
                f.strip_prefix(&prefijo)
                    .and_then(|r| r.strip_suffix(".json"))
                    .is_some_and(es_corrida)
            })
        })
        .collect();
    ficheros.sort();
    ficheros.reverse();
    ficheros
        .iter()
        .filter_map(|p| {
            let t = std::fs::read_to_string(p).ok()?;
            let n = parse::parse(&t).ok()?;
            let Json::Obj(mut m) = de_node(&n) else {
                return None;
            };
            let corrida = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_prefix(&prefijo))
                .unwrap_or("")
                .to_string();
            m.insert("corrida".into(), Json::s(corrida));
            Some(Json::Obj(m))
        })
        .collect()
}

fn ficha(raiz: &Path, f: &Funcion) -> Json {
    let opt = |v: Option<String>| v.map(Json::s).unwrap_or(Json::Bool(false));
    let resultados = resultados_de(raiz, &f.qn());
    let ultimo = resultados.first().cloned().unwrap_or(Json::Bool(false));
    Json::obj([
        ("name", Json::s(&f.nombre)),
        ("namespace", Json::s(&f.ns)),
        ("schema", Json::s(&f.schema)),
        ("runtime", opt(f.texto("runtime"))),
        ("model", opt(f.texto("model"))),
        ("over", opt(f.texto("over"))),
        ("prompt", opt(f.texto("prompt"))),
        (
            "output",
            Json::Arr(f.output().into_iter().map(Json::s).collect()),
        ),
        ("effects", Json::Bool(f.spec.get("effects").is_some())),
        ("resultados", Json::Int(resultados.len() as i64)),
        ("ultimo", ultimo),
    ])
}

/// `AAAAMMDDTHHMMSSZ`: el instante de la petición, en el nombre del Job y en el
/// mensaje del commit de la cola.
pub(crate) fn corrida_ahora() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (dias, resto) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        resto / 3600,
        (resto % 3600) / 60,
        resto % 60
    )
}

impl Servidor {
    /// `GET /funciones`: las del árbol, con su forma y su último resultado.
    pub(crate) fn funciones(&self, raiz: &Path) -> Respuesta {
        let lista: Vec<Json> = funciones_de(raiz).iter().map(|f| ficha(raiz, f)).collect();
        Respuesta::ok(Json::obj([("functions", Json::Arr(lista))]))
    }

    /// `GET /funciones/{ns}[/{schema}]/{n}/resultados`: los informes, del más nuevo al más viejo.
    pub(crate) fn resultados(
        &self,
        raiz: &Path,
        ns: &str,
        schema: &str,
        nombre: &str,
    ) -> Respuesta {
        let qn = ore_core::normalize::corto(ns, schema, nombre);
        if funciones_de(raiz).iter().all(|f| f.qn() != qn) {
            return Respuesta::error(404, format!("no hay ninguna función `{qn}`"));
        }
        Respuesta::ok(Json::obj([
            ("function", Json::s(&qn)),
            ("resultados", Json::Arr(resultados_de(raiz, &qn))),
        ]))
    }

    /// `POST /funciones/{ns}[/{schema}]/{n}/invocar`: decide si se puede, y encola.
    pub(crate) fn invocar(
        &self,
        raiz: &Path,
        ns: &str,
        schema: &str,
        nombre: &str,
        sujeto: &Identidad,
    ) -> Respuesta {
        if let Err(m) = token(ns) {
            return Respuesta::error(422, format!("espacio de nombres: {m}"));
        }
        let pedida = ore_core::normalize::corto(ns, schema, nombre);
        let funciones = funciones_de(raiz);
        let Some(f) = funciones.iter().find(|f| f.qn() == pedida) else {
            return Respuesta::error(404, format!("no hay ninguna función `{pedida}`"));
        };
        let qn = f.qn();

        // ── ① el árbol compila donde la función vive ──────────────────────
        let diags = match self.diagnosticos_de(raiz) {
            Ok(d) => d,
            Err(r) => return r,
        };
        let mio = format!("packages/{ns}/");
        let propios: Vec<Json> = diags
            .into_iter()
            .filter(|d| {
                let donde = match d {
                    Json::Obj(m) => match m.get("donde") {
                        Some(Json::Str(s)) => s.clone(),
                        _ => String::new(),
                    },
                    _ => String::new(),
                };
                donde.starts_with(&mio) || !donde.starts_with("packages/")
            })
            .collect();
        if !propios.is_empty() {
            return Respuesta {
                codigo: 422,
                cuerpo: Json::obj([
                    (
                        "error",
                        Json::s(format!(
                            "el árbol no compila donde `{qn}` vive: no se encola"
                        )),
                    ),
                    ("diagnosticos", Json::Arr(propios)),
                ]),
            };
        }

        // ── ② una función de lectura con modelo ───────────────────────────
        let runtime = f.texto("runtime").unwrap_or_default();
        if runtime != "model" {
            return Respuesta::error(
                422,
                format!(
                    "`{qn}` es `runtime: {runtime}`: sólo se invoca `runtime: model` (F4a); wasm es F4b"
                ),
            );
        }
        if f.spec.get("effects").is_some() {
            return Respuesta::error(
                422,
                format!(
                    "`{qn}` declara `effects`: una función que propone es el paso 4 de F4a; ésta lee y devuelve"
                ),
            );
        }
        if f.spec.get("authorization").is_some() {
            return Respuesta::error(
                422,
                format!(
                    "`{qn}` declara `authorization`: Cedar sobre la invocación no se evalúa todavía, y no se finge"
                ),
            );
        }
        let Some(over) = f.texto("over") else {
            return Respuesta::error(
                422,
                format!("`{qn}` no dice `over`: sin filas no hay sobre qué invocar"),
            );
        };
        // En su contexto (0038): en un schema, `over` puede ir en una parte y es
        // de ese schema; `p.default.n` es `p.n`.
        let over = ore_core::normalize::qualify_catalogo(&over, Some(&f.ns), &f.schema);
        let Some(modelo) = f.texto("model") else {
            return Respuesta::error(422, format!("`{qn}` no dice `model`"));
        };

        // ── ③ el Model resuelve a una puerta y un id ──────────────────────
        let nombre_modelo = modelo
            .strip_prefix("modelo/")
            .unwrap_or(&modelo)
            .to_string();
        let (url, id) = match self.resolver_modelo(raiz, &nombre_modelo, Some(&f.ns), &f.schema) {
            Ok(v) => v,
            Err(r) => return r,
        };

        // ── ④ la copia de `over` está hecha ───────────────────────────────
        let Some((paquete, vista)) = over.split_once('.') else {
            return Respuesta::error(422, format!("`over: {over}` no es `<paquete>.<vista>`"));
        };
        // 0033: la copia de `over` es el primer dataset bajando por su cadena
        // (ella misma incluida si es un dataset). Se mira con el compilador, que
        // es quien sabe qué hay debajo; sin dataset no hay de dónde leer.
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        let doc = pkg.docs.iter().find(|d| {
            matches!(
                d.kind,
                ore_core::document::Kind::View | ore_core::document::Kind::Dataset
            ) && d.qname().as_deref() == Some(over.as_str())
        });
        let copia_qn = doc
            .and_then(|d| ore_core::vistas::raiz_de_lectura(&pkg, d))
            .and_then(|c| c.qname());
        let Some(copia_qn) = copia_qn else {
            return Respuesta::error(
                409,
                format!(
                    "`{over}` no tiene dataset debajo: una función lee la copia, nunca el origen (0029 ③). Decide la copia primero"
                ),
            );
        };
        let copia = ore_core::punteros::leer_en(&raiz.join(ore_core::punteros::CARPETA), &copia_qn)
            .map(|(_, n)| n);
        // Con qué se lee: el `metadata_location` del dataset o, mientras quede
        // alguno, la `clave` de un sobre heredado.
        let clave = copia.as_ref().and_then(|n| {
            let estado = campo(n, "estado").unwrap_or_default();
            campo(n, "metadata_location")
                .or_else(|| campo(n, "clave"))
                .filter(|c| !c.is_empty() && matches!(estado.as_str(), "copiada" | "al-dia"))
        });
        let _ = (paquete, vista);
        let Some(clave) = clave else {
            let estado = copia
                .as_ref()
                .and_then(|n| campo(n, "estado"))
                .unwrap_or_else(|| "sin informe: el Job de la copia no ha pasado".into());
            return Respuesta::error(
                409,
                format!("la copia de `{over}` no está hecha ({estado}): no hay qué leer todavía"),
            );
        };

        // ── ⑤ a la cola ───────────────────────────────────────────────────
        let corrida = corrida_ahora();
        let (job, encolado) = self.encolar_invocacion(&qn, &url, &id, &corrida, sujeto);
        Respuesta {
            codigo: if job.is_some() { 202 } else { 503 },
            cuerpo: Json::obj([
                ("function", Json::s(&qn)),
                ("over", Json::s(&over)),
                ("copia", Json::s(&clave)),
                (
                    "model",
                    Json::obj([
                        ("nombre", Json::s(&nombre_modelo)),
                        ("id", Json::s(&id)),
                        ("url", Json::s(&url)),
                    ]),
                ),
                ("corrida", Json::s(&corrida)),
                ("job", job.map(Json::s).unwrap_or(Json::Bool(false))),
                ("encolado", Json::s(encolado)),
                (
                    "fichero",
                    Json::s(
                        f.ruta
                            .strip_prefix(raiz)
                            .unwrap_or(&f.ruta)
                            .display()
                            .to_string()
                            .replace('\\', "/"),
                    ),
                ),
            ]),
        }
    }

    /// El Job a la cola: `(nombre del Job, qué pasó)`. Sin cola (`--cola`
    /// ausente) no hay quien lo rinda: se dice, y no hay Job.
    fn encolar_invocacion(
        &self,
        funcion: &str,
        puerta: &str,
        modelo: &str,
        corrida: &str,
        sujeto: &Identidad,
    ) -> (Option<String>, String) {
        let Some(forja) = &self.cola else {
            return (
                None,
                "NO encolado: este servidor no sabe de ninguna cola (`--cola`)".into(),
            );
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return (None, format!("NO encolado: {e}")),
        };
        let dir = prestado.ruta();
        let plantilla = match std::fs::read_to_string(dir.join(cola::PLANTILLA_INVOCACION)) {
            Ok(t) => t,
            Err(_) => {
                return (
                    None,
                    format!(
                        "NO encolado: la cola no trae `{}`; hay que converger este inquilino",
                        cola::PLANTILLA_INVOCACION
                    ),
                );
            }
        };
        let (fichero, texto) = match cola::rendir_invocacion(
            &plantilla,
            &cola::Invocacion {
                funcion,
                puerta,
                modelo,
                corrida,
            },
        ) {
            Ok(v) => v,
            Err(e) => return (None, format!("NO encolado: {e}")),
        };
        let job = texto
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("name: invocar-")
                    .map(|r| format!("invocar-{r}"))
            })
            .unwrap_or_default();
        if let Err(e) = std::fs::write(dir.join(&fichero), &texto) {
            return (
                None,
                format!("NO encolado: no se pudo escribir `{fichero}`: {e}"),
            );
        }
        if !forja.hay_cambios(dir) {
            return (Some(job), format!("ya encolado como `{fichero}`"));
        }
        match forja.publicar(dir, sujeto, &format!("Invocar {funcion} ({corrida})")) {
            Ok(c) => (Some(job), format!("encolado como `{fichero}` · commit {c}")),
            Err(e) => (None, format!("NO encolado: {e}")),
        }
    }
}
