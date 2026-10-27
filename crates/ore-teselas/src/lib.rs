//! **El motor de teselas** (ADR 0072 C3, F2·1): una tesela MVT de una instantánea del
//! lago, **pura**: no sabe de personas, del catálogo ni de políticas; recibe lo que
//! leer (el `metadata.json` de la instantánea, la columna geo, su CRS), la tesela,
//! los límites y una credencial prestada para el dataset, y devuelve los bytes o
//! «vacía». La misma petición da siempre los mismos bytes (D-F2-6): el `ETag` de
//! `ore-serve` es su huella.
//!
//! La cadena, la de consenso del ámbito (D-F2-3, medida en F2·0a contra `ST_AsMVT`):
//! a la tesela (0..4096, y hacia abajo), **simplificar por debajo del píxel** (RDP,
//! 1 u), **recortar con margen** (64 u), a la rejilla entera, sin repetidos, fuera los
//! anillos de menos de 4 u², orientar (spec 2.1); orden Hilbert + id.
//!
//! La poda (D-F2-2): los grupos de filas cuya cobertura `<geom>_bbox` no toca la
//! tesela no se leen (vale para `ore-store` y Spark); los pies, en caché. Dentro de cada
//! grupo, las páginas de la cobertura que su índice de columnas descarta tampoco.
//!
//! La lectura, por oleadas (F2·3, D-F2-8 a D-F2-10, [`lector`]): con los pies en memoria la
//! tesela sabe qué páginas necesita y las trae juntas —primero las de la cobertura, luego
//! `geom` e `id` de las filas que la tocan—, de la caché de páginas o en paralelo a GCS. La
//! espera no ocupa turno: los turnos (CPU) sólo cubren descodificar, recortar y codificar.
//!
//! Los tres tipos de MVT (F2·1·6, K-F2·1-g..j), en una sola capa: **puntos** (los que caen
//! en la tesela con su margen; nunca se funden puntos de filas distintas: la densidad se
//! agrega, no se esconde), **líneas** (RDP abierto a 1 u y Liang–Barsky contra el marco: la
//! que sale y vuelve a entrar se parte en trozos) y **polígonos**. Una `GeometryCollection`
//! se descompone, un elemento por tipo con el mismo id; Z y M se descartan; las curvas y lo
//! que no sea de los siete tipos simples cuenta en `descartados`.
//!
//! ⚠️ F2·1 es el esqueleto: lo que F2·0a dejó para F2·3 —filtrar por fila antes de
//! descodificar, reparar tras la rejilla, el suelo calculado, la reproyección
//! general— no está. Hoy, CRS `EPSG:4326`/`OGC:CRS84` y `EPSG:3857`.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use arrow_array::{
    Array, BinaryArray, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array,
    LargeBinaryArray, RecordBatch, StructArray,
};
use arrow_schema::ArrowError;
use ore_base::json::Json;
use ore_store::almacen::Almacen;
use ore_store::lago::Lago;
use ore_store::muestra::FicheroVivo;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder, RowSelection,
};
use parquet::file::metadata::PageIndexPolicy;
use parquet::file::page_index::column_index::ColumnIndexMetaData;
use parquet::file::page_index::offset_index::PageLocation;
use parquet::file::statistics::Statistics;

pub mod indice;
pub mod indice_sql;
use indice::Manifiesto;
pub mod lector;
pub mod piramide;
use lector::{Paginas, Trozos};
use piramide::Piramide;

/// La versión del protocolo (C3): el motor acepta ésta y la anterior (N y N−1).
pub const PROTOCOLO: u32 = 1;
pub const EXTENT: f64 = 4096.0;
pub const BUFFER: f64 = 64.0;
const R: f64 = 6_378_137.0;
/// Cuántos pies y listas de ficheros se guardan (por clave inmutable).
const CACHE: usize = 4096;
/// Cuánto vale lo que dijo el manifiesto de un índice (I3·1): no es inmutable (cambia al publicarse
/// otra generación), y lo pide el descriptor al abrir un mapa.
const VIDA_DEL_INDICE: Duration = Duration::from_secs(10);
/// Cuántas instantáneas hacia atrás se busca un índice (I3·4): una por GET, por el `metadata-log`
/// de la tabla. Más atrás, el índice se rehace.
const HISTORIA: usize = 20;

/// **Las filas borradas por posición de una instantánea** (I4·4), por fichero de datos (su clave
/// en el almacén): sus posiciones, ordenadas y sin repetir. Las lee el motor una vez por instantánea.
type Borradas = HashMap<String, Vec<u64>>;

/// Los ficheros vivos de una instantánea `(clave, bytes)` y sus filas borradas.
type Vivos = (Arc<Vec<(String, u64)>>, Arc<Borradas>);

/// La selección de un grupo de `n` filas que empieza en la fila `desde` de su fichero, sin las
/// posiciones `borradas` (ordenadas) que caen en él.
fn sin_borradas(borradas: &[u64], desde: u64, n: usize) -> RowSelection {
    let a = borradas.partition_point(|p| *p < desde);
    let z = borradas.partition_point(|p| *p < desde + n as u64);
    let mut quedan = Vec::new();
    let mut previa = 0usize;
    for p in &borradas[a..z] {
        let i = (p - desde) as usize;
        if i > previa {
            quedan.push(previa..i);
        }
        previa = i + 1;
    }
    if previa < n {
        quedan.push(previa..n);
    }
    RowSelection::from_consecutive_ranges(quedan.into_iter(), n)
}

/// Lo que el motor sabe de una instantánea: lo dice su `metadata.json`, que es inmutable.
struct Instantanea {
    filas: u64,
    secuencia: u64,
    ms: i64,
    /// Las instantáneas anteriores de la tabla (su `metadata-log`), de la más nueva a la más vieja.
    anteriores: Vec<String>,
}

// ── C3 v1: la petición ──────────────────────────────────────────────────────────────────────────

/// La credencial prestada para el dataset (la de `prestar`, con su forma de Iceberg).
#[derive(Clone, Debug)]
pub enum Credencial {
    Gcs {
        token: String,
    },
    S3 {
        endpoint: String,
        region: String,
        clave: String,
        secreto: String,
    },
}

/// **La pirámide** de la capa (F2·3·2·1, D-F2-15): un PMTiles con las teselas por debajo del
/// suelo y su propia credencial (vive en el almacenamiento de ORE, no en el del dataset). Con
/// ella, una tesela de sus zooms sale de ahí; las demás, del crudo.
#[derive(Clone, Debug)]
pub struct FuentePiramide {
    pub ubicacion: String,
    pub credencial: Credencial,
}

#[derive(Clone, Debug)]
pub struct Peticion {
    pub z: u32,
    pub x: u32,
    pub y: u32,
    pub metadata_location: String,
    pub geometria: String,
    pub crs: String,
    pub id: Option<String>,
    /// El nombre de la capa en el MVT (el `id` de `vector_layers` del descriptor).
    pub capa: String,
    pub plazo_ms: u64,
    pub max_bytes: usize,
    pub credencial: Credencial,
    pub piramide: Option<FuentePiramide>,
}

/// Lo que la tesela costó (`x-ore-cuenta`).
#[derive(Default, Debug, Clone)]
pub struct Cuenta {
    pub grupos: usize,
    pub grupos_leidos: usize,
    /// Las filas de los grupos leídos; `filas`, las que la criba por fila dejó pasar.
    pub filas_en_grupos: usize,
    /// Las de las páginas de la cobertura que su índice de columnas no descarta.
    pub filas_en_paginas: usize,
    pub filas: usize,
    pub elementos: usize,
    /// Los elementos por tipo (F2·1·6): una fila con colección cuenta en cada uno.
    pub puntos: usize,
    pub lineas: usize,
    pub poligonos: usize,
    pub diminutos: usize,
    /// Las filas cuya geometría no se pinta (curvas, superficies poliédricas, WKB roto).
    pub descartados: usize,
    /// Las idas al almacén de esta tesela (F2·3·0), sus oleadas y sus bytes; los trozos que
    /// salieron de la caché de páginas y los que trajo otra tesela que los pedía a la vez.
    pub peticiones: usize,
    pub oleadas: usize,
    pub bytes: usize,
    pub en_cache: usize,
    pub compartidas: usize,
    pub listar_ms: u64,
    pub leer_ms: u64,
    /// Lo que esperó a un turno de CPU (dentro de `leer_ms` y `geom_ms`).
    pub turno_ms: u64,
    /// Descodificar la cobertura y cribar (dentro de `leer_ms`).
    pub criba_ms: u64,
    pub geom_ms: u64,
    pub codificar_ms: u64,
    /// 1 si la tesela salió entera de la caché de teselas (sin leer ni calcular nada).
    pub tesela_en_cache: usize,
    /// 1 si salió de la pirámide (F2·3·2·1), no del crudo.
    pub piramide: usize,
}

impl Cuenta {
    pub fn json(&self) -> Json {
        let n = |v: usize| Json::Int(v as i64);
        let m = |v: u64| Json::Int(v as i64);
        Json::obj([
            ("grupos", n(self.grupos)),
            ("grupos_leidos", n(self.grupos_leidos)),
            ("filas_en_grupos", n(self.filas_en_grupos)),
            ("filas_en_paginas", n(self.filas_en_paginas)),
            ("filas", n(self.filas)),
            ("elementos", n(self.elementos)),
            ("puntos", n(self.puntos)),
            ("lineas", n(self.lineas)),
            ("poligonos", n(self.poligonos)),
            ("diminutos", n(self.diminutos)),
            ("descartados", n(self.descartados)),
            ("peticiones", n(self.peticiones)),
            ("oleadas", n(self.oleadas)),
            ("bytes", n(self.bytes)),
            ("en_cache", n(self.en_cache)),
            ("compartidas", n(self.compartidas)),
            ("listar_ms", m(self.listar_ms)),
            ("leer_ms", m(self.leer_ms)),
            ("turno_ms", m(self.turno_ms)),
            ("criba_ms", m(self.criba_ms)),
            ("geom_ms", m(self.geom_ms)),
            ("codificar_ms", m(self.codificar_ms)),
            ("tesela_en_cache", n(self.tesela_en_cache)),
            ("piramide", n(self.piramide)),
        ])
    }
}

/// Por qué no hay tesela: 422 la petición, 503 el plazo o el tamaño, 500 el resto.
#[derive(Debug)]
pub enum Fallo {
    Invalida(String),
    Plazo(String),
    Interno(String),
}

impl Fallo {
    pub fn codigo(&self) -> u16 {
        match self {
            Fallo::Invalida(_) => 422,
            Fallo::Plazo(_) => 503,
            Fallo::Interno(_) => 500,
        }
    }
    pub fn motivo(&self) -> &str {
        match self {
            Fallo::Invalida(m) | Fallo::Plazo(m) | Fallo::Interno(m) => m,
        }
    }
}

/// La petición desde su JSON (C3 v1). Todo lo que falta o sobra de forma es `Invalida`.
pub fn peticion(texto: &str) -> Result<Peticion, Fallo> {
    let mal = |m: String| Fallo::Invalida(m);
    let n =
        ore_base::parse::parse(texto).map_err(|e| mal(format!("el cuerpo no es JSON: {e:?}")))?;
    let s = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let num = |k: &str| -> Result<u64, Fallo> {
        s(k).ok_or_else(|| mal(format!("falta `{k}`")))?
            .parse::<u64>()
            .map_err(|_| mal(format!("`{k}` no es un entero")))
    };
    let protocolo = num("protocolo")? as u32;
    if protocolo != PROTOCOLO && protocolo + 1 != PROTOCOLO {
        return Err(mal(format!(
            "protocolo {protocolo}: este motor habla {PROTOCOLO} (y el anterior)"
        )));
    }
    if s("tms").as_deref() != Some("WebMercatorQuad") {
        return Err(mal("`tms`: sólo `WebMercatorQuad` (F2)".into()));
    }
    if s("formato").as_deref() != Some("mvt") {
        return Err(mal("`formato`: sólo `mvt` (F2)".into()));
    }
    let (z, x, y) = (num("z")? as u32, num("x")?, num("y")?);
    if z > 24 || x >= 1u64 << z || y >= 1u64 << z {
        return Err(mal(format!("la tesela {z}/{x}/{y} no existe")));
    }
    let f = n
        .get("fuente")
        .map(|(_, v)| v)
        .ok_or_else(|| mal("falta `fuente`".into()))?;
    let fs = |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let metadata_location =
        fs("metadata_location").ok_or_else(|| mal("falta `fuente.metadata_location`".into()))?;
    let geometria = fs("geometria").ok_or_else(|| mal("falta `fuente.geometria`".into()))?;
    let crs = fs("crs").ok_or_else(|| mal("falta `fuente.crs`".into()))?;
    if !matches!(crs.as_str(), "EPSG:4326" | "OGC:CRS84" | "EPSG:3857") {
        return Err(mal(format!(
            "CRS `{crs}`: el motor reproyecta 4326 y 3857 (el resto, F2·3)"
        )));
    }
    let l = n.get("limites").map(|(_, v)| v);
    let lim = |k: &str, d: u64| {
        l.and_then(|l| l.get(k))
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let c = n
        .get("credencial")
        .map(|(_, v)| v)
        .ok_or_else(|| mal("falta `credencial`".into()))?;
    let credencial = credencial_de(c, &metadata_location, "credencial")?;
    let piramide = match f.get("piramide").map(|(_, v)| v) {
        None => None,
        Some(pi) => {
            let ubicacion = pi
                .get("ubicacion")
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| mal("falta `fuente.piramide.ubicacion`".into()))?;
            // sin la suya, la del dataset: la pirámide vive bajo la tabla (D-F2-15) y la
            // credencial prestada para ella la cubre
            let credencial = match pi.get("credencial").map(|(_, v)| v) {
                Some(c) => credencial_de(c, &ubicacion, "fuente.piramide.credencial")?,
                None => credencial.clone(),
            };
            Some(FuentePiramide {
                ubicacion,
                credencial,
            })
        }
    };
    Ok(Peticion {
        z,
        x: x as u32,
        y: y as u32,
        metadata_location,
        geometria,
        crs,
        id: fs("id").filter(|v| !v.is_empty()),
        capa: s("capa").unwrap_or_else(|| "capa".into()),
        plazo_ms: lim("plazo_ms", 10_000),
        max_bytes: lim("max_bytes", 2_000_000) as usize,
        credencial,
        piramide,
    })
}

/// `gs://cubo/clave` → `clave` (el objeto dentro de su cubo).
fn clave_de_objeto(ubicacion: &str) -> Result<String, Fallo> {
    ubicacion
        .split_once("://")
        .and_then(|(_, r)| r.split_once('/'))
        .map(|(_, k)| k.to_string())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| Fallo::Invalida(format!("`{ubicacion}`: sin objeto")))
}

/// La credencial para leer `ubicacion` (gs:// o s3://), con las claves de Iceberg.
fn credencial_de(
    c: &ore_base::parse::Node,
    ubicacion: &str,
    donde: &str,
) -> Result<Credencial, Fallo> {
    let cs = |k: &str| c.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let falta = |k: &str| Fallo::Invalida(format!("falta `{donde}.{k}`"));
    Ok(if ubicacion.starts_with("gs://") {
        Credencial::Gcs {
            token: cs("gcs.oauth2.token").ok_or_else(|| falta("gcs.oauth2.token"))?,
        }
    } else if ubicacion.starts_with("s3://") {
        Credencial::S3 {
            endpoint: cs("s3.endpoint").ok_or_else(|| falta("s3.endpoint"))?,
            region: cs("s3.region").unwrap_or_else(|| "auto".into()),
            clave: cs("s3.access-key-id").ok_or_else(|| falta("s3.access-key-id"))?,
            secreto: cs("s3.secret-access-key").ok_or_else(|| falta("s3.secret-access-key"))?,
        }
    } else {
        return Err(Fallo::Invalida(format!("`{ubicacion}`: ni gs:// ni s3://")));
    })
}

// ── Los turnos ──────────────────────────────────────────────────────────────────────────────────

/// **Los turnos** (F2·3): un tope que se espera dentro del plazo. Los de CPU, tantos como CPU,
/// cubren lo que es cálculo (descodificar, recortar, codificar): sin tope, las ~20 teselas
/// que pide MapLibre al abrir una vista se repartían 1 CPU y la memoria llegaba al OOM. La
/// espera de GCS no ocupa turno de CPU (D-F2-9: el banco midió el motor a 61 m de CPU
/// esperando); tiene los suyos (teselas en vuelo, GET a la vez).
pub struct Turnos {
    libres: Mutex<usize>,
    cv: Condvar,
}

/// Un turno tomado: se devuelve al soltarlo.
pub struct Turno<'a>(&'a Turnos);

impl Turnos {
    pub fn nuevos(n: usize) -> Turnos {
        Turnos {
            libres: Mutex::new(n.max(1)),
            cv: Condvar::new(),
        }
    }

    /// `ORE_TESELAS_TURNOS`, o las CPU que el proceso tiene (la cuota del cgroup incluida).
    pub fn de_la_maquina() -> Turnos {
        let n = std::env::var("ORE_TESELAS_TURNOS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
        Turnos::nuevos(n)
    }

    /// Un turno, esperando como mucho `espera`; `None` si no lo hubo a tiempo.
    pub fn entrar(&self, espera: Duration) -> Option<Turno<'_>> {
        let fin = Instant::now() + espera;
        let mut libres = self.libres.lock().unwrap();
        while *libres == 0 {
            let ahora = Instant::now();
            if ahora >= fin {
                return None;
            }
            libres = self.cv.wait_timeout(libres, fin - ahora).unwrap().0;
        }
        *libres -= 1;
        Some(Turno(self))
    }
}

impl Drop for Turno<'_> {
    fn drop(&mut self) {
        *self.0.libres.lock().unwrap() += 1;
        self.0.cv.notify_one();
    }
}

// ── El motor ────────────────────────────────────────────────────────────────────────────────────

/// Una caché pequeña por clave inmutable: al llenarse se vacía (lo inmutable se recalcula igual).
struct Cache<V: Clone>(Mutex<HashMap<String, V>>);

impl<V: Clone> Cache<V> {
    fn new() -> Self {
        Cache(Mutex::new(HashMap::new()))
    }
    fn get(&self, k: &str) -> Option<V> {
        self.0.lock().unwrap().get(k).cloned()
    }
    fn put(&self, k: String, v: V) {
        let mut m = self.0.lock().unwrap();
        if m.len() >= CACHE {
            m.clear();
        }
        m.insert(k, v);
    }
}

/// **La caché de teselas** (D-S-4, D-F2-10): la misma petición da los mismos bytes (D-F2-6),
/// así que una tesela ya hecha no se vuelve a leer ni a calcular. Con tope en bytes; la usada
/// hace más se va primero. Las que fallan (plazo, tamaño) no se guardan.
/// Una tesela hecha (sus bytes, o vacía, y su cuenta).
type Hecha = Arc<Hecha1>;
/// Una tesela (sus bytes, o vacía) y lo que costó.
type Hecha1 = (Option<Vec<u8>>, Cuenta);

/// Las guardadas (con su último uso), los bytes que suman y el reloj del uso.
struct Guardadas {
    m: HashMap<String, (Hecha, u64)>,
    total: u64,
    reloj: u64,
}

struct Teselas {
    g: Mutex<Guardadas>,
    tope: u64,
}

impl Teselas {
    fn sacar(&self, k: &str) -> Option<Hecha> {
        let mut g = self.g.lock().unwrap();
        g.reloj += 1;
        let reloj = g.reloj;
        g.m.get_mut(k).map(|(v, t)| {
            *t = reloj;
            v.clone()
        })
    }

    fn guardar(&self, k: String, v: Hecha) {
        let largo = v.0.as_ref().map_or(0, |b| b.len() as u64) + k.len() as u64;
        if largo > self.tope {
            return;
        }
        let mut g = self.g.lock().unwrap();
        g.reloj += 1;
        let reloj = g.reloj;
        if let Some((viejo, _)) = g.m.insert(k.clone(), (v, reloj)) {
            g.total -= viejo.0.as_ref().map_or(0, |b| b.len() as u64) + k.len() as u64;
        }
        g.total += largo;
        if g.total > self.tope {
            let mut por_edad: Vec<(u64, String, u64)> =
                g.m.iter()
                    .map(|(k, (v, t))| {
                        (
                            *t,
                            k.clone(),
                            v.0.as_ref().map_or(0, |b| b.len() as u64) + k.len() as u64,
                        )
                    })
                    .collect();
            por_edad.sort_unstable_by_key(|e| e.0);
            let meta = self.tope / 10 * 9;
            for (_, k, n) in por_edad {
                if g.total <= meta {
                    break;
                }
                g.m.remove(&k);
                g.total -= n;
            }
        }
    }
}

/// La clave de una tesela: todo lo que decide sus bytes (no la credencial ni el plazo).
fn clave_de(p: &Peticion) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}/{}/{}|{}|{}",
        p.metadata_location,
        p.geometria,
        p.crs,
        p.id.as_deref().unwrap_or(""),
        p.capa,
        p.z,
        p.x,
        p.y,
        p.max_bytes,
        p.piramide.as_ref().map_or("", |f| f.ubicacion.as_str())
    )
}

pub struct Motor {
    pies: Cache<ArrowReaderMetadata>,
    listas: Cache<Arc<Vec<(String, u64)>>>,
    cuentas: Cache<(Arc<dyn Almacen>, Arc<Lago>)>,
    /// Las pirámides abiertas (su directorio entero), por ubicación: el PMTiles es inmutable.
    piramides: Cache<Arc<Piramide>>,
    /// El manifiesto vigente de cada índice (o que no hay), por ubicación de la fuente y por poco
    /// tiempo ([`VIDA_DEL_INDICE`]).
    indices: Mutex<HashMap<String, (Instant, Option<Json>)>>,
    /// De cada instantánea, lo que dice su `metadata.json` (inmutable).
    instantaneas: Cache<Arc<Instantanea>>,
    /// De cada instantánea, sus filas borradas por posición (I4·4).
    borradas: Cache<Arc<Borradas>>,
    paginas: Paginas,
    turnos: Turnos,
    teselas: Teselas,
}

impl Default for Motor {
    fn default() -> Self {
        Motor::nuevo()
    }
}

impl Motor {
    pub fn nuevo() -> Motor {
        Motor::con(Paginas::de_la_maquina(), Turnos::de_la_maquina())
    }

    pub fn con(paginas: Paginas, turnos: Turnos) -> Motor {
        Motor {
            pies: Cache::new(),
            listas: Cache::new(),
            cuentas: Cache::new(),
            piramides: Cache::new(),
            indices: Mutex::new(HashMap::new()),
            instantaneas: Cache::new(),
            borradas: Cache::new(),
            paginas,
            turnos,
            // `ORE_TESELAS_TESELAS_MB` (256)
            teselas: Teselas {
                g: Mutex::new(Guardadas {
                    m: HashMap::new(),
                    total: 0,
                    reloj: 0,
                }),
                tope: std::env::var("ORE_TESELAS_TESELAS_MB")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(256)
                    << 20,
            },
        }
    }

    /// Los bytes en la caché de páginas ahora.
    pub fn paginas_guardadas(&self) -> u64 {
        self.paginas.guardado()
    }

    /// Un turno de CPU dentro del plazo (lo que espera, en `turno_ms`).
    fn turno(&self, fin: Instant, cu: &mut Cuenta) -> Result<Turno<'_>, Fallo> {
        let t = Instant::now();
        let r = self.turnos.entrar(lector::queda(fin));
        cu.turno_ms += t.elapsed().as_millis() as u64;
        r.ok_or_else(|| {
            Fallo::Plazo("el motor está lleno: la tesela no tuvo turno de CPU en su plazo".into())
        })
    }

    /// Los pies de los ficheros: de la caché, o los que faltan a la vez (su cola en una
    /// oleada; el índice de páginas, si no cabe en la cola, aparte).
    fn pies_de(
        &self,
        cuenta: &Arc<dyn Almacen>,
        ficheros: &[(String, u64)],
        fin: Instant,
        cu: &mut Cuenta,
    ) -> Result<Vec<ArrowReaderMetadata>, Fallo> {
        let mut pies: Vec<Option<ArrowReaderMetadata>> =
            ficheros.iter().map(|(c, _)| self.pies.get(c)).collect();
        let faltan: Vec<(String, std::ops::Range<u64>)> = ficheros
            .iter()
            .zip(&pies)
            .filter(|(_, m)| m.is_none())
            .map(|((c, b), _)| (c.clone(), b.saturating_sub(lector::COLA)..*b))
            .collect();
        if !faltan.is_empty() {
            let mut colas = self.paginas.traer(cuenta, faltan, fin, cu)?;
            for ((clave, bytes), m) in ficheros.iter().zip(pies.iter_mut()) {
                if m.is_some() {
                    continue;
                }
                let t = Trozos {
                    cuenta: cuenta.clone(),
                    clave: clave.clone(),
                    largo: *bytes,
                    trozos: Arc::new(colas.remove(clave).unwrap_or_default()),
                    aparte: Arc::default(),
                };
                // con el índice de páginas: la tesela elige páginas, no grupos enteros
                let leido = ArrowReaderMetadata::load(
                    &t,
                    ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional),
                )
                .map_err(|e| Fallo::Interno(e.to_string()))?;
                lector::sumar_aparte(&t.aparte, cu);
                self.pies.put(clave.clone(), leido.clone());
                *m = Some(leido);
            }
        }
        Ok(pies.into_iter().flatten().collect())
    }

    /// La cuenta del almacén para esta credencial y este bucket (una por credencial: un
    /// cliente con sus conexiones vivas, no uno por tesela).
    fn cuenta(&self, p: &Peticion) -> Result<(Arc<dyn Almacen>, Arc<Lago>), Fallo> {
        self.cuenta_de(&p.metadata_location, &p.credencial)
    }

    fn cuenta_de(
        &self,
        ubicacion: &str,
        credencial: &Credencial,
    ) -> Result<(Arc<dyn Almacen>, Arc<Lago>), Fallo> {
        let resto = ubicacion.split_once("://").map_or("", |x| x.1);
        let bucket = resto.split('/').next().unwrap_or("").to_string();
        let k = match credencial {
            Credencial::Gcs { token } => format!("gs|{bucket}|{token}"),
            Credencial::S3 {
                endpoint, clave, ..
            } => format!("s3|{endpoint}|{bucket}|{clave}"),
        };
        let cache = &self.cuentas;
        if let Some(c) = cache.get(&k) {
            return Ok(c);
        }
        let cuenta: Arc<dyn Almacen> = match credencial {
            Credencial::Gcs { token } => {
                Arc::new(ore_store::gcs::Cuenta::con_token(&bucket, token).map_err(Fallo::Interno)?)
            }
            Credencial::S3 {
                endpoint,
                region,
                clave,
                secreto,
            } => Arc::new(ore_store::r2::Cuenta {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                clave: clave.clone(),
                secreto: secreto.clone(),
            }),
        };
        let v = (cuenta.clone(), Arc::new(Lago::nuevo(cuenta)));
        cache.put(k, v.clone());
        Ok(v)
    }

    /// Los ficheros vivos de la instantánea (clave, bytes) y sus filas borradas: el `metadata.json`
    /// es inmutable.
    fn ficheros(
        &self,
        p: &Peticion,
        lago: &Lago,
        cuenta: &Arc<dyn Almacen>,
    ) -> Result<Vivos, Fallo> {
        self.vivos_de(&p.metadata_location, lago, cuenta)
    }

    fn vivos_de(&self, ml: &str, lago: &Lago, cuenta: &Arc<dyn Almacen>) -> Result<Vivos, Fallo> {
        if let (Some(v), Some(b)) = (self.listas.get(ml), self.borradas.get(ml)) {
            return Ok((v, b));
        }
        let tabla = lago.abrir(ml, "teselas").map_err(Fallo::Interno)?;
        let (_, vivos, de_borrados) =
            ore_store::muestra::ficheros_vivos_y_borrados(lago, &tabla).map_err(Fallo::Invalida)?;
        let borradas = Arc::new(leer_borradas(cuenta, &de_borrados)?);
        self.borradas.put(ml.to_string(), borradas.clone());
        let cache = &self.listas;
        let v = Arc::new(
            vivos
                .into_iter()
                .map(|FicheroVivo { clave, bytes, .. }| (clave, bytes))
                .collect::<Vec<_>>(),
        );
        cache.put(ml.to_string(), v.clone());
        Ok((v, borradas))
    }

    /// **Una tesela**: `Ok(None)` si está vacía.
    pub fn tesela(&self, p: &Peticion) -> Result<(Option<Vec<u8>>, Cuenta), Fallo> {
        let t0 = Instant::now();
        if let Some(r) = self.de_la_cache(p) {
            return Ok(r);
        }
        let mut cu = Cuenta::default();
        if let Some(f) = &p.piramide {
            let (cuenta, _) = self.cuenta_de(&f.ubicacion, &f.credencial)?;
            if let Some(r) = self.de_la_piramide(p, f, &cuenta, t0, &mut cu)? {
                self.a_la_cache(p, &r);
                return Ok(r);
            }
        }
        let (cuenta, lago) = self.cuenta(p)?;
        let r = self.tesela_en(p, cuenta, &lago, t0, &mut cu)?;
        self.a_la_cache(p, &r);
        Ok(r)
    }

    /// **¿Hay pirámide, y qué cubre?** (F2·3·2·5, `POST /v1/piramide`): lo que `ore-serve`
    /// pregunta al hacer el descriptor. `{ubicacion, credencial}` → `Ok(None)` si no está;
    /// si está, sus zooms y su manifiesto (el `.json` de al lado, si lo hay). Abrirla la deja
    /// en memoria para las teselas.
    /// `(la pirámide si está, su fallo si el Job que la construía falló)` (F2·3·2·6).
    pub fn resumen_piramide(&self, texto: &str) -> Result<(Option<Json>, Option<Json>), Fallo> {
        let mal = |m: String| Fallo::Invalida(m);
        let n = ore_base::parse::parse(texto)
            .map_err(|e| mal(format!("el cuerpo no es JSON: {e:?}")))?;
        let ubicacion = n
            .get("ubicacion")
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| mal("falta `ubicacion`".into()))?;
        let c = n
            .get("credencial")
            .map(|(_, v)| v)
            .ok_or_else(|| mal("falta `credencial`".into()))?;
        let f = FuentePiramide {
            credencial: credencial_de(c, &ubicacion, "credencial")?,
            ubicacion,
        };
        let (cuenta, _) = self.cuenta_de(&f.ubicacion, &f.credencial)?;
        match self.resumen_en(&f, &cuenta)? {
            Some(r) => Ok((Some(r), None)),
            None => Ok((None, self.fallo_en(&f, &cuenta)?)),
        }
    }

    /// **¿Hay índice, y cuál?** (I3·1, `POST /v1/indice {ubicacion, metadata_location,
    /// credencial}`): lo que `ore-serve` pregunta al hacer el descriptor. `ubicacion` es la de la
    /// tabla de la capa; el índice vive bajo ella (`<tabla>/_ore_capa/<receta>/`), así que la
    /// credencial prestada para el dataset lo lee. Contesta `{indice?, instantanea_ms?, fuente}`:
    /// el manifiesto del índice de la instantánea de ahora o de la última anterior que lo tenga, y
    /// cuándo es la suya; y de la de ahora, sus filas y su secuencia, con las que el descriptor
    /// decide si hace falta un índice y si el que hay va por detrás (I3·2).
    pub fn resumen_indice(&self, texto: &str) -> Result<Json, Fallo> {
        let mal = |m: String| Fallo::Invalida(m);
        let n = ore_base::parse::parse(texto)
            .map_err(|e| mal(format!("el cuerpo no es JSON: {e:?}")))?;
        let ubicacion = n
            .get("ubicacion")
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| mal("falta `ubicacion`".into()))?;
        let c = n
            .get("credencial")
            .map(|(_, v)| v)
            .ok_or_else(|| mal("falta `credencial`".into()))?;
        let credencial = credencial_de(c, &ubicacion, "credencial")?;
        let ml = n
            .get("metadata_location")
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| mal("falta `metadata_location`".into()))?;
        let (cuenta, lago) = self.cuenta_de(&ubicacion, &credencial)?;
        let mut r = vec![];
        if let Some(m) = self.indice_en(&ubicacion, &ml, &cuenta, &lago)? {
            let s = Manifiesto::de_texto(&m.jcs()).map_err(Fallo::Interno)?;
            // cuándo es lo que pinta: lo que el descriptor enseña («datos de las hh:mm»)
            if let Ok((_, _, ms)) = self.instantanea_en(&s.snapshot_fuente, &lago) {
                r.push(("instantanea_ms", Json::s(ms.to_string())));
            }
            r.push(("indice", m));
        }
        let (filas, secuencia, ms) = self.instantanea_en(&ml, &lago)?;
        r.push((
            "fuente",
            Json::obj([
                ("filas", Json::s(filas.to_string())),
                ("secuencia", Json::s(secuencia.to_string())),
                ("ms", Json::s(ms.to_string())),
            ]),
        ));
        Ok(Json::obj(r))
    }

    /// **El clic** (0072 I3·3, `POST /v1/fila {metadata_location, ficheros, ordinal, fila,
    /// credencial}`): la fila entera de la tabla del cliente que un elemento del índice nombra por
    /// su posición. `ficheros` es la tabla de ordinales que el Job del índice dejó (la clave del
    /// objeto, en el cubo de la tabla); el fichero tiene que ser uno vivo de la instantánea
    /// `metadata_location`. Lee una fila de un grupo (con los pies en caché y el índice de
    /// páginas, sólo sus páginas) y la devuelve como texto, sin las columnas binarias (la
    /// geometría ya está en la tesela).
    pub fn fila(&self, texto: &str) -> Result<Json, Fallo> {
        let mal = |m: String| Fallo::Invalida(m);
        let n = ore_base::parse::parse(texto)
            .map_err(|e| mal(format!("el cuerpo no es JSON: {e:?}")))?;
        let s = |k: &str| {
            n.get(k)
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| mal(format!("falta `{k}`")))
        };
        let num = |k: &str| -> Result<u64, Fallo> {
            s(k)?
                .parse()
                .map_err(|_| mal(format!("`{k}` no es un entero")))
        };
        let ml = s("metadata_location")?;
        let c = n
            .get("credencial")
            .map(|(_, v)| v)
            .ok_or_else(|| mal("falta `credencial`".into()))?;
        let credencial = credencial_de(c, &ml, "credencial")?;
        let (cuenta, lago) = self.cuenta_de(&ml, &credencial)?;
        self.fila_en(
            &ml,
            &s("ficheros")?,
            num("ordinal")?,
            num("fila")?,
            &cuenta,
            &lago,
        )
    }

    /// Lo mismo con el almacén ya dado (las pruebas, sin red).
    pub fn fila_en(
        &self,
        ml: &str,
        ficheros: &str,
        ordinal: u64,
        fila: u64,
        cuenta: &Arc<dyn Almacen>,
        lago: &Lago,
    ) -> Result<Json, Fallo> {
        let fuera = |m: String| Fallo::Invalida(m);
        // el ordinal → la URI del fichero (la tabla que el Job dejó junto al índice)
        let tabla = cuenta
            .leer(ficheros)
            .map_err(Fallo::Interno)?
            .ok_or_else(|| fuera(format!("no está la tabla de ficheros `{ficheros}`")))?;
        let n = ore_base::parse::parse(&tabla)
            .map_err(|e| Fallo::Interno(format!("`{ficheros}`: {e:?}")))?;
        let uri = n
            .get("ficheros")
            .map(|(_, v)| v.items())
            .unwrap_or_default()
            .iter()
            .find_map(|par| {
                let p = par.items();
                (p.first()?.as_str()?.parse::<u64>().ok()? == ordinal)
                    .then(|| p.get(1)?.as_str().map(str::to_string))
                    .flatten()
            })
            .ok_or_else(|| fuera(format!("el ordinal {ordinal} no es de este índice")))?;
        // y tiene que ser un fichero vivo de la instantánea
        let clave = clave_de_objeto(&uri)?;
        let (vivos, borradas) = self.vivos_de(ml, lago, cuenta)?;
        if borradas
            .get(&clave)
            .is_some_and(|b| b.binary_search(&fila).is_ok())
        {
            return Err(fuera(format!("la fila {fila} de `{uri}` está borrada")));
        }
        let Some((clave, bytes)) = vivos.iter().find(|(c, _)| *c == clave).cloned() else {
            return Err(fuera(format!(
                "`{uri}` no es un fichero vivo de la instantánea"
            )));
        };
        let mut cu = Cuenta::default();
        let fin = Instant::now() + Duration::from_secs(10);
        let meta = self
            .pies_de(cuenta, &[(clave.clone(), bytes)], fin, &mut cu)?
            .remove(0);
        // el grupo de la fila, y su sitio dentro
        let (mut grupo, mut desde) = (None, 0u64);
        for (g, rg) in meta.metadata().row_groups().iter().enumerate() {
            let n = rg.num_rows().max(0) as u64;
            if fila < desde + n {
                grupo = Some(g);
                break;
            }
            desde += n;
        }
        let Some(grupo) = grupo else {
            return Err(fuera(format!("`{uri}` no tiene la fila {fila}")));
        };
        // sin lo binario (la geometría) ni su cobertura `<col>_bbox`, que es de ORE
        let campos = meta.schema().fields();
        let binaria = |f: &arrow_schema::Field| {
            matches!(
                f.data_type(),
                arrow_schema::DataType::Binary
                    | arrow_schema::DataType::LargeBinary
                    | arrow_schema::DataType::BinaryView
            )
        };
        let raices: Vec<usize> = campos
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                !binaria(f)
                    && !f
                        .name()
                        .strip_suffix("_bbox")
                        .is_some_and(|g| campos.iter().any(|o| o.name() == g && binaria(o)))
            })
            .map(|(i, _)| i)
            .collect();
        let esquema = meta.parquet_schema();
        let hojas: Vec<usize> = (0..esquema.num_columns())
            .filter(|&h| raices.contains(&esquema.get_column_root_idx(h)))
            .collect();
        let sel = RowSelection::from(vec![
            parquet::arrow::arrow_reader::RowSelector::skip((fila - desde) as usize),
            parquet::arrow::arrow_reader::RowSelector::select(1),
        ]);
        // lo que hace falta de cada columna (con el índice de páginas, su diccionario y la página
        // de la fila), en una oleada y en paralelo: de una en una eran ~1,5 s (I3·4)
        let mut traido = self.paginas.traer(
            cuenta,
            unidades(&meta, &clave, grupo, &hojas, &sel),
            fin,
            &mut cu,
        )?;
        let t = Trozos {
            cuenta: cuenta.clone(),
            trozos: Arc::new(traido.remove(&clave).unwrap_or_default()),
            clave,
            largo: bytes,
            aparte: Arc::default(),
        };
        let lector = ParquetRecordBatchReaderBuilder::new_with_metadata(t, meta.clone())
            .with_row_groups(vec![grupo])
            .with_projection(ProjectionMask::leaves(esquema, hojas.iter().copied()))
            .with_row_selection(sel)
            .build()
            .map_err(|e| Fallo::Interno(e.to_string()))?;
        let mut propiedades = vec![];
        for lote in lector {
            let lote = lote.map_err(|e| Fallo::Interno(e.to_string()))?;
            if let Some(f) = ore_store::carga::filas_para_ver(&lote).into_iter().next() {
                propiedades = f.into_iter().collect();
                break;
            }
        }
        Ok(Json::obj([
            ("fichero", Json::s(&uri)),
            ("fila", Json::s(fila.to_string())),
            (
                "propiedades",
                Json::Obj(
                    propiedades
                        .into_iter()
                        .map(|(k, v)| (k, Json::s(v)))
                        .collect(),
                ),
            ),
        ]))
    }

    /// `(filas, secuencia, cuándo en ms)` de una instantánea: lo dice su snapshot actual
    /// (`total-records` del resumen, su número de secuencia y su hora).
    pub fn instantanea_en(&self, ml: &str, lago: &Lago) -> Result<(u64, u64, i64), Fallo> {
        let i = self.instantanea(ml, lago)?;
        Ok((i.filas, i.secuencia, i.ms))
    }

    fn instantanea(&self, ml: &str, lago: &Lago) -> Result<Arc<Instantanea>, Fallo> {
        if let Some(v) = self.instantaneas.get(ml) {
            return Ok(v);
        }
        let t = lago.abrir(ml, "teselas").map_err(Fallo::Invalida)?;
        let (filas, secuencia, ms) = t.metadata().current_snapshot().map_or((0, 0, 0), |s| {
            (
                s.summary()
                    .additional_properties
                    .get("total-records")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0),
                s.sequence_number().max(0) as u64,
                s.timestamp_ms(),
            )
        });
        let v = Arc::new(Instantanea {
            filas,
            secuencia,
            ms,
            anteriores: t
                .metadata()
                .metadata_log()
                .iter()
                .rev()
                .map(|l| l.metadata_file.clone())
                .filter(|f| f != ml)
                .collect(),
        });
        self.instantaneas.put(ml.to_string(), v.clone());
        Ok(v)
    }

    /// **El índice de la instantánea `ml`, o de la última anterior que lo tenga** (I3·4): con GETs
    /// por nombre (el de `ml` y los de su `metadata-log`, hasta [`HISTORIA`]) y no listando, que la
    /// credencial prestada para el dataset no puede. Lo que dijo vale [`VIDA_DEL_INDICE`]. Un
    /// manifiesto de otra fuente bajo esta tabla no se sirve: sería pintar otra cosa.
    pub fn indice_en(
        &self,
        ubicacion: &str,
        ml: &str,
        cuenta: &Arc<dyn Almacen>,
        lago: &Lago,
    ) -> Result<Option<Json>, Fallo> {
        let ubicacion = ubicacion.trim_end_matches('/');
        let k = format!("{ubicacion}|{ml}");
        if let Some((cuando, r)) = self.indices.lock().unwrap().get(&k)
            && cuando.elapsed() < VIDA_DEL_INDICE
        {
            return Ok(r.clone());
        }
        let mut candidatas = vec![ml.to_string()];
        candidatas.extend(
            self.instantanea(ml, lago)?
                .anteriores
                .iter()
                .take(HISTORIA)
                .cloned(),
        );
        let r = self.buscar_indice(ubicacion, &candidatas, cuenta)?;
        let mut g = self.indices.lock().unwrap();
        if g.len() >= CACHE {
            g.clear();
        }
        g.insert(k, (Instant::now(), r.clone()));
        Ok(r)
    }

    /// El manifiesto de la primera de `candidatas` (de la más nueva a la más vieja) que tenga
    /// índice: un GET por cada una hasta dar con él.
    fn buscar_indice(
        &self,
        ubicacion: &str,
        candidatas: &[String],
        cuenta: &Arc<dyn Almacen>,
    ) -> Result<Option<Json>, Fallo> {
        let prefijo =
            indice::prefijo_en_el_almacen(ubicacion, &indice::I1).map_err(Fallo::Invalida)?;
        for c in candidatas {
            let Some(m) =
                indice::de_la_instantanea(cuenta.as_ref(), &prefijo, c).map_err(Fallo::Interno)?
            else {
                continue;
            };
            if m.fuente.ubicacion.trim_end_matches('/') != ubicacion {
                return Err(Fallo::Interno(format!(
                    "el índice bajo `{ubicacion}` es de `{}`",
                    m.fuente.ubicacion
                )));
            }
            return Ok(Some(m.a_json()));
        }
        Ok(None)
    }

    /// El fallo del Job de una pirámide que no está (`<clave>.fallo.json`, lo deja `ore-piramide`
    /// al fallar y lo quita al acabar bien), o `None`.
    pub fn fallo_en(
        &self,
        f: &FuentePiramide,
        cuenta: &Arc<dyn Almacen>,
    ) -> Result<Option<Json>, Fallo> {
        let clave = clave_de_objeto(&f.ubicacion)?;
        Ok(cuenta
            .leer(&format!("{clave}.fallo.json"))
            .map_err(Fallo::Interno)?
            .filter(|t| ore_base::parse::parse(t).is_ok())
            .map(Json::Crudo))
    }

    /// Lo mismo con el almacén ya dado (las pruebas, sin red).
    pub fn resumen_en(
        &self,
        f: &FuentePiramide,
        cuenta: &Arc<dyn Almacen>,
    ) -> Result<Option<Json>, Fallo> {
        let clave = clave_de_objeto(&f.ubicacion)?;
        if self.piramides.get(&f.ubicacion).is_none()
            && !cuenta.existe(&clave).map_err(Fallo::Interno)?
        {
            return Ok(None);
        }
        let mut cu = Cuenta::default();
        let fin = Instant::now() + Duration::from_secs(10);
        let pi = self.piramide(f, cuenta, &clave, fin, &mut cu)?;
        let manifiesto = cuenta
            .leer(&format!("{clave}.json"))
            .ok()
            .flatten()
            .filter(|t| ore_base::parse::parse(t).is_ok())
            .map_or(Json::Obj(Default::default()), Json::Crudo);
        Ok(Some(Json::obj([
            ("minzoom", Json::Int(pi.cabecera.minzoom as i64)),
            ("maxzoom", Json::Int(pi.cabecera.maxzoom as i64)),
            ("manifiesto", manifiesto),
        ])))
    }

    /// La pirámide abierta (de la memoria, o abriéndola: dos oleadas, una vez).
    fn piramide(
        &self,
        f: &FuentePiramide,
        cuenta: &Arc<dyn Almacen>,
        clave: &str,
        fin: Instant,
        cu: &mut Cuenta,
    ) -> Result<Arc<Piramide>, Fallo> {
        if let Some(pi) = self.piramides.get(&f.ubicacion) {
            return Ok(pi);
        }
        let pi = Arc::new(Piramide::abrir(&self.paginas, cuenta, clave, fin, cu)?);
        self.piramides.put(f.ubicacion.clone(), pi.clone());
        Ok(pi)
    }

    /// **Una tesela de la pirámide** (F2·3·2·1): `Ok(None)` si su zoom no es de la pirámide (la
    /// sirve el crudo). Abrirla son dos oleadas, una vez; cada tesela, un GET (o ninguno).
    fn de_la_piramide(
        &self,
        p: &Peticion,
        f: &FuentePiramide,
        cuenta: &Arc<dyn Almacen>,
        t0: Instant,
        cu: &mut Cuenta,
    ) -> Result<Option<Hecha1>, Fallo> {
        let fin = t0 + Duration::from_millis(p.plazo_ms);
        let clave = clave_de_objeto(&f.ubicacion)?;
        let t1 = Instant::now();
        let pi = self.piramide(f, cuenta, &clave, fin, cu)?;
        let c = &pi.cabecera;
        if p.z < c.minzoom || p.z > c.maxzoom {
            return Ok(None);
        }
        cu.piramide = 1;
        let Some(r) = pi.donde(p.z, p.x, p.y) else {
            cu.leer_ms = t1.elapsed().as_millis() as u64;
            return Ok(Some((None, cu.clone())));
        };
        let a = r.start;
        let mut t = self
            .paginas
            .traer(cuenta, vec![(clave.clone(), r)], fin, cu)?;
        let crudo = t
            .remove(&clave)
            .and_then(|mut m| m.remove(&a))
            .ok_or_else(|| Fallo::Interno("el almacén no devolvió la tesela".into()))?;
        cu.leer_ms = t1.elapsed().as_millis() as u64;
        let t2 = Instant::now();
        let bytes = pi.mvt(&crudo, &p.capa).map_err(Fallo::Interno)?;
        cu.codificar_ms = t2.elapsed().as_millis() as u64;
        if bytes.len() > p.max_bytes {
            return Err(Fallo::Plazo(format!(
                "la tesela de la pirámide pesa {} bytes y el límite es {}",
                bytes.len(),
                p.max_bytes
            )));
        }
        Ok(Some((Some(bytes), cu.clone())))
    }

    /// Una tesela ya hecha: sus bytes, y su cuenta sin lectura (nada se leyó esta vez).
    fn de_la_cache(&self, p: &Peticion) -> Option<(Option<Vec<u8>>, Cuenta)> {
        let v = self.teselas.sacar(&clave_de(p))?;
        let mut c = v.1.clone();
        (c.peticiones, c.oleadas, c.bytes, c.en_cache, c.compartidas) = (0, 0, 0, 0, 0);
        (
            c.listar_ms,
            c.leer_ms,
            c.turno_ms,
            c.criba_ms,
            c.geom_ms,
            c.codificar_ms,
        ) = (0, 0, 0, 0, 0, 0);
        c.tesela_en_cache = 1;
        Some((v.0.clone(), c))
    }

    fn a_la_cache(&self, p: &Peticion, r: &(Option<Vec<u8>>, Cuenta)) {
        self.teselas.guardar(clave_de(p), Arc::new(r.clone()));
    }

    /// Lo mismo con el almacén ya dado (las pruebas, sin red).
    pub fn tesela_con(
        &self,
        p: &Peticion,
        cuenta: Arc<dyn Almacen>,
    ) -> Result<(Option<Vec<u8>>, Cuenta), Fallo> {
        if let Some(r) = self.de_la_cache(p) {
            return Ok(r);
        }
        let mut cu = Cuenta::default();
        if let Some(f) = &p.piramide
            && let Some(r) = self.de_la_piramide(p, f, &cuenta, Instant::now(), &mut cu)?
        {
            self.a_la_cache(p, &r);
            return Ok(r);
        }
        let lago = Lago::nuevo(cuenta.clone());
        let r = self.tesela_en(p, cuenta, &lago, Instant::now(), &mut cu)?;
        self.a_la_cache(p, &r);
        Ok(r)
    }

    fn tesela_en(
        &self,
        p: &Peticion,
        cuenta: Arc<dyn Almacen>,
        lago: &Lago,
        t0: Instant,
        cu: &mut Cuenta,
    ) -> Result<(Option<Vec<u8>>, Cuenta), Fallo> {
        let plazo = |c: &str| {
            if t0.elapsed().as_millis() as u64 > p.plazo_ms {
                Err(Fallo::Plazo(format!(
                    "la tesela pasó su plazo ({} ms) al {c}",
                    p.plazo_ms
                )))
            } else {
                Ok(())
            }
        };
        let (ficheros, borradas) = self.ficheros(p, lago, &cuenta)?;
        cu.listar_ms = t0.elapsed().as_millis() as u64;

        let lonlat = p.crs != "EPSG:3857";
        let c3857 = caja_3857(p.z, p.x, p.y);
        // la caja en el CRS de los datos, con el margen de la tesela
        let cd = {
            let (a, b, c, d) = if lonlat {
                caja_lonlat(p.z, p.x, p.y)
            } else {
                c3857
            };
            let (mx, my) = ((c - a) * BUFFER / EXTENT, (d - b) * BUFFER / EXTENT);
            (a - mx, b - my, c + mx, d + my)
        };
        let t1 = Instant::now();
        let fin = t0 + Duration::from_millis(p.plazo_ms);
        let cobertura = format!("{}_bbox", p.geometria);
        let pies = self.pies_de(&cuenta, &ficheros, fin, cu)?;

        // ── el plan: los grupos que tocan y, en cada uno, las filas de las páginas que tocan ──
        struct Grupo {
            f: usize,
            rg: usize,
            caja: Option<Vec<usize>>,
            datos: Vec<usize>,
            sel: RowSelection,
        }
        let mut grupos: Vec<Grupo> = vec![];
        for (f, meta) in pies.iter().enumerate() {
            let esquema = meta.schema().clone();
            esquema
                .index_of(&p.geometria)
                .map_err(|_| Fallo::Invalida(format!("no hay columna `{}`", p.geometria)))?;
            if let Some(id) = &p.id {
                esquema
                    .index_of(id)
                    .map_err(|_| Fallo::Invalida(format!("no hay columna `{id}`")))?;
            }
            let cols = meta.parquet_schema().columns();
            let hoja = |campo: &str| {
                cols.iter().position(|c| {
                    let partes = c.path().parts();
                    partes.len() == 2 && partes[0] == cobertura && partes[1] == campo
                })
            };
            let caja = match (hoja("xmin"), hoja("ymin"), hoja("xmax"), hoja("ymax")) {
                (Some(a), Some(b), Some(c), Some(d)) => Some([a, b, c, d]),
                // sin cobertura (un fichero de antes de F1): se lee el grupo entero
                _ => None,
            };
            let datos: Vec<usize> = (0..cols.len())
                .filter(|&i| {
                    let raiz = &cols[i].path().parts()[0];
                    *raiz == p.geometria || p.id.as_deref() == Some(raiz.as_str())
                })
                .collect();
            let fuera = borradas.get(&ficheros[f].0);
            let mut inicio = 0u64;
            for (rg, g) in meta.metadata().row_groups().iter().enumerate() {
                cu.grupos += 1;
                let desde = inicio;
                inicio += g.num_rows().max(0) as u64;
                let toca = match caja {
                    Some([a, b, c, d]) => match (
                        minmax(g.column(a).statistics()),
                        minmax(g.column(b).statistics()),
                        minmax(g.column(c).statistics()),
                        minmax(g.column(d).statistics()),
                    ) {
                        // xmin ≤ caja.x1, ymin ≤ caja.y1, xmax ≥ caja.x0, ymax ≥ caja.y0
                        (Some(xa), Some(ya), Some(xb), Some(yb)) => {
                            xa.0 <= cd.2 && ya.0 <= cd.3 && xb.1 >= cd.0 && yb.1 >= cd.1
                        }
                        _ => true,
                    },
                    None => true,
                };
                if !toca {
                    continue;
                }
                cu.grupos_leidos += 1;
                cu.filas_en_grupos += g.num_rows().max(0) as usize;
                let sel = match caja {
                    Some(h) => paginas_que_tocan(meta, rg, h, cd),
                    None => {
                        RowSelection::from(vec![parquet::arrow::arrow_reader::RowSelector::select(
                            g.num_rows().max(0) as usize,
                        )])
                    }
                };
                // I4·4: lo borrado por posición no se lee (el índice que sigue a su tabla; una
                // tabla escrita con merge-on-read)
                let sel = match fuera {
                    Some(b) => {
                        sel.intersection(&sin_borradas(b, desde, g.num_rows().max(0) as usize))
                    }
                    None => sel,
                };
                if !sel.selects_any() {
                    continue;
                }
                cu.filas_en_paginas += sel.row_count();
                grupos.push(Grupo {
                    f,
                    rg,
                    caja: caja.map(|h| h.to_vec()),
                    datos: datos.clone(),
                    sel,
                });
            }
        }
        let trozos_de =
            |traido: &HashMap<String, std::collections::BTreeMap<u64, bytes::Bytes>>, f: usize| {
                Trozos {
                    cuenta: cuenta.clone(),
                    clave: ficheros[f].0.clone(),
                    largo: ficheros[f].1,
                    trozos: Arc::new(traido.get(&ficheros[f].0).cloned().unwrap_or_default()),
                    aparte: Arc::default(),
                }
            };

        // ── oleada 1: las páginas de la cobertura; con turno, la criba por fila (D-F2-2) ──
        let pedidas: Vec<_> = grupos
            .iter()
            .filter_map(|g| g.caja.as_ref().map(|h| (g, h)))
            .flat_map(|(g, h)| unidades(&pies[g.f], &ficheros[g.f].0, g.rg, h, &g.sel))
            .collect();
        if !pedidas.is_empty() {
            let traido = self.paginas.traer(&cuenta, pedidas, fin, cu)?;
            plazo("leer")?;
            let _turno = self.turno(fin, cu)?;
            let tc = Instant::now();
            for g in grupos.iter_mut() {
                let Some(h) = &g.caja else { continue };
                let t = trozos_de(&traido, g.f);
                let aparte = t.aparte.clone();
                let meta = &pies[g.f];
                let lector = ParquetRecordBatchReaderBuilder::new_with_metadata(t, meta.clone())
                    .with_row_groups(vec![g.rg])
                    .with_projection(ProjectionMask::leaves(
                        meta.parquet_schema(),
                        h.iter().copied(),
                    ))
                    .with_row_selection(g.sel.clone())
                    .with_batch_size(65_536)
                    .build()
                    .map_err(|e| Fallo::Interno(e.to_string()))?;
                let mut tocan = vec![];
                for lote in lector {
                    let lote = lote.map_err(|e| Fallo::Interno(e.to_string()))?;
                    tocan.push(
                        toca_la_caja(&lote, &cobertura, cd)
                            .map_err(|e| Fallo::Interno(e.to_string()))?,
                    );
                }
                lector::sumar_aparte(&aparte, cu);
                g.sel = g.sel.and_then(&RowSelection::from_filters(&tocan));
            }
            cu.criba_ms = tc.elapsed().as_millis() as u64;
            plazo("leer")?;
        }
        grupos.retain(|g| g.sel.selects_any());

        // ── oleada 2: `geom` e `id` de las filas que tocan; con turno, hasta el final ──
        let pedidas: Vec<_> = grupos
            .iter()
            .flat_map(|g| unidades(&pies[g.f], &ficheros[g.f].0, g.rg, &g.datos, &g.sel))
            .collect();
        let traido = self.paginas.traer(&cuenta, pedidas, fin, cu)?;
        plazo("leer")?;
        let _turno = self.turno(fin, cu)?;
        let mut geoms: Vec<(Option<u64>, Vec<u8>)> = Vec::new();
        for g in &grupos {
            let t = trozos_de(&traido, g.f);
            let aparte = t.aparte.clone();
            let meta = &pies[g.f];
            let lector = ParquetRecordBatchReaderBuilder::new_with_metadata(t, meta.clone())
                .with_row_groups(vec![g.rg])
                .with_projection(ProjectionMask::leaves(
                    meta.parquet_schema(),
                    g.datos.iter().copied(),
                ))
                .with_row_selection(g.sel.clone())
                .with_batch_size(65_536)
                .build()
                .map_err(|e| Fallo::Interno(e.to_string()))?;
            for lote in lector {
                plazo("leer")?;
                let lote = lote.map_err(|e| Fallo::Interno(e.to_string()))?;
                recoger(&lote, &p.geometria, p.id.as_deref(), &mut geoms)?;
            }
            lector::sumar_aparte(&aparte, cu);
        }
        cu.filas = geoms.len();
        cu.leer_ms = t1.elapsed().as_millis() as u64;
        plazo("descodificar")?;

        let t2 = Instant::now();
        let a_tesela = |x: f64, y: f64| -> (f64, f64) {
            let (mx, my) = if lonlat {
                (
                    x.to_radians() * R,
                    R * (std::f64::consts::FRAC_PI_4 + y.to_radians() / 2.0)
                        .tan()
                        .ln(),
                )
            } else {
                (x, y)
            };
            (
                (mx - c3857.0) / (c3857.2 - c3857.0) * EXTENT,
                (c3857.3 - my) / (c3857.3 - c3857.1) * EXTENT,
            )
        };
        let mut salida: Vec<Elemento> = Vec::new();
        for (k, (id, wkb)) in geoms.iter().enumerate() {
            if k % 4096 == 0 {
                plazo("recortar")?;
            }
            let Some(gs) = geometrias(wkb, &a_tesela) else {
                cu.descartados += 1;
                continue;
            };
            // desempate estable sin id: la huella de la geometría (D-F2-6)
            let desempate = id.unwrap_or_else(|| fnv(wkb));
            for g in gs {
                let Some((tipo, partes, (cx, cy))) = a_la_tesela(g, &mut cu.diminutos) else {
                    continue;
                };
                salida.push(Elemento {
                    hilbert: hilbert(cx, cy),
                    desempate,
                    tipo,
                    id: *id,
                    partes,
                });
            }
        }
        // el tipo desempata las piezas de una misma colección
        salida.sort_by_key(|e| (e.hilbert, e.desempate, e.tipo));
        cu.elementos = salida.len();
        for e in &salida {
            match e.tipo {
                PUNTO => cu.puntos += 1,
                LINEA => cu.lineas += 1,
                _ => cu.poligonos += 1,
            }
        }
        cu.geom_ms = t2.elapsed().as_millis() as u64;
        if salida.is_empty() {
            return Ok((None, cu.clone()));
        }
        let t3 = Instant::now();
        let bytes = mvt(&salida, &p.capa);
        cu.codificar_ms = t3.elapsed().as_millis() as u64;
        if bytes.len() > p.max_bytes {
            return Err(Fallo::Plazo(format!(
                "la tesela pesa {} bytes y el límite es {}: este zoom pide agregado (F4) o subir el suelo",
                bytes.len(),
                p.max_bytes
            )));
        }
        Ok((Some(bytes), cu.clone()))
    }
}

/// De un lote, las geometrías WKB y su id (si lo hay).
/// Las filas de un lote: su id (si la columna lo da) y su WKB; las nulas, fuera. Lo usa también
/// `ore-piramide` (F2·3·2·2): las dos leen igual.
pub fn recoger(
    lote: &RecordBatch,
    geometria: &str,
    id: Option<&str>,
    out: &mut Vec<(Option<u64>, Vec<u8>)>,
) -> Result<(), Fallo> {
    let g = lote
        .column_by_name(geometria)
        .ok_or_else(|| Fallo::Invalida(format!("no hay columna `{geometria}`")))?;
    let ids = id.and_then(|c| lote.column_by_name(c));
    let id_de = |i: usize| -> Option<u64> {
        let c = ids?;
        if c.is_null(i) {
            return None;
        }
        if let Some(a) = c.as_any().downcast_ref::<Int64Array>() {
            return u64::try_from(a.value(i)).ok();
        }
        if let Some(a) = c.as_any().downcast_ref::<Int32Array>() {
            return u64::try_from(a.value(i)).ok();
        }
        None
    };
    if let Some(a) = g.as_any().downcast_ref::<LargeBinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                out.push((id_de(i), a.value(i).to_vec()));
            }
        }
    } else if let Some(a) = g.as_any().downcast_ref::<BinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                out.push((id_de(i), a.value(i).to_vec()));
            }
        }
    } else {
        return Err(Fallo::Invalida(format!(
            "`{geometria}` no es WKB (binary): {}",
            g.data_type()
        )));
    }
    Ok(())
}

/// La criba por fila: ¿toca la caja de cada fila (`<geom>_bbox`, en el CRS de los datos) la
/// caja de la tesela con su margen? Una fila sin caja (geometría vacía) no pinta nada: fuera.
fn toca_la_caja(
    lote: &RecordBatch,
    cobertura: &str,
    c: (f64, f64, f64, f64),
) -> Result<BooleanArray, ArrowError> {
    let s = lote
        .column_by_name(cobertura)
        .and_then(|a| a.as_any().downcast_ref::<StructArray>())
        .ok_or_else(|| ArrowError::SchemaError(format!("`{cobertura}` no es una cobertura")))?;
    let hoja = |n: &str| -> Result<Vec<f64>, ArrowError> {
        let a = s
            .column_by_name(n)
            .ok_or_else(|| ArrowError::SchemaError(format!("`{cobertura}` sin `{n}`")))?;
        let nulo = |i: usize| s.is_null(i) || a.is_null(i);
        if let Some(f) = a.as_any().downcast_ref::<Float64Array>() {
            Ok((0..f.len())
                .map(|i| if nulo(i) { f64::NAN } else { f.value(i) })
                .collect())
        } else if let Some(f) = a.as_any().downcast_ref::<Float32Array>() {
            Ok((0..f.len())
                .map(|i| if nulo(i) { f64::NAN } else { f.value(i) as f64 })
                .collect())
        } else {
            Err(ArrowError::SchemaError(format!(
                "`{cobertura}.{n}` no es un número: {}",
                a.data_type()
            )))
        }
    };
    let (x0, y0, x1, y1) = (hoja("xmin")?, hoja("ymin")?, hoja("xmax")?, hoja("ymax")?);
    // NaN compara falso: la fila sin caja queda fuera
    Ok(BooleanArray::from(
        (0..s.len())
            .map(|i| x0[i] <= c.2 && y0[i] <= c.3 && x1[i] >= c.0 && y1[i] >= c.1)
            .collect::<Vec<bool>>(),
    ))
}

/// Las páginas de un fichero (su índice de offsets), si lo trae.
fn paginas_de(meta: &ArrowReaderMetadata, rg: usize, hoja: usize) -> Option<&[PageLocation]> {
    meta.metadata()
        .offset_index()
        .and_then(|o| o.get(rg))
        .and_then(|r| r.get(hoja))
        .map(|o| o.page_locations().as_slice())
        .filter(|l| !l.is_empty())
}

/// El mínimo y el máximo de una página: `None` si el índice no lo sabe (la página pasa),
/// `Some(None)` si la página es toda nulos (no pinta nada).
fn pagina(ci: &ColumnIndexMetaData, i: usize) -> Option<Option<(f64, f64)>> {
    if (i as u64) >= ci.num_pages() {
        return None;
    }
    match ci {
        ColumnIndexMetaData::DOUBLE(c) => {
            Some(c.min_value(i).zip(c.max_value(i)).map(|(a, b)| (*a, *b)))
        }
        ColumnIndexMetaData::FLOAT(c) => Some(
            c.min_value(i)
                .zip(c.max_value(i))
                .map(|(a, b)| (*a as f64, *b as f64)),
        ),
        _ => None,
    }
}

/// **Las filas de un grupo cuyas páginas de cobertura pueden tocar la tesela** (D-F2-2, por
/// página): el índice de columnas de cada una de las cuatro hojas descarta las páginas que
/// no la tocan, y sólo se leen las filas que ninguna hoja descarta. Sin índice, todas.
fn paginas_que_tocan(
    meta: &ArrowReaderMetadata,
    rg: usize,
    hojas: [usize; 4],
    cd: (f64, f64, f64, f64),
) -> RowSelection {
    let m = meta.metadata();
    let n = m.row_group(rg).num_rows().max(0) as usize;
    let mut toca = vec![true; n];
    for (k, &hoja) in hojas.iter().enumerate() {
        let ci = m
            .column_index()
            .and_then(|c| c.get(rg))
            .and_then(|c| c.get(hoja));
        let (Some(ci), Some(locs)) = (ci, paginas_de(meta, rg, hoja)) else {
            continue;
        };
        for (i, loc) in locs.iter().enumerate() {
            let desde = (loc.first_row_index.max(0) as usize).min(n);
            let hasta = locs
                .get(i + 1)
                .map_or(n, |s| s.first_row_index.max(0) as usize)
                .min(n);
            let pasa = match pagina(ci, i) {
                None => true,
                Some(None) => false,
                // xmin ≤ caja.x1, ymin ≤ caja.y1, xmax ≥ caja.x0, ymax ≥ caja.y0
                Some(Some((min, max))) => match k {
                    0 => min <= cd.2,
                    1 => min <= cd.3,
                    2 => max >= cd.0,
                    _ => max >= cd.1,
                },
            };
            if !pasa {
                toca[desde..hasta].fill(false);
            }
        }
    }
    let mut tramos = vec![];
    let mut i = 0;
    while i < n {
        if toca[i] {
            let a = i;
            while i < n && toca[i] {
                i += 1;
            }
            tramos.push(a..i);
        } else {
            i += 1;
        }
    }
    RowSelection::from_consecutive_ranges(tramos.into_iter(), n)
}

/// **Lee los ficheros de borrados por posición** (`file_path`, `pos`) de una instantánea: por
/// fichero de datos (su clave), sus posiciones ordenadas y sin repetir.
fn leer_borradas(cuenta: &Arc<dyn Almacen>, claves: &[String]) -> Result<Borradas, Fallo> {
    use arrow_array::{LargeStringArray, StringArray};
    let mut out: Borradas = HashMap::new();
    for clave in claves {
        let b = cuenta
            .leer_bytes(clave)
            .map_err(Fallo::Interno)?
            .ok_or_else(|| Fallo::Interno(format!("el fichero de borrados `{clave}` no está")))?;
        let lector = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(b))
            .and_then(|b| b.build())
            .map_err(|e| Fallo::Interno(format!("`{clave}`: {e}")))?;
        for lote in lector {
            let lote = lote.map_err(|e| Fallo::Interno(format!("`{clave}`: {e}")))?;
            let (Some(rutas), Some(pos)) =
                (lote.column_by_name("file_path"), lote.column_by_name("pos"))
            else {
                return Err(Fallo::Interno(format!(
                    "`{clave}` no es un fichero de borrados por posición (`file_path`, `pos`)"
                )));
            };
            let pos = pos
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| Fallo::Interno(format!("`{clave}`: `pos` no es Int64")))?;
            let ruta = |i: usize| -> Option<&str> {
                if let Some(a) = rutas.as_any().downcast_ref::<StringArray>() {
                    return Some(a.value(i));
                }
                rutas
                    .as_any()
                    .downcast_ref::<LargeStringArray>()
                    .map(|a| a.value(i))
            };
            for i in 0..lote.num_rows() {
                let r = ruta(i)
                    .ok_or_else(|| Fallo::Interno(format!("`{clave}`: `file_path` no es texto")))?;
                out.entry(clave_de_objeto(r)?)
                    .or_default()
                    .push(pos.value(i).max(0) as u64);
            }
        }
    }
    for v in out.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    Ok(out)
}

/// **Lo que una tesela pide de unas hojas de un grupo**: con el índice de offsets, la página
/// de diccionario y las páginas de las filas elegidas; sin él, el trozo de columna entero.
fn unidades(
    meta: &ArrowReaderMetadata,
    clave: &str,
    rg: usize,
    hojas: &[usize],
    sel: &RowSelection,
) -> Vec<(String, std::ops::Range<u64>)> {
    let mut out = vec![];
    for &h in hojas {
        let col = meta.metadata().row_group(rg).column(h);
        match paginas_de(meta, rg, h) {
            Some(locs) => {
                // lo que hay antes de la primera página es la de diccionario, como la lee
                // `parquet` (aunque el pie no traiga `dictionary_page_offset`)
                let (a, z) = (col.byte_range().0, locs[0].offset.max(0) as u64);
                if z > a {
                    out.push((clave.to_string(), a..z));
                }
                out.extend(
                    sel.scan_ranges(locs)
                        .into_iter()
                        .map(|r| (clave.to_string(), r)),
                );
            }
            None => {
                let (a, n) = col.byte_range();
                out.push((clave.to_string(), a..a + n));
            }
        }
    }
    out
}

fn minmax(s: Option<&Statistics>) -> Option<(f64, f64)> {
    match s? {
        Statistics::Double(d) => Some((*d.min_opt()?, *d.max_opt()?)),
        Statistics::Float(d) => Some((*d.min_opt()? as f64, *d.max_opt()? as f64)),
        _ => None,
    }
}

/// La caja lon/lat (grados) de la tesela: Mercator conserva el rectángulo.
pub fn caja_lonlat(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z as i32);
    let lat = |t: f64| {
        (std::f64::consts::PI * (1.0 - 2.0 * t / n))
            .sinh()
            .atan()
            .to_degrees()
    };
    (
        x as f64 / n * 360.0 - 180.0,
        lat(y as f64 + 1.0),
        (x as f64 + 1.0) / n * 360.0 - 180.0,
        lat(y as f64),
    )
}

pub fn caja_3857(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z as i32);
    let ancho = 2.0 * std::f64::consts::PI * R / n;
    let x0 = -std::f64::consts::PI * R + x as f64 * ancho;
    let y1 = std::f64::consts::PI * R - y as f64 * ancho;
    (x0, y1 - ancho, x0 + ancho, y1)
}

/// El `GeomType` de MVT (spec 2.1, 4.3.4).
const PUNTO: u8 = 1;
const LINEA: u8 = 2;
const POLIGONO: u8 = 3;

struct Elemento {
    hilbert: u64,
    desempate: u64,
    tipo: u8,
    id: Option<u64>,
    /// Punto: una parte con todos los puntos. Línea: un trozo por parte. Polígono: los
    /// anillos (exterior y sus agujeros, uno tras otro).
    partes: Vec<Vec<(i64, i64)>>,
}

/// Un polígono: su exterior y sus agujeros (en coordenadas de tesela, o de origen con
/// [`geometrias`] y la identidad).
pub type Poligono = Vec<Vec<(f64, f64)>>;

/// Lo que una fila pinta, ya en coordenadas de tesela: a lo sumo uno de cada tipo (una
/// colección se descompone, y los «multi» se funden en el suyo).
#[derive(Debug, PartialEq)]
pub enum Geometria {
    Puntos(Vec<(f64, f64)>),
    Lineas(Vec<Vec<(f64, f64)>>),
    Poligonos(Vec<Poligono>),
}

/// Una geometría ya en la tesela: `(tipo, partes, centro)` (el centro, para Hilbert).
type Pieza = (u8, Vec<Vec<(i64, i64)>>, (i64, i64));

/// Una geometría a la tesela, o `None` si no cae en ella o no queda nada (lo que se
/// aplasta suma en `diminutos`).
fn a_la_tesela(g: Geometria, diminutos: &mut usize) -> Option<Pieza> {
    match g {
        Geometria::Puntos(ps) => {
            // dentro del marco con su margen; los repetidos de un mismo multipunto, uno
            let mut v: Vec<(i64, i64)> = Vec::with_capacity(ps.len());
            for q in ps.into_iter().filter(|q| dentro(*q)) {
                let p = (q.0.round() as i64, q.1.round() as i64);
                if !v.contains(&p) {
                    v.push(p);
                }
            }
            if v.is_empty() {
                return None;
            }
            let c = centro(&v);
            Some((PUNTO, vec![v], c))
        }
        Geometria::Lineas(ls) => {
            let mut partes = Vec::new();
            for l in ls {
                if !toca_el_marco(&l) {
                    continue;
                }
                let s = rdp_abierto(&l, 1.0);
                let trozos = if s.iter().all(|q| dentro(*q)) {
                    vec![s]
                } else {
                    recortar_linea(&s)
                };
                for t in trozos {
                    match a_rejilla_linea(&t) {
                        Some(r) => partes.push(r),
                        None => *diminutos += 1,
                    }
                }
            }
            if partes.is_empty() {
                return None;
            }
            let c = centro(&partes.concat());
            Some((LINEA, partes, c))
        }
        Geometria::Poligonos(mp) => {
            let toca = mp.iter().any(|pg| pg[0].iter().any(|q| dentro(*q)))
                || mp.iter().any(|pg| cubre(&pg[0], EXTENT / 2.0));
            if !toca {
                return None;
            }
            // simplificar por debajo del píxel y recortar con margen (lo que no cabe entero)
            let mut anillos: Vec<Vec<(i64, i64)>> = Vec::new();
            for pg in &mp {
                let recortado: Vec<Vec<(f64, f64)>> = pg
                    .iter()
                    .map(|r| {
                        let s = rdp(r, 1.0);
                        if s.iter().all(|q| dentro(*q)) {
                            s
                        } else {
                            recortar(&s)
                        }
                    })
                    .collect();
                let Some(ext) = a_rejilla(&recortado[0], true) else {
                    *diminutos += 1;
                    continue;
                };
                anillos.push(ext);
                anillos.extend(recortado[1..].iter().filter_map(|h| a_rejilla(h, false)));
            }
            if anillos.is_empty() {
                return None;
            }
            let c = centro(&anillos[0]);
            Some((POLIGONO, anillos, c))
        }
    }
}

/// ¿Toca la caja de la línea el marco con su margen? (antes de simplificar nada)
fn toca_el_marco(l: &[(f64, f64)]) -> bool {
    let (lo, hi) = (-BUFFER, EXTENT + BUFFER);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for q in l {
        x0 = x0.min(q.0);
        y0 = y0.min(q.1);
        x1 = x1.max(q.0);
        y1 = y1.max(q.1);
    }
    x1 >= lo && x0 <= hi && y1 >= lo && y0 <= hi
}

/// Douglas-Peucker de una línea abierta: conserva sus dos extremos.
fn rdp_abierto(l: &[(f64, f64)], eps: f64) -> Vec<(f64, f64)> {
    if l.len() <= 2 {
        return l.to_vec();
    }
    let mut guardar = vec![false; l.len()];
    guardar[0] = true;
    guardar[l.len() - 1] = true;
    let mut pila = vec![(0usize, l.len() - 1)];
    while let Some((a, b)) = pila.pop() {
        let (mut m, mut dm) = (0usize, 0.0f64);
        for i in a + 1..b {
            let d = d_segmento(l[i], l[a], l[b]);
            if d > dm {
                (m, dm) = (i, d);
            }
        }
        if dm > eps * eps {
            guardar[m] = true;
            pila.push((a, m));
            pila.push((m, b));
        }
    }
    l.iter()
        .zip(guardar)
        .filter(|(_, g)| *g)
        .map(|(q, _)| *q)
        .collect()
}

/// Liang–Barsky de un segmento contra el marco con su margen: lo que queda dentro.
fn segmento_dentro(a: (f64, f64), b: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    let (lo, hi) = (-BUFFER, EXTENT + BUFFER);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-dx, a.0 - lo),
        (dx, hi - a.0),
        (-dy, a.1 - lo),
        (dy, hi - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return None;
            }
            t0 = t0.max(r);
        } else {
            if r < t0 {
                return None;
            }
            t1 = t1.min(r);
        }
    }
    let en = |t: f64| (a.0 + t * dx, a.1 + t * dy);
    Some((en(t0), en(t1)))
}

/// La línea recortada al marco con su margen: un trozo por cada vez que entra.
fn recortar_linea(l: &[(f64, f64)]) -> Vec<Vec<(f64, f64)>> {
    let mut trozos = Vec::new();
    let mut actual: Vec<(f64, f64)> = Vec::new();
    for w in l.windows(2) {
        match segmento_dentro(w[0], w[1]) {
            Some((p, q)) => {
                // sigue el trozo si el segmento empieza donde acabó el anterior
                if actual.last() != Some(&p) {
                    if actual.len() >= 2 {
                        trozos.push(std::mem::take(&mut actual));
                    }
                    actual.clear();
                    actual.push(p);
                }
                actual.push(q);
            }
            None => {
                if actual.len() >= 2 {
                    trozos.push(std::mem::take(&mut actual));
                }
                actual.clear();
            }
        }
    }
    if actual.len() >= 2 {
        trozos.push(actual);
    }
    trozos
}

/// La línea en enteros sin vértices repetidos; `None` si se queda en un punto.
fn a_rejilla_linea(l: &[(f64, f64)]) -> Option<Vec<(i64, i64)>> {
    let mut v: Vec<(i64, i64)> = Vec::with_capacity(l.len());
    for c in l {
        let p = (c.0.round() as i64, c.1.round() as i64);
        if v.last() != Some(&p) {
            v.push(p);
        }
    }
    (v.len() >= 2).then_some(v)
}

fn dentro(q: (f64, f64)) -> bool {
    q.0 >= -BUFFER && q.0 <= EXTENT + BUFFER && q.1 >= -BUFFER && q.1 <= EXTENT + BUFFER
}

/// Douglas-Peucker de un anillo cerrado con tolerancia `eps` (en unidades de tesela: 1 u
/// es por debajo del píxel). Conserva el primero y el último (el cierre).
fn rdp(r: &[(f64, f64)], eps: f64) -> Vec<(f64, f64)> {
    if r.len() <= 4 {
        return r.to_vec();
    }
    let mut guardar = vec![false; r.len()];
    guardar[0] = true;
    guardar[r.len() - 1] = true;
    // el anillo empieza y acaba en el mismo punto: se parte por el más lejano a él
    let lejos = (1..r.len() - 1)
        .max_by(|&a, &b| d2(r[a], r[0]).total_cmp(&d2(r[b], r[0])))
        .unwrap_or(1);
    guardar[lejos] = true;
    let mut pila = vec![(0usize, lejos), (lejos, r.len() - 1)];
    while let Some((a, b)) = pila.pop() {
        let (mut m, mut dm) = (0usize, 0.0f64);
        for i in a + 1..b {
            let d = d_segmento(r[i], r[a], r[b]);
            if d > dm {
                (m, dm) = (i, d);
            }
        }
        if dm > eps * eps {
            guardar[m] = true;
            pila.push((a, m));
            pila.push((m, b));
        }
    }
    r.iter()
        .zip(guardar)
        .filter(|(_, g)| *g)
        .map(|(q, _)| *q)
        .collect()
}

fn d2(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)
}

/// La distancia al cuadrado de `q` al segmento `a`–`b`.
fn d_segmento(q: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx * dx + dy * dy;
    if l == 0.0 {
        return d2(q, a);
    }
    let t = (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / l).clamp(0.0, 1.0);
    d2(q, (a.0 + t * dx, a.1 + t * dy))
}

/// Sutherland-Hodgman contra el marco de la tesela con su margen: el anillo recortado,
/// cerrado (o vacío si no queda nada). Puede dejar aristas sobre el borde del margen,
/// que caen fuera de lo que se ve.
fn recortar(r: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let (lo, hi) = (-BUFFER, EXTENT + BUFFER);
    let mut v: Vec<(f64, f64)> = r.to_vec();
    if v.len() > 1 && v.first() == v.last() {
        v.pop();
    }
    // (eje, límite, ¿por debajo?): x ≥ lo, x ≤ hi, y ≥ lo, y ≤ hi
    for (eje, lim, min) in [(0, lo, true), (0, hi, false), (1, lo, true), (1, hi, false)] {
        if v.is_empty() {
            break;
        }
        let val = |q: (f64, f64)| if eje == 0 { q.0 } else { q.1 };
        let dentro = |q: (f64, f64)| if min { val(q) >= lim } else { val(q) <= lim };
        let corte = |a: (f64, f64), b: (f64, f64)| {
            let t = (lim - val(a)) / (val(b) - val(a));
            (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1))
        };
        let mut out = Vec::with_capacity(v.len() + 4);
        for i in 0..v.len() {
            let (a, b) = (v[(i + v.len() - 1) % v.len()], v[i]);
            match (dentro(a), dentro(b)) {
                (true, true) => out.push(b),
                (true, false) => out.push(corte(a, b)),
                (false, true) => {
                    out.push(corte(a, b));
                    out.push(b);
                }
                (false, false) => {}
            }
        }
        v = out;
    }
    if let Some(&p) = v.first() {
        v.push(p);
    }
    v
}

/// ¿Contiene el exterior el punto (c, c)? (una tesela entera dentro de un polígono grande)
fn cubre(r: &[(f64, f64)], c: f64) -> bool {
    let mut dentro = false;
    let mut j = r.len().saturating_sub(1);
    for i in 0..r.len() {
        let (a, b) = (r[i], r[j]);
        if (a.1 > c) != (b.1 > c) && c < (b.0 - a.0) * (c - a.1) / (b.1 - a.1) + a.0 {
            dentro = !dentro;
        }
        j = i;
    }
    dentro
}

fn area(r: &[(i64, i64)]) -> f64 {
    let mut s = 0i128;
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        s += a.0 as i128 * b.1 as i128 - b.0 as i128 * a.1 as i128;
    }
    s as f64 / 2.0
}

/// El anillo en enteros, sin repetidos ni el cierre; `None` si queda en < 4 u² o < 3
/// vértices. Exterior con área > 0 (horario con y hacia abajo), interior < 0 (spec 2.1).
fn a_rejilla(l: &[(f64, f64)], exterior: bool) -> Option<Vec<(i64, i64)>> {
    let mut v: Vec<(i64, i64)> = Vec::with_capacity(l.len());
    for c in l {
        let p = (c.0.round() as i64, c.1.round() as i64);
        if v.last() != Some(&p) {
            v.push(p);
        }
    }
    if v.len() > 1 && v.first() == v.last() {
        v.pop();
    }
    if v.len() < 3 {
        return None;
    }
    let a = area(&v);
    if a.abs() < 4.0 {
        return None;
    }
    if (a > 0.0) != exterior {
        v.reverse();
    }
    Some(v)
}

fn centro(r: &[(i64, i64)]) -> (i64, i64) {
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for &(x, y) in r {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    ((x0 + x1) / 2, (y0 + y1) / 2)
}

/// La clave Hilbert en una rejilla 2¹⁶ sobre la tesela con su margen.
fn hilbert(x: i64, y: i64) -> u64 {
    let n: i64 = 1 << 16;
    let celda = |v: i64| {
        (((v as f64 + BUFFER) / (EXTENT + 2.0 * BUFFER) * (n - 1) as f64).round() as i64)
            .clamp(0, n - 1)
    };
    let (mut x, mut y) = (celda(x), celda(y));
    let mut d: u64 = 0;
    let mut s = n / 2;
    while s > 0 {
        let rx = ((x & s) > 0) as i64;
        let ry = ((y & s) > 0) as i64;
        d += (s as u64) * (s as u64) * ((3 * rx) ^ ry) as u64;
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
}

/// FNV-1a de 64 bits: determinista, sin dependencias (el desempate y el `ETag`).
pub fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in b {
        h = (h ^ *x as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Lo que pinta una geometría WKB (ISO o EWKB, Z/M ignorados), ya en coordenadas de tesela:
/// a lo sumo un `Geometria` por tipo, en el orden punto, línea, polígono. `None` si el WKB
/// está roto o trae un tipo que no se pinta (curvas, superficies poliédricas, TIN): la fila
/// cuenta en `descartados`. Un vacío (`POINT EMPTY`, una colección sin nada) es `Some([])`.
/// Las geometrías de un WKB (ISO o EWKB), con `f` aplicada a cada vértice. `ore-piramide` la
/// llama con la identidad: agrega en las coordenadas de origen.
pub fn geometrias(b: &[u8], f: &dyn Fn(f64, f64) -> (f64, f64)) -> Option<Vec<Geometria>> {
    fn u32_(b: &[u8], i: &mut usize, le: bool) -> Option<u32> {
        let s: [u8; 4] = b.get(*i..*i + 4)?.try_into().ok()?;
        *i += 4;
        Some(if le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    }
    fn f64_(b: &[u8], i: &mut usize, le: bool) -> Option<f64> {
        let s: [u8; 8] = b.get(*i..*i + 8)?.try_into().ok()?;
        *i += 8;
        Some(if le {
            f64::from_le_bytes(s)
        } else {
            f64::from_be_bytes(s)
        })
    }
    /// `(tipo, coordenadas de más por vértice, ¿lleva SRID?)`, de ISO (1000/2000/3000) o EWKB.
    fn dims(t: u32) -> (u32, usize, bool) {
        let srid = t & 0x2000_0000 != 0;
        let extra_ew = (t & 0x8000_0000 != 0) as usize + (t & 0x4000_0000 != 0) as usize;
        let t = t & 0x0fff_ffff;
        let extra = match t / 1000 {
            1 | 2 => 1,
            3 => 2,
            _ => extra_ew,
        };
        (t % 1000, extra, srid)
    }
    /// Una secuencia de vértices; los que no son números (un `POINT EMPTY` es NaN), fuera.
    fn vertices(
        b: &[u8],
        i: &mut usize,
        le: bool,
        extra: usize,
        n: u32,
        f: &dyn Fn(f64, f64) -> (f64, f64),
    ) -> Option<Vec<(f64, f64)>> {
        let mut l = Vec::with_capacity(n.min(1 << 16) as usize);
        for _ in 0..n {
            let x = f64_(b, i, le)?;
            let y = f64_(b, i, le)?;
            *i += 8 * extra;
            if x.is_finite() && y.is_finite() {
                l.push(f(x, y));
            }
        }
        Some(l)
    }
    /// Una geometría desde su cabecera, con la colección recursiva (hasta 8 de hondo).
    fn una(
        b: &[u8],
        i: &mut usize,
        out: &mut Vec<Geometria>,
        f: &dyn Fn(f64, f64) -> (f64, f64),
        hondo: u32,
    ) -> Option<()> {
        let le = *b.get(*i)? == 1;
        *i += 1;
        let (t, extra, srid) = dims(u32_(b, i, le)?);
        if srid {
            *i += 4;
        }
        match t {
            1 => {
                let p = vertices(b, i, le, extra, 1, f)?;
                if !p.is_empty() {
                    out.push(Geometria::Puntos(p));
                }
            }
            2 => {
                let n = u32_(b, i, le)?;
                let l = vertices(b, i, le, extra, n, f)?;
                if l.len() >= 2 {
                    out.push(Geometria::Lineas(vec![l]));
                }
            }
            3 => {
                let n = u32_(b, i, le)?;
                let mut anillos = Vec::with_capacity(n.min(1 << 10) as usize);
                for _ in 0..n {
                    let k = u32_(b, i, le)?;
                    anillos.push(vertices(b, i, le, extra, k, f)?);
                }
                if anillos.first().is_some_and(|r| r.len() >= 3) {
                    out.push(Geometria::Poligonos(vec![anillos]));
                }
            }
            // multipunto, multilínea, multipolígono y colección: sus partes, cada una con su cabecera
            4..=7 if hondo < 8 => {
                let n = u32_(b, i, le)?;
                for _ in 0..n {
                    una(b, i, out, f, hondo + 1)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
    let mut partes = Vec::new();
    una(b, &mut 0, &mut partes, f, 0)?;
    // los «multi» y la colección se funden por tipo: un elemento de cada uno por fila
    let (mut ps, mut ls, mut pg) = (Vec::new(), Vec::new(), Vec::new());
    for g in partes {
        match g {
            Geometria::Puntos(v) => ps.extend(v),
            Geometria::Lineas(v) => ls.extend(v),
            Geometria::Poligonos(v) => pg.extend(v),
        }
    }
    let mut out = Vec::new();
    if !ps.is_empty() {
        out.push(Geometria::Puntos(ps));
    }
    if !ls.is_empty() {
        out.push(Geometria::Lineas(ls));
    }
    if !pg.is_empty() {
        out.push(Geometria::Poligonos(pg));
    }
    Some(out)
}

// ── MVT (protobuf a mano, spec 2.1) ─────────────────────────────────────────────────────────────
fn varint(mut v: u64, o: &mut Vec<u8>) {
    while v >= 0x80 {
        o.push((v as u8) | 0x80);
        v >>= 7;
    }
    o.push(v as u8);
}
fn clave(campo: u32, tipo: u32, o: &mut Vec<u8>) {
    varint(((campo << 3) | tipo) as u64, o);
}
fn zz(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Una capa de polígonos, en el orden dado; campos en orden fijo (D-F2-6).
fn mvt(elementos: &[Elemento], nombre: &str) -> Vec<u8> {
    let mut capa = Vec::with_capacity(elementos.len() * 64 + 64);
    clave(15, 0, &mut capa);
    varint(2, &mut capa);
    clave(1, 2, &mut capa);
    varint(nombre.len() as u64, &mut capa);
    capa.extend_from_slice(nombre.as_bytes());
    let (mut feat, mut geom) = (Vec::new(), Vec::new());
    for e in elementos {
        feat.clear();
        geom.clear();
        let (mut cx, mut cy) = (0i64, 0i64);
        let mut a = |q: (i64, i64), geom: &mut Vec<u8>| {
            varint(zz(q.0 - cx), geom);
            varint(zz(q.1 - cy), geom);
            (cx, cy) = q;
        };
        if e.tipo == PUNTO {
            // MoveTo n: todos los puntos en un solo comando
            varint(((e.partes[0].len() as u64) << 3) | 1, &mut geom);
            for q in &e.partes[0] {
                a(*q, &mut geom);
            }
        } else {
            for r in &e.partes {
                varint((1 << 3) | 1, &mut geom); // MoveTo 1
                a(r[0], &mut geom);
                varint((((r.len() - 1) as u64) << 3) | 2, &mut geom); // LineTo n−1
                for q in &r[1..] {
                    a(*q, &mut geom);
                }
                if e.tipo == POLIGONO {
                    varint((1 << 3) | 7, &mut geom); // ClosePath
                }
            }
        }
        if let Some(id) = e.id {
            clave(1, 0, &mut feat);
            varint(id, &mut feat);
        }
        clave(3, 0, &mut feat);
        varint(e.tipo as u64, &mut feat); // POINT, LINESTRING o POLYGON
        clave(4, 2, &mut feat);
        varint(geom.len() as u64, &mut feat);
        feat.extend_from_slice(&geom);
        clave(2, 2, &mut capa);
        varint(feat.len() as u64, &mut capa);
        capa.extend_from_slice(&feat);
    }
    clave(5, 0, &mut capa);
    varint(EXTENT as u64, &mut capa);
    let mut t = Vec::with_capacity(capa.len() + 8);
    clave(3, 2, &mut t);
    varint(capa.len() as u64, &mut t);
    t.extend_from_slice(&capa);
    t
}

#[cfg(test)]
mod pruebas;
