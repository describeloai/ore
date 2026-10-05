//! **Una lectura**: la petición a un `servir` caliente, su flujo Arrow con el
//! presupuesto cumplido, y cómo terminó.
//!
//! Tres hilos, y por qué:
//!
//! - **el que lee el marco** del conector (`servir` trocea su respuesta con
//!   longitudes) y pasa los trozos; al acabar devuelve el proceso al fondo si el
//!   marco quedó entero. Lee hasta el cierre **siempre**, también si la
//!   lectura se cortó: un marco a medias haría inservible el proceso;
//! - **el que cuenta**: lee el flujo Arrow lote a lote, corta en la fila exacta
//!   (filas), antes del lote que pasaría (bytes) o cuando el reloj lo dice
//!   (tiempo), y lo vuelve a escribir hacia quien pidió. Cortar es además
//!   **cancelar en el origen** (`{"cancelar": id}`): el presupuesto protege la
//!   base del cliente, no sólo la memoria de la pasarela;
//! - **el reloj**: a `ms` corta por tiempo y, si el conector no suelta en
//!   `soltar`, lo mata.
//!
//! Si quien pidió se va, escribirle falla y eso también corta y cancela.

use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use ore_driver::{Codigo, Fallo};

use crate::cotas::Presupuesto;
use crate::fondo::{Fondo, Plaza};

/// Cómo terminó una lectura (los *trailers* y `GET /v1/read/{id}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Final {
    /// `completo`, `cortado` o `error`.
    pub estado: &'static str,
    /// Por qué se cortó (`filas`, `bytes`, `tiempo`, `cancelada`,
    /// `desconexion`) o el error del conector (`codigo: mensaje`, tapado).
    pub motivo: String,
    pub filas: u64,
    pub bytes: u64,
    pub ms: u64,
}

/// **El mando de una lectura en curso**: cortarla, y saber por qué se cortó.
pub struct Control {
    pub id: String,
    pub origen: String,
    pub empezo: Instant,
    motivo: Mutex<Option<String>>,
    entrada: Mutex<Option<Arc<Mutex<std::process::ChildStdin>>>>,
    pid: AtomicU64,
    terminada: AtomicBool,
    pub filas: AtomicU64,
    pub bytes: AtomicU64,
}

impl Control {
    pub fn new(id: &str, origen: &str) -> Arc<Control> {
        Arc::new(Control {
            id: id.to_string(),
            origen: origen.to_string(),
            empezo: Instant::now(),
            motivo: Mutex::new(None),
            entrada: Mutex::new(None),
            pid: AtomicU64::new(0),
            terminada: AtomicBool::new(false),
            filas: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        })
    }

    /// **Corta**: el primer motivo es el que queda, y el conector recibe
    /// `{"cancelar": id}` para que corte en el origen. `false` si ya estaba
    /// cortada o terminada.
    pub fn cortar(&self, motivo: &str) -> bool {
        if self.terminada.load(Ordering::SeqCst) {
            return false;
        }
        {
            let mut m = self.motivo.lock().expect("motivo");
            if m.is_some() {
                return false;
            }
            *m = Some(motivo.to_string());
        }
        if let Some(e) = self.entrada.lock().expect("entrada").as_ref()
            && let Ok(mut e) = e.lock()
        {
            let linea = ore_core::json::Json::obj([(
                "cancelar",
                ore_core::json::Json::s(self.id.as_str()),
            )]);
            let _ = e.write_all(format!("{}\n", linea.jcs()).as_bytes());
            let _ = e.flush();
        }
        true
    }

    pub fn motivo(&self) -> Option<String> {
        self.motivo.lock().expect("motivo").clone()
    }

    pub fn terminada(&self) -> bool {
        self.terminada.load(Ordering::SeqCst)
    }
}

/// Lo que devuelve [`empezar`].
pub enum Inicio {
    /// Falló antes del primer byte: un estado HTTP y el fallo, en JSON.
    Error {
        http: u16,
        fallo: Fallo,
        final_: Final,
    },
    /// Empezó: el flujo Arrow, y dónde estará cómo terminó cuando termine.
    Flujo {
        lector: Box<dyn Read + Send>,
        final_: Arc<Mutex<Option<Final>>>,
    },
}

/// El estado HTTP de un fallo antes del primer byte (`docs/federation.md` §3).
pub fn http_de(c: Codigo) -> u16 {
    match c {
        Codigo::Operador => 400,
        Codigo::Objeto => 404,
        Codigo::Tiempo => 504,
        Codigo::Credencial | Codigo::Conexion | Codigo::Origen => 502,
    }
}

/// **Empieza una lectura**: manda `linea` al proceso de la plaza, espera a su
/// cabecera y, si empezó, deja los hilos corriendo y devuelve el flujo.
/// `al_terminar` se llama una vez, con el final, sea cual sea.
pub fn empezar(
    fondo: &Arc<Fondo>,
    mut plaza: Plaza,
    linea: &str,
    control: Arc<Control>,
    presupuesto: Presupuesto,
    al_terminar: Box<dyn FnOnce(&Final) + Send>,
) -> Inicio {
    let mut proceso = plaza.proceso.take().expect("una plaza trae su proceso");
    *control.entrada.lock().expect("entrada") = Some(proceso.entrada.clone());
    control
        .pid
        .store(u64::from(proceso.pid()), Ordering::SeqCst);
    reloj(&control, presupuesto.ms, fondo.cotas.soltar);

    let fin_error =
        |control: &Control, fallo: Fallo, al_terminar: Box<dyn FnOnce(&Final) + Send>| {
            let motivo = control
                .motivo()
                .or_else(|| (fallo.codigo == Codigo::Tiempo).then(|| "tiempo".to_string()));
            let (http, fallo) = match motivo.as_deref() {
                Some("tiempo") => (
                    504,
                    Fallo::new(
                        Codigo::Tiempo,
                        format!("se agotó el tiempo de la lectura ({} ms)", presupuesto.ms),
                    ),
                ),
                Some("cancelada") => (409, Fallo::origen("la lectura se canceló")),
                _ => (http_de(fallo.codigo), fallo),
            };
            let final_ = Final {
                estado: if motivo.is_some() { "cortado" } else { "error" },
                motivo: motivo
                    .unwrap_or_else(|| format!("{}: {}", fallo.codigo.as_str(), fallo.mensaje)),
                filas: 0,
                bytes: 0,
                ms: control.empezo.elapsed().as_millis() as u64,
            };
            control.terminada.store(true, Ordering::SeqCst);
            fondo.anotar(&control.origen, final_.estado, final_.ms);
            al_terminar(&final_);
            Inicio::Error {
                http,
                fallo,
                final_,
            }
        };

    // Un conector caliente cuya conexión murió (el origen se reinició, alguien
    // la terminó) contesta `conexion` antes del primer byte: se relanza UNA
    // vez. Leer es repetible y aún no salió nada hacia quien pidió.
    let mut intento = 0;
    loop {
        intento += 1;
        if let Err(e) = proceso.pedir(linea) {
            proceso.matar();
            fondo.soltar(plaza, false);
            return fin_error(
                &control,
                Fallo::new(
                    Codigo::Conexion,
                    format!("el conector no acepta la petición: {e}"),
                ),
                al_terminar,
            );
        }

        // La cabecera: `{"estado":"ok",…}` o el error entero en una línea.
        let mut cabecera = String::new();
        match proceso.salida.read_line(&mut cabecera) {
            Ok(0) | Err(_) => {
                proceso.matar();
                fondo.soltar(plaza, false);
                return fin_error(
                    &control,
                    Fallo::new(Codigo::Conexion, "el conector terminó sin contestar"),
                    al_terminar,
                );
            }
            Ok(_) => {}
        }
        let n = match ore_core::parse::parse(cabecera.trim()) {
            Ok(n) => n,
            Err(_) => {
                proceso.matar();
                fondo.soltar(plaza, false);
                return fin_error(
                    &control,
                    Fallo::origen("el conector contestó algo que no es su marco"),
                    al_terminar,
                );
            }
        };
        if n.get("estado").and_then(|(_, v)| v.as_str()) != Some("error") {
            break;
        }
        // Un error en una línea deja el marco entero: el proceso sigue sirviendo.
        let fallo = Fallo::de_nodo(&n).unwrap_or_else(|| Fallo::origen("un error sin código"));
        let relanzable = fallo.codigo == Codigo::Conexion
            && plaza.caliente
            && intento == 1
            && control.motivo().is_none();
        plaza.proceso = Some(proceso);
        if !relanzable {
            fondo.soltar(plaza, true);
            return fin_error(&control, fallo, al_terminar);
        }
        if let Err(e) = plaza.relanzar() {
            fondo.soltar(plaza, false);
            return fin_error(
                &control,
                Fallo::new(Codigo::Conexion, format!("no se relanzó el conector: {e}")),
                al_terminar,
            );
        }
        proceso = plaza.proceso.take().expect("recién relanzado");
        *control.entrada.lock().expect("entrada") = Some(proceso.entrada.clone());
        control
            .pid
            .store(u64::from(proceso.pid()), Ordering::SeqCst);
    }

    // Empezó. Del marco al contador, y del contador a quien pidió.
    let (a_contar, de_marco) = sync_channel::<Vec<u8>>(8);
    let (a_http, de_contar) = sync_channel::<Vec<u8>>(8);
    let fin_conector: Arc<Mutex<Option<Result<(), Fallo>>>> = Arc::new(Mutex::new(None));
    let final_: Arc<Mutex<Option<Final>>> = Arc::new(Mutex::new(None));

    {
        let fondo = Arc::clone(fondo);
        let fin_conector = fin_conector.clone();
        std::thread::spawn(move || {
            let r = leer_marco(&mut proceso.salida, &a_contar);
            let sano = r.is_ok();
            *fin_conector.lock().expect("fin") = Some(match r {
                Ok(fin) => fin,
                Err(e) => Err(Fallo::new(Codigo::Conexion, e)),
            });
            drop(a_contar);
            if sano {
                plaza.proceso = Some(proceso);
                fondo.soltar(plaza, true);
            } else {
                proceso.matar();
                fondo.soltar(plaza, false);
            }
        });
    }
    {
        let control = control.clone();
        let final_ = final_.clone();
        let fondo = Arc::clone(fondo);
        std::thread::spawn(move || {
            // ⛔ Un emisor se guarda hasta tener el final: si el canal se
            //   cerrara al salir de `contar`, quien lee pediría los *trailers*
            //   antes de que existan.
            let guarda = a_http.clone();
            let (estado, motivo) = contar(&control, de_marco, a_http, presupuesto);
            // El marco termina de leerse antes de decir cómo acabó.
            let fin = loop {
                if let Some(f) = fin_conector.lock().expect("fin").take() {
                    break f;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            };
            let (estado, motivo) = match (estado, fin) {
                (Some(e), _) => (e, motivo),
                // El `timeoutMs` que se empujó ES el presupuesto de tiempo: si
                // salta el del conector antes que el reloj, es el mismo corte.
                (None, Err(f)) if f.codigo == Codigo::Tiempo => ("cortado", "tiempo".to_string()),
                (None, Err(f)) => ("error", format!("{}: {}", f.codigo.as_str(), f.mensaje)),
                (None, Ok(())) => ("completo", String::new()),
            };
            let f = Final {
                estado,
                motivo,
                filas: control.filas.load(Ordering::SeqCst),
                bytes: control.bytes.load(Ordering::SeqCst),
                ms: control.empezo.elapsed().as_millis() as u64,
            };
            control.terminada.store(true, Ordering::SeqCst);
            fondo.anotar(&control.origen, f.estado, f.ms);
            *final_.lock().expect("final") = Some(f.clone());
            al_terminar(&f);
            // Ya con el final puesto: quien lee ve el fin y pide los *trailers*.
            drop(guarda);
        });
    }
    Inicio::Flujo {
        lector: Box::new(Canal {
            de: de_contar,
            trozo: Vec::new(),
            pos: 0,
        }),
        final_,
    }
}

/// El reloj: a `ms` corta por tiempo; si en `soltar` el conector sigue sin
/// terminar, lo mata.
fn reloj(control: &Arc<Control>, ms: u64, soltar: std::time::Duration) {
    let control = control.clone();
    std::thread::spawn(move || {
        let hasta = control.empezo + std::time::Duration::from_millis(ms);
        while Instant::now() < hasta {
            if control.terminada() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if control.terminada() {
            return;
        }
        control.cortar("tiempo");
        let limite = Instant::now() + soltar;
        while Instant::now() < limite {
            if control.terminada() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !control.terminada() {
            matar(control.pid.load(Ordering::SeqCst) as u32);
        }
    });
}

#[cfg(unix)]
fn matar(pid: u32) {
    if pid > 0 {
        // SAFETY: `kill` con un pid de un hijo nuestro y SIGKILL; si ya no
        // existe, falla sin efecto.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn matar(_pid: u32) {}

/// Lee el resto del marco de `servir` (los trozos y el cierre) y pasa los
/// trozos. Si el contador ya no los quiere, se siguen leyendo y se tiran: el
/// marco tiene que acabar entero. `Err` si el marco se rompió.
fn leer_marco<R: BufRead>(
    salida: &mut R,
    a_contar: &SyncSender<Vec<u8>>,
) -> Result<Result<(), Fallo>, String> {
    let roto = |e: std::io::Error| format!("el marco del conector está roto: {e}");
    let mut quiere = true;
    let mut largo = [0u8; 4];
    loop {
        salida.read_exact(&mut largo).map_err(roto)?;
        let n = u32::from_be_bytes(largo) as usize;
        if n == 0 {
            break;
        }
        let mut trozo = vec![0u8; n];
        salida.read_exact(&mut trozo).map_err(roto)?;
        if quiere && a_contar.send(trozo).is_err() {
            quiere = false;
        }
    }
    let mut l = String::new();
    if salida.read_line(&mut l).map_err(roto)? == 0 {
        return Err("el marco acaba sin su cierre".into());
    }
    let f = ore_core::parse::parse(l.trim()).map_err(|e| format!("el cierre no es JSON: {e:?}"))?;
    match f.get("fin").and_then(|(_, v)| v.as_str()) {
        Some("ok") => Ok(Ok(())),
        Some("error") => {
            Ok(Err(Fallo::de_nodo(&f).unwrap_or_else(|| {
                Fallo::origen("un cierre con error sin código")
            })))
        }
        _ => Err("un cierre sin `fin`".into()),
    }
}

/// **El contador**: lee el Arrow que llega, cumple el presupuesto y escribe a
/// quien pidió. Devuelve `Some(("cortado", motivo))` si cortó, `None` si el
/// flujo acabó solo (y entonces manda lo que diga el conector).
fn contar(
    control: &Control,
    de_marco: Receiver<Vec<u8>>,
    a_http: SyncSender<Vec<u8>>,
    p: Presupuesto,
) -> (Option<&'static str>, String) {
    let entrada = Canal {
        de: de_marco,
        trozo: Vec::new(),
        pos: 0,
    };
    let mut lector = match StreamReader::try_new(entrada, None) {
        Ok(l) => l,
        Err(_) => {
            // Sin esquema: el conector falló justo después de la cabecera, o
            // se cortó antes de un lote. Lo dice el cierre, o el motivo.
            return match control.motivo() {
                Some(m) => (Some("cortado"), m),
                None => (None, String::new()),
            };
        }
    };
    let esquema = lector.schema();
    let salida = Envio {
        a: a_http,
        enviados: 0,
    };
    let mut escritor = match StreamWriter::try_new(
        std::io::BufWriter::with_capacity(64 * 1024, salida),
        &esquema,
    ) {
        Ok(w) => w,
        Err(_) => {
            control.cortar("desconexion");
            drenar(lector);
            return (Some("cortado"), "desconexion".into());
        }
    };
    let mut corte: Option<String> = None;
    let mut roto = false;
    let mut filas: u64 = 0;
    loop {
        if let Some(m) = control.motivo() {
            corte = Some(m);
            break;
        }
        let lote = match lector.next() {
            None => break,
            Some(Ok(l)) => l,
            Some(Err(_)) => {
                roto = true;
                break;
            }
        };
        let mut lote = lote;
        let queda = p.filas.saturating_sub(filas);
        let mut por_filas = false;
        if lote.num_rows() as u64 > queda {
            lote = lote.slice(0, queda as usize);
            por_filas = true;
        }
        // Por bytes: lo que cabe del lote, por su tamaño medio por fila.
        let enviados =
            escritor.get_ref().get_ref().enviados + escritor.get_ref().buffer().len() as u64;
        let mut por_bytes = false;
        let peso = lote.get_array_memory_size() as u64;
        if lote.num_rows() > 0 && enviados + peso > p.bytes {
            let por_fila = (peso / lote.num_rows() as u64).max(1);
            let caben = p.bytes.saturating_sub(enviados) / por_fila;
            lote = lote.slice(0, (caben as usize).min(lote.num_rows()));
            por_bytes = true;
        }
        if lote.num_rows() > 0 {
            let escrito = escritor.write(&lote).is_ok() && escritor.get_mut().flush().is_ok();
            if !escrito {
                control.cortar("desconexion");
                corte = Some("desconexion".into());
                break;
            }
            filas += lote.num_rows() as u64;
            control.filas.store(filas, Ordering::SeqCst);
            control
                .bytes
                .store(escritor.get_ref().get_ref().enviados, Ordering::SeqCst);
        }
        if por_bytes {
            control.cortar("bytes");
            corte = Some("bytes".into());
            break;
        }
        if por_filas {
            control.cortar("filas");
            corte = Some("filas".into());
            break;
        }
    }
    // Lo cortado también es un flujo Arrow válido, con su fin: quien lee tiene
    // las filas hasta el corte, y los *trailers* dicen por qué.
    if corte.as_deref() != Some("desconexion") && !(roto && corte.is_none()) {
        let _ = escritor.finish();
        if let Ok(mut b) = escritor.into_inner() {
            let _ = b.flush();
            control.bytes.store(b.get_ref().enviados, Ordering::SeqCst);
        }
    }
    drenar(lector);
    if roto && corte.is_none() {
        corte = control.motivo();
    }
    match corte {
        Some(m) => (Some("cortado"), m),
        None => (None, String::new()),
    }
}

/// Lo que queda del marco se lee y se tira: el hilo del marco tiene que llegar
/// al cierre para devolver el proceso.
fn drenar(mut lector: StreamReader<Canal>) {
    for _ in lector.get_mut().de.iter() {}
}

/// Un `Read` sobre trozos que llegan por un canal. Fin cuando el canal se cierra.
pub struct Canal {
    de: Receiver<Vec<u8>>,
    trozo: Vec<u8>,
    pos: usize,
}

impl Read for Canal {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.trozo.len() {
            match self.de.recv() {
                Ok(t) => {
                    self.trozo = t;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.trozo.len() - self.pos);
        buf[..n].copy_from_slice(&self.trozo[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Un `Write` que manda cada escritura por un canal; falla si el otro lado se
/// fue (quien pidió cerró la conexión).
struct Envio {
    a: SyncSender<Vec<u8>>,
    enviados: u64,
}

impl Write for Envio {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.a.send(buf.to_vec()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "quien pidió se fue")
        })?;
        self.enviados += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
