//! **`ore-acceso`**: el puente de un módulo del plano de datos a `ore-iam` (0047).
//!
//! Dos verbos para decidir y contar, y uno para saber de quién es lo que se crea:
//!
//! - [`Acceso::puede`] pregunta si alguien puede hacer algo (AuthZEN 1.0,
//!   `POST /access/v1/evaluation`). Decide `ore-iam`; esto sólo pregunta, guarda
//!   la respuesta lo que ella diga (`vale`) y, si no hay respuesta, **niega**.
//! - [`Acceso::hizo`] dice lo que se hizo (`POST /access/v1/eventos`), a la huella
//!   de la organización. Si `ore-iam` no contesta, el evento espera en disco y se
//!   reintenta ([`Acceso::reintentar`]) con la decisión que lo dejó pasar: el
//!   token de la persona ya habrá caducado, y `ore-iam` sabe de quién era.
//!
//! Y [`Acceso::hizo_antes`], para lo que no puede quedarse sin rastro —saltarse
//! una protección, entregar un secreto—: se registra ANTES de actuar, y si no se
//! puede registrar, no se actúa.
//!
//! Y [`Acceso::quien`] (0052 · Ownership; la 048 de `ore-iam`): el handle de quien
//! crea, para escribir `owner: user:<handle>` en lo que nace. Un handle no cambia,
//! así que se guarda sin plazo.
//!
//! Y [`Buzon`], para todo lo demás que se escribe (0047 A6.4): el evento se echa y
//! la respuesta no espera a `ore-iam`. Casi nada de eso tiene decisión, así que un
//! reintento necesita el token de la persona: vive sólo en la memoria del buzón y
//! se reintenta mientras valga. Pasado eso, el evento va a `muertos/` sin token.
//!
//! # Quién pregunta: dos tokens
//!
//! `Authorization` lleva **la celda** —su token de Workload Identity, que da la
//! [`Credencial`]—; `Ore-Sujeto`, **la persona**, tal cual llegó al módulo. La
//! organización no viaja: `ore-iam` la deduce de la celda (0047 § «El contrato»).
//!
//! # Lo que M2 dejó escrito aquí
//!
//! - **El nombre con punto final** ([`DESTINO`]): con `ndots:5` un nombre de cuatro
//!   puntos prueba antes los dominios de búsqueda, y el punto ahorra la mitad de
//!   la mediana (4,4 → 2,1 ms).
//! - **Una conexión por pregunta**: ni el servidor ni el cliente de `ore-entrada`
//!   mantienen la conexión, a propósito, y abrirla cuesta 0,3 ms.
//! - **El plazo** ([`PLAZO`]): con un p99 de 5 ms, 2 s son cuatrocientas veces el
//!   p99. ⟨M2⟩ baja a 500 ms cuando Q1 hable.

use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::{Plazos, pedir_con};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Dónde vive `ore-iam` dentro del clúster, con punto final (M2).
pub const DESTINO: &str = "ore-iam.identidad.svc.cluster.local.:8090";

/// Cuánto se espera a quien decide antes de negar con 503 (⟨M2⟩, de partida 2 s).
pub const PLAZO: Duration = Duration::from_secs(2);

/// El techo de `vale`: diga lo que diga la respuesta, no se guarda más. Con esto,
/// una revocación tarda como mucho medio minuto en valer.
pub const VALE_MAXIMO: u64 = 30;

/// Cuántas respuestas se guardan como mucho. Pasado el techo se vacía entera: es
/// una caché de segundos, y vaciarla sólo cuesta volver a preguntar.
const CACHE_MAXIMA: usize = 10_000;

// ── la credencial de la celda ────────────────────────────────────────────────

/// De dónde sale el token con el que la celda se presenta.
pub trait Credencial: Send + Sync {
    fn token(&self) -> Result<String, String>;
}

/// Un token fijo: para pruebas, y para un módulo que ya lo trae.
pub struct Fija(pub String);

impl Credencial for Fija {
    fn token(&self) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

/// El token de Workload Identity del pod, del servidor de metadatos (M3): lo firma
/// Google, con la audiencia que se pide, y vive una hora. Se guarda 50 minutos.
/// No hay nada guardado que robar: nace en el nodo.
pub struct Metadatos {
    pub audiencia: String,
    guardado: Mutex<Option<(String, Instant)>>,
}

impl Metadatos {
    pub fn nuevo(audiencia: &str) -> Metadatos {
        Metadatos {
            audiencia: audiencia.to_string(),
            guardado: Mutex::new(None),
        }
    }
}

const VIDA_DEL_TOKEN: Duration = Duration::from_secs(50 * 60);

impl Credencial for Metadatos {
    fn token(&self) -> Result<String, String> {
        if let Ok(g) = self.guardado.lock()
            && let Some((t, cuando)) = g.as_ref()
            && cuando.elapsed() < VIDA_DEL_TOKEN
        {
            return Ok(t.clone());
        }
        let (codigo, cuerpo) = pedir_con(
            "GET",
            "169.254.169.254:80",
            &format!(
                "/computeMetadata/v1/instance/service-accounts/default/identity?audience={}&format=full",
                self.audiencia
            ),
            &[("Metadata-Flavor", "Google")],
            None,
            Plazos {
                conectar: Duration::from_secs(2),
                responder: Duration::from_secs(5),
            },
        )?;
        let t = cuerpo.trim().to_string();
        if codigo != 200 || t.split('.').count() != 3 {
            return Err(format!(
                "el servidor de metadatos no dio un token ({codigo})"
            ));
        }
        if let Ok(mut g) = self.guardado.lock() {
            *g = Some((t.clone(), Instant::now()));
        }
        Ok(t)
    }
}

// ── puede ────────────────────────────────────────────────────────────────────

/// Sobre qué se pregunta. `organizacion` con id `-` es «la de la celda» (las
/// preguntas de P2); las de un recurso llevan su tipo y su nombre.
#[derive(Clone, Copy, Debug)]
pub struct Recurso<'a> {
    pub tipo: &'a str,
    pub id: &'a str,
}

impl Recurso<'static> {
    /// La organización de la celda.
    pub const ORGANIZACION: Recurso<'static> = Recurso {
        tipo: "organizacion",
        id: "-",
    };
}

/// Lo que contesta [`Acceso::puede`]. Todo lo que no es `Permite` niega.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Puede. `id` va dentro del `hizo` que siga.
    Permite { id: String },
    /// No puede, y lo que le falta (para la persona). La ruta contesta 403.
    Niega { id: String, motivo: String },
    /// No hubo quien decidiera: `ore-iam` no contestó a tiempo, contestó 5xx, o
    /// la celda no se pudo presentar. La ruta contesta **503**; nunca deja pasar.
    SinRespuesta { motivo: String },
    /// El token de la persona dejó de valer entre la puerta y la pregunta. 401.
    SujetoInvalido { motivo: String },
}

impl Decision {
    pub fn permite(&self) -> bool {
        matches!(self, Decision::Permite { .. })
    }

    /// El código HTTP que la ruta contesta si no deja pasar.
    pub fn codigo(&self) -> u16 {
        match self {
            Decision::Permite { .. } => 200,
            Decision::Niega { .. } => 403,
            Decision::SinRespuesta { .. } => 503,
            Decision::SujetoInvalido { .. } => 401,
        }
    }
}

type Clave = (String, String, String, String);

/// Por qué no hay handle ([`Acceso::quien`]).
#[derive(Clone, Debug, PartialEq)]
pub enum SinHandle {
    /// `ore-iam` dice que no es una persona de la organización (un agente, alguien
    /// de fuera). Nada nace a su nombre: 403.
    NoEsPersona(String),
    /// No hubo respuesta que valga: 503, y se reintenta.
    SinRespuesta(String),
}

// ── hizo ─────────────────────────────────────────────────────────────────────

/// Lo que un módulo cuenta que hizo (0047 § «`hizo`»).
#[derive(Clone, Debug)]
pub struct Evento {
    /// Lo pone quien emite; dos eventos con el mismo `id` son uno.
    pub id: String,
    pub operacion: String,
    pub sobre: String,
    /// `hecho`, `negado`, `fallido` o `en-curso`.
    pub resultado: String,
    /// El `id` de la decisión que lo dejó pasar. Sin él, un evento que no llega no
    /// se puede reintentar: `ore-iam` no sabría de quién es.
    pub decision: Option<String>,
    /// El commit que lleva la historia fina, si lo hay (H13: se apunta, no se copia).
    pub commit: Option<String>,
    /// Para el que cierra un `en-curso`: el `id` del que lo abrió.
    pub abre: Option<String>,
    pub detalle: Option<Json>,
}

impl Evento {
    fn json(&self) -> Json {
        let mut o = vec![
            ("id", Json::s(&self.id)),
            ("operacion", Json::s(&self.operacion)),
            ("sobre", Json::s(&self.sobre)),
            ("resultado", Json::s(&self.resultado)),
        ];
        for (k, v) in [
            ("decision", &self.decision),
            ("commit", &self.commit),
            ("abre", &self.abre),
        ] {
            if let Some(v) = v {
                o.push((k, Json::s(v)));
            }
        }
        if let Some(d) = &self.detalle {
            o.push(("detalle", d.clone()));
        }
        Json::obj(o)
    }
}

/// Lo que pasó con un `hizo`.
#[derive(Clone, Debug, PartialEq)]
pub enum Hecho {
    Anotado,
    /// Ya estaba: el mismo `id` llegó antes.
    YaEstaba,
    /// `ore-iam` no contestó y el evento espera en disco, con su motivo.
    Encolado(String),
}

/// Un identificador para un evento: único en este proceso y entre procesos (el
/// reloj en nanosegundos, el `pid` y un contador).
pub fn nuevo_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "ev-{t:x}-{:x}-{:x}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

// ── el cliente ───────────────────────────────────────────────────────────────

pub struct Acceso {
    destino: String,
    credencial: Box<dyn Credencial>,
    plazos: Plazos,
    cache: Mutex<HashMap<Clave, (Decision, Instant)>>,
    /// Los handles ya preguntados, por `sub`. No caducan: `ore-iam` no los cambia.
    handles: Mutex<HashMap<String, String>>,
    /// Donde esperan los eventos que no llegaron. Sin directorio, no se encolan.
    pendientes: Option<PathBuf>,
}

impl Acceso {
    pub fn nuevo(destino: &str, credencial: Box<dyn Credencial>) -> Acceso {
        Acceso {
            destino: destino.to_string(),
            credencial,
            plazos: Plazos {
                conectar: PLAZO,
                responder: PLAZO,
            },
            cache: Mutex::new(HashMap::new()),
            handles: Mutex::new(HashMap::new()),
            pendientes: None,
        }
    }

    /// El plazo para `ore-iam`, si no es el de siempre.
    pub fn con_plazo(mut self, plazo: Duration) -> Acceso {
        self.plazos = Plazos {
            conectar: plazo,
            responder: plazo,
        };
        self
    }

    /// El directorio donde esperan los eventos que no llegaron.
    pub fn con_pendientes(mut self, dir: PathBuf) -> Acceso {
        self.pendientes = Some(dir);
        self
    }

    fn pedir(
        &self,
        camino: &str,
        sujeto: Option<&str>,
        cuerpo: &Json,
    ) -> Result<(u16, String), String> {
        let celda = self
            .credencial
            .token()
            .map_err(|e| format!("la celda no se pudo presentar: {e}"))?;
        let autorizacion = format!("Bearer {celda}");
        let mut cabeceras = vec![("Authorization", autorizacion.as_str())];
        if let Some(s) = sujeto {
            cabeceras.push(("Ore-Sujeto", s));
        }
        pedir_con(
            "POST",
            &self.destino,
            camino,
            &cabeceras,
            Some(cuerpo),
            self.plazos,
        )
    }

    /// ¿Puede `sujeto_id` hacer `accion` sobre `recurso`? `sujeto_token` es el token
    /// del realm con el que llegó a este módulo, tal cual (sin `Bearer `).
    pub fn puede(
        &self,
        sujeto_token: &str,
        sujeto_id: &str,
        accion: &str,
        recurso: Recurso,
        ruta: &str,
    ) -> Decision {
        self.preguntar(sujeto_token, sujeto_id, accion, recurso, ruta, false)
    }

    /// «¿Podría?»: lo mismo que [`Acceso::puede`], para que una pantalla sepa qué
    /// enseñar. Una denegación no va a la huella —nadie intentó nada— y un permiso
    /// no deja pasar ningún acto (su `id` no vale para `hizo`).
    pub fn podria(
        &self,
        sujeto_token: &str,
        sujeto_id: &str,
        accion: &str,
        recurso: Recurso,
        ruta: &str,
    ) -> Decision {
        self.preguntar(sujeto_token, sujeto_id, accion, recurso, ruta, true)
    }

    fn preguntar(
        &self,
        sujeto_token: &str,
        sujeto_id: &str,
        accion: &str,
        recurso: Recurso,
        ruta: &str,
        consulta: bool,
    ) -> Decision {
        let clave: Clave = (
            sujeto_id.to_string(),
            format!("{accion}{}", if consulta { "?" } else { "" }),
            recurso.tipo.to_string(),
            recurso.id.to_string(),
        );
        if let Ok(c) = self.cache.lock()
            && let Some((d, hasta)) = c.get(&clave)
            && Instant::now() < *hasta
        {
            return d.clone();
        }
        let cuerpo = Json::obj([
            (
                "subject",
                Json::obj([("type", Json::s("persona")), ("id", Json::s(sujeto_id))]),
            ),
            ("action", Json::obj([("name", Json::s(accion))])),
            (
                "resource",
                Json::obj([("type", Json::s(recurso.tipo)), ("id", Json::s(recurso.id))]),
            ),
            (
                "context",
                Json::obj([("ruta", Json::s(ruta)), ("consulta", Json::Bool(consulta))]),
            ),
        ]);
        let (codigo, texto) = match self.pedir("/access/v1/evaluation", Some(sujeto_token), &cuerpo)
        {
            Ok(r) => r,
            Err(e) => {
                return Decision::SinRespuesta {
                    motivo: format!("no hay quien decida: {e}"),
                };
            }
        };
        let n = parse::parse(&texto).ok();
        let campo = |k: &str| n.as_ref().and_then(|n| texto_de(n, &[k]));
        match codigo {
            200 => {}
            401 if campo("error").is_some_and(|e| e.contains("Ore-Sujeto")) => {
                return Decision::SujetoInvalido {
                    motivo: campo("error").unwrap_or_default(),
                };
            }
            c => {
                return Decision::SinRespuesta {
                    motivo: format!(
                        "no hay quien decida: `ore-iam` contestó {c}{}",
                        campo("error")
                            .map(|e| format!(" · {e}"))
                            .unwrap_or_default()
                    ),
                };
            }
        }
        let Some(n) = n else {
            return Decision::SinRespuesta {
                motivo: "no hay quien decida: la respuesta de `ore-iam` no analiza".into(),
            };
        };
        let id = texto_de(&n, &["context", "id"]).unwrap_or_default();
        let d = match texto_de(&n, &["decision"]).as_deref() {
            Some("true") => Decision::Permite { id },
            Some("false") => Decision::Niega {
                id,
                motivo: texto_de(&n, &["context", "motivo"]).unwrap_or_else(|| "no puedes".into()),
            },
            _ => {
                return Decision::SinRespuesta {
                    motivo: "no hay quien decida: la respuesta no trae `decision`".into(),
                };
            }
        };
        let vale = texto_de(&n, &["context", "vale"])
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            .min(VALE_MAXIMO);
        if vale > 0
            && let Ok(mut c) = self.cache.lock()
        {
            if c.len() >= CACHE_MAXIMA {
                c.clear();
            }
            c.insert(
                clave,
                (d.clone(), Instant::now() + Duration::from_secs(vale)),
            );
        }
        d
    }

    /// **El handle de una persona** (`POST /access/v1/quien`): lo que va detrás de
    /// `user:` en el `owner` de lo que crea. `sujeto_token` es el token con el que
    /// llegó quien pide —la persona, o el agente de su puesto—; `sub`, la persona.
    ///
    /// `Err` si `ore-iam` no contesta o dice que no es una persona de la
    /// organización: lo que se iba a crear no nace con un dueño inventado.
    pub fn quien(&self, sujeto_token: Option<&str>, sub: &str) -> Result<String, SinHandle> {
        if let Ok(h) = self.handles.lock()
            && let Some(h) = h.get(sub)
        {
            return Ok(h.clone());
        }
        let cuerpo = Json::obj([("subject", Json::obj([("id", Json::s(sub))]))]);
        let (codigo, texto) = self
            .pedir("/access/v1/quien", sujeto_token, &cuerpo)
            .map_err(|e| {
                SinHandle::SinRespuesta(format!(
                    "no se pudo preguntar a `ore-iam` quién es `{sub}`: {e}"
                ))
            })?;
        let n = parse::parse(&texto).ok();
        let campo = |k: &str| n.as_ref().and_then(|n| texto_de(n, &[k]));
        let h = match (codigo, campo("handle")) {
            (200, Some(h)) if ore_core::pertenencia::es_handle(&format!("user:{h}")) => h,
            (c, _) => {
                let m = format!(
                    "`ore-iam` no dio el handle de `{sub}` ({c}){}",
                    campo("error")
                        .map(|e| format!(" · {e}"))
                        .unwrap_or_default()
                );
                return Err(if c == 404 {
                    SinHandle::NoEsPersona(m)
                } else {
                    SinHandle::SinRespuesta(m)
                });
            }
        };
        if let Ok(mut m) = self.handles.lock() {
            if m.len() >= CACHE_MAXIMA {
                m.clear();
            }
            m.insert(sub.to_string(), h.clone());
        }
        Ok(h)
    }

    /// Lo que se hizo, DESPUÉS de hacerlo. No hace fallar lo hecho: si `ore-iam` no
    /// contesta, el evento espera en disco (si trae `decision` y hay directorio) y
    /// se reintenta. Sólo es `Err` si no se pudo ni anotar ni encolar.
    pub fn hizo(&self, sujeto_token: Option<&str>, e: &Evento) -> Result<Hecho, String> {
        match self.enviar(sujeto_token, e) {
            Ok(h) => Ok(h),
            Err(motivo) => self.encolar(e, &motivo),
        }
    }

    /// Lo que se va a hacer y no puede quedarse sin rastro, ANTES de hacerlo. Si no
    /// se puede anotar, `Err`: la ruta contesta 503 y no actúa.
    pub fn hizo_antes(&self, sujeto_token: Option<&str>, e: &Evento) -> Result<Hecho, String> {
        self.enviar(sujeto_token, e)
    }

    fn enviar(&self, sujeto_token: Option<&str>, e: &Evento) -> Result<Hecho, String> {
        self.enviar_con_motivo(sujeto_token, e).map_err(|(m, _)| m)
    }

    /// Como `enviar`, y el error dice si fue un RECHAZO (un 4xx: reintentar no
    /// cambia nada) o que no hubo respuesta (reintentar, sí).
    fn enviar_con_motivo(
        &self,
        sujeto_token: Option<&str>,
        e: &Evento,
    ) -> Result<Hecho, (String, bool)> {
        let (codigo, texto) = self
            .pedir("/access/v1/eventos", sujeto_token, &e.json())
            .map_err(|m| (m, false))?;
        match codigo {
            201 => Ok(Hecho::Anotado),
            200 => Ok(Hecho::YaEstaba),
            c => Err((
                format!(
                    "`ore-iam` contestó {c}{}",
                    parse::parse(&texto)
                        .ok()
                        .and_then(|n| texto_de(&n, &["error"]))
                        .map(|e| format!(" · {e}"))
                        .unwrap_or_default()
                ),
                (400..500).contains(&c),
            )),
        }
    }

    /// A `muertos/`, sin token: el evento y por qué no llegó. Lo hecho sigue en su
    /// commit (H13); esto es el índice que falta, para quien lo quiera rehacer.
    fn enterrar(&self, e: &Evento, motivo: &str) -> Result<PathBuf, String> {
        let Some(dir) = &self.pendientes else {
            return Err(format!("{motivo}; y no hay directorio de pendientes"));
        };
        enterrar_en(dir, e, motivo)
    }

    fn encolar(&self, e: &Evento, motivo: &str) -> Result<Hecho, String> {
        let Some(dir) = &self.pendientes else {
            return Err(format!("{motivo}; y no hay directorio de pendientes"));
        };
        if e.decision.is_none() {
            return Err(format!(
                "{motivo}; y sin `decision` no se puede reintentar: `ore-iam` no sabría de quién es"
            ));
        }
        std::fs::create_dir_all(dir)
            .map_err(|x| format!("{motivo}; y no se pudo crear `{}`: {x}", dir.display()))?;
        let f = dir.join(format!("{}.json", e.id));
        std::fs::write(&f, e.json().jcs())
            .map_err(|x| format!("{motivo}; y no se pudo encolar: {x}"))?;
        Ok(Hecho::Encolado(motivo.to_string()))
    }

    /// Manda los eventos que esperan, sin `Ore-Sujeto`: el sujeto sale de su
    /// decisión. Devuelve `(enviados, siguen)`. Los que `ore-iam` rechaza con 400
    /// —la decisión ya no está viva— pasan a `muertos/`, y se dice: no se pierden
    /// en silencio ni se reintentan para siempre.
    pub fn reintentar(&self) -> (usize, usize) {
        let Some(dir) = &self.pendientes else {
            return (0, 0);
        };
        let Ok(entradas) = std::fs::read_dir(dir) else {
            return (0, 0);
        };
        let (mut enviados, mut siguen) = (0, 0);
        for f in entradas
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
        {
            let Ok(texto) = std::fs::read_to_string(&f) else {
                continue;
            };
            let Ok(n) = parse::parse(&texto) else {
                continue;
            };
            let cuerpo = Json::de_node(&n);
            match self.pedir("/access/v1/eventos", None, &cuerpo) {
                Ok((200 | 201, _)) => {
                    let _ = std::fs::remove_file(&f);
                    enviados += 1;
                }
                Ok((400, _)) => {
                    let muertos = dir.join("muertos");
                    let _ = std::fs::create_dir_all(&muertos);
                    if let Some(nombre) = f.file_name() {
                        let _ = std::fs::rename(&f, muertos.join(nombre));
                    }
                }
                _ => siguen += 1,
            }
        }
        (enviados, siguen)
    }
}

fn enterrar_en(dir: &Path, e: &Evento, motivo: &str) -> Result<PathBuf, String> {
    let muertos = dir.join("muertos");
    std::fs::create_dir_all(&muertos)
        .map_err(|x| format!("{motivo}; y no se pudo crear `{}`: {x}", muertos.display()))?;
    let f = muertos.join(format!("{}.json", e.id));
    let cuerpo = Json::obj([("evento", e.json()), ("motivo", Json::s(motivo))]);
    std::fs::write(&f, cuerpo.jcs())
        .map_err(|x| format!("{motivo}; y no se pudo enterrar: {x}"))?;
    Ok(f)
}

// ── el buzón (0047 A6.4) ─────────────────────────────────────────────────────

/// Cuánto vive el token de una persona en el buzón: el realm los emite por 300 s,
/// y uno que llegó justo al caducar no sirve a los cinco minutos.
pub const VIDA_DEL_SUJETO: Duration = Duration::from_secs(240);

/// Cuántos eventos caben esperando. Pasado el techo, el que llega va a `muertos/`:
/// un `ore-iam` caído no puede comerse la memoria de la celda.
const CABEN: usize = 10_000;

/// Un evento esperando, con el token con el que llegó y cuándo.
struct Carta {
    token: Option<String>,
    evento: Evento,
    desde: Instant,
    intentos: u32,
    siguiente: Instant,
}

/// **Echar un evento sin esperar a `ore-iam`** (0047 A6.4). Un hilo lo manda; si
/// no hay respuesta, lo reintenta mientras el token valga ([`VIDA_DEL_SUJETO`]),
/// cada vez más espaciado. Pasado eso, si tiene decisión espera en disco como
/// los de [`Acceso::hizo`]; si no, a `muertos/`. Un rechazo (4xx) va a `muertos/`
/// en el acto: reintentarlo no lo arregla.
///
/// ⛔ El token de la persona **no toca el disco**: guardarlo para reintentar
///   sería guardar una credencial.
#[derive(Clone)]
pub struct Buzon {
    cola: SyncSender<Carta>,
    acceso: Arc<Acceso>,
}

impl Buzon {
    pub fn nuevo(acceso: Arc<Acceso>) -> Buzon {
        Buzon::con_vida(acceso, VIDA_DEL_SUJETO)
    }

    /// Con otra vida para el token: para las pruebas.
    pub fn con_vida(acceso: Arc<Acceso>, vida: Duration) -> Buzon {
        let (cola, llegan) = sync_channel::<Carta>(CABEN);
        let a = Arc::clone(&acceso);
        std::thread::spawn(move || cartero(&a, llegan, vida));
        Buzon { cola, acceso }
    }

    /// Echa el evento y vuelve en el acto.
    pub fn echar(&self, token: Option<String>, evento: Evento) {
        let ahora = Instant::now();
        let carta = Carta {
            token,
            evento,
            desde: ahora,
            intentos: 0,
            siguiente: ahora,
        };
        match self.cola.try_send(carta) {
            Ok(()) => {}
            Err(TrySendError::Full(c) | TrySendError::Disconnected(c)) => {
                muerto(&self.acceso, &c.evento, "el buzón está lleno");
            }
        }
    }
}

fn muerto(acceso: &Acceso, e: &Evento, motivo: &str) {
    match acceso.enterrar(e, motivo) {
        Ok(f) => eprintln!(
            "acceso · ✗ `{}` sin huella: {motivo} (en {})",
            e.operacion,
            f.display()
        ),
        Err(m) => eprintln!("acceso · ✗ `{}` sin huella: {m}", e.operacion),
    }
}

/// El hilo del buzón: lo que llega, se manda; lo que no pudo, espera su turno.
fn cartero(acceso: &Acceso, llegan: Receiver<Carta>, vida: Duration) {
    let mut esperan: VecDeque<Carta> = VecDeque::new();
    loop {
        let plazo = esperan
            .iter()
            .map(|c| c.siguiente)
            .min()
            .map(|t| t.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(3600));
        match llegan.recv_timeout(plazo) {
            Ok(c) => intentar(acceso, c, vida, &mut esperan),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if esperan.is_empty() => return,
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(plazo.min(Duration::from_secs(1)));
            }
        }
        let ahora = Instant::now();
        let (toca, no): (Vec<Carta>, Vec<Carta>) =
            esperan.drain(..).partition(|c| c.siguiente <= ahora);
        esperan.extend(no);
        for c in toca {
            intentar(acceso, c, vida, &mut esperan);
        }
    }
}

fn intentar(acceso: &Acceso, mut c: Carta, vida: Duration, esperan: &mut VecDeque<Carta>) {
    // Pasada su vida, el token ya no vale: se manda sin él (vale si tiene decisión).
    let vivo = c.desde.elapsed() < vida;
    let token = if vivo { c.token.as_deref() } else { None };
    match acceso.enviar_con_motivo(token, &c.evento) {
        Ok(_) => {}
        Err((m, true)) => muerto(acceso, &c.evento, &m),
        Err((m, false)) if vivo => {
            c.intentos += 1;
            // 2, 4, 8, 16 segundos, y como mucho 30.
            let espera = Duration::from_secs((1u64 << c.intentos.min(5)).min(30));
            c.siguiente = Instant::now() + espera.min(vida / 4);
            if c.intentos == 1 {
                eprintln!("acceso · `{}` espera: {m}", c.evento.operacion);
            }
            esperan.push_back(c);
        }
        Err((m, false)) => match acceso.encolar(&c.evento, &m) {
            Ok(_) => eprintln!(
                "acceso · `{}` espera en disco (con su decisión): {m}",
                c.evento.operacion
            ),
            Err(_) => muerto(
                acceso,
                &c.evento,
                &format!("{m}; el token de quien lo hizo ya no vale y no hay decisión"),
            ),
        },
    }
}

/// Un texto dentro de un JSON, por su camino.
fn texto_de(n: &Node, camino: &[&str]) -> Option<String> {
    let mut actual = n;
    for k in camino {
        actual = actual.get(k).map(|(_, v)| v)?;
    }
    actual.as_str().map(str::to_string)
}

#[cfg(test)]
mod pruebas;
