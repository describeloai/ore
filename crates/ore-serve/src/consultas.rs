//! **Las consultas de Query** (`docs/query.md` §3.2): la admisión (Q3) y la cola
//! de los trabajadores DuckDB de `ore-consulta` (Q4).
//!
//! - **`POST /consultas {texto, rama?, limite?}`** decide antes de correr en qué
//!   clase va (L, G o E), por qué y con cuánta memoria e hilos, por lo que va a
//!   leer (`ore explain --estimate`, en la rama: el reparto y, del lago, sus
//!   manifiestos). Con `--consultas` además **la corre**: resuelve los nombres en
//!   la rama como lo hace el motor (`fuentes_de_sql` + `datos_en`: cada dataset
//!   con su credencial prestada y acotada a él), la encola y espera el resultado
//!   hasta [`ESPERA_DE_QUIEN_PREGUNTA`]; si tarda más, `202 {id}`.
//! - **`GET /consultas/{id}`**: cómo va, y el resultado. Sólo quien la lanzó.
//! - **`GET /consultas/trabajo`**: un trabajador pide trabajo **por espera
//!   larga**, diciendo cuánto le cabe (`x-ore-libre-mb`, `x-ore-libres-hilos`).
//!   Vuelve en cuanto hay una consulta que le quepa, o con 204 a los
//!   [`ESPERA_DEL_TRABAJADOR`]. El trabajador no acepta conexiones, como el puesto;
//!   y como pide lo que le cabe, el reparto entre trabajadores sale solo.
//! - **`POST /consultas/{id}/resultado`**: lo que corrió (el Arrow, en base64).
//!
//! Lo que el trabajador recibe lleva las credenciales de esa consulta: salen de
//! aquí al tomarla y no se guardan. Lo que todavía no corre en un trabajador
//! —lo que se lee en vivo, una colección, una Function del árbol— se dice con
//! `409 consulta/en-el-puesto`, y sigue en el puesto; E llega en Q7 y, hasta
//! entonces, corre en G con un aviso.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use ore_core::admision::{self, Clase, Umbrales};
use ore_core::escalado::{self, Foto, Hueco};
use ore_core::json::Json;

use crate::rutas::Servidor;
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::Identidad;

/// Lo que `POST /consultas` espera al resultado antes de contestar `202 {id}`.
/// Por debajo del plazo de la consola; una plaza de conexión mientras tanto.
pub const ESPERA_DE_QUIEN_PREGUNTA: Duration = Duration::from_secs(25);
/// La espera larga de un trabajador sin trabajo: vuelve a pedir al acabar.
pub const ESPERA_DEL_TRABAJADOR: Duration = Duration::from_secs(25);
/// Una consulta encolada que ningún trabajador toma en este tiempo falla y lo
/// dice: no hay un «preparando cómputo» (§3.1); si pasa, falta capacidad (Q5).
pub const SIN_TRABAJADOR: Duration = Duration::from_secs(60);
/// Una consulta tomada cuyo resultado no llega (el trabajador murió).
pub const SIN_RESULTADO: Duration = Duration::from_secs(15 * 60);
/// Lo terminado se guarda para quien pregunta por su `id`, y luego se olvida.
pub const SE_GUARDA: Duration = Duration::from_secs(10 * 60);
/// Las que caben a la vez en la cola, encoladas o corriendo.
pub const EN_LA_COLA: usize = 256;
/// Q5·3 · Con un trabajador de quita y pon en camino, lo encolado le espera
/// hasta aquí antes de fallar por falta de capacidad (un nodo frío: ~100 s).
pub const SIN_TRABAJADOR_CON_PEDIDO: Duration = Duration::from_secs(180);
/// Q5·3 · Un trabajador está vivo si pidió hace menos que esto (la espera larga
/// es de 25 s) o corre algo.
pub const VIVO: Duration = Duration::from_secs(60);
/// Q5·3 · Un trabajador pedido que no pide en este tiempo no llegó: se retira.
pub const ARRANQUE_MAXIMO: Duration = Duration::from_secs(5 * 60);
/// Q5·3 · Hay actividad si se encoló o se entregó algo hace menos que esto.
pub const ACTIVIDAD: Duration = Duration::from_secs(5 * 60);
/// Q5·3 · Cada cuánto se mira la regla aunque nada la despierte.
pub const CADENCIA_DEL_ESCALADO: Duration = Duration::from_secs(5);
/// Las filas que vuelven: hasta Q6 (paginar), sólo las primeras.
pub const LIMITE: u64 = 1000;
pub const LIMITE_MAXIMO: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fase {
    Encolada,
    Corriendo,
    Hecha,
    Fallida,
}

impl Fase {
    pub fn nombre(self) -> &'static str {
        match self {
            Fase::Encolada => "encolada",
            Fase::Corriendo => "corriendo",
            Fase::Hecha => "hecha",
            Fase::Fallida => "fallida",
        }
    }
}

struct Consulta {
    dueno: String,
    memoria_mb: u64,
    hilos: u64,
    /// Lo que se le da al trabajador: se vacía al tomarla (lleva credenciales).
    trabajo: Option<Json>,
    fase: Fase,
    creada: Instant,
    tomada: Option<Instant>,
    terminada: Option<Instant>,
    trabajador: Option<String>,
    resultado: Option<Json>,
}

#[derive(Default)]
struct Estado {
    pendientes: VecDeque<String>,
    consultas: BTreeMap<String, Consulta>,
    siguiente: u64,
    /// Q5·3 · Los trabajadores que piden (`persona#pod`) y lo que les cabe.
    trabajadores: BTreeMap<String, Visto>,
    /// Q5·3 · Los de quita y pon pedidos a la cola, por el nombre de su Job.
    pedidos: BTreeMap<String, Pedido>,
    actividad: Option<Instant>,
    decision: Option<escalado::Decision>,
}

/// Q5·3 · Lo que un trabajador dijo al pedir, menos lo que tomó desde entonces.
struct Visto {
    ultima: Instant,
    libre: Hueco,
}

/// Q5·3 · Un trabajador de quita y pon en la cola: su fichero, desde cuándo, y
/// quién es (`persona#pod`) en cuanto pide.
struct Pedido {
    fichero: String,
    desde: Instant,
    llego: Option<String>,
}

/// **La cola de `ore-consulta`**, en memoria: un `ore-serve` por celda.
#[derive(Default)]
pub struct Cola {
    /// `--consultas`: correrlas en los trabajadores. Sin él, sólo se admite.
    pub activa: bool,
    /// `--consultas-techo` (Q5·3): los trabajadores que la celda puede tener a
    /// la vez, contando el suelo. 1 (o 0) = sólo el suelo: no se pide ninguno.
    pub techo: u64,
    estado: Mutex<Estado>,
    aviso: Condvar,
    /// Lo que despierta a la regla de escalado antes de su cadencia.
    toque: (Mutex<bool>, Condvar),
}

/// Cómo está una consulta, para quien la lanzó.
#[derive(Debug, Clone)]
pub struct Vista {
    pub fase: Fase,
    pub cola_ms: Option<u64>,
    pub correr_ms: Option<u64>,
    pub resultado: Option<Json>,
}

impl Cola {
    pub fn nueva(activa: bool, techo: u64) -> Cola {
        Cola {
            activa,
            techo,
            ..Default::default()
        }
    }

    /// Si hay que mirar la regla de escalado: con `--consultas` y un techo por
    /// encima del suelo.
    pub fn escala(&self) -> bool {
        self.activa && self.techo > 1
    }

    /// Que la regla mire ya (algo se encoló).
    fn tocar(&self) {
        *self.toque.0.lock().unwrap() = true;
        self.toque.1.notify_all();
    }

    /// Lo que espera el hilo de la regla: un toque, o la cadencia.
    pub fn esperar_toque(&self, cadencia: Duration) {
        let g = self.toque.0.lock().unwrap();
        let (mut g, _) = self
            .toque
            .1
            .wait_timeout_while(g, cadencia, |tocado| !*tocado)
            .unwrap();
        *g = false;
    }

    /// Q5·3 · Lo que se sabe ahora, para la regla, y los de quita y pon que ya
    /// se fueron (o no llegaron), que se dejan de contar y se retiran de la cola:
    /// `(foto, [(job, fichero)])`.
    pub fn foto(&self) -> (Foto, Vec<(String, String)>) {
        let mut e = self.estado.lock().unwrap();
        let ahora = Instant::now();
        barrer(&mut e, ahora);
        let vivo = |e: &Estado, quien: &str| {
            e.trabajadores
                .get(quien)
                .is_some_and(|v| ahora.duration_since(v.ultima) < VIVO)
                || e.consultas
                    .values()
                    .any(|c| c.fase == Fase::Corriendo && c.trabajador.as_deref() == Some(quien))
        };
        let mut retirar = Vec::new();
        for (job, p) in &e.pedidos {
            let fuera = match &p.llego {
                Some(q) => !vivo(&e, q),
                None => ahora.duration_since(p.desde) > ARRANQUE_MAXIMO,
            };
            if fuera {
                retirar.push((job.clone(), p.fichero.clone()));
            }
        }
        for (job, _) in &retirar {
            e.pedidos.remove(job);
        }
        let quienes: Vec<String> = e.trabajadores.keys().cloned().collect();
        let mut vivos = Vec::new();
        for q in quienes {
            if vivo(&e, &q) {
                vivos.push(e.trabajadores[&q].libre);
            } else if e
                .trabajadores
                .get(&q)
                .is_some_and(|v| ahora.duration_since(v.ultima) > ARRANQUE_MAXIMO)
            {
                e.trabajadores.remove(&q);
            }
        }
        let foto = Foto {
            vivos,
            en_camino: e.pedidos.values().filter(|p| p.llego.is_none()).count() as u64,
            encoladas: e
                .pendientes
                .iter()
                .filter_map(|id| e.consultas.get(id))
                .map(|c| Hueco {
                    mb: c.memoria_mb,
                    hilos: c.hilos,
                })
                .collect(),
            activa: e
                .actividad
                .is_some_and(|t| ahora.duration_since(t) < ACTIVIDAD),
        };
        (foto, retirar)
    }

    /// Q5·3 · Lo que la regla decidió, y los Jobs que se encolaron por ello.
    pub fn anotar(&self, decision: escalado::Decision, pedidos: Vec<(String, String)>) {
        let mut e = self.estado.lock().unwrap();
        let ahora = Instant::now();
        for (job, fichero) in pedidos {
            e.pedidos.insert(
                job,
                Pedido {
                    fichero,
                    desde: ahora,
                    llego: None,
                },
            );
        }
        e.decision = Some(decision);
    }

    /// Q5·3 · `GET /consultas/capacidad`: lo admitido, lo libre y lo pedido.
    pub fn capacidad(&self) -> Json {
        let (foto, _) = self.foto();
        let e = self.estado.lock().unwrap();
        let ahora = Instant::now();
        let suma = |hs: &[Hueco]| {
            Json::obj([
                ("mb", Json::Int(hs.iter().map(|h| h.mb).sum::<u64>() as i64)),
                (
                    "hilos",
                    Json::Int(hs.iter().map(|h| h.hilos).sum::<u64>() as i64),
                ),
            ])
        };
        let corriendo: Vec<Hueco> = e
            .consultas
            .values()
            .filter(|c| c.fase == Fase::Corriendo)
            .map(|c| Hueco {
                mb: c.memoria_mb,
                hilos: c.hilos,
            })
            .collect();
        Json::obj([
            ("techo", Json::Int(self.techo as i64)),
            ("trabajadores", Json::Int(foto.vivos.len() as i64)),
            ("libre", suma(&foto.vivos)),
            ("corriendo", Json::Int(corriendo.len() as i64)),
            ("admitido", suma(&corriendo)),
            ("encoladas", Json::Int(foto.encoladas.len() as i64)),
            ("encolado", suma(&foto.encoladas)),
            ("activa", Json::Bool(foto.activa)),
            (
                "pedidos",
                Json::Arr(
                    e.pedidos
                        .iter()
                        .map(|(job, p)| {
                            Json::obj([
                                ("job", Json::s(job)),
                                ("llego", Json::Bool(p.llego.is_some())),
                                (
                                    "hace_s",
                                    Json::Int(ahora.duration_since(p.desde).as_secs() as i64),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "decision",
                match &e.decision {
                    Some(d) => Json::obj([
                        ("pedir", Json::Int(d.pedir as i64)),
                        ("razon", Json::s(d.razon.as_str())),
                    ]),
                    None => Json::obj([]),
                },
            ),
        ])
    }

    /// Encola un trabajo (sin `id`: se lo pone aquí). `Err` si la cola está llena.
    pub fn encolar(
        &self,
        dueno: &str,
        memoria_mb: u64,
        hilos: u64,
        trabajo: Json,
    ) -> Result<String, String> {
        let mut e = self.estado.lock().unwrap();
        barrer(&mut e, Instant::now());
        let vivas = e
            .consultas
            .values()
            .filter(|c| matches!(c.fase, Fase::Encolada | Fase::Corriendo))
            .count();
        if vivas >= EN_LA_COLA {
            return Err(format!(
                "hay {vivas} consultas en la cola de esta celda: no cabe otra"
            ));
        }
        e.siguiente += 1;
        let id = format!("q{}-{}", e.siguiente, sal());
        let mut trabajo = trabajo;
        if let Json::Obj(m) = &mut trabajo {
            m.insert("id".into(), Json::s(&id));
        }
        e.consultas.insert(
            id.clone(),
            Consulta {
                dueno: dueno.to_string(),
                memoria_mb,
                hilos,
                trabajo: Some(trabajo),
                fase: Fase::Encolada,
                creada: Instant::now(),
                tomada: None,
                terminada: None,
                trabajador: None,
                resultado: None,
            },
        );
        e.pendientes.push_back(id.clone());
        e.actividad = Some(Instant::now());
        drop(e);
        self.aviso.notify_all();
        self.tocar();
        Ok(id)
    }

    /// Un trabajador con `libre_mb` y `hilos` libres toma la primera consulta
    /// encolada que le cabe, esperando hasta `espera`. `None`: nada en ese rato.
    pub fn tomar(
        &self,
        trabajador: &str,
        libre_mb: u64,
        hilos: u64,
        espera: Duration,
    ) -> Option<Json> {
        let hasta = Instant::now() + espera;
        let mut e = self.estado.lock().unwrap();
        // Q5·3 · Quién pide y cuánto le cabe; y si es un pedido, que llegó.
        e.trabajadores.insert(
            trabajador.to_string(),
            Visto {
                ultima: Instant::now(),
                libre: Hueco {
                    mb: libre_mb,
                    hilos,
                },
            },
        );
        let pod = trabajador.rsplit('#').next().unwrap_or("");
        for (job, p) in e.pedidos.iter_mut() {
            if p.llego.is_none() && pod.starts_with(&format!("{job}-")) {
                p.llego = Some(trabajador.to_string());
            }
        }
        loop {
            let ahora = Instant::now();
            barrer(&mut e, ahora);
            let cabe = e.pendientes.iter().position(|id| {
                e.consultas
                    .get(id)
                    .is_some_and(|c| c.memoria_mb <= libre_mb && c.hilos <= hilos)
            });
            if let Some(i) = cabe {
                let id = e.pendientes.remove(i).unwrap();
                let c = e.consultas.get_mut(&id).unwrap();
                c.fase = Fase::Corriendo;
                c.tomada = Some(ahora);
                c.trabajador = Some(trabajador.to_string());
                let (mb, h, t) = (c.memoria_mb, c.hilos, c.trabajo.take());
                if let Some(v) = e.trabajadores.get_mut(trabajador) {
                    v.libre.mb = v.libre.mb.saturating_sub(mb);
                    v.libre.hilos = v.libre.hilos.saturating_sub(h);
                    v.ultima = ahora;
                }
                return t;
            }
            if ahora >= hasta {
                return None;
            }
            e = self.aviso.wait_timeout(e, hasta - ahora).unwrap().0;
        }
    }

    /// Lo que corrió. Sólo la consulta que ese trabajador tomó, y una vez.
    pub fn entregar(
        &self,
        trabajador: &str,
        id: &str,
        bien: bool,
        resultado: Json,
    ) -> Result<(), String> {
        let mut e = self.estado.lock().unwrap();
        let Some(c) = e.consultas.get_mut(id) else {
            return Err(format!("no hay ninguna consulta `{id}` (¿se olvidó?)"));
        };
        if c.fase != Fase::Corriendo || c.trabajador.as_deref() != Some(trabajador) {
            return Err(format!(
                "`{id}` no la está corriendo `{trabajador}` (está {})",
                c.fase.nombre()
            ));
        }
        c.fase = if bien { Fase::Hecha } else { Fase::Fallida };
        c.terminada = Some(Instant::now());
        c.resultado = Some(resultado);
        e.actividad = Some(Instant::now());
        drop(e);
        self.aviso.notify_all();
        Ok(())
    }

    /// Cómo está `id`, para `dueno`, esperando hasta `espera` a que termine.
    /// `None` si no existe o no es suya: lo mismo, a propósito.
    pub fn esperar(&self, id: &str, dueno: &str, espera: Duration) -> Option<Vista> {
        let hasta = Instant::now() + espera;
        let mut e = self.estado.lock().unwrap();
        loop {
            let ahora = Instant::now();
            barrer(&mut e, ahora);
            let c = e.consultas.get(id).filter(|c| c.dueno == dueno)?;
            if matches!(c.fase, Fase::Hecha | Fase::Fallida) || ahora >= hasta {
                let ms = |a: Option<Instant>, b: Option<Instant>| match (a, b) {
                    (Some(a), Some(b)) => Some(b.saturating_duration_since(a).as_millis() as u64),
                    _ => None,
                };
                return Some(Vista {
                    fase: c.fase,
                    cola_ms: ms(Some(c.creada), c.tomada),
                    correr_ms: ms(c.tomada, c.terminada),
                    resultado: c.resultado.clone(),
                });
            }
            e = self.aviso.wait_timeout(e, hasta - ahora).unwrap().0;
        }
    }
}

/// Lo que nadie toma, lo que no vuelve y lo ya visto, fuera.
fn barrer(e: &mut Estado, ahora: Instant) {
    let mut caducadas = Vec::new();
    // Q5·3 · Con un trabajador en camino, lo encolado le espera un poco más.
    let espera = if e.pedidos.values().any(|p| p.llego.is_none()) {
        SIN_TRABAJADOR_CON_PEDIDO
    } else {
        SIN_TRABAJADOR
    };
    for (id, c) in e.consultas.iter_mut() {
        let (fallo, codigo) = match c.fase {
            Fase::Encolada if ahora.duration_since(c.creada) > espera => (
                format!(
                    "ningún trabajador de Query la tomó en {} s: a esta celda le falta capacidad",
                    espera.as_secs()
                ),
                "consulta/sin-trabajador",
            ),
            Fase::Corriendo
                if c.tomada
                    .is_some_and(|t| ahora.duration_since(t) > SIN_RESULTADO) =>
            {
                (
                    format!(
                        "el trabajador `{}` no devolvió nada en {} min",
                        c.trabajador.as_deref().unwrap_or("?"),
                        SIN_RESULTADO.as_secs() / 60
                    ),
                    "consulta/sin-resultado",
                )
            }
            Fase::Hecha | Fase::Fallida => {
                if c.terminada
                    .is_some_and(|t| ahora.duration_since(t) > SE_GUARDA)
                {
                    caducadas.push(id.clone());
                }
                continue;
            }
            _ => continue,
        };
        c.fase = Fase::Fallida;
        c.trabajo = None;
        c.terminada = Some(ahora);
        c.resultado = Some(Json::obj([
            ("codigo", Json::s(codigo)),
            ("error", Json::s(fallo)),
        ]));
    }
    for id in caducadas {
        e.consultas.remove(&id);
    }
    let consultas = &e.consultas;
    e.pendientes
        .retain(|id| consultas.get(id).is_some_and(|c| c.fase == Fase::Encolada));
}

/// Que un `id` no se adivine contando: lo que no es tuyo es 404, pero tampoco
/// se enumera.
fn sal() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    format!("{:016x}", h.finish())
}

/// Un trabajador de `ore-consulta` (`ore-entrada` ata la audiencia a su cuenta).
pub fn es_trabajador(sujeto: &Identidad) -> bool {
    sujeto
        .persona
        .strip_prefix(ore_entrada::oidc::PREFIJO_PUESTO)
        .and_then(|r| r.split('/').next())
        == Some(ore_entrada::oidc::PUESTO_DE_CONSULTA)
}

/// Por qué una consulta no corre (todavía) en un trabajador, de lo que el
/// servidor resolvió: lo que se lee en vivo, una colección, una Function.
fn en_el_puesto(resuelto: &Json) -> Option<String> {
    let Json::Obj(m) = resuelto else {
        return None;
    };
    if m.contains_key("functions") {
        return Some("llama a una Function del árbol".into());
    }
    let Some(Json::Obj(f)) = m.get("fuentes") else {
        return None;
    };
    for (n, v) in f {
        let Json::Obj(v) = v else { continue };
        if v.contains_key("federada") || v.contains_key("vistaFederada") {
            return Some(format!("`{n}` se lee en vivo de un origen"));
        }
        if v.contains_key("collection") {
            return Some(format!("`{n}` es una colección"));
        }
    }
    None
}

fn base64_a_bytes(s: &str) -> Result<Vec<u8>, String> {
    let valor = |c: u8| -> Result<u32, String> {
        Ok(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("`{}` no es base64", c as char)),
        } as u32)
    };
    let s = s.trim().trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for t in s.chunks(4) {
        let mut n = 0u32;
        for (i, c) in t.iter().enumerate() {
            n |= valor(*c)? << (18 - 6 * i);
        }
        match t.len() {
            4 => out.extend([(n >> 16) as u8, (n >> 8) as u8, n as u8]),
            3 => out.extend([(n >> 16) as u8, (n >> 8) as u8]),
            2 => out.push((n >> 16) as u8),
            _ => return Err("base64 cortado".into()),
        }
    }
    Ok(out)
}

fn cabecera_u64(p: &Peticion, k: &str) -> Option<u64> {
    p.cabeceras.get(k).and_then(|v| v.trim().parse().ok())
}

/// Quien firma en la cola lo que pide la regla de escalado: el propio servidor.
fn ore_serve() -> Identidad {
    Identidad {
        persona: "ore-serve".into(),
        agente: None,
        correo: None,
        nombre: None,
        tipo: None,
        usuario: None,
    }
}

impl Servidor {
    /// **Q5·3 · Una vuelta de la regla de escalado** (`ore_core::escalado`): pide
    /// a la cola los trabajadores de quita y pon que falten y retira los ficheros
    /// de los que ya se fueron, en un solo commit. Devuelve lo que hay que decir
    /// en el registro (vacío si no pasó nada).
    pub(crate) fn escalar_consultas(&self) -> String {
        if !self.consultas.escala() {
            return String::new();
        }
        let Some(forja) = &self.cola else {
            return String::new();
        };
        let (foto, retirar) = self.consultas.foto();
        let decision = escalado::escalar(&foto, &Umbrales::default(), self.consultas.techo);
        if decision.pedir == 0 && retirar.is_empty() {
            self.consultas.anotar(decision, Vec::new());
            return String::new();
        }
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return format!("consultas · escalado: no se clonó la cola: {e}"),
        };
        let dir = prestado.ruta();
        for (_, fichero) in &retirar {
            let _ = std::fs::remove_file(dir.join(fichero));
        }
        let mut pedidos = Vec::new();
        if decision.pedir > 0 {
            let Ok(plantilla) = std::fs::read_to_string(dir.join(crate::cola::PLANTILLA_CONSULTA))
            else {
                self.consultas.anotar(decision, Vec::new());
                return format!(
                    "consultas · escalado: la cola no trae `{}`: hay que converger esta celda",
                    crate::cola::PLANTILLA_CONSULTA
                );
            };
            let instante = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            for i in 0..decision.pedir {
                match crate::cola::rendir_consulta(&plantilla, &format!("{instante}-{i}")) {
                    Ok((fichero, texto, job)) => {
                        if let Err(e) = std::fs::write(dir.join(&fichero), texto) {
                            return format!(
                                "consultas · escalado: no se escribió `{fichero}`: {e}"
                            );
                        }
                        pedidos.push((job, fichero));
                    }
                    Err(e) => return format!("consultas · escalado: {e}"),
                }
            }
        }
        if !forja.hay_cambios(dir) {
            self.consultas.anotar(decision, pedidos);
            return String::new();
        }
        let que = format!(
            "Query: {} trabajador(es) más, {} fuera ({})",
            pedidos.len(),
            retirar.len(),
            decision.razon
        );
        match forja.publicar(dir, &ore_serve(), &que) {
            Ok(c) => {
                let linea = format!("consultas · escalado · {que} · commit {c}");
                self.consultas.anotar(decision, pedidos);
                linea
            }
            // Lo pedido no se cuenta como en camino si no llegó a la cola; lo
            // retirado sí deja de contarse: la próxima vuelta lo vuelve a quitar.
            Err(e) => {
                self.consultas.anotar(decision, Vec::new());
                format!("consultas · escalado: NO encolado: {e}")
            }
        }
    }

    /// `GET /consultas/capacidad` (Q5·3): lo admitido, lo libre y lo pedido.
    pub(crate) fn capacidad_de_consultas(&self) -> Respuesta {
        Respuesta::ok(self.consultas.capacidad())
    }

    pub(crate) fn admitir_consulta(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        let t0 = Instant::now();
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(_) => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let texto = campo("texto").unwrap_or_default();
        if texto.trim().is_empty() {
            return Respuesta::error(422, "una consulta quiere `texto`");
        }
        let limite = match campo("limite").map(|l| l.parse::<u64>()) {
            None => LIMITE,
            Some(Ok(l)) if (1..=LIMITE_MAXIMO).contains(&l) => l,
            Some(_) => {
                return Respuesta::error(422, format!("`limite` va de 1 a {LIMITE_MAXIMO} filas"));
            }
        };
        let rama = campo("rama").filter(|r| !r.is_empty() && r != "main");
        let malas = admision::rutas_sospechosas(&texto);
        if !malas.is_empty() {
            return Respuesta::error(
                422,
                format!(
                    "una ruta de la consulta sale de donde está (`..` o `%2e`): {}",
                    malas.join(", ")
                ),
            );
        }
        let linea = match self.explicar(rama.as_deref(), &texto, true, true) {
            Ok(l) => l,
            Err(r) => return r,
        };
        let plan = match ore_core::parse::parse(&linea) {
            Ok(p) => p,
            Err(_) => return Respuesta::error(502, "`ore explain` no devolvió JSON"),
        };
        // Lo que el reparto niega (gobierno, coste, interruptor), con su código.
        if plan.get("ok").and_then(|(_, v)| v.as_str()) == Some("false") {
            let c = |k: &str| {
                plan.get(k)
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let http = c("http").parse::<u16>().unwrap_or(422);
            let mut r = Respuesta::error(http, c("mensaje"));
            if let Json::Obj(m) = &mut r.cuerpo {
                m.insert("codigo".into(), Json::s(c("codigo")));
                m.insert("tabla".into(), Json::s(c("tabla")));
            }
            return r;
        }
        let entrada = admision::entrada_de_explain(&plan);
        let u = Umbrales::default();
        let mut decision = admision::admitir(&entrada, &u);
        let decidir_ms = t0.elapsed().as_millis() as u64;
        let mut cuerpo = BTreeMap::from([
            (
                "lee".to_string(),
                Json::obj([
                    ("filas", Json::Int(entrada.filas as i64)),
                    ("bytes", Json::Int(entrada.bytes as i64)),
                    ("estimada", Json::Bool(entrada.estimada)),
                    (
                        "en_vivo",
                        Json::Arr(entrada.en_vivo.iter().map(Json::s).collect()),
                    ),
                ]),
            ),
            (
                "estimacion".to_string(),
                plan.get("estimacion")
                    .map(|(_, v)| Json::de_node(v))
                    .unwrap_or_else(|| Json::obj([])),
            ),
        ]);
        let ms = |m: Vec<(&'static str, u64)>| {
            Json::obj(m.into_iter().map(|(k, v)| (k, Json::Int(v as i64))))
        };
        if !self.consultas.activa {
            cuerpo.insert("admision".into(), decision.json());
            cuerpo.insert("estado".into(), Json::s("admitida"));
            cuerpo.insert("ms".into(), Json::Int(decidir_ms as i64));
            return Respuesta::ok(Json::Obj(cuerpo));
        }
        // ── Q4 · correrla ──────────────────────────────────────────────────
        // E todavía no (Q7): corre en G, que es lo más grande que hay, y se dice.
        if decision.clase == Clase::E {
            decision.clase = Clase::G;
            decision.avisos.push(
                "iría a E (Spark elástico), que aún no está: corre en G, el trabajador entero"
                    .into(),
            );
        }
        cuerpo.insert("admision".into(), decision.json());
        let al_puesto = |motivo: String, mut cuerpo: BTreeMap<String, Json>| {
            cuerpo.insert("codigo".into(), Json::s("consulta/en-el-puesto"));
            cuerpo.insert(
                "error".into(),
                Json::s(format!(
                    "{motivo}: esta consulta corre todavía en el puesto, no en un trabajador de Query"
                )),
            );
            Respuesta {
                codigo: 409,
                cuerpo: Json::Obj(cuerpo),
            }
        };
        if let Some(t) = entrada.en_vivo.first() {
            return al_puesto(format!("`{t}` se lee en vivo de un origen"), cuerpo);
        }
        let t1 = Instant::now();
        let resuelto =
            self.fuentes_de_sql(rama.clone(), &texto, &|n| self.datos_en(rama.clone(), n));
        if resuelto.codigo != 200 {
            return resuelto;
        }
        if let Some(motivo) = en_el_puesto(&resuelto.cuerpo) {
            return al_puesto(motivo, cuerpo);
        }
        let fuentes = match &resuelto.cuerpo {
            Json::Obj(m) => m.get("fuentes").cloned().unwrap_or_else(|| Json::obj([])),
            _ => Json::obj([]),
        };
        let resolver_ms = t1.elapsed().as_millis() as u64;
        let trabajo = Json::obj([
            ("texto", Json::s(texto.as_str())),
            ("fuentes", fuentes),
            ("limite", Json::Int(limite as i64)),
            ("clase", Json::s(decision.clase.nombre())),
            ("memoria_mb", Json::Int(decision.memoria_mb as i64)),
            ("hilos", Json::Int(decision.hilos as i64)),
        ]);
        let id = match self.consultas.encolar(
            &sujeto.persona,
            decision.memoria_mb,
            decision.hilos,
            trabajo,
        ) {
            Ok(id) => id,
            Err(m) => return Respuesta::error(503, m),
        };
        cuerpo.insert("id".into(), Json::s(&id));
        let vista = self
            .consultas
            .esperar(&id, &sujeto.persona, ESPERA_DE_QUIEN_PREGUNTA);
        let mut r = con_vista(cuerpo, vista);
        if let Json::Obj(m) = &mut r.cuerpo {
            let mut v = vec![
                ("decidir", decidir_ms),
                ("resolver", resolver_ms),
                ("total", t0.elapsed().as_millis() as u64),
            ];
            if let Some(Json::Obj(t)) = m.get("ms") {
                for (k, x) in t {
                    if let (Json::Int(x), Some(k)) =
                        (x, ["cola", "correr"].iter().find(|c| **c == k.as_str()))
                    {
                        v.push((k, *x as u64));
                    }
                }
            }
            m.insert("ms".into(), ms(v));
        }
        r
    }

    /// `GET /consultas/{id}` (con `x-ore-espera: <s>`, hasta 25): sólo quien la lanzó.
    pub(crate) fn ver_consulta(&self, p: &Peticion, sujeto: &Identidad, id: &str) -> Respuesta {
        let espera = cabecera_u64(p, "x-ore-espera")
            .map(Duration::from_secs)
            .unwrap_or(Duration::ZERO)
            .min(ESPERA_DE_QUIEN_PREGUNTA);
        match self.consultas.esperar(id, &sujeto.persona, espera) {
            None => Respuesta::error(404, format!("no hay ninguna consulta `{id}` tuya")),
            v => con_vista(BTreeMap::from([("id".into(), Json::s(id))]), v),
        }
    }

    /// `GET /consultas/trabajo`: un trabajador pide lo que le cabe.
    pub(crate) fn trabajo_para_el_trabajador(&self, p: &Peticion, sujeto: &Identidad) -> Respuesta {
        if !es_trabajador(sujeto) {
            return Respuesta::error(403, "esta ruta es de los trabajadores de Query");
        }
        let (Some(libre), Some(hilos)) = (
            cabecera_u64(p, "x-ore-libre-mb"),
            cabecera_u64(p, "x-ore-libres-hilos"),
        ) else {
            return Respuesta::error(
                422,
                "un trabajador dice lo que le cabe: `x-ore-libre-mb` y `x-ore-libres-hilos`",
            );
        };
        let quien = trabajador_de(p, sujeto);
        match self
            .consultas
            .tomar(&quien, libre, hilos, ESPERA_DEL_TRABAJADOR)
        {
            Some(t) => Respuesta::ok(t),
            None => Respuesta {
                codigo: 204,
                cuerpo: Json::obj([]),
            },
        }
    }

    /// `POST /consultas/{id}/resultado {ok, arrow?, codigo?, error?, …}`.
    pub(crate) fn resultado_del_trabajador(
        &self,
        p: &Peticion,
        sujeto: &Identidad,
        id: &str,
    ) -> Respuesta {
        if !es_trabajador(sujeto) {
            return Respuesta::error(403, "esta ruta es de los trabajadores de Query");
        }
        let Ok(n) = ore_core::parse::parse(&p.cuerpo) else {
            return Respuesta::error(400, "el cuerpo no es JSON");
        };
        let s = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let bien = s("ok").as_deref() == Some("true");
        // Lo que el trabajador midió, tal cual (números).
        let mut medido = BTreeMap::new();
        for k in [
            "instancia_ms",
            "correr_ms",
            "espera_ms",
            "filas",
            "bytes",
            "pico_mb",
        ] {
            if let Some(v) = s(k).and_then(|v| v.parse::<i64>().ok()) {
                medido.insert(k.to_string(), Json::Int(v));
            }
        }
        let mut resultado = BTreeMap::from([("trabajador".to_string(), Json::Obj(medido))]);
        let mut bien = bien;
        if bien {
            let filas = s("arrow")
                .ok_or_else(|| "falta `arrow`".to_string())
                .and_then(|a| base64_a_bytes(&a))
                .and_then(|arrow| {
                    let peticion = Json::obj([
                        ("desde", Json::s("0")),
                        ("limite", Json::s(LIMITE_MAXIMO.to_string())),
                    ])
                    .jcs();
                    crate::preview::a_filas(&self.binario, &peticion, &arrow)
                });
            match filas {
                Ok(f) => {
                    resultado.insert("filas".into(), Json::Crudo(f.trim().to_string()));
                    resultado.insert(
                        "truncada".into(),
                        Json::Bool(s("truncada").as_deref() == Some("true")),
                    );
                }
                Err(e) => {
                    bien = false;
                    resultado.insert("codigo".into(), Json::s("consulta/resultado"));
                    resultado.insert("error".into(), Json::s(e));
                }
            }
        } else {
            resultado.insert(
                "codigo".into(),
                Json::s(s("codigo").unwrap_or_else(|| "consulta/sql".into())),
            );
            resultado.insert(
                "error".into(),
                Json::s(s("error").unwrap_or_else(|| "el trabajador no dijo por qué".into())),
            );
        }
        let quien = trabajador_de(p, sujeto);
        match self
            .consultas
            .entregar(&quien, id, bien, Json::Obj(resultado))
        {
            Ok(()) => Respuesta::ok(Json::obj([("id", Json::s(id))])),
            Err(m) => Respuesta::error(409, m),
        }
    }
}

/// Qué trabajador es (su pod, `x-ore-trabajador`), dentro de la cuenta que el
/// token garantiza: todos los de la celda valen lo mismo, y esto sólo ata un
/// resultado a quien tomó la consulta.
fn trabajador_de(p: &Peticion, sujeto: &Identidad) -> String {
    let pod = p
        .cabeceras
        .get("x-ore-trabajador")
        .map(|v| v.trim())
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 64
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        })
        .unwrap_or("?");
    format!("{}#{pod}", sujeto.persona)
}

/// La respuesta de una consulta: su fase y, si acabó, su resultado (o su error).
fn con_vista(mut cuerpo: BTreeMap<String, Json>, vista: Option<Vista>) -> Respuesta {
    let Some(v) = vista else {
        return Respuesta::error(404, "esa consulta ya no está");
    };
    cuerpo.insert("estado".into(), Json::s(v.fase.nombre()));
    let mut ms = BTreeMap::new();
    if let Some(c) = v.cola_ms {
        ms.insert("cola".to_string(), Json::Int(c as i64));
    }
    if let Some(c) = v.correr_ms {
        ms.insert("correr".to_string(), Json::Int(c as i64));
    }
    cuerpo.insert("ms".into(), Json::Obj(ms));
    let mut codigo = 202;
    if let Some(Json::Obj(r)) = v.resultado {
        codigo = if v.fase == Fase::Hecha { 200 } else { 422 };
        for (k, x) in r {
            cuerpo.insert(k, x);
        }
        // Lo que no tomó ningún trabajador es de capacidad, no de la consulta.
        if matches!(cuerpo.get("codigo"), Some(Json::Str(c)) if c == "consulta/sin-trabajador") {
            codigo = 503;
        }
    }
    Respuesta {
        codigo,
        cuerpo: Json::Obj(cuerpo),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::sync::Arc;

    fn trabajo() -> Json {
        Json::obj([("texto", Json::s("select 1"))])
    }

    #[test]
    fn un_trabajador_toma_lo_que_le_cabe_y_en_orden() {
        let c = Cola::nueva(true, 1);
        let g = c.encolar("persona:ana", 39_321, 8, trabajo()).unwrap();
        let l = c.encolar("persona:bea", 2048, 2, trabajo()).unwrap();
        // Con 4 GB libres no cabe la G: toma la L, que va detrás.
        let t = c.tomar("w#1", 4096, 8, Duration::ZERO).unwrap();
        assert!(t.jcs().contains(&l), "{}", t.jcs());
        assert!(c.tomar("w#1", 4096, 8, Duration::ZERO).is_none());
        // Con el trabajador entero libre, la G.
        let t = c.tomar("w#2", 39_321, 8, Duration::ZERO).unwrap();
        assert!(t.jcs().contains(&g));
    }

    #[test]
    fn la_espera_larga_vuelve_en_cuanto_hay_trabajo() {
        let c = Arc::new(Cola::nueva(true, 1));
        let c2 = c.clone();
        let h = std::thread::spawn(move || {
            let t0 = Instant::now();
            let t = c2.tomar("w#1", 8192, 8, Duration::from_secs(5));
            (t.is_some(), t0.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        c.encolar("persona:ana", 2048, 2, trabajo()).unwrap();
        let (tomo, tardo) = h.join().unwrap();
        assert!(tomo);
        assert!(tardo < Duration::from_secs(2), "{tardo:?}");
    }

    #[test]
    fn el_resultado_es_de_quien_la_tomo_y_lo_ve_solo_quien_la_lanzo() {
        let c = Cola::nueva(true, 1);
        let id = c.encolar("persona:ana", 2048, 2, trabajo()).unwrap();
        // Las credenciales salen al tomarla y no se quedan.
        assert!(c.tomar("w#1", 4096, 2, Duration::ZERO).is_some());
        assert!(
            c.entregar("w#2", &id, true, Json::obj([])).is_err(),
            "otro trabajador no entrega lo que no tomó"
        );
        c.entregar("w#1", &id, true, Json::obj([("filas", Json::s("[]"))]))
            .unwrap();
        assert!(
            c.entregar("w#1", &id, true, Json::obj([])).is_err(),
            "una vez"
        );
        assert!(c.esperar(&id, "persona:bea", Duration::ZERO).is_none());
        let v = c.esperar(&id, "persona:ana", Duration::ZERO).unwrap();
        assert_eq!(v.fase, Fase::Hecha);
        assert!(v.cola_ms.is_some() && v.correr_ms.is_some());
    }

    #[test]
    fn lo_que_nadie_toma_falla_y_dice_que_falta_capacidad() {
        let mut e = Estado::default();
        let antes = Instant::now();
        e.consultas.insert(
            "q1".into(),
            Consulta {
                dueno: "persona:ana".into(),
                memoria_mb: 1,
                hilos: 1,
                trabajo: Some(trabajo()),
                fase: Fase::Encolada,
                creada: antes,
                tomada: None,
                terminada: None,
                trabajador: None,
                resultado: None,
            },
        );
        e.pendientes.push_back("q1".into());
        barrer(&mut e, antes + SIN_TRABAJADOR + Duration::from_secs(1));
        let q = &e.consultas["q1"];
        assert_eq!(q.fase, Fase::Fallida);
        assert!(q.trabajo.is_none());
        assert!(e.pendientes.is_empty());
        let r = con_vista(
            BTreeMap::new(),
            Some(Vista {
                fase: q.fase,
                cola_ms: None,
                correr_ms: None,
                resultado: q.resultado.clone(),
            }),
        );
        assert_eq!(r.codigo, 503);
        assert!(r.cuerpo.jcs().contains("consulta/sin-trabajador"));
        // Y lo terminado se olvida a su tiempo.
        barrer(
            &mut e,
            antes + SIN_TRABAJADOR + SE_GUARDA + Duration::from_secs(2),
        );
        assert!(e.consultas.is_empty());
    }

    #[test]
    fn lo_que_aun_no_corre_en_un_trabajador_se_dice() {
        let f = |v: Json| Json::obj([("fuentes", Json::obj([("a.b.c", v)]))]);
        assert!(en_el_puesto(&f(Json::obj([("federada", Json::obj([]))]))).is_some());
        assert!(en_el_puesto(&f(Json::obj([("vistaFederada", Json::s("x"))]))).is_some());
        assert!(en_el_puesto(&f(Json::obj([("collection", Json::s("a.b.c"))]))).is_some());
        assert!(
            en_el_puesto(&f(Json::obj([(
                "metadata_location",
                Json::s("gs://b/t/metadata/1.metadata.json")
            )])))
            .is_none()
        );
        let mut con_fn = f(Json::obj([]));
        if let Json::Obj(m) = &mut con_fn {
            m.insert("functions".into(), Json::Arr(vec![]));
        }
        assert!(en_el_puesto(&con_fn).is_some());
    }

    #[test]
    fn el_trabajador_se_reconoce_por_su_identidad() {
        let id = |p: &str| Identidad {
            persona: p.into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: Some("agente".into()),
            usuario: None,
        };
        assert!(es_trabajador(&id("agente:puesto/ore-consulta/1")));
        assert!(!es_trabajador(&id("agente:puesto/puesto-ana-1/2")));
        assert!(!es_trabajador(&id("persona:ore-consulta")));
    }

    /// Q5·3 · Hace `d` en el pasado (sin desbordar en una máquina recién encendida).
    fn hace(d: Duration) -> Instant {
        Instant::now().checked_sub(d).unwrap_or_else(Instant::now)
    }

    #[test]
    fn la_foto_cuenta_lo_que_cada_trabajador_dice_y_lo_que_tomo() {
        let c = Cola::nueva(true, 3);
        // Nadie ha pedido: ningún trabajador, y sin actividad.
        let (f, _) = c.foto();
        assert!(f.vivos.is_empty() && !f.activa);
        // El del suelo pide con todo libre y toma una L.
        c.encolar("persona:ana", 2048, 2, trabajo()).unwrap();
        assert!(
            c.tomar("w#ore-consulta-5d8f-abcde", 39_321, 8, Duration::ZERO)
                .is_some()
        );
        let (f, retirar) = c.foto();
        assert_eq!(
            f.vivos,
            vec![Hueco {
                mb: 39_321 - 2048,
                hilos: 6
            }]
        );
        assert!(f.activa && f.encoladas.is_empty() && retirar.is_empty());
        // Lo encolado que nadie ha tomado cuenta con su memoria y sus hilos.
        c.encolar("persona:bea", 39_321, 8, trabajo()).unwrap();
        assert_eq!(
            c.foto().0.encoladas,
            vec![Hueco {
                mb: 39_321,
                hilos: 8
            }]
        );
    }

    #[test]
    fn un_pedido_esta_en_camino_hasta_que_su_pod_pide_y_se_retira_al_irse() {
        let c = Cola::nueva(true, 3);
        let d = escalado::Decision {
            pedir: 1,
            razon: "prueba".into(),
        };
        c.anotar(
            d,
            vec![(
                "consulta-1a2b3c4d".into(),
                "58-la-consulta-1a2b3c4d.yaml".into(),
            )],
        );
        assert_eq!(c.foto().0.en_camino, 1);
        // Pide un pod de ese Job: ya no está en camino, está vivo.
        assert!(
            c.tomar("w#consulta-1a2b3c4d-x7k2p", 39_321, 8, Duration::ZERO)
                .is_none()
        );
        let (f, retirar) = c.foto();
        assert_eq!((f.en_camino, f.vivos.len()), (0, 1));
        assert!(retirar.is_empty());
        // Deja de pedir (se fue por ocioso): su fichero sale de la cola, una vez.
        c.estado
            .lock()
            .unwrap()
            .trabajadores
            .get_mut("w#consulta-1a2b3c4d-x7k2p")
            .unwrap()
            .ultima = hace(VIVO + Duration::from_secs(1));
        let (f, retirar) = c.foto();
        assert_eq!(
            retirar,
            vec![(
                "consulta-1a2b3c4d".to_string(),
                "58-la-consulta-1a2b3c4d.yaml".to_string()
            )]
        );
        assert!(f.vivos.is_empty());
        assert!(c.foto().1.is_empty());
        let j = c.capacidad().jcs();
        assert!(
            j.contains("\"techo\":3") && j.contains("\"razon\":\"prueba\""),
            "{j}"
        );
    }

    #[test]
    fn el_que_corre_algo_esta_vivo_aunque_no_pida_y_el_que_no_llega_se_retira() {
        let c = Cola::nueva(true, 3);
        c.encolar("persona:ana", 39_321, 8, trabajo()).unwrap();
        assert!(c.tomar("w#a", 39_321, 8, Duration::ZERO).is_some());
        // Lleno, no pide; pero corre una consulta: cuenta, con nada libre.
        c.estado
            .lock()
            .unwrap()
            .trabajadores
            .get_mut("w#a")
            .unwrap()
            .ultima = hace(VIVO + Duration::from_secs(1));
        assert_eq!(c.foto().0.vivos, vec![Hueco { mb: 0, hilos: 0 }]);
        // Un pedido que no pide en `ARRANQUE_MAXIMO` no llegó: se retira.
        c.anotar(
            escalado::Decision {
                pedir: 1,
                razon: String::new(),
            },
            vec![(
                "consulta-0000ffff".into(),
                "58-la-consulta-0000ffff.yaml".into(),
            )],
        );
        c.estado
            .lock()
            .unwrap()
            .pedidos
            .get_mut("consulta-0000ffff")
            .unwrap()
            .desde = hace(ARRANQUE_MAXIMO + Duration::from_secs(1));
        assert_eq!(c.foto().1.len(), 1);
    }

    #[test]
    fn con_un_pedido_en_camino_lo_encolado_le_espera() {
        let mut e = Estado::default();
        let antes = Instant::now();
        e.consultas.insert(
            "q1".into(),
            Consulta {
                dueno: "persona:ana".into(),
                memoria_mb: 1,
                hilos: 1,
                trabajo: Some(trabajo()),
                fase: Fase::Encolada,
                creada: antes,
                tomada: None,
                terminada: None,
                trabajador: None,
                resultado: None,
            },
        );
        e.pendientes.push_back("q1".into());
        e.pedidos.insert(
            "consulta-1".into(),
            Pedido {
                fichero: "f".into(),
                desde: antes,
                llego: None,
            },
        );
        barrer(&mut e, antes + SIN_TRABAJADOR + Duration::from_secs(1));
        assert_eq!(e.consultas["q1"].fase, Fase::Encolada);
        barrer(
            &mut e,
            antes + SIN_TRABAJADOR_CON_PEDIDO + Duration::from_secs(1),
        );
        assert_eq!(e.consultas["q1"].fase, Fase::Fallida);
    }

    #[test]
    fn sin_techo_por_encima_del_suelo_no_se_escala() {
        assert!(!Cola::nueva(true, 1).escala());
        assert!(!Cola::nueva(false, 4).escala());
        assert!(Cola::nueva(true, 2).escala());
    }

    #[test]
    fn el_toque_despierta_a_la_regla_antes_de_su_cadencia() {
        let c = std::sync::Arc::new(Cola::nueva(true, 2));
        let c2 = c.clone();
        let h = std::thread::spawn(move || {
            let t0 = Instant::now();
            c2.esperar_toque(Duration::from_secs(10));
            t0.elapsed()
        });
        std::thread::sleep(Duration::from_millis(100));
        c.encolar("persona:ana", 1, 1, trabajo()).unwrap();
        assert!(h.join().unwrap() < Duration::from_secs(5));
    }

    #[test]
    fn base64_de_ida_y_vuelta() {
        for b in [&b""[..], b"a", b"ab", b"abc", b"\x00\xff\x10arrow"] {
            let s = crate::preview::base64(b);
            assert_eq!(base64_a_bytes(&s).unwrap(), b);
        }
        assert!(base64_a_bytes("a$==").is_err());
    }
}
