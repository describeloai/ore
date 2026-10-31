//! ⭐ **EL LUGAR DONDE SE TRABAJA** (0079 §19.1, E1a·5b·3): el directorio del
//! árbol, para los documentos, y **la puerta de los punteros** de esa ref
//! ([`ore_core::punteros::Punteros`]).
//!
//! Los manejadores siguen recibiendo un `&Path` de la costura de siempre
//! (`leyendo`, `leyendo_en`, `escribiendo`, `escribiendo_en`, `moviendo_datos`),
//! y lo que toca un puntero abre aquí su puerta: [`Lugar::de`] es **la única
//! función que la construye**, y [`heredada`] la que sirve a un `ore` hijo por
//! la puerta heredada (0079 §19.2 P1, `crate::heredada`).
//!
//! # Los punteros viven en el catálogo, y sólo ahí (0079 ·5g·3)
//!
//! El catálogo transaccional del inquilino: su esquema (`--catalogo-esquema t_<celda>`) en el
//! Postgres de `CATALOGO_URL`. La puerta de la ref en curso la abre [`abrir_catalogo`], con una
//! conexión del pool; las ramas son su tabla de refs; y a `ore` se le sirve siempre por la puerta
//! heredada (`ore` no abre sockets). Sin catálogo no hay punteros: [`Lugar::de`] da una puerta
//! cerrada que contesta su fallo, y `ore-serve` no arranca (se prueba al arrancar, [`comprobar`]).
//! `ficheros` (los punteros en el árbol de git) se retiró en ·5g·3.
//!
//! # La ref, por la costura y sin cambiar los cierres
//!
//! La ref la conoce la costura (la rama de `leyendo_en`/`escribiendo_en`); los
//! cierres siguen recibiendo un `&Path`. La costura la pone mientras corre el
//! cierre ([`EnRef::poner`], una marca del hilo como `copia::EnRama`), y
//! [`Lugar::de`] y `crate::heredada` la leen ([`ref_en_curso`]). `None` es
//! `main` (la rama por defecto; `Some("main")` se normaliza a `None`).
//!
//! ⚠️ Una marca de hilo: un manejador que abre un `Lugar` desde otro hilo
//!   (ninguno hoy) lo abriría en `main`: ese hilo tendría que llevarse la ref.
//!
//! | | Qué |
//! |---|---|
//! | [`Lugar::de`] | el directorio y su puerta |
//! | [`Lugar::puntero`] | el de un nombre, como lo leía `leer_en` (`None` si no está o no es un nombre) |
//! | [`Lugar::en_una_rama`] | si el árbol es el de una rama con lo no tocado leído de `main` |
//! | [`ramas`] | la puerta del ciclo de vida de las ramas (crear, borrar, traer `main`) |
//! | [`heredada`] | la puerta que se le sirve a un `ore` hijo, para su `abrir` |
//! | [`version_de_la_ref`] | la clave de las cachés que el commit no da |
//! | [`segundos`] | un `cuando` de [`ore_core::punteros::Version`] a segundos de época |

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use ore_core::parse::Node;
use ore_core::punteros::grabadora::{Cinta, Grabadora};
use ore_core::punteros::protocolo::{Apertura, Puerta};
use ore_core::punteros::{Cambio, Contexto, Fallo, Leido, Punteros, Ramas, Reclamados, Version};
use ore_entrada::http::Respuesta;

// ── El catálogo ─────────────────────────────────────────────────────────────

/// Dónde viven los punteros de este inquilino: el esquema del inquilino (`--catalogo-esquema
/// t_<celda>`) en el Postgres de `CATALOGO_URL` (0079).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Catalogo {
    pub(crate) esquema: String,
    pub(crate) url: String,
}

static CATALOGO: OnceLock<Catalogo> = OnceLock::new();

/// **El catálogo de este ore-serve**, una vez, al arrancar (`main`).
pub(crate) fn configurar(c: Catalogo) -> Result<(), String> {
    CATALOGO
        .set(c)
        .map_err(|_| "el catálogo de los punteros ya estaba elegido".to_string())
}

/// El catálogo, o por qué no lo hay. En las pruebas, el de cada una ([`de_la_prueba`]).
fn catalogo() -> Result<&'static Catalogo, Fallo> {
    #[cfg(test)]
    if let Some(c) = de_la_prueba::catalogo() {
        return Ok(c);
    }
    CATALOGO.get().ok_or_else(|| {
        Fallo::Almacen(if cfg!(test) {
            "sin ORE_CATALOGO_PRUEBAS: los punteros viven en el catálogo, y esta prueba necesita uno".into()
        } else {
            "sin catálogo: ore-serve arranca con --catalogo-esquema y su URL".into()
        })
    })
}

/// ⭐ 0079 ·5g·3 · **El catálogo de cada prueba.** Con `ORE_CATALOGO_PRUEBAS` (la URL de un Postgres
/// donde se puedan crear esquemas), cada prueba trabaja en el suyo, como en producción cada celda: el
/// de su hilo, que libtest nombra con la prueba. Un hilo que un manejador lance se lleva el de quien
/// lo lanzó ([`Llevado`]). Sin la variable, una prueba que toca punteros falla diciéndolo.
#[cfg(test)]
pub(crate) mod de_la_prueba {
    use super::Catalogo;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    thread_local! {
        /// El esquema que un hilo lanzado heredó de quien lo lanzó.
        static LLEVADO: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    fn url() -> Option<&'static str> {
        static U: OnceLock<Option<String>> = OnceLock::new();
        U.get_or_init(|| {
            std::env::var("ORE_CATALOGO_PRUEBAS")
                .ok()
                .filter(|u| !u.trim().is_empty())
        })
        .as_deref()
    }

    /// El esquema de este hilo: el llevado, o uno por el nombre de la prueba.
    pub(crate) fn esquema() -> Option<String> {
        url()?;
        if let Some(e) = LLEVADO.with(|l| l.borrow().clone()) {
            return Some(e);
        }
        let hilo = std::thread::current();
        let nombre = hilo.name().unwrap_or("sin-nombre");
        // FNV-1a: estable entre corridas, para poder mirar el esquema de una prueba que falló.
        let h = nombre.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
        Some(format!("t_p{:016x}_{}", h, std::process::id()))
    }

    pub(super) fn catalogo() -> Option<&'static Catalogo> {
        static HECHAS: OnceLock<Mutex<BTreeMap<String, &'static Catalogo>>> = OnceLock::new();
        let esquema = esquema()?;
        let mut h = HECHAS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(i) = h.get(&esquema) {
            return Some(i);
        }
        let url = url()?.to_string();
        let mut c = postgres::Client::connect(&url, postgres::NoTls)
            .expect("ORE_CATALOGO_PRUEBAS: conectar");
        c.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {esquema}"))
            .expect("crear el esquema de la prueba");
        let i: &'static Catalogo = Box::leak(Box::new(Catalogo {
            esquema: esquema.clone(),
            url,
        }));
        h.insert(esquema, i);
        Some(i)
    }

    /// **Un puntero de prueba, por la puerta** (`nombre` corto: `base.n` o `base.schema.n`): las
    /// pruebas preparan su estado como lo escribe el producto, sea cual sea el almacén.
    pub(crate) fn poner_puntero(raiz: &std::path::Path, nombre: &str, puntero: &str) {
        let j = ore_core::parse::parse(puntero)
            .map(|n| ore_core::json::Json::de_node(&n))
            .expect("el puntero de la prueba es JSON");
        super::Lugar::de(raiz)
            .punteros()
            .escribir(
                &[ore_core::punteros::Cambio {
                    nombre: nombre.to_string(),
                    que: ore_core::punteros::Que::Sustituir(j),
                    esperado: ore_core::punteros::Esperado::Cualquiera,
                }],
                &ore_core::punteros::Contexto {
                    autor: "prueba".into(),
                    operacion: "prueba".into(),
                    idempotencia: None,
                },
            )
            .unwrap_or_else(|e| panic!("poner el puntero de `{nombre}`: {e}"));
    }

    /// El puntero de `nombre` tal como lo guarda el almacén, o `None`.
    pub(crate) fn leer_puntero(raiz: &std::path::Path, nombre: &str) -> Option<String> {
        super::Lugar::de(raiz)
            .punteros()
            .uno(nombre)
            .unwrap_or_else(|e| panic!("leer el puntero de `{nombre}`: {e}"))
            .map(|l| l.texto)
    }

    /// Quita el puntero de `nombre`.
    pub(crate) fn quitar_puntero(raiz: &std::path::Path, nombre: &str) {
        super::Lugar::de(raiz)
            .punteros()
            .escribir(
                &[ore_core::punteros::Cambio {
                    nombre: nombre.to_string(),
                    que: ore_core::punteros::Que::Retirar,
                    esperado: ore_core::punteros::Esperado::Cualquiera,
                }],
                &ore_core::punteros::Contexto {
                    autor: "prueba".into(),
                    operacion: "prueba".into(),
                    idempotencia: None,
                },
            )
            .unwrap_or_else(|e| panic!("quitar el puntero de `{nombre}`: {e}"));
    }

    /// **El esquema de este hilo, para el que se va a lanzar**: [`Llevado::aqui`] antes del
    /// `spawn`, [`Llevado::poner`] dentro.
    pub(crate) struct Llevado(Option<String>);

    impl Llevado {
        pub(crate) fn aqui() -> Llevado {
            Llevado(esquema())
        }
        pub(crate) fn poner(self) {
            LLEVADO.with(|l| *l.borrow_mut() = self.0);
        }
    }
}

/// El esquema de quien lanza un hilo, para que el hilo trabaje en el mismo (sólo en las pruebas;
/// en producción hay uno y no lleva nada).
#[cfg(test)]
pub(crate) use de_la_prueba::Llevado;
#[cfg(not(test))]
pub(crate) struct Llevado;
#[cfg(not(test))]
impl Llevado {
    pub(crate) fn aqui() -> Llevado {
        Llevado
    }
    pub(crate) fn poner(self) {}
}

/// ⭐⭐ **EL ENCHUFE DEL CATÁLOGO** (0079 ·5c·2): la puerta de `ref` (`None` =
/// `main`) en el esquema del inquilino, con una conexión del pool de ore-serve (·5c·2 enchufado).
/// La ref, la puerta heredada y el arranque pasan por aquí.
fn abrir_catalogo(esquema: &str, url: &str, rama: Option<&str>) -> Result<Box<dyn Puerta>, Fallo> {
    Ok(Box::new(prestada(esquema, url, rama)?))
}

/// Una conexión del pool de `esquema` (o una nueva), sobre `rama`.
fn prestada(esquema: &str, url: &str, rama: Option<&str>) -> Result<Prestada, Fallo> {
    let almacen = match pool()
        .lock()
        .ok()
        .and_then(|mut p| p.get_mut(esquema)?.pop())
    {
        Some(a) => a,
        None => ore_catalogo::almacen::Almacen::conectar(url, esquema).map_err(Fallo::Almacen)?,
    };
    Ok(Prestada {
        esquema: esquema.to_string(),
        c: Some(ore_catalogo::Catalogo::sobre(almacen, rama)),
        rota: std::sync::atomic::AtomicBool::new(false),
    })
}

/// Cuántas conexiones esperan en el pool: las que vuelven de más se cierran. En las pruebas,
/// ninguna: hay un esquema por prueba, y guardarlas agotaría las conexiones del Postgres.
fn en_el_pool() -> usize {
    if cfg!(test) { 0 } else { 8 }
}

/// **El pool de conexiones al catálogo** (0079 §19.2 P1): conectar cuesta 24 ms de mediana y 66
/// en p99; una petición toma una que espera y la devuelve al soltar la puerta. Por esquema: en
/// producción hay uno; en las pruebas, uno por prueba ([`de_la_prueba`]).
type Pool = std::sync::Mutex<BTreeMap<String, Vec<ore_catalogo::almacen::Almacen>>>;
fn pool() -> &'static Pool {
    static P: OnceLock<Pool> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// La puerta del catálogo con una conexión prestada del pool: al soltarse, la conexión vuelve,
/// salvo si el almacén falló con ella (una conexión rota no se presta otra vez).
struct Prestada {
    esquema: String,
    c: Option<ore_catalogo::Catalogo>,
    rota: std::sync::atomic::AtomicBool,
}

impl Prestada {
    fn con<T>(
        &self,
        f: impl FnOnce(&ore_catalogo::Catalogo) -> Result<T, Fallo>,
    ) -> Result<T, Fallo> {
        let c = self
            .c
            .as_ref()
            .ok_or_else(|| Fallo::Almacen("la puerta del catálogo ya se soltó".into()))?;
        let r = f(c);
        if let Err(Fallo::Almacen(_)) = &r {
            self.rota.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        r
    }
}

impl Drop for Prestada {
    fn drop(&mut self) {
        if let Some(c) = self.c.take()
            && !self.rota.load(std::sync::atomic::Ordering::Relaxed)
            && let Ok(mut p) = pool().lock()
        {
            let p = p.entry(std::mem::take(&mut self.esquema)).or_default();
            if p.len() < en_el_pool() {
                p.push(c.soltar());
            }
        }
    }
}

impl Punteros for Prestada {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo> {
        self.con(|c| c.uno(nombre))
    }
    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        self.con(|c| c.todos())
    }
    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo> {
        self.con(|c| c.historia(nombre, hasta))
    }
    fn escribir(&self, cambios: &[Cambio], ctx: &Contexto) -> Result<(), Fallo> {
        self.con(|c| c.escribir(cambios, ctx))
    }
    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        self.con(|c| c.reclamados())
    }
    fn version(&self) -> Result<String, Fallo> {
        self.con(|c| c.version())
    }
}

impl Ramas for Prestada {
    fn crear(&self, rama: &str, desde: &str) -> Result<(), Fallo> {
        self.con(|c| c.crear(rama, desde))
    }
    fn borrar(&self, rama: &str) -> Result<(), Fallo> {
        self.con(|c| c.borrar(rama))
    }
    fn traer(&self, rama: &str) -> Result<(), Fallo> {
        self.con(|c| c.traer(rama))
    }
}

/// **Al arrancar: que la puerta de `main` abra y conteste.** Un ore-serve que no puede leer un
/// puntero no arranca: mejor que contestar 503 a cada petición.
pub(crate) fn comprobar() -> Result<(), Fallo> {
    let c = catalogo()?;
    abrir_catalogo(&c.esquema, &c.url, None)?
        .version()
        .map(|_| ())
}

// ── La ref en curso ─────────────────────────────────────────────────────────

thread_local! {
    /// La ref del árbol que la costura le está dando al cierre. `None` = `main`.
    static REF: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// **Esta petición trabaja en `rama`** mientras viva (la costura, alrededor del
/// cierre). Al soltarla vuelve la de antes: se anidan.
pub(crate) struct EnRef(Option<String>);

impl EnRef {
    pub(crate) fn poner(rama: Option<&str>) -> EnRef {
        let rama = rama.filter(|r| *r != "main").map(str::to_string);
        EnRef(REF.with(|c| c.replace(rama)))
    }
}

impl Drop for EnRef {
    fn drop(&mut self) {
        let antes = self.0.take();
        REF.with(|c| *c.borrow_mut() = antes);
    }
}

/// La ref de lo que corre en este hilo: `None` es `main`.
pub(crate) fn ref_en_curso() -> Option<String> {
    REF.with(|c| c.borrow().clone())
}

// ── El lugar ────────────────────────────────────────────────────────────────

/// El directorio de un árbol y la puerta de sus punteros.
pub(crate) struct Lugar<'a> {
    raiz: &'a Path,
    rama: Option<String>,
    punteros: Box<dyn Punteros + 'a>,
}

impl<'a> Lugar<'a> {
    /// ⭐ **La puerta de un árbol: la única que se construye en ore-serve**, en
    /// la ref en curso ([`ref_en_curso`]).
    pub(crate) fn de(raiz: &'a Path) -> Lugar<'a> {
        let rama = ref_en_curso();
        let abierta = catalogo().and_then(|c| abrir_catalogo(&c.esquema, &c.url, rama.as_deref()));
        let punteros: Box<dyn Punteros + 'a> = match (abierta, cinta_en_curso()) {
            // ·5c·5c·2: una definición en curso graba en vez de escribir.
            (Ok(p), Some(c)) => Box::new(Grabadora::sobre(p, c)),
            (Ok(p), None) => p,
            (Err(f), _) => Box::new(Cerrada(f)),
        };
        Lugar {
            raiz,
            rama,
            punteros,
        }
    }

    /// El directorio, para los documentos.
    #[allow(dead_code)]
    pub(crate) fn raiz(&self) -> &Path {
        self.raiz
    }

    pub(crate) fn punteros(&self) -> &dyn Punteros {
        self.punteros.as_ref()
    }

    /// El puntero de `corto`, si lo hay: lo que daba `leer_en(datasets, corto)`.
    /// Un nombre que no es `base.n` ni `base.schema.n`, o un almacén que no
    /// contesta, es «no hay», como entonces.
    pub(crate) fn puntero(&self, corto: &str) -> Option<Node> {
        self.punteros.uno(corto).ok().flatten().map(|l| l.puntero)
    }

    /// **¿Es el árbol de una rama, con lo que no tocó leído de `main`?** Lo
    /// que decide si una respuesta dice `de: main | rama` (0044 C.2 ③): la ref
    /// con la que se abrió el lugar (toda rama lee de `main` lo que no tocó).
    pub(crate) fn en_una_rama(&self) -> bool {
        self.rama.is_some()
    }
}

/// ⭐ **El ciclo de vida de las ramas** ([`Ramas`]), para las propuestas: la
/// única que se construye. La tabla de refs del catálogo (la puerta de `main`).
pub(crate) fn ramas() -> Box<dyn Ramas> {
    match catalogo().and_then(|c| abrir_catalogo(&c.esquema, &c.url, None)) {
        Ok(p) => p,
        Err(f) => Box::new(Cerrada(f)),
    }
}

/// ⭐ **La puerta que se le sirve a un `ore` hijo** (la puerta heredada,
/// `crate::heredada`): la misma que daría [`Lugar::de`] para su ref, la de `rama`: **la que
/// puso la costura**, no la que diga el hijo (su `ORE_REF` sale de aquí; quien manda es
/// ore-serve). El catálogo no usa los directorios de la apertura.
pub(crate) fn heredada(
    _: &Apertura,
    _: &Path,
    rama: Option<&str>,
    cinta: Option<&Cinta>,
) -> Result<Box<dyn Puerta>, Fallo> {
    let c = catalogo()?;
    let p = abrir_catalogo(&c.esquema, &c.url, rama)?;
    Ok(match cinta {
        // ·5c·5c·2: lo que `ore` escriba durante una definición, grabado (renombrar).
        Some(c) => Box::new(Grabadora::sobre(p, c.clone())),
        None => p,
    })
}

// ── La grabación de una definición (0079 ·5c·5c·2) ──────────────────────────────────────────

thread_local! {
    /// La cinta de la definición en curso en este hilo, si la hay (sólo con el catálogo).
    static CINTA: RefCell<Option<Cinta>> = const { RefCell::new(None) };
}

/// **Una definición en curso** (retirar, renombrar) mientras viva: toda puerta que se abra en
/// este hilo ([`Lugar::de`], y la heredada de un `ore` que se lance desde él) graba en vez de
/// escribir, y lo grabado es el plan (`crate::definicion`).
pub(crate) struct Grabando(Option<Cinta>);

impl Grabando {
    pub(crate) fn abrir() -> Grabando {
        Grabando(CINTA.with(|c| c.replace(Some(Cinta::nueva()))))
    }
}

impl Drop for Grabando {
    fn drop(&mut self) {
        let antes = self.0.take();
        CINTA.with(|c| *c.borrow_mut() = antes);
    }
}

/// La cinta de la definición en curso en este hilo, si la hay.
pub(crate) fn cinta_en_curso() -> Option<Cinta> {
    CINTA.with(|c| c.borrow().clone())
}

/// **El catálogo de `rama`, para lo que no es la puerta** (las definiciones, ·5c·5c): `f` con la
/// conexión del pool.
pub(crate) fn en_el_catalogo<T>(
    rama: Option<&str>,
    f: impl FnOnce(&ore_catalogo::Catalogo) -> T,
) -> Result<T, Fallo> {
    let c = catalogo()?;
    let rama = rama.filter(|r| *r != "main");
    let p = prestada(&c.esquema, &c.url, rama)?;
    Ok(f(p.c.as_ref().expect("recién puesta")))
}

/// **La clave de las cachés que el commit de git no da** (0079 §19.1: «las
/// cachés pasan a tener como clave `version()`»): `version()` de la ref
/// (0,6 ms p50 medido en ·5c P2).
pub(crate) fn version_de_la_ref(rama: Option<&str>) -> Result<String, Fallo> {
    let c = catalogo()?;
    let rama = rama.filter(|r| *r != "main");
    abrir_catalogo(&c.esquema, &c.url, rama)?.version()
}

/// Una puerta que no abrió: cada pregunta contesta su fallo (503). Así
/// [`Lugar::de`] no cambia de tipo y quien lee un puntero ya sabe qué hacer con
/// un almacén que no contesta.
struct Cerrada(Fallo);

impl Punteros for Cerrada {
    fn uno(&self, _: &str) -> Result<Option<Leido>, Fallo> {
        Err(self.0.clone())
    }
    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        Err(self.0.clone())
    }
    fn historia(&self, _: &str, _: usize) -> Result<Vec<Version>, Fallo> {
        Err(self.0.clone())
    }
    fn escribir(&self, _: &[Cambio], _: &Contexto) -> Result<(), Fallo> {
        Err(self.0.clone())
    }
    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        Err(self.0.clone())
    }
    fn version(&self) -> Result<String, Fallo> {
        Err(self.0.clone())
    }
}

impl Ramas for Cerrada {
    fn crear(&self, _: &str, _: &str) -> Result<(), Fallo> {
        Err(self.0.clone())
    }
    fn borrar(&self, _: &str) -> Result<(), Fallo> {
        Err(self.0.clone())
    }
    fn traer(&self, _: &str) -> Result<(), Fallo> {
        Err(self.0.clone())
    }
}

/// Lo que no pudo la puerta de las ramas, como respuesta.
pub(crate) fn fallo_de_ramas(que: &str, e: ore_core::punteros::Fallo) -> Respuesta {
    Respuesta::error(e.codigo(), format!("{que}: {e}"))
}

/// Un instante RFC 3339 (`%cI` de git: `2026-10-28T10:00:00+02:00`) a segundos
/// de época, con su desfase.
pub(crate) fn segundos(cuando: &str) -> Option<i64> {
    let base = ore_core::frescura::epoca(cuando)?;
    let resto = cuando.get(19..)?;
    // Las fracciones de segundo, si las hay, no cuentan.
    let resto = resto.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let desfase = match resto {
        "" | "Z" | "z" => 0,
        r => {
            let signo = match r.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let (h, m) = r[1..].split_once(':').unwrap_or((&r[1..], "0"));
            signo * (h.parse::<i64>().ok()? * 3600 + m.parse::<i64>().ok()? * 60)
        }
    };
    Some(base - desfase)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_segundos_de_un_cuando_de_git_cuentan_su_desfase() {
        let utc = segundos("2026-10-28T08:00:00Z").unwrap();
        assert_eq!(segundos("2026-10-28T10:00:00+02:00"), Some(utc));
        assert_eq!(segundos("2026-10-28T03:00:00-05:00"), Some(utc));
        assert_eq!(segundos("2026-10-28T08:00:00.123Z"), Some(utc));
        assert_eq!(segundos("ayer"), None);
    }

    #[test]
    fn el_lugar_lee_por_la_puerta_y_sabe_si_es_una_rama() {
        let d = std::env::temp_dir().join(format!(
            "ore-serve-lugar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(d.join("datasets/ventas/default")).unwrap();
        crate::lugar::de_la_prueba::poner_puntero(
            &d,
            "ventas.pedidos",
            "{\"estado\": \"copiada\"}\n",
        );
        let l = Lugar::de(&d);
        let p = l.puntero("ventas.pedidos").unwrap();
        assert_eq!(
            p.get("estado").and_then(|(_, v)| v.as_str()),
            Some("copiada")
        );
        assert!(l.puntero("ventas.otro").is_none());
        assert!(l.puntero("sin_base").is_none(), "un no-nombre es «no hay»");
        assert!(!l.en_una_rama());
        // En una rama, lo que no tocó se lee de `main`, y el lugar lo sabe.
        crate::lugar::ramas().crear("r", "main").unwrap();
        let _r = EnRef::poner(Some("r"));
        let en_r = Lugar::de(&d);
        assert!(en_r.en_una_rama());
        assert!(
            en_r.puntero("ventas.pedidos").is_some(),
            "la rama hereda de `main`"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// La ref de la costura se anida y vuelve; `main` es `None`.
    #[test]
    fn la_ref_en_curso_se_anida_y_vuelve() {
        assert_eq!(ref_en_curso(), None);
        {
            let _a = EnRef::poner(Some("r1"));
            assert_eq!(ref_en_curso().as_deref(), Some("r1"));
            {
                let _b = EnRef::poner(Some("main"));
                assert_eq!(ref_en_curso(), None, "`main` es `None`");
            }
            assert_eq!(ref_en_curso().as_deref(), Some("r1"));
        }
        assert_eq!(ref_en_curso(), None);
    }

    /// Una puerta que no abrió contesta su fallo a todo, con su código.
    #[test]
    fn una_puerta_cerrada_contesta_su_fallo() {
        let c = Cerrada(Fallo::Almacen("no".into()));
        assert_eq!(c.uno("a.b").unwrap_err().codigo(), 503);
        assert_eq!(c.version().unwrap_err().codigo(), 503);
        assert_eq!(c.crear("r", "main").unwrap_err().codigo(), 503);
    }
}
