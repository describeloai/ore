//! **Los conectores calientes y la cola por origen** (`docs/federation.md` §3).
//!
//! Por origen hay como mucho `concurrencia` lecturas a la vez; las demás
//! esperan en cola hasta `espera`, y con la cola llena o la espera agotada la
//! lectura no empieza (`saturado`). Es la base de un cliente: M3 midió que 50
//! a la vez le tumban conexiones.
//!
//! Cada lectura usa un **proceso `servir`** de su familia y su credencial. El
//! bucle de `servir` atiende una petición tras otra, así que un proceso es una
//! lectura a la vez y **la concurrencia de un origen es su número de procesos**.
//! Al acabar, el proceso vuelve a estar libre con su conexión abierta: la
//! lectura siguiente con la misma credencial no abre proceso ni conexión.
//!
//! ⛔ Dos credenciales nunca comparten proceso: la clave es la `url` entera
//!   (que la lleva), guardada como huella y no en claro.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::cotas::Cotas;

/// La huella de una credencial: decide qué proceso sirve a quién, sin
/// guardar la `url`.
pub fn huella(tipo: &str, url: &str) -> u64 {
    let mut h = DefaultHasher::new();
    tipo.hash(&mut h);
    url.hash(&mut h);
    h.finish()
}

/// Un conector `servir` vivo.
pub struct Proceso {
    hijo: Child,
    /// Compartida: por aquí entra la petición, y también `{"cancelar": id}`
    /// mientras otro hilo lee la respuesta.
    pub entrada: Arc<Mutex<ChildStdin>>,
    pub salida: BufReader<ChildStdout>,
    pub huella: u64,
    usado: Instant,
}

impl Proceso {
    /// Lanza `<programa> servir`. Su salida de error se lee en un hilo y se
    /// tapa con `url` antes de salir: el conector ya tapa la suya, y esto es la
    /// segunda puerta.
    pub fn lanzar(programa: &Path, huella: u64, url: &str) -> std::io::Result<Proceso> {
        let mut hijo = Command::new(programa)
            .arg("servir")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let entrada = hijo
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("sin stdin"))?;
        let salida = hijo
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("sin stdout"))?;
        if let Some(err) = hijo.stderr.take() {
            let url = url.to_string();
            let nombre = programa
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            std::thread::spawn(move || {
                for l in BufReader::new(err).lines().map_while(Result::ok) {
                    eprintln!("{nombre}: {}", ore_driver::tapar(&l, &url));
                }
            });
        }
        Ok(Proceso {
            hijo,
            entrada: Arc::new(Mutex::new(entrada)),
            salida: BufReader::new(salida),
            huella,
            usado: Instant::now(),
        })
    }

    pub fn pid(&self) -> u32 {
        self.hijo.id()
    }

    /// Manda una línea (una petición).
    pub fn pedir(&self, linea: &str) -> std::io::Result<()> {
        let mut e = self
            .entrada
            .lock()
            .map_err(|_| std::io::Error::other("entrada envenenada"))?;
        e.write_all(linea.as_bytes())?;
        e.write_all(b"\n")?;
        e.flush()
    }

    pub fn matar(mut self) {
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

/// Lo que se cuenta de un origen (`GET /v1/origins`).
#[derive(Debug, Clone, Default)]
pub struct Medidas {
    pub lecturas: u64,
    pub completas: u64,
    pub cortadas: u64,
    pub errores: u64,
    pub saturadas: u64,
    pub procesos_lanzados: u64,
    /// Las últimas 200 duraciones, para p50 y p95.
    pub ms: VecDeque<u64>,
}

impl Medidas {
    pub fn percentil(&self, p: f64) -> Option<u64> {
        if self.ms.is_empty() {
            return None;
        }
        let mut v: Vec<u64> = self.ms.iter().copied().collect();
        v.sort_unstable();
        let i = ((v.len() as f64 - 1.0) * p).round() as usize;
        v.get(i).copied()
    }
}

#[derive(Default)]
struct EstadoOrigen {
    activas: usize,
    en_cola: usize,
    libres: Vec<Proceso>,
    medidas: Medidas,
}

pub struct Origen {
    estado: Mutex<EstadoOrigen>,
    hay_sitio: Condvar,
}

/// Por qué una lectura no empezó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saturado {
    ColaLlena,
    EsperaAgotada,
}

/// Una plaza en un origen, con su proceso. Se devuelve con [`Fondo::soltar`];
/// si no se devuelve (un pánico), `Drop` libera la plaza y mata el proceso.
pub struct Plaza {
    pub origen: String,
    pub proceso: Option<Proceso>,
    /// Si el proceso ya estaba caliente (su conexión puede haber muerto).
    pub caliente: bool,
    tipo: String,
    url: String,
    fondo: Arc<Fondo>,
    devuelta: bool,
}

impl Plaza {
    /// **Un proceso nuevo en la misma plaza**: el caliente se mata. Para
    /// cuando su conexión murió (el origen se reinició, alguien la terminó):
    /// leer es repetible y aún no salió nada.
    pub fn relanzar(&mut self) -> std::io::Result<()> {
        if let Some(p) = self.proceso.take() {
            p.matar();
        }
        let h = huella(&self.tipo, &self.url);
        let p = Proceso::lanzar(&self.fondo.programa(&self.tipo), h, &self.url)?;
        self.fondo
            .origen(&self.origen)
            .estado
            .lock()
            .expect("origen")
            .medidas
            .procesos_lanzados += 1;
        self.proceso = Some(p);
        self.caliente = false;
        Ok(())
    }
}

impl Drop for Plaza {
    fn drop(&mut self) {
        if !self.devuelta {
            if let Some(p) = self.proceso.take() {
                p.matar();
            }
            self.fondo.liberar(&self.origen, None);
        }
    }
}

/// **El fondo**: los orígenes, sus colas y sus procesos.
pub struct Fondo {
    origenes: Mutex<BTreeMap<String, Arc<Origen>>>,
    pub cotas: Cotas,
    /// Donde están los conectores: `<dir>/ore-read-<tipo>`.
    pub conectores: PathBuf,
}

impl Fondo {
    pub fn new(cotas: Cotas, conectores: PathBuf) -> Arc<Fondo> {
        Arc::new(Fondo {
            origenes: Mutex::new(BTreeMap::new()),
            cotas,
            conectores,
        })
    }

    pub fn programa(&self, tipo: &str) -> PathBuf {
        self.conectores.join(format!("ore-read-{tipo}"))
    }

    fn origen(&self, nombre: &str) -> Arc<Origen> {
        let mut o = self.origenes.lock().expect("orígenes");
        o.entry(nombre.to_string())
            .or_insert_with(|| {
                Arc::new(Origen {
                    estado: Mutex::new(EstadoOrigen::default()),
                    hay_sitio: Condvar::new(),
                })
            })
            .clone()
    }

    /// **Una plaza** en `origen` y un proceso de `tipo` con esa credencial:
    /// uno libre y caliente si lo hay, uno nuevo si no.
    pub fn tomar(
        self: &Arc<Self>,
        origen: &str,
        tipo: &str,
        url: &str,
    ) -> Result<Result<Plaza, Saturado>, String> {
        let o = self.origen(origen);
        let h = huella(tipo, url);
        let caliente = {
            let mut e = o.estado.lock().expect("origen");
            if e.activas >= self.cotas.concurrencia {
                if e.en_cola >= self.cotas.cola {
                    e.medidas.saturadas += 1;
                    return Ok(Err(Saturado::ColaLlena));
                }
                e.en_cola += 1;
                let hasta = Instant::now() + self.cotas.espera;
                while e.activas >= self.cotas.concurrencia {
                    let queda = hasta.saturating_duration_since(Instant::now());
                    if queda.is_zero() {
                        e.en_cola -= 1;
                        e.medidas.saturadas += 1;
                        return Ok(Err(Saturado::EsperaAgotada));
                    }
                    e = o.hay_sitio.wait_timeout(e, queda).expect("origen").0;
                }
                e.en_cola -= 1;
            }
            e.activas += 1;
            e.medidas.lecturas += 1;
            let i = e.libres.iter().position(|p| p.huella == h);
            i.map(|i| e.libres.swap_remove(i))
        };
        let es_caliente = caliente.is_some();
        let proceso = match caliente {
            Some(p) => p,
            None => match Proceso::lanzar(&self.programa(tipo), h, url) {
                Ok(p) => {
                    o.estado.lock().expect("origen").medidas.procesos_lanzados += 1;
                    p
                }
                Err(e) => {
                    self.liberar(origen, None);
                    return Err(format!("no arranca el conector de `{tipo}`: {e}"));
                }
            },
        };
        Ok(Ok(Plaza {
            origen: origen.to_string(),
            proceso: Some(proceso),
            caliente: es_caliente,
            tipo: tipo.to_string(),
            url: url.to_string(),
            fondo: Arc::clone(self),
            devuelta: false,
        }))
    }

    /// Devuelve la plaza: con el proceso si su marco quedó entero (vuelve a
    /// estar libre y caliente), sin él si hubo que matarlo.
    pub fn soltar(&self, mut plaza: Plaza, sano: bool) {
        plaza.devuelta = true;
        let p = plaza.proceso.take();
        let p = match p {
            Some(p) if sano => Some(p),
            Some(p) => {
                p.matar();
                None
            }
            None => None,
        };
        self.liberar(&plaza.origen, p);
    }

    fn liberar(&self, origen: &str, proceso: Option<Proceso>) {
        let o = self.origen(origen);
        let mut e = o.estado.lock().expect("origen");
        e.activas = e.activas.saturating_sub(1);
        if let Some(mut p) = proceso {
            p.usado = Instant::now();
            e.libres.push(p);
            // Libres como mucho tantos como la concurrencia: el más viejo, fuera.
            while e.libres.len() > self.cotas.concurrencia {
                let viejo = (0..e.libres.len())
                    .min_by_key(|i| e.libres[*i].usado)
                    .expect("hay libres");
                e.libres.swap_remove(viejo).matar();
            }
        }
        o.hay_sitio.notify_one();
    }

    /// Anota cómo terminó una lectura de `origen`.
    pub fn anotar(&self, origen: &str, estado: &str, ms: u64) {
        let o = self.origen(origen);
        let mut e = o.estado.lock().expect("origen");
        match estado {
            "completo" => e.medidas.completas += 1,
            "cortado" => e.medidas.cortadas += 1,
            _ => e.medidas.errores += 1,
        }
        e.medidas.ms.push_back(ms);
        if e.medidas.ms.len() > 200 {
            e.medidas.ms.pop_front();
        }
    }

    /// **Cierra los conectores ociosos** (y con ellos su conexión al origen).
    /// Devuelve cuántos cerró.
    pub fn barrer(&self) -> usize {
        let ociosa = self.cotas.ociosa;
        let mut n = 0;
        let origenes: Vec<Arc<Origen>> = self
            .origenes
            .lock()
            .expect("orígenes")
            .values()
            .cloned()
            .collect();
        for o in origenes {
            let viejos: Vec<Proceso> = {
                let mut e = o.estado.lock().expect("origen");
                let (viejos, siguen): (Vec<Proceso>, Vec<Proceso>) = e
                    .libres
                    .drain(..)
                    .partition(|p| p.usado.elapsed() >= ociosa);
                e.libres = siguen;
                viejos
            };
            n += viejos.len();
            for p in viejos {
                p.matar();
            }
        }
        n
    }

    /// El hilo que barre, cada tanto.
    pub fn barrendero(self: &Arc<Self>) {
        let f = Arc::clone(self);
        let cada =
            (self.cotas.ociosa / 4).clamp(Duration::from_millis(250), Duration::from_secs(5));
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(cada);
                f.barrer();
            }
        });
    }

    /// Por origen: activas, en cola, procesos libres y lo medido.
    pub fn estado(&self) -> Vec<(String, usize, usize, usize, Medidas)> {
        let origenes: Vec<(String, Arc<Origen>)> = self
            .origenes
            .lock()
            .expect("orígenes")
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        origenes
            .into_iter()
            .map(|(k, o)| {
                let e = o.estado.lock().expect("origen");
                (k, e.activas, e.en_cola, e.libres.len(), e.medidas.clone())
            })
            .collect()
    }
}
