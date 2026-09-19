//! **El puesto** (ADR 0031, W3.1): la sesión viva de una persona en su celda.
//!
//! Un puesto es un Job de Kueue que **no termina**: la imagen `puesto-python:1`
//! con el agente dentro (`puesto/python/agente.py`), la identidad del pod y la
//! cola del inquilino. Este servidor no toca Kubernetes: **escribe la cola**
//! (`51-el-puesto-<persona>.yaml`, rendido de `plantilla-puesto.txt`) y Flux
//! rinde el Job, como con la copia y la invocación. Retirarlo es quitar el
//! fichero (`prune: true`).
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
    pub enviada: Instant,
    pub empezada: Option<Instant>,
    pub salida: Option<Json>,
}

#[derive(Debug)]
pub(crate) struct Puesto {
    pub persona: String,
    pub lenguaje: String,
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

/// `persona:ana` → `ana`; un `sub` opaco, recortado y en minúsculas.
pub(crate) fn id_de(persona: &str) -> String {
    let s = persona.rsplit(':').next().unwrap_or(persona);
    let s = cola::nombre_de_objeto(s);
    let s = if s.len() > 24 {
        s[..24].trim_end_matches('-').to_string()
    } else {
        s
    };
    format!("puesto-{s}")
}

fn es_agente(sujeto: &Identidad) -> bool {
    sujeto.tipo.as_deref() == Some("agente") || sujeto.persona.starts_with("agente:")
}

fn ficha(id: &str, p: &Puesto) -> Json {
    let vivo = p.estado == Estado::Vivo && p.latido.is_some_and(|l| l.elapsed() < SIN_LATIDO);
    let estado = match p.estado {
        Estado::Vivo if !vivo => "perdido",
        e => e.dice(),
    };
    Json::obj([
        ("id", Json::s(id)),
        ("persona", Json::s(&p.persona)),
        ("lenguaje", Json::s(&p.lenguaje)),
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
    /// celda. Uno por persona: si ya lo tiene, 200 con el que hay.
    pub(crate) fn abrir_puesto(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        if es_agente(sujeto) {
            return Respuesta::error(403, "un agente no abre puestos: los abre una persona");
        }
        let (lenguaje, rama) = if cuerpo.trim().is_empty() {
            ("python".to_string(), None)
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
            (l, r)
        };
        if lenguaje != "python" {
            return Respuesta::error(
                422,
                format!("`{lenguaje}` no tiene puesto todavía: hoy sólo `python` (0031 W3.1)"),
            );
        }
        if let Some(r) = &rama
            && let Err(m) = crate::propuestas::nombre_de_rama_valido(r)
        {
            return Respuesta::error(422, m);
        }
        let id = id_de(&sujeto.persona);
        {
            let lista = self.puestos.lista.lock().unwrap();
            if let Some(p) = lista.get(&id)
                && p.estado != Estado::Cerrado
            {
                return Respuesta::ok(ficha(&id, p));
            }
        }
        // ⭐ La capa (0031 W3.2): lo que el árbol declara, resuelto. Lista →
        //   el puesto nace con ella; pendiente → se encola y 409 para que la
        //   consola espere; error → 409 con el motivo (y se reintenta la capa).
        let e = match self.leyendo_en(rama.as_deref(), |raiz| {
            let e = crate::entorno::entorno_de(raiz);
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
        let (fichero, job, dicho) = match self.encolar_puesto(&id, sujeto, rama.as_deref(), &capa) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let p = Puesto {
            persona: sujeto.persona.clone(),
            lenguaje,
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

    /// `GET /puestos`: los de la persona (uno, hoy).
    pub(crate) fn puestos_de(&self, sujeto: &Identidad) -> Respuesta {
        let lista = self.puestos.lista.lock().unwrap();
        let mios: Vec<Json> = lista
            .iter()
            .filter(|(_, p)| p.persona == sujeto.persona)
            .map(|(id, p)| ficha(id, p))
            .collect();
        Respuesta::ok(Json::obj([("puestos", Json::Arr(mios))]))
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
        if p.estado == Estado::Vivo && p.latido.is_some_and(|l| l.elapsed() > SIN_LATIDO) {
            return Respuesta::error(
                409,
                format!(
                    "el puesto lleva {} s sin dar señales: se perdió (¿TTL, tope o relevo?); ciérralo y abre otro",
                    p.latido.map(|l| l.elapsed().as_secs()).unwrap_or(0)
                ),
            );
        }
        let num = p.siguiente;
        p.siguiente += 1;
        p.celdas.insert(
            num,
            Celda {
                texto: texto.to_string(),
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
                drop(lista);
                self.puestos.campana.notify_all();
                return Respuesta::ok(Json::obj([
                    ("pendiente", Json::Bool(true)),
                    ("celda", Json::Int(n as i64)),
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
        let salida = match ore_core::parse::parse(cuerpo) {
            Ok(v) => crate::rutas::de_node(&v),
            Err(_) => return Respuesta::error(400, "la salida no es JSON"),
        };
        let tipo_ok = matches!(&salida, Json::Obj(m) if matches!(m.get("tipo"), Some(Json::Str(t)) if ["tabla", "texto", "error", "vacia"].contains(&t.as_str())));
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
        c.salida = Some(salida);
        drop(lista);
        self.puestos.campana.notify_all();
        Respuesta::ok(Json::obj([
            ("celda", Json::Int(n as i64)),
            ("estado", Json::s("hecha")),
        ]))
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
        let (ns, nombre, vista) = (ns.to_string(), nombre.to_string(), vista.to_string());
        self.leyendo_en(rama.as_deref(), move |raiz| {
            datos_de(raiz, &ns, &nombre, &vista)
        })
    }

    // ── la cola ─────────────────────────────────────────────────────────────

    fn encolar_puesto(
        &self,
        id: &str,
        sujeto: &Identidad,
        rama: Option<&str>,
        capa: &str,
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
        let (fichero, texto, job) = cola::rendir_puesto(&plantilla, id, rama.unwrap_or(""), capa)
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
        match forja.publicar(dir, sujeto, &format!("Abrir el puesto {id}")) {
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

/// Lo que el árbol dice de la copia de una vista: `copias/<p>_<v>.json`, el
/// informe que deja `ore materialize` (0027). Con `estado: copiada|al-dia` y
/// `clave`, la copia está; si no, 409 con lo que el informe diga.
fn datos_de(raiz: &Path, ns: &str, nombre: &str, vista: &str) -> Respuesta {
    // ¿Existe la vista? El informe de una vista que no está es un 404, no un 409.
    let hay_vista = std::fs::read_dir(raiz.join("packages").join(ns).join("views"))
        .map(|d| {
            d.flatten()
                .any(|e| std::fs::read_to_string(e.path()).is_ok_and(|t| nombra(&t, nombre)))
        })
        .unwrap_or(false);
    if !hay_vista {
        return Respuesta::error(
            404,
            format!("no hay ninguna `View` `{vista}` en el paquete `{ns}`"),
        );
    }
    let informe = raiz.join("copias").join(format!("{ns}_{nombre}.json"));
    let Ok(texto) = std::fs::read_to_string(&informe) else {
        return Respuesta::error(
            409,
            format!(
                "la copia de `{vista}` no está hecha: no declara `materialized` o aún no se copió"
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
    if !matches!(estado.as_str(), "copiada" | "al-dia") || clave.is_empty() {
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
        ("estado", Json::s(estado)),
        ("clave", Json::s(clave)),
        ("plan", Json::s(campo("plan"))),
        ("filas", Json::s(campo("filas"))),
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

    #[test]
    fn el_id_sale_de_la_persona() {
        assert_eq!(id_de("persona:ana"), "puesto-ana");
        assert_eq!(id_de("persona:Ana García"), "puesto-ana-garc-a");
        assert_eq!(
            id_de("4f0a9c2e-1b2c-4d5e-8f90-1234567890ab"),
            "puesto-4f0a9c2e-1b2c-4d5e-8f90"
        );
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
