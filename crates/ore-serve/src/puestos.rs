//! **El puesto** (ADR 0031, W3.1): la sesión viva de una persona en su celda.
//!
//! Un puesto es un Job de Kueue que **no termina**: la imagen de un **entorno**
//! (`puesto-python:1`, `puesto-node:1`, `puesto-jvm:1` — W3.4) con su agente
//! dentro (`puesto/python/agente.py`, `puesto/node/agente.mjs`,
//! `puesto/jvm/ore/Agente.java`), la identidad del pod y la cola del
//! inquilino. Este servidor no toca Kubernetes: **escribe la cola**
//! (`51-el-puesto-<persona>-<entorno>.yaml`, rendido de `plantilla-puesto.txt`)
//! y Flux rinde el Job, como con la copia y la invocación. Retirarlo es quitar
//! el fichero (`prune: true`).
//!
//! # Lenguajes y entornos
//!
//! Una celda lleva un **lenguaje** (`python`, `sql`, `typescript`, `javascript`,
//! `java`); un puesto es de un **entorno** (`python`, `node`, `jvm`). Uno por
//! persona **y entorno**: `puesto-<persona>-<entorno>`. `python` corre en
//! `python`, `typescript`/`javascript` en `node`, `java` en `jvm`, y `sql` en
//! cualquiera (los tres agentes llevan DuckDB y el mismo `sql()`).
//!
//! # Sin entrada
//!
//! El puesto no acepta conexiones. El agente **pide trabajo** aquí por polling
//! largo y **entrega la salida** aquí; la consola **manda celdas** aquí y
//! **espera la salida** aquí. Todo el estado vivo —qué celdas hay, cuál corre,
//! qué salió— está en memoria de este proceso: un puesto que este servidor no
//! recuerda (tras un relevo) contesta 410 al agente y el agente se cierra.
//!
//! ```text
//! la persona                                  el agente (en el pod)
//!   POST /puestos            → encolado          GET  /puestos/{id}/pendiente   (20 s)
//!   GET  /puestos/{id}       → encolado|vivo     POST /puestos/{id}/celdas/{n}/salida
//!   POST /puestos/{id}/ejecutar {texto} → celda  GET  /puestos/{id}/datos/{vista}
//!   GET  /puestos/{id}/celdas/{n}  (20 s) → salida
//!   DELETE /puestos/{id}     → cerrado
//! ```
//!
//! # Quién
//!
//! Las rutas de la persona exigen que el puesto sea **suyo**. Las del agente
//! exigen un **agente** (`rubix_tipo: agente` por OIDC; `agente:…` por cabecera
//! en las pruebas) y atan el puesto al **primero** que lo reclama: otro agente
//! es 403. `datos` resuelve la vista **en nombre de la persona** dueña, con lo
//! que el árbol dice de su copia (`copias/<p>_<v>.json`), y devuelve la clave:
//! el pod la baja con su propia identidad; el código nunca ve una credencial.
//!
//! # Vida
//!
//! El TTL de inactividad lo lleva el agente (sin celdas N segundos, se va); el
//! tope, el Job (`activeDeadlineSeconds`). Aquí, un puesto sin latido en
//! [`SIN_LATIDO`] pasa a `perdido` y una celda que se le mande contesta 409.

use crate::cola;
use crate::rutas::Servidor;

/// La cabecera con la que el agente de un puesto dice desde qué puesto
/// escribe: el sujeto pasa a ser la persona que lo abrió (y la rama, la del
/// puesto).
pub(crate) const PUESTO: &str = "x-ore-puesto";
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Cuánto se retiene una espera (el agente pidiendo trabajo, la consola
/// esperando una salida). Por debajo del plazo de lectura de `http.rs` (30 s).
const ESPERA: Duration = Duration::from_secs(20);
/// Sin latido del agente durante esto, el puesto está perdido.
const SIN_LATIDO: Duration = Duration::from_secs(90);
/// Encolado sin que ningún agente lo reclame durante esto, el puesto está
/// perdido (el Job no llegó a arrancar, o alguien se lo llevó de la cola: en
/// frío el nodo tarda ~2 min; esto es cinco veces eso).
const SIN_ARRANCAR: Duration = Duration::from_secs(600);

/// Un puesto que no va a contestar: vivo sin latido, o encolado sin arrancar.
fn perdido(p: &Puesto) -> bool {
    match p.estado {
        Estado::Vivo => p.latido.is_some_and(|l| l.elapsed() > SIN_LATIDO),
        Estado::Encolado => p.creado.elapsed() > SIN_ARRANCAR,
        Estado::Cerrado => false,
    }
}
/// Una celda no puede ser un fichero.
const TEXTO_MAXIMO: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Estado {
    /// En la cola: Flux aún no lo rindió, o el pod aún no arrancó.
    Encolado,
    /// El agente pide trabajo.
    Vivo,
    /// Cerrado por la persona (y fuera de la cola).
    Cerrado,
}

impl Estado {
    fn dice(self) -> &'static str {
        match self {
            Estado::Encolado => "encolado",
            Estado::Vivo => "vivo",
            Estado::Cerrado => "cerrado",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Celda {
    pub texto: String,
    /// `python` (por defecto) o `sql` (W3.3: la consulta entera a `ore.sql`).
    pub lenguaje: String,
    pub enviada: Instant,
    pub empezada: Option<Instant>,
    pub salida: Option<Json>,
}

/// **Un trabajo** (0031 §9, W3.7 ④): un fichero del árbol corrido como una
/// sola celda en un Job que termina. Es un puesto con `trabajo` puesto: la
/// misma cola, la misma plantilla (con `TRABAJO`), el mismo agente (que sale
/// tras la celda), y el informe en `trabajos/<id>.json` del árbol.
#[derive(Debug, Clone)]
pub(crate) struct Trabajo {
    /// La ruta del fichero en el árbol (`packages/<p>/transforms/x.py`).
    pub codigo: String,
    /// El commit del que se leyó (`local` sobre un directorio).
    pub commit: String,
    /// El informe, cuando la celda terminó: qué quedó en `trabajos/`.
    pub informe: Option<Json>,
}

#[derive(Debug)]
pub(crate) struct Puesto {
    pub persona: String,
    /// `python`, `node` o `jvm`: la imagen (`cola::ENTORNOS`).
    pub entorno: String,
    pub rama: Option<String>,
    pub fichero: String,
    pub job: String,
    pub creado: Instant,
    pub estado: Estado,
    pub agente: Option<String>,
    pub latido: Option<Instant>,
    pub siguiente: u64,
    pub pendientes: VecDeque<u64>,
    pub celdas: BTreeMap<u64, Celda>,
    pub trabajo: Option<Trabajo>,
    /// **Dónde vive** (0036 ④): la carpeta del repositorio, si se dijo. De ella
    /// salen su capa, su rama y su clase.
    pub repositorio: Option<String>,
    /// **La clase de su repositorio** (0036 ⑤), resuelta al abrir contra la
    /// tabla del producto. Es un **techo**: lo que la clase no deja, no se
    /// hace —aunque el código lo declare—, y lo que deja lo sigue decidiendo
    /// el gobierno de siempre.
    pub clase: Option<&'static ore_core::clases::Clase>,
    /// **Lo que el transform que corre declaró** (0031 W3.7 gobierno ⑤). El
    /// SDK lo dice al entrar en `@transform(inputs, output)` y lo retira al
    /// salir; mientras está, el servidor sólo resuelve sus `inputs` y sólo
    /// deja escribir su `output`. Hasta ⑤ el 403 vivía sólo en el SDK y
    /// `ore.puesto.pedir()` a pelo lo rodeaba (medido).
    pub transform: Option<Transform>,
}

/// Lo declarado por el transform que corre en este puesto.
#[derive(Debug, Clone)]
pub(crate) struct Transform {
    pub nombre: String,
    pub inputs: Vec<String>,
    pub output: String,
}

/// Todo lo vivo, bajo un candado, y una campana para las esperas.
#[derive(Default)]
pub(crate) struct Puestos {
    lista: Mutex<BTreeMap<String, Puesto>>,
    campana: Condvar,
}

impl std::fmt::Debug for Puestos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Puestos")
    }
}

/// `persona:ana` + `node` → `puesto-ana-node`; un `sub` opaco, recortado y en
/// minúsculas. Uno por persona, entorno **y repositorio** (0036 ④).
///
/// Hasta 0036 era uno por persona y entorno: dos repositorios de la misma
/// persona recibían **el mismo puesto** (medido, 0035 ⑥ §4), y con él la misma
/// capa, la misma rama y el mismo «lo mío». El repositorio es la unidad de
/// trabajo, así que la sesión es suya: del alcance se toma **la última
/// carpeta**, que es como se llama el repositorio para quien trabaja.
pub(crate) fn id_de(persona: &str, entorno: &str, repositorio: Option<&str>) -> String {
    let corto = |s: &str, n: usize| {
        let s = cola::nombre_de_objeto(s);
        if s.len() > n {
            s[..n].trim_end_matches('-').to_string()
        } else {
            s
        }
    };
    let quien = corto(persona.rsplit(':').next().unwrap_or(persona), 24);
    match repositorio
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|r| r.trim_matches('/').rsplit('/').next().map(str::to_string))
    {
        Some(repo) => format!("puesto-{quien}-{entorno}-{}", corto(&repo, 20)),
        None => format!("puesto-{quien}-{entorno}"),
    }
}

/// Los lenguajes que una celda puede llevar.
const LENGUAJES: [&str; 5] = ["python", "sql", "typescript", "javascript", "java"];

/// En qué entorno corre un lenguaje (`sql`: en el que haya → `python` si hay que
/// abrir uno). Un nombre de entorno vale como lenguaje al abrir.
pub(crate) fn entorno_de(lenguaje: &str) -> Option<&'static str> {
    match lenguaje {
        "python" | "sql" => Some("python"),
        "typescript" | "javascript" | "node" => Some("node"),
        "java" | "jvm" => Some("jvm"),
        _ => None,
    }
}

/// ¿Corre este lenguaje en este entorno? `sql` corre en todos.
fn corre_en(lenguaje: &str, entorno: &str) -> bool {
    lenguaje == "sql" || entorno_de(lenguaje) == Some(entorno)
}

pub(crate) fn es_agente(sujeto: &Identidad) -> bool {
    sujeto.tipo.as_deref() == Some("agente") || sujeto.persona.starts_with("agente:")
}

fn ficha(id: &str, p: &Puesto) -> Json {
    let estado = if perdido(p) {
        "perdido"
    } else {
        p.estado.dice()
    };
    let mut f = ficha_base(id, p, estado);
    // Dónde vive y de qué clase es (0036 ④ y ⑤): quien mire el puesto ve el
    // repositorio, su plantilla y si esa clase deja escribir datos.
    if let Json::Obj(m) = &mut f {
        if let Some(r) = &p.repositorio {
            m.insert("repositorio".into(), Json::s(r));
        }
        if let Some(c) = p.clase {
            m.insert("plantilla".into(), Json::s(c.id));
            m.insert("escribe".into(), Json::Bool(c.escribe));
        }
    }
    // Lo declarado, mientras corre (⑤): quien mire el puesto ve qué transform
    // hay dentro y qué dijo que iba a leer y escribir.
    if let (Some(t), Json::Obj(m)) = (&p.transform, &mut f) {
        m.insert("transform".into(), Json::s(&t.nombre));
        m.insert(
            "inputs".into(),
            Json::Arr(t.inputs.iter().map(Json::s).collect()),
        );
        m.insert("output".into(), Json::s(&t.output));
    }
    if let (Some(t), Json::Obj(m)) = (&p.trabajo, &mut f) {
        m.insert("codigo".into(), Json::s(&t.codigo));
        m.insert("commit".into(), Json::s(&t.commit));
        m.insert(
            "trabajo".into(),
            Json::s(match (&t.informe, estado) {
                (Some(_), _) => "hecho",
                (None, "cerrado") => "cerrado",
                (None, "perdido") => "perdido",
                (None, "vivo") => "corriendo",
                _ => "encolado",
            }),
        );
        if let Some(i) = &t.informe {
            m.insert("informe".into(), i.clone());
        }
    }
    f
}

fn ficha_base(id: &str, p: &Puesto, estado: &str) -> Json {
    Json::obj([
        ("id", Json::s(id)),
        ("persona", Json::s(&p.persona)),
        ("entorno", Json::s(&p.entorno)),
        ("rama", Json::s(p.rama.clone().unwrap_or_default())),
        ("estado", Json::s(estado)),
        ("job", Json::s(&p.job)),
        ("fichero", Json::s(&p.fichero)),
        ("segundos", Json::Int(p.creado.elapsed().as_secs() as i64)),
        ("celdas", Json::Int(p.celdas.len() as i64)),
        ("pendientes", Json::Int(p.pendientes.len() as i64)),
        (
            "ultimo_latido_hace",
            Json::Int(p.latido.map(|l| l.elapsed().as_secs() as i64).unwrap_or(-1)),
        ),
    ])
}

fn ficha_de_celda(n: u64, c: &Celda) -> Json {
    let estado = if c.salida.is_some() {
        "hecha"
    } else if c.empezada.is_some() {
        "corriendo"
    } else {
        "pendiente"
    };
    let mut m = BTreeMap::new();
    m.insert("celda".to_string(), Json::Int(n as i64));
    m.insert("estado".to_string(), Json::s(estado));
    m.insert(
        "espera_ms".to_string(),
        Json::Int(c.enviada.elapsed().as_millis() as i64),
    );
    if let Some(s) = &c.salida {
        m.insert("salida".to_string(), s.clone());
    }
    Json::Obj(m)
}

impl Servidor {
    // ── la persona ──────────────────────────────────────────────────────────

    /// `POST /puestos {lenguaje?, rama?}`: el puesto de la persona en esta
    /// celda para ese lenguaje (o entorno). Uno por persona y entorno: si ya
    /// lo tiene, 200 con el que hay.
    pub(crate) fn abrir_puesto(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        if es_agente(sujeto) {
            return Respuesta::error(403, "un agente no abre puestos: los abre una persona");
        }
        let (lenguaje, rama, repositorio) = if cuerpo.trim().is_empty() {
            ("python".to_string(), None, None)
        } else {
            let n = match ore_core::parse::parse(cuerpo) {
                Ok(n) => n,
                Err(_) => return Respuesta::error(400, "el cuerpo no es JSON"),
            };
            let l = n
                .get("lenguaje")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("python")
                .to_string();
            let r = n
                .get("rama")
                .and_then(|(_, v)| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            // ⭐ El repositorio (0036 ③): la capa con la que nace el puesto es
            //   LA SUYA —la raíz, su paquete y él—, no la unión de la celda.
            let rep = n
                .get("repositorio")
                .and_then(|(_, v)| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            (l, r, rep)
        };
        let repositorio = match crate::entorno::alcance_valido(repositorio.as_deref()) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let Some(entorno) = entorno_de(&lenguaje) else {
            return Respuesta::error(
                422,
                format!(
                    "`{lenguaje}` no tiene puesto: python, sql, typescript, javascript o java (o un entorno: {})",
                    cola::ENTORNOS.join(", ")
                ),
            );
        };
        if let Some(r) = &rama
            && let Err(m) = crate::propuestas::nombre_de_rama_valido(r)
        {
            return Respuesta::error(422, m);
        }
        // ⭐ La clase del repositorio (0036 ⑤): se lee de SU manifiesto y se
        //   guarda con el puesto. Una clase que no ejecuta —`semantics`— no
        //   abre sesión: lo suyo son documentos del árbol, y decirlo aquí
        //   ahorra abrir un pod para nada.
        let clase = match &repositorio {
            None => None,
            Some(a) => {
                let r = self.leyendo_en(
                    rama.as_deref(),
                    |raiz| match ore_core::repositorios::leer(raiz)
                        .into_iter()
                        .find(|r| r.ruta == *a)
                    {
                        None => Respuesta::error(404, format!("`{a}` no es un repositorio")),
                        Some(r) => Respuesta::ok(Json::obj([(
                            "plantilla",
                            r.plantilla
                                .as_deref()
                                .map(Json::s)
                                .unwrap_or(Json::Crudo("null".into())),
                        )])),
                    },
                );
                if r.codigo != 200 {
                    return r;
                }
                let id = match &r.cuerpo {
                    Json::Obj(m) => match m.get("plantilla") {
                        Some(Json::Str(s)) => s.clone(),
                        _ => String::new(),
                    },
                    _ => String::new(),
                };
                let c = ore_core::clases::de(&id);
                if let Some(c) = c
                    && !c.ejecuta
                {
                    return Respuesta::error(
                        422,
                        format!(
                            "un repositorio `{}` no ejecuta: lo suyo son documentos del árbol, que se escriben por `/documentos` y `/arbol`",
                            c.id
                        ),
                    );
                }
                c
            }
        };
        let rama = match self.rama_del_puesto(sujeto, rama, repositorio.as_deref()) {
            Ok(r) => r,
            Err(r) => return r,
        };
        let id = id_de(&sujeto.persona, entorno, repositorio.as_deref());
        {
            let lista = self.puestos.lista.lock().unwrap();
            // Uno por persona: si lo tiene y da señales (o aún arranca), es ése.
            // Uno PERDIDO (vivo sin latido: TTL, tope o relevo; o encolado
            // que nunca arrancó) se sustituye.
            if let Some(p) = lista.get(&id)
                && p.estado != Estado::Cerrado
                && !perdido(p)
            {
                return Respuesta::ok(ficha(&id, p));
            }
        }
        // ⭐ La capa (0031 W3.2): lo que el árbol declara, resuelto. Lista →
        //   el puesto nace con ella; pendiente → se encola y 409 para que la
        //   consola espere; error → 409 con el motivo (y se reintenta la capa).
        //   Hoy la capa es de Python (`pyproject.toml` → ruedas); `node` y `jvm`
        //   nacen con lo que trae su imagen (W3.4; la suya, cuando se mida).
        let e = if entorno != "python" {
            Json::obj([("estado", Json::s("sin-dependencias"))])
        } else {
            match self.leyendo_en(rama.as_deref(), |raiz| {
                if let Some(a) = &repositorio
                    && !raiz.join(a).is_dir()
                {
                    return Respuesta::error(404, format!("no hay `{a}` en el árbol"));
                }
                let e = crate::entorno::entorno_de_en(raiz, repositorio.as_deref());
                Respuesta::ok(Json::obj([
                    ("estado", Json::s(e.estado)),
                    ("digest", Json::s(&e.digest)),
                    (
                        "declarado",
                        Json::Arr(e.declarado.iter().map(Json::s).collect()),
                    ),
                    ("informe", e.informe.unwrap_or_else(|| Json::obj([]))),
                ]))
            }) {
                r if r.codigo != 200 => return r,
                r => r.cuerpo,
            }
        };
        let campo = |k: &str| match &e {
            Json::Obj(m) => match m.get(k) {
                Some(Json::Str(s)) => s.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let capa = match campo("estado").as_str() {
            "lista" => campo("digest"),
            "sin-dependencias" => String::new(),
            estado => {
                let intento = if estado == "error" {
                    format!(
                        "r{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    )
                } else {
                    "1".to_string()
                };
                let dicho = match self.encolar_capa(
                    &campo("digest"),
                    rama.as_deref().unwrap_or(""),
                    repositorio.as_deref().unwrap_or(""),
                    sujeto,
                    &intento,
                ) {
                    Ok((job, d)) => format!("{d} · Job {job}"),
                    Err(r) => return r,
                };
                let mut cuerpo = Json::obj([
                    (
                        "error",
                        Json::s(format!(
                            "la capa del árbol no está lista ({estado}): se resuelve ahora; vuelve a abrir el puesto en un minuto"
                        )),
                    ),
                    ("capa", Json::s(campo("digest"))),
                    ("cola", Json::s(dicho)),
                ]);
                if let Json::Obj(m) = &mut cuerpo {
                    m.insert("entorno".into(), e.clone());
                }
                return Respuesta {
                    codigo: 409,
                    cuerpo,
                };
            }
        };
        // A la cola: Flux rinde el Job.
        let (fichero, job, dicho) =
            match self.encolar_puesto(&id, sujeto, rama.as_deref(), &capa, entorno, "") {
                Ok(v) => v,
                Err(r) => return r,
            };
        let p = Puesto {
            persona: sujeto.persona.clone(),
            entorno: entorno.to_string(),
            rama,
            fichero,
            job,
            creado: Instant::now(),
            estado: Estado::Encolado,
            agente: None,
            latido: None,
            siguiente: 1,
            pendientes: VecDeque::new(),
            celdas: BTreeMap::new(),
            trabajo: None,
            repositorio: repositorio.clone(),
            clase,
            transform: None,
        };
        let mut lista = self.puestos.lista.lock().unwrap();
        let f = ficha(&id, &p);
        lista.insert(id, p);
        let mut f = f;
        if let Json::Obj(m) = &mut f {
            m.insert("cola".into(), Json::s(dicho));
        }
        Respuesta::creado(f)
    }

    /// `GET /puestos`: los de la persona (uno por entorno, como mucho).
    pub(crate) fn puestos_de(&self, sujeto: &Identidad) -> Respuesta {
        let lista = self.puestos.lista.lock().unwrap();
        let mios: Vec<Json> = lista
            .iter()
            .filter(|(_, p)| p.persona == sujeto.persona && p.trabajo.is_none())
            .map(|(id, p)| ficha(id, p))
            .collect();
        Respuesta::ok(Json::obj([("puestos", Json::Arr(mios))]))
    }

    // ── el trabajo (0031 §9, W3.7 ④) ────────────────────────────────────────

    /// `POST /trabajos {codigo, rama?}`: corre `codigo` —un fichero del árbol,
    /// `.py`, `.ts`/`.js`/`.mjs` o `.java`— como un Job que termina, con el
    /// entorno de su extensión, la capa del árbol y la identidad de la
    /// persona (en su rama). Es un puesto de una sola celda: el fichero, tal
    /// como está en el commit; el agente sale al terminarla y el informe va
    /// a `trabajos/<id>.json`. 202 con `{id, job, codigo, commit}`.
    pub(crate) fn abrir_trabajo(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        if es_agente(sujeto) {
            return Respuesta::error(403, "un agente no lanza trabajos: los lanza una persona");
        }
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) if !cuerpo.trim().is_empty() => n,
            _ => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        let Some(codigo) = n
            .get("codigo")
            .and_then(|(_, v)| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            return Respuesta::error(
                422,
                "falta `codigo`: la ruta del fichero en el árbol (`packages/<p>/transforms/x.py`)",
            );
        };
        if codigo.starts_with('/') || codigo.split('/').any(|s| s == ".." || s.is_empty()) {
            return Respuesta::error(422, format!("`{codigo}` no es una ruta del árbol"));
        }
        let (entorno, lenguaje) = match codigo.rsplit_once('.').map(|(_, e)| e) {
            Some("py") => ("python", "python"),
            Some("ts" | "js" | "mjs") => ("node", "typescript"),
            Some("java") => ("jvm", "java"),
            _ => {
                return Respuesta::error(
                    422,
                    format!(
                        "`{codigo}` no es de ningún entorno: `.py`, `.ts`/`.js`/`.mjs` o `.java`"
                    ),
                );
            }
        };
        let rama = n
            .get("rama")
            .and_then(|(_, v)| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(r) = &rama
            && let Err(m) = crate::propuestas::nombre_de_rama_valido(r)
        {
            return Respuesta::error(422, m);
        }
        // Un trabajo (`POST /trabajos`) no es una sesión: corre y termina. Por
        // eso sigue sin repositorio —su rama y su nombre son los de antes—;
        // acotarlo por la carpeta del fichero sería otra decisión, y se toma
        // cuando haya una medida que la pida.
        let rama = match self.rama_del_puesto(sujeto, rama, None) {
            Ok(r) => r,
            Err(r) => return r,
        };
        // El fichero, tal como está en el commit de la rama: lo que corre es
        // exactamente eso, y el commit va al informe y a la procedencia de lo
        // que escriba (`ORE_CODIGO=<ruta>@<commit>`).
        let (texto, commit) = match self.leyendo_en(rama.as_deref(), |raiz| {
            let ruta = raiz.join(&codigo);
            let Ok(texto) = std::fs::read_to_string(&ruta) else {
                return Respuesta::error(404, format!("no hay `{codigo}` en el árbol"));
            };
            if texto.len() > TEXTO_MAXIMO {
                return Respuesta::error(422, format!("`{codigo}` pasa de {TEXTO_MAXIMO} bytes"));
            }
            let commit = std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(raiz)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "local".into());
            Respuesta::ok(Json::obj([
                ("texto", Json::s(&texto)),
                ("commit", Json::s(commit)),
            ]))
        }) {
            r if r.codigo != 200 => return r,
            r => match &r.cuerpo {
                Json::Obj(m) => (
                    match m.get("texto") {
                        Some(Json::Str(s)) => s.clone(),
                        _ => String::new(),
                    },
                    match m.get("commit") {
                        Some(Json::Str(s)) => s.clone(),
                        _ => "local".into(),
                    },
                ),
                _ => return Respuesta::error(500, "el árbol no contestó"),
            },
        };
        // La capa, como al abrir un puesto (W3.2): la del árbol si está lista;
        // si no, se encola y 409 para que se vuelva a pedir.
        let capa = match self.capa_para(entorno, rama.as_deref(), None, sujeto) {
            Ok(c) => c,
            Err(r) => return r,
        };
        let ahora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let h = ore_core::digest::de_bytes(format!("{codigo}|{commit}|{ahora}").as_bytes());
        let quien = id_de(&sujeto.persona, entorno, None);
        let quien = quien
            .strip_prefix("puesto-")
            .and_then(|q| q.strip_suffix(&format!("-{entorno}")))
            .unwrap_or("x");
        let id = format!(
            "trabajo-{quien}-{}",
            &h["sha256:".len().."sha256:".len() + 8]
        );
        let (fichero, job, dicho) = match self.encolar_puesto(
            &id,
            sujeto,
            rama.as_deref(),
            &capa,
            entorno,
            &format!("{codigo}@{commit}"),
        ) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let mut p = Puesto {
            persona: sujeto.persona.clone(),
            entorno: entorno.to_string(),
            rama,
            fichero,
            job,
            creado: Instant::now(),
            estado: Estado::Encolado,
            agente: None,
            latido: None,
            siguiente: 2,
            pendientes: VecDeque::from([1]),
            celdas: BTreeMap::new(),
            trabajo: Some(Trabajo {
                codigo: codigo.clone(),
                commit: commit.clone(),
                informe: None,
            }),
            // Un trabajo no vive en un repositorio: corre y termina (0036 ④).
            repositorio: None,
            clase: None,
            transform: None,
        };
        p.celdas.insert(
            1,
            Celda {
                texto,
                lenguaje: lenguaje.to_string(),
                enviada: Instant::now(),
                empezada: None,
                salida: None,
            },
        );
        let mut lista = self.puestos.lista.lock().unwrap();
        let mut f = ficha(&id, &p);
        lista.insert(id, p);
        if let Json::Obj(m) = &mut f {
            m.insert("cola".into(), Json::s(dicho));
        }
        Respuesta {
            codigo: 202,
            cuerpo: f,
        }
    }

    /// `GET /trabajos`: los de la persona, del más reciente al más viejo.
    pub(crate) fn trabajos_de(&self, sujeto: &Identidad) -> Respuesta {
        let lista = self.puestos.lista.lock().unwrap();
        let mut mios: Vec<(Instant, Json)> = lista
            .iter()
            .filter(|(_, p)| p.persona == sujeto.persona && p.trabajo.is_some())
            .map(|(id, p)| (p.creado, ficha(id, p)))
            .collect();
        mios.sort_by_key(|(c, _)| std::cmp::Reverse(*c));
        Respuesta::ok(Json::obj([(
            "trabajos",
            Json::Arr(mios.into_iter().map(|(_, f)| f).collect()),
        )]))
    }

    /// `GET /trabajos/{id}`: la ficha, con el informe si ya terminó.
    pub(crate) fn trabajo(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let lista = self.puestos.lista.lock().unwrap();
        match lista.get(id) {
            Some(p) if p.trabajo.is_some() => {
                if p.persona != sujeto.persona && !es_agente(sujeto) {
                    return Respuesta::error(403, "ese trabajo es de otra persona");
                }
                Respuesta::ok(ficha(id, p))
            }
            _ => Respuesta::error(404, format!("no hay ningún trabajo `{id}`")),
        }
    }

    /// La capa con la que nace un puesto o un trabajo: la del árbol si está
    /// lista (o ninguna, fuera de python); pendiente o con error, se encola y
    /// 409 con el motivo.
    fn capa_para(
        &self,
        entorno: &str,
        rama: Option<&str>,
        alcance: Option<&str>,
        sujeto: &Identidad,
    ) -> Result<String, Respuesta> {
        if entorno != "python" {
            return Ok(String::new());
        }
        let e = match self.leyendo_en(rama, |raiz| {
            let e = crate::entorno::entorno_de_en(raiz, alcance);
            Respuesta::ok(Json::obj([
                ("estado", Json::s(e.estado)),
                ("digest", Json::s(&e.digest)),
            ]))
        }) {
            r if r.codigo != 200 => return Err(r),
            r => r.cuerpo,
        };
        let campo = |k: &str| match &e {
            Json::Obj(m) => match m.get(k) {
                Some(Json::Str(s)) => s.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        match campo("estado").as_str() {
            "lista" => Ok(campo("digest")),
            "sin-dependencias" => Ok(String::new()),
            estado => {
                let intento = if estado == "error" {
                    format!(
                        "r{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    )
                } else {
                    "1".to_string()
                };
                let dicho = match self.encolar_capa(
                    &campo("digest"),
                    rama.unwrap_or(""),
                    alcance.unwrap_or(""),
                    sujeto,
                    &intento,
                ) {
                    Ok((job, d)) => format!("{d} · Job {job}"),
                    Err(r) => return Err(r),
                };
                Err(Respuesta {
                    codigo: 409,
                    cuerpo: Json::obj([
                        (
                            "error",
                            Json::s(format!(
                                "la capa del árbol no está lista ({estado}): se resuelve ahora; vuelve a pedirlo en un minuto"
                            )),
                        ),
                        ("capa", Json::s(campo("digest"))),
                        ("cola", Json::s(dicho)),
                    ]),
                })
            }
        }
    }

    /// El informe de un trabajo, al terminar su celda: `trabajos/<id>.json`
    /// en el árbol (en la rama del trabajo), firmado por la persona; el
    /// puesto se cierra y sale de la cola. Lo que la consola enseña en Data ›
    /// Jobs es el Job (por el informador); lo que queda es esto.
    fn informar_trabajo(&self, agente: &Identidad, id: &str, n: u64) {
        let (persona, rama, fichero, mut informe) = {
            let lista = self.puestos.lista.lock().unwrap();
            let Some(p) = lista.get(id) else { return };
            let Some(t) = &p.trabajo else { return };
            let Some(c) = p.celdas.get(&n) else { return };
            let salida = c.salida.clone().unwrap_or(Json::obj([]));
            let (tipo, ms) = match ore_core::parse::parse(&salida.jcs()) {
                Ok(s) => (
                    s.get("tipo")
                        .and_then(|(_, v)| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    s.get("ms")
                        .and_then(|(_, v)| v.as_str())
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(0),
                ),
                Err(_) => (String::new(), 0),
            };
            let ahora = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            (
                p.persona.clone(),
                p.rama.clone(),
                p.fichero.clone(),
                Json::obj([
                    ("id", Json::s(id)),
                    ("codigo", Json::s(&t.codigo)),
                    ("commit", Json::s(&t.commit)),
                    ("persona", Json::s(&p.persona)),
                    ("rama", Json::s(p.rama.clone().unwrap_or_default())),
                    ("entorno", Json::s(&p.entorno)),
                    ("job", Json::s(&p.job)),
                    (
                        "estado",
                        Json::s(if tipo == "error" { "error" } else { "hecho" }),
                    ),
                    ("ms", Json::Int(ms)),
                    ("terminado_s", Json::Int(ahora)),
                    // Lo que el transform declaró, si lo hubo (⑤): el informe
                    // dice qué se dejó leer y escribir, no sólo qué salió.
                    (
                        "declarado",
                        match &p.transform {
                            Some(t) => Json::obj([
                                ("transform", Json::s(&t.nombre)),
                                ("inputs", Json::Arr(t.inputs.iter().map(Json::s).collect())),
                                ("output", Json::s(&t.output)),
                            ]),
                            None => Json::obj([]),
                        },
                    ),
                    ("salida", salida),
                ]),
            )
        };
        let quien = Identidad {
            persona: persona.clone(),
            agente: Some(agente.persona.clone()),
            correo: None,
            nombre: None,
            tipo: None,
        };
        let ruta = format!("trabajos/{id}.json");
        let texto = informe.pretty() + "\n";
        let r = self.escribiendo_en(rama.as_deref(), &quien, &format!("Trabajo {id}"), |raiz| {
            let f = raiz.join(&ruta);
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            match std::fs::write(&f, &texto) {
                Ok(()) => Respuesta::ok(Json::obj([("fichero", Json::s(&ruta))])),
                Err(e) => Respuesta::error(500, format!("no se pudo escribir `{ruta}`: {e}")),
            }
        });
        if let Json::Obj(m) = &mut informe {
            m.insert("fichero".into(), Json::s(&ruta));
            if let Json::Obj(c) = &r.cuerpo
                && let Some(Json::Str(commit)) = c.get("commit")
            {
                m.insert("informe_commit".into(), Json::s(commit));
            }
            if r.codigo >= 300 {
                m.insert(
                    "informe_error".into(),
                    Json::s(format!("{}: {}", r.codigo, r.cuerpo.jcs())),
                );
            }
        }
        {
            let mut lista = self.puestos.lista.lock().unwrap();
            if let Some(p) = lista.get_mut(id) {
                if let Some(t) = &mut p.trabajo {
                    t.informe = Some(informe);
                }
                p.estado = Estado::Cerrado;
                p.pendientes.clear();
            }
        }
        self.puestos.campana.notify_all();
        let _ = self.desencolar_puesto(&fichero, &quien);
    }

    /// `GET /puestos/{id}`.
    pub(crate) fn puesto(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let lista = self.puestos.lista.lock().unwrap();
        match lista.get(id) {
            None => Respuesta::error(404, format!("no hay ningún puesto `{id}`")),
            Some(p) if p.persona != sujeto.persona && !es_agente(sujeto) => {
                Respuesta::error(403, "ese puesto es de otra persona")
            }
            Some(p) => Respuesta::ok(ficha(id, p)),
        }
    }

    /// `DELETE /puestos/{id}`: fuera de la cola (Flux retira el Job) y cerrado.
    pub(crate) fn cerrar_puesto(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let fichero = {
            let mut lista = self.puestos.lista.lock().unwrap();
            let Some(p) = lista.get_mut(id) else {
                return Respuesta::error(404, format!("no hay ningún puesto `{id}`"));
            };
            if p.persona != sujeto.persona {
                return Respuesta::error(403, "ese puesto es de otra persona");
            }
            p.estado = Estado::Cerrado;
            p.pendientes.clear();
            p.fichero.clone()
        };
        self.puestos.campana.notify_all();
        let dicho = self.desencolar_puesto(&fichero, sujeto);
        Respuesta::ok(Json::obj([
            ("id", Json::s(id)),
            ("estado", Json::s("cerrado")),
            ("cola", Json::s(dicho)),
        ]))
    }

    /// `POST /puestos/{id}/ejecutar {texto}`: una celda a la cola del puesto.
    /// 202 con su número; se espera con `GET /puestos/{id}/celdas/{n}`.
    pub(crate) fn ejecutar_en_puesto(
        &self,
        sujeto: &Identidad,
        id: &str,
        cuerpo: &str,
    ) -> Respuesta {
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(_) => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        let Some(texto) = n.get("texto").and_then(|(_, v)| v.as_str()) else {
            return Respuesta::error(422, "una celda lleva `texto`");
        };
        let lenguaje = n
            .get("lenguaje")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("python")
            .to_string();
        if !LENGUAJES.contains(&lenguaje.as_str()) {
            return Respuesta::error(
                422,
                format!(
                    "`{lenguaje}` no corre en un puesto: {}",
                    LENGUAJES.join(", ")
                ),
            );
        }
        if texto.len() > TEXTO_MAXIMO {
            return Respuesta::error(422, "la celda es demasiado larga (256 KiB)");
        }
        let mut lista = self.puestos.lista.lock().unwrap();
        let Some(p) = lista.get_mut(id) else {
            return Respuesta::error(404, format!("no hay ningún puesto `{id}`"));
        };
        if p.persona != sujeto.persona {
            return Respuesta::error(403, "ese puesto es de otra persona");
        }
        if p.estado == Estado::Cerrado {
            return Respuesta::error(410, "el puesto está cerrado: abre otro");
        }
        if !corre_en(&lenguaje, &p.entorno) {
            return Respuesta::error(
                422,
                format!(
                    "una celda `{lenguaje}` no corre en un puesto `{}`: abre uno `{}`",
                    p.entorno,
                    entorno_de(&lenguaje).unwrap_or("?")
                ),
            );
        }
        if perdido(p) {
            return Respuesta::error(
                409,
                if p.estado == Estado::Encolado {
                    format!(
                        "el puesto lleva {} s encolado sin arrancar: se perdió (¿el Job no está?); ciérralo y abre otro",
                        p.creado.elapsed().as_secs()
                    )
                } else {
                    format!(
                        "el puesto lleva {} s sin dar señales: se perdió (¿TTL, tope o relevo?); ciérralo y abre otro",
                        p.latido.map(|l| l.elapsed().as_secs()).unwrap_or(0)
                    )
                },
            );
        }
        let num = p.siguiente;
        p.siguiente += 1;
        p.celdas.insert(
            num,
            Celda {
                texto: texto.to_string(),
                lenguaje,
                enviada: Instant::now(),
                empezada: None,
                salida: None,
            },
        );
        p.pendientes.push_back(num);
        let estado = p.estado.dice();
        drop(lista);
        self.puestos.campana.notify_all();
        Respuesta {
            codigo: 202,
            cuerpo: Json::obj([
                ("celda", Json::Int(num as i64)),
                ("puesto", Json::s(estado)),
            ]),
        }
    }

    /// `GET /puestos/{id}/celdas/{n}`: la salida, esperando hasta [`ESPERA`].
    pub(crate) fn celda_del_puesto(&self, sujeto: &Identidad, id: &str, n: u64) -> Respuesta {
        let limite = Instant::now() + ESPERA;
        let mut lista = self.puestos.lista.lock().unwrap();
        loop {
            let Some(p) = lista.get(id) else {
                return Respuesta::error(404, format!("no hay ningún puesto `{id}`"));
            };
            if p.persona != sujeto.persona {
                return Respuesta::error(403, "ese puesto es de otra persona");
            }
            let Some(c) = p.celdas.get(&n) else {
                return Respuesta::error(404, format!("el puesto no tiene una celda {n}"));
            };
            if c.salida.is_some() || Instant::now() >= limite {
                return Respuesta::ok(ficha_de_celda(n, c));
            }
            let (l, _) = self
                .puestos
                .campana
                .wait_timeout(lista, limite.saturating_duration_since(Instant::now()))
                .unwrap();
            lista = l;
        }
    }

    // ── el agente ───────────────────────────────────────────────────────────

    fn reclamar<'a>(
        lista: &'a mut BTreeMap<String, Puesto>,
        sujeto: &Identidad,
        id: &str,
    ) -> Result<&'a mut Puesto, Respuesta> {
        if !es_agente(sujeto) {
            return Err(Respuesta::error(403, "esta ruta es del agente del puesto"));
        }
        let Some(p) = lista.get_mut(id) else {
            return Err(Respuesta::error(
                410,
                format!("no hay ningún puesto `{id}` (¿un relevo?): ciérrate"),
            ));
        };
        if p.estado == Estado::Cerrado {
            return Err(Respuesta::error(410, "el puesto está cerrado: ciérrate"));
        }
        match &p.agente {
            None => p.agente = Some(sujeto.persona.clone()),
            Some(a) if a != &sujeto.persona => {
                return Err(Respuesta::error(
                    403,
                    "ese puesto ya lo reclamó otro agente",
                ));
            }
            _ => {}
        }
        p.estado = Estado::Vivo;
        p.latido = Some(Instant::now());
        Ok(p)
    }

    /// `GET /puestos/{id}/pendiente`: la siguiente celda, esperando hasta
    /// [`ESPERA`]. `{pendiente: false}` si no hay.
    pub(crate) fn pendiente_del_puesto(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let limite = Instant::now() + ESPERA;
        let mut lista = self.puestos.lista.lock().unwrap();
        loop {
            let p = match Self::reclamar(&mut lista, sujeto, id) {
                Ok(p) => p,
                Err(r) => return r,
            };
            if let Some(n) = p.pendientes.pop_front() {
                let c = p.celdas.get_mut(&n).expect("la celda pendiente existe");
                c.empezada = Some(Instant::now());
                let texto = c.texto.clone();
                let lenguaje = c.lenguaje.clone();
                drop(lista);
                self.puestos.campana.notify_all();
                return Respuesta::ok(Json::obj([
                    ("pendiente", Json::Bool(true)),
                    ("celda", Json::Int(n as i64)),
                    ("lenguaje", Json::s(lenguaje)),
                    ("texto", Json::s(texto)),
                ]));
            }
            if Instant::now() >= limite {
                return Respuesta::ok(Json::obj([("pendiente", Json::Bool(false))]));
            }
            let (l, _) = self
                .puestos
                .campana
                .wait_timeout(lista, limite.saturating_duration_since(Instant::now()))
                .unwrap();
            lista = l;
        }
    }

    /// `POST /puestos/{id}/celdas/{n}/salida`: lo que salió, tal cual (tipada
    /// por el agente: tabla, texto, error, vacía).
    pub(crate) fn salida_del_puesto(
        &self,
        sujeto: &Identidad,
        id: &str,
        n: u64,
        cuerpo: &str,
    ) -> Respuesta {
        // Se analiza para comprobar `tipo`, y se guarda **tal cual llegó**: la
        // salida lleva `null` y dobles (0032 §1), y el `Json` del núcleo no los
        // modela —pasarla por `de_node` los volvía las cadenas `"null"` y `"1.5"`
        // en la consola—. Lo que el agente escribió es lo que la consola lee.
        let leida = match ore_core::parse::parse(cuerpo) {
            Ok(v) => crate::rutas::de_node(&v),
            Err(_) => return Respuesta::error(400, "la salida no es JSON"),
        };
        let tipo_ok = matches!(&leida, Json::Obj(m) if matches!(m.get("tipo"), Some(Json::Str(t)) if ["tabla", "texto", "error", "vacia"].contains(&t.as_str())));
        if !tipo_ok {
            return Respuesta::error(422, "la salida lleva `tipo`: tabla, texto, error o vacia");
        }
        let mut lista = self.puestos.lista.lock().unwrap();
        let p = match Self::reclamar(&mut lista, sujeto, id) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let Some(c) = p.celdas.get_mut(&n) else {
            return Respuesta::error(404, format!("el puesto no tiene una celda {n}"));
        };
        c.salida = Some(Json::Crudo(cuerpo.trim().to_string()));
        let es_trabajo = p.trabajo.is_some();
        drop(lista);
        self.puestos.campana.notify_all();
        if es_trabajo {
            self.informar_trabajo(sujeto, id, n);
        }
        Respuesta::ok(Json::obj([
            ("celda", Json::Int(n as i64)),
            ("estado", Json::s("hecha")),
        ]))
    }

    /// **Un puesto sin rama nace en `<persona>/puesto`** (0031 W3.7 gobierno ④,
    /// regla 4): lo que una persona escribe o declara desde código vive en su
    /// rama hasta que lo publica por propuesta; `main` no se toca desde una
    /// celda. Medido antes: todo lo de bob (anexar, retirar, redeclarar la
    /// Entity de ana y desclasificarla) caía en `main`. La rama la crea el
    /// servidor por git si no está; con rama dicha, la dicha; y sobre un
    /// directorio (el banco, las pruebas) no hay ramas y se sigue en él.
    fn rama_del_puesto(
        &self,
        sujeto: &Identidad,
        rama: Option<String>,
        repositorio: Option<&str>,
    ) -> Result<Option<String>, Respuesta> {
        if rama.is_some() {
            return Ok(rama);
        }
        let crate::rutas::Arbol::Forja(forja) = &self.arbol else {
            return Ok(None);
        };
        // 0036 ④: `<persona>/<repo>` cuando se trabaja en uno. Dos repositorios
        // de la misma persona dejan de pisarse la rama; sin repositorio, la de
        // siempre (`<persona>/puesto`).
        let donde = repositorio
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|r| r.trim_matches('/').rsplit('/').next())
            .map(cola::nombre_de_objeto)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "puesto".to_string());
        let nombre = format!("{}/{donde}", crate::propuestas::prefijo_de(&sujeto.persona));
        match forja.asegurar_rama(&nombre) {
            Ok(_) => Ok(Some(nombre)),
            Err(e) => Err(Respuesta::error(
                502,
                format!("no se pudo abrir la rama `{nombre}` del puesto: {e}"),
            )),
        }
    }

    /// `GET /puestos/{id}/datos/{vista}`: qué copia es `<paquete>.<vista>`, en
    /// nombre de la persona dueña del puesto. 404 sin vista, 409 sin copia.
    pub(crate) fn datos_del_puesto(&self, sujeto: &Identidad, id: &str, vista: &str) -> Respuesta {
        let rama = {
            let mut lista = self.puestos.lista.lock().unwrap();
            match Self::reclamar(&mut lista, sujeto, id) {
                Ok(p) => p.rama.clone(),
                Err(r) => return r,
            }
        };
        let Some((ns, nombre)) = vista.split_once('.') else {
            return Respuesta::error(422, "una vista es `<paquete>.<nombre>`");
        };
        if let Err(m) = crate::rutas::token(ns).and(crate::rutas::token(nombre)) {
            return Respuesta::error(422, m);
        }
        // Lo declarado manda (⑤): mientras un transform corre, este puesto sólo
        // resuelve sus `inputs`. El mismo 403 que el SDK da, en el servidor.
        if let Some(t) = self.transform_de(id)
            && !t.inputs.iter().any(|i| i == vista)
        {
            return Respuesta::error(
                403,
                format!(
                    "`{vista}` no está en los inputs de `{}` ({}): un transform sólo lee lo que declara",
                    t.nombre,
                    t.inputs.join(", ")
                ),
            );
        }
        let (ns, nombre, vista) = (ns.to_string(), nombre.to_string(), vista.to_string());
        let r = self.leyendo_en(rama.as_deref(), |raiz| {
            self.con_credencial(raiz, datos_de(raiz, &ns, &nombre, &vista))
        });
        // **El fallback de rama** (0031 §4, W3.7 ③): una rama lee las copias
        // de `main` mientras no tenga las suyas. Lo que la rama no tiene —ni
        // el documento (404) ni el dataset (409)— se busca en `main`, y la
        // respuesta lo dice (`rama: main`); lo que la rama sí tiene manda.
        // Medido antes: sin esto, lo que `main` ganaba tras abrir la rama era
        // «no hay ninguna View» desde ella.
        if rama.is_some() && matches!(r.codigo, 404 | 409) {
            let mut de_main = self.leyendo_en(None, |raiz| {
                self.con_credencial(raiz, datos_de(raiz, &ns, &nombre, &vista))
            });
            if de_main.codigo == 200 {
                if let Json::Obj(m) = &mut de_main.cuerpo {
                    m.insert("rama".into(), Json::s("main"));
                }
                return de_main;
            }
        }
        r
    }

    /// `POST /puestos/{id}/transform {nombre, inputs, output}` y
    /// `DELETE /puestos/{id}/transform` (0031 W3.7 gobierno ⑤): el agente
    /// **declara al servidor** lo que el transform de la celda va a leer y
    /// escribir, y lo retira al salir. Mientras está, `datos` sólo resuelve
    /// `inputs` y el catálogo sólo carga o confirma `output`: el 403 pasa del
    /// SDK al servidor, y la procedencia deja de poder mentir por omisión.
    /// Dos transforms a la vez en el mismo puesto: 409 (un transform no llama
    /// a otro, y el SDK ya lo niega).
    pub(crate) fn declarar_transform(
        &self,
        sujeto: &Identidad,
        id: &str,
        cuerpo: &str,
    ) -> Respuesta {
        let Ok(n) = ore_core::parse::parse(cuerpo) else {
            return Respuesta::error(400, "el cuerpo no es JSON");
        };
        let campo = |k: &str| {
            n.get(k)
                .and_then(|(_, v)| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let Some(output) = campo("output") else {
            return Respuesta::error(422, "un transform declara `output`: `<paquete>.<tabla>`");
        };
        let inputs: Vec<String> = n
            .get("inputs")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect();
        if inputs
            .iter()
            .chain([&output])
            .any(|x| x.split('.').count() != 2)
        {
            return Respuesta::error(422, "`inputs` y `output` son `<paquete>.<nombre>`");
        }
        let nombre = campo("nombre").unwrap_or_else(|| "transform".into());
        let mut lista = self.puestos.lista.lock().unwrap();
        let p = match Self::reclamar(&mut lista, sujeto, id) {
            Ok(p) => p,
            Err(r) => return r,
        };
        if let Some(t) = &p.transform {
            return Respuesta::error(
                409,
                format!(
                    "`{}` ya está corriendo en este puesto: un transform no llama a otro",
                    t.nombre
                ),
            );
        }
        p.transform = Some(Transform {
            nombre: nombre.clone(),
            inputs: inputs.clone(),
            output: output.clone(),
        });
        Respuesta::ok(Json::obj([
            ("transform", Json::s(nombre)),
            ("inputs", Json::Arr(inputs.iter().map(Json::s).collect())),
            ("output", Json::s(output)),
        ]))
    }

    pub(crate) fn retirar_transform(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let mut lista = self.puestos.lista.lock().unwrap();
        let p = match Self::reclamar(&mut lista, sujeto, id) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let habia = p.transform.take();
        Respuesta::ok(Json::obj([(
            "transform",
            match habia {
                Some(t) => Json::s(t.nombre),
                None => Json::Bool(false),
            },
        )]))
    }

    /// Lo declarado por el transform que corre en un puesto, si corre alguno.
    /// La clase del repositorio donde vive un puesto, si vive en uno (0036 ⑤).
    /// Es lo que el catálogo mira antes de dejar escribir.
    pub(crate) fn clase_de(&self, id: &str) -> Option<&'static ore_core::clases::Clase> {
        self.puestos
            .lista
            .lock()
            .unwrap()
            .get(id)
            .and_then(|p| p.clase)
    }

    pub(crate) fn transform_de(&self, id: &str) -> Option<Transform> {
        self.puestos
            .lista
            .lock()
            .unwrap()
            .get(id)
            .and_then(|p| p.transform.clone())
    }

    /// **La credencial de lectura** (0031 W3.7 gobierno ②b): lo que `datos`
    /// resolvió lleva, si el almacén sabe acotar, una credencial **sólo para
    /// leer** ese dataset (`ore datasets --cargar --prestar --leer` →
    /// `ore-store prestar {modo: leer}`: `objectViewer` bajo su prefijo,
    /// STS 50–60 ms medidos). El SDK lee con ella y no con la identidad del
    /// pod, que desde ②b no ve los datasets del bucket: la única forma de leer
    /// es pasar por aquí, y aquí decide el conducto. Un almacén que no acota
    /// (el S3 de mentira) presta lo que tiene; sin almacén, nada, y el SDK
    /// sigue como antes.
    fn con_credencial(&self, raiz: &Path, mut r: Respuesta) -> Respuesta {
        if r.codigo != 200 {
            return r;
        }
        let Json::Obj(m) = &r.cuerpo else { return r };
        let Some(Json::Str(ml)) = m.get("metadata_location") else {
            return r;
        };
        if ml.is_empty() {
            return r;
        }
        let Some(Json::Str(ds)) = m.get("dataset") else {
            return r;
        };
        let args: Vec<String> = [
            "datasets",
            ".",
            "--cargar",
            ds.as_str(),
            "--prestar",
            "--leer",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let Ok(s) = crate::mando::correr(&self.binario, raiz, &args) else {
            return r;
        };
        if s.codigo != 0 {
            return r;
        }
        let Some(j) = s
            .stdout
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .and_then(|l| ore_core::parse::parse(l).ok())
        else {
            return r;
        };
        if let Some((_, c)) = j.get("config")
            && let Json::Obj(cfg) = Json::de_node(c)
            && !cfg.is_empty()
            && let Json::Obj(m) = &mut r.cuerpo
        {
            m.insert("credencial".into(), Json::Obj(cfg));
        }
        r
    }

    /// **Quién escribe desde un puesto** (0031 §11): el agente pide en nombre
    /// de la persona que abrió el puesto, y en la rama del puesto. Sólo el
    /// agente que lo reclamó; sin tocar su estado.
    /// **Desde un puesto, quien escribe es la persona que lo abrió, y en su
    /// rama.** Lo que `/v1` (el catálogo) hace desde W3.6c y `/documentos`
    /// no hacía (medido en «Lo medido para W3.7» §1: la View que una celda
    /// declaraba la firmaba `agente:local`, en `main`). Con `x-ore-puesto`,
    /// el sujeto pasa a ser la persona (el agente queda como `agente`) y la
    /// rama, la del puesto si tiene; sin la cabecera, lo que llegó.
    pub(crate) fn sujeto_del_puesto(
        &self,
        p: &ore_entrada::http::Peticion,
        sujeto: &Identidad,
        rama: Option<&str>,
    ) -> Result<(Identidad, Option<String>), Respuesta> {
        match p
            .cabeceras
            .get(PUESTO)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            Some(id) => {
                let (persona, rama_del_puesto) = self.persona_del_puesto(sujeto, id)?;
                Ok((
                    Identidad {
                        persona,
                        agente: Some(sujeto.persona.clone()),
                        correo: None,
                        nombre: None,
                        tipo: None,
                    },
                    rama_del_puesto.or_else(|| rama.map(String::from)),
                ))
            }
            None => Ok((sujeto.clone(), rama.map(String::from))),
        }
    }

    pub(crate) fn persona_del_puesto(
        &self,
        sujeto: &Identidad,
        id: &str,
    ) -> Result<(String, Option<String>), Respuesta> {
        if !es_agente(sujeto) {
            return Err(Respuesta::error(
                403,
                "`x-ore-puesto` es del agente del puesto",
            ));
        }
        let lista = self.puestos.lista.lock().unwrap();
        let Some(p) = lista.get(id) else {
            return Err(Respuesta::error(
                410,
                format!("no hay ningún puesto `{id}`"),
            ));
        };
        if p.agente.as_deref() != Some(sujeto.persona.as_str()) {
            return Err(Respuesta::error(403, "ese puesto no es de este agente"));
        }
        Ok((p.persona.clone(), p.rama.clone()))
    }

    // ── la cola ─────────────────────────────────────────────────────────────

    fn encolar_puesto(
        &self,
        id: &str,
        sujeto: &Identidad,
        rama: Option<&str>,
        capa: &str,
        entorno: &str,
        trabajo: &str,
    ) -> Result<(String, String, String), Respuesta> {
        let Some(forja) = &self.cola else {
            return Err(Respuesta::error(
                503,
                "este servidor no sabe de ninguna cola (`--cola`): no hay quien rinda un puesto",
            ));
        };
        let prestado = forja
            .clonar()
            .map_err(|e| Respuesta::error(502, e.to_string()))?;
        let dir = prestado.ruta();
        let plantilla =
            std::fs::read_to_string(dir.join(cola::PLANTILLA_PUESTO)).map_err(|_| {
                Respuesta::error(
                    503,
                    format!(
                        "la cola no trae `{}`: hay que converger este inquilino",
                        cola::PLANTILLA_PUESTO
                    ),
                )
            })?;
        // El instante de apertura va en el Job: reabrir (tras un TTL, un tope o
        // un relevo) rinde OTRO nombre, y Flux retira el Job viejo y crea el nuevo.
        let abierto = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default();
        let (fichero, texto, job) = cola::rendir_puesto(
            &plantilla,
            id,
            rama.unwrap_or(""),
            capa,
            &abierto,
            entorno,
            trabajo,
        )
        .map_err(|e| Respuesta::error(500, e))?;
        std::fs::write(dir.join(&fichero), &texto)
            .map_err(|e| Respuesta::error(500, format!("no se pudo escribir `{fichero}`: {e}")))?;
        if !forja.hay_cambios(dir) {
            return Ok((
                fichero.clone(),
                job,
                format!("ya encolado como `{fichero}`"),
            ));
        }
        let que = if id.starts_with("trabajo-") {
            "Lanzar el trabajo"
        } else {
            "Abrir el puesto"
        };
        match forja.publicar(dir, sujeto, &format!("{que} {id}")) {
            Ok(c) => Ok((
                fichero.clone(),
                job,
                format!("encolado como `{fichero}` · commit {c}"),
            )),
            Err(e) => Err(Respuesta::error(502, format!("NO encolado: {e}"))),
        }
    }

    fn desencolar_puesto(&self, fichero: &str, sujeto: &Identidad) -> String {
        let Some(forja) = &self.cola else {
            return "sin cola (`--cola`): nada que desencolar".into();
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return format!("NO desencolado: {e}"),
        };
        let dir = prestado.ruta();
        if !dir.join(fichero).is_file() {
            return format!("`{fichero}` no estaba en la cola");
        }
        if let Err(e) = std::fs::remove_file(dir.join(fichero)) {
            return format!("NO desencolado: {e}");
        }
        match forja.publicar(dir, sujeto, &format!("Cerrar el puesto ({fichero})")) {
            Ok(c) => format!("`{fichero}` fuera de la cola · commit {c}"),
            Err(e) => format!("NO desencolado: {e}"),
        }
    }
}

/// ¿`name: <nombre>` aparece como palabra entera (en bloque o en línea)?
fn nombra(texto: &str, nombre: &str) -> bool {
    let clave = format!("name: {nombre}");
    texto.match_indices(&clave).any(|(i, _)| {
        texto[i + clave.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
    })
}

/// Lo que el árbol dice del dataset de un nombre (0031 §10, 0033: **un
/// lector, un camino**): el nombre resuelve a un `Dataset` —o a una `View`
/// cuya raíz de lectura es uno— y su puntero está en `datasets/<p>_<n>.json`.
/// Con `estado: copiada|al-dia` y `metadata_location` (o `clave`, heredado),
/// el dataset está; si no, 409 con lo que el puntero diga. Una View virtual o
/// una Table es 409 «sin dataset»: no hay camino por el que esta ruta llegue a
/// un origen.
fn datos_de(raiz: &Path, ns: &str, nombre: &str, vista: &str) -> Respuesta {
    let hay = |carpeta: &str| {
        std::fs::read_dir(raiz.join("packages").join(ns).join(carpeta))
            .map(|d| {
                d.flatten().find_map(|e| {
                    std::fs::read_to_string(e.path())
                        .ok()
                        .filter(|t| nombra(t, nombre))
                })
            })
            .unwrap_or(None)
    };
    // ¿Existe el documento? El puntero de algo que no está es un 404, no un 409.
    // Un dataset se lee por su puntero; una vista, por el del primer dataset
    // que tenga debajo (lo dice el compilador); una tabla, por ninguno.
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let del_dataset = if hay("datasets").is_some() {
        format!("{ns}.{nombre}")
    } else if hay("views").is_some() {
        let copia = pkg
            .docs
            .iter()
            .find(|d| {
                d.kind == ore_core::document::Kind::View && d.qname().as_deref() == Some(vista)
            })
            .and_then(|v| ore_core::vistas::raiz_de_lectura(&pkg, v))
            .and_then(|c| c.qname());
        match copia {
            Some(c) => c,
            None => {
                return Respuesta::error(
                    409,
                    format!(
                        "`{vista}` es una `View` virtual: no tiene ningún dataset debajo del que leer. Declara un `Dataset` con `from` sobre ella, o léela como pregunta (sql)"
                    ),
                );
            }
        }
    } else if hay("tables").is_some() {
        return Respuesta::error(
            409,
            format!(
                "`{vista}` es una `Table` de una fuente, no un dataset: se lee por un `Dataset` que la copie, nunca del origen"
            ),
        );
    } else {
        return Respuesta::error(
            404,
            format!("no hay ningún `Dataset`, `View` ni `Table` `{vista}` en el paquete `{ns}`"),
        );
    };
    // **El conducto de la lectura** (0031 W3.7 gobierno ②): lo que el dataset
    // lleva en cada campo —por su raíz y por las entidades de su cadena—
    // contra `contextSurface.workspace` (o `materialization.payload` si el
    // árbol no lo declara). Medido antes: un dataset que una Entity
    // clasificaba `high` salía entero por `over()` con un conducto de `low`.
    // Se decide sobre los bytes que se van a leer (el dataset), no sobre la
    // vista pedida: es lo que el SDK trae.
    if let Err(n) = ore_core::flow::lectura_desde_puesto(&pkg, &del_dataset) {
        let mut r = Respuesta::error(403, format!("{}: {}", n.codigo, n.mensaje));
        if let Json::Obj(m) = &mut r.cuerpo {
            m.insert("codigo".into(), Json::s(n.codigo));
        }
        return r;
    }
    let clasificacion: Json = Json::Obj(
        ore_core::flow::clasificacion_de(&pkg, &del_dataset)
            .into_iter()
            .map(|(k, v)| (k, Json::s(v)))
            .collect(),
    );
    let informe = raiz
        .join("datasets")
        .join(format!("{}.json", del_dataset.replace('.', "_")));
    let Ok(texto) = std::fs::read_to_string(&informe) else {
        return Respuesta::error(
            409,
            format!(
                "el dataset `{del_dataset}` no está: aún no se copió, o nadie lo escribió todavía"
            ),
        );
    };
    let n = match ore_core::parse::parse(&texto) {
        Ok(n) => n,
        Err(_) => return Respuesta::error(502, format!("el informe de `{vista}` no analiza")),
    };
    let campo = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let estado = campo("estado");
    let clave = campo("clave");
    // El puntero de un dataset (0031 §10): `metadata_location` es el `metadata.json`
    // vigente de una tabla Iceberg, y el SDK lo lee por la raíz y la versión. Una
    // copia heredada trae `clave` (el sobre ORECOPY1); las dos formas conviven.
    let metadata_location = campo("metadata_location");
    if !matches!(estado.as_str(), "copiada" | "al-dia")
        || (clave.is_empty() && metadata_location.is_empty())
    {
        return Respuesta::error(
            409,
            format!(
                "la copia de `{vista}` no está hecha: el informe dice `{estado}`{}",
                if campo("error").is_empty() {
                    String::new()
                } else {
                    format!(" · {}", campo("error"))
                }
            ),
        );
    }
    Respuesta::ok(Json::obj([
        ("vista", Json::s(vista)),
        ("dataset", Json::s(&del_dataset)),
        ("estado", Json::s(estado)),
        ("clave", Json::s(clave)),
        ("metadata_location", Json::s(metadata_location)),
        ("snapshot", Json::s(campo("snapshot"))),
        ("plan", Json::s(campo("plan"))),
        ("filas", Json::s(campo("filas"))),
        ("clasificacion", clasificacion),
        (
            "bucket",
            Json::s(std::env::var("ORE_GCS_BUCKET").unwrap_or_default()),
        ),
        (
            "almacen",
            Json::s(std::env::var("ORE_STORE").unwrap_or_default()),
        ),
    ]))
}

#[cfg(test)]
mod prueba {
    use super::*;

    /// **Un lector, un camino** (0031 §10, 0033): un dataset resuelve por su
    /// puntero en `datasets/`; una View, por el del primer dataset que tenga
    /// debajo; una View virtual y una Table son 409, y lo que no está es 404.
    #[test]
    fn el_puesto_resuelve_datasets_y_vistas_con_dataset_debajo() {
        let d = std::env::temp_dir().join(format!("ore-datos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in [
            "packages/v/views",
            "packages/v/tables",
            "packages/v/datasets",
            "datasets",
        ] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/package.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: v, version: 0.1.0, status: active, domain: v }\nspec: { owner: team:v }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/tables/origen.yaml"),
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: origen, namespace: v }\nspec:\n  datasource: pg\n  object: public.origen\n  columns: { id: { type: Integer } }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
        )
        .unwrap();
        // El dataset mantenido (la copia) y la vista que pregunta sobre él.
        std::fs::write(
            d.join("packages/v/datasets/pedidos.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: v }\nspec:\n  owner: team:v\n  from: { table: v.origen }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/views/grandes.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: grandes, namespace: v }\nspec:\n  owner: team:v\n  from: { dataset: v.pedidos }\n  fields: { id: id }\n",
        )
        .unwrap();
        // Una vista virtual, sobre la tabla: no hay de dónde leer.
        std::fs::write(
            d.join("packages/v/views/virtual.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: virtual, namespace: v }\nspec:\n  owner: team:v\n  from: { table: v.origen }\n  fields: { id: id }\n",
        )
        .unwrap();
        // El dataset escrito, con su puntero.
        std::fs::write(
            d.join("packages/v/datasets/salida.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: salida, namespace: v }\nspec:\n  owner: team:v\n  columns: { id: { type: Integer } }\n  changes: { mode: append }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("datasets/v_pedidos.json"),
            "{\"estado\":\"copiada\",\"metadata_location\":\"gs://b/copias/v_pedidos/metadata/1.metadata.json\",\"snapshot\":\"1\",\"dataset\":\"copias/v_pedidos\"}",
        )
        .unwrap();
        std::fs::write(
            d.join("datasets/v_salida.json"),
            "{\"estado\":\"copiada\",\"metadata_location\":\"gs://b/datasets/v_salida/metadata/2.metadata.json\",\"snapshot\":\"2\"}",
        )
        .unwrap();
        // El dataset mantenido, por su puntero (los bytes siguen en `copias/`).
        let r = datos_de(&d, "v", "pedidos", "v.pedidos");
        assert_eq!(r.codigo, 200, "{:?}", r.cuerpo);
        assert!(
            r.cuerpo.jcs().contains("copias/v_pedidos"),
            "{}",
            r.cuerpo.jcs()
        );
        // La vista sobre el dataset lee el puntero del dataset.
        let r = datos_de(&d, "v", "grandes", "v.grandes");
        assert_eq!(r.codigo, 200, "{:?}", r.cuerpo);
        assert!(
            r.cuerpo.jcs().contains("copias/v_pedidos"),
            "{}",
            r.cuerpo.jcs()
        );
        // El dataset escrito.
        let r = datos_de(&d, "v", "salida", "v.salida");
        assert_eq!(r.codigo, 200, "{:?}", r.cuerpo);
        assert!(
            r.cuerpo.jcs().contains("datasets/v_salida"),
            "{}",
            r.cuerpo.jcs()
        );
        // La vista virtual: 409, sin dataset debajo.
        let r = datos_de(&d, "v", "virtual", "v.virtual");
        assert_eq!(r.codigo, 409, "{:?}", r.cuerpo);
        assert!(r.cuerpo.jcs().contains("virtual"), "{}", r.cuerpo.jcs());
        // La tabla: 409, no un dataset.
        let r = datos_de(&d, "v", "origen", "v.origen");
        assert_eq!(r.codigo, 409, "{:?}", r.cuerpo);
        assert!(
            r.cuerpo.jcs().contains("no un dataset"),
            "{}",
            r.cuerpo.jcs()
        );
        let r = datos_de(&d, "v", "nadie", "v.nadie");
        assert_eq!(r.codigo, 404);
        // el dataset escrito sin puntero todavía: 409, no 404
        std::fs::remove_file(d.join("datasets/v_salida.json")).unwrap();
        let r = datos_de(&d, "v", "salida", "v.salida");
        assert_eq!(r.codigo, 409, "{:?}", r.cuerpo);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_id_sale_de_la_persona_del_entorno_y_del_repositorio() {
        assert_eq!(id_de("persona:ana", "python", None), "puesto-ana-python");
        assert_eq!(
            id_de("persona:Ana García", "node", None),
            "puesto-ana-garc-a-node"
        );
        assert_eq!(
            id_de("4f0a9c2e-1b2c-4d5e-8f90-1234567890ab", "jvm", None),
            "puesto-4f0a9c2e-1b2c-4d5e-8f90-jvm"
        );
        // 0036 ④: dos repositorios de la misma persona son DOS sesiones.
        assert_eq!(
            id_de("persona:ana", "python", Some("packages/hr/raw")),
            "puesto-ana-python-raw"
        );
        assert_ne!(
            id_de("persona:ana", "python", Some("packages/hr/raw")),
            id_de("persona:ana", "python", Some("packages/hr/clean"))
        );
        // El nombre del repositorio también se acorta y se limpia.
        assert_eq!(
            id_de("persona:ana", "python", Some("packages/hr/Con Espacios")),
            "puesto-ana-python-con-espacios"
        );
        assert_eq!(
            id_de("persona:ana", "python", Some("  ")),
            "puesto-ana-python",
            "sin repositorio, el de siempre"
        );
    }

    #[test]
    fn cada_lenguaje_corre_en_su_entorno_y_sql_en_todos() {
        assert_eq!(entorno_de("python"), Some("python"));
        assert_eq!(entorno_de("typescript"), Some("node"));
        assert_eq!(entorno_de("javascript"), Some("node"));
        assert_eq!(entorno_de("java"), Some("jvm"));
        assert_eq!(entorno_de("sql"), Some("python"));
        assert_eq!(entorno_de("jvm"), Some("jvm"));
        assert_eq!(entorno_de("rust"), None);
        assert!(corre_en("sql", "node") && corre_en("sql", "jvm") && corre_en("sql", "python"));
        assert!(corre_en("typescript", "node") && !corre_en("typescript", "python"));
        assert!(corre_en("java", "jvm") && !corre_en("python", "jvm"));
    }

    #[test]
    fn nombra_en_bloque_y_en_linea_y_no_por_prefijo() {
        assert!(nombra(
            "metadata: { name: espanoles, namespace: hr }",
            "espanoles"
        ));
        assert!(nombra(
            "metadata:
  name: espanoles
",
            "espanoles"
        ));
        assert!(!nombra("metadata: { name: espanoles2 }", "espanoles"));
    }

    #[test]
    fn un_agente_se_reconoce_por_tipo_o_por_prefijo() {
        let a = Identidad {
            persona: "x".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: Some("agente".into()),
        };
        let b = Identidad {
            persona: "agente:puesto-ana".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: None,
        };
        let c = Identidad {
            persona: "persona:ana".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: None,
        };
        assert!(es_agente(&a) && es_agente(&b) && !es_agente(&c));
    }
}
