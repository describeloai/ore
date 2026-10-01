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
    /// Los campos de `output`. Vacío si no tiene, o si es un valor (`{type:
    /// T}`, v1alpha18 01 §4.7): entonces sale en la columna `valor`.
    fn output(&self) -> Vec<String> {
        self.spec
            .get("output")
            .filter(|(_, o)| ore_core::promover::salida_valor(o).is_none())
            .map(|(_, o)| {
                o.entries()
                    .iter()
                    .filter_map(|(k, _)| k.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Las `Function` del árbol, **las que el compilador ve**: en cualquier carpeta
/// del paquete, también la de un repositorio (0050 P3). Antes se buscaban en
/// `packages/<ns>/functions/` y en la de cada schema, y una función declarada
/// dentro de su repositorio compilaba y salía en el catálogo, pero invocarla
/// era 404 (medido el 2026-10-01, M1).
fn funciones_de(raiz: &Path) -> Vec<Funcion> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let mut out: Vec<Funcion> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Function)
        .filter_map(|d| {
            let meta = |k: &str| d.meta(k).and_then(|v| v.as_str()).map(str::to_string);
            let (nombre, ns) = (meta("name")?, meta("namespace")?);
            let schema = meta("schema")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
            let spec = d
                .root
                .get("spec")
                .map(|(_, s)| s.clone())
                .unwrap_or(Node::Sequence {
                    items: Vec::new(),
                    pos: d.root.pos(),
                });
            Some(Funcion {
                ruta: d.path.clone(),
                ns,
                schema,
                nombre,
                spec,
            })
        })
        .collect();
    out.sort_by(|a, b| a.ruta.cmp(&b.ruta));
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn invocar(
        &self,
        raiz: &Path,
        ns: &str,
        schema: &str,
        nombre: &str,
        sujeto: &Identidad,
        cuerpo: &str,
        plan: &std::cell::RefCell<Option<PlanPython>>,
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

        // ── ②′ una función de código (0050 P3): se decide aquí y corre como
        //    un trabajo del puesto, que se lanza fuera de esta lectura ─────
        if f.texto("runtime").as_deref() == Some("python") {
            return match self.plan_python(raiz, f, &qn, cuerpo, sujeto) {
                Ok(p) => {
                    let r = Respuesta {
                        codigo: 202,
                        cuerpo: Json::obj([
                            ("function", Json::s(&qn)),
                            ("runtime", Json::s("python")),
                            ("corrida", Json::s(&p.invocada.corrida)),
                        ]),
                    };
                    *plan.borrow_mut() = Some(p);
                    r
                }
                Err(r) => r,
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
        let clave = match copia_de(raiz, &over) {
            Ok(c) => c,
            Err(r) => return r,
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

// ── 0050 P3 · la función de código ──────────────────────────────────────────
//
// Una `Function` de `runtime: python` (OOS v1alpha18) corre como **un trabajo
// del puesto** (0031): la misma cola, la misma imagen, el mismo agente. Lo que
// cambia es la celda: no es un fichero del árbol sino **el arnés**, que trae el
// módulo tal como está en el commit, llama al `def` con la fila (si hay
// `over`) y con los parámetros por su nombre, comprueba lo que devuelve contra
// `output` y corta al pasar `limits.timeout`. Lo que el trabajo puede leer es
// lo declarado (`over` y `reads`, como el `transform` de 0031 W3.7 ⑤) y no
// puede escribir nada: una función de lectura devuelve.

/// **La copia de `over` está hecha**, y con qué se lee: lo mismo para
/// `runtime: model` y para una función de código, antes de encolar (0029 ③:
/// una función lee la copia, nunca el origen).
///
/// 0033: la copia de `over` es el primer dataset bajando por su cadena (ella
/// misma incluida si es un dataset); la de una vista SQL, el dataset que la
/// copia entera, encima (ADR 0040 paso 4c). Se mira con el compilador, que es
/// quien sabe dónde está; sin dataset no hay de dónde leer.
fn copia_de(raiz: &Path, over: &str) -> Result<String, Respuesta> {
    if !over.contains('.') {
        return Err(Respuesta::error(
            422,
            format!("`over: {over}` no es `<paquete>.<vista>`"),
        ));
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let doc = pkg.docs.iter().find(|d| {
        matches!(
            d.kind,
            ore_core::document::Kind::View | ore_core::document::Kind::Dataset
        ) && d.qname().as_deref() == Some(over)
    });
    let Some(copia_qn) = doc
        .and_then(|d| ore_core::vistas::dataset_de_lectura(&pkg, d))
        .and_then(|c| c.qname())
    else {
        return Err(Respuesta::error(
            409,
            format!(
                "`{over}` no tiene dataset debajo: una función lee la copia, nunca el origen (0029 ③). Decide la copia primero"
            ),
        ));
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
    clave.ok_or_else(|| {
        let estado = copia
            .as_ref()
            .and_then(|n| campo(n, "estado"))
            .unwrap_or_else(|| "sin informe: el Job de la copia no ha pasado".into());
        Respuesta::error(
            409,
            format!("la copia de `{over}` no está hecha ({estado}): no hay qué leer todavía"),
        )
    })
}

/// Lo decidido al leer el árbol, para lanzar fuera de la lectura.
pub(crate) struct PlanPython {
    pub invocada: crate::puestos::Invocada,
    /// `packages/<p>/…/<f>.py`: el fichero del árbol que corre.
    pub codigo: String,
    pub commit: String,
    pub arnes: String,
    pub lee: Vec<String>,
}

impl Servidor {
    fn plan_python(
        &self,
        raiz: &Path,
        f: &Funcion,
        qn: &str,
        cuerpo: &str,
        sujeto: &Identidad,
    ) -> Result<PlanPython, Respuesta> {
        if crate::puestos::es_agente(sujeto) {
            return Err(Respuesta::error(
                403,
                "una función de código corre como un trabajo, y un trabajo lo lanza una persona",
            ));
        }
        for (clave, por_que) in [
            (
                "effects",
                "cómo propone una función de código tiene su propia especificación (0050); ésta lee y devuelve",
            ),
            (
                "authorization",
                "Cedar sobre la invocación no se evalúa todavía, y no se finge",
            ),
        ] {
            if f.spec.get(clave).is_some() {
                return Err(Respuesta::error(
                    422,
                    format!("`{qn}` declara `{clave}`: {por_que}"),
                ));
            }
        }

        // Los parámetros, contra `input`: nada que no declare, todo lo
        // obligatorio, y cada valor de su tipo.
        let parametros = parametros_de(f, cuerpo)?;

        // Los modelos que el código puede llamar (`models`, 0050 P4): cada uno
        // resuelto aquí a su puerta y su id servido, como el de `runtime:
        // model`. El código los pide por la referencia tal como se escribió
        // (`extractor`) o por su nombre entero (`ventas.default.extractor`).
        let mut modelos = std::collections::BTreeMap::new();
        let mut usados = Vec::new();
        for m in f.spec.get("models").map(|(_, v)| v.items()).unwrap_or(&[]) {
            let Some(r) = m.as_str() else { continue };
            let nombre = r.strip_prefix("modelo/").unwrap_or(r);
            let (url, id) = self.resolver_modelo(raiz, nombre, Some(&f.ns), &f.schema)?;
            let entero = ore_core::normalize::qualify_catalogo(nombre, Some(&f.ns), &f.schema);
            let ficha = Json::obj([("url", Json::s(&url)), ("model", Json::s(&id))]);
            modelos.insert(nombre.to_string(), ficha.clone());
            modelos.insert(entero.clone(), ficha);
            usados.push(entero);
        }
        let modelos = Json::Obj(modelos);

        // Lo que puede leer: `over` y `reads`, en su contexto (0038).
        let cualificar = |v: &str| ore_core::normalize::qualify_catalogo(v, Some(&f.ns), &f.schema);
        let over = f
            .texto("over")
            .map(|o| ore_core::normalize::a_corto(&cualificar(&o)).into_owned());
        // Lo que trabaja fila a fila se puede leer desde el puesto —la regla
        // del puesto, no la del Job de `runtime: model`: una vista se lee como
        // pregunta sobre los datasets que tiene DEBAJO—, y si no, 409 antes de
        // encolar y no un error dentro del trabajo.
        if let Some(o) = &over {
            let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
            let se_lee = pkg.docs.iter().any(|d| {
                matches!(
                    d.kind,
                    ore_core::document::Kind::View | ore_core::document::Kind::Dataset
                ) && d.qname().as_deref() == Some(o.as_str())
                    && ore_core::vistas::se_lee_de_datasets(&pkg, d)
            });
            if !se_lee {
                return Err(Respuesta::error(
                    409,
                    format!(
                        "`{o}` no tiene ningún dataset debajo del que leer: una función lee la copia, nunca el origen (0029 ③). Declara un `Dataset` con `from` sobre ella"
                    ),
                ));
            }
        }
        let mut lee: Vec<String> = over.iter().cloned().collect();
        for r in f.spec.get("reads").map(|(_, v)| v.items()).unwrap_or(&[]) {
            if let Some(r) = r.as_str() {
                lee.push(ore_core::normalize::a_corto(&cualificar(r)).into_owned());
            }
        }

        // El código, tal como está en este commit.
        let entrypoint = f.texto("entrypoint").unwrap_or_default();
        let Some((ruta, def)) = ore_core::promover::entrypoint(&entrypoint) else {
            return Err(Respuesta::error(
                422,
                format!("`entrypoint: {entrypoint}` no es `<ruta>.py:<def>`"),
            ));
        };
        let mut carpeta = f.ruta.parent().map(Path::to_path_buf);
        while let Some(c) = &carpeta {
            if c.join("package.yaml").is_file() || c == raiz {
                break;
            }
            carpeta = c.parent().map(Path::to_path_buf);
        }
        let carpeta = carpeta.unwrap_or_else(|| raiz.to_path_buf());
        let fichero = carpeta.join(ruta);
        let Ok(fuente) = std::fs::read_to_string(&fichero) else {
            return Err(Respuesta::error(
                422,
                format!("`{ruta}` no está en el paquete (OOS2042)"),
            ));
        };
        let codigo = fichero
            .strip_prefix(raiz)
            .unwrap_or(&fichero)
            .display()
            .to_string()
            .replace('\\', "/");
        let commit = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(raiz)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "local".into());

        let plazo = f
            .spec
            .get("limits")
            .and_then(|(_, l)| l.get("timeout"))
            .and_then(|(_, t)| t.as_str())
            .and_then(segundos)
            .unwrap_or(0);
        let arnes = arnes(&Arnes {
            funcion: qn,
            fichero: &codigo,
            fuente: &fuente,
            def,
            parametros: &parametros,
            modelos: &modelos,
            over: over.as_deref(),
            output: &f.output(),
            plazo,
        });
        Ok(PlanPython {
            invocada: crate::puestos::Invocada {
                qn: qn.to_string(),
                corrida: corrida_ahora(),
                parametros,
                modelos: usados,
            },
            codigo,
            commit,
            arnes,
            lee,
        })
    }

    /// Lanza lo decidido: un trabajo con el arnés como celda, lo declarado
    /// como techo de lo que lee y nada que escribir.
    pub(crate) fn lanzar_funcion(&self, sujeto: &Identidad, plan: PlanPython) -> Respuesta {
        let rama = match self.rama_para_trabajo(sujeto) {
            Ok(r) => r,
            Err(r) => return r,
        };
        let qn = plan.invocada.qn.clone();
        let corrida = plan.invocada.corrida.clone();
        let mut r = self.lanzar_trabajo(
            sujeto,
            rama,
            "python",
            "python",
            plan.codigo,
            plan.commit,
            plan.arnes,
            Vec::new(),
            Some(crate::puestos::Transform {
                nombre: qn.clone(),
                inputs: plan.lee,
                // Nada: una función de lectura no escribe (v1alpha10 §1). El
                // catálogo compara la tabla escrita con esto, y nada es igual.
                output: String::new(),
            }),
            Some(plan.invocada),
        );
        if let Json::Obj(m) = &mut r.cuerpo {
            m.insert("function".into(), Json::s(&qn));
            m.insert("runtime".into(), Json::s("python"));
            m.insert("corrida".into(), Json::s(&corrida));
        }
        r
    }
}

/// `60s`, `5m`, `1h` → segundos.
fn segundos(d: &str) -> Option<u64> {
    let d = d.trim();
    let (n, u) = d.split_at(d.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = n.parse().ok()?;
    match u {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "ms" => Some(n.div_ceil(1000)),
        _ => None,
    }
}

/// `{"parametros": {...}}` contra `input`: los obligatorios están, no sobra
/// ninguno, y cada valor es de su tipo. El resultado es JSON con los tipos de
/// verdad (un `Decimal` como número), que es lo que el arnés le pasa al `def`.
fn parametros_de(f: &Funcion, cuerpo: &str) -> Result<Json, Respuesta> {
    let n = if cuerpo.trim().is_empty() {
        None
    } else {
        Some(parse::parse(cuerpo).map_err(|_| Respuesta::error(400, "el cuerpo no es JSON"))?)
    };
    let dados = n
        .as_ref()
        .and_then(|n| n.get("parametros"))
        .map(|(_, v)| v.entries())
        .unwrap_or(&[]);
    let declarados = f.spec.get("input").map(|(_, v)| v.entries()).unwrap_or(&[]);
    for (k, _) in dados {
        let k = k.as_str().unwrap_or("");
        if !declarados.iter().any(|(d, _)| d.as_str() == Some(k)) {
            return Err(Respuesta::error(
                422,
                format!("`{k}` no es un parámetro de la función: `input` no lo declara"),
            ));
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for (k, decl) in declarados {
        let k = k.as_str().unwrap_or("");
        let tipo = campo(decl, "type").unwrap_or_default();
        let obligatorio = campo(decl, "required").as_deref() == Some("true");
        match dados.iter().find(|(d, _)| d.as_str() == Some(k)) {
            None if obligatorio => {
                return Err(Respuesta::error(
                    422,
                    format!("falta `{k}`: es obligatorio ({tipo})"),
                ));
            }
            None => {}
            Some((_, v)) => {
                let j = valor_de(&tipo, v).map_err(|m| {
                    Respuesta::error(422, format!("`{k}` tiene que ser {tipo}: {m}"))
                })?;
                out.insert(k.to_string(), j);
            }
        }
    }
    Ok(Json::Obj(out))
}

/// Un valor del cuerpo, de su tipo de OOS.
fn valor_de(tipo: &str, v: &Node) -> Result<Json, String> {
    use ore_core::parse::Style;
    if let Some(dentro) = tipo.strip_prefix("list<").and_then(|t| t.strip_suffix('>')) {
        let Node::Sequence { items, .. } = v else {
            return Err("no es una lista".into());
        };
        return items
            .iter()
            .map(|i| valor_de(dentro, i))
            .collect::<Result<Vec<_>, _>>()
            .map(Json::Arr);
    }
    let (raw, plano) = match v {
        Node::Scalar { raw, style, .. } => (raw.as_str(), matches!(style, Style::Plain)),
        _ => return Err("no es un valor suelto".into()),
    };
    let base = tipo.split('<').next().unwrap_or(tipo).trim();
    match base {
        "Integer" => match (plano, raw.parse::<i64>()) {
            (true, Ok(i)) => Ok(Json::Int(i)),
            _ => Err(format!("`{raw}` no es un entero")),
        },
        "Decimal" | "Float" | "Money" | "Quantity" => match (plano, raw.parse::<f64>()) {
            (true, Ok(x)) if x.is_finite() => Ok(Json::Crudo(raw.to_string())),
            _ => Err(format!("`{raw}` no es un número")),
        },
        "Boolean" => match (plano, raw) {
            (true, "true") => Ok(Json::Bool(true)),
            (true, "false") => Ok(Json::Bool(false)),
            _ => Err(format!("`{raw}` no es `true` ni `false`")),
        },
        "Date" => {
            let b = raw.as_bytes();
            let ok = b.len() == 10
                && b.iter().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        *c == b'-'
                    } else {
                        c.is_ascii_digit()
                    }
                });
            if ok {
                Ok(Json::s(raw))
            } else {
                Err(format!("`{raw}` no es una fecha `AAAA-MM-DD`"))
            }
        }
        _ if plano && matches!(raw, "null" | "~") => Err("es nulo".into()),
        _ => Ok(Json::s(raw)),
    }
}

struct Arnes<'a> {
    funcion: &'a str,
    fichero: &'a str,
    fuente: &'a str,
    def: &'a str,
    parametros: &'a Json,
    modelos: &'a Json,
    over: Option<&'a str>,
    output: &'a [String],
    plazo: u64,
}

/// La celda que corre la función. Un literal de cadena JSON es un literal de
/// cadena de Python válido, así que todo lo que viene de fuera —el código, los
/// parámetros, los nombres— entra como cadena y se lee con `json.loads` o se
/// compila con `compile`: nada se interpola como código.
fn arnes(a: &Arnes<'_>) -> String {
    let cad = |s: &str| Json::s(s).jcs();
    let output = Json::Arr(a.output.iter().map(Json::s).collect()).jcs();
    format!(
        r#"# El arnés de una función de código (ORE 0050 P3): {funcion}
import dataclasses as _dc
import json as _json
import signal as _signal
import pyarrow as _pa

_PARAMETROS = _json.loads({parametros})
_MODELOS = _json.loads({modelos})
_OUTPUT = _json.loads({output})
_PLAZO = {plazo}


def _plazo(*_):
    raise TimeoutError("la invocación pasó de limits.timeout (%ss)" % _PLAZO)


if _PLAZO and hasattr(_signal, "SIGALRM"):
    _signal.signal(_signal.SIGALRM, _plazo)
    _signal.alarm(_PLAZO)

# Lo único que `ore.modelo()` deja llamar: lo declarado en `models` (0050 P4).
import ore as _ore

_ore._modelos_de_la_funcion(_MODELOS)

_modulo = {{"__name__": "ore_funcion", "__file__": {fichero}}}
exec(compile({fuente}, {fichero}, "exec"), _modulo)
_f = _modulo[{def_}]


def _una(*fila):
    try:
        v = _f(*fila, **_PARAMETROS)
    except TimeoutError:
        raise
    except Exception as e:  # noqa: BLE001 — una fila que falla se dice y se sigue
        return {{"_error": "%s: %s" % (type(e).__name__, e)}}
    if _dc.is_dataclass(v) and not isinstance(v, type):
        v = {{c.name: getattr(v, c.name) for c in _dc.fields(v)}}
    if _OUTPUT:
        if not isinstance(v, dict) or set(v) != set(_OUTPUT):
            dijo = sorted(v) if isinstance(v, dict) else type(v).__name__
            return {{"_error": "devolvió %s y `output` declara %s" % (dijo, _OUTPUT)}}
        return dict(v, _error=None)
    return {{"valor": _json.dumps(v, default=str), "_error": None}}


_over = {over}
if _over:
    _res = [_una(f) for f in over(_over, como="arrow").to_pylist()]
else:
    _res = [_una()]
if hasattr(_signal, "SIGALRM"):
    _signal.alarm(0)
_pa.Table.from_pylist(_res)
"#,
        funcion = a.funcion,
        parametros = cad(&a.parametros.jcs()),
        modelos = cad(&a.modelos.jcs()),
        output = cad(&output),
        plazo = a.plazo,
        fichero = cad(a.fichero),
        fuente = cad(a.fuente),
        def_ = cad(a.def),
        over = a.over.map(cad).unwrap_or_else(|| "None".into()),
    )
}

#[cfg(test)]
mod tests_python {
    use super::*;

    #[test]
    fn los_plazos_se_leen_en_segundos() {
        assert_eq!(segundos("60s"), Some(60));
        assert_eq!(segundos("5m"), Some(300));
        assert_eq!(segundos("1h"), Some(3600));
        assert_eq!(segundos("1500ms"), Some(2));
        assert_eq!(segundos("x"), None);
    }

    #[test]
    fn los_valores_se_leen_de_su_tipo() {
        let v = |s: &str| {
            parse::parse(&format!("{{\"v\": {s}}}"))
                .unwrap()
                .get("v")
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(valor_de("Integer", &v("3")).unwrap().jcs(), "3");
        assert!(valor_de("Integer", &v("\"3\"")).is_err());
        assert_eq!(valor_de("Decimal", &v("1.5")).unwrap().jcs(), "1.5");
        assert!(valor_de("Decimal", &v("\"x\"")).is_err());
        assert_eq!(valor_de("Boolean", &v("true")).unwrap().jcs(), "true");
        assert_eq!(
            valor_de("Date", &v("\"2026-10-01\"")).unwrap().jcs(),
            "\"2026-10-01\""
        );
        assert!(valor_de("Date", &v("\"01/10/2026\"")).is_err());
        assert_eq!(
            valor_de("list<Integer>", &v("[1, 2]")).unwrap().jcs(),
            "[1,2]"
        );
        assert_eq!(
            valor_de("String", &v("\"hola\"")).unwrap().jcs(),
            "\"hola\""
        );
    }

    #[test]
    fn el_arnes_no_interpola_codigo() {
        let p = Json::obj([("q", Json::s("\"); import os #"))]);
        let t = arnes(&Arnes {
            funcion: "ventas.riesgo",
            fichero: "packages/ventas/funciones/riesgo.py",
            fuente: "def riesgo(c, umbral):\n    return {\"n\": 1}\n",
            def: "riesgo",
            parametros: &p,
            modelos: &Json::obj([]),
            over: Some("ventas.clientes"),
            output: &["n".to_string()],
            plazo: 60,
        });
        // Lo de fuera va dentro de un literal de cadena JSON, escapado.
        assert!(
            t.contains(r#"_json.loads("{\"q\":\"\\\"); import os #\"}")"#),
            "{t}"
        );
        assert!(t.contains("_over = \"ventas.clientes\""));
        assert!(t.contains("_PLAZO = 60"));
        assert!(t.contains(r#"_f = _modulo["riesgo"]"#));
    }
}
