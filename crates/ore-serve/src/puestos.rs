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
use ore_entrada::http::{Emisor, Flujo, Respuesta, Salida};
use ore_entrada::identidad::Identidad;
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Cuánto se retiene una espera (el agente pidiendo trabajo, la consola
/// esperando una salida). Por debajo del plazo de lectura de `http.rs` (30 s).
const ESPERA: Duration = Duration::from_secs(20);
/// Sin latido del agente durante esto, el puesto está perdido.
const SIN_LATIDO: Duration = Duration::from_secs(90);
/// Cada cuánto escribe algo un flujo que no tiene nada que contar. Por debajo
/// de lo que cualquier intermediario da por muerta una conexión callada.
const LATIDO: Duration = Duration::from_secs(10);
/// Cuántos mensajes de vuelta se guardan sin que nadie los recoja. Un editor
/// cerrado no puede hacer crecer la memoria de este servidor: lo más viejo se
/// tira, y quien vuelva lo notará porque su número saltó.
const LSP_RETENIDOS: usize = 500;
/// Lo que vive un flujo antes de despedirse él mismo. No es un límite técnico:
/// es que **el balanceador corta igualmente** (0037 ②, plazo de backend), y
/// más vale terminar diciendo «vuelve» —con el número por el que ibas— que
/// dejar que lo corten en medio de un evento.
const VIDA: Duration = Duration::from_secs(240);
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
    /// Lo que el agente corre, si no es `texto` tal cual: una celda `sql` que
    /// escribe en el árbol corre como la celda de `celda_de_sql` (`python`),
    /// y lo que la persona escribió sigue siendo su SQL.
    pub corre: Option<(String, String)>,
    /// Lo que se dice sin parar la celda (0038: `ORE-SQL-2P`), como
    /// diagnósticos con `severidad: aviso`; van en su ficha, junto a la salida.
    pub avisos: Vec<Json>,
    pub enviada: Instant,
    pub empezada: Option<Instant>,
    pub salida: Option<Json>,
    /// **De qué guion es** (0039): las sentencias de un `.sql` de varias
    /// corren como celdas seguidas, y si una falla las de detrás no corren.
    pub lote: Option<Lote>,
}

/// El lugar de una celda en su guion (0039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lote {
    /// El número de la primera celda del guion: el que lo nombra.
    pub primera: u64,
    /// Qué sentencia es (desde 0) y cuántas hay.
    pub i: usize,
    pub n: usize,
}

/// Una sentencia de un guion, lista para ser su celda.
struct SentenciaDelLote {
    texto: String,
    corre: (String, &'static str),
    avisos: Vec<Json>,
    pos: Option<ore_core::diag::Pos>,
    que: &'static str,
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
    /// **El servidor de lenguaje** (0037 ③a), en las dos direcciones.
    ///
    /// Lo que el editor manda (`textDocument/didChange`, `completion`…) espera
    /// aquí a que el agente lo recoja; lo que el servidor contesta espera aquí
    /// a que el editor lo recoja. Son mensajes de LSP tal cual, sin mirarlos:
    /// este servidor es el CONDUCTO, no el que entiende.
    pub lsp_al_servidor: VecDeque<Json>,
    /// Qué flujo del agente recoge lo de arriba: el ÚLTIMO que se abrió. Un
    /// agente que se reinicia abre uno nuevo, y el viejo —que el servidor aún
    /// cree abierto— deja de recoger (se llevaba los mensajes a una conexión
    /// muerta: medido en `el-editor-sql-a-fondo.py`, 1 de cada 6 relevos).
    pub lsp_generacion: u64,
    /// Lo de vuelta, cada uno con su número —para que un editor que reconecta
    /// diga por dónde iba y no repita ni pierda.
    pub lsp_a_la_consola: VecDeque<(u64, Json)>,
    pub lsp_siguiente: u64,
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

/// ¿Este entorno declara sus dependencias en el árbol? `python` desde W3.2
/// (`pyproject.toml`) y `jvm` desde 0037 ③c (`pom.xml`). `node` no: su sesión
/// nace con lo que trae su imagen, y sembrar un `package.json` que nadie
/// resuelve sería sembrar una promesa.
pub(crate) fn declara_capa(entorno: &str) -> bool {
    entorno == crate::entorno::PYTHON || entorno == crate::entorno::JVM
}

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
    if !c.avisos.is_empty() {
        m.insert("avisos".to_string(), Json::Arr(c.avisos.clone()));
    }
    if let Some(l) = c.lote {
        m.insert(
            "lote".to_string(),
            Json::obj([
                ("primera", Json::Int(l.primera as i64)),
                ("i", Json::Int(l.i as i64)),
                ("n", Json::Int(l.n as i64)),
            ]),
        );
    }
    Json::Obj(m)
}

/// Un fallo o un aviso del SQL del árbol, en la forma de los diagnósticos del
/// árbol (`fichero`, `linea`, `columna`), que es la que el editor pinta.
fn diagnostico(f: &ore_core::sql_del_arbol::Fallo, fichero: &str, severidad: &str) -> Json {
    let mut m = vec![
        ("codigo", Json::s(f.codigo.unwrap_or(""))),
        ("mensaje", Json::s(&f.mensaje)),
        ("fichero", Json::s(fichero)),
        ("severidad", Json::s(severidad)),
    ];
    if let Some(p) = f.pos {
        m.push(("linea", Json::Int(p.line as i64)));
        m.push(("columna", Json::Int(p.col as i64)));
    }
    if let Some(a) = &f.ayuda {
        m.push(("ayuda", Json::s(a)));
    }
    Json::obj(m)
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
                // ⭐ EL ENTORNO LO PIDE LA CLASE (⑧b): un `transforms-java` abre
                //   un puesto jvm y no uno de python — su código no corre en el
                //   otro. Se dice con su nombre en vez de abrir el que no es, y
                //   la consola lo manda ya resuelto: las clases viajan en el
                //   índice (`clases[]`) con su `lenguaje`.
                if let Some(c) = c
                    && let Some(suyo) = ore_core::clases::entorno_de(c)
                    && suyo != entorno
                {
                    return Respuesta::error(
                        422,
                        format!(
                            "`{}` es un repositorio `{}`: su puesto es `{suyo}`, no `{entorno}` (pide `lenguaje: {}`)",
                            a, c.id, c.lenguaje
                        ),
                    );
                }
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
        //   Desde 0037 ③c son DOS: `python` (`pyproject.toml` → ruedas) y `jvm`
        //   (`pom.xml` → jars). `node` sigue naciendo con lo que trae su imagen.
        let e = if !declara_capa(entorno) {
            Json::obj([("estado", Json::s("sin-dependencias"))])
        } else {
            match self.leyendo_en(rama.as_deref(), |raiz| {
                if let Some(a) = &repositorio
                    && !raiz.join(a).is_dir()
                {
                    return Respuesta::error(404, format!("no hay `{a}` en el árbol"));
                }
                let e = crate::entorno::entorno_de_en(raiz, repositorio.as_deref(), entorno);
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
                    entorno,
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
            lsp_al_servidor: VecDeque::new(),
            lsp_generacion: 0,
            lsp_a_la_consola: VecDeque::new(),
            lsp_siguiente: 0,
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
            // `sql` corre donde corre python: no hay imagen de SQL. Lo que
            // corre de verdad lo decide `celda_de_sql` con la frase delante.
            Some("sql") => ("python", "sql"),
            _ => {
                return Respuesta::error(
                    422,
                    format!(
                        "`{codigo}` no es de ningún entorno: `.py`, `.ts`/`.js`/`.mjs`, `.java` o `.sql`"
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
        let (texto, commit, lenguaje, avisos) = match self.leyendo_en(rama.as_deref(), |raiz| {
            let ruta = raiz.join(&codigo);
            let Ok(texto) = std::fs::read_to_string(&ruta) else {
                return Respuesta::error(404, format!("no hay `{codigo}` en el árbol"));
            };
            if texto.len() > TEXTO_MAXIMO {
                return Respuesta::error(422, format!("`{codigo}` pasa de {TEXTO_MAXIMO} bytes"));
            }
            // Un `.sql` se analiza y se coteja con el árbol DE ESTE COMMIT antes
            // de encolar nada: lo que no es una unidad es 422 con su sitio, no
            // un Job que falla a los dos minutos.
            // 0038: y lo que se dice sin pararlo (`ORE-SQL-2P`), en la celda
            let avisos: Vec<Json> = if lenguaje == "sql" {
                ore_core::sql_del_arbol::analizar(&texto)
                    .map(|u| {
                        u.avisos
                            .iter()
                            .map(|f| diagnostico(f, &codigo, "aviso"))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let (texto, lenguaje) = if lenguaje == "sql" {
                match celda_de_sql(raiz, &codigo, &texto) {
                    Ok(c) => c,
                    Err(r) => return r,
                }
            } else {
                (texto, lenguaje)
            };
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
                ("lenguaje", Json::s(lenguaje)),
                ("commit", Json::s(commit)),
                ("avisos", Json::Arr(avisos)),
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
                    // un `.sql` que escribe corre como python (`celda_de_sql`)
                    match m.get("lenguaje") {
                        Some(Json::Str(s)) if s == "python" => "python",
                        _ => lenguaje,
                    },
                    match m.get("avisos") {
                        Some(Json::Arr(a)) => a.clone(),
                        _ => Vec::new(),
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
            lsp_al_servidor: VecDeque::new(),
            lsp_generacion: 0,
            lsp_a_la_consola: VecDeque::new(),
            lsp_siguiente: 0,
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
                corre: None,
                avisos,
                enviada: Instant::now(),
                empezada: None,
                salida: None,
                lote: None,
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
    /// lista (o ninguna, en un entorno que no declara); pendiente o con error,
    /// se encola y 409 con el motivo.
    fn capa_para(
        &self,
        entorno: &str,
        rama: Option<&str>,
        alcance: Option<&str>,
        sujeto: &Identidad,
    ) -> Result<String, Respuesta> {
        if !declara_capa(entorno) {
            return Ok(String::new());
        }
        let e = match self.leyendo_en(rama, |raiz| {
            let e = crate::entorno::entorno_de_en(raiz, alcance, entorno);
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
                    entorno,
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

    /// **Una celda `sql` que escribe en el árbol, ¿cómo corre?** Decidido
    /// (2026-09-24): en la sesión, una frase que crea o inserta en un
    /// `paquete.nombre` de un paquete del árbol ESCRIBE de verdad —el
    /// dataset, como `write()` desde Python—. Medido antes
    /// (`medida-el-sql-que-escribe.sh`): iba entera a `ore.sql()`, DuckDB la
    /// hacía en su memoria, daba `Count` y no dejaba nada, ni para la celda
    /// siguiente.
    ///
    /// Corre por el MISMO camino que un `.sql` como trabajo
    /// ([`celda_de_sql`], cotejado con el árbol de la rama del puesto): UNA
    /// sentencia, lo que lee y lo que escribe sacado de ella, un `@transform`
    /// con `write()`. Lo que no es una unidad es la salida de error de la
    /// celda, con sus diagnósticos. Lo que crea en otra parte (`tmp.t`, `x`)
    /// sigue siendo de DuckDB.
    fn sql_que_escribe(
        &self,
        id: &str,
        texto: &str,
        fichero: &str,
    ) -> Result<(Desvio, Vec<Json>), Respuesta> {
        use ore_core::sql_del_arbol::{EscribeEnElArbol, escribe_en_el_arbol};
        let rama = match self.puestos.lista.lock().unwrap().get(id) {
            Some(p) => p.rama.clone(),
            // el 404 (o el 403) lo dice quien sigue
            None => return Ok((Desvio::Ninguno, Vec::new())),
        };
        let mut desvio = Desvio::Ninguno;
        let mut avisos = Vec::new();
        let r = self.leyendo_en(rama.as_deref(), |raiz| {
            let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
            // ⭐ 0039: **un guion** —varias sentencias que el árbol sabe
            //   correr, cotejadas en orden— es una celda por sentencia. Si no
            //   coteja y escribe en el árbol, no corre nada (mejor que a medias);
            //   si no es del árbol (`create schema tmp; …`), es de DuckDB entero,
            //   como siempre.
            if let Ok(t) = ore_core::sql_del_arbol::guion::guion(texto)
                && t.len() > 1
            {
                let f = ore_core::sql_del_arbol::guion::cotejar_guion(&pkg, &t);
                if f.is_empty() {
                    desvio = Desvio::Guion(
                        t.iter()
                            .map(|x| SentenciaDelLote {
                                texto: x.texto.clone(),
                                corre: celda_de_sentencia(fichero, x),
                                avisos: x
                                    .avisos
                                    .iter()
                                    .map(|a| diagnostico(a, fichero, "aviso"))
                                    .collect(),
                                pos: x.pos,
                                que: x.sentencia.que(),
                            })
                            .collect(),
                    );
                    return Respuesta::ok(Json::obj([]));
                }
                if escribe_en_el_arbol(texto, &pkg).is_some() {
                    desvio = Desvio::Error(error_de_celda(
                        format!(
                            "el guion no corre —ninguna de sus {} sentencias—: {}",
                            t.len(),
                            f[0].mensaje
                        ),
                        Json::Arr(f.iter().map(|x| diagnostico(x, fichero, "error")).collect()),
                    ));
                    return Respuesta::ok(Json::obj([]));
                }
            }
            // 0038: los nombres de dos partes se dicen, lea o escriba la celda
            avisos = ore_core::sql_del_arbol::avisos_de_celda(texto, &pkg)
                .iter()
                .map(|f| diagnostico(f, fichero, "aviso"))
                .collect();
            desvio = match escribe_en_el_arbol(texto, &pkg) {
                None => Desvio::Ninguno,
                Some(EscribeEnElArbol::Vista(n)) => Desvio::Error(error_de_celda(
                    format!(
                        "`{n}` sería una View del árbol, y una View no nace de una celda: se \
                         declara (un `.yaml` en `packages/<paquete>/views/`, o `declare()`)"
                    ),
                    Json::Arr(Vec::new()),
                )),
                Some(que @ (EscribeEnElArbol::Tabla(_) | EscribeEnElArbol::Crea(_))) => match celda_de_sesion(raiz, fichero, texto) {
                    Ok((celda, "python")) => Desvio::Corre(celda),
                    Ok(_) => Desvio::Ninguno,
                    Err(r) => {
                        let (primero, diagnosticos) = match &r.cuerpo {
                            Json::Obj(m) => (
                                match m.get("diagnosticos") {
                                    Some(Json::Arr(d)) => d
                                        .first()
                                        .and_then(|d| match d {
                                            Json::Obj(x) => match x.get("mensaje") {
                                                Some(Json::Str(s)) => Some(s.clone()),
                                                _ => None,
                                            },
                                            _ => None,
                                        })
                                        .unwrap_or_default(),
                                    _ => String::new(),
                                },
                                m.get("diagnosticos")
                                    .cloned()
                                    .unwrap_or(Json::Arr(Vec::new())),
                            ),
                            _ => (String::new(), Json::Arr(Vec::new())),
                        };
                        let dice = match que {
                            EscribeEnElArbol::Crea(n) => format!(
                                "la celda crea `{n}` en el catálogo, y eso corre como una sentencia \
                                 del árbol: {primero}"
                            ),
                            EscribeEnElArbol::Tabla(n) | EscribeEnElArbol::Vista(n) => format!(
                                "la celda escribe en `{n}` (el lago), y lo que escribe en el lago \
                                 corre como un `.sql` del árbol: {primero}"
                            ),
                        };
                        Desvio::Error(error_de_celda(dice, diagnosticos))
                    }
                },
            };
            Respuesta::ok(Json::obj([]))
        });
        if r.codigo != 200 {
            return Err(r);
        }
        Ok((desvio, avisos))
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
        let desvio = if lenguaje == "sql" {
            // el `.sql` del editor, si lo dice: da nombre al transform y a los
            // diagnósticos; sin él, `consulta.sql`
            let fichero = n
                .get("fichero")
                .and_then(|(_, v)| v.as_str())
                .filter(|f| f.ends_with(".sql") && !f.chars().any(char::is_control))
                .unwrap_or("consulta.sql");
            match self.sql_que_escribe(id, texto, fichero) {
                Ok(d) => d,
                Err(r) => return r,
            }
        } else {
            (Desvio::Ninguno, Vec::new())
        };
        let (desvio, avisos) = desvio;
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
        if let Desvio::Guion(sentencias) = desvio {
            // Una celda por sentencia, seguidas en la cola: el agente las corre
            // de una en una, en la misma sesión, y lo que escribe una lo lee la
            // siguiente.
            let n = sentencias.len();
            let mut celdas = Vec::new();
            let mut dichas = Vec::new();
            for (i, s) in sentencias.into_iter().enumerate() {
                let k = num + i as u64;
                p.celdas.insert(
                    k,
                    Celda {
                        texto: s.texto.clone(),
                        lenguaje: lenguaje.clone(),
                        corre: Some((s.corre.0, s.corre.1.to_string())),
                        avisos: s.avisos,
                        enviada: Instant::now(),
                        empezada: None,
                        salida: None,
                        lote: Some(Lote { primera: num, i, n }),
                    },
                );
                p.pendientes.push_back(k);
                celdas.push(Json::Int(k as i64));
                let mut d = vec![
                    ("celda", Json::Int(k as i64)),
                    ("que", Json::s(s.que)),
                    ("texto", Json::s(&s.texto)),
                ];
                if let Some(pos) = s.pos {
                    d.push(("linea", Json::Int(pos.line as i64)));
                    d.push(("columna", Json::Int(pos.col as i64)));
                }
                dichas.push(Json::obj(d));
            }
            p.siguiente += n as u64;
            let estado = p.estado.dice();
            drop(lista);
            self.puestos.campana.notify_all();
            return Respuesta {
                codigo: 202,
                cuerpo: Json::obj([
                    ("celda", Json::Int(num as i64)),
                    ("celdas", Json::Arr(celdas)),
                    ("sentencias", Json::Arr(dichas)),
                    ("puesto", Json::s(estado)),
                ]),
            };
        }
        p.siguiente += 1;
        let (corre, error) = match desvio {
            Desvio::Ninguno => (None, None),
            Desvio::Corre(c) => (Some((c, "python".to_string())), None),
            Desvio::Error(e) => (None, Some(e)),
            Desvio::Guion(_) => unreachable!("el guion ya se encoló"),
        };
        // una frase que no se puede correr se dice YA, como la salida de su
        // celda: el agente no llega a verla
        let hecha = error.is_some();
        p.celdas.insert(
            num,
            Celda {
                texto: texto.to_string(),
                lenguaje,
                corre,
                avisos,
                enviada: Instant::now(),
                empezada: hecha.then(Instant::now),
                salida: error,
                lote: None,
            },
        );
        if !hecha {
            p.pendientes.push_back(num);
        }
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

    /// `GET /puestos/{id}/flujo`: lo que le pasa a este puesto, **mientras
    /// pasa**, como eventos (`text/event-stream`).
    ///
    /// ⭐ Es la tubería de 0037 ②. Hasta hoy la consola preguntaba una vez por
    ///   celda (`GET …/celdas/{n}`, 20 s retenidos) y una conexión por
    ///   pregunta: sirve para una celda cada pocos segundos y no sirve para lo
    ///   que viene —un servicio de lenguaje son 84 mensajes por segundo—.
    ///
    /// Se retoma con `last-event-id`, que es lo que un `EventSource` manda solo
    /// al reconectar: se emiten las celdas con número mayor que ese, así que
    /// una conexión cortada no pierde ni repite nada.
    ///
    /// ⛔ NADA SE ESCRIBE CON EL CANDADO COGIDO. Escribir puede bloquear hasta
    ///   el plazo de escritura, y hacerlo con `lista` en la mano pararía a
    ///   TODOS los puestos —agentes incluidos— mientras un navegador lento lee.
    pub(crate) fn flujo_del_puesto(&self, sujeto: &Identidad, id: &str, desde: u64) -> Salida {
        // Lo que es un error se contesta como un error: con su código y su
        // motivo, antes de abrir nada.
        {
            let lista = self.puestos.lista.lock().unwrap();
            let Some(p) = lista.get(id) else {
                return Salida::Una(Respuesta::error(
                    404,
                    format!("no hay ningún puesto `{id}`"),
                ));
            };
            if p.persona != sujeto.persona {
                return Salida::Una(Respuesta::error(403, "ese puesto es de otra persona"));
            }
        }
        let puestos = Arc::clone(&self.puestos);
        let id = id.to_string();
        Salida::Flujo(Flujo {
            tipo: "text/event-stream",
            escribir: Box::new(move |e: &mut Emisor<'_>| emitir_puesto(&puestos, &id, desde, e)),
        })
    }

    // ── el servidor de lenguaje (0037 ③a) ───────────────────────────────────
    //
    // ⭐ ESTE SERVIDOR NO ENTIENDE LSP, Y ES A PROPÓSITO. Lo que viaja son los
    //   mensajes tal cual: el editor los escribe, el agente los da al servidor
    //   de lenguaje que corre en el puesto, y lo que conteste vuelve por el
    //   mismo sitio. Interpretarlos aquí sería poner a este proceso —el que
    //   guarda el árbol de todo el mundo— a analizar código de alguien.
    //
    // ⛔ Y el puesto es de UNA persona: el editor que manda es el suyo, y el
    //   agente que recoge es el que reclamó el puesto. Las dos puertas son las
    //   mismas que ya tenían las celdas.

    /// `POST /puestos/{id}/lsp {mensajes: [...]}`: lo que el editor le dice al
    /// servidor de lenguaje. 202 y el número de los que quedan por recoger.
    pub(crate) fn lsp_de_la_consola(
        &self,
        sujeto: &Identidad,
        id: &str,
        cuerpo: &str,
    ) -> Respuesta {
        let mensajes = match mensajes_de(cuerpo) {
            Ok(m) => m,
            Err(r) => return r,
        };
        let mut lista = self.puestos.lista.lock().unwrap();
        let Some(p) = lista.get_mut(id) else {
            return Respuesta::error(404, format!("no hay ningún puesto `{id}`"));
        };
        if p.persona != sujeto.persona {
            return Respuesta::error(403, "ese puesto es de otra persona");
        }
        if p.estado == Estado::Cerrado {
            return Respuesta::error(410, "el puesto está cerrado");
        }
        for m in mensajes {
            p.lsp_al_servidor.push_back(m);
        }
        let quedan = p.lsp_al_servidor.len();
        drop(lista);
        self.puestos.campana.notify_all();
        Respuesta {
            codigo: 202,
            cuerpo: Json::obj([("pendientes", Json::Int(quedan as i64))]),
        }
    }

    /// `POST /puestos/{id}/lsp/salida {mensajes: [...]}`: lo que el servidor de
    /// lenguaje contesta. Lo manda el agente.
    pub(crate) fn lsp_del_servidor(&self, sujeto: &Identidad, id: &str, cuerpo: &str) -> Respuesta {
        let mensajes = match mensajes_de(cuerpo) {
            Ok(m) => m,
            Err(r) => return r,
        };
        let mut lista = self.puestos.lista.lock().unwrap();
        let p = match Self::reclamar(&mut lista, sujeto, id) {
            Ok(p) => p,
            Err(r) => return r,
        };
        for m in mensajes {
            p.lsp_siguiente += 1;
            let n = p.lsp_siguiente;
            p.lsp_a_la_consola.push_back((n, m));
            while p.lsp_a_la_consola.len() > LSP_RETENIDOS {
                p.lsp_a_la_consola.pop_front();
            }
        }
        let ultimo = p.lsp_siguiente;
        drop(lista);
        self.puestos.campana.notify_all();
        Respuesta::ok(Json::obj([("ultimo", Json::Int(ultimo as i64))]))
    }

    /// `GET /puestos/{id}/lsp/agente`: el flujo por el que el agente RECIBE lo
    /// que el editor manda. Un mensaje por evento, en cuanto llega.
    pub(crate) fn flujo_lsp_del_agente(&self, sujeto: &Identidad, id: &str) -> Salida {
        let generacion = {
            let mut lista = self.puestos.lista.lock().unwrap();
            let p = match Self::reclamar(&mut lista, sujeto, id) {
                Ok(p) => p,
                Err(r) => return Salida::Una(r),
            };
            p.lsp_generacion += 1;
            p.lsp_generacion
        };
        // el flujo viejo, si lo hay, se despierta y se va
        self.puestos.campana.notify_all();
        let puestos = Arc::clone(&self.puestos);
        let id = id.to_string();
        Salida::Flujo(Flujo {
            tipo: "text/event-stream",
            escribir: Box::new(move |e: &mut Emisor<'_>| {
                emitir_lsp(&puestos, &id, e, Some(generacion), 0);
            }),
        })
    }

    /// `GET /puestos/{id}/lsp/consola`: el flujo por el que el editor RECIBE lo
    /// que el servidor de lenguaje contesta. Se retoma con `last-event-id`.
    pub(crate) fn flujo_lsp_de_la_consola(
        &self,
        sujeto: &Identidad,
        id: &str,
        desde: u64,
    ) -> Salida {
        {
            let lista = self.puestos.lista.lock().unwrap();
            let Some(p) = lista.get(id) else {
                return Salida::Una(Respuesta::error(
                    404,
                    format!("no hay ningún puesto `{id}`"),
                ));
            };
            if p.persona != sujeto.persona {
                return Salida::Una(Respuesta::error(403, "ese puesto es de otra persona"));
            }
        }
        let puestos = Arc::clone(&self.puestos);
        let id = id.to_string();
        Salida::Flujo(Flujo {
            tipo: "text/event-stream",
            escribir: Box::new(move |e: &mut Emisor<'_>| {
                emitir_lsp(&puestos, &id, e, None, desde);
            }),
        })
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
                let (texto, lenguaje) = c
                    .corre
                    .clone()
                    .unwrap_or_else(|| (c.texto.clone(), c.lenguaje.clone()));
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
        // ⭐ 0039: una sentencia de un guion que falla para el guion, como en
        //   Databricks: las de detrás no corren y lo dicen («saltada», y por
        //   cuál). Lo que ya corrió, corrió: no hay transacción entre
        //   sentencias.
        let fallo = matches!(&leida, Json::Obj(m) if matches!(m.get("tipo"), Some(Json::Str(t)) if t == "error"));
        if fallo && let Some(l) = c.lote {
            let detras: Vec<u64> = ((n + 1)..(l.primera + l.n as u64)).collect();
            p.pendientes.retain(|k| !detras.contains(k));
            for k in detras {
                if let Some(d) = p.celdas.get_mut(&k)
                    && d.salida.is_none()
                {
                    d.empezada = Some(Instant::now());
                    d.salida = Some(Json::obj([
                        ("tipo", Json::s("vacia")),
                        ("saltada", Json::Bool(true)),
                        ("por", Json::Int(n as i64)),
                        ("ms", Json::Int(0)),
                    ]));
                }
            }
        }
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
        // `base.nombre` o `base.schema.nombre` (0038), en su forma corta: la
        // clave del árbol, la de los punteros y la que el transform declara.
        let vista = ore_core::normalize::a_corto(vista).into_owned();
        let vista = vista.as_str();
        let Some((ns, schema, nombre)) = ore_core::punteros::partes(vista) else {
            return Respuesta::error(
                422,
                "un nombre del árbol es `<base>.<schema>.<nombre>` (o `<base>.<nombre>`, en `default`)",
            );
        };
        if let Err(m) = crate::rutas::token(ns)
            .and(crate::rutas::token(schema))
            .and(crate::rutas::token(nombre))
        {
            return Respuesta::error(422, m);
        }
        // Lo declarado manda (⑤): mientras un transform corre, este puesto sólo
        // resuelve sus `inputs`. El mismo 403 que el SDK da, en el servidor.
        if let Some(t) = self.transform_de(id)
            && !t
                .inputs
                .iter()
                .any(|i| ore_core::normalize::a_corto(i) == vista)
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
            self.datos_o_vista(raiz, &ns, &nombre, &vista)
        });
        // **El fallback de rama** (0031 §4, W3.7 ③): una rama lee las copias
        // de `main` mientras no tenga las suyas. Lo que la rama no tiene —ni
        // el documento (404) ni el dataset (409)— se busca en `main`, y la
        // respuesta lo dice (`rama: main`); lo que la rama sí tiene manda.
        // Medido antes: sin esto, lo que `main` ganaba tras abrir la rama era
        // «no hay ninguna View» desde ella.
        if rama.is_some() && matches!(r.codigo, 404 | 409) {
            let mut de_main =
                self.leyendo_en(None, |raiz| self.datos_o_vista(raiz, &ns, &nombre, &vista));
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
            return Respuesta::error(
                422,
                "un transform declara `output`: `<base>.<schema>.<tabla>`",
            );
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
            .any(|x| !matches!(x.split('.').count(), 2 | 3))
        {
            return Respuesta::error(
                422,
                "`inputs` y `output` son `<base>.<schema>.<nombre>` (o `<base>.<nombre>`, en `default`)",
            );
        }
        // En su forma corta (0038): lo que `datos` y el catálogo comparan.
        let output = ore_core::normalize::a_corto(&output).into_owned();
        let inputs: Vec<String> = inputs
            .iter()
            .map(|i| ore_core::normalize::a_corto(i).into_owned())
            .collect();
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

    /// **`POST /puestos/{id}/sql {texto}`: `sql()` sin regex.**
    ///
    /// Los tres SDK buscaban los nombres con una regex (`VISTAS_EN_SQL`) que
    /// fallaba 5 de 13 casos —un nombre en un comentario mataba la celda, `from
    /// a, b` se saltaba la segunda— y cada nombre era una ida y vuelta. Ahora el
    /// texto entero viene aquí: `sql_del_arbol::nombres_a_resolver` (el
    /// tokenizador y el árbol DEL PUESTO como filtro; 52 de 52 medidos) dice qué
    /// nombres lee, y cada uno se resuelve con `datos_del_puesto` —el mismo
    /// camino que `GET datos`: lo declarado (⑤), el conducto, el fallback a
    /// `main`, la credencial, y una View como su pregunta—. Uno que no se
    /// resuelve devuelve su respuesta tal cual, con `nombre`: el SDK la
    /// convierte en el error de siempre (LookupError, RuntimeError,
    /// PermissionError). `{fuentes: {<p>.<n>: <lo de datos>}}`.
    pub(crate) fn sql_del_puesto(&self, sujeto: &Identidad, id: &str, cuerpo: &str) -> Respuesta {
        let rama = {
            let mut lista = self.puestos.lista.lock().unwrap();
            match Self::reclamar(&mut lista, sujeto, id) {
                Ok(p) => p.rama.clone(),
                Err(r) => return r,
            }
        };
        let texto = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n
                .get("texto")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("")
                .to_string(),
            Err(_) => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        if texto.trim().is_empty() {
            return Respuesta::error(422, "sql() quiere una consulta");
        }
        let r = self.leyendo_en(rama.as_deref(), |raiz| {
            let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
            Respuesta::ok(Json::Arr(
                ore_core::sql_del_arbol::nombres_a_resolver(&texto, &pkg)
                    .into_iter()
                    .map(Json::s)
                    .collect(),
            ))
        });
        let nombres: Vec<String> = match &r.cuerpo {
            Json::Arr(xs) if r.codigo == 200 => xs
                .iter()
                .filter_map(|x| match x {
                    Json::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
            _ => return r,
        };
        let mut fuentes = std::collections::BTreeMap::new();
        for n in nombres {
            let mut d = self.datos_del_puesto(sujeto, id, &n);
            if d.codigo != 200 {
                if let Json::Obj(m) = &mut d.cuerpo {
                    m.insert("nombre".into(), Json::s(&n));
                }
                return d;
            }
            fuentes.insert(n, d.cuerpo);
        }
        Respuesta::ok(Json::obj([("fuentes", Json::Obj(fuentes))]))
    }

    /// Lo que se lee por un nombre: un dataset por su puntero (`datos_de`), o
    /// **una View como la pregunta que es** (`datos_de_vista`). Si una View y
    /// un dataset se llaman igual, manda el dataset, como en `datos_de`.
    fn datos_o_vista(&self, raiz: &Path, ns: &str, nombre: &str, vista: &str) -> Respuesta {
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        let solo_vista = pkg.view(vista).is_some() && pkg.dataset(vista).is_none();
        if solo_vista {
            self.datos_de_vista(raiz, &pkg, vista)
        } else {
            self.con_credencial(raiz, datos_de(raiz, ns, nombre, vista))
        }
    }

    /// **Una View leída desde un puesto: su SQL sobre sus datasets.**
    ///
    /// Medido (`medida-la-vista-con-filtro.py`): hasta aquí una View se
    /// resolvía al puntero de su dataset raíz y el SDK hacía `select *` sobre
    /// él —20 000 filas y 4 columnas donde la View dice 5 000 y 2—. Ahora
    /// `ore ask --sql` la compila a SQL de DuckDB sobre sus datasets
    /// (`"__ore_dataset"."<p>.<n>"`), y **cada dataset se resuelve aquí mismo**
    /// con `datos_de` y su credencial: su conducto, su puntero, su 409 si no
    /// está. Aquí y no en el SDK por lo declarado (⑤): un transform que lee
    /// la View no declara sus datasets, y pedirlos aparte sería un 403.
    ///
    /// `{vista, consulta, columnas, datasets: {<p>.<n>: <lo que datos_de da>}}`.
    fn datos_de_vista(&self, raiz: &Path, pkg: &ore_core::link::Package, vista: &str) -> Respuesta {
        // Una View virtual —sobre una Table, sin dataset debajo— no tiene de
        // dónde leerse: lo mismo que decía `datos_de`, con las mismas palabras.
        if let Some(v) = pkg.view(vista)
            && ore_core::vistas::raiz_de_lectura(pkg, v).is_none()
        {
            return Respuesta::error(
                409,
                format!(
                    "`{vista}` es una `View` virtual: no tiene ningún dataset debajo del que leer. Declara un `Dataset` con `from` sobre ella, o léela como pregunta (sql)"
                ),
            );
        }
        if let Err(n) = ore_core::flow::lectura_desde_puesto(pkg, vista) {
            let mut r = Respuesta::error(403, format!("{}: {}", n.codigo, n.mensaje));
            if let Json::Obj(m) = &mut r.cuerpo {
                m.insert("codigo".into(), Json::s(n.codigo));
            }
            return r;
        }
        let args: Vec<String> = ["ask", ".", "--vista", vista, "--sql"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let salida = match crate::mando::correr(&self.binario, raiz, &args) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(500, e.to_string()),
        };
        let json = salida
            .stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| l.starts_with('{'))
            .and_then(|l| ore_core::parse::parse(l).ok());
        let Some(j) = json.filter(|_| salida.bien()) else {
            let m = salida
                .stderr
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("la View no se compila")
                .trim_start_matches("error: ")
                .to_string();
            return Respuesta::error(
                409,
                format!("`{vista}` no se puede leer desde un puesto: {m}"),
            );
        };
        let texto = |k: &str| {
            j.get(k)
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let mut datasets = std::collections::BTreeMap::new();
        for d in j
            .get("datasets")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|d| d.as_str())
        {
            let Some((dns, _, dn)) = ore_core::punteros::partes(d) else {
                continue;
            };
            let r = self.con_credencial(raiz, datos_de(raiz, dns, dn, d));
            if r.codigo != 200 {
                // El 403 del conducto o el 409 de una copia que no está, del
                // dataset: la View no se lee sin él, y se dice por qué.
                return r;
            }
            datasets.insert(d.to_string(), r.cuerpo);
        }
        Respuesta::ok(Json::obj([
            ("vista", Json::s(vista)),
            ("consulta", Json::s(texto("consulta"))),
            (
                "columnas",
                j.get("columnas")
                    .map(|(_, v)| Json::de_node(v))
                    .unwrap_or(Json::obj([])),
            ),
            ("datasets", Json::Obj(datasets)),
        ]))
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

/// Cómo corre una celda `sql` de la sesión ([`Servidor::sql_que_escribe`]).
enum Desvio {
    /// Tal cual, en `ore.sql()`: lee, o escribe en la memoria de DuckDB.
    Ninguno,
    /// Escribe en el árbol: el agente corre esta celda de Python.
    Corre(String),
    /// No se puede correr: esta es su salida, ya.
    Error(Json),
    /// Un guion (0039): una celda por sentencia, en orden.
    Guion(Vec<SentenciaDelLote>),
}

/// La salida `error` de una celda que no llega al agente (la forma de la del
/// agente, con los diagnósticos del árbol: `fichero`, `linea`, `columna`).
fn error_de_celda(mensaje: String, diagnosticos: Json) -> Json {
    Json::obj([
        ("tipo", Json::s("error")),
        ("nombre", Json::s("SQL")),
        ("mensaje", Json::s(mensaje)),
        ("traza", Json::s("")),
        ("texto", Json::s("")),
        ("ms", Json::Int(0)),
        ("diagnosticos", diagnosticos),
    ])
}

/// **Un `.sql` del árbol como la celda de un trabajo** (el SQL del árbol).
///
/// La frase se analiza y se coteja con el árbol de este commit
/// (`ore_core::sql_del_arbol`): lo que no es una unidad —o lee lo que no se
/// lee, o escribe lo que no se escribe— es 422 con los fallos en la forma de
/// los diagnósticos del árbol (`fichero`, `linea`, `columna`), que es lo que el
/// editor ya sabe pintar.
///
/// - Un `select` es un análisis: la celda es la consulta, en `sql`, y lo que
///   devuelve va al informe.
/// - Una frase que escribe se corre con **el mismo `@transform` que un `.py`**,
///   con lo que la frase declara: el servidor deja leer sólo eso y escribir
///   sólo eso (W3.7 gobierno ⑤), y lo escrito lleva la misma procedencia
///   `{codigo, inputs, transform}`. La celda la escribe este proceso a partir
///   del análisis, no el cliente: la declaración no puede mentir.
fn celda_de_sql(
    raiz: &Path,
    codigo: &str,
    texto: &str,
) -> Result<(String, &'static str), Respuesta> {
    use ore_core::sql_del_arbol::{Fallo, analizar, cotejar};
    let fallos: Vec<Fallo> = match analizar(texto) {
        Ok(u) => {
            let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
            let f = cotejar(&pkg, &u);
            if f.is_empty() {
                return Ok(celda_de_unidad(codigo, &u));
            }
            f
        }
        Err(f) => f,
    };
    Err(rechazo(codigo, &fallos))
}

/// **Una celda `sql` de la sesión que escribe o crea en el árbol** (0039): la
/// sentencia se analiza como parte de un guion —así caben las que crean:
/// `create … database`, `create schema`, `create dataset (cols)`— y se coteja
/// con el árbol de la rama del puesto.
fn celda_de_sesion(
    raiz: &Path,
    codigo: &str,
    texto: &str,
) -> Result<(String, &'static str), Respuesta> {
    use ore_core::sql_del_arbol::Fallo;
    use ore_core::sql_del_arbol::guion::{cotejar_guion, guion};
    let fallos: Vec<Fallo> = match guion(texto) {
        Ok(t) if t.len() > 1 => vec![
            Fallo::new(
                "una celda que escribe en el árbol es UNA sentencia",
                t[1].pos,
            )
            .ayuda("parte la celda en dos"),
        ],
        Ok(t) => {
            let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
            let f = cotejar_guion(&pkg, &t);
            if f.is_empty() {
                return Ok(celda_de_sentencia(codigo, &t[0]));
            }
            f
        }
        Err(f) => f,
    };
    Err(rechazo(codigo, &fallos))
}

/// **Una sentencia del guion, como la celda que la corre** (0039). La que lee
/// o escribe datos es [`celda_de_unidad`]; la que crea algo del catálogo llama
/// al verbo del SDK —`crear_base`, `crear_schema`, `crear_dataset`— con lo que
/// la frase dice. La escribe este proceso a partir del análisis, no el cliente.
fn celda_de_sentencia(
    codigo: &str,
    t: &ore_core::sql_del_arbol::guion::Trozo,
) -> (String, &'static str) {
    use ore_core::sql_del_arbol::guion::Sentencia as S;
    let c = |s: &str| Json::s(s).jcs();
    let si = |b: bool| if b { "True" } else { "False" };
    let cabeza = format!(
        "# `{codigo}`: `{}`, con lo que la frase dice (lo escribe ore-serve, no el cliente).\n",
        t.sentencia.que()
    );
    let cuerpo = match &t.sentencia {
        S::Unidad(u) => return celda_de_unidad(codigo, u),
        S::CrearBase {
            nombre,
            clase,
            origen,
            si_no_existe,
            ..
        } => {
            let (o, inc) = match origen {
                Some(o) => (
                    c(&o.nombre),
                    Json::Arr(o.incluye.iter().map(Json::s).collect()).jcs(),
                ),
                None => ("None".to_string(), "None".to_string()),
            };
            format!(
                "from ore import crear_base, _resultado_de_crear\n\n\
                 _hecho = crear_base({}, clase={}, origen={o}, incluye={inc}, si_no_existe={})\n\
                 print(\"%s · %s database · %s\" % (_hecho[\"base\"], _hecho[\"clase\"], \"creada\" if _hecho[\"creada\"] else \"ya estaba\"))\n\
                 _resultado_de_crear(\"%s database %s\" % (_hecho[\"clase\"], _hecho[\"base\"]), _hecho[\"creada\"])\n",
                c(nombre),
                c(clase.como_en_el_alta()),
                si(*si_no_existe)
            )
        }
        S::CrearSchema {
            base,
            schema,
            si_no_existe,
            ..
        } => format!(
            "from ore import crear_schema, _resultado_de_crear\n\n\
             _hecho = crear_schema({}, {}, si_no_existe={})\n\
             print(\"%s · schema · %s\" % (_hecho[\"schema\"], \"creado\" if _hecho[\"creado\"] else \"ya estaba\"))\n\
             _resultado_de_crear(\"schema \" + _hecho[\"schema\"], _hecho[\"creado\"])\n",
            c(base),
            c(schema),
            si(*si_no_existe)
        ),
        S::CrearDataset {
            destino,
            columnas,
            clave,
            si_no_existe,
        } => {
            let clave = if clave.is_empty() {
                "None".to_string()
            } else {
                Json::Arr(clave.iter().map(Json::s).collect()).jcs()
            };
            let cols = Json::Arr(
                columnas
                    .iter()
                    .map(|k| Json::Arr(vec![Json::s(&k.nombre), Json::s(&k.tipo)]))
                    .collect(),
            )
            .jcs();
            format!(
                "from ore import crear_dataset, _resultado_de_crear\n\n\
                 _hecho = crear_dataset({}, {cols}, clave={clave}, si_no_existe={})\n\
                 print(\"%s · dataset vacío · %s\" % (_hecho[\"dataset\"], \"creado\" if _hecho[\"creado\"] else \"ya estaba\"))\n\
                 _resultado_de_crear(\"dataset \" + _hecho[\"dataset\"], _hecho[\"creado\"])\n",
                c(&destino.referencia()),
                si(*si_no_existe)
            )
        }
    };
    (cabeza + &cuerpo, "python")
}

/// Los fallos de un `.sql` como la respuesta 422 que el editor sabe pintar.
fn rechazo(codigo: &str, fallos: &[ore_core::sql_del_arbol::Fallo]) -> Respuesta {
    let diagnosticos = fallos
        .iter()
        .map(|f| diagnostico(f, codigo, "error"))
        .collect();
    Respuesta {
        codigo: 422,
        cuerpo: Json::obj([
            (
                "error",
                Json::s(format!(
                    "`{codigo}` no es una unidad que se pueda correr: {}",
                    fallos[0].mensaje
                )),
            ),
            ("diagnosticos", Json::Arr(diagnosticos)),
        ]),
    }
}

/// La celda que corre una unidad ya cotejada. El nombre del transform es el
/// del fichero (`resumen.sql` → `resumen`), que es lo que la procedencia dice.
fn celda_de_unidad(codigo: &str, u: &ore_core::sql_del_arbol::Unidad) -> (String, &'static str) {
    let Some(e) = &u.escribe else {
        return (u.consulta.clone(), "sql");
    };
    let base = codigo
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".sql"))
        .unwrap_or("");
    let nombre = if !base.is_empty()
        && !base.starts_with(|c: char| c.is_ascii_digit())
        && base.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        base.to_string()
    } else {
        "consulta".to_string()
    };
    // Las cadenas van como JSON, que Python lee igual: comillas, saltos y
    // no-ASCII quedan escapados o tal cual, nunca abiertos.
    let cadena = |s: &str| Json::s(s).jcs();
    let inputs = Json::Arr(u.lee.iter().map(|n| Json::s(n.referencia())).collect()).jcs();
    let salida = cadena(&e.destino.referencia());
    // Un `insert` con columnas sin alias (`select letra, 0.5 from …`): ésas
    // toman el nombre de la columna de la tabla en su posición, como en SQL.
    let (mut importa, mut datos) = if e.por_posicion.is_empty() {
        (
            String::new(),
            format!("sql({}, como=\"arrow\")", cadena(&u.consulta)),
        )
    } else {
        let posiciones = Json::Arr(
            e.por_posicion
                .iter()
                .map(|p| Json::Int(*p as i64))
                .collect(),
        )
        .jcs();
        (
            ", _por_posicion".to_string(),
            format!(
                "_por_posicion(sql({}, como=\"arrow\"), {salida}, {posiciones})",
                cadena(&u.consulta)
            ),
        )
    };
    // 0039: un `insert` lleva cada valor al tipo de su columna, como en SQL
    // (`current_timestamp` es TIMESTAMPTZ; la columna, TIMESTAMP). Un
    // `create or replace` no: sus tipos son los de su consulta.
    if e.modo != ore_core::sql_del_arbol::Modo::Sobrescribir {
        importa.push_str(", _como_la_tabla");
        datos = format!("_como_la_tabla({datos}, {salida})");
    }
    let celda = format!(
        "# `{codigo}`: la frase declara lo que lee y lo que escribe, y corre con el\n\
         # mismo `@transform` que un `.py` (lo escribe ore-serve, no el cliente).\n\
         from ore import transform, sql, write, _resultado_de_escritura{importa}\n\
         \n\
         \n\
         @transform(inputs={inputs}, output={salida})\n\
         def {nombre}():\n    \
             return write({salida}, {datos}, modo={modo})\n\
         \n\
         \n\
         _escrito = {nombre}()\n\
         print(\"%s · %s · %d filas%s\" % ({salida}, {modo}, _escrito[\"filas\"], \" · la misma escritura: nada nuevo\" if _escrito[\"repetida\"] else \"\"))\n\
         _resultado_de_escritura(_escrito)\n",
        modo = cadena(e.modo.como_en_write()),
    );
    (celda, "python")
}

/// Lo que el árbol dice del dataset de un nombre (0031 §10, 0033: **un
/// lector, un camino**): el nombre resuelve a un `Dataset` y su puntero está
/// en `datasets/<p>_<n>.json`. Con `estado: copiada|al-dia` y
/// `metadata_location` (o `clave`, heredado), el dataset está; si no, 409 con
/// lo que el puntero diga. Una Table es 409 «sin dataset»: no hay camino por el
/// que esta ruta llegue a un origen. Y una View también es 409 aquí: se lee por
/// su pregunta (`datos_de_vista`), nunca por el puntero de su raíz.
fn datos_de(raiz: &Path, ns: &str, nombre: &str, vista: &str) -> Respuesta {
    // ¿Existe el documento? El puntero de algo que no está es un 404, no un 409.
    // Un dataset se lee por su puntero; una vista, por su pregunta; una tabla,
    // por ninguno. Se busca en el árbol compilado por la forma corta (0038:
    // `p.n` en `default`, `p.s.n` en otro schema; la carpeta no nombra nada),
    // y con la prioridad de siempre: Dataset, View, Table.
    let _ = nombre;
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let de = |k: ore_core::document::Kind| {
        pkg.docs
            .iter()
            .find(|d| d.kind == k && d.qname().as_deref() == Some(vista))
    };
    use ore_core::document::Kind;
    let del_dataset = if de(Kind::Dataset).is_some() {
        vista.to_string()
    } else if let Some(v) = de(Kind::View) {
        match ore_core::vistas::raiz_de_lectura(&pkg, v).and_then(|c| c.qname()) {
            // ⛔ Nunca el puntero de la raíz: leerlo con `select *` era no
            // aplicar la View (medido: 20 000 filas y 4 columnas donde dice
            // 5 000 y 2). Una View se lee por su pregunta, `datos_de_vista`.
            Some(_) => {
                return Respuesta::error(
                    409,
                    format!(
                        "`{vista}` es una `View`: se lee como la pregunta que es, no por el puntero de su dataset"
                    ),
                );
            }
            None => {
                return Respuesta::error(
                    409,
                    format!(
                        "`{vista}` es una `View` virtual: no tiene ningún dataset debajo del que leer. Declara un `Dataset` con `from` sobre ella, o léela como pregunta (sql)"
                    ),
                );
            }
        }
    } else if de(Kind::Table).is_some() {
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
    let Some((_, n)) =
        ore_core::punteros::leer_en(&raiz.join(ore_core::punteros::CARPETA), &del_dataset)
    else {
        return Respuesta::error(
            409,
            format!(
                "el dataset `{del_dataset}` no está: aún no se copió, o nadie lo escribió todavía"
            ),
        );
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

/// Los mensajes de un cuerpo `{mensajes: ["<json>", ...]}`.
///
/// ⛔⛔ CADA MENSAJE VIAJA COMO UNA CADENA, Y NO ES UN CAPRICHO. Un mensaje de
///   LSP lleva `null`, dobles y anidamiento, y el `Json` de este árbol **no los
///   modela a propósito**: pasarlos por `de_node` devolvería las cadenas
///   `"null"` y `"1.5"` (está medido y escrito en `json.rs`). Así que aquí no
///   se analizan: se comprueba que son cadenas, se guardan tal cual y se emiten
///   tal cual (`Json::Crudo`). Este servidor es el CONDUCTO, no el que entiende
///   — interpretarlos sería poner al proceso que guarda el árbol de todo el
///   mundo a leer el código de alguien.
fn mensajes_de(cuerpo: &str) -> Result<Vec<Json>, Respuesta> {
    let Ok(nodo) = ore_core::parse::parse(cuerpo) else {
        return Err(Respuesta::error(400, "el cuerpo no es JSON"));
    };
    let Json::Obj(m) = crate::rutas::de_node(&nodo) else {
        return Err(Respuesta::error(422, "el cuerpo no es un objeto JSON"));
    };
    let Some(Json::Arr(ms)) = m.get("mensajes") else {
        return Err(Respuesta::error(422, "falta `mensajes`, y es una lista"));
    };
    if ms.len() > 200 {
        return Err(Respuesta::error(
            413,
            "demasiados mensajes de golpe (máximo 200)",
        ));
    }
    let mut fuera = Vec::with_capacity(ms.len());
    for m in ms {
        let Json::Str(t) = m else {
            return Err(Respuesta::error(
                422,
                "cada mensaje es una CADENA con el JSON dentro: este servidor no lo abre",
            ));
        };
        fuera.push(Json::Crudo(t.clone()));
    }
    Ok(fuera)
}

/// El cuerpo de los dos flujos de LSP. `del_agente` dice en qué dirección:
/// el agente recoge lo que el editor manda (y se CONSUME), el editor recoge lo
/// que el servidor contesta (y se RETIENE con su número, para poder volver).
///
/// ⛔ Sin el candado cogido mientras se escribe, como el flujo del puesto.
/// `generacion`: `Some` si es el flujo del AGENTE (el que recoge y consume lo
/// que el editor manda), con el número que le tocó al abrirse; `None` si es el
/// de la consola (el que lee lo de vuelta, sin consumir).
fn emitir_lsp(
    puestos: &Puestos,
    id: &str,
    e: &mut Emisor<'_>,
    generacion: Option<u64>,
    desde: u64,
) {
    let del_agente = generacion.is_some();
    let fin = Instant::now() + VIDA;
    let mut visto = desde;
    loop {
        // El del agente: si otro flujo más nuevo recoge, éste se va; y si el que
        // leía cerró, no se saca nada de la cola (se lo llevaría a la nada).
        if let Some(g) = generacion {
            let (relevado, hay) = {
                let lista = puestos.lista.lock().unwrap();
                match lista.get(id) {
                    Some(p) => (p.lsp_generacion != g, !p.lsp_al_servidor.is_empty()),
                    None => (false, false),
                }
            };
            if relevado {
                e.evento(
                    "fin",
                    None,
                    &Json::obj([("motivo", Json::s("otro flujo del agente recoge"))]),
                );
                return;
            }
            if hay && !e.vivo() {
                return;
            }
        }
        let (mensajes, fuera) = {
            let mut lista = puestos.lista.lock().unwrap();
            let Some(p) = lista.get_mut(id) else {
                drop(lista);
                e.evento(
                    "fin",
                    None,
                    &Json::obj([("motivo", Json::s("el puesto ya no está"))]),
                );
                return;
            };
            let fuera = p.estado == Estado::Cerrado || perdido(p);
            let mensajes: Vec<(Option<u64>, Json)> = if del_agente {
                if generacion != Some(p.lsp_generacion) {
                    Vec::new()
                } else {
                    p.lsp_al_servidor.drain(..).map(|m| (None, m)).collect()
                }
            } else {
                p.lsp_a_la_consola
                    .iter()
                    .filter(|(n, _)| *n > visto)
                    .map(|(n, m)| (Some(*n), m.clone()))
                    .collect()
            };
            (mensajes, fuera)
        };
        for (n, m) in mensajes {
            if !e.evento("lsp", n, &m) {
                return;
            }
            if let Some(n) = n {
                visto = n;
            }
        }
        if fuera {
            e.evento(
                "fin",
                None,
                &Json::obj([("motivo", Json::s("el puesto se acabó"))]),
            );
            return;
        }
        if Instant::now() >= fin {
            e.evento(
                "fin",
                Some(visto),
                &Json::obj([("motivo", Json::s("vuelve"))]),
            );
            return;
        }
        if !e.latido() {
            return;
        }
        let lista = puestos.lista.lock().unwrap();
        let _ = puestos.campana.wait_timeout(lista, LATIDO).unwrap();
    }
}

/// El cuerpo del flujo: mira, suelta el candado, escribe, espera la campana.
fn emitir_puesto(puestos: &Puestos, id: &str, desde: u64, e: &mut Emisor<'_>) {
    let fin = Instant::now() + VIDA;
    let mut visto = desde;
    let mut dicho = String::new();
    loop {
        // ── lo que hay que contar, copiado bajo el candado y nada más ──
        let (ficha_ahora, estado, nuevas, acabado) = {
            let lista = puestos.lista.lock().unwrap();
            let Some(p) = lista.get(id) else {
                // El puesto desapareció de la lista: se dice y se cierra.
                drop(lista);
                e.evento(
                    "fin",
                    None,
                    &Json::obj([("motivo", Json::s("el puesto ya no está"))]),
                );
                return;
            };
            let estado = if perdido(p) {
                "perdido"
            } else {
                p.estado.dice()
            };
            let nuevas: Vec<(u64, Json)> = p
                .celdas
                .iter()
                .filter(|(n, c)| **n > visto && c.salida.is_some())
                .map(|(n, c)| (*n, ficha_de_celda(*n, c)))
                .collect();
            (
                ficha(id, p),
                estado.to_string(),
                nuevas,
                estado == "cerrado" || estado == "perdido",
            )
        };

        // ── y ahora, sin candado, se escribe ──
        if estado != dicho {
            if !e.evento("puesto", None, &ficha_ahora) {
                return;
            }
            dicho = estado;
        }
        for (n, f) in nuevas {
            if !e.evento("celda", Some(n), &f) {
                return;
            }
            visto = n;
        }
        if acabado {
            e.evento(
                "fin",
                None,
                &Json::obj([("motivo", Json::s(format!("el puesto está {dicho}")))]),
            );
            return;
        }
        if Instant::now() >= fin {
            // Se despide él, y dice por dónde iba: quien lea vuelve con
            // `last-event-id` y no se pierde nada.
            e.evento(
                "fin",
                Some(visto),
                &Json::obj([("motivo", Json::s("vuelve"))]),
            );
            return;
        }
        if !e.latido() {
            return;
        }
        let lista = puestos.lista.lock().unwrap();
        let _ = puestos.campana.wait_timeout(lista, LATIDO).unwrap();
    }
}

#[cfg(test)]
mod prueba {
    use super::*;

    /// **Un lector, un camino** (0031 §10, 0033): un dataset resuelve por su
    /// puntero en `datasets/`; una View no (se lee por su pregunta); una View
    /// virtual y una Table son 409, y lo que no está es 404.
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
        // La vista sobre el dataset NO da el puntero del dataset: leerlo con
        // `select *` era no aplicar la View. Va por `datos_de_vista`.
        let r = datos_de(&d, "v", "grandes", "v.grandes");
        assert_eq!(r.codigo, 409, "{:?}", r.cuerpo);
        assert!(
            r.cuerpo.jcs().contains("como la pregunta que es"),
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

        // 0038: un dataset en el schema `espana`, por su nombre de tres
        // partes (la forma corta `v.espana.clientes`), con su puntero en su
        // sitio; el mismo nombre en `default` no es él
        std::fs::create_dir_all(d.join("packages/v/espana/datasets")).unwrap();
        std::fs::write(
            d.join("packages/v/espana/schema.yaml"),
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: espana, namespace: v }\nspec: { owner: team:v }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/espana/datasets/clientes.yaml"),
            "apiVersion: oos.dev/v1alpha13\nkind: Dataset\nmetadata: { name: clientes, namespace: v, schema: espana }\nspec:\n  owner: team:v\n  columns: { id: { type: Integer } }\n  changes: { mode: append }\n",
        )
        .unwrap();
        std::fs::create_dir_all(d.join("datasets/v/espana")).unwrap();
        std::fs::write(
            d.join("datasets/v/espana/clientes.json"),
            "{\"estado\":\"copiada\",\"metadata_location\":\"gs://b/ore/v2/catalogo/v/espana/clientes/metadata/1.metadata.json\",\"snapshot\":\"1\",\"dataset\":\"catalogo/v/espana/clientes\"}",
        )
        .unwrap();
        let r = datos_de(&d, "v", "clientes", "v.espana.clientes");
        assert_eq!(r.codigo, 200, "{:?}", r.cuerpo);
        assert!(
            r.cuerpo.jcs().contains("catalogo/v/espana/clientes"),
            "{}",
            r.cuerpo.jcs()
        );
        let r = datos_de(&d, "v", "clientes", "v.clientes");
        assert_eq!(r.codigo, 404, "{:?}", r.cuerpo);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// **Un `.sql` como trabajo** (el SQL del árbol): un `select` corre tal
    /// cual en `sql`; lo que escribe corre con el `@transform` que su frase
    /// declara, escrito por el servidor; lo que no es una unidad es 422 con los
    /// fallos en la forma de los diagnósticos del árbol.
    #[test]
    fn un_sql_del_arbol_es_la_celda_de_un_trabajo() {
        let d = std::env::temp_dir().join(format!("ore-celda-sql-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let escribe = |rel: &str, t: &str| {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        escribe(
            "ontology.config.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n",
        );
        escribe(
            "packages/v/package.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: v, version: 0.1.0, status: active, domain: v }\nspec: { owner: team:v }\n",
        );
        escribe(
            "packages/v/tables/origen.yaml",
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: origen, namespace: v }\nspec:\n  datasource: pg\n  object: public.origen\n  columns: { id: { type: Integer } }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
        );
        escribe(
            "packages/v/datasets/pedidos.yaml",
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: v }\nspec:\n  owner: team:v\n  from: { table: v.origen }\n",
        );

        let (celda, lenguaje) = celda_de_sql(
            &d,
            "packages/v/transforms/mira.sql",
            "select count(*) from v.pedidos;\n",
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(
            (celda.as_str(), lenguaje),
            ("select count(*) from v.pedidos", "sql")
        );

        let (celda, lenguaje) = celda_de_sql(
            &d,
            "packages/v/transforms/resumen.sql",
            "insert or replace into v.resumen\n-- los \"grandes\"\nselect id from v.pedidos where id > 1\n",
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert_eq!(lenguaje, "python");
        for trozo in [
            "@transform(inputs=[\"v.pedidos\"], output=\"v.resumen\")",
            "def resumen():",
            "return write(\"v.resumen\", _como_la_tabla(sql(\"select id from v.pedidos where id > 1\", como=\"arrow\"), \"v.resumen\"), modo=\"upsert\")",
            "_escrito = resumen()",
            "print(\"%s · %s · %d filas%s\" % (\"v.resumen\", \"upsert\", _escrito[\"filas\"]",
        ] {
            assert!(celda.contains(trozo), "falta {trozo:?} en:\n{celda}");
        }
        // un nombre de fichero que no es un identificador no rompe la celda
        let (celda, _) = celda_de_sql(
            &d,
            "packages/v/transforms/1-resumen.sql",
            "create or replace dataset v.r as select * from v.pedidos",
        )
        .unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
        assert!(celda.contains("def consulta():"), "{celda}");

        let r = celda_de_sql(
            &d,
            "packages/v/transforms/malo.sql",
            "select *\nfrom v.origen join v.nadie using (id)",
        )
        .unwrap_err();
        assert_eq!(r.codigo, 422);
        let j = r.cuerpo.jcs();
        assert!(
            j.contains("`Table` de una fuente") && j.contains("v.nadie"),
            "{j}"
        );
        assert!(
            j.contains("\"fichero\":\"packages/v/transforms/malo.sql\",\"linea\":2")
                || (j.contains("\"linea\":2") && j.contains("\"columna\":6")),
            "{j}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0039: lo que crea en el catálogo corre como el verbo del SDK, con lo
    /// que la frase dice; lo que lee o escribe, como siempre.
    #[test]
    fn cada_sentencia_del_guion_es_su_celda() {
        use ore_core::sql_del_arbol::guion::guion;
        let celda = |q: &str| {
            let t = guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}"));
            celda_de_sentencia("x.sql", &t[0])
        };
        let (c, l) = celda("create schema if not exists ventas.demo");
        assert_eq!(l, "python");
        assert!(
            c.contains("crear_schema(\"ventas\", \"demo\", si_no_existe=True)"),
            "{c}"
        );
        let (c, _) = celda("create dataset ventas.demo.clientes (id bigint, n varchar)");
        assert!(
            c.contains("crear_dataset(\"ventas.demo.clientes\", [[\"id\",\"long\"],[\"n\",\"string\"]], clave=None, si_no_existe=False)"),
            "{c}"
        );
        let (c, _) = celda("create foreign database espejo from origin erp include (s.*, t.x)");
        assert!(
            c.contains("crear_base(\"espejo\", clase=\"foreign\", origen=\"erp\", incluye=[\"s.*\",\"t.x\"], si_no_existe=False)"),
            "{c}"
        );
        let (c, _) = celda("create database mi_base");
        assert!(
            c.contains("clase=\"standard\", origen=None, incluye=None"),
            "{c}"
        );
        let (c, l) = celda("select 1");
        assert_eq!((c.as_str(), l), ("select 1", "sql"));
        let (c, _) = celda("insert into ventas.x (a) values (1)");
        assert!(
            c.contains("@transform(inputs=[], output=\"ventas.x\")"),
            "{c}"
        );
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
