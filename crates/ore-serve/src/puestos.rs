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
//!
//! # La cola y la memoria
//!
//! El puesto vive en dos sitios: su fichero en la cola (duradero: Flux recrea
//! el Job mientras esté) y su estado aquí (en memoria). Medido el 2026-10-02:
//! cuando se separaban —un reinicio de este servidor, el TTL del agente— el
//! fichero se quedaba, Flux recreaba el Job, el agente recibía 410 y se iba,
//! y así cada diez minutos, reservando nodos de pago hasta agotar la cuota.
//! La regla: **en la cola sólo está lo que esta memoria tiene vivo.** El
//! agente que cierra por inactividad lo notifica (`POST /puestos/{id}/cierre`), y
//! [`barrer_la_cola`] quita, al arrancar y cada [`BARRIDO`], lo demás.

use crate::cola;
use crate::rutas::Servidor;

/// La cabecera con la que el agente de un puesto dice desde qué puesto
/// escribe: el sujeto pasa a ser la persona que lo abrió (y la rama, la del
/// puesto).
pub(crate) const PUESTO: &str = "x-ore-puesto";
use ore_core::json::Json;
use ore_entrada::http::{Emisor, Flujo, Respuesta, Salida};
use ore_entrada::identidad::Identidad;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    /// Si el trabajo es la invocación de una función de código (0050 P3): su
    /// informe va también a `resultados/`, donde lo busca quien la consume.
    pub funcion: Option<Invocada>,
}

/// La invocación de una `Function` de `runtime: python` que corre como trabajo.
#[derive(Debug, Clone)]
pub(crate) struct Invocada {
    /// Su forma corta (`p.f`, `p.s.f`).
    pub qn: String,
    pub corrida: String,
    pub parametros: Json,
    /// Los modelos que declara (`models`), ya resueltos: si hay alguno, el
    /// trabajo lleva la etiqueta que abre la salida al gateway (0050 P4).
    pub modelos: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct Puesto {
    pub persona: String,
    /// **La decisión de `ore-iam` que lo dejó abrir** (`puesto:abrir`, 0053 F4·3):
    /// se pregunta con el token de la persona, que sólo está al abrirlo, y la
    /// nombra cada evento que sale del puesto — la celda habla con el token de su
    /// pod, y sin ella `ore-iam` no sabría a quién anotarlo. `None` sin puente.
    pub decision: Option<String>,
    /// `python`, `node` o `jvm`: la imagen (`cola::ENTORNOS`).
    pub entorno: String,
    pub rama: Option<String>,
    pub fichero: String,
    pub job: String,
    /// R1 · La apertura: el instante (s) que va en su Job y en la audiencia de
    /// la credencial de su pod (`ore-serve/puestos/<id>/<apertura>`).
    pub apertura: String,
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
    /// ⭐ 0050 P5·1 · **La capa con la que arrancó** (`capa-<12 hex>`, o vacía
    /// sin capa): el puesto la baja al nacer y no la cambia. La consola la
    /// compara con la de ahora del repositorio (`GET /entorno`) para decir
    /// «Libraries changed» —y reiniciar— aunque el cambio venga de otro sitio.
    pub capa: String,
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
    /// **Las colecciones que el puesto leyó sin transform** (0049 B4·2): una
    /// sesión interactiva lee la media que quiera, y queda aquí —en su forma
    /// corta— lo que leyó. Es la procedencia de lo que esa sesión escriba
    /// (el `derivedFrom` de una colección escrita, v1alpha19 `01` §2).
    pub colecciones_leidas: BTreeSet<String>,
}

/// Lo declarado por el transform que corre en este puesto.
#[derive(Debug, Clone)]
pub(crate) struct Transform {
    pub nombre: String,
    pub inputs: Vec<String>,
    pub output: String,
    /// **Lo fijado** (0049 B4·2): de cada input que es una colección, la
    /// transacción que su puntero tenía al declararlo. El trabajo lee ésa
    /// aunque la colección cambie mientras corre: una lectura que se repite
    /// da lo mismo, y la procedencia dice *qué* transacción se leyó.
    pub fijadas: BTreeMap<String, Fijada>,
}

/// La transacción de una colección que un transform fijó al declararla.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Fijada {
    pub metadata_location: String,
    pub transaccion: String,
}

/// Qué deja leer de una colección el puesto de quien pregunta (0049 B4·2).
#[derive(Debug, PartialEq)]
pub(crate) enum MediaDelPuesto {
    /// Sin transform (o sin puesto): se lee la del puntero, y se anota.
    Libre,
    /// Declarada por el transform que corre: la fijada, si se fijó.
    Declarada(Option<Fijada>),
    /// El transform que corre no la declaró: 403.
    NoDeclarada {
        transform: String,
        inputs: Vec<String>,
    },
}

/// Todo lo vivo, bajo un candado, y una campana para las esperas.
#[derive(Default)]
pub(crate) struct Puestos {
    lista: Mutex<BTreeMap<String, Puesto>>,
    campana: Condvar,
    /// Encolar (hasta que el puesto está en `lista`) y barrer no se cruzan:
    /// si no, el barrido vería en la cola el fichero de un puesto que aún no
    /// está en la memoria, y se lo llevaría.
    cola: Mutex<()>,
}

/// Cada cuánto se barre la cola ([`barrer_la_cola`]).
pub(crate) const BARRIDO: Duration = Duration::from_secs(300);

/// Los ficheros de puestos y trabajos de la cola que no son de `vivos`.
fn sobrantes<'a>(
    en_la_cola: impl IntoIterator<Item = &'a str>,
    vivos: &BTreeSet<String>,
) -> Vec<String> {
    let mut v: Vec<String> = en_la_cola
        .into_iter()
        .filter(|f| {
            (f.starts_with("51-el-puesto-") || f.starts_with("54-el-trabajo-"))
                && f.ends_with(".yaml")
        })
        .filter(|f| !vivos.contains(*f))
        .map(str::to_string)
        .collect();
    v.sort();
    v
}

/// ⭐ **La cola, con lo que esta memoria tiene vivo** (ver «La cola y la
/// memoria»). Un fichero de puesto o de trabajo cuyo puesto no está aquí —un
/// reinicio lo olvidó— o está perdido —el agente se fue sin decirlo, o el Job
/// llegó a su tope— sale de la cola, y el perdido pasa a cerrado (si su agente
/// vuelve, 410). Lo que dice va al registro.
pub(crate) fn barrer_la_cola(forja: &crate::git::Forja, puestos: &Puestos) -> String {
    let _cola = puestos.cola.lock().unwrap_or_else(|e| e.into_inner());
    let prestado = match forja.clonar() {
        Ok(p) => p,
        Err(e) => return format!("NO barrida: {e}"),
    };
    let dir = prestado.ruta();
    let vivos: BTreeSet<String> = {
        let mut lista = puestos.lista.lock().unwrap();
        for p in lista.values_mut() {
            if perdido(p) {
                p.estado = Estado::Cerrado;
                p.pendientes.clear();
            }
        }
        lista
            .values()
            .filter(|p| p.estado != Estado::Cerrado)
            .map(|p| p.fichero.clone())
            .collect()
    };
    puestos.campana.notify_all();
    let nombres: Vec<String> = match std::fs::read_dir(dir) {
        Ok(d) => d
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .collect(),
        Err(e) => return format!("NO barrida: {e}"),
    };
    let fuera = sobrantes(nombres.iter().map(String::as_str), &vivos);
    if fuera.is_empty() {
        return String::new();
    }
    for f in &fuera {
        if let Err(e) = std::fs::remove_file(dir.join(f)) {
            return format!("NO barrida: `{f}`: {e}");
        }
    }
    let quien = Identidad {
        persona: "ore-serve".into(),
        agente: None,
        correo: None,
        nombre: None,
        tipo: None,
        usuario: None,
    };
    let que = format!(
        "Barrer la cola: {} sin puesto vivo ({})",
        fuera.len(),
        fuera.join(", ")
    );
    match forja.publicar(dir, &quien, &que) {
        Ok(c) => format!("{que} · commit {c}"),
        Err(e) => format!("NO barrida: {e}"),
    }
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
    let sin_prefijo = persona.rsplit(':').next().unwrap_or(persona);
    match repositorio
        .map(str::trim)
        .map(|r| r.trim_matches('/'))
        .filter(|s| !s.is_empty())
    {
        // ⭐ R3 · Un puesto por repositorio, y el repositorio ENTERO: la última
        //   carpeta sola juntaba `a/funcs` y `b/funcs` en un puesto. Lo legible
        //   se acorta y lo que distingue va en el hash (persona + ruta).
        //
        //   ⛔ Y en 63: el Job se llama `<id>-<8 hex>` y su nombre es el valor de
        //   la etiqueta `job-name` de su pod. Antes, con un `sub` de UUID y un
        //   repositorio largo, daba 68 y el Job no se creaba. Ahora, como mucho:
        //   7 + 16 + 1 + 6 + 1 + 12 + 1 + 6 = 50, y con el `-<8 hex>`, 59.
        Some(ruta) => {
            let repo = ruta.rsplit('/').next().unwrap_or(ruta);
            let h = ore_core::digest::de_bytes(format!("{persona}\n{ruta}").as_bytes());
            let h = &h["sha256:".len().."sha256:".len() + 6];
            format!(
                "puesto-{}-{entorno}-{}-{h}",
                corto(sin_prefijo, 16),
                corto(repo, 12)
            )
        }
        None => format!("puesto-{}-{entorno}", corto(sin_prefijo, 24)),
    }
}

/// Los lenguajes que una celda puede llevar.
const LENGUAJES: [&str; 5] = ["python", "sql", "typescript", "javascript", "java"];

/// ¿Este entorno declara sus dependencias en el árbol? `python` desde W3.2
/// (`pyproject.toml`), `jvm` desde 0037 ③c (`pom.xml`) y `node` desde 0050 R3
/// T5b (`package.json`): los tres.
pub(crate) fn declara_capa(entorno: &str) -> bool {
    entorno == crate::entorno::PYTHON
        || entorno == crate::entorno::JVM
        || entorno == crate::entorno::NODE
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

/// R1 · **El puesto que declara la credencial** de quien llama: `(id, apertura)`
/// si es `agente:puesto/<id>/<apertura>` —verificado por `ore-entrada` contra
/// la audiencia que `ore-serve` puso en el Job—. Entonces es ESE puesto, en ESA
/// apertura, y lo que diga `x-ore-puesto` no cuenta.
pub(crate) fn declarado(sujeto: &Identidad) -> Option<(&str, &str)> {
    sujeto
        .persona
        .strip_prefix(ore_entrada::oidc::PREFIJO_PUESTO)?
        .split_once('/')
}

/// R1 · ¿Es esta la apertura del puesto que declara la credencial? Un puesto
/// reabierto (TTL, tope, relevo) es otra apertura: la credencial de la vieja ya
/// no lo es.
fn es_su_apertura(sujeto: &Identidad, id: &str, p: &Puesto) -> bool {
    declarado(sujeto).is_none_or(|(suyo, apertura)| suyo == id && apertura == p.apertura)
}

/// R1 · El puesto que habla, bajo el candado de la lista: el que declara su
/// credencial, si está vivo y en esa apertura; si no la trae, el que dice la
/// cabecera (el camino de antes, que comprueba el agente donde se usa).
pub(crate) fn puesto_que_llama_en(
    lista: &BTreeMap<String, Puesto>,
    cabecera: Option<&String>,
    sujeto: &Identidad,
) -> Option<String> {
    if let Some((id, _)) = declarado(sujeto) {
        return lista
            .get(id)
            .filter(|p| p.estado != Estado::Cerrado && es_su_apertura(sujeto, id, p))
            .map(|_| id.to_string());
    }
    cabecera
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// [`Servidor::media_del_puesto`], sobre la lista ya bajo su candado.
pub(crate) fn media_en(
    lista: &mut BTreeMap<String, Puesto>,
    sujeto: &Identidad,
    coleccion: &str,
) -> MediaDelPuesto {
    if !es_agente(sujeto) {
        return MediaDelPuesto::Libre;
    }
    let mut suyos: Vec<&mut Puesto> = lista
        .values_mut()
        .filter(|p| {
            p.estado != Estado::Cerrado && p.agente.as_deref() == Some(sujeto.persona.as_str())
        })
        .collect();
    if let Some(t) = suyos.iter().find_map(|p| p.transform.as_ref()) {
        return if t.inputs.iter().any(|i| i == coleccion) {
            MediaDelPuesto::Declarada(t.fijadas.get(coleccion).cloned())
        } else {
            MediaDelPuesto::NoDeclarada {
                transform: t.nombre.clone(),
                inputs: t.inputs.clone(),
            }
        };
    }
    for p in suyos.iter_mut() {
        p.colecciones_leidas.insert(coleccion.to_string());
    }
    MediaDelPuesto::Libre
}

/// **Quién escribe una colección desde un puesto** (0049 B4b·2): el puesto del
/// agente que pide —como en [`media_en`], por el agente y no por la cabecera—,
/// su persona, su rama y la procedencia de lo que escriba.
#[derive(Debug, PartialEq)]
pub(crate) struct EscrituraDelPuesto {
    pub id: String,
    pub persona: String,
    pub rama: Option<String>,
    /// `{puesto, transform, inputs, fijadas}` dentro de un transform;
    /// `{puesto, leidas}` en una sesión.
    pub procedencia: Json,
}

/// Lo que deja escribir en `coleccion` (forma corta) el puesto de quien pide.
/// `Ok(None)`: no es un agente, o no tiene puesto vivo, y escribe como sí
/// mismo. `Err((tipo, detalle))`: la clase del repositorio no escribe datos
/// (0036 ⑤, sólo quita), o el transform que corre no la declaró como `output`.
pub(crate) fn escritura_en(
    lista: &BTreeMap<String, Puesto>,
    sujeto: &Identidad,
    coleccion: &str,
) -> Result<Option<EscrituraDelPuesto>, (&'static str, String)> {
    if !es_agente(sujeto) {
        return Ok(None);
    }
    // R1 · El que declara la credencial, si la trae; si no, los del agente.
    let todos: Vec<(&String, &Puesto)> = match declarado(sujeto) {
        Some(_) => puesto_que_llama_en(lista, None, sujeto)
            .and_then(|id| lista.get_key_value(&id))
            .into_iter()
            .collect(),
        None => lista
            .iter()
            .filter(|(_, p)| {
                p.estado != Estado::Cerrado && p.agente.as_deref() == Some(sujeto.persona.as_str())
            })
            .collect(),
    };
    let Some((id, p)) = todos
        .iter()
        .find(|(_, p)| p.transform.is_some())
        .or_else(|| todos.first())
    else {
        // ⛔ R1 · Un pod verificado sin puesto no escribe «como sí mismo»: un
        //   pod es siempre un puesto, y uno sin él no es nadie con techo.
        if declarado(sujeto).is_some() {
            return Err((
                "media/sin-puesto",
                "esta credencial declara un puesto que no está vivo en esta apertura".into(),
            ));
        }
        return Ok(None);
    };
    if let Some(c) = p.clase
        && !c.escribe
    {
        return Err((
            "media/sin-permiso",
            format!(
                "este puesto vive en un repositorio `{}`, y esa clase no escribe datos: para escribir, un repositorio `transforms` o `models`",
                c.id
            ),
        ));
    }
    let procedencia = match &p.transform {
        Some(t) if t.output != coleccion => {
            return Err((
                "media/no-declarada",
                format!(
                    "`{coleccion}` no es el output de `{}` (`{}`): un transform sólo escribe lo que declara",
                    t.nombre,
                    if t.output.is_empty() {
                        "nada"
                    } else {
                        &t.output
                    }
                ),
            ));
        }
        Some(t) => Json::obj([
            ("puesto", Json::s(*id)),
            ("transform", Json::s(&t.nombre)),
            ("inputs", Json::Arr(t.inputs.iter().map(Json::s).collect())),
            ("fijadas", fijadas_json(&t.fijadas)),
        ]),
        None => Json::obj([
            ("puesto", Json::s(*id)),
            (
                "leidas",
                Json::Arr(p.colecciones_leidas.iter().map(Json::s).collect()),
            ),
        ]),
    };
    Ok(Some(EscrituraDelPuesto {
        id: (*id).clone(),
        persona: p.persona.clone(),
        rama: p.rama.clone(),
        procedencia,
    }))
}

/// `{<colección>: <transacción>}`: lo que un transform fijó, como se enseña.
fn fijadas_json(f: &BTreeMap<String, Fijada>) -> Json {
    Json::Obj(
        f.iter()
            .map(|(c, x)| (c.clone(), Json::s(&x.transaccion)))
            .collect(),
    )
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
        m.insert("capa".into(), Json::s(&p.capa));
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
        if !t.fijadas.is_empty() {
            m.insert("fijadas".into(), fijadas_json(&t.fijadas));
        }
    }
    if let Json::Obj(m) = &mut f
        && !p.colecciones_leidas.is_empty()
    {
        m.insert(
            "colecciones_leidas".into(),
            Json::Arr(p.colecciones_leidas.iter().map(Json::s).collect()),
        );
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
        //
        // ⛔⛔ Y SE LEE DE LA RAMA POR DEFECTO, no de la del puesto (2026-10-02).
        //   La identidad de un repositorio —que existe y de qué clase es— es un
        //   REGISTRO, como en GitHub o en Foundry: vive en `main` (protegida) y
        //   no en cada rama. Leída de la rama del puesto, cambiar en ella
        //   `functions-python` por `transforms-python` en el manifiesto abría un
        //   puesto que escribe datos: el techo se saltaba editando un fichero.
        //   Las ramas llevan el CONTENIDO; cambiar la clase es tocar `main`, con
        //   propuesta. Y un repositorio que sólo está en una rama no es uno.
        let clase = match &repositorio {
            None => None,
            Some(a) => {
                let r = self.leyendo(
                    |raiz| match ore_core::repositorios::leer(raiz)
                        .into_iter()
                        .find(|r| r.ruta == *a)
                    {
                        None => Respuesta::error(
                            404,
                            format!(
                                "`{a}` no es un repositorio registrado: un repositorio existe en la rama por defecto, y en una rama sólo vive su contenido"
                            ),
                        ),
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
        // ⭐ 0053 F4·3 · La decisión que lo deja abrir, ahora que está el token de
        //   la persona: la renueva también el que ya lo tenía.
        let decision = self.decision_de_apertura(sujeto);
        {
            let mut lista = self.puestos.lista.lock().unwrap();
            // Uno por persona: si lo tiene y da señales (o aún arranca), es ése.
            // Uno PERDIDO (vivo sin latido: TTL, tope o relevo; o encolado
            // que nunca arrancó) se sustituye. Y uno abierto en OTRA rama
            // también: la sesión lee y escribe la rama en que se trabaja, y
            // devolver el de otra era cotejar contra un árbol que no es el tuyo.
            if let Some(p) = lista.get_mut(&id)
                && p.estado != Estado::Cerrado
                && !perdido(p)
                && p.rama == rama
            {
                if decision.is_some() {
                    p.decision = decision;
                }
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
        // A la cola: Flux rinde el Job. Hasta que esté en la lista, sin barrido.
        let _cola = self.puestos.cola.lock().unwrap_or_else(|e| e.into_inner());
        let (fichero, job, dicho, apertura) =
            match self.encolar_puesto(&id, sujeto, rama.as_deref(), &capa, entorno, "", false) {
                Ok(v) => v,
                Err(r) => return r,
            };
        let p = Puesto {
            persona: sujeto.persona.clone(),
            entorno: entorno.to_string(),
            rama,
            fichero,
            job,
            apertura,
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
            capa: capa.clone(),
            clase,
            transform: None,
            colecciones_leidas: BTreeSet::new(),
            decision: decision.clone(),
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
        self.lanzar_trabajo(
            sujeto, rama, entorno, lenguaje, codigo, commit, texto, avisos, None, None,
        )
    }

    /// Un trabajo, ya decidido: la capa, el Job en la cola y la celda. Lo usan
    /// `POST /trabajos` (un fichero del árbol) y la invocación de una función
    /// de código (0050 P3: el arnés que llama al `def`), que llega con lo
    /// declarado ya puesto —`transform`, el techo de lo que lee y escribe— y
    /// con la función a la que informar.
    /// La rama de un trabajo que no la pide: la de la persona, como la de
    /// `POST /trabajos` sin `rama` (lo escrito es de quien lo escribió).
    pub(crate) fn rama_para_trabajo(
        &self,
        sujeto: &Identidad,
    ) -> Result<Option<String>, Respuesta> {
        self.rama_del_puesto(sujeto, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lanzar_trabajo(
        &self,
        sujeto: &Identidad,
        rama: Option<String>,
        entorno: &str,
        lenguaje: &str,
        codigo: String,
        commit: String,
        texto: String,
        avisos: Vec<Json>,
        transform: Option<Transform>,
        funcion: Option<Invocada>,
    ) -> Respuesta {
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
        let _cola = self.puestos.cola.lock().unwrap_or_else(|e| e.into_inner());
        let (fichero, job, dicho, apertura) = match self.encolar_puesto(
            &id,
            sujeto,
            rama.as_deref(),
            &capa,
            entorno,
            &format!("{codigo}@{commit}"),
            funcion.as_ref().is_some_and(|f| !f.modelos.is_empty()),
        ) {
            Ok(v) => v,
            Err(r) => return r,
        };
        // Un trabajo lo lanza una persona (o una función, en su nombre): su huella, igual.
        let decision = self.decision_de_apertura(sujeto);
        let mut p = Puesto {
            persona: sujeto.persona.clone(),
            entorno: entorno.to_string(),
            rama,
            fichero,
            job,
            apertura,
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
                funcion,
            }),
            // Un trabajo no vive en un repositorio: corre y termina (0036 ④).
            repositorio: None,
            capa: capa.clone(),
            clase: None,
            transform,
            colecciones_leidas: BTreeSet::new(),
            decision: decision.clone(),
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
                    // ⭐ 0050 R3 T5b: la capa de Node llega con ore-serve, y su
                    //   plantilla (`plantilla-capa-node.txt`) con el inquilino
                    //   convergido, que es a mano. Entre medias, una sesión de
                    //   Node con un `package.json` en su alcance NO se niega: nace
                    //   sin capa, como hasta hoy, y se dice en el registro.
                    Err(r) if entorno == crate::entorno::NODE && r.codigo == 503 => {
                        eprintln!(
                            "puestos · la capa de Node {} no se encola (la cola no trae su plantilla: hay que converger el inquilino): el puesto nace sin ella",
                            campo("digest")
                        );
                        return Ok(String::new());
                    }
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
        let (persona, rama, fichero, mut informe, funcion) = {
            let lista = self.puestos.lista.lock().unwrap();
            let Some(p) = lista.get(id) else { return };
            let Some(t) = &p.trabajo else { return };
            let funcion = t.funcion.clone();
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
                funcion,
            )
        };
        let quien = Identidad {
            persona: persona.clone(),
            agente: Some(agente.persona.clone()),
            correo: None,
            nombre: None,
            tipo: None,
            usuario: None,
        };
        let ruta = format!("trabajos/{id}.json");
        let texto = informe.pretty() + "\n";
        // 0050 P3: la invocación de una función deja además su resultado donde
        // lo busca quien la consume (`GET /funciones/…/resultados`, la consola),
        // con el mismo nombre que el del Job de `runtime: model`.
        let resultado = funcion.as_ref().map(|f| {
            let mut m = match &informe {
                Json::Obj(m) => m.clone(),
                _ => Default::default(),
            };
            m.insert("function".into(), Json::s(&f.qn));
            m.insert("runtime".into(), Json::s("python"));
            m.insert("corrida".into(), Json::s(&f.corrida));
            m.insert("parametros".into(), f.parametros.clone());
            m.insert("trabajo".into(), Json::s(id));
            (
                format!(
                    "resultados/{}_{}.json",
                    ore_core::punteros::resultados_de(&f.qn),
                    f.corrida
                ),
                Json::Obj(m).pretty() + "\n",
            )
        });
        let mensaje = match &funcion {
            Some(f) => format!("Invocar: {} ({})", f.qn, f.corrida),
            None => format!("Trabajo {id}"),
        };
        let r = self.escribiendo_en(rama.as_deref(), &quien, &mensaje, |raiz| {
            for (ruta, texto) in
                std::iter::once((&ruta, &texto)).chain(resultado.as_ref().map(|(r, t)| (r, t)))
            {
                let f = raiz.join(ruta);
                if let Some(d) = f.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                if let Err(e) = std::fs::write(&f, texto) {
                    return Respuesta::error(500, format!("no se pudo escribir `{ruta}`: {e}"));
                }
            }
            Respuesta::ok(Json::obj([("fichero", Json::s(&ruta))]))
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
                                corre: celda_de_sentencia(fichero, x, &pkg),
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
                // ADR 0040 paso 5: `create view` / `drop view` de una View del
                // árbol corren como una sentencia del guion, como `create schema`.
                Some(que) => match celda_de_sesion(raiz, fichero, texto) {
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
                            EscribeEnElArbol::Vista(n) => format!(
                                "la celda crea o quita la vista `{n}` del árbol, y eso corre como \
                                 una sentencia del árbol: {primero}"
                            ),
                            EscribeEnElArbol::Tabla(n) => format!(
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
        if !es_su_apertura(sujeto, id, p) {
            return Err(Respuesta::error(
                403,
                "esta credencial declara otro puesto, u otra apertura de éste: ciérrate",
            ));
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

    /// `POST /puestos/{id}/latido` (0049 B2·3): **estoy vivo, y ocupado**. Lo
    /// manda el agente mientras corre una celda, cada 30 s: el agente no pide
    /// trabajo mientras trabaja, y sin latido el puesto pasaba a `perdido` a los
    /// [`SIN_LATIDO`] —una celda de OCR de diez minutos lo perdía a mitad—.
    /// No reclama ninguna celda: sólo `reclamar`, que pone el latido.
    pub(crate) fn latido_del_puesto(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let mut lista = self.puestos.lista.lock().unwrap();
        match Self::reclamar(&mut lista, sujeto, id) {
            Ok(_) => Respuesta::sin_contenido(),
            Err(r) => r,
        }
    }

    /// `POST /puestos/{id}/cierre`: **el agente notifica su cierre por inactividad** (TTL):
    /// cerrado y fuera de la cola. Sin esto el fichero se quedaba, Flux recreaba
    /// el Job y el agente volvía a arrancar, a recibir 410 y a salir cada diez minutos.
    pub(crate) fn cierre_por_inactividad(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let (fichero, quien) = {
            let mut lista = self.puestos.lista.lock().unwrap();
            let p = match Self::reclamar(&mut lista, sujeto, id) {
                Ok(p) => p,
                Err(r) => return r,
            };
            p.estado = Estado::Cerrado;
            p.pendientes.clear();
            let quien = Identidad {
                persona: p.persona.clone(),
                agente: Some(sujeto.persona.clone()),
                correo: None,
                nombre: None,
                tipo: None,
                usuario: None,
            };
            (p.fichero.clone(), quien)
        };
        self.puestos.campana.notify_all();
        let dicho = self.desencolar_puesto(&fichero, &quien);
        Respuesta::ok(Json::obj([
            ("id", Json::s(id)),
            ("estado", Json::s("cerrado")),
            ("cola", Json::s(dicho)),
        ]))
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
        // Y su `output` (0049 B5·2): un incremental lee lo que ya escribió para
        // saber qué está hecho; leerse no es una entrada (no entra en el linaje).
        if let Some(t) = self.transform_de(id)
            && ore_core::normalize::a_corto(&t.output) != vista
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
        // 0049 B4·2: las colecciones de `inputs` se fijan en la rama del
        // puesto, fuera del candado (leer el árbol hace un `fetch`).
        let rama = {
            let mut lista = self.puestos.lista.lock().unwrap();
            match Self::reclamar(&mut lista, sujeto, id) {
                Ok(p) => p.rama.clone(),
                Err(r) => return r,
            }
        };
        let fijadas = self.fijar_colecciones(rama.as_deref(), &inputs);
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
        let enseñadas = fijadas_json(&fijadas);
        p.transform = Some(Transform {
            nombre: nombre.clone(),
            inputs: inputs.clone(),
            output: output.clone(),
            fijadas,
        });
        Respuesta::ok(Json::obj([
            ("transform", Json::s(nombre)),
            ("inputs", Json::Arr(inputs.iter().map(Json::s).collect())),
            ("output", Json::s(output)),
            ("fijadas", enseñadas),
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

    /// **Lo que una ruta `/media` deja leer a quien pregunta** (0049 B4·2).
    ///
    /// El puesto se busca por el agente que lo reclamó, no por `x-ore-puesto`:
    /// la cabecera la pone el SDK y el código de la celda la puede quitar, y
    /// lo declarado no se rodea quitando una cabecera. Si alguno de sus
    /// puestos corre un transform, manda: sólo sus `inputs`, y de la
    /// transacción fijada. Si no, se lee libre y la colección se anota en
    /// sus puestos (`colecciones_leidas`). Quien no es agente —la consola, una
    /// persona— lee libre y no se anota: no tiene puesto.
    pub(crate) fn media_del_puesto(&self, sujeto: &Identidad, coleccion: &str) -> MediaDelPuesto {
        media_en(&mut self.puestos.lista.lock().unwrap(), sujeto, coleccion)
    }

    /// [`escritura_en`], bajo el candado de la lista.
    pub(crate) fn escritura_del_puesto(
        &self,
        sujeto: &Identidad,
        coleccion: &str,
    ) -> Result<Option<EscrituraDelPuesto>, (&'static str, String)> {
        escritura_en(&self.puestos.lista.lock().unwrap(), sujeto, coleccion)
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
        // Each name with whether it is a `MediaCollection` (0049 B7·1).
        // And the tree `Function`s it calls, with the text rewritten for
        // DuckDB (0049 B7·2): the SDK registers each one under its internal name.
        let mut llamadas: Option<Json> = None;
        // 0053 F6·1: los nombres que llegan a un origen —una `Table`, o una vista
        // sin copia cuya raíz es una— se leen en vivo: `(nombre, tabla)` y
        // `(vista, su SQL)`.
        let mut tablas: Vec<(String, String)> = Vec::new();
        let mut vistas_vivas: Vec<(String, String)> = Vec::new();
        let r = self.leyendo_en(rama.as_deref(), |raiz| {
            let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
            for n in ore_core::sql_del_arbol::nombres_a_resolver(&texto, &pkg) {
                if let Some(t) = pkg.table(&n) {
                    tablas.push((n, t.qname().unwrap_or_default()));
                } else if let Some(v) = pkg.view(&n)
                    && !ore_core::vistas::se_lee_de_datasets(&pkg, v)
                    && !ore_core::reparto::tablas_de_la_vista(&pkg, v).is_empty()
                    && let Some(sql) = ore_core::reparto::sql_de_vista(v)
                {
                    vistas_vivas.push((n, sql));
                }
            }
            let (query, calls) = ore_core::sql_del_arbol::sql_calls(&texto, &pkg);
            if !calls.is_empty() {
                llamadas = Some(Json::obj([
                    ("query", Json::s(query)),
                    (
                        "functions",
                        Json::Arr(
                            calls
                                .into_iter()
                                .map(|c| {
                                    Json::obj([
                                        ("name", Json::s(c.name)),
                                        ("internal", Json::s(c.internal)),
                                        ("arity", Json::Int(c.arity as i64)),
                                        ("table", Json::Bool(c.table)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]));
            }
            Respuesta::ok(Json::Arr(
                ore_core::sql_del_arbol::nombres_a_resolver(&texto, &pkg)
                    .into_iter()
                    .map(|n| {
                        let col = pkg.docs.iter().any(|d| {
                            d.kind == ore_core::document::Kind::MediaCollection
                                && d.qname().as_deref() == Some(n.as_str())
                        });
                        Json::Arr(vec![Json::s(n), Json::Bool(col)])
                    })
                    .collect(),
            ))
        });
        let nombres: Vec<(String, bool)> = match &r.cuerpo {
            Json::Arr(xs) if r.codigo == 200 => xs
                .iter()
                .filter_map(|x| match x {
                    Json::Arr(p) => match (p.first(), p.get(1)) {
                        (Some(Json::Str(s)), Some(Json::Bool(c))) => Some((s.clone(), *c)),
                        _ => None,
                    },
                    _ => None,
                })
                .collect(),
            _ => return r,
        };
        let mut fuentes = std::collections::BTreeMap::new();
        // 0053 F6·1: lo que llega a un origen, repartido UNA vez para toda la
        // sentencia (F5: una lectura por tabla); el SDK pide cada lectura a
        // `/federation/read` y ejecuta la sentencia tal cual.
        if !tablas.is_empty() || !vistas_vivas.is_empty() {
            let linea = match self.explicar(rama.as_deref(), &texto, true) {
                Ok(l) => l,
                Err(r) => return r,
            };
            let plan = match ore_core::parse::parse(&linea) {
                Ok(n) => Json::de_node(&n),
                Err(_) => return Respuesta::error(502, "`ore explain` no devolvió JSON"),
            };
            let Json::Obj(plan) = plan else {
                return Respuesta::error(502, "`ore explain` no devolvió un objeto");
            };
            if !matches!(plan.get("ok"), Some(Json::Bool(true))) {
                let campo = |k: &str| match plan.get(k) {
                    Some(Json::Str(v)) => v.clone(),
                    _ => String::new(),
                };
                let http = match plan.get("http") {
                    Some(Json::Int(h)) => *h as u16,
                    _ => 422,
                };
                let mut cuerpo = vec![
                    ("error", Json::s(campo("mensaje"))),
                    ("codigo", Json::s(campo("codigo"))),
                ];
                if !campo("tabla").is_empty() {
                    cuerpo.push(("nombre", Json::s(campo("tabla"))));
                }
                return Respuesta {
                    codigo: http,
                    cuerpo: Json::obj(cuerpo),
                };
            }
            let lecturas = match plan.get("lecturas") {
                Some(Json::Arr(ls)) => ls.clone(),
                _ => Vec::new(),
            };
            let tabla_de = |l: &Json| match l {
                Json::Obj(m) => match m.get("tabla") {
                    Some(Json::Str(t)) => t.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            for l in &lecturas {
                fuentes.insert(tabla_de(l), Json::obj([("federada", l.clone())]));
            }
            // Un nombre escrito de otra forma (`a.default.t`) que la tabla.
            for (n, qn) in &tablas {
                if let Some(l) = lecturas.iter().find(|l| &tabla_de(l) == qn) {
                    fuentes.insert(n.clone(), Json::obj([("federada", l.clone())]));
                }
            }
            for (n, sql) in &vistas_vivas {
                fuentes.insert(
                    n.clone(),
                    Json::obj([("vistaFederada", Json::s(sql.as_str()))]),
                );
            }
            if let Some(Json::Arr(av)) = plan.get("avisos")
                && !av.is_empty()
            {
                fuentes.insert(
                    "__avisos".into(),
                    Json::obj([("avisos", Json::Arr(av.clone()))]),
                );
            }
        }
        for (n, coleccion) in nombres {
            if fuentes.contains_key(&n) {
                continue;
            }
            // 0049 B7·1: a collection is read by its items, and the SDK lists
            // them through ore-medios, where what is declared is enforced (B4·2):
            // here it only says so.
            if coleccion {
                fuentes.insert(n.clone(), Json::obj([("collection", Json::s(&n))]));
                continue;
            }
            let mut d = self.datos_del_puesto(sujeto, id, &n);
            if d.codigo != 200 {
                if let Json::Obj(m) = &mut d.cuerpo {
                    m.insert("nombre".into(), Json::s(&n));
                }
                return d;
            }
            fuentes.insert(n, d.cuerpo);
        }
        let mut cuerpo = Json::obj([("fuentes", Json::Obj(fuentes))]);
        if let (Json::Obj(m), Some(Json::Obj(l))) = (&mut cuerpo, llamadas) {
            m.extend(l);
        }
        Respuesta::ok(cuerpo)
    }

    /// Lo que se lee por un nombre: un dataset por su puntero (`datos_de`), o
    /// **una View como la pregunta que es** (`datos_de_vista`). Si una View y
    /// un dataset se llaman igual, manda el dataset, como en `datos_de`.
    fn datos_o_vista(&self, raiz: &Path, ns: &str, nombre: &str, vista: &str) -> Respuesta {
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        let solo_vista = pkg.view(vista).is_some() && pkg.dataset(vista).is_none();
        let mut r = if solo_vista {
            self.datos_de_vista(raiz, &pkg, vista)
        } else {
            self.con_credencial(raiz, datos_de(raiz, ns, nombre, vista))
        };
        // ORE 0051 P7 · v1alpha22 `01` §7: qué columnas nunca son nulas, del
        // árbol —lo que el origen garantiza o la consulta deriva—, porque el
        // SDK lee con DuckDB y DuckDB no mira la marca de Iceberg. Sólo si hay
        // alguna: la respuesta de lo que no garantiza nada es la de antes.
        let doc = pkg.dataset(vista).or_else(|| pkg.view(vista));
        if let (200, Some(d), Json::Obj(m)) = (r.codigo, doc, &mut r.cuerpo) {
            let nunca: Vec<Json> = ore_core::vistas::nulabilidad_de_vista(&pkg, d)
                .into_iter()
                .filter(|(_, n)| n.nunca_nula())
                .map(|(c, _)| Json::s(c))
                .collect();
            if !nunca.is_empty() {
                m.insert("nunca_nulas".into(), Json::Arr(nunca));
            }
        }
        r
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
            && !ore_core::vistas::se_lee_de_datasets(pkg, v)
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
        // ⛔ R1 · Un pod verificado es SIEMPRE su puesto, diga o no la cabecera:
        //   quitarla era escribir como el agente, sin la persona ni el techo.
        let id = self.puesto_que_llama(p, sujeto);
        if declarado(sujeto).is_some() && id.is_none() {
            return Err(Respuesta::error(
                403,
                "esta credencial declara un puesto que no está vivo en esta apertura: ciérrate",
            ));
        }
        match id.as_deref() {
            Some(id) => {
                let (persona, rama_del_puesto) = self.persona_del_puesto(sujeto, id)?;
                Ok((
                    Identidad {
                        persona,
                        agente: Some(sujeto.persona.clone()),
                        correo: None,
                        nombre: None,
                        tipo: None,
                        usuario: None,
                    },
                    rama_del_puesto.or_else(|| rama.map(String::from)),
                ))
            }
            None => Ok((sujeto.clone(), rama.map(String::from))),
        }
    }

    /// **`POST /puestos/{id}/explain {texto}`** (0053 F6·1): el reparto de una
    /// sentencia, en la rama del puesto —qué va a cada origen y qué hace
    /// DuckDB—, en JSON (`plan`) y para leer (`texto`). Un no del reparto no es
    /// un error de la ruta: va en el plan.
    pub(crate) fn explain_del_puesto(
        &self,
        sujeto: &Identidad,
        id: &str,
        cuerpo: &str,
    ) -> Respuesta {
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
            return Respuesta::error(422, "explain() quiere una consulta");
        }
        let plan = match self.explicar(rama.as_deref(), &texto, true) {
            Ok(l) => match ore_core::parse::parse(&l) {
                Ok(n) => Json::de_node(&n),
                Err(_) => return Respuesta::error(502, "`ore explain` no devolvió JSON"),
            },
            Err(r) => return r,
        };
        let leido = match self.explicar(rama.as_deref(), &texto, false) {
            Ok(t) => t,
            Err(r) => return r,
        };
        Respuesta::ok(Json::obj([("plan", plan), ("texto", Json::s(leido))]))
    }

    /// R1 · El puesto que habla: [`puesto_que_llama_en`], bajo el candado.
    pub(crate) fn puesto_que_llama(
        &self,
        p: &ore_entrada::http::Peticion,
        sujeto: &Identidad,
    ) -> Option<String> {
        puesto_que_llama_en(
            &self.puestos.lista.lock().unwrap(),
            p.cabeceras.get(PUESTO),
            sujeto,
        )
    }

    /// 0053 F4·3 · `puesto:abrir`, con el token de quien lo abre. Sin puente o sin
    /// token, nada; si `ore-iam` niega o no contesta, el puesto se abre igual —
    /// abrir no lo exigía— y lo que haga dentro no tendrá huella: se dice aquí.
    pub(crate) fn decision_de_apertura(&self, sujeto: &Identidad) -> Option<String> {
        let acceso = self.acceso.as_ref()?;
        let Some(t) = crate::acceso::testigo_de_la_peticion() else {
            eprintln!(
                "puestos · {} abre sin token: lo que haga no tendrá huella",
                sujeto.persona
            );
            return None;
        };
        match acceso.puede(
            &t,
            &sujeto.persona,
            "puesto:abrir",
            ore_acceso::Recurso::ORGANIZACION,
            "POST /puestos",
        ) {
            ore_acceso::Decision::Permite { id } => Some(id),
            d => {
                eprintln!(
                    "puestos · ✗ `puesto:abrir` para {}: {} · lo que haga no tendrá huella",
                    sujeto.persona,
                    d.codigo()
                );
                None
            }
        }
    }

    /// 0053 F4·3 · **La decisión con que se anota lo que pide un puesto**: la de
    /// su apertura, si quien llama es el agente que lo reclamó. Lo demás, `None`
    /// (una persona en la consola trae su token, que es mejor prueba).
    pub(crate) fn decision_de_quien_llama(
        &self,
        p: &ore_entrada::http::Peticion,
    ) -> Option<String> {
        let proveedor = self.identidad.as_ref()?;
        let sujeto = proveedor(&p.cabeceras).ok()?;
        if !es_agente(&sujeto) {
            return None;
        }
        let id = self.puesto_que_llama(p, &sujeto)?;
        let lista = self.puestos.lista.lock().unwrap();
        let puesto = lista.get(&id)?;
        if puesto.agente.as_deref() != Some(sujeto.persona.as_str()) {
            return None;
        }
        puesto.decision.clone()
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

    #[allow(clippy::too_many_arguments)]
    fn encolar_puesto(
        &self,
        id: &str,
        sujeto: &Identidad,
        rama: Option<&str>,
        capa: &str,
        entorno: &str,
        trabajo: &str,
        usa_modelo: bool,
    ) -> Result<(String, String, String, String), Respuesta> {
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
        // 0050 P4: el trabajo de una función que declara `models` sale al
        // gateway, y ningún otro puesto.
        let plantilla = if usa_modelo {
            cola::con_salida_al_modelo(&plantilla).map_err(|e| Respuesta::error(503, e))?
        } else {
            plantilla
        };
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
                abierto,
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
                abierto,
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
                let anclada = ore_core::sql_del_arbol::anchored_to(&pkg, &u);
                return Ok(celda_de_unidad(codigo, &u, anclada.as_deref()));
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
                return Ok(celda_de_sentencia(codigo, &t[0], &pkg));
            }
            f
        }
        Err(f) => f,
    };
    Err(rechazo(codigo, &fallos))
}

/// **Una sentencia del guion, como la celda que la corre** (0039). La que lee
/// o escribe datos es [`celda_de_unidad`]; la que crea algo del catálogo llama
/// al verbo del SDK —`create_database`, `create_schema`, `create_dataset`…— con
/// lo que la frase dice. La escribe este proceso a partir del análisis, no el
/// cliente, y por los nombres de `ore_core::sdk` (S3): en inglés y con la guarda
/// de la versión del SDK delante.
fn celda_de_sentencia(
    codigo: &str,
    t: &ore_core::sql_del_arbol::guion::Trozo,
    pkg: &ore_core::link::Package,
) -> (String, &'static str) {
    use ore_core::sql_del_arbol::guion::Sentencia as S;
    let c = |s: &str| Json::s(s).jcs();
    let si = |b: bool| if b { "True" } else { "False" };
    let cabeza = format!(
        "# `{codigo}`: `{}`, as the statement says (written by ore-serve, not the client).\n{}",
        t.sentencia.que(),
        ore_core::sdk::guarda_python()
    );
    let cuerpo = match &t.sentencia {
        S::Unidad(u) => {
            let anclada = ore_core::sql_del_arbol::anchored_to(pkg, u);
            return celda_de_unidad(codigo, u, anclada.as_deref());
        }
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
                "from ore import create_database, _resultado_de_crear\n\n\
                 _hecho = create_database({}, kind={}, origin={o}, include={inc}, if_not_exists={})\n\
                 print(\"%s · %s database · %s\" % (_hecho[\"database\"], _hecho[\"kind\"], \"created\" if _hecho[\"created\"] else \"already exists\"))\n\
                 _resultado_de_crear(\"%s database %s\" % (_hecho[\"kind\"], _hecho[\"database\"]), _hecho[\"created\"])\n",
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
            "from ore import create_schema, _resultado_de_crear\n\n\
             _hecho = create_schema({}, {}, if_not_exists={})\n\
             print(\"%s · schema · %s\" % (_hecho[\"schema\"], \"created\" if _hecho[\"created\"] else \"already exists\"))\n\
             _resultado_de_crear(\"schema \" + _hecho[\"schema\"], _hecho[\"created\"])\n",
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
                "from ore import create_dataset, _resultado_de_crear\n\n\
                 _hecho = create_dataset({}, {cols}, key={clave}, if_not_exists={})\n\
                 print(\"%s · empty dataset · %s\" % (_hecho[\"dataset\"], \"created\" if _hecho[\"created\"] else \"already exists\"))\n\
                 _resultado_de_crear(\"dataset \" + _hecho[\"dataset\"], _hecho[\"created\"])\n",
                c(&destino.referencia()),
                si(*si_no_existe)
            )
        }
        // ADR 0040 paso 5. Lo que el árbol sabe al escribir la celda va en la
        // llamada: si ya hay una vista con ese nombre, el contrato que tenía
        // (para decir si reemplazarla lo rompe). El contrato nuevo lo describe
        // DuckDB en el puesto, que es quien sabe ejecutarla. ⭐ Y el dueño NO va:
        // lo pone `PUT /documentos` —quien la crea, o el que ya tenía si se
        // reemplaza— (0052 · Ownership).
        S::CrearVista {
            destino,
            consulta,
            columnas,
            comentario,
            o_reemplaza,
            si_no_existe,
            evolucion,
            materializada,
            ..
        } => {
            let r = destino.referencia();
            let hay = pkg.view(&r);
            let anterior = match hay {
                Some(v) if ore_core::vistas::es_sql(v) => {
                    match ore_core::vistas::tipos_del_contrato(v) {
                        Ok(t) => Json::Obj(
                            t.iter()
                                .map(|(k, v)| (k.clone(), Json::s(v.to_string())))
                                .collect(),
                        )
                        .jcs(),
                        Err(_) => "None".to_string(),
                    }
                }
                _ => "None".to_string(),
            };
            let cols = if columnas.is_empty() {
                "None".to_string()
            } else {
                // Un literal de Python: cada cadena como JSON (que Python lee
                // igual) y la ausencia como `None`.
                format!(
                    "[{}]",
                    columnas
                        .iter()
                        .map(|k| format!(
                            "[{},{}]",
                            c(&k.nombre),
                            k.comentario.as_deref().map_or("None".to_string(), c)
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            format!(
                "from ore import create_view, _resultado_de_crear\n\n\
                 _hecho = create_view({}, {}, columns={cols}, comment={}, owner=None, or_replace={}, \
                 if_not_exists={}, schema_evolution={}, exists={}, previous_columns={anterior}, materialized={})\n\
                 print(\"%s · view · %s\" % (_hecho[\"view\"], _hecho[\"status\"]))\n\
                 _resultado_de_crear(\"view \" + _hecho[\"view\"], _hecho[\"status\"])\n",
                c(&r),
                c(consulta),
                comentario.as_deref().map_or("None".to_string(), c),
                si(*o_reemplaza),
                si(*si_no_existe),
                si(*evolucion),
                si(hay.is_some()),
                si(*materializada),
            )
        }
        // ADR 0049 B4·4: la colección escrita, por el mismo verbo que Python. El
        // dueño no va: lo pone el servidor (0052 · Ownership).
        S::CrearColeccion {
            destino,
            media,
            formatos,
            comentario,
            si_no_existe,
            source,
            is_virtual,
        } => format!(
            "from ore import create_collection, _resultado_de_crear\n\n\
             _hecho = create_collection({}, {}, {}, comment={}, if_not_exists={}{})\n\
             print(\"%s · media collection · %s\" % (_hecho[\"collection\"], \"created\" if _hecho[\"created\"] else \"already exists\"))\n\
             _resultado_de_crear(\"media collection \" + _hecho[\"collection\"], _hecho[\"created\"])\n",
            c(&destino.referencia()),
            c(media),
            Json::Arr(formatos.iter().map(Json::s).collect()).jcs(),
            comentario.as_deref().map_or("None".to_string(), c),
            si(*si_no_existe),
            // 0049 B8: from an object table, managed or virtual.
            source.as_ref().map_or(String::new(), |t| format!(
                ", source={}, virtual={}",
                c(&t.referencia()),
                si(*is_virtual)
            ))
        ),
        // 0049 B8: served in place ↔ copied into the lake.
        S::AlterCollection { target, managed } => format!(
            "from ore import alter_collection, _resultado_de_crear\n\n\
             _hecho = alter_collection({}, managed={})\n\
             print(\"%s · media collection · %s\" % (_hecho[\"collection\"], _hecho[\"status\"]))\n\
             _resultado_de_crear(\"media collection \" + _hecho[\"collection\"], _hecho[\"status\"])\n",
            c(&target.referencia()),
            si(*managed)
        ),
        // 0049 B8·3: its columns and its detail (`DESCRIBE TABLE EXTENDED`).
        S::Describe { kind, target } => format!(
            "from ore import describe, _resultado_de_describir\n\n\
             _filas = describe({}, kind={})\n\
             _resultado_de_describir(_filas)\n",
            c(&target.referencia()),
            c(kind)
        ),
        S::BorrarVista { destino, si_existe } => format!(
            "from ore import drop_view, _resultado_de_crear\n\n\
             _hecho = drop_view({}, if_exists={})\n\
             print(\"%s · view · %s\" % (_hecho[\"view\"], _hecho[\"status\"]))\n\
             _resultado_de_crear(\"view \" + _hecho[\"view\"], _hecho[\"status\"])\n",
            c(&destino.referencia()),
            si(*si_existe)
        ),
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
fn celda_de_unidad(
    codigo: &str,
    u: &ore_core::sql_del_arbol::Unidad,
    anclada: Option<&str>,
) -> (String, &'static str) {
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
    // 0049 B7·3: written from a collection, the dataset is anchored to it and
    // computed item by item by `apply()`: the query, per item, with the same
    // registry by key. What did not change is neither computed nor written.
    if let Some(c) = anclada {
        let col = cadena(c);
        let celda = format!(
            "# `{codigo}`: the statement reads the collection {col}, so its dataset is\n\
             # anchored to it and computed item by item (written by ore-serve, not the client).\n\
             {guarda}\
             from ore import transform, collection, _sql_per_item, _resultado_de_aplicar\n\
             \n\
             \n\
             @transform(inputs=[collection({col})], output={salida})\n\
             def {nombre}():\n    \
                 return _sql_per_item({salida}, {col}, {consulta}, {fn_name})\n\
             \n\
             \n\
             _hecho = {nombre}()\n\
             print(\"%s · anchored to %s · %d items: %d new, %d recomputed, %d skipped, %d errors, \
             %d removed · %d rows%s\" % ({salida}, {col}, _hecho[\"items\"], _hecho[\"new\"], \
             _hecho[\"recomputed\"], _hecho[\"skipped\"], _hecho[\"errors\"], _hecho[\"removed\"], \
             _hecho[\"rows\"], \"\" if _hecho[\"written\"] else \" · nothing new\"))\n\
             _resultado_de_aplicar(_hecho)\n",
            consulta = cadena(&u.consulta),
            fn_name = cadena(&nombre),
            guarda = ore_core::sdk::guarda_python(),
        );
        return (celda, "python");
    }
    // Un `insert` con columnas sin alias (`select letra, 0.5 from …`): ésas
    // toman el nombre de la columna de la tabla en su posición, como en SQL.
    let (mut importa, mut datos) = if e.por_posicion.is_empty() {
        (
            String::new(),
            format!("sql({}, format=\"arrow\")", cadena(&u.consulta)),
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
                "_por_posicion(sql({}, format=\"arrow\"), {salida}, {posiciones})",
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
        "# `{codigo}`: the statement declares what it reads and what it writes, and runs\n\
         # with the same `@transform` as a `.py` (written by ore-serve, not the client).\n\
         {guarda}\
         from ore import transform, sql, write, _resultado_de_escritura{importa}\n\
         \n\
         \n\
         @transform(inputs={inputs}, output={salida})\n\
         def {nombre}():\n    \
             return write({salida}, {datos}, mode={modo})\n\
         \n\
         \n\
         _escrito = {nombre}()\n\
         print(\"%s · %s · %d rows%s\" % ({salida}, {modo}, _escrito[\"rows\"], \" · the same write: nothing new\" if _escrito[\"repeated\"] else \"\"))\n\
         _resultado_de_escritura(_escrito)\n",
        modo = cadena(e.modo.como_en_el_sdk()),
        guarda = ore_core::sdk::guarda_python(),
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
    // 0046 E9.4: las columnas que son la huella de un item, y de que coleccion
    // (la Entity que el dataset respalda lo declara). El SDK sirve el item por
    // `GET /colecciones/{b}/{s}/{n}/items/{huella}`.
    let media: std::collections::BTreeMap<String, Json> = pkg
        .docs
        .iter()
        .find(|d| d.kind == Kind::Dataset && d.qname().as_deref() == Some(del_dataset.as_str()))
        .map(|d| ore_core::vistas::media_de(&pkg, d))
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, Json::s(v)))
        .collect();
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
        ("media", Json::Obj(media)),
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
            "return write(\"v.resumen\", _como_la_tabla(sql(\"select id from v.pedidos where id > 1\", format=\"arrow\"), \"v.resumen\"), mode=\"upsert\")",
            "_escrito = resumen()",
            "print(\"%s · %s · %d rows%s\" % (\"v.resumen\", \"upsert\", _escrito[\"rows\"]",
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
        // un árbol vacío: la vista no existe y su dueño es el equipo de su base
        let vacio = std::env::temp_dir().join(format!("ore-celda-{}", std::process::id()));
        std::fs::create_dir_all(&vacio).unwrap();
        let (pkg, _) = ore_core::validate::cargar_paquete(&vacio);
        let celda = |q: &str| {
            let t = guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}"));
            celda_de_sentencia("x.sql", &t[0], &pkg)
        };
        let (c, l) = celda("create schema if not exists ventas.demo");
        assert_eq!(l, "python");
        assert!(
            c.contains("create_schema(\"ventas\", \"demo\", if_not_exists=True)"),
            "{c}"
        );
        let (c, _) = celda("create dataset ventas.demo.clientes (id bigint, n varchar)");
        assert!(
            c.contains("create_dataset(\"ventas.demo.clientes\", [[\"id\",\"long\"],[\"n\",\"string\"]], key=None, if_not_exists=False)"),
            "{c}"
        );
        let (c, _) = celda("create foreign database espejo from origin erp include (s.*, t.x)");
        assert!(
            c.contains("create_database(\"espejo\", kind=\"foreign\", origin=\"erp\", include=[\"s.*\",\"t.x\"], if_not_exists=False)"),
            "{c}"
        );
        let (c, _) = celda("create database mi_base");
        assert!(
            c.contains("kind=\"standard\", origin=None, include=None"),
            "{c}"
        );
        let (c, l) = celda("select 1");
        assert_eq!((c.as_str(), l), ("select 1", "sql"));
        let (c, _) = celda("insert into ventas.x (a) values (1)");
        assert!(
            c.contains("@transform(inputs=[], output=\"ventas.x\")"),
            "{c}"
        );
        // ADR 0040 paso 5: la vista, con su consulta tal cual y lo que el árbol sabe
        let (c, l) = celda(
            "create or replace view ventas.v (a comment 'la a', b) comment 'x' as\nselect 1 as a, 2 as b",
        );
        assert_eq!(l, "python");
        assert!(
            c.contains("create_view(\"ventas.v\", \"select 1 as a, 2 as b\", columns=[[\"a\",\"la a\"],[\"b\",None]], comment=\"x\", owner=None, or_replace=True, if_not_exists=False, schema_evolution=False, exists=False, previous_columns=None, materialized=False)"),
            "{c}"
        );
        // ADR 0040 paso 7: la materializada, la misma llamada y su copia
        let (c, _) = celda("create materialized view ventas.m as select 1 as a");
        assert!(c.contains("materialized=True)"), "{c}");
        let (c, _) = celda("drop view if exists ventas.v");
        assert!(c.contains("drop_view(\"ventas.v\", if_exists=True)"), "{c}");
        let (c, _) = celda(
            "create media collection if not exists ventas.demo.docs media document formats (pdf) comment 'x'",
        );
        assert!(
            c.contains("create_collection(\"ventas.demo.docs\", \"document\", [\"pdf\"], comment=\"x\", if_not_exists=True)"),
            "{c}"
        );
        // 0049 B8: from an object table, and served in place ↔ managed
        let (c, _) = celda(
            "create media collection ventas.demo.v media document formats (pdf) from object table s3.docs.t virtual",
        );
        assert!(
            c.contains("if_not_exists=False, source=\"s3.docs.t\", virtual=True)"),
            "{c}"
        );
        let (c, _) = celda("alter media collection ventas.demo.v set managed");
        assert!(
            c.contains("alter_collection(\"ventas.demo.v\", managed=True)"),
            "{c}"
        );
        // 0049 B8·3: where it is and what it holds
        let (c, _) = celda("describe media collection ventas.demo.v");
        assert!(
            c.contains("describe(\"ventas.demo.v\", kind=\"media collection\")")
                && c.contains("_resultado_de_describir(_filas)"),
            "{c}"
        );
    }

    /// ⭐ S3 · EL CÓDIGO QUE ORE GENERA CASA CON EL SDK. Cada forma que se
    /// genera —una celda por sentencia del guion, la de un `.sql` del árbol—:
    ///
    /// - empieza por la guarda de la versión (`ore_core::sdk`);
    /// - no lleva ningún nombre de antes (S5 los retira y nada se rompe);
    /// - y lo que importa de `ore` existe en el SDK de verdad: se lee su fuente
    ///   (`puesto/python/ore/__init__.py`), sin ejecutar Python.
    ///
    /// Con `ORE_CELDAS_GENERADAS=<dir>` además se vuelcan, y
    /// `pruebas-de-fuego/el-codigo-generado-casa-con-el-sdk.py` coteja cada
    /// llamada con la firma real de la función (los argumentos).
    #[test]
    fn el_codigo_generado_casa_con_el_sdk() {
        use ore_core::sql_del_arbol::guion::guion;
        let vacio = std::env::temp_dir().join(format!("ore-celdas-sdk-{}", std::process::id()));
        std::fs::create_dir_all(&vacio).unwrap();
        let (pkg, _) = ore_core::validate::cargar_paquete(&vacio);
        let mut corpus: Vec<(String, String)> = [
            "create schema if not exists ventas.demo",
            "create dataset ventas.demo.clientes (id bigint, n varchar) ",
            "create database mi_base",
            "create foreign database espejo from origin erp include (s.*, t.x)",
            "create or replace view ventas.v (a comment 'la a', b) comment 'x' as select 1 as a, 2 as b",
            "create view if not exists ventas.v2 with schema evolution as select 1 as a",
            "create materialized view ventas.m as select 1 as a",
            "drop view if exists ventas.v",
            "create media collection if not exists ventas.demo.docs media document formats (pdf) comment 'x'",
            "create media collection ventas.demo.vdocs media document formats (pdf) from object table s3.docs.t virtual",
            "alter media collection ventas.demo.vdocs set managed",
            "describe media collection ventas.demo.vdocs",
            "describe view ventas.v",
            "insert into ventas.x (a) values (1)",
            "insert into ventas.x select 1, 2",
            "insert or replace into ventas.x select 1 as a",
            "create or replace dataset ventas.r as select 1 as a",
        ]
        .iter()
        .enumerate()
        .filter_map(|(i, q)| {
            let t = guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}"));
            let (c, l) = celda_de_sentencia("x.sql", &t[0], &pkg);
            (l == "python").then(|| (format!("{i:02}-sentencia"), c))
        })
        .collect();
        // 0049 B7·3: the dataset written from a collection, anchored.
        let u = ore_core::sql_del_arbol::analizar(
            "create or replace dataset ventas.paginas as select c.path from ventas.docs as c",
        )
        .unwrap();
        corpus.push((
            "90-anclada".into(),
            celda_de_unidad("x.sql", &u, Some("ventas.docs")).0,
        ));
        assert!(corpus.len() >= 15, "{}", corpus.len());
        let sdk = include_str!("../../../puesto/python/ore/__init__.py");
        // Lo que el SDK exporta: su `__all__` (lo público) y sus `def` del
        // primer nivel (los `_resultado_de_*` que sólo usa el código generado).
        let todos = sdk
            .find("__all__ = [")
            .map(|i| &sdk[i..i + sdk[i..].find(']').unwrap()])
            .expect("el SDK no tiene `__all__`");
        let exporta =
            |n: &str| todos.contains(&format!("\"{n}\"")) || sdk.contains(&format!("\ndef {n}("));
        let guarda = ore_core::sdk::guarda_python();
        for (n, c) in &corpus {
            assert!(
                c.contains(&guarda),
                "{n}: sin la guarda de la versión del SDK:\n{c}"
            );
            let antes = ore_core::sdk::nombres_de_antes_en(c);
            assert!(
                antes.is_empty(),
                "{n}: lleva nombres de antes {antes:?}:\n{c}"
            );
            for l in c.lines().filter(|l| l.starts_with("from ore import ")) {
                for i in l.trim_start_matches("from ore import ").split(", ") {
                    assert!(
                        exporta(i.trim()),
                        "{n}: importa `{i}`, que el SDK no tiene:\n{c}"
                    );
                }
            }
        }
        if let Ok(dir) = std::env::var("ORE_CELDAS_GENERADAS") {
            std::fs::create_dir_all(&dir).unwrap();
            for (n, c) in corpus.drain(..) {
                std::fs::write(std::path::Path::new(&dir).join(format!("{n}.py")), c).unwrap();
            }
        }
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
        let raw = id_de("persona:ana", "python", Some("packages/hr/raw"));
        assert!(
            raw.starts_with("puesto-ana-python-raw-")
                && raw.len() == "puesto-ana-python-raw-".len() + 6,
            "{raw}"
        );
        assert_ne!(
            raw,
            id_de("persona:ana", "python", Some("packages/hr/clean"))
        );
        // R3 · Y dos con la misma carpeta en paquetes distintos, también.
        assert_ne!(
            raw,
            id_de("persona:ana", "python", Some("packages/ventas/raw"))
        );
        // Y de dos personas, aunque el `sub` empiece igual.
        assert_ne!(
            id_de(
                "4f0a9c2e-1b2c-4d5e-8f90-aaaa",
                "python",
                Some("packages/hr/raw")
            ),
            id_de(
                "4f0a9c2e-1b2c-4d5e-8f90-bbbb",
                "python",
                Some("packages/hr/raw")
            )
        );
        // El nombre del repositorio también se acorta y se limpia.
        assert!(
            id_de("persona:ana", "python", Some("packages/hr/Con Espacios"))
                .starts_with("puesto-ana-python-con-espacios-")
        );
        // ⛔ R3 · El Job (`<id>-<8 hex>`) cabe en 63, con el `sub` y la carpeta más largos.
        let largo = id_de(
            "21e8ffd9-5aae-4797-a9f9-b81bde8e1780",
            "python",
            Some("packages/un_paquete_muy_largo/una_carpeta_de_repositorio_larguisima"),
        );
        assert!(largo.len() + 9 <= 63, "{largo} ({})", largo.len() + 9);
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

    fn un_puesto(agente: &str) -> Puesto {
        Puesto {
            persona: "persona:ana".into(),
            entorno: "python".into(),
            rama: None,
            fichero: String::new(),
            job: String::new(),
            apertura: "1".into(),
            creado: Instant::now(),
            estado: Estado::Vivo,
            agente: Some(agente.into()),
            latido: None,
            siguiente: 1,
            pendientes: VecDeque::new(),
            celdas: BTreeMap::new(),
            trabajo: None,
            repositorio: None,
            capa: String::new(),
            clase: None,
            lsp_al_servidor: VecDeque::new(),
            lsp_generacion: 0,
            lsp_a_la_consola: VecDeque::new(),
            lsp_siguiente: 0,
            transform: None,
            colecciones_leidas: BTreeSet::new(),
            decision: None,
        }
    }

    fn agente(p: &str) -> Identidad {
        Identidad {
            persona: p.into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: Some("agente".into()),
            usuario: None,
        }
    }

    /// R1 · Cada puesto es el que DECLARA su credencial: el de functions no
    /// escribe aunque el de transforms, del mismo agente de celda, sí; la
    /// cabecera no cuenta; otra apertura no es el puesto; y no hay «reclamar
    /// primero» que valga para otro.
    /// La cola sólo guarda lo vivo: puestos y trabajos que la memoria no tiene
    /// vivos salen; lo demás de la cola (plantillas, copias) no se toca.
    #[test]
    fn de_la_cola_sobra_lo_que_no_esta_vivo() {
        let vivos: BTreeSet<String> = ["51-el-puesto-ana-python.yaml".to_string()].into();
        let cola = [
            "51-el-puesto-ana-python.yaml",
            "51-el-puesto-ana-node.yaml",
            "54-el-trabajo-ana.yaml",
            "plantilla-puesto.txt",
            "50-la-copia-x.yaml",
            "51-el-puesto-ana-jvm.yml",
        ];
        assert_eq!(
            sobrantes(cola, &vivos),
            ["51-el-puesto-ana-node.yaml", "54-el-trabajo-ana.yaml"]
        );
        assert!(sobrantes(["plantilla-puesto.txt"], &BTreeSet::new()).is_empty());
    }

    #[test]
    fn cada_puesto_es_el_que_declara_su_credencial() {
        let de_f = agente("agente:puesto/puesto-ana-python-funcs/100");
        let de_t = agente("agente:puesto/puesto-ana-python-trans/200");
        let mut lista = BTreeMap::new();
        let mut f = un_puesto(&de_f.persona);
        f.apertura = "100".into();
        f.clase = ore_core::clases::de("functions-python");
        let mut t = un_puesto(&de_t.persona);
        t.apertura = "200".into();
        t.clase = ore_core::clases::de("transforms-python");
        lista.insert("puesto-ana-python-funcs".to_string(), f);
        lista.insert("puesto-ana-python-trans".to_string(), t);

        assert!(matches!(
            escritura_en(&lista, &de_f, "legal.archivo.paginas"),
            Err(("media/sin-permiso", _))
        ));
        assert_eq!(
            escritura_en(&lista, &de_t, "legal.archivo.paginas")
                .unwrap()
                .unwrap()
                .id,
            "puesto-ana-python-trans"
        );

        // La cabecera no cuenta: el de functions diciendo ser el de transforms
        // sigue siendo el suyo.
        let otra = "puesto-ana-python-trans".to_string();
        assert_eq!(
            puesto_que_llama_en(&lista, Some(&otra), &de_f).as_deref(),
            Some("puesto-ana-python-funcs")
        );
        // Un agente de celda (sin credencial de puesto) sigue por la cabecera.
        assert_eq!(
            puesto_que_llama_en(&lista, Some(&otra), &agente("agente:celda")).as_deref(),
            Some("puesto-ana-python-trans")
        );

        // Otra apertura del mismo puesto (reabierto): ya no es él.
        let vieja = agente("agente:puesto/puesto-ana-python-funcs/99");
        assert_eq!(puesto_que_llama_en(&lista, Some(&otra), &vieja), None);
        assert!(matches!(
            escritura_en(&lista, &vieja, "legal.archivo.paginas"),
            Err(("media/sin-puesto", _))
        ));
        // Y uno que no existe, tampoco.
        let nadie = agente("agente:puesto/puesto-bea-python-x/1");
        assert_eq!(puesto_que_llama_en(&lista, Some(&otra), &nadie), None);

        // Reclamar: sólo el suyo, en su apertura.
        let p = &lista["puesto-ana-python-funcs"];
        assert!(es_su_apertura(&de_f, "puesto-ana-python-funcs", p));
        assert!(!es_su_apertura(&de_t, "puesto-ana-python-funcs", p));
        assert!(!es_su_apertura(&vieja, "puesto-ana-python-funcs", p));
        assert!(es_su_apertura(
            &agente("agente:celda"),
            "puesto-ana-python-funcs",
            p
        ));
    }

    /// 0049 B4·2: dentro de un transform, sólo lo declarado y de lo fijado;
    /// fuera, libre y anotado; y el puesto es el del agente, no el de la cabecera.
    #[test]
    fn la_media_de_un_puesto_obedece_a_su_transform() {
        let fijada = Fijada {
            metadata_location: "gs://lago/legal/contratos/v3.json".into(),
            transaccion: "tx-3".into(),
        };
        let mut lista = BTreeMap::new();
        lista.insert("puesto-ana".to_string(), un_puesto("agente:ana"));
        let ana = agente("agente:ana");

        // Sin transform: libre, y queda anotado lo leído.
        assert_eq!(
            media_en(&mut lista, &ana, "legal.archivo.contratos"),
            MediaDelPuesto::Libre
        );
        assert!(
            lista["puesto-ana"]
                .colecciones_leidas
                .contains("legal.archivo.contratos")
        );

        // Con transform: lo declarado, de lo fijado; lo demás, 403.
        lista.get_mut("puesto-ana").unwrap().transform = Some(Transform {
            nombre: "paginar".into(),
            inputs: vec!["legal.archivo.contratos".into(), "legal.registro".into()],
            output: "legal.archivo.paginas".into(),
            fijadas: BTreeMap::from([("legal.archivo.contratos".to_string(), fijada.clone())]),
        });
        assert_eq!(
            media_en(&mut lista, &ana, "legal.archivo.contratos"),
            MediaDelPuesto::Declarada(Some(fijada))
        );
        assert_eq!(
            media_en(&mut lista, &ana, "legal.registro"),
            MediaDelPuesto::Declarada(None),
            "declarado pero no era una colección al declararlo: sin fijar"
        );
        assert!(matches!(
            media_en(&mut lista, &ana, "legal.archivo.fotos"),
            MediaDelPuesto::NoDeclarada { transform, .. } if transform == "paginar"
        ));
        assert!(
            !lista["puesto-ana"]
                .colecciones_leidas
                .contains("legal.archivo.fotos"),
            "lo negado no se anota"
        );

        // El transform de otro agente no manda sobre éste; quien no es agente lee libre.
        let beto = agente("agente:beto");
        lista.insert("puesto-beto".to_string(), un_puesto("agente:beto"));
        assert_eq!(
            media_en(&mut lista, &beto, "legal.archivo.fotos"),
            MediaDelPuesto::Libre
        );
        let persona = Identidad {
            tipo: None,
            usuario: None,
            ..agente("persona:ana")
        };
        assert_eq!(
            media_en(&mut lista, &persona, "legal.archivo.fotos"),
            MediaDelPuesto::Libre
        );

        // Un puesto cerrado ya no es de nadie.
        lista.get_mut("puesto-ana").unwrap().estado = Estado::Cerrado;
        assert_eq!(
            media_en(&mut lista, &ana, "legal.archivo.fotos"),
            MediaDelPuesto::Libre
        );
    }

    /// 0049 B4b·2: desde un puesto, el `output` del transform y nada más, con
    /// su procedencia; en una sesión, lo leído; la clase sólo quita.
    #[test]
    fn la_escritura_de_un_puesto_obedece_a_su_transform_y_a_su_clase() {
        let mut lista = BTreeMap::new();
        let mut p = un_puesto("agente:ana");
        p.rama = Some("ana/paginas".into());
        p.colecciones_leidas
            .insert("legal.archivo.contratos".into());
        lista.insert("puesto-ana".to_string(), p);
        let ana = agente("agente:ana");

        // Una sesión: escribe, y lleva lo que leyó.
        let e = escritura_en(&lista, &ana, "legal.archivo.paginas")
            .unwrap()
            .unwrap();
        assert_eq!(
            (e.id.as_str(), e.persona.as_str(), e.rama.as_deref()),
            ("puesto-ana", "persona:ana", Some("ana/paginas"))
        );
        assert_eq!(
            e.procedencia,
            Json::obj([
                ("puesto", Json::s("puesto-ana")),
                (
                    "leidas",
                    Json::Arr(vec![Json::s("legal.archivo.contratos")])
                ),
            ])
        );

        // Un transform: su output, con sus inputs y lo fijado; otra, 403.
        lista.get_mut("puesto-ana").unwrap().transform = Some(Transform {
            nombre: "paginar".into(),
            inputs: vec!["legal.archivo.contratos".into()],
            output: "legal.archivo.paginas".into(),
            fijadas: BTreeMap::from([(
                "legal.archivo.contratos".to_string(),
                Fijada {
                    metadata_location: "m3".into(),
                    transaccion: "3".into(),
                },
            )]),
        });
        let e = escritura_en(&lista, &ana, "legal.archivo.paginas")
            .unwrap()
            .unwrap();
        let Json::Obj(m) = &e.procedencia else {
            panic!()
        };
        assert_eq!(m.get("transform"), Some(&Json::s("paginar")));
        assert_eq!(
            m.get("fijadas"),
            Some(&Json::obj([("legal.archivo.contratos", Json::s("3"))]))
        );
        assert_eq!(
            escritura_en(&lista, &ana, "legal.archivo.otra").map(|_| ()),
            Err((
                "media/no-declarada",
                "`legal.archivo.otra` no es el output de `paginar` (`legal.archivo.paginas`): un transform sólo escribe lo que declara".to_string()
            ))
        );

        // Una clase que no escribe quita, aunque el transform lo declare.
        lista.get_mut("puesto-ana").unwrap().clase = ore_core::clases::de("analytics-python");
        assert!(matches!(
            escritura_en(&lista, &ana, "legal.archivo.paginas"),
            Err(("media/sin-permiso", _))
        ));

        // Quien no es agente, o un agente sin puesto, escribe como sí mismo.
        let persona = Identidad {
            tipo: None,
            usuario: None,
            ..agente("persona:ana")
        };
        assert_eq!(
            escritura_en(&lista, &persona, "legal.archivo.paginas"),
            Ok(None)
        );
        assert_eq!(
            escritura_en(&lista, &agente("agente:nadie"), "legal.archivo.paginas"),
            Ok(None)
        );
    }

    #[test]
    fn un_agente_se_reconoce_por_tipo_o_por_prefijo() {
        let a = Identidad {
            persona: "x".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: Some("agente".into()),
            usuario: None,
        };
        let b = Identidad {
            persona: "agente:puesto-ana".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: None,
            usuario: None,
        };
        let c = Identidad {
            persona: "persona:ana".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: None,
            usuario: None,
        };
        assert!(es_agente(&a) && es_agente(&b) && !es_agente(&c));
    }
}
