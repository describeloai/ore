//! **Lanzar un conector** como lo lanzaría la pasarela: un proceso con su
//! verbo, la petición por stdin, las filas por stdout y lo demás por stderr.
//!
//! Y medirlo mientras: el pico de memoria (en Linux, `/proc/<pid>/status`), el
//! primer byte y el total. Todo lo que el conector escribe en texto queda en un
//! registro, para que el caso 12 busque ahí los secretos.

use ore_driver::Fallo;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Lo que una invocación dejó.
#[derive(Debug, Default)]
pub struct Salida {
    /// Si terminó con éxito.
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// Del arranque al final.
    pub ms: u128,
    /// Del arranque al primer byte de stdout.
    pub primer_byte_ms: Option<u128>,
    /// El pico de memoria residente, en KiB, si el sistema lo deja ver.
    pub pico_kib: Option<u64>,
    /// Si hubo que matarlo por pasar de su plazo.
    pub matado: bool,
}

impl Salida {
    /// El error tipado de v2: la última línea JSON de stderr con `codigo`.
    pub fn fallo(&self) -> Option<Fallo> {
        self.stderr.lines().rev().find_map(|l| {
            let l = l.trim();
            let json = l.find('{').map(|i| &l[i..])?;
            Fallo::de_nodo(&ore_core::parse::parse(json).ok()?)
        })
    }

    /// Una línea para un informe: cómo terminó, sin la salida.
    pub fn resumen(&self) -> String {
        match (self.ok, self.fallo()) {
            (true, _) => format!("ok en {} ms", self.ms),
            (false, Some(f)) => format!("`{}`: {}", f.codigo.as_str(), corto(&f.mensaje)),
            (false, None) if self.matado => format!("matado a los {} ms", self.ms),
            (false, None) => format!("error sin tipar: {}", corto(self.stderr.trim())),
        }
    }
}

/// Un texto en una línea, y no más largo que lo que se lee de un vistazo.
pub fn corto(t: &str) -> String {
    let t = t.replace('\n', " ⏎ ");
    if t.chars().count() > 160 {
        format!("{}…", t.chars().take(160).collect::<String>())
    } else {
        t
    }
}

/// Un conector: la ruta de su binario y el registro de lo que dijo.
pub struct Conector {
    pub ruta: PathBuf,
    registro: RefCell<Vec<String>>,
}

/// El pico de memoria de un proceso, de `/proc`. `VmHWM` es el pico que el
/// núcleo lleva; si no está, el `VmRSS` de ese instante.
fn memoria(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let campo = |k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
    };
    campo("VmHWM:").or_else(|| campo("VmRSS:"))
}

impl Conector {
    pub fn new(ruta: PathBuf) -> Conector {
        Conector {
            ruta,
            registro: RefCell::new(Vec::new()),
        }
    }

    /// Todo el texto que el conector escribió hasta ahora.
    pub fn registro(&self) -> Vec<String> {
        self.registro.borrow().clone()
    }

    pub fn anotar(&self, texto: String) {
        self.registro.borrow_mut().push(texto);
    }

    /// Arranca `verbo` con stdin, stdout y stderr en tuberías.
    pub fn lanzar(&self, verbo: &str) -> std::io::Result<Child> {
        Command::new(&self.ruta)
            .arg(verbo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }

    /// **Una invocación entera**: `verbo` con `entrada` por stdin, esperando
    /// como mucho `plazo`. Con `cortar_a`, le manda `SIGTERM` a ese tiempo (el
    /// caso 10).
    pub fn correr(
        &self,
        verbo: &str,
        entrada: &str,
        plazo: Duration,
        cortar_a: Option<Duration>,
    ) -> Salida {
        let inicio = Instant::now();
        let mut hijo = match self.lanzar(verbo) {
            Ok(h) => h,
            Err(e) => {
                return Salida {
                    stderr: format!("no arranca `{}`: {e}", self.ruta.display()),
                    ..Default::default()
                };
            }
        };
        let pid = hijo.id();
        let mut stdin = hijo.stdin.take().expect("tubería");
        let mut stdout = hijo.stdout.take().expect("tubería");
        let mut stderr = hijo.stderr.take().expect("tubería");

        let primer = Arc::new(AtomicU64::new(u64::MAX));
        let lector = {
            let primer = primer.clone();
            std::thread::spawn(move || {
                let mut todo = Vec::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match stdout.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if todo.is_empty() {
                                primer
                                    .store(inicio.elapsed().as_millis() as u64, Ordering::Relaxed);
                            }
                            todo.extend_from_slice(&buf[..n]);
                        }
                    }
                }
                todo
            })
        };
        let errores = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let vivo = Arc::new(AtomicBool::new(true));
        let pico = Arc::new(AtomicU64::new(0));
        let vigia = {
            let (vivo, pico) = (vivo.clone(), pico.clone());
            std::thread::spawn(move || {
                while vivo.load(Ordering::Relaxed) {
                    if let Some(k) = memoria(pid) {
                        pico.fetch_max(k, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };
        let _ = stdin.write_all(entrada.as_bytes());
        drop(stdin);

        let mut matado = false;
        let mut cortado = false;
        let estado = loop {
            match hijo.try_wait() {
                Ok(Some(e)) => break Some(e),
                Ok(None) => {}
                Err(_) => break None,
            }
            let t = inicio.elapsed();
            if let Some(c) = cortar_a
                && !cortado
                && t >= c
            {
                cortado = true;
                let _ = Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status();
            }
            if t >= plazo {
                matado = true;
                let _ = hijo.kill();
                break hijo.wait().ok();
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        vivo.store(false, Ordering::Relaxed);
        let _ = vigia.join();
        let stdout = lector.join().unwrap_or_default();
        let stderr = errores.join().unwrap_or_default();
        let ms = inicio.elapsed().as_millis();

        self.anotar(stderr.clone());
        if !stdout.starts_with(&[0xFF, 0xFF, 0xFF, 0xFF]) {
            self.anotar(String::from_utf8_lossy(&stdout).into_owned());
        }
        let p = primer.load(Ordering::Relaxed);
        let k = pico.load(Ordering::Relaxed);
        Salida {
            ok: !matado && estado.is_some_and(|e| e.success()),
            stdout,
            stderr,
            ms,
            primer_byte_ms: (p != u64::MAX).then_some(u128::from(p)),
            pico_kib: (k > 0).then_some(k),
            matado,
        }
    }
}
