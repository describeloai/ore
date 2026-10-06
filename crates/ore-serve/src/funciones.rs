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
            // 0056 V2·3: la función propia (v1alpha26) no tiene `namespace`: su
            // espacio es `functions`, y su forma corta, `functions.<nombre>`.
            let ns = if ore_core::funcion_propia::es_propia(d) {
                Some(ore_core::funcion_propia::ESPACIO.to_string())
            } else {
                meta("namespace")
            };
            let (nombre, ns) = (meta("name")?, ns?);
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

/// **La función que una ruta pide** (0056 V2·3): la de ese nombre; y el de
/// antes de v1alpha26 —`/funciones/<base>/<n>`— sigue llevando a la función
/// propia que salió de esa base (su `entrypoint` está en `packages/<base>/`),
/// como en SQL y en `get_function`.
fn buscar<'a>(
    funciones: &'a [Funcion],
    ns: &str,
    schema: &str,
    nombre: &str,
) -> Option<&'a Funcion> {
    let pedida = ore_core::normalize::corto(ns, schema, nombre);
    funciones.iter().find(|f| f.qn() == pedida).or_else(|| {
        let desde = format!("packages/{ns}/");
        (schema == ore_core::normalize::SCHEMA_POR_DEFECTO
            && ns != ore_core::funcion_propia::ESPACIO)
            .then(|| {
                funciones.iter().find(|f| {
                    f.ns == ore_core::funcion_propia::ESPACIO
                        && f.nombre == nombre
                        && f.texto("entrypoint").is_some_and(|e| e.starts_with(&desde))
                })
            })
            .flatten()
    })
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
    let (sub, prefijo) = carpeta_y_prefijo(qn);
    let Ok(es) = std::fs::read_dir(raiz.join("resultados").join(&sub)) else {
        return Vec::new();
    };
    let ficheros = es
        .flatten()
        .filter_map(|e| {
            let nombre = e.file_name().to_str()?.to_string();
            let texto = std::fs::read_to_string(e.path()).ok()?;
            Some((nombre, texto, None))
        })
        .collect();
    informes(&prefijo, ficheros)
}

/// Dónde están los informes de `qn` dentro de `resultados/`, y cómo empiezan.
fn carpeta_y_prefijo(qn: &str) -> (String, String) {
    let nombre = ore_core::punteros::resultados_de(qn);
    let (sub, base) = nombre.rsplit_once('/').unwrap_or(("", &nombre));
    (sub.to_string(), format!("{base}_"))
}

/// Los informes, de `(fichero, texto, rama)`: los de esa función (lo que sigue
/// al prefijo es una corrida), cada uno con su `corrida` —y su `rama`, si no
/// es la que se lee—, del más reciente al más viejo y sin repetir.
fn informes(prefijo: &str, ficheros: Vec<(String, String, Option<String>)>) -> Vec<Json> {
    let mut por_corrida: std::collections::BTreeMap<String, Json> = Default::default();
    for (nombre, texto, rama) in ficheros {
        let Some(corrida) = nombre
            .strip_prefix(prefijo)
            .and_then(|r| r.strip_suffix(".json"))
            .filter(|c| es_corrida(c))
        else {
            continue;
        };
        if por_corrida.contains_key(corrida) {
            continue;
        }
        let Ok(n) = parse::parse(&texto) else {
            continue;
        };
        let Json::Obj(mut m) = de_node(&n) else {
            continue;
        };
        m.insert("corrida".into(), Json::s(corrida));
        if let Some(r) = rama {
            m.insert("rama".into(), Json::s(r));
        }
        por_corrida.insert(corrida.to_string(), Json::Obj(m));
    }
    por_corrida.into_values().rev().collect()
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
        let pedida = ore_core::normalize::corto(ns, schema, nombre);
        let Some(qn) = buscar(&funciones_de(raiz), ns, schema, nombre).map(Funcion::qn) else {
            return Respuesta::error(404, format!("no hay ninguna función `{pedida}`"));
        };
        // 0050 F4: los de `main` y los de cada rama. Una invocación confirma su
        // informe en la rama del puesto de quien la lanzó, no en `main`.
        let lista = match &self.arbol {
            crate::rutas::Arbol::Forja(forja) => {
                let (sub, prefijo) = carpeta_y_prefijo(&qn);
                let dir = if sub.is_empty() {
                    "resultados".to_string()
                } else {
                    format!("resultados/{sub}")
                };
                match forja.en_las_ramas(&dir, &prefijo) {
                    Ok(fs) => informes(
                        &prefijo,
                        fs.into_iter()
                            .map(|(rama, ruta, t)| {
                                let nombre = ruta.rsplit('/').next().unwrap_or(&ruta).to_string();
                                (nombre, t, rama)
                            })
                            .collect(),
                    ),
                    Err(_) => resultados_de(raiz, &qn),
                }
            }
            crate::rutas::Arbol::Directorio(_) => resultados_de(raiz, &qn),
        };
        Respuesta::ok(Json::obj([
            ("function", Json::s(&qn)),
            ("resultados", Json::Arr(lista)),
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
        let Some(f) = buscar(&funciones, ns, schema, nombre) else {
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

        // ── ②′ una función de código (0050 P3; TypeScript, R3 T5): se decide
        //    aquí y corre como un trabajo del puesto, que se lanza fuera de
        //    esta lectura ─────────────────────────────────────────────────
        if matches!(f.texto("runtime").as_deref(), Some("python" | "node")) {
            return match self.plan_python(raiz, f, &qn, cuerpo, sujeto) {
                Ok(p) => {
                    let r = Respuesta {
                        codigo: 202,
                        cuerpo: Json::obj([
                            ("function", Json::s(&qn)),
                            ("runtime", Json::s(p.runtime())),
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

/// Lo decidido al leer el árbol, para lanzar fuera de la lectura. De una
/// función de Python o, desde R3 T5, de TypeScript (`entorno: node`).
pub(crate) struct PlanPython {
    pub invocada: crate::puestos::Invocada,
    /// `packages/<p>/…/<f>.py` (o `.ts`): el fichero del árbol que corre.
    pub codigo: String,
    pub commit: String,
    pub arnes: String,
    pub lee: Vec<String>,
    /// `python` o `node`: la imagen del puesto que corre el trabajo.
    pub entorno: &'static str,
    /// El lenguaje de la celda: `python` o `typescript`.
    pub lenguaje: &'static str,
}

impl PlanPython {
    /// El `runtime` del documento.
    pub(crate) fn runtime(&self) -> &'static str {
        if self.entorno == "node" {
            "node"
        } else {
            "python"
        }
    }
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
        let node = f.texto("runtime").as_deref() == Some("node");
        if node && f.spec.get("models").is_some() {
            return Err(Respuesta::error(
                422,
                format!(
                    "`{qn}` declara `models`, y el SDK de Node no llama a un modelo todavía: no se finge"
                ),
            ));
        }

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
        let leido = if node {
            ore_core::promover::entrypoint_ts(&entrypoint).map(|r| (r, ""))
        } else {
            ore_core::promover::entrypoint(&entrypoint)
        };
        let Some((ruta, def)) = leido else {
            return Err(Respuesta::error(
                422,
                format!(
                    "`entrypoint: {entrypoint}` no es {}",
                    if node {
                        "`<ruta>.ts`"
                    } else {
                        "`<ruta>.py:<def>`"
                    }
                ),
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
        let a = Arnes {
            funcion: qn,
            fichero: &codigo,
            fuente: &fuente,
            def,
            parametros: &parametros,
            modelos: &modelos,
            over: over.as_deref(),
            plazo,
        };
        let arnes = if node {
            // Node borra los tipos antes de ejecutar: el contrato lee la firma
            // DERIVADA de este mismo fichero (la del documento, más la `forma`
            // que el documento no dice: si un entero llega como `bigint`).
            let firma = match ore_code::typescript::derivar(&fuente, ruta)
                .funciones
                .into_iter()
                .next()
                .map(|x| x.resultado)
            {
                Some(Ok(firma)) => firma,
                _ => {
                    return Err(Respuesta::error(
                        422,
                        format!("`{ruta}` no da la firma de una función (OOS2042/OOS2043)"),
                    ));
                }
            };
            arnes_node(&a, &firma_del_contrato(&firma))
        } else {
            arnes(&a)
        };
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
            entorno: if node { "node" } else { "python" },
            lenguaje: if node { "typescript" } else { "python" },
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
        // 0049 B4·2: lo que lee y es una colección, fijado al lanzar.
        let fijadas = self.fijar_colecciones(rama.as_deref(), &plan.lee);
        let runtime = plan.runtime();
        let mut r = self.lanzar_trabajo(
            sujeto,
            rama,
            plan.entorno,
            plan.lenguaje,
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
                fijadas,
            }),
            Some(plan.invocada),
        );
        if let Json::Obj(m) = &mut r.cuerpo {
            m.insert("function".into(), Json::s(&qn));
            m.insert("runtime".into(), Json::s(runtime));
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

/// Un valor del cuerpo, de su tipo de OOS. Lo que se comprueba aquí, antes de
/// encolar, es la forma (v1alpha20 `01` §7); el contrato del `def` lo convierte
/// al tipo de Python en el arnés y comprueba el resto.
fn valor_de(tipo: &str, v: &Node) -> Result<Json, String> {
    match ore_core::types::parse_type(tipo) {
        Ok(t) => valor_de_tipo(&t, v),
        // Un tipo que no es de OOS ya lo dice el árbol (OOS3001): aquí, tal cual.
        Err(_) => Ok(de_node(v)),
    }
}

/// `(cifras enteras, decimales)` de un número escrito en decimal.
fn cifras(raw: &str) -> (usize, usize) {
    let sin_signo = raw.trim_start_matches(['-', '+']);
    let (ent, dec) = sin_signo.split_once('.').unwrap_or((sin_signo, ""));
    let ent = ent.trim_start_matches('0');
    (ent.len(), dec.trim_end_matches('0').len())
}

fn valor_de_tipo(t: &ore_core::types::Type, v: &Node) -> Result<Json, String> {
    use ore_core::parse::Style;
    use ore_core::types::Type;
    match t {
        Type::List(dentro) => {
            let Node::Sequence { items, .. } = v else {
                return Err("no es una lista".into());
            };
            return items
                .iter()
                .map(|i| valor_de_tipo(dentro, i))
                .collect::<Result<Vec<_>, _>>()
                .map(Json::Arr);
        }
        // Un objeto con esos campos y ninguno más; uno que falta es nulo.
        Type::Struct(campos) => {
            let Node::Mapping { entries, .. } = v else {
                return Err("no es un objeto".into());
            };
            let mut out = std::collections::BTreeMap::new();
            for (k, x) in entries {
                let k = k.as_str().unwrap_or("");
                let Some((_, tk)) = campos.iter().find(|(n, _)| n == k) else {
                    return Err(format!("`{k}` no es un campo de `{t}`"));
                };
                let es_nulo =
                    matches!(x, Node::Scalar { raw, style: Style::Plain, .. } if raw == "null");
                let j = if es_nulo {
                    Json::Crudo("null".into())
                } else {
                    valor_de_tipo(tk, x).map_err(|m| format!("`{k}`: {m}"))?
                };
                out.insert(k.to_string(), j);
            }
            return Ok(Json::Obj(out));
        }
        // La referencia a un ítem: un objeto con dónde está (v1alpha16 `03` §2).
        Type::Media(_) => {
            let Node::Mapping { entries, .. } = v else {
                return Err("no es la referencia a un ítem (un objeto con `uri`, `path`…)".into());
            };
            if !entries
                .iter()
                .any(|(k, _)| matches!(k.as_str(), Some("uri" | "path")))
            {
                return Err("la referencia a un ítem lleva su `uri` o su `path`".into());
            }
            return Ok(de_node(v));
        }
        _ => {}
    }
    let (raw, plano) = match v {
        Node::Scalar { raw, style, .. } => (raw.as_str(), matches!(style, Style::Plain)),
        _ => return Err("no es un valor suelto".into()),
    };
    let numero = |raw: &str| -> Result<(), String> {
        match (plano, raw.parse::<f64>()) {
            (true, Ok(x)) if x.is_finite() => Ok(()),
            _ => Err(format!("`{raw}` no es un número")),
        }
    };
    match t {
        Type::Decimal { precision, escala } => {
            numero(raw)?;
            let (ent, dec) = cifras(raw);
            if dec > usize::from(*escala) || ent > usize::from(precision - escala) {
                return Err(format!("`{raw}` no cabe en `{t}`"));
            }
            return Ok(Json::Crudo(raw.to_string()));
        }
        Type::Parametric { precision, .. } => {
            numero(raw)?;
            if cifras(raw).1 > *precision as usize {
                return Err(format!("`{raw}` tiene más de {precision} decimales"));
            }
            return Ok(Json::Crudo(raw.to_string()));
        }
        _ => {}
    }
    let base = match t {
        Type::Scalar(s) => s.as_str(),
        _ => "",
    };
    match base {
        "Integer" => match (plano, raw.parse::<i64>()) {
            (true, Ok(i)) => Ok(Json::Int(i)),
            _ => Err(format!("`{raw}` no es un entero")),
        },
        "Decimal" | "Float" => numero(raw).map(|_| Json::Crudo(raw.to_string())),
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
        "Time" => {
            let partes: Vec<&str> = raw.split(':').collect();
            let ok = (2..=3).contains(&partes.len())
                && partes.iter().enumerate().all(|(i, p)| {
                    let p = if i == 2 {
                        p.split('.').next().unwrap_or("")
                    } else {
                        p
                    };
                    p.len() == 2 && p.bytes().all(|c| c.is_ascii_digit())
                });
            if ok {
                Ok(Json::s(raw))
            } else {
                Err(format!("`{raw}` no es una hora `HH:MM` o `HH:MM:SS`"))
            }
        }
        // Un instante lleva su zona: `Z` o `±HH:MM` al final.
        "DateTimeTz" => {
            let zona = raw.ends_with('Z') || {
                let b = raw.as_bytes();
                b.len() > 6 && matches!(b[b.len() - 6], b'+' | b'-') && b[b.len() - 3] == b':'
            };
            if raw.len() >= 16 && raw.as_bytes().get(10) == Some(&b'T') && zona {
                Ok(Json::s(raw))
            } else {
                Err(format!(
                    "`{raw}` no es un instante ISO 8601 con zona (`…Z` o `…+02:00`)"
                ))
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
    plazo: u64,
}

/// La celda que corre la función. Un literal de cadena JSON es un literal de
/// cadena de Python válido, así que todo lo que viene de fuera —el código, los
/// parámetros, los nombres— entra como cadena y se lee con `json.loads` o se
/// compila con `compile`: nada se interpola como código.
fn arnes(a: &Arnes<'_>) -> String {
    let cad = |s: &str| Json::s(s).jcs();
    format!(
        r#"# El arnés de una función de código (ORE 0050 P3, G3): {funcion}
import dataclasses as _dc
import decimal as _decimal
import json as _json
import signal as _signal
import traceback as _tb
import pyarrow as _pa

# Los números, exactos: un `Decimal` llega como `Decimal` (G3), y el contrato
# del `def` lo baja a `int` o `float` si es lo que anota.
_PARAMETROS = _json.loads({parametros}, parse_float=_decimal.Decimal)
_MODELOS = _json.loads({modelos})
_PLAZO = {plazo}
_FICHERO = {fichero}


def _plazo(*_):
    raise TimeoutError("la invocación pasó de limits.timeout (%ss)" % _PLAZO)


if _PLAZO and hasattr(_signal, "SIGALRM"):
    _signal.signal(_signal.SIGALRM, _plazo)
    _signal.alarm(_PLAZO)

# Lo único que `ore.model()` deja llamar: lo declarado en `models` (0050 P4).
{guarda}from ore.contrato import llamada as _llamada

_ore._modelos_de_la_funcion(_MODELOS)

_modulo = {{"__name__": "ore_funcion", "__file__": _FICHERO}}
exec(compile({fuente}, _FICHERO, "exec"), _modulo)
_f = _modulo[{def_}]
# Su contrato (G3, `ore.contrato`): cada parámetro del tipo que anota, y lo
# que devuelve del tipo que anota. El `@function` del SDK ya lo trae.
_f = _f if getattr(_f, "__ore_contrato__", False) else _llamada(_f)


def _donde(e):
    """Dónde se rompió, en el `.py` de la función."""
    for fr in reversed(_tb.extract_tb(e.__traceback__)):
        if fr.filename == _FICHERO:
            return " (%s, línea %d)" % (_FICHERO, fr.lineno)
    return ""


def _plano(v):
    """Lo compuesto, como objetos y listas: un `Struct`, una lista de ellos, una
    referencia a un ítem (v1alpha20)."""
    if _dc.is_dataclass(v) and not isinstance(v, type):
        return {{c.name: _plano(getattr(v, c.name)) for c in _dc.fields(v)}}
    if isinstance(v, (list, tuple)):
        return [_plano(x) for x in v]
    return v


def _fila(v):
    if _dc.is_dataclass(v) and not isinstance(v, type):
        return _plano(v)
    return {{"valor": _plano(v)}}


_over = {over}
if _over:
    # Una llamada por fila: la que falla se dice en `_error`, y las demás siguen.
    _res = []
    for _f_ in over(_over, format="arrow").to_pylist():
        try:
            _res.append(dict(_fila(_f(_f_, **_PARAMETROS)), _error=None))
        except TimeoutError:
            raise
        except Exception as e:  # noqa: BLE001
            _res.append({{"_error": "%s: %s%s" % (type(e).__name__, e, _donde(e))}})
else:
    # Una llamada: si falla, falla la invocación, con su porqué y su línea.
    try:
        _v = _f(**_PARAMETROS)
    except TimeoutError:
        raise
    except Exception as e:  # noqa: BLE001
        raise RuntimeError("%s: %s%s" % (type(e).__name__, e, _donde(e))) from None
    _res = [_fila(_v)]
if hasattr(_signal, "SIGALRM"):
    _signal.alarm(0)
_pa.Table.from_pylist(_res)
"#,
        funcion = a.funcion,
        guarda = ore_core::sdk::guarda_python(),
        parametros = cad(&a.parametros.jcs()),
        modelos = cad(&a.modelos.jcs()),
        plazo = a.plazo,
        fichero = cad(a.fichero),
        fuente = cad(a.fuente),
        def_ = cad(a.def),
        over = a.over.map(cad).unwrap_or_else(|| "None".into()),
    )
}

/// La firma que lee el contrato de Node (`ore/contract.mjs`): la derivada, con
/// cada tipo en su `forma` (`BigInt` para un entero que el código declaró
/// `bigint`).
fn firma_del_contrato(f: &ore_code::Firma) -> Json {
    let campos = |cs: &[ore_code::Campo]| {
        Json::Arr(
            cs.iter()
                .map(|c| {
                    Json::obj([
                        ("name", Json::s(&c.nombre)),
                        ("type", Json::s(c.tipo.forma())),
                        ("required", Json::Bool(c.requerido)),
                    ])
                })
                .collect(),
        )
    };
    Json::obj([
        ("input", campos(&f.entrada)),
        (
            "output",
            match &f.salida {
                ore_code::Salida::Valor(t) => Json::obj([("type", Json::s(t.forma()))]),
                ore_code::Salida::Campos(cs) => Json::obj([("fields", campos(cs))]),
            },
        ),
        ("over", Json::Bool(f.over.is_some())),
    ])
}

/// La celda que corre una función de TypeScript (R3 T5): una celda-módulo (el
/// agente de Node importa lo que empieza por `import`, con `await` arriba).
/// Borra los tipos del fichero (`stripTypeScriptTypes`, que deja cada cosa en
/// su línea), lo importa, llama a su `export default` con `contract.call` y la
/// firma derivada, una vez o una por fila de `over`, y exporta por defecto las
/// filas: el agente las devuelve como la tabla del resultado, como el
/// `pa.Table` del arnés de Python. Un literal JSON es un literal de JavaScript:
/// lo de fuera entra como cadena, nada se interpola como código.
fn arnes_node(a: &Arnes<'_>, firma: &Json) -> String {
    let cad = |s: &str| Json::s(s).jcs();
    format!(
        r#"// El arnés de una función de TypeScript (ORE 0050 R3 T5): {funcion}
import {{ contract, over }} from "ore";
import {{ stripTypeScriptTypes }} from "node:module";
import {{ writeFileSync, rmSync }} from "node:fs";
import {{ dirname, join }} from "node:path";
import {{ fileURLToPath, pathToFileURL }} from "node:url";

const FICHERO = {fichero};
const FIRMA = JSON.parse({firma});
const PLAZO = {plazo};
const OVER = {over};
// Los números, exactos: uno que un `number` no guarda tal cual llega como el
// texto que se escribió (un entero de 64 bits, un decimal largo), y el
// contrato lo convierte a lo que la firma declara.
const PARAMETROS = JSON.parse({parametros}, (k, v, c) => {{
  if (typeof v !== "number" || !c || typeof c.source !== "string" || /[eE]/.test(c.source)) return v;
  const escrito = c.source.includes(".") ? c.source.replace(/0+$/, "").replace(/\.$/, "") : c.source;
  return String(v) === escrito ? v : c.source;
}});

const f = join(dirname(fileURLToPath(import.meta.url)), "funcion-" + Date.now() + ".mjs");
const url = pathToFileURL(f).href;
writeFileSync(f, stripTypeScriptTypes({fuente}, {{ mode: "strip" }}));
let m;
try {{
  m = await import(url);
}} finally {{
  rmSync(f, {{ force: true }});
}}
if (typeof m.default !== "function") throw new TypeError(FICHERO + " no exporta una función por defecto");

/** Dónde se rompió, en el `.ts` de la función. */
const donde = (e) => {{
  const r = String(e?.stack ?? "").match(new RegExp(url.replace(/[.*+?^${{}}()|[\]\\]/g, "\\$&") + ":(\\d+)"));
  return r ? ` (${{FICHERO}}, línea ${{r[1]}})` : "";
}};
const conPlazo = (p) => {{
  if (!PLAZO) return p;
  let t;
  const corte = new Promise((_, no) => {{ t = setTimeout(() => no(new Error(`la invocación pasó de limits.timeout (${{PLAZO}}s)`)), PLAZO * 1000); }});
  return Promise.race([p, corte]).finally(() => clearTimeout(t));
}};
const fila = (v) => (FIRMA.output.fields ? v : {{ valor: v }});
const llamar = (row) => contract.call(m.default, FIRMA, PARAMETROS, {{ row, name: {nombre} }}).then((v) => contract.toWire(v));

let filas;
if (OVER) {{
  // Una llamada por fila: la que falla se dice en `_error`, y las demás siguen.
  filas = await conPlazo((async () => {{
    const out = [];
    for (const r of await over(OVER)) {{
      try {{
        out.push({{ ...fila(await llamar(r)), _error: null }});
      }} catch (e) {{
        out.push({{ _error: `${{e?.name ?? "Error"}}: ${{e?.message ?? e}}${{donde(e)}}` }});
      }}
    }}
    return out;
  }})());
}} else {{
  // Una llamada: si falla, falla la invocación, con su porqué y su línea.
  try {{
    filas = [fila(await conPlazo(llamar(undefined)))];
  }} catch (e) {{
    throw new Error(`${{e?.name ?? "Error"}}: ${{e?.message ?? e}}${{donde(e)}}`);
  }}
}}
export default filas;
"#,
        funcion = a.funcion,
        fichero = cad(a.fichero),
        firma = cad(&firma.jcs()),
        plazo = a.plazo,
        over = a.over.map(cad).unwrap_or_else(|| "null".into()),
        parametros = cad(&a.parametros.jcs()),
        fuente = cad(a.fuente),
        nombre = cad(a.funcion),
    )
}

#[cfg(test)]
mod tests_python {
    use super::*;

    /// R3 T5: el arnés de una función de TypeScript lleva la firma DERIVADA
    /// (con `BigInt` donde el código dijo `bigint`), el fuente y los
    /// parámetros como cadenas, y exporta las filas por defecto.
    #[test]
    fn el_arnes_de_node_lleva_la_firma_derivada() {
        let fuente = "import type { Integer } from \"ore\";\n\
                      export default function repeat(id: bigint, n: Integer = 1): string { return \"x\"; }\n";
        let d = ore_code::typescript::derivar(fuente, "t/functions/repeat.ts");
        let firma = d.funciones[0].resultado.as_ref().unwrap();
        let f = firma_del_contrato(firma);
        assert_eq!(
            f.jcs(),
            r#"{"input":[{"name":"id","required":true,"type":"BigInt"},{"name":"n","required":false,"type":"Integer"}],"output":{"type":"String"},"over":false}"#
        );
        let parametros = Json::obj([("id", Json::s("1"))]);
        let modelos = Json::Obj(Default::default());
        let a = Arnes {
            funcion: "ventas.repeat",
            fichero: "packages/ventas/t/functions/repeat.ts",
            fuente,
            def: "",
            parametros: &parametros,
            modelos: &modelos,
            over: None,
            plazo: 30,
        };
        let celda = arnes_node(&a, &f);
        assert!(celda.starts_with("// El arnés de una función de TypeScript"));
        assert!(celda.contains("\nimport { contract, over } from \"ore\";\n"));
        assert!(celda.contains("const PLAZO = 30;"));
        assert!(celda.contains("const OVER = null;"));
        assert!(celda.contains(r#"JSON.parse("{\"input\":[{\"name\":\"id\""#));
        assert!(celda.contains("stripTypeScriptTypes(\"import type { Integer } from \\\"ore\\\";"));
        assert!(celda.trim_end().ends_with("export default filas;"));
        // Sin `{` ni `}` sueltos de `format!`: cada `${…}` del JavaScript, entero.
        assert!(celda.contains("`la invocación pasó de limits.timeout (${PLAZO}s)`"));
    }

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

    /// v1alpha20 `01` §7: la forma de cada tipo nuevo en el cuerpo de una
    /// invocación, antes de encolar.
    #[test]
    fn los_valores_de_v1alpha20_se_leen_de_su_tipo() {
        let v = |s: &str| {
            parse::parse(&format!("{{\"v\": {s}}}"))
                .unwrap()
                .get("v")
                .unwrap()
                .1
                .clone()
        };
        let ok = |t: &str, s: &str| valor_de(t, &v(s)).map(|j| j.jcs());
        assert_eq!(ok("Decimal<5, 2>", "123.45").unwrap(), "123.45");
        assert!(
            ok("Decimal<5, 2>", "1234.5")
                .unwrap_err()
                .contains("no cabe")
        );
        assert!(
            ok("Decimal<5, 2>", "1.234")
                .unwrap_err()
                .contains("no cabe")
        );
        assert_eq!(ok("Money<EUR, 2>", "12.50").unwrap(), "12.50");
        assert!(
            ok("Money<EUR, 2>", "1.234")
                .unwrap_err()
                .contains("2 decimales")
        );
        assert_eq!(ok("Time", "\"08:30\"").unwrap(), "\"08:30\"");
        assert!(ok("Time", "\"8h\"").is_err());
        assert!(ok("DateTimeTz", "\"2026-10-02T08:00:00+02:00\"").is_ok());
        assert!(ok("DateTimeTz", "\"2026-10-02T08:00:00Z\"").is_ok());
        assert!(
            ok("DateTimeTz", "\"2026-10-02T08:00:00\"")
                .unwrap_err()
                .contains("con zona")
        );
        assert_eq!(
            ok(
                "Struct<a: Integer, b: Money<EUR, 2>>",
                "{\"a\": 1, \"b\": 2.5}"
            )
            .unwrap(),
            "{\"a\":1,\"b\":2.5}"
        );
        assert!(
            ok("Struct<a: Integer>", "{\"c\": 1}")
                .unwrap_err()
                .contains("`c` no es un campo")
        );
        assert!(
            ok("Struct<a: Integer>", "{\"a\": \"x\"}")
                .unwrap_err()
                .contains("`a`")
        );
        assert_eq!(
            ok("list<Struct<a: Integer>>", "[{\"a\": 1}, {\"a\": null}]").unwrap(),
            "[{\"a\":1},{\"a\":null}]"
        );
        assert!(
            ok(
                "Media<legal.archivo.contratos>",
                "{\"uri\": \"s3://b/c.pdf\"}"
            )
            .is_ok()
        );
        assert!(ok("Media<legal.archivo.contratos>", "\"c.pdf\"").is_err());
        assert_eq!(ok("Opaque", "\"aG9sYQ==\"").unwrap(), "\"aG9sYQ==\"");
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
            plazo: 60,
        });
        // Lo de fuera va dentro de un literal de cadena JSON, escapado.
        assert!(
            t.contains(r#"_json.loads("{\"q\":\"\\\"); import os #\"}", parse_float"#),
            "{t}"
        );
        assert!(t.contains("_over = \"ventas.clientes\""));
        assert!(t.contains("_PLAZO = 60"));
        assert!(t.contains(r#"_f = _modulo["riesgo"]"#));
        // S3: con la guarda de la versión del SDK y sin un nombre de antes.
        assert!(t.contains(&ore_core::sdk::guarda_python()), "{t}");
        let antes = ore_core::sdk::nombres_de_antes_en(&t);
        assert!(antes.is_empty(), "{antes:?}");
        if let Ok(dir) = std::env::var("ORE_CELDAS_GENERADAS") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("90-arnes.py"), &t).unwrap();
        }
    }
}
