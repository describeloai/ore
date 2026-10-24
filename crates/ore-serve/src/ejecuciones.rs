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

/// Un paso de una ejecución: una sentencia de un guion.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Paso {
    pub statement: String,
    /// `ok`, `failed` o `skipped`.
    pub status: &'static str,
    pub duration_ms: i64,
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
        ] {
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                m.push((k, Json::s(v)));
            }
        }
        if !self.steps.is_empty() {
            m.push((
                "steps",
                Json::s(
                    Json::Arr(
                        self.steps
                            .iter()
                            .map(|p| {
                                Json::obj([
                                    (
                                        "statement",
                                        Json::s(recortar(&p.statement, TOPE_DE_SENTENCIA)),
                                    ),
                                    ("status", Json::s(p.status)),
                                    ("duration_ms", Json::Int(p.duration_ms)),
                                ])
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
    })
}

/// **La cola de filas por escribir**, de `(tabla, fila)`. Apagada (sin
/// `--executions`) no guarda nada.
#[derive(Default)]
pub(crate) struct Registro {
    encendido: AtomicBool,
    cola: Mutex<Vec<(String, String)>>,
    campana: Condvar,
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
pub(crate) fn asegurar_al_arrancar(ore: &Path) {
    let [_, org] = destinos("");
    let mut espera = Duration::from_secs(5);
    for _ in 0..12 {
        match asegurar(ore, &org) {
            Ok(_) => {
                eprintln!("executions · `{org}` está");
                return;
            }
            Err(e) => {
                eprintln!("executions · `{org}` aún no ({e}): otra vez en {espera:?}");
                std::thread::sleep(espera);
                espera = (espera * 2).min(Duration::from_secs(300));
            }
        }
    }
    eprintln!("executions · `{org}` no se pudo crear: nacerá con la primera fila");
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
                },
                Paso {
                    statement: "select 2".into(),
                    status: "failed",
                    duration_ms: 1,
                },
            ],
        };
        let n = ore_core::parse::parse(&e.fila(None)).unwrap();
        let v = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        assert_eq!(v("subject").as_deref(), Some("user:ana"));
        assert_eq!(v("wait_ms").as_deref(), Some("20"));
        assert_eq!(v("duration_ms").as_deref(), Some("1000"));
        assert_eq!(v("compute").as_deref(), Some("G"));
        assert_eq!(v("cpu_ms").as_deref(), Some("1200"));
        assert_eq!(v("peak_memory_mb").as_deref(), Some("900"));
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
