//! **El esqueleto de un origen v2**: lo que repite cualquier `ore-read-<tipo>`
//! y no es del origen.
//!
//! Un lector v2 hacía siempre lo mismo alrededor de su parte: despachar
//! `capacidades`/`leer`/`estimar`/`servir`, leer la petición de stdin, levantar
//! el `SIGTERM` que cancela en el origen, mantener una sesión por credencial en
//! `servir` y tirarla si se pierde la conexión. Eran ~130 líneas copiadas entre
//! `ore-read-postgres` y `ore-read-bigquery` (y su `senal.rs`, idéntico salvo la
//! llamada a cancelar). Con un origen nuevo por semana, copiar es la forma de
//! que diverjan.
//!
//! **Lo que escribe quien trae un origen** es solo esto: [`Origen`] —conectar,
//! leer a Arrow, estimar, cancelar y qué declara— y un `main` de una línea:
//!
//! ```ignore
//! fn main() -> std::process::ExitCode {
//!     ore_driver::origen::principal::<MiOrigen>(|verbo, args| /* catalogo, check… */)
//! }
//! ```
//!
//! Los verbos que dependen de cada fuente (`catalogo`, `check`, `explorar`,
//! `testigo`) siguen siendo del origen: [`principal`] se los pasa a `otros`.

use crate::capacidades::Capacidades;
use crate::{Codigo, Fallo, Peticion};
use std::io::{Read as _, Write};
use std::process::ExitCode;
use std::time::Duration;

/// Cuánto vive una sesión sin uso en `servir`.
pub const SESION_OCIOSA: Duration = Duration::from_secs(60);

pub trait Origen: 'static {
    /// Una conexión abierta (un `Client`, un cliente HTTP…). `servir` guarda una
    /// por credencial.
    type Sesion;
    /// Lo que une al que lee con quien cancela: la bandera, el `CancelToken`,
    /// el job. Se crea por proceso y se comparte entre hilos.
    type Token: Default + Clone + Send + Sync + 'static;

    const CAPACIDADES: Capacidades;

    fn conectar(url: &str) -> Result<Self::Sesion, Fallo>;

    /// Escribe el flujo **Arrow IPC** de la petición en `salida` y devuelve las
    /// filas. Es quien **rearma** el token al empezar: el de la lectura anterior
    /// puede venir cancelado.
    fn leer(
        sesion: &mut Self::Sesion,
        token: &Self::Token,
        p: &Peticion,
        salida: &mut dyn Write,
    ) -> Result<u64, Fallo>;

    /// La estimación, en la línea JSON que contesta el verbo `estimar`.
    fn estimar(sesion: &mut Self::Sesion, p: &Peticion) -> Result<String, Fallo>;

    /// Corta en el origen lo que corre ahora. Se llama desde otro hilo: al
    /// recibir `SIGTERM`, con `{"cancelar": id}` y al cerrarse la entrada.
    fn cancelar(token: &Self::Token);
}

/// Un fallo, como lo ve quien lanzó el proceso: una línea JSON por stderr, con
/// la credencial tapada.
fn fallar<O: Origen>(f: Fallo, url: &str) -> ExitCode {
    eprintln!("{}: {}", O::CAPACIDADES.conector, f.tapado(url).linea());
    ExitCode::FAILURE
}

/// El verbo v2 `verbo`, o `None` si no es de los cuatro (y lo atiende el origen).
pub fn v2<O: Origen>(verbo: &str) -> Option<ExitCode> {
    match verbo {
        "capacidades" => {
            println!("{}", O::CAPACIDADES.json());
            Some(ExitCode::SUCCESS)
        }
        "leer" | "estimar" => Some(una_vez::<O>(verbo)),
        "servir" => Some(servir::<O>()),
        _ => None,
    }
}

fn una_vez<O: Origen>(verbo: &str) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut entrada) {
        return fallar::<O>(Fallo::operador(format!("no se pudo leer stdin: {e}")), "");
    }
    let p = match crate::leer_peticion(&entrada) {
        Ok(p) => p,
        Err(e) => return fallar::<O>(Fallo::operador(e), ""),
    };
    if verbo == "estimar" && !O::CAPACIDADES.estimar {
        return fallar::<O>(Fallo::operador("este origen no estima"), &p.url);
    }
    let token = O::Token::default();
    al_terminar::<O>(token.clone());
    let hecho = O::conectar(&p.url).and_then(|mut s| {
        if verbo == "estimar" {
            println!("{}", O::estimar(&mut s, &p)?);
            return Ok(());
        }
        let mut salida = std::io::BufWriter::new(std::io::stdout().lock());
        O::leer(&mut s, &token, &p, &mut salida)?;
        salida
            .flush()
            .map_err(|e| Fallo::new(Codigo::Conexion, e.to_string()))
    });
    match hecho {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => fallar::<O>(f, &p.url),
    }
}

/// **`servir`**: peticiones una tras otra, con una sesión por credencial.
fn servir<O: Origen>() -> ExitCode {
    let token = O::Token::default();
    al_terminar::<O>(token.clone());
    let mut sesiones: crate::servir::Conexiones<O::Sesion> =
        crate::servir::Conexiones::new(SESION_OCIOSA);
    let para_cancelar = token.clone();
    let mut salida = std::io::BufWriter::new(std::io::stdout().lock());
    let hecho = crate::servir::servir(
        std::io::BufReader::new(std::io::stdin()),
        &mut salida,
        std::sync::Arc::new(move || O::cancelar(&para_cancelar)),
        |p, r| {
            let s = sesiones.tomar(&p.url, O::conectar)?;
            let leido = O::leer(s, &token, p, r);
            // Una sesión que perdió la conexión no se vuelve a usar.
            if leido.as_ref().is_err_and(|f| f.codigo == Codigo::Conexion) {
                sesiones.quitar(&p.url);
            }
            leido
        },
    );
    match hecho {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: servir: {e}", O::CAPACIDADES.conector);
            ExitCode::FAILURE
        }
    }
}

/// **`SIGTERM` cancela en el origen** (`docs/federation.md` §1.5). Matar el
/// proceso no basta: el origen sigue con la consulta, y a veces facturándola.
/// El manejador solo levanta una bandera (lo único seguro dentro de él); un
/// hilo la vigila, llama a [`Origen::cancelar`] y termina con 143, el código de
/// quien muere por `SIGTERM`.
#[cfg(unix)]
fn al_terminar<O: Origen>(token: O::Token) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RECIBIDA: AtomicBool = AtomicBool::new(false);
    extern "C" fn manejar(_: libc::c_int) {
        RECIBIDA.store(true, Ordering::SeqCst);
    }
    // SAFETY: `manejar` solo escribe un atómico.
    unsafe {
        libc::signal(libc::SIGTERM, manejar as *const () as libc::sighandler_t);
    }
    std::thread::spawn(move || {
        loop {
            if RECIBIDA.load(Ordering::SeqCst) {
                O::cancelar(&token);
                std::process::exit(143);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
}

/// Fuera de Unix no hay `SIGTERM`: quien para el proceso lo mata, y la
/// cancelación es la de `servir` (`{"cancelar": id}` o cerrar la entrada).
#[cfg(not(unix))]
fn al_terminar<O: Origen>(_: O::Token) {}

/// **El `main` de un origen**: los cuatro verbos v2 aquí, y el resto a `otros`
/// (que recibe el verbo y los argumentos que le siguen).
pub fn principal<O: Origen>(otros: impl FnOnce(&str, &[String]) -> ExitCode) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(verbo) = args.first() else {
        eprintln!(
            "{}: falta el verbo (capacidades, leer, estimar, servir, catalogo, check, explorar, testigo)",
            O::CAPACIDADES.conector
        );
        return ExitCode::FAILURE;
    };
    v2::<O>(verbo).unwrap_or_else(|| otros(verbo, &args[1..]))
}
