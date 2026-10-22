//! `ore-copiador`: el copiador residente de la celda.
//!
//! ```text
//! ore-copiador [--escucha 0.0.0.0:8100] [--guion /usr/local/lib/ore/copiar-worker.sh] [--a-la-vez 3]
//! ```
//!
//! **Qué es.** Un proceso siempre encendido que hace lo que hace el Job `copiar-*`
//! de `malla/48-la-copia.yaml` para una lista de datasets, sin lo que el Job paga
//! por arrancar de cero (un pod, tres init containers, el token de la forja y el
//! del agente). No reimplementa la copia: lanza `copiar-worker.sh`, que ejecuta el
//! mismo `ore materialize --vista …` y el mismo `publicar()`.
//!
//! **Qué hace.**
//! - `POST /copiar {"vistas": ["base.schema.dataset", …]}` → `202` al instante. Cada
//!   dataset queda `ingesting`; una vista que ya se está copiando no se duplica.
//! - `GET /estado` → el estado de cada dataset (`ingesting`, `ready`, `failed` con
//!   su motivo) y cuántas pasadas hay en marcha. Es lo que `ore-serve` y la
//!   consola leen para el spinner.
//! - `GET /salud`.
//!
//! **Qué NO decide.** Qué datasets tienen copia, con qué clave y desde qué fuente
//! es de `ore-serve` y del árbol; el copiador copia lo que se le pide o dice por
//! qué no. Si no responde, `ore-serve` encola el Job de siempre.
//!
//! El script cuenta lo que pasa con líneas `@@ <dataset> listo` y
//! `@@ <dataset> fallo <motivo>`; el resto de su salida es el registro de la pasada.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};

/// Cuánto se recuerda un dataset que ya acabó (listo o fallado).
const RECUERDO: Duration = Duration::from_secs(30 * 60);
/// Las últimas líneas del registro de una pasada que se guardan para el motivo.
const COLA_DEL_REGISTRO: usize = 40;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fase {
    Ingesting,
    Ready,
    Failed,
}

impl Fase {
    fn nombre(self) -> &'static str {
        match self {
            Fase::Ingesting => "ingesting",
            Fase::Ready => "ready",
            Fase::Failed => "failed",
        }
    }
}

struct Dataset {
    fase: Fase,
    motivo: String,
    desde: Instant,
    acabo: Option<Instant>,
    pasada: u64,
}

struct Cuenta {
    datasets: BTreeMap<String, Dataset>,
    /// Pasadas en marcha.
    corriendo: usize,
    /// Pasadas esperando hueco.
    esperando: usize,
    siguiente: u64,
}

pub struct Copiador {
    guion: PathBuf,
    a_la_vez: usize,
    cuenta: Mutex<Cuenta>,
    hueco: Condvar,
}

/// Un nombre de dataset que se puede pasar a un shell sin comillas.
fn nombre_valido(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 200
        && v.split('.').count() >= 2
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Lo que dice una línea `@@ <dataset> listo|fallo [motivo]`.
fn marca(linea: &str) -> Option<(&str, Fase, String)> {
    let resto = linea.strip_prefix("@@ ")?;
    let (vista, resto) = resto.split_once(' ').unwrap_or((resto, ""));
    let (que, motivo) = resto.split_once(' ').unwrap_or((resto, ""));
    match que {
        "listo" => Some((vista, Fase::Ready, String::new())),
        "fallo" => Some((vista, Fase::Failed, motivo.trim().to_string())),
        _ => None,
    }
}

impl Copiador {
    pub fn new(guion: PathBuf, a_la_vez: usize) -> Arc<Copiador> {
        Arc::new(Copiador {
            guion,
            a_la_vez: a_la_vez.max(1),
            cuenta: Mutex::new(Cuenta {
                datasets: BTreeMap::new(),
                corriendo: 0,
                esperando: 0,
                siguiente: 1,
            }),
            hueco: Condvar::new(),
        })
    }

    fn cuenta(&self) -> std::sync::MutexGuard<'_, Cuenta> {
        self.cuenta.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn atender(self: &Arc<Self>, p: &Peticion) -> Respuesta {
        match (p.metodo.as_str(), p.segmentos().as_slice()) {
            ("GET", ["salud"]) => Respuesta::ok(Json::obj([("ok", Json::Bool(true))])),
            ("GET", ["estado"]) => self.estado(),
            ("POST", ["copiar"]) => self.copiar(&p.cuerpo),
            (_, ["salud" | "estado" | "copiar"]) => Respuesta::error(405, "método no admitido"),
            _ => Respuesta::error(404, "no hay tal ruta"),
        }
    }

    fn estado(&self) -> Respuesta {
        let mut c = self.cuenta();
        let ahora = Instant::now();
        c.datasets
            .retain(|_, d| d.acabo.is_none_or(|t| ahora.duration_since(t) < RECUERDO));
        let datasets = Json::Obj(
            c.datasets
                .iter()
                .map(|(v, d)| {
                    let mut campos = vec![
                        ("estado".to_string(), Json::s(d.fase.nombre())),
                        (
                            "segundos".to_string(),
                            Json::Int(
                                d.acabo.unwrap_or(ahora).duration_since(d.desde).as_secs() as i64
                            ),
                        ),
                        ("pasada".to_string(), Json::Int(d.pasada as i64)),
                    ];
                    if !d.motivo.is_empty() {
                        campos.push(("motivo".to_string(), Json::s(&d.motivo)));
                    }
                    (v.clone(), Json::Obj(campos.into_iter().collect()))
                })
                .collect(),
        );
        Respuesta::ok(Json::obj([
            ("datasets", datasets),
            ("corriendo", Json::Int(c.corriendo as i64)),
            ("esperando", Json::Int(c.esperando as i64)),
        ]))
    }

    fn copiar(self: &Arc<Self>, cuerpo: &str) -> Respuesta {
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(e) => return Respuesta::error(400, format!("el cuerpo no es JSON: {e:?}")),
        };
        let pedidas: Vec<String> = match n.get("vistas") {
            Some((_, v)) => v
                .items()
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect(),
            None => return Respuesta::error(422, "falta `vistas`"),
        };
        if pedidas.is_empty() {
            return Respuesta::error(422, "`vistas` está vacía");
        }
        if let Some(mala) = pedidas.iter().find(|v| !nombre_valido(v)) {
            return Respuesta::error(422, format!("`{mala}` no es un nombre de dataset"));
        }
        let (pasada, nuevas) = {
            let mut c = self.cuenta();
            let pasada = c.siguiente;
            let mut nuevas = Vec::new();
            for v in pedidas {
                if c.datasets
                    .get(&v)
                    .is_some_and(|d| d.fase == Fase::Ingesting)
                {
                    continue;
                }
                c.datasets.insert(
                    v.clone(),
                    Dataset {
                        fase: Fase::Ingesting,
                        motivo: String::new(),
                        desde: Instant::now(),
                        acabo: None,
                        pasada,
                    },
                );
                nuevas.push(v);
            }
            if !nuevas.is_empty() {
                c.siguiente += 1;
                c.esperando += 1;
            }
            (pasada, nuevas)
        };
        if nuevas.is_empty() {
            return Respuesta {
                codigo: 202,
                cuerpo: Json::obj([("ya", Json::Bool(true))]),
            };
        }
        let yo = Arc::clone(self);
        let lista = nuevas.clone();
        std::thread::spawn(move || yo.pasada(pasada, lista));
        Respuesta {
            codigo: 202,
            cuerpo: Json::obj([
                ("pasada", Json::Int(pasada as i64)),
                ("vistas", Json::Arr(nuevas.iter().map(Json::s).collect())),
            ]),
        }
    }

    fn poner(&self, vista: &str, fase: Fase, motivo: String) {
        let mut c = self.cuenta();
        if let Some(d) = c.datasets.get_mut(vista)
            && d.fase == Fase::Ingesting
        {
            d.fase = fase;
            d.motivo = motivo;
            d.acabo = Some(Instant::now());
        }
    }

    /// Una pasada: espera hueco, lanza el guion y traduce sus marcas.
    fn pasada(&self, id: u64, vistas: Vec<String>) {
        {
            let mut c = self.cuenta();
            while c.corriendo >= self.a_la_vez {
                c = self.hueco.wait(c).unwrap_or_else(|e| e.into_inner());
            }
            c.esperando -= 1;
            c.corriendo += 1;
        }
        let inicio = Instant::now();
        eprintln!("pasada {id}: {}", vistas.join(","));
        let mut cola: VecDeque<String> = VecDeque::new();
        let salida = Command::new("sh")
            .arg(&self.guion)
            .arg(id.to_string())
            .arg(vistas.join(","))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn();
        let estado = match salida {
            Ok(mut hijo) => {
                if let Some(o) = hijo.stdout.take() {
                    for linea in BufReader::new(o).lines().map_while(Result::ok) {
                        if let Some((v, fase, motivo)) = marca(&linea) {
                            self.poner(v, fase, motivo);
                        }
                        eprintln!("[{id}] {linea}");
                        cola.push_back(linea);
                        if cola.len() > COLA_DEL_REGISTRO {
                            cola.pop_front();
                        }
                    }
                }
                hijo.wait()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|e| e.to_string())
            }
            Err(e) => format!("no se pudo lanzar el guion: {e}"),
        };
        // Lo que el guion no cerró no se copió: lo dice la última línea útil.
        let motivo = cola
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty() && !l.starts_with("@@"))
            .cloned()
            .unwrap_or(estado.clone());
        for v in &vistas {
            self.poner(
                v,
                Fase::Failed,
                format!("la pasada terminó sin copiarlo: {motivo}"),
            );
        }
        eprintln!("pasada {id}: acabó en {:?} ({estado})", inicio.elapsed());
        let mut c = self.cuenta();
        c.corriendo -= 1;
        self.hueco.notify_one();
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let valor = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let escucha = valor("--escucha").unwrap_or_else(|| "0.0.0.0:8100".into());
    let guion = valor("--guion")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/local/lib/ore/copiar-worker.sh"));
    let a_la_vez = valor("--a-la-vez")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3usize);
    if !guion.is_file() {
        eprintln!("ore-copiador: no está el guion {}", guion.display());
        return ExitCode::from(2);
    }
    let copiador = Copiador::new(guion, a_la_vez);
    let l = match TcpListener::bind(&escucha) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ore-copiador: no se escucha en {escucha}: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("ore-copiador: escuchando en {escucha}, {a_la_vez} pasadas a la vez");
    match ore_entrada::http::servir(l, move |p| copiador.atender(p)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ore-copiador: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn las_marcas_del_guion() {
        assert_eq!(
            marca("@@ b.s.d listo"),
            Some(("b.s.d", Fase::Ready, String::new()))
        );
        assert_eq!(
            marca("@@ b.s.d fallo la fuente no contesta"),
            Some(("b.s.d", Fase::Failed, "la fuente no contesta".into()))
        );
        assert_eq!(marca("### copiando"), None);
        assert_eq!(marca("@@ b.s.d otra"), None);
    }

    #[test]
    fn solo_pasan_nombres_de_dataset() {
        assert!(nombre_valido("hbase_s.saas.cuentas"));
        assert!(nombre_valido("base.tabla"));
        assert!(!nombre_valido("solo"));
        assert!(!nombre_valido("a.b; rm -rf /"));
        assert!(!nombre_valido("a.b c"));
        assert!(!nombre_valido(""));
    }

    fn con_guion(nombre: &str, cuerpo: &str) -> Arc<Copiador> {
        let d = std::env::temp_dir().join(format!("ore-copiador-{}-{nombre}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let g = d.join("g.sh");
        std::fs::write(&g, cuerpo).unwrap();
        Copiador::new(g, 2)
    }

    fn espera(c: &Arc<Copiador>, v: &str) -> (Fase, String) {
        for _ in 0..200 {
            {
                let g = c.cuenta();
                if let Some(d) = g.datasets.get(v)
                    && d.fase != Fase::Ingesting
                {
                    return (d.fase, d.motivo.clone());
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("`{v}` no terminó");
    }

    #[test]
    fn una_pasada_lista_los_que_el_guion_dice_y_falla_los_que_calla() {
        let c = con_guion(
            "marcas",
            "echo '@@ a.uno listo'\necho ruido\necho '@@ a.dos fallo sin permiso'\n",
        );
        let r = c.copiar(r#"{"vistas":["a.uno","a.dos","a.tres"]}"#);
        assert_eq!(r.codigo, 202);
        assert_eq!(espera(&c, "a.uno").0, Fase::Ready);
        assert_eq!(espera(&c, "a.dos"), (Fase::Failed, "sin permiso".into()));
        let (fase, motivo) = espera(&c, "a.tres");
        assert_eq!(fase, Fase::Failed);
        assert!(motivo.contains("sin copiarlo"), "{motivo}");
    }

    #[test]
    fn una_vista_que_ya_se_copia_no_se_duplica() {
        let c = con_guion("duplica", "sleep 1\necho '@@ a.uno listo'\n");
        assert_eq!(c.copiar(r#"{"vistas":["a.uno"]}"#).codigo, 202);
        let r = c.copiar(r#"{"vistas":["a.uno"]}"#);
        assert_eq!(r.codigo, 202);
        assert!(r.cuerpo.jcs().contains("\"ya\":true"), "{}", r.cuerpo.jcs());
        assert_eq!(espera(&c, "a.uno").0, Fase::Ready);
    }

    #[test]
    fn el_cuerpo_malo_se_rechaza() {
        let c = con_guion("malo", "true\n");
        assert_eq!(c.copiar("{").codigo, 400);
        assert_eq!(c.copiar(r#"{"vistas":[]}"#).codigo, 422);
        assert_eq!(c.copiar(r#"{"vistas":["a;b"]}"#).codigo, 422);
        assert_eq!(c.copiar("{}").codigo, 422);
    }
}
