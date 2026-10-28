//! **La puerta de los punteros en `ore`** (0079 §19.1, E1a·5b·2 y ·5b·5): lo
//! único de `ore-cli` que sabe dónde viven. Cada verbo la abre una vez
//! ([`abrir`]) sobre la raíz del árbol, `--informe` (la carpeta) y
//! `--reclaman` (lo de las demás ramas), y pasa `&dyn Punteros` hacia abajo.
//!
//! | | Qué |
//! |---|---|
//! | [`Local`] | `Ficheros` de `ore_core`: escribe entero o nada (0073 §9 H4) y no deja carpetas vacías al retirar |
//! | [`Proceso`] | la misma puerta **al otro lado de un proceso** (`ore_core::punteros::protocolo`, donde está el protocolo): el binario que diga `ORE_PUNTEROS`, un hijo por ejecución, por líneas JSON |
//! | [`servir`] | `ore punteros-servir`: el servidor de referencia del protocolo, sobre [`Local`] |
//!
//!
//! El protocolo, su cliente y su servidor viven en `ore_core::punteros::protocolo`.

use std::collections::BTreeSet;
use std::path::Path;

use ore_core::json::Json;
use ore_core::parse::Node;
use ore_core::punteros::protocolo::{Apertura, Proceso, Puerta, Servidor};
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
        Some(orden) => Box::new(Proceso::hijo(
            &orden,
            raiz,
            carpeta,
            ajenos,
            std::env::var("ORE_REF").ok().filter(|r| !r.is_empty()),
        )),
        None => Box::new(Local::nuevo(raiz, carpeta, ajenos)),
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
