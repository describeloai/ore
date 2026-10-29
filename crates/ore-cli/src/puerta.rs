//! **La puerta de los punteros en `ore`** (0079 §19.1, E1a·5b·2 y ·5b·5): lo
//! único de `ore-cli` que sabe dónde viven. Cada verbo la abre una vez
//! ([`abrir`]) sobre la raíz del árbol, `--informe` (la carpeta) y
//! `--reclaman` (lo de las demás ramas), y pasa `&dyn Punteros` hacia abajo.
//!
//! | | Qué |
//! |---|---|
//! | [`Local`] | `Ficheros` de `ore_core`: escribe entero o nada (0073 §9 H4) y no deja carpetas vacías al retirar |
//! | [`Proceso`] | la misma puerta **al otro lado de un proceso** (`ore_core::punteros::protocolo`, donde está el protocolo): el binario que diga `ORE_PUNTEROS`, un hijo por ejecución, por líneas JSON |
//! | **la puerta heredada** | `ORE_PUNTEROS=fd:<lee>,<escribe>` (0079 ·5c·3): el mismo protocolo por dos tuberías que `ore` hereda de ore-serve, que le sirve la puerta desde su lado ([`heredada`]) |
//! | [`servir`] | `ore punteros-servir`: el servidor de referencia del protocolo, sobre [`Local`] |
//!
//!
//! El protocolo, su cliente y su servidor viven en `ore_core::punteros::protocolo`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use ore_core::json::Json;
use ore_core::parse::Node;
use ore_core::punteros::protocolo::{Apertura, Canal, Proceso, Puerta, Servidor, Tuberias};
use ore_core::punteros::{De, Esperado, Fallo, Ficheros, Punteros, Reclamados};

pub use ore_core::punteros::protocolo::VARIABLE;

/// **La puerta de esta ejecución**: [`Proceso`] si `ORE_PUNTEROS` lo dice, y si
/// no, [`Local`] sobre `raiz` (los punteros en `carpeta`, o en su sitio de
/// siempre) con lo que reclaman las demás ramas en `ajenos`.
pub fn abrir(raiz: &Path, carpeta: Option<&Path>, ajenos: Option<&Path>) -> Box<dyn Punteros> {
    match std::env::var(VARIABLE)
        .ok()
        .filter(|o| !o.trim().is_empty())
    {
        Some(orden) => {
            let rama = std::env::var("ORE_REF").ok().filter(|r| !r.is_empty());
            match orden.trim().strip_prefix("fd:") {
                Some(fds) => Box::new(Proceso::por(heredada(fds), raiz, carpeta, ajenos, rama)),
                None => Box::new(Proceso::hijo(&orden, raiz, carpeta, ajenos, rama)),
            }
        }
        None => Box::new(Local::nuevo(raiz, carpeta, ajenos)),
    }
}

// ── La puerta heredada ──────────────────────────────────────────────────────

/// **El canal de la puerta heredada** (`fd:<lee>,<escribe>`, 0079 ·5c·3): ore-serve
/// lanzó este `ore` con dos tuberías y le sirve el protocolo por ellas.
///
/// - **Un solo par por proceso, compartido.** Los descriptores son de este
///   proceso una vez: se toman la primera vez ([`TUBERIAS`]) y cada puerta que
///   se abra después habla por las mismas.
/// - **Varias puertas, un servidor.** El protocolo no tiene sesiones: el
///   servidor sirve la puerta del último `abrir`. Si un verbo abre dos (otra
///   carpeta, otros ajenos), cada una vuelve a mandar su `abrir` antes de
///   preguntar cuando la última abierta fue la otra ([`Turno`]).
/// - **`O_CLOEXEC` otra vez:** ore-serve se la quitó para que pasaran el
///   `exec`; los hijos de `ore` (`ore-store-<gcs>`) no se las llevan, o
///   ore-serve no vería el final de la tubería al acabar `ore`.
/// - Lo que no sirve (`fd:` mal escrito, un descriptor cerrado, Windows) es
///   una puerta que contesta su fallo a todo (503), como un almacén caído.
pub fn heredada(fds: &str) -> Box<dyn Canal> {
    match TUBERIAS.get_or_init(|| tomar(fds).map(Arc::new)) {
        Ok(c) => Box::new(Turno {
            par: Arc::clone(c),
            mia: OnceLock::new(),
        }),
        Err(m) => Box::new(Rota(m.clone())),
    }
}

static TUBERIAS: OnceLock<Result<Arc<Compartidas>, String>> = OnceLock::new();

/// El par de tuberías, y el `abrir` que el servidor tiene vigente.
struct Compartidas(Mutex<(Tuberias, Option<String>)>);

#[cfg(unix)]
fn tomar(fds: &str) -> Result<Compartidas, String> {
    use std::os::fd::FromRawFd;
    let mal = || format!("`{VARIABLE}=fd:{fds}`: se espera `fd:<lee>,<escribe>`");
    let (l, e) = fds.split_once(',').ok_or_else(mal)?;
    let lee: i32 = l.trim().parse().map_err(|_| mal())?;
    let escribe: i32 = e.trim().parse().map_err(|_| mal())?;
    if lee < 3 || escribe < 3 || lee == escribe {
        return Err(mal());
    }
    for fd in [lee, escribe] {
        cerrar_al_exec(fd).map_err(|e| format!("`{VARIABLE}`: el descriptor {fd}: {e}"))?;
    }
    // SAFETY: los dos existen (`fcntl` los acaba de ver), son distintos, y se
    // toman una sola vez por proceso (`TUBERIAS`): nadie más los cierra.
    let (lector, escritor) = unsafe {
        (
            std::fs::File::from_raw_fd(lee),
            std::fs::File::from_raw_fd(escribe),
        )
    };
    Ok(Compartidas(Mutex::new((
        Tuberias::nuevas(Box::new(lector), Box::new(escritor)),
        None,
    ))))
}

#[cfg(not(unix))]
fn tomar(_: &str) -> Result<Compartidas, String> {
    Err(format!(
        "`{VARIABLE}=fd:…`: la puerta heredada sólo existe en unix"
    ))
}

/// Pone `FD_CLOEXEC` en `fd`, y de paso comprueba que está abierto.
#[cfg(unix)]
fn cerrar_al_exec(fd: i32) -> std::io::Result<()> {
    use std::ffi::c_int;
    const F_GETFD: c_int = 1;
    const F_SETFD: c_int = 2;
    const FD_CLOEXEC: c_int = 1;
    unsafe extern "C" {
        /// `fcntl(2)`, de la libc con la que `std` ya enlaza (sin crate `libc`).
        fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    }
    // SAFETY: F_GETFD y F_SETFD sobre un entero; no tocan memoria.
    unsafe {
        let marcas = fcntl(fd, F_GETFD);
        if marcas < 0 || fcntl(fd, F_SETFD, marcas | FD_CLOEXEC) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Lo que una puerta habla por el par compartido: su `abrir` (la primera línea
/// que manda [`Proceso`]) y, antes de cada pregunta, otra vez si el servidor
/// tiene abierta la de otra.
struct Turno {
    par: Arc<Compartidas>,
    mia: OnceLock<String>,
}

impl Canal for Turno {
    fn ida_y_vuelta(&self, linea: &str) -> Result<String, Fallo> {
        let mut g = self
            .par
            .0
            .lock()
            .map_err(|_| Fallo::Almacen("la puerta heredada quedó rota".into()))?;
        let (tuberias, vigente) = &mut *g;
        let Some(mia) = self.mia.get() else {
            // La primera línea de un `Proceso` es su `abrir`.
            let vuelta = tuberias.ida_y_vuelta(linea)?;
            let _ = self.mia.set(linea.to_string());
            *vigente = Some(linea.to_string());
            return Ok(vuelta);
        };
        if vigente.as_deref() != Some(mia.as_str()) {
            let vuelta = tuberias.ida_y_vuelta(mia)?;
            if !vuelta.trim_start().starts_with("{\"ok\"") {
                *vigente = None;
                return Ok(vuelta);
            }
            *vigente = Some(mia.clone());
        }
        tuberias.ida_y_vuelta(linea)
    }
}

/// Una puerta heredada que no se pudo tomar: su fallo, a todo.
struct Rota(String);

impl Canal for Rota {
    fn ida_y_vuelta(&self, _: &str) -> Result<String, Fallo> {
        Err(Fallo::Almacen(self.0.clone()))
    }
}

/// El puntero de `nombre`, si lo hay. Un nombre que no lo es no tiene puntero
/// (como antes: no se encontraba el fichero); lo demás es un fallo.
pub fn leer(p: &dyn Punteros, nombre: &str) -> Result<Option<Node>, Fallo> {
    match p.uno(nombre) {
        Ok(l) => Ok(l.map(|l| l.puntero)),
        Err(Fallo::Nombre(_)) => Ok(None),
        Err(f) => Err(f),
    }
}

/// Un campo escalar no vacío.
fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// El *compare-and-set* de quien escribe sobre lo que leyó: que no hubiera
/// nada, que siga apuntando a la misma `metadata_location`, o —un puntero sin
/// ella (un error, un sobre heredado)— sin comprobar.
pub fn esperado_de(previo: Option<&Node>) -> Esperado {
    match previo {
        None => Esperado::Nada,
        Some(p) => campo(p, "metadata_location").map_or(Esperado::Cualquiera, Esperado::Ml),
    }
}

/// **Lo que reclaman las demás refs**, para `tambien` (lo que una rama nombra
/// de un dataset no se recoge) y para las colecciones de las demás ramas.
///
/// [`Punteros::reclamados`] es lo que nombra **cualquier** ref viva, esta
/// incluida; antes `--reclaman` traía sólo las demás. Lo propio sale de lo que
/// esta ref escribió (`De::Aqui`) al empezar: lo heredado de `main` en una rama
/// sigue contando como de otra (lo es: `main` lo nombra), como cuando el Job
/// de la rama ponía `main` entero en `--reclaman`. Sin quitarlo, la versión
/// que una pasada deja atrás contaría viva y lo suyo esperaría a la siguiente.
pub struct Ajenos {
    pub reclamados: Reclamados,
    propios: BTreeSet<String>,
}

impl Ajenos {
    pub fn de(p: &dyn Punteros) -> Result<Ajenos, Fallo> {
        let reclamados = p.reclamados()?;
        let propios = p
            .todos()?
            .into_values()
            .filter(|l| l.de == De::Aqui)
            .filter_map(|l| campo(&l.puntero, "metadata_location"))
            .collect();
        Ok(Ajenos {
            reclamados,
            propios,
        })
    }

    /// Las `metadata_location` de `dataset` que nombran las demás refs.
    pub fn de_dataset(&self, dataset: &str) -> Json {
        Json::Arr(
            self.reclamados
                .ubicaciones
                .get(dataset)
                .into_iter()
                .flatten()
                .filter(|m| !self.propios.contains(*m))
                .map(Json::s)
                .collect(),
        )
    }

    /// Las colecciones de las demás refs, `(dataset, metadata_location)`.
    pub fn colecciones(&self) -> impl Iterator<Item = &(String, String)> {
        self.reclamados
            .colecciones
            .iter()
            .filter(|(_, m)| !self.propios.contains(m))
    }
}

// ── Local ───────────────────────────────────────────────────────────────────

/// **La puerta sin proceso**: [`Ficheros`] de `ore_core` sobre `raiz`, los
/// punteros en `carpeta` (o en su sitio, `<raiz>/datasets`) y lo que reclaman
/// las demás ramas en `ajenos`. Escribe entero o nada y no deja carpetas vacías
/// al retirar (lo hace `Ficheros`, 0079 ·5b·4).
pub struct Local;

impl Local {
    pub fn nuevo(raiz: &Path, carpeta: Option<&Path>, ajenos: Option<&Path>) -> Ficheros {
        let f = Ficheros::en(raiz).con_ajenos(ajenos);
        match carpeta {
            Some(c) => f.con_carpeta(c),
            None => f,
        }
    }
}

/// **`ore punteros-servir`**: el protocolo por stdin y stdout, sobre [`Local`].
/// **`ore puntero <nombre>`** (0079 ·5d): el puntero de `nombre` en el árbol `raiz`, por la
/// puerta (la de `ORE_PUNTEROS`, si la hay). El texto tal cual en stdout; 66 si no lo hay, 65
/// si no es un nombre, 69 si el almacén no contesta.
pub fn uno(raiz: &Path, nombre: &str) -> std::process::ExitCode {
    use ore_core::punteros::Fallo;
    match abrir(raiz, None, None).uno(nombre) {
        Ok(Some(l)) => {
            println!("{}", l.texto.trim_end());
            std::process::ExitCode::SUCCESS
        }
        Ok(None) => std::process::ExitCode::from(66),
        Err(Fallo::Nombre(n)) => {
            eprintln!("error: `{n}` no es `base.nombre` ni `base.schema.nombre`");
            std::process::ExitCode::from(65)
        }
        Err(e) => {
            eprintln!("error: el puntero de `{nombre}`: {e}");
            std::process::ExitCode::from(69)
        }
    }
}

pub fn servir() -> std::process::ExitCode {
    let mut s = Servidor::nuevo(Box::new(|a: &Apertura| {
        Ok(Box::new(Local::nuevo(
            &a.raiz,
            a.carpeta.as_deref(),
            a.ajenos.as_deref(),
        )) as Box<dyn Puerta>)
    }));
    match ore_core::punteros::protocolo::servir(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        &mut s,
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::from(74),
    }
}
