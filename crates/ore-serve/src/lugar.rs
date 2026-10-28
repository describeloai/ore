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
//! # El interruptor (0079 ·5c·3): `--punteros ficheros|catalogo`
//!
//! | | La puerta | Las ramas | `ore` (por `mando`) |
//! |---|---|---|---|
//! | **`ficheros`** (por defecto) | [`Ficheros`] sobre el directorio, que la costura ya montó superpuesto si es una rama (0044 C.2 ③) y que publica la costura con el resto del árbol, como siempre | no hace nada: lo hace git | lee los ficheros él mismo, como siempre; **por la puerta heredada sólo si `ORE_PUERTA_HEREDADA=1`** (las pruebas, ·5c·4) |
//! | **`catalogo`** (apagado en todos los inquilinos hasta ·5g) | el catálogo de la ref en curso ([`abrir_catalogo`], `--catalogo-esquema t_<org>`, `CATALOGO_URL`) | su tabla de refs | **siempre por la puerta heredada**: `ore` no puede abrir un socket, y ore-serve le sirve el catálogo desde su pool |
//!
//! ⛔ **El catálogo se enchufa en UNA línea**: [`abrir_catalogo`]. Hoy contesta
//!   «·5c·2: Catalogo aún no enchufado» (503), y `ore-serve --punteros catalogo`
//!   no arranca (se prueba la puerta al arrancar, [`comprobar`]).
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
//!   (ninguno hoy) lo abriría en `main`. Con `ficheros` da igual —manda el
//!   directorio—; con `catalogo`, ese hilo tendría que llevarse la ref.
//!
//! | | Qué |
//! |---|---|
//! | [`Lugar::de`] | el directorio y su puerta |
//! | [`Lugar::puntero`] | el de un nombre, como lo leía `leer_en` (`None` si no está o no es un nombre) |
//! | [`Lugar::en_una_rama`] | si el árbol es el de una rama con lo no tocado leído de `main` |
//! | [`ramas`] | la puerta del ciclo de vida de las ramas (crear, borrar, traer `main`) |
//! | [`heredada`] | la puerta que se le sirve a un `ore` hijo, para su `abrir` |
//! | [`version_de_la_ref`] | la clave de las cachés que el commit no da (sólo `catalogo`) |
//! | [`segundos`] | un `cuando` de [`ore_core::punteros::Version`] a segundos de época |

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use ore_core::parse::Node;
use ore_core::punteros::protocolo::{Apertura, Puerta};
use ore_core::punteros::{
    Cambio, Contexto, Fallo, Ficheros, Leido, Punteros, Ramas, Reclamados, Version,
};
use ore_entrada::http::Respuesta;

// ── El interruptor ──────────────────────────────────────────────────────────

/// Dónde viven los punteros de este inquilino (`--punteros`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Implementacion {
    /// En el árbol, como ficheros: lo de siempre.
    Ficheros,
    /// En el catálogo transaccional (0079): el esquema del inquilino
    /// (`--catalogo-esquema t_<org>`) en el Postgres de `CATALOGO_URL`.
    Catalogo { esquema: String, url: String },
}

static IMPLEMENTACION: OnceLock<Implementacion> = OnceLock::new();

/// **Elige la implementación**, una vez, al arrancar (`main`). Sin llamarla
/// (las pruebas), [`Implementacion::Ficheros`].
pub(crate) fn configurar(i: Implementacion) -> Result<(), String> {
    IMPLEMENTACION
        .set(i)
        .map_err(|_| "la implementación de los punteros ya estaba elegida".to_string())
}

fn implementacion() -> &'static Implementacion {
    IMPLEMENTACION.get_or_init(|| Implementacion::Ficheros)
}

/// ¿Los punteros en el catálogo? Lo que decide las escrituras en dos fases
/// (0079 ·5c·5, `crate::dos_fases`).
pub(crate) fn con_catalogo() -> bool {
    matches!(implementacion(), Implementacion::Catalogo { .. })
}

/// La variable que **fuerza la puerta heredada con `ficheros`**: la suite entera
/// de ore-serve por el mismo camino que tomará el catálogo (0079 ·5c·4).
pub(crate) const FORZAR_HEREDADA: &str = "ORE_PUERTA_HEREDADA";

/// **¿Se le sirve la puerta a `ore`, o la abre él?** Con el catálogo, siempre
/// (`ore` no puede hablar con Postgres); con ficheros, sólo si
/// [`FORZAR_HEREDADA`]` = 1`. Lo de hoy, por defecto, no cambia.
pub(crate) fn puerta_heredada() -> bool {
    static FORZADA: OnceLock<bool> = OnceLock::new();
    *implementacion() != Implementacion::Ficheros
        || *FORZADA.get_or_init(|| std::env::var(FORZAR_HEREDADA).is_ok_and(|v| v.trim() == "1"))
}

/// ⭐⭐ **EL ENCHUFE DEL CATÁLOGO** (0079 ·5c·2): la puerta de `ref` (`None` =
/// `main`) en el esquema del inquilino, con una conexión del pool de ore-serve (·5c·2 enchufado).
/// La ref, la puerta heredada y el arranque pasan por aquí.
fn abrir_catalogo(esquema: &str, url: &str, rama: Option<&str>) -> Result<Box<dyn Puerta>, Fallo> {
    let almacen = match pool().lock().ok().and_then(|mut p| p.pop()) {
        Some(a) => a,
        None => ore_catalogo::almacen::Almacen::conectar(url, esquema).map_err(Fallo::Almacen)?,
    };
    Ok(Box::new(Prestada {
        c: Some(ore_catalogo::Catalogo::sobre(almacen, rama)),
        rota: std::sync::atomic::AtomicBool::new(false),
    }))
}

/// Cuántas conexiones esperan en el pool: las que vuelven de más se cierran.
const EN_EL_POOL: usize = 8;

/// **El pool de conexiones al catálogo** (0079 §19.2 P1): conectar cuesta 24 ms de mediana y 66
/// en p99; una petición toma una que espera y la devuelve al soltar la puerta.
fn pool() -> &'static std::sync::Mutex<Vec<ore_catalogo::almacen::Almacen>> {
    static P: OnceLock<std::sync::Mutex<Vec<ore_catalogo::almacen::Almacen>>> = OnceLock::new();
    P.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// La puerta del catálogo con una conexión prestada del pool: al soltarse, la conexión vuelve,
/// salvo si el almacén falló con ella (una conexión rota no se presta otra vez).
struct Prestada {
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
            && p.len() < EN_EL_POOL
        {
            p.push(c.soltar());
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

/// **Al arrancar, con `catalogo`: que la puerta de `main` abra y conteste.** Un
/// ore-serve que no puede leer un puntero no arranca: mejor que contestar 503 a
/// cada petición.
pub(crate) fn comprobar() -> Result<(), Fallo> {
    match implementacion() {
        Implementacion::Ficheros => Ok(()),
        Implementacion::Catalogo { esquema, url } => {
            abrir_catalogo(esquema, url, None)?.version().map(|_| ())
        }
    }
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
        let punteros: Box<dyn Punteros + 'a> = match implementacion() {
            Implementacion::Ficheros => Box::new(Ficheros::en(raiz)),
            Implementacion::Catalogo { esquema, url } => {
                match abrir_catalogo(esquema, url, rama.as_deref()) {
                    Ok(p) => p,
                    Err(f) => Box::new(Cerrada(f)),
                }
            }
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
    /// que decide si una respuesta dice `de: main | rama` (0044 C.2 ③).
    ///
    /// Con ficheros, el [`crate::git::AL_DIA`] que deja la superposición —que
    /// existe sólo en una rama por detrás de `main`, lo mismo que miraba
    /// `de_main`—; con el catálogo, la ref con la que se abrió el lugar (toda
    /// rama lee de `main` lo que no tocó).
    pub(crate) fn en_una_rama(&self) -> bool {
        match implementacion() {
            Implementacion::Ficheros => self.raiz.join(crate::git::AL_DIA).is_file(),
            Implementacion::Catalogo { .. } => self.rama.is_some(),
        }
    }
}

/// ⭐ **El ciclo de vida de las ramas** ([`Ramas`]), para las propuestas: la
/// única que se construye. Con [`Ficheros`] no hace nada —crear, borrar y
/// traer una rama de git ya mueve sus punteros—, y por eso no necesita un
/// árbol; con el catálogo, su tabla de refs (la puerta de `main`).
pub(crate) fn ramas() -> Box<dyn Ramas> {
    match implementacion() {
        Implementacion::Ficheros => Box::new(Ficheros::en(Path::new(""))),
        Implementacion::Catalogo { esquema, url } => match abrir_catalogo(esquema, url, None) {
            Ok(p) => p,
            Err(f) => Box::new(Cerrada(f)),
        },
    }
}

/// ⭐ **La puerta que se le sirve a un `ore` hijo** (la puerta heredada,
/// `crate::heredada`): la misma que daría [`Lugar::de`] para su árbol y su ref.
///
/// - Con ficheros, la de `ore` mismo (`ore_cli::puerta::Local`): sobre su
///   `raiz`, en su `carpeta` (`--informe`) y con sus `ajenos` (`--reclaman`),
///   que llegan relativos a su directorio, `cwd`.
/// - Con el catálogo, la de `rama`: **la que puso la costura**, no la que diga
///   el hijo (su `ORE_REF` sale de aquí; quien manda es ore-serve).
pub(crate) fn heredada(
    a: &Apertura,
    cwd: &Path,
    rama: Option<&str>,
) -> Result<Box<dyn Puerta>, Fallo> {
    match implementacion() {
        Implementacion::Ficheros => {
            let f = Ficheros::en(&cwd.join(&a.raiz))
                .con_ajenos(a.ajenos.as_deref().map(|d| cwd.join(d)).as_deref());
            Ok(Box::new(match &a.carpeta {
                Some(c) => f.con_carpeta(&cwd.join(c)),
                None => f,
            }))
        }
        Implementacion::Catalogo { esquema, url } => abrir_catalogo(esquema, url, rama),
    }
}

/// **La clave de las cachés que el commit de git no da** (0079 §19.1: «las
/// cachés pasan a tener como clave `version()`»). Con ficheros, `None`: los
/// punteros están en el commit y la clave de siempre ya los cuenta, así que
/// el camino caliente no paga nada. Con el catálogo, `version()` de la ref
/// (0,6 ms p50 medido en ·5c P2).
pub(crate) fn version_de_la_ref(rama: Option<&str>) -> Result<Option<String>, Fallo> {
    match implementacion() {
        Implementacion::Ficheros => Ok(None),
        Implementacion::Catalogo { esquema, url } => {
            let rama = rama.filter(|r| *r != "main");
            abrir_catalogo(esquema, url, rama)?.version().map(Some)
        }
    }
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
        std::fs::write(
            d.join("datasets/ventas/default/pedidos.json"),
            "{\"estado\": \"copiada\"}\n",
        )
        .unwrap();
        let l = Lugar::de(&d);
        let p = l.puntero("ventas.pedidos").unwrap();
        assert_eq!(
            p.get("estado").and_then(|(_, v)| v.as_str()),
            Some("copiada")
        );
        assert!(l.puntero("ventas.otro").is_none());
        assert!(l.puntero("sin_base").is_none(), "un no-nombre es «no hay»");
        assert!(!l.en_una_rama());
        std::fs::write(
            d.join(crate::git::AL_DIA),
            "{\"main\": \"m\", \"rutas\": {}}\n",
        )
        .unwrap();
        assert!(l.en_una_rama());
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

    /// Con ficheros, la puerta heredada es la de `ore`: su raíz, su carpeta y
    /// sus ajenos, relativos a su directorio.
    #[test]
    fn la_puerta_heredada_con_ficheros_es_la_de_ore() {
        let d =
            std::env::temp_dir().join(format!("ore-serve-heredada-lugar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("informe/ventas/default")).unwrap();
        std::fs::write(
            d.join("informe/ventas/default/pedidos.json"),
            "{\"estado\": \"copiada\"}\n",
        )
        .unwrap();
        let a = Apertura {
            raiz: ".".into(),
            carpeta: Some("informe".into()),
            ajenos: None,
            rama: Some("r".into()),
        };
        let p = heredada(&a, &d, Some("r")).unwrap();
        assert!(p.uno("ventas.pedidos").unwrap().is_some());
        let sin = Apertura {
            carpeta: None,
            ..a.clone()
        };
        assert!(
            heredada(&sin, &d, None)
                .unwrap()
                .uno("ventas.pedidos")
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&d);
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
