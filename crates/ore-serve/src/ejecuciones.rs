//! **`system.default.executions`** (docs/ejecuciones.md, X1): una fila por
//! ejecución —lo que se pulsa en una hoja de Query, una celda, un Build, un
//! Job—, escrita por este proceso al verla terminar. El código del cliente no
//! la escribe: como la escritura de E2, la cuenta el servidor.
//!
//! - **Dónde** (§8.1 ①): una tabla por persona (`system/executions/users/<h>`)
//!   y otra de la organización (`system/executions/org`). Cada fila va a las
//!   dos; cada uno lee la suya y el admin la de la organización (X2). La
//!   credencial que el catálogo presta está acotada a una tabla, así que nadie
//!   alcanza los ficheros de otro.
//! - **Fuera del árbol** (§8.1 ②): sus punteros los guarda `ore-store anexar`
//!   en el lago, junto a la tabla. Un lote cada pocos segundos no cabe en la
//!   historia del árbol.
//! - **Sin hacer esperar** (§8.1 ③): cerrar una ejecución deja su fila en una
//!   cola; un hilo la anexa cada [`CADA`] o al juntar [`LOTE`]. Lo que falla
//!   espera al siguiente; un reinicio pierde, como mucho, lo que esperaba.
//!
//! Las columnas escalares van planas (`source_kind`, `error_message`…) y las
//! listas (`read`, `steps`) como JSON, que DuckDB lee con `from_json`: el lago
//! escribe filas planas.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use ore_core::json::Json;
use sha2::{Digest, Sha256};

use crate::puestos::Puesto;

/// El nombre con el que se lee (X2).
pub(crate) const NOMBRE: &str = "system.default.executions";
/// Cada cuánto se anexa lo que haya.
pub(crate) const CADA: Duration = Duration::from_secs(10);
/// Cuántas filas despiertan al hilo antes de [`CADA`].
pub(crate) const LOTE: usize = 500;
/// DX5 · lo que se guarda de `statement`; de lo demás, su huella.
pub(crate) const TOPE_DE_SENTENCIA: usize = 64 * 1024;
/// X5 DX10 · lo que se guarda del log de una ingesta: su FINAL, que es donde
/// está el motivo de un fallo.
pub(crate) const TOPE_DE_LOG: usize = 64 * 1024;
/// Lo más que espera en memoria si el lago no contesta: lo viejo se pierde
/// antes que el proceso.
const ESPERAN_COMO_MUCHO: usize = 20_000;

/// Ahora, en milisegundos de época.
pub(crate) fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Un paso de una ejecución: una sentencia de un guion, o (X5 DX11) un
/// dataset de una copia o una fuente de un catálogo.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Paso {
    pub statement: String,
    /// `ok`, `failed` o `skipped`.
    pub status: &'static str,
    pub duration_ms: i64,
    /// X5 · lo que una ingesta dice de cada paso; nada en una consulta.
    pub rows: Option<i64>,
    pub bytes: Option<i64>,
    pub error: Option<String>,
}

/// **Una ejecución terminada** (§8.2): lo que va a su fila.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ejecucion {
    pub id: String,
    /// `query`, `cell`, `build` o `job` (DX2).
    pub kind: &'static str,
    /// La persona, sin prefijo (va a la fila como `user:<persona>`).
    pub persona: String,
    pub source_kind: &'static str,
    pub source_id: Option<String>,
    pub source_commit: Option<String>,
    pub source_branch: Option<String>,
    /// Lo ejecutado, entero (se recorta al escribir la fila).
    pub statement: String,
    /// Milisegundos de época.
    pub queued_at: i64,
    pub started_at: i64,
    pub ended_at: i64,
    /// `ok` o `failed`.
    pub status: &'static str,
    pub error_name: Option<String>,
    pub error_message: Option<String>,
    /// `L` o `G` (E corre en G hasta Query Q7).
    pub compute: &'static str,
    pub worker: String,
    /// X3 · la CPU de sus celdas, sumada; la memoria y el derrame, lo más alto.
    /// `None` si el agente no lo midió (uno de antes).
    pub cpu_ms: Option<i64>,
    pub peak_memory_mb: Option<i64>,
    pub spill_mb: Option<i64>,
    /// Lo leído durante la ejecución, y si fue en vivo.
    pub read: Vec<(String, bool)>,
    /// En un guion de varias sentencias, una por paso.
    pub steps: Vec<Paso>,
    /// Elastic Sessions S2b · Lo que la sesión creció mientras corría (`growth`),
    /// el tope con que acabó (sólo en el session pool) y, si quiso crecer y no
    /// pudo, por qué.
    pub growth: Vec<crate::elastica::Crecimiento>,
    pub memory_final_mb: Option<i64>,
    pub growth_refused: Option<String>,
    /// X5 DX10 · el final del log del ejecutor, en una ingesta.
    pub log: Option<String>,
}

/// La huella de una sentencia (§4 `fingerprint`): sin mayúsculas ni espacios
/// de más, para que la misma consulta escrita de otra forma agrupe igual. La
/// admisión la usará (X4).
pub(crate) fn huella(sentencia: &str) -> String {
    let normal = sentencia
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let d = Sha256::digest(normal.trim_end_matches(';').trim().as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lo que cabe del FINAL de `s` en `tope` bytes, sin partir un carácter.
fn recortar_el_final(s: &str, tope: usize) -> &str {
    if s.len() <= tope {
        return s;
    }
    let mut i = s.len() - tope;
    while !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
}

/// Lo que cabe de `s` en `tope` bytes, sin partir un carácter.
fn recortar(s: &str, tope: usize) -> &str {
    if s.len() <= tope {
        return s;
    }
    let mut i = tope;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// El instante, en la forma que el lago lee para `DateTimeTz`.
fn instante(ms: i64) -> String {
    let s = ore_core::frescura::iso(ms.div_euclid(1000));
    format!("{}.{:03}Z", s.trim_end_matches('Z'), ms.rem_euclid(1000))
}

impl Ejecucion {
    /// La fila, como una línea JSON de cadenas (lo que `ore-store anexar` lee).
    /// Lo que no se sabe no va: es nulo.
    pub(crate) fn fila(&self, organizacion: Option<&str>) -> String {
        let mut m: Vec<(&str, Json)> = vec![
            ("id", Json::s(&self.id)),
            ("kind", Json::s(self.kind)),
            (
                "subject",
                Json::s(format!("user:{}", handle(&self.persona))),
            ),
            ("source_kind", Json::s(self.source_kind)),
            (
                "statement",
                Json::s(recortar(&self.statement, TOPE_DE_SENTENCIA)),
            ),
            ("fingerprint", Json::s(huella(&self.statement))),
            ("queued_at", Json::s(instante(self.queued_at))),
            ("started_at", Json::s(instante(self.started_at))),
            ("ended_at", Json::s(instante(self.ended_at))),
            (
                "wait_ms",
                Json::s((self.started_at - self.queued_at).max(0).to_string()),
            ),
            (
                "duration_ms",
                Json::s((self.ended_at - self.started_at).max(0).to_string()),
            ),
            ("status", Json::s(self.status)),
            ("compute", Json::s(self.compute)),
            ("worker", Json::s(&self.worker)),
            (
                "read",
                Json::s(
                    Json::Arr(
                        self.read
                            .iter()
                            .map(|(n, vivo)| {
                                Json::obj([("name", Json::s(n)), ("live", Json::Bool(*vivo))])
                            })
                            .collect(),
                    )
                    .jcs(),
                ),
            ),
        ];
        for (k, v) in [
            ("organization", organizacion.map(String::from)),
            ("source_id", self.source_id.clone()),
            ("source_commit", self.source_commit.clone()),
            ("source_branch", self.source_branch.clone()),
            ("error_name", self.error_name.clone()),
            ("error_message", self.error_message.clone()),
            ("cpu_ms", self.cpu_ms.map(|v| v.to_string())),
            ("peak_memory_mb", self.peak_memory_mb.map(|v| v.to_string())),
            ("spill_mb", self.spill_mb.map(|v| v.to_string())),
            (
                "memory_final_mb",
                self.memory_final_mb.map(|v| v.to_string()),
            ),
            ("growth_refused", self.growth_refused.clone()),
            (
                "log",
                self.log
                    .as_deref()
                    .map(|l| recortar_el_final(l, TOPE_DE_LOG).to_string()),
            ),
        ] {
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                m.push((k, Json::s(v)));
            }
        }
        if !self.growth.is_empty() {
            m.push((
                "growth",
                Json::s(
                    Json::Arr(
                        self.growth
                            .iter()
                            .map(|g| {
                                Json::obj([
                                    ("at_ms", Json::Int(g.at_ms)),
                                    ("from_mb", Json::Int(g.from_mb as i64)),
                                    ("to_mb", Json::Int(g.to_mb as i64)),
                                    ("cpu_m", Json::Int(g.cpu_m as i64)),
                                    ("reason", Json::s(&g.reason)),
                                ])
                            })
                            .collect(),
                    )
                    .jcs(),
                ),
            ));
        }
        if !self.steps.is_empty() {
            m.push((
                "steps",
                Json::s(
                    Json::Arr(
                        self.steps
                            .iter()
                            .map(|p| {
                                let mut m = vec![
                                    (
                                        "statement",
                                        Json::s(recortar(&p.statement, TOPE_DE_SENTENCIA)),
                                    ),
                                    ("status", Json::s(p.status)),
                                    ("duration_ms", Json::Int(p.duration_ms)),
                                ];
                                if let Some(f) = p.rows {
                                    m.push(("rows", Json::Int(f)));
                                }
                                if let Some(b) = p.bytes {
                                    m.push(("bytes", Json::Int(b)));
                                }
                                if let Some(e) = &p.error {
                                    m.push(("error", Json::s(e)));
                                }
                                Json::obj(m)
                            })
                            .collect(),
                    )
                    .jcs(),
                ),
            ));
        }
        Json::obj(m).jcs()
    }
}

/// §8.1 ① · Las dos tablas a las que va la fila de una persona.
pub(crate) fn destinos(persona: &str) -> [String; 2] {
    [
        format!("system/executions/users/{}", handle(persona)),
        "system/executions/org".to_string(),
    ]
}

/// El handle de una persona (`persona:ana`, `ana@x.es` → `ana`), con la regla
/// con la que `ore-iam` da los suyos ([`ore_core::pertenencia::handle_de`]):
/// letras, dígitos y guiones, así que como carpeta no sale de su sitio. X2
/// resuelve la tabla de quien pregunta con la misma.
pub(crate) fn handle(persona: &str) -> String {
    ore_core::pertenencia::handle_de(persona.rsplit(':').next().unwrap_or(persona))
        .unwrap_or_else(|| "persona".into())
}

/// Lo que dice la salida de una celda: su `tipo`, su `ms` y, si falló, su
/// `nombre` y su `mensaje`.
struct Salida {
    tipo: String,
    ms: i64,
    nombre: Option<String>,
    mensaje: Option<String>,
    saltada: bool,
    /// X3 · lo que el agente midió: CPU (ms), memoria pico y derrame (MB).
    consumo: Option<(i64, i64, i64)>,
}

fn salida_de(s: &Json) -> Salida {
    let n = ore_core::parse::parse(&s.jcs()).ok();
    let campo = |k: &str| {
        n.as_ref()
            .and_then(|n| n.get(k))
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    };
    Salida {
        tipo: campo("tipo").unwrap_or_default(),
        ms: campo("ms").and_then(|v| v.parse().ok()).unwrap_or(0),
        nombre: campo("nombre"),
        mensaje: campo("mensaje"),
        saltada: campo("saltada").as_deref() == Some("true"),
        consumo: n.as_ref().and_then(|n| n.get("consumo")).map(|(_, c)| {
            let num = |k: &str| {
                c.get(k)
                    .and_then(|(_, v)| v.as_str())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            };
            (num("cpu_ms"), num("memoria_pico_mb"), num("derrame_mb"))
        }),
    }
}

/// **El id de la ejecución de una celda**: `<job>/<n>`. El Job de Kubernetes lleva
/// un sufijo nuevo cada vez que el puesto se recrea, y el id del puesto no: con
/// `<puesto>/<n>` la hoja de las 12:10 y la de las 13:45 eran las dos `…/1`
/// (2026-10-24). Es el mismo que la escritura deja en `desde.ejecucion`, y por él
/// se enlaza una versión con su ejecución (X4). Sin Job (pruebas), el puesto.
pub(crate) fn id_de_la_celda(id: &str, p: &Puesto, n: u64) -> String {
    let base = if p.job.is_empty() { id } else { p.job.as_str() };
    format!("{base}/{n}")
}

/// **¿Terminó una ejecución con la salida de la celda `n`?** Si sí, cuál.
///
/// Una celda suelta es una ejecución; un guion (0039) lo es entero, y termina
/// con su última sentencia o con la que falla (las de detrás quedan
/// saltadas). Un Preview no entra todavía (DX2). `ahora` en ms de época: los
/// instantes de las celdas son relativos y se pasan a época desde él.
pub(crate) fn terminada(id: &str, p: &Puesto, n: u64, ahora: i64) -> Option<Ejecucion> {
    let c = p.celdas.get(&n)?;
    if c.ensayo.is_some() {
        return None;
    }
    let esta = salida_de(c.salida.as_ref()?);
    let (primera, total) = match c.lote {
        Some(l) if esta.tipo != "error" && l.i + 1 < l.n => return None,
        Some(l) => (l.primera, l.n as u64),
        None => (n, 1),
    };
    let celdas: Vec<(&crate::puestos::Celda, Salida)> = (primera..primera + total)
        .filter_map(|k| p.celdas.get(&k))
        .map(|c| (c, c.salida.as_ref().map(salida_de)))
        .map(|(c, s)| {
            (
                c,
                s.unwrap_or(Salida {
                    tipo: String::new(),
                    ms: 0,
                    nombre: None,
                    mensaje: None,
                    saltada: true,
                    consumo: None,
                }),
            )
        })
        .collect();
    let (primera_celda, _) = celdas.first()?;
    let epoca = |i: Option<std::time::Instant>| {
        i.map(|i| ahora - i.elapsed().as_millis() as i64)
            .unwrap_or(ahora)
    };
    let queued_at = epoca(Some(primera_celda.enviada));
    let started_at = epoca(primera_celda.empezada).max(queued_at);
    let fallo = celdas.iter().find(|(_, s)| s.tipo == "error");
    let kind = match &p.trabajo {
        Some(t) if t.build.is_some() => "build",
        Some(_) => "job",
        None if p.hoja_de_query => "query",
        None => "cell",
    };
    let e = crate::puestos::escritura_del_puesto(id, p);
    let medidas = celdas.iter().filter_map(|(_, s)| s.consumo);
    let steps = if celdas.len() > 1 {
        celdas
            .iter()
            .map(|(c, s)| Paso {
                statement: c.texto.clone(),
                status: if s.saltada {
                    "skipped"
                } else if s.tipo == "error" {
                    "failed"
                } else {
                    "ok"
                },
                duration_ms: s.ms,
                ..Paso::default()
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(Ejecucion {
        id: match kind {
            "build" | "job" => id.to_string(),
            _ => id_de_la_celda(id, p, primera),
        },
        kind,
        persona: p.persona.clone(),
        source_kind: e.desde.como_texto(),
        source_id: e.id,
        source_commit: e.commit,
        source_branch: p.rama.clone(),
        statement: celdas
            .iter()
            .map(|(c, _)| c.texto.trim())
            .collect::<Vec<_>>()
            .join("\n"),
        queued_at,
        started_at,
        ended_at: ahora.max(started_at),
        status: if fallo.is_some() { "failed" } else { "ok" },
        error_name: fallo.and_then(|(_, s)| s.nombre.clone()),
        error_message: fallo.and_then(|(_, s)| s.mensaje.clone()),
        compute: if p.entorno == crate::cola::SPARK {
            "G"
        } else {
            "L"
        },
        worker: id.to_string(),
        cpu_ms: medidas.clone().map(|(c, _, _)| c).reduce(|a, b| a + b),
        peak_memory_mb: medidas.clone().map(|(_, m, _)| m).max(),
        spill_mb: medidas.map(|(_, _, d)| d).max(),
        read: p
            .leidas_ahora
            .iter()
            .map(|(n, v)| (n.clone(), *v))
            .collect(),
        steps,
        growth: p.elastica.crecido_ahora.clone(),
        memory_final_mb: p
            .elastica
            .en_el_pool
            .then(|| crate::elastica::escalon(p.elastica.escalon).memoria_tope_mb as i64),
        growth_refused: p.elastica.negado_ahora.clone(),
        log: None,
    })
}

/// **La cola de filas por escribir**, de `(tabla, fila)`. Apagada (sin
/// `--executions`) no guarda nada.
#[derive(Default)]
pub(crate) struct Registro {
    encendido: AtomicBool,
    cola: Mutex<Vec<(String, String)>>,
    campana: Condvar,
    /// X5 · Lo que `ore-serve` encoló y nadie ha cerrado todavía (§8.12).
    abiertas: Mutex<BTreeMap<String, Abierta>>,
    /// Y lo último que se cerró: cerrar dos veces no escribe dos filas.
    cerradas: Mutex<std::collections::VecDeque<String>>,
}

impl std::fmt::Debug for Registro {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registro")
            .field("encendido", &self.encendido.load(Ordering::Relaxed))
            .finish()
    }
}

impl Registro {
    pub(crate) fn encender(&self) {
        self.encendido.store(true, Ordering::Relaxed);
    }

    pub(crate) fn encendido(&self) -> bool {
        self.encendido.load(Ordering::Relaxed)
    }

    /// Deja la fila de `e` para sus dos tablas. No espera a nada.
    pub(crate) fn apuntar(&self, e: &Ejecucion, organizacion: Option<&str>) {
        if !self.encendido() {
            return;
        }
        let fila = e.fila(organizacion);
        let mut cola = self.cola.lock().unwrap();
        for d in destinos(&e.persona) {
            cola.push((d, fila.clone()));
        }
        if cola.len() >= LOTE {
            self.campana.notify_one();
        }
    }

    /// X5 · **Abrir** una ingesta que se acaba de encolar. La misma, otra vez
    /// (un `ya encolado`), se queda como estaba.
    pub(crate) fn abrir(&self, a: Abierta) {
        if !self.encendido() {
            return;
        }
        self.abiertas
            .lock()
            .unwrap()
            .entry(a.id.clone())
            .or_insert(a);
    }

    /// X5 · **Cerrar** `id` con lo que dice quien la ejecutó, y dejar su fila.
    /// Una que no está abierta pero tiene nombre de Job de ingesta (la abrió
    /// otro `ore-serve`, o éste antes de reiniciarse: DX13) se cierra igual, a
    /// nombre de quien cierra.
    pub(crate) fn cerrar(&self, id: &str, c: Cierre, quien: &str, ahora: i64) -> Cerrado {
        if !self.encendido() {
            return Cerrado::Apagado;
        }
        let a = self.abiertas.lock().unwrap().remove(id);
        let a = match a {
            Some(a) => a,
            None if self.cerradas.lock().unwrap().iter().any(|x| x == id) => {
                return Cerrado::YaCerrada;
            }
            None => match Abierta::de_un_job(id, quien, ahora, c.started_at) {
                Some(a) => a,
                None => return Cerrado::Desconocida,
            },
        };
        self.recordar_cerrada(id);
        let organizacion = a.organizacion.clone();
        self.apuntar(&a.cerrada(c, ahora), organizacion.as_deref());
        Cerrado::Escrita
    }

    fn recordar_cerrada(&self, id: &str) {
        let mut cerradas = self.cerradas.lock().unwrap();
        cerradas.push_back(id.to_string());
        while cerradas.len() > CERRADAS_RECORDADAS {
            cerradas.pop_front();
        }
    }

    /// X5 · **El barrido**: lo que pasó su plazo sin que nadie lo cerrara queda
    /// `lost`, con su fila. Nunca `ok` por omisión.
    pub(crate) fn barrer(&self, ahora: i64) -> usize {
        let vencidas: Vec<Abierta> = {
            let mut abiertas = self.abiertas.lock().unwrap();
            let ids: Vec<String> = abiertas
                .iter()
                .filter(|(_, a)| ahora >= a.queued_at + a.plazo_ms)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| abiertas.remove(id)).collect()
        };
        for a in &vencidas {
            eprintln!(
                "executions · `{}` no se cerró en {} s: lost",
                a.id,
                a.plazo_ms / 1000
            );
            self.recordar_cerrada(&a.id);
            self.apuntar(&a.perdida(ahora), a.organizacion.as_deref());
        }
        vencidas.len()
    }

    /// Lo que haya, tras esperar como mucho `cada` (o hasta juntar un lote).
    fn tomar(&self, cada: Duration) -> Vec<(String, String)> {
        let cola = self.cola.lock().unwrap();
        let (mut cola, _) = self
            .campana
            .wait_timeout_while(cola, cada, |c| c.len() < LOTE)
            .unwrap();
        std::mem::take(&mut *cola)
    }

    /// Lo que no entró vuelve delante, y si el lago lleva mucho sin contestar
    /// se pierde lo más viejo.
    fn devolver(&self, mut no_entraron: Vec<(String, String)>) {
        if no_entraron.is_empty() {
            return;
        }
        let mut cola = self.cola.lock().unwrap();
        no_entraron.append(&mut cola);
        if no_entraron.len() > ESPERAN_COMO_MUCHO {
            let sobran = no_entraron.len() - ESPERAN_COMO_MUCHO;
            eprintln!(
                "executions · el lago no contesta: se pierden {sobran} filas, las más viejas"
            );
            no_entraron.drain(..sobran);
        }
        *cola = no_entraron;
    }
}

/// Cuántos cierres se recuerdan para no escribir dos veces el mismo.
const CERRADAS_RECORDADAS: usize = 2_000;

/// Lo que contesta [`Registro::cerrar`].
#[derive(Debug, PartialEq)]
pub(crate) enum Cerrado {
    Escrita,
    YaCerrada,
    Desconocida,
    Apagado,
}

/// X5 · **Una ingesta encolada que nadie ha cerrado** (§8.12): lo que
/// `ore-serve` sabe al encolarla. Su fila nace al cerrarla.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Abierta {
    /// El nombre del Job (DX8), o `copiador/<…>` en el copiador residente.
    pub id: String,
    /// `copy`, `catalog`, `invocation` o `refresh` (DX7).
    pub kind: &'static str,
    /// Qué la pidió: `ingest`, `copy`, `recopy`, `schedule`, `source`, `function`.
    pub source_kind: &'static str,
    /// Quien la pidió (DX12).
    pub persona: String,
    /// Lo que se copia, cataloga o invoca: va a `statement`.
    pub que: String,
    pub source_id: Option<String>,
    pub source_branch: Option<String>,
    /// El commit de la cola que la encoló.
    pub source_commit: Option<String>,
    pub queued_at: i64,
    /// DX9 · cuánto se espera su cierre desde `queued_at`.
    pub plazo_ms: i64,
    /// `job:<nombre>` o `ore-copiador`.
    pub worker: String,
    pub organizacion: Option<String>,
}

/// DX9 · El plazo de cada plantilla: su `activeDeadlineSeconds` y 10 min de Flux.
pub(crate) const PLAZO_DE_FLUX_MS: i64 = 10 * 60 * 1000;
pub(crate) const PLAZO_COPIA_MS: i64 = 3600 * 1000 + PLAZO_DE_FLUX_MS;
pub(crate) const PLAZO_CATALOGO_MS: i64 = 600 * 1000 + PLAZO_DE_FLUX_MS;
pub(crate) const PLAZO_INVOCACION_MS: i64 = 3600 * 1000 + PLAZO_DE_FLUX_MS;
pub(crate) const PLAZO_COPIADOR_MS: i64 = 3600 * 1000;

/// X5 · **Lo que dice quien la ejecutó**, al cerrar (`POST /ejecuciones/{id}/cierre`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Cierre {
    /// `ok` o `failed`.
    pub status: &'static str,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub error_name: Option<String>,
    pub error_message: Option<String>,
    /// El commit que publicó, si publicó.
    pub commit: Option<String>,
    pub log: Option<String>,
    pub steps: Vec<Paso>,
}

impl Cierre {
    /// El cuerpo del `POST`: `{status, started_at, ended_at, error_name,
    /// error_message, commit, log, steps: [{name, status, rows, bytes,
    /// duration_ms, error}]}`, con los instantes en milisegundos de época.
    pub(crate) fn del_cuerpo(cuerpo: &str) -> Result<Cierre, String> {
        let n = ore_core::parse::parse(cuerpo).map_err(|_| "the body is not JSON".to_string())?;
        fn texto(n: &ore_core::parse::Node, k: &str) -> Option<String> {
            n.get(k)
                .and_then(|(_, v)| v.as_str())
                .filter(|v| !v.trim().is_empty() && *v != "null")
                .map(String::from)
        }
        fn entero(n: &ore_core::parse::Node, k: &str) -> Option<i64> {
            texto(n, k).and_then(|v| v.trim().parse().ok())
        }
        let status = match texto(&n, "status").as_deref() {
            Some("ok") => "ok",
            Some("failed") => "failed",
            otro => return Err(format!("`status` is `ok` or `failed`, not {otro:?}")),
        };
        let mut steps = Vec::new();
        if let Some((_, l)) = n.get("steps") {
            for p in l.items() {
                steps.push(Paso {
                    statement: texto(p, "name").ok_or("a step without `name`")?,
                    status: match texto(p, "status").as_deref() {
                        Some("ok") => "ok",
                        Some("skipped") => "skipped",
                        _ => "failed",
                    },
                    duration_ms: entero(p, "duration_ms").unwrap_or(0),
                    rows: entero(p, "rows"),
                    bytes: entero(p, "bytes"),
                    error: texto(p, "error"),
                });
            }
        }
        Ok(Cierre {
            status,
            started_at: entero(&n, "started_at"),
            ended_at: entero(&n, "ended_at"),
            error_name: texto(&n, "error_name"),
            error_message: texto(&n, "error_message"),
            commit: texto(&n, "commit"),
            log: texto(&n, "log"),
            steps,
        })
    }
}

impl Abierta {
    /// Una que se acaba de encolar, sin rama ni commit todavía.
    pub(crate) fn nueva(
        id: impl Into<String>,
        kind: &'static str,
        source_kind: &'static str,
        persona: &str,
        que: impl Into<String>,
        plazo_ms: i64,
    ) -> Abierta {
        let id = id.into();
        Abierta {
            worker: format!("job:{id}"),
            id,
            kind,
            source_kind,
            persona: persona.to_string(),
            que: que.into(),
            source_id: None,
            source_branch: None,
            source_commit: None,
            queued_at: ahora_ms(),
            plazo_ms,
            organizacion: None,
        }
    }

    /// DX13 · La que no estaba abierta: por el prefijo del nombre de su Job se
    /// sabe qué es; quién la pidió no, y va a nombre de quien la cierra.
    fn de_un_job(id: &str, quien: &str, ahora: i64, empezo: Option<i64>) -> Option<Abierta> {
        let (kind, source_kind, plazo_ms) = if id.starts_with("copiar-prog-") {
            ("refresh", "schedule", PLAZO_COPIA_MS)
        } else if id.starts_with("copiar-rehacer-") {
            ("copy", "recopy", PLAZO_COPIA_MS)
        } else if id.starts_with("copiar-") {
            ("copy", "copy", PLAZO_COPIA_MS)
        } else if id.starts_with("catalogo-") {
            ("catalog", "source", PLAZO_CATALOGO_MS)
        } else if id.starts_with("invocar-") {
            ("invocation", "function", PLAZO_INVOCACION_MS)
        } else {
            return None;
        };
        Some(Abierta {
            id: id.to_string(),
            kind,
            source_kind,
            persona: quien.to_string(),
            que: id.to_string(),
            source_id: None,
            source_branch: None,
            source_commit: None,
            queued_at: empezo.unwrap_or(ahora),
            plazo_ms,
            worker: format!("job:{id}"),
            organizacion: None,
        })
    }

    fn fila(&self, status: &'static str, ahora: i64) -> Ejecucion {
        Ejecucion {
            id: self.id.clone(),
            kind: self.kind,
            persona: self.persona.clone(),
            source_kind: self.source_kind,
            source_id: self.source_id.clone(),
            source_commit: self.source_commit.clone(),
            source_branch: self.source_branch.clone(),
            statement: self.que.clone(),
            queued_at: self.queued_at,
            started_at: self.queued_at,
            ended_at: ahora,
            status,
            error_name: None,
            error_message: None,
            compute: "driver",
            worker: self.worker.clone(),
            cpu_ms: None,
            peak_memory_mb: None,
            spill_mb: None,
            read: Vec::new(),
            steps: Vec::new(),
            growth: Vec::new(),
            memory_final_mb: None,
            growth_refused: None,
            log: None,
        }
    }

    /// Su fila, con lo que dijo quien la ejecutó.
    pub(crate) fn cerrada(&self, c: Cierre, ahora: i64) -> Ejecucion {
        let mut e = self.fila(c.status, c.ended_at.unwrap_or(ahora));
        e.started_at = c.started_at.unwrap_or(self.queued_at).max(self.queued_at);
        e.error_name = c.error_name;
        e.error_message = c.error_message;
        e.source_commit = c.commit.or(e.source_commit);
        e.log = c.log;
        e.steps = c.steps;
        e
    }

    /// Su fila cuando nadie la cerró a tiempo (§8.12 ③).
    pub(crate) fn perdida(&self, ahora: i64) -> Ejecucion {
        let mut e = self.fila("lost", ahora);
        e.error_name = Some("NotReported".into());
        e.error_message = Some(format!(
            "the job did not report within {} min: it ran past its deadline or its pod died",
            self.plazo_ms / 60_000
        ));
        e
    }

    /// Su fila cuando ni siquiera se pudo encolar (§8.12 ①).
    pub(crate) fn no_encolada(&self, motivo: &str, ahora: i64) -> Ejecucion {
        let mut e = self.fila("failed", ahora);
        e.error_name = Some("NotQueued".into());
        e.error_message = Some(motivo.to_string());
        e
    }
}

/// El commit que `forja.publicar` dejó, de lo que contesta un encolar
/// (`… · commit <c>`).
fn commit_de(dicho: &str) -> Option<String> {
    dicho
        .rsplit_once("commit ")
        .map(|(_, c)| c.trim().to_string())
        .filter(|c| !c.is_empty())
}

impl crate::rutas::Servidor {
    /// X5 · **Tras encolar una ingesta** (§8.12 ①), con lo que contestó el
    /// encolar: `encolado …` / `copiando ya …` la abre; `NO …` deja ya su fila
    /// `failed` (se pidió y no pudo ni empezar); `ya encolado …` y lo demás no
    /// hace nada, porque es la misma que ya está abierta.
    pub(crate) fn tras_encolar(&self, mut a: Abierta, dicho: &str) {
        let r = &self.puestos.ejecuciones;
        a.organizacion = self.organizacion.clone();
        if dicho.starts_with("NO ") {
            r.apuntar(
                &a.no_encolada(dicho, ahora_ms()),
                self.organizacion.as_deref(),
            );
        } else if dicho.starts_with("encolado") || dicho.starts_with("copiando ya") {
            a.source_commit = commit_de(dicho);
            r.abrir(a);
        }
    }

    /// X5 · **`POST /ejecuciones/{id}/cierre`** (§8.12 ②): lo cierra quien la
    /// ejecutó —el Job, el copiador—, que es un agente de la celda (DX14).
    pub(crate) fn cerrar_ejecucion(
        &self,
        sujeto: &ore_entrada::identidad::Identidad,
        id: &str,
        cuerpo: &str,
    ) -> ore_entrada::http::Respuesta {
        use ore_entrada::http::Respuesta;
        if !crate::puestos::es_agente(sujeto) {
            return Respuesta::error(
                403,
                "only the cell's agent closes an execution: the job or the copier that ran it",
            );
        }
        if id.is_empty()
            || id.len() > 200
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Respuesta::error(400, format!("`{id}` is not an execution id"));
        }
        let c = match Cierre::del_cuerpo(cuerpo) {
            Ok(c) => c,
            Err(m) => return Respuesta::error(400, m),
        };
        let dicho = match self
            .puestos
            .ejecuciones
            .cerrar(id, c, &sujeto.persona, ahora_ms())
        {
            Cerrado::Escrita => None,
            Cerrado::YaCerrada => Some("already closed: its row is written"),
            Cerrado::Apagado => Some("executions is off on this server"),
            Cerrado::Desconocida => {
                return Respuesta::error(404, format!("no execution `{id}` is open"));
            }
        };
        let mut m = vec![
            ("id", Json::s(id)),
            ("written", Json::Bool(dicho.is_none())),
        ];
        if let Some(d) = dicho {
            m.push(("reason", Json::s(d)));
        }
        Respuesta::ok(Json::obj(m))
    }
}

/// X5 · **El nombre del Job** de un manifiesto rendido para la cola: el
/// `metadata.name` del primer `kind: Job`.
pub(crate) fn nombre_del_job(texto: &str) -> Option<String> {
    let mut en_job = false;
    for l in texto.lines() {
        if l.starts_with("kind:") {
            en_job = l.trim() == "kind: Job";
        } else if en_job && let Some(n) = l.strip_prefix("  name: ") {
            return Some(n.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// **Un lote**: las filas de cada tabla, en una sola llamada por tabla. Las
/// de la que falla vuelven; las demás ya están.
pub(crate) fn escribir_lote(
    lote: Vec<(String, String)>,
    anexar: &impl Fn(&str, &[String]) -> Result<(), String>,
) -> Vec<(String, String)> {
    let mut por_tabla: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (t, f) in lote {
        por_tabla.entry(t).or_default().push(f);
    }
    let mut no_entraron = Vec::new();
    for (t, filas) in por_tabla {
        if let Err(e) = anexar(&t, &filas) {
            eprintln!("executions · `{t}`: {} filas esperan: {e}", filas.len());
            no_entraron.extend(filas.into_iter().map(|f| (t.clone(), f)));
        }
    }
    no_entraron
}

/// El hilo que escribe: para siempre, cada [`CADA`].
pub(crate) fn escribir_siempre(
    r: &Registro,
    anexar: impl Fn(&str, &[String]) -> Result<(), String>,
) {
    loop {
        r.barrer(ahora_ms());
        let lote = r.tomar(CADA);
        if !lote.is_empty() {
            r.devolver(escribir_lote(lote, &anexar));
        }
    }
}

/// La cabecera de `ore-store anexar` para la tabla `dataset`.
fn cabecera(dataset: &str) -> String {
    Json::obj([
        ("dataset", Json::s(dataset)),
        ("plan", Json::s(NOMBRE)),
        ("conducto", Json::s("system")),
        (
            "esquema",
            Json::Obj(
                ore_core::sistema::COLUMNAS_DE_EJECUCIONES
                    .iter()
                    .map(|(c, t, _)| (c.to_string(), Json::s(*t)))
                    .collect(),
            ),
        ),
    ])
    .jcs()
}

/// `ore-store <verbo>`: la petición en la primera línea, lo demás detrás.
fn almacen(ore: &Path, verbo: &str, entrada: &str) -> Result<String, String> {
    let programa = crate::preview::programa_del_almacen(ore);
    let mut hijo = Command::new(&programa)
        .arg(verbo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("no se pudo ejecutar `{}`: {e}", programa.display()))?;
    {
        let mut e = hijo.stdin.take().ok_or("no se pudo escribir en el hijo")?;
        e.write_all(entrada.as_bytes())
            .and_then(|_| e.write_all(b"\n"))
            .map_err(|e| format!("no se pudo escribir en `{}`: {e}", programa.display()))?;
    }
    let s = hijo
        .wait_with_output()
        .map_err(|e| format!("`{}` no terminó: {e}", programa.display()))?;
    if !s.status.success() {
        let e = String::from_utf8_lossy(&s.stderr);
        return Err(e
            .lines()
            .find_map(|l| l.strip_prefix("error: "))
            .map(String::from)
            .unwrap_or_else(|| format!("`ore-store {verbo}` falló")));
    }
    Ok(String::from_utf8_lossy(&s.stdout).trim().to_string())
}

/// `ore-store anexar`: la cabecera y, detrás, una fila por línea. Sin filas,
/// la tabla nace vacía (lo que X2 hace la primera vez que alguien la lee).
pub(crate) fn anexar_con(ore: &Path, dataset: &str, filas: &[String]) -> Result<(), String> {
    let mut texto = cabecera(dataset);
    for f in filas {
        texto.push('\n');
        texto.push_str(f);
    }
    almacen(ore, "anexar", &texto).map(|_| ())
}

/// Un campo de texto de la línea JSON que contesta `ore-store`.
fn campo_de(linea: &str, k: &str) -> Option<String> {
    ore_core::parse::parse(linea)
        .ok()?
        .get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
        .map(String::from)
}

/// X2b · **El `metadata.json` vigente de la tabla `dataset`**, y si aún no
/// existe, la crea vacía (`ore-store anexar` sin filas): existe desde el
/// principio. Idempotente.
pub(crate) fn asegurar(ore: &Path, dataset: &str) -> Result<String, String> {
    let peticion = Json::obj([("dataset", Json::s(dataset))]).jcs();
    if let Some(ml) = campo_de(&almacen(ore, "puntero", &peticion)?, "metadata_location") {
        return Ok(ml);
    }
    anexar_con(ore, dataset, &[])?;
    campo_de(&almacen(ore, "puntero", &peticion)?, "metadata_location")
        .ok_or_else(|| format!("`{dataset}` no tiene puntero"))
}

/// X2b · Al arrancar con `--executions`: la tabla de la organización existe,
/// vacía si nadie ha ejecutado nada (en una celda nueva, justo tras la
/// semilla). Si el lago aún no contesta, se reintenta, cada vez más espaciado.
/// Dice si quedó.
pub(crate) fn asegurar_al_arrancar(ore: &Path) -> bool {
    let [_, org] = destinos("");
    let mut espera = Duration::from_secs(5);
    for _ in 0..12 {
        match asegurar(ore, &org) {
            Ok(_) => {
                eprintln!("executions · `{org}` está");
                return true;
            }
            Err(e) => {
                eprintln!("executions · `{org}` aún no ({e}): otra vez en {espera:?}");
                std::thread::sleep(espera);
                espera = (espera * 2).min(Duration::from_secs(300));
            }
        }
    }
    eprintln!("executions · `{org}` no se pudo crear: nacerá con la primera fila");
    false
}

/// Elastic Sessions S4a·3 · **Al arrancar, lo recordado**: la tabla de la
/// organización, sólo con las columnas que la previsión necesita
/// (`ore-store leer` con `columnas`), y de ella lo de los últimos
/// [`crate::prevision::DIAS`] días.
pub(crate) fn calentar_la_prevision(ore: &Path, prevision: &crate::prevision::Prevision) {
    let [_, org] = destinos("");
    let leido = asegurar(ore, &org).and_then(|ml| {
        almacen(
            ore,
            "leer",
            &Json::obj([
                ("metadata_location", Json::s(ml)),
                ("dataset", Json::s(&org)),
                ("columnas", Json::s(crate::prevision::COLUMNAS)),
            ])
            .jcs(),
        )
    });
    match leido {
        Ok(lineas) => {
            let desde = ahora_ms() / 1000 - crate::prevision::DIAS * 86_400;
            let dia = ore_core::frescura::iso(desde);
            let n = crate::prevision::calentar(prevision, &lineas, dia.get(..10).unwrap_or(""));
            eprintln!(
                "executions · la previsión recuerda {n} ejecuciones de los últimos {} días",
                crate::prevision::DIAS
            );
        }
        Err(e) => eprintln!("executions · la previsión empieza sin recuerdos ({e})"),
    }
}

/// X2b · **El Preview de la System table** para quien mira desde la consola:
/// una página de su tabla (o de la de todos, si es admin), por `ore-store
/// muestra`, con la forma del Preview de un dataset.
pub(crate) fn muestra(
    ore: &Path,
    persona: &str,
    todas: bool,
    desde: u64,
    limite: u64,
    snapshot: Option<&str>,
) -> Result<Json, String> {
    let dataset = tabla_de(persona, todas);
    let ml = asegurar(ore, &dataset)?;
    let mut pet = vec![
        ("metadata_location", Json::s(ml)),
        ("dataset", Json::s(&dataset)),
        ("desde", Json::s(desde.to_string())),
        ("limite", Json::s(limite.to_string())),
    ];
    if let Some(s) = snapshot {
        pet.push(("snapshot", Json::s(s)));
    }
    let linea = almacen(ore, "muestra", &Json::obj(pet).jcs())?;
    let n = ore_core::parse::parse(&linea)
        .map_err(|_| "`ore-store muestra` no devolvió JSON".to_string())?;
    let mut j = Json::de_node(&n);
    if let Json::Obj(m) = &mut j {
        m.insert("origen".into(), Json::s("lago"));
        m.insert(
            "alcance".into(),
            Json::s(if todas { "organization" } else { "own" }),
        );
    }
    Ok(j)
}

/// X2 · **Qué tabla lee quien pregunta**: la suya o, si puede leer las de
/// todos (`ejecuciones:leer-todas`, ORGADMIN), la de la organización.
pub(crate) fn tabla_de(persona: &str, todas: bool) -> String {
    let [suya, org] = destinos(persona);
    if todas { org } else { suya }
}

/// X2 · **`datos` de `system.default.executions`** para quien pregunta: el
/// `metadata.json` vigente de su tabla y la credencial que sólo lee, acotada
/// a ella (`ore-store prestar`, `modo: leer`). La forma es la de un dataset del
/// árbol (`puestos::datos_de`), para que el SDK la lea igual. La primera vez,
/// si nadie ha ejecutado nada todavía, la tabla nace vacía: existe desde el
/// primer momento.
pub(crate) fn datos(ore: &Path, persona: &str, todas: bool) -> Result<Json, String> {
    let dataset = tabla_de(persona, todas);
    let ml = asegurar(ore, &dataset)?;
    let prestamo = almacen(
        ore,
        "prestar",
        &Json::obj([("dataset", Json::s(&dataset)), ("modo", Json::s("leer"))]).jcs(),
    )?;
    let credencial = ore_core::parse::parse(&prestamo)
        .ok()
        .and_then(|n| n.get("config").map(|(_, c)| Json::de_node(c)))
        .unwrap_or_else(|| Json::obj([]));
    Ok(Json::obj([
        ("vista", Json::s(ore_core::sistema::EJECUCIONES)),
        ("dataset", Json::s(&dataset)),
        ("estado", Json::s("ok")),
        ("clave", Json::s("")),
        ("metadata_location", Json::s(ml)),
        ("snapshot", Json::s("")),
        ("plan", Json::s(NOMBRE)),
        ("filas", Json::s("")),
        ("clasificacion", Json::obj([])),
        ("media", Json::obj([])),
        ("geo", Json::obj([])),
        (
            "bucket",
            Json::s(std::env::var("ORE_GCS_BUCKET").unwrap_or_default()),
        ),
        (
            "almacen",
            Json::s(std::env::var("ORE_STORE").unwrap_or_default()),
        ),
        ("credencial", credencial),
    ]))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    // ── X5 · las ingestas (docs/ejecuciones.md §8.12) ──

    fn filas(r: &Registro) -> Vec<ore_core::parse::Node> {
        std::mem::take(&mut *r.cola.lock().unwrap())
            .into_iter()
            .filter(|(t, _)| t.ends_with("/org"))
            .map(|(_, f)| ore_core::parse::parse(&f).unwrap())
            .collect()
    }

    fn campo(f: &ore_core::parse::Node, k: &str) -> Option<String> {
        f.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
    }

    #[test]
    fn una_copia_se_abre_y_su_cierre_deja_una_fila_con_pasos_y_log() {
        let r = Registro::default();
        r.encender();
        let mut a = Abierta::nueva(
            "copiar-prog-abc",
            "refresh",
            "schedule",
            "ana",
            "olist.customers, olist.orders",
            PLAZO_COPIA_MS,
        );
        a.queued_at = 1_000;
        r.abrir(a);
        let c = Cierre::del_cuerpo(
            r#"{"status":"failed","started_at":2000,"ended_at":9000,"commit":"abc123",
                "error_message":"olist.orders: the source said 401","log":"copiando\nolist.orders ✗ 401",
                "steps":[{"name":"olist.customers","status":"ok","rows":99441,"bytes":5000,"duration_ms":3000},
                         {"name":"olist.orders","status":"failed","error":"401"}]}"#,
        )
        .unwrap();
        assert_eq!(
            r.cerrar("copiar-prog-abc", c.clone(), "agente", 10_000),
            Cerrado::Escrita
        );
        // cerrar dos veces no escribe dos filas
        assert_eq!(
            r.cerrar("copiar-prog-abc", c, "agente", 11_000),
            Cerrado::YaCerrada
        );
        let f = filas(&r);
        assert_eq!(f.len(), 1);
        let f = &f[0];
        assert_eq!(campo(f, "kind").as_deref(), Some("refresh"));
        assert_eq!(campo(f, "source_kind").as_deref(), Some("schedule"));
        assert_eq!(campo(f, "status").as_deref(), Some("failed"));
        assert_eq!(campo(f, "subject").as_deref(), Some("user:ana"));
        assert_eq!(campo(f, "source_commit").as_deref(), Some("abc123"));
        assert_eq!(campo(f, "wait_ms").as_deref(), Some("1000"));
        assert_eq!(campo(f, "duration_ms").as_deref(), Some("7000"));
        assert!(campo(f, "log").unwrap().ends_with("401"));
        let pasos = campo(f, "steps").unwrap();
        assert!(pasos.contains(r#""rows":99441"#), "{pasos}");
        assert!(pasos.contains(r#""error":"401""#), "{pasos}");
    }

    #[test]
    fn lo_que_nadie_cierra_queda_lost_y_lo_que_no_se_encola_failed() {
        let r = Registro::default();
        r.encender();
        let mut a = Abierta::nueva("copiar-x", "copy", "ingest", "ana", "v", 60_000);
        a.queued_at = 0;
        r.abrir(a.clone());
        assert_eq!(r.barrer(59_999), 0);
        assert_eq!(r.barrer(60_000), 1);
        // su cierre tardío no escribe otra
        let ok = Cierre {
            status: "ok",
            ..Default::default()
        };
        assert_eq!(
            r.cerrar("copiar-x", ok, "agente", 70_000),
            Cerrado::YaCerrada
        );
        let f = filas(&r);
        assert_eq!(f.len(), 1);
        assert_eq!(campo(&f[0], "status").as_deref(), Some("lost"));
        assert_eq!(campo(&f[0], "error_name").as_deref(), Some("NotReported"));
        let e = a.no_encolada("NO encolado: la cola no trae `plantilla-copia.txt`", 5);
        assert_eq!(
            (e.status, e.error_name.as_deref()),
            ("failed", Some("NotQueued"))
        );
    }

    #[test]
    fn un_cierre_de_un_job_que_no_se_abrio_aqui_se_reconoce_por_su_nombre() {
        let r = Registro::default();
        r.encender();
        let ok = Cierre {
            status: "ok",
            ..Default::default()
        };
        assert_eq!(
            r.cerrar("catalogo-ventas-1a2b", ok.clone(), "agente", 1),
            Cerrado::Escrita
        );
        assert_eq!(r.cerrar("otra-cosa", ok, "agente", 1), Cerrado::Desconocida);
        let f = filas(&r);
        assert_eq!(campo(&f[0], "kind").as_deref(), Some("catalog"));
        assert_eq!(campo(&f[0], "subject").as_deref(), Some("user:agente"));
        // apagado (sin `--executions`) no abre ni escribe
        let apagado = Registro::default();
        apagado.abrir(Abierta::nueva("copiar-y", "copy", "copy", "ana", "v", 1));
        assert_eq!(
            apagado.cerrar(
                "copiar-y",
                Cierre {
                    status: "ok",
                    ..Default::default()
                },
                "agente",
                1
            ),
            Cerrado::Apagado
        );
    }

    #[test]
    fn el_cierre_se_lee_del_cuerpo_y_lo_raro_se_dice() {
        assert!(Cierre::del_cuerpo(r#"{"status":"done"}"#).is_err());
        assert!(Cierre::del_cuerpo(r#"{"status":"ok","steps":[{"status":"ok"}]}"#).is_err());
        let c = Cierre::del_cuerpo(r#"{"status":"ok","started_at":null}"#).unwrap();
        assert_eq!((c.status, c.started_at), ("ok", None));
    }

    #[test]
    fn el_log_guarda_su_final_y_el_job_su_nombre() {
        let largo = format!("{}fin", "x".repeat(TOPE_DE_LOG));
        assert!(recortar_el_final(&largo, TOPE_DE_LOG).ends_with("fin"));
        assert_eq!(recortar_el_final(&largo, TOPE_DE_LOG).len(), TOPE_DE_LOG);
        assert_eq!(recortar_el_final("ñandú", 3), "dú");
        let texto = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: otra\n---\napiVersion: batch/v1\nkind: Job\nmetadata:\n  name: copiar-prog-1f2e\n";
        assert_eq!(nombre_del_job(texto).as_deref(), Some("copiar-prog-1f2e"));
        assert_eq!(nombre_del_job("kind: ConfigMap\n  name: x\n"), None);
        assert_eq!(
            commit_de("encolado como `48-a.yaml` · commit 9f3e"),
            Some("9f3e".into())
        );
    }

    #[test]
    fn la_huella_no_mira_mayusculas_ni_espacios() {
        assert_eq!(huella("select 1 from t;"), huella("SELECT  1\n  FROM t"));
        assert_ne!(huella("select 1"), huella("select 2"));
    }

    #[test]
    fn la_sentencia_se_recorta_sin_partir_un_caracter() {
        assert_eq!(recortar("año", 2), "a");
        assert_eq!(recortar("año", 3), "añ");
        assert_eq!(recortar("año", 9), "año");
    }

    #[test]
    fn el_instante_lleva_sus_milisegundos() {
        assert_eq!(instante(1_761_300_000_123), "2025-10-24T10:00:00.123Z");
    }

    #[test]
    fn el_admin_lee_la_de_la_organizacion_y_los_demas_la_suya() {
        assert_eq!(
            tabla_de("persona:ana", false),
            "system/executions/users/ana"
        );
        assert_eq!(tabla_de("persona:ana", true), "system/executions/org");
    }

    #[test]
    fn cada_persona_tiene_su_tabla_y_la_organizacion_la_suya() {
        assert_eq!(
            destinos("ana"),
            [
                "system/executions/users/ana".to_string(),
                "system/executions/org".to_string()
            ]
        );
        // nada sale de su carpeta
        assert_eq!(destinos("persona:../Bea")[0], "system/executions/users/bea");
        assert_eq!(destinos("ana.g@x.es")[0], "system/executions/users/ana-g");
    }

    #[test]
    fn un_lote_va_por_tablas_y_lo_que_falla_vuelve() {
        let r = Registro::default();
        let e = Ejecucion {
            id: "puesto-ana-python/3".into(),
            kind: "query",
            persona: "ana".into(),
            source_kind: "query",
            source_id: None,
            source_commit: None,
            source_branch: Some("ana/puesto".into()),
            statement: "select 1".into(),
            queued_at: 1_000,
            started_at: 1_500,
            ended_at: 4_000,
            status: "ok",
            error_name: None,
            error_message: None,
            compute: "L",
            worker: "puesto-ana-python".into(),
            cpu_ms: None,
            peak_memory_mb: None,
            spill_mb: None,
            read: vec![("ventas.pedidos".into(), false)],
            steps: vec![],
            growth: vec![],
            memory_final_mb: None,
            growth_refused: None,
            log: None,
        };
        // apagado no guarda nada
        r.apuntar(&e, Some("victor"));
        assert!(r.cola.lock().unwrap().is_empty());
        r.encender();
        r.apuntar(&e, Some("victor"));
        r.apuntar(
            &Ejecucion {
                persona: "bea".into(),
                ..e.clone()
            },
            Some("victor"),
        );
        let lote = std::mem::take(&mut *r.cola.lock().unwrap());
        assert_eq!(lote.len(), 4);
        let llamadas = Mutex::new(Vec::new());
        let no = escribir_lote(lote, &|t: &str, filas: &[String]| {
            llamadas.lock().unwrap().push((t.to_string(), filas.len()));
            if t.ends_with("/bea") {
                Err("el lago no contesta".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(
            *llamadas.lock().unwrap(),
            [
                ("system/executions/org".to_string(), 2),
                ("system/executions/users/ana".to_string(), 1),
                ("system/executions/users/bea".to_string(), 1),
            ]
        );
        // sólo vuelve la de bea, a su tabla: la de la organización ya entró
        assert_eq!(no.len(), 1);
        assert_eq!(no[0].0, "system/executions/users/bea");
        r.devolver(no);
        assert_eq!(r.cola.lock().unwrap().len(), 1);
    }

    /// 2026-10-24: la hoja de un puesto recreado repetía `…/1`. El id va por el
    /// Job (otro sufijo en cada vida del puesto), y la escritura lleva el mismo.
    #[test]
    fn el_id_no_se_repite_al_recrear_el_puesto_y_es_el_de_la_escritura() {
        use crate::puestos::Celda;
        use std::time::Instant;
        let vida = |job: &str| {
            let mut p = crate::puestos::prueba::un_puesto("agente:ana");
            p.job = job.into();
            p.hoja_de_query = true;
            p.celdas.insert(
                1,
                Celda {
                    texto: "create or replace dataset v.a as select 1".into(),
                    lenguaje: "sql".into(),
                    corre: None,
                    avisos: Vec::new(),
                    enviada: Instant::now(),
                    empezada: Some(Instant::now()),
                    salida: None,
                    lote: None,
                    ensayo: None,
                },
            );
            let escritura = crate::puestos::escritura_del_puesto("puesto-ana-python", &p);
            p.celdas.get_mut(&1).unwrap().salida =
                Some(Json::Crudo(r#"{"tipo":"vacia","ms":1}"#.into()));
            let fila = terminada("puesto-ana-python", &p, 1, 10_000).unwrap();
            (fila.id, escritura.ejecucion)
        };
        let (a, ea) = vida("puesto-ana-python-caa05864");
        let (b, eb) = vida("puesto-ana-python-b7374b9d");
        assert_eq!(a, "puesto-ana-python-caa05864/1");
        assert_ne!(a, b);
        assert_eq!(ea.as_deref(), Some(a.as_str()));
        assert_eq!(eb.as_deref(), Some(b.as_str()));
    }

    /// Un guion de dos sentencias en la hoja (en G) es una ejecución: termina
    /// con la última, con sus dos pasos; si la primera falla, termina con ella
    /// y la de detrás queda saltada. Un Preview no entra.
    #[test]
    fn un_guion_es_una_ejecucion_con_sus_pasos() {
        use crate::puestos::{Celda, Lote};
        use std::time::Instant;
        let celda = |texto: &str, i: usize| Celda {
            texto: texto.into(),
            lenguaje: "sql".into(),
            corre: None,
            avisos: Vec::new(),
            enviada: Instant::now(),
            empezada: Some(Instant::now()),
            salida: None,
            lote: Some(Lote {
                primera: 4,
                i,
                n: 2,
            }),
            ensayo: None,
        };
        let salida = |s: &str| Some(Json::Crudo(s.into()));
        let mut g = crate::puestos::prueba::un_puesto("agente:ana");
        g.entorno = crate::cola::SPARK.into();
        g.hoja_de_query = true;
        g.celdas
            .insert(4, celda("create or replace dataset v.a as select 1", 0));
        g.celdas.insert(5, celda("select * from v.a", 1));
        g.leidas_ahora.insert("v.a".into(), false);
        g.celdas.get_mut(&4).unwrap().salida = salida(
            r#"{"tipo":"vacia","ms":7,"consumo":{"cpu_ms":300,"memoria_pico_mb":800,"derrame_mb":0}}"#,
        );
        assert_eq!(terminada("g", &g, 4, 10_000), None, "falta la segunda");
        g.celdas.get_mut(&5).unwrap().salida = salida(
            r#"{"tipo":"tabla","ms":3,"consumo":{"cpu_ms":200,"memoria_pico_mb":1200,"derrame_mb":40}}"#,
        );
        let e = terminada("g", &g, 5, 10_000).expect("termina con la última");
        assert_eq!(
            (e.id.as_str(), e.kind, e.compute, e.status),
            ("g/4", "query", "G", "ok")
        );
        assert_eq!(
            e.statement,
            "create or replace dataset v.a as select 1
select * from v.a"
        );
        assert_eq!(
            e.steps.iter().map(|p| p.status).collect::<Vec<_>>(),
            ["ok", "ok"]
        );
        assert_eq!(e.read, [("v.a".to_string(), false)]);
        // X3: la CPU se suma; la memoria y el derrame, lo más alto
        assert_eq!(
            (e.cpu_ms, e.peak_memory_mb, e.spill_mb),
            (Some(500), Some(1200), Some(40))
        );
        assert_eq!(e.source_kind, "query");
        // la primera falla: termina ahí, con la de detrás saltada
        g.celdas.get_mut(&4).unwrap().salida =
            salida(r#"{"tipo":"error","nombre":"CatalogException","mensaje":"no hay v","ms":2}"#);
        g.celdas.get_mut(&5).unwrap().salida =
            salida(r#"{"tipo":"vacia","saltada":true,"por":4,"ms":0}"#);
        let e = terminada("g", &g, 4, 10_000).expect("termina con la que falla");
        assert_eq!(e.status, "failed");
        assert_eq!(e.error_name.as_deref(), Some("CatalogException"));
        assert_eq!(
            e.steps.iter().map(|p| p.status).collect::<Vec<_>>(),
            ["failed", "skipped"]
        );
        // una celda suelta de una sesión, en L
        let mut s = crate::puestos::prueba::un_puesto("agente:ana");
        let mut c = celda("print(1)", 0);
        c.lote = None;
        c.salida = salida(r#"{"tipo":"texto","ms":1}"#);
        s.celdas.insert(1, c);
        let e = terminada("s", &s, 1, 10_000).unwrap();
        assert_eq!((e.id.as_str(), e.kind, e.compute), ("s/1", "cell", "L"));
        assert_eq!(e.cpu_ms, None, "un agente de antes no lo mide");
        assert!(e.steps.is_empty());
    }

    #[test]
    fn la_fila_es_plana_y_dice_lo_que_sabe() {
        let e = Ejecucion {
            id: "x/1".into(),
            kind: "query",
            persona: "ana".into(),
            source_kind: "query",
            source_id: Some("informe.sql".into()),
            source_commit: None,
            source_branch: None,
            statement: "select 1;\nselect 2".into(),
            queued_at: 0,
            started_at: 20,
            ended_at: 1_020,
            status: "failed",
            error_name: Some("BinderException".into()),
            error_message: Some("no existe t".into()),
            compute: "G",
            worker: "x".into(),
            cpu_ms: Some(1200),
            peak_memory_mb: Some(900),
            spill_mb: Some(0),
            read: vec![],
            steps: vec![
                Paso {
                    statement: "select 1".into(),
                    status: "ok",
                    duration_ms: 5,
                    ..Paso::default()
                },
                Paso {
                    statement: "select 2".into(),
                    status: "failed",
                    duration_ms: 1,
                    ..Paso::default()
                },
            ],
            growth: vec![crate::elastica::Crecimiento {
                at_ms: 500,
                from_mb: 4096,
                to_mb: 8192,
                cpu_m: 4000,
                reason: "derrama".into(),
            }],
            memory_final_mb: Some(8192),
            growth_refused: Some("quota".into()),
            log: None,
        };
        let n = ore_core::parse::parse(&e.fila(None)).unwrap();
        let v = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        assert_eq!(v("subject").as_deref(), Some("user:ana"));
        assert_eq!(v("wait_ms").as_deref(), Some("20"));
        assert_eq!(v("duration_ms").as_deref(), Some("1000"));
        assert_eq!(v("compute").as_deref(), Some("G"));
        assert_eq!(v("cpu_ms").as_deref(), Some("1200"));
        assert_eq!(v("peak_memory_mb").as_deref(), Some("900"));
        // Elastic Sessions S2b: lo que creció, en JSON; el tope final y la negativa.
        assert_eq!(
            v("growth").as_deref(),
            Some(r#"[{"at_ms":500,"cpu_m":4000,"from_mb":4096,"reason":"derrama","to_mb":8192}]"#)
        );
        assert_eq!(v("memory_final_mb").as_deref(), Some("8192"));
        assert_eq!(v("growth_refused").as_deref(), Some("quota"));
        assert_eq!(v("error_name").as_deref(), Some("BinderException"));
        assert_eq!(v("organization"), None);
        assert_eq!(v("source_commit"), None);
        assert!(v("steps").unwrap().contains("\"status\":\"failed\""));
        // todas son cadenas: `ore-store anexar` sólo lee filas planas
        for (k, val) in n.entries() {
            assert!(val.as_str().is_some(), "{k:?}");
        }
        // y todas son columnas declaradas
        for (k, _) in n.entries() {
            let k = k.as_str().unwrap();
            assert!(
                ore_core::sistema::COLUMNAS_DE_EJECUCIONES
                    .iter()
                    .any(|(c, _, _)| *c == k),
                "{k}"
            );
        }
    }
}
