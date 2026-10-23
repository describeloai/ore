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
//! tesela no se leen (vale para `ore-store` y Spark); los pies, en caché.
//!
//! ⚠️ F2·1 es el esqueleto: lo que F2·0a dejó para F2·3 —filtrar por fila antes de
//! descodificar, reparar tras la rejilla, el suelo calculado, la reproyección
//! general— no está. Hoy, CRS `EPSG:4326`/`OGC:CRS84` y `EPSG:3857`.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arrow_array::{Array, BinaryArray, Int32Array, Int64Array, LargeBinaryArray, RecordBatch};
use ore_core::json::Json;
use ore_store::almacen::Almacen;
use ore_store::lago::Lago;
use ore_store::muestra::{FicheroVivo, Rangos};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::file::statistics::Statistics;

/// La versión del protocolo (C3): el motor acepta ésta y la anterior (N y N−1).
pub const PROTOCOLO: u32 = 1;
pub const EXTENT: f64 = 4096.0;
pub const BUFFER: f64 = 64.0;
const R: f64 = 6_378_137.0;
/// Cuántos pies y listas de ficheros se guardan (por clave inmutable).
const CACHE: usize = 4096;

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
}

/// Lo que la tesela costó (`x-ore-cuenta`).
#[derive(Default, Debug, Clone)]
pub struct Cuenta {
    pub grupos: usize,
    pub grupos_leidos: usize,
    pub filas: usize,
    pub elementos: usize,
    pub diminutos: usize,
    pub listar_ms: u64,
    pub leer_ms: u64,
    pub geom_ms: u64,
    pub codificar_ms: u64,
}

impl Cuenta {
    pub fn json(&self) -> Json {
        let n = |v: usize| Json::Int(v as i64);
        let m = |v: u64| Json::Int(v as i64);
        Json::obj([
            ("grupos", n(self.grupos)),
            ("grupos_leidos", n(self.grupos_leidos)),
            ("filas", n(self.filas)),
            ("elementos", n(self.elementos)),
            ("diminutos", n(self.diminutos)),
            ("listar_ms", m(self.listar_ms)),
            ("leer_ms", m(self.leer_ms)),
            ("geom_ms", m(self.geom_ms)),
            ("codificar_ms", m(self.codificar_ms)),
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
        ore_core::parse::parse(texto).map_err(|e| mal(format!("el cuerpo no es JSON: {e:?}")))?;
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
    let cs = |k: &str| c.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let credencial = if metadata_location.starts_with("gs://") {
        Credencial::Gcs {
            token: cs("gcs.oauth2.token")
                .ok_or_else(|| mal("falta `credencial.gcs.oauth2.token`".into()))?,
        }
    } else if metadata_location.starts_with("s3://") {
        Credencial::S3 {
            endpoint: cs("s3.endpoint")
                .ok_or_else(|| mal("falta `credencial.s3.endpoint`".into()))?,
            region: cs("s3.region").unwrap_or_else(|| "auto".into()),
            clave: cs("s3.access-key-id")
                .ok_or_else(|| mal("falta `credencial.s3.access-key-id`".into()))?,
            secreto: cs("s3.secret-access-key")
                .ok_or_else(|| mal("falta `credencial.s3.secret-access-key`".into()))?,
        }
    } else {
        return Err(mal(format!("`{metadata_location}`: ni gs:// ni s3://")));
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
    })
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

pub struct Motor {
    pies: Cache<ArrowReaderMetadata>,
    listas: Cache<Arc<Vec<(String, u64)>>>,
    cuentas: Cache<(Arc<dyn Almacen>, Arc<Lago>)>,
}

impl Default for Motor {
    fn default() -> Self {
        Motor::nuevo()
    }
}

impl Motor {
    pub fn nuevo() -> Motor {
        Motor {
            pies: Cache::new(),
            listas: Cache::new(),
            cuentas: Cache::new(),
        }
    }

    /// La cuenta del almacén para esta credencial y este bucket (una por credencial: un
    /// cliente con sus conexiones vivas, no uno por tesela).
    fn cuenta(&self, p: &Peticion) -> Result<(Arc<dyn Almacen>, Arc<Lago>), Fallo> {
        let resto = p.metadata_location.split_once("://").map_or("", |x| x.1);
        let bucket = resto.split('/').next().unwrap_or("").to_string();
        let k = match &p.credencial {
            Credencial::Gcs { token } => format!("gs|{bucket}|{token}"),
            Credencial::S3 {
                endpoint, clave, ..
            } => format!("s3|{endpoint}|{bucket}|{clave}"),
        };
        let cache = &self.cuentas;
        if let Some(c) = cache.get(&k) {
            return Ok(c);
        }
        let cuenta: Arc<dyn Almacen> = match &p.credencial {
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

    /// Los ficheros vivos de la instantánea (clave, bytes): el `metadata.json` es inmutable.
    fn ficheros(&self, p: &Peticion, lago: &Lago) -> Result<Arc<Vec<(String, u64)>>, Fallo> {
        let cache = &self.listas;
        if let Some(v) = cache.get(&p.metadata_location) {
            return Ok(v);
        }
        let tabla = lago
            .abrir(&p.metadata_location, "teselas")
            .map_err(Fallo::Interno)?;
        let (_, vivos) =
            ore_store::muestra::ficheros_vivos(lago, &tabla).map_err(Fallo::Invalida)?;
        let v = Arc::new(
            vivos
                .into_iter()
                .map(|FicheroVivo { clave, bytes, .. }| (clave, bytes))
                .collect::<Vec<_>>(),
        );
        cache.put(p.metadata_location.clone(), v.clone());
        Ok(v)
    }

    /// **Una tesela**: `Ok(None)` si está vacía.
    pub fn tesela(&self, p: &Peticion) -> Result<(Option<Vec<u8>>, Cuenta), Fallo> {
        let t0 = Instant::now();
        let mut cu = Cuenta::default();
        let (cuenta, lago) = self.cuenta(p)?;
        self.tesela_en(p, cuenta, &lago, t0, &mut cu)
    }

    /// Lo mismo con el almacén ya dado (las pruebas, sin red).
    pub fn tesela_con(
        &self,
        p: &Peticion,
        cuenta: Arc<dyn Almacen>,
    ) -> Result<(Option<Vec<u8>>, Cuenta), Fallo> {
        let lago = Lago::nuevo(cuenta.clone());
        let mut cu = Cuenta::default();
        self.tesela_en(p, cuenta, &lago, Instant::now(), &mut cu)
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
        let ficheros = self.ficheros(p, lago)?;
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
        let mut geoms: Vec<(Option<u64>, Vec<u8>)> = Vec::new();
        let cobertura = format!("{}_bbox", p.geometria);
        for (clave, bytes) in ficheros.iter() {
            plazo("leer")?;
            let leidos = Arc::new(AtomicU64::new(0));
            let rangos = Rangos::abrir(cuenta.clone(), clave.clone(), *bytes, leidos)
                .map_err(Fallo::Interno)?;
            let pies = &self.pies;
            let meta = match pies.get(clave) {
                Some(m) => m,
                None => {
                    let m = ArrowReaderMetadata::load(&rangos, ArrowReaderOptions::new())
                        .map_err(|e| Fallo::Interno(e.to_string()))?;
                    pies.put(clave.clone(), m.clone());
                    m
                }
            };
            let esquema = meta.schema().clone();
            let ig = esquema
                .index_of(&p.geometria)
                .map_err(|_| Fallo::Invalida(format!("no hay columna `{}`", p.geometria)))?;
            let ii = match &p.id {
                Some(id) => Some(
                    esquema
                        .index_of(id)
                        .map_err(|_| Fallo::Invalida(format!("no hay columna `{id}`")))?,
                ),
                None => None,
            };
            let cols = meta.parquet_schema().columns();
            let hoja = |campo: &str| {
                cols.iter().position(|c| {
                    let partes = c.path().parts();
                    partes.len() == 2 && partes[0] == cobertura && partes[1] == campo
                })
            };
            let (h0, h1, h2, h3) = (hoja("xmin"), hoja("ymin"), hoja("xmax"), hoja("ymax"));
            let mut grupos = vec![];
            for (i, rg) in meta.metadata().row_groups().iter().enumerate() {
                cu.grupos += 1;
                let toca = match (h0, h1, h2, h3) {
                    (Some(a), Some(b), Some(c), Some(d)) => match (
                        minmax(rg.column(a).statistics()),
                        minmax(rg.column(b).statistics()),
                        minmax(rg.column(c).statistics()),
                        minmax(rg.column(d).statistics()),
                    ) {
                        // xmin ≤ caja.x1, ymin ≤ caja.y1, xmax ≥ caja.x0, ymax ≥ caja.y0
                        (Some(xa), Some(ya), Some(xb), Some(yb)) => {
                            xa.0 <= cd.2 && ya.0 <= cd.3 && xb.1 >= cd.0 && yb.1 >= cd.1
                        }
                        _ => true,
                    },
                    // sin cobertura (un fichero de antes de F1): se lee el grupo
                    _ => true,
                };
                if toca {
                    grupos.push(i);
                }
            }
            cu.grupos_leidos += grupos.len();
            if grupos.is_empty() {
                continue;
            }
            let raices: Vec<usize> = std::iter::once(ig).chain(ii).collect();
            let mascara = ProjectionMask::roots(meta.parquet_schema(), raices);
            let lector = ParquetRecordBatchReaderBuilder::new_with_metadata(rangos, meta.clone())
                .with_row_groups(grupos)
                .with_projection(mascara)
                .with_batch_size(65_536)
                .build()
                .map_err(|e| Fallo::Interno(e.to_string()))?;
            for lote in lector {
                plazo("leer")?;
                let lote = lote.map_err(|e| Fallo::Interno(e.to_string()))?;
                recoger(&lote, &p.geometria, p.id.as_deref(), &mut geoms)?;
            }
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
            let Some(mp) = poligonos(wkb, &a_tesela) else {
                continue;
            };
            let toca = mp.iter().any(|pg| pg[0].iter().any(|q| dentro(*q)))
                || mp.iter().any(|pg| cubre(&pg[0], EXTENT / 2.0));
            if !toca {
                continue;
            }
            // simplificar por debajo del píxel y recortar con margen (lo que no cabe entero)
            let recortada: Vec<Vec<Vec<(f64, f64)>>> = mp
                .iter()
                .map(|pg| {
                    pg.iter()
                        .map(|r| {
                            let s = rdp(r, 1.0);
                            if s.iter().all(|q| dentro(*q)) {
                                s
                            } else {
                                recortar(&s)
                            }
                        })
                        .collect()
                })
                .collect();
            let mut anillos: Vec<Vec<(i64, i64)>> = Vec::new();
            for pg in &recortada {
                let Some(ext) = a_rejilla(&pg[0], true) else {
                    cu.diminutos += 1;
                    continue;
                };
                anillos.push(ext);
                anillos.extend(pg[1..].iter().filter_map(|h| a_rejilla(h, false)));
            }
            if anillos.is_empty() {
                continue;
            }
            let (cx, cy) = centro(&anillos[0]);
            // desempate estable sin id: la huella de la geometría (D-F2-6)
            let desempate = id.unwrap_or_else(|| fnv(wkb));
            salida.push(Elemento {
                hilbert: hilbert(cx, cy),
                desempate,
                id: *id,
                anillos,
            });
        }
        salida.sort_by_key(|e| (e.hilbert, e.desempate));
        cu.elementos = salida.len();
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
fn recoger(
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

struct Elemento {
    hilbert: u64,
    desempate: u64,
    id: Option<u64>,
    anillos: Vec<Vec<(i64, i64)>>,
}

/// Un polígono: su exterior y sus agujeros, en coordenadas de tesela.
type Poligono = Vec<Vec<(f64, f64)>>;

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

/// Polígono o multipolígono WKB (ISO o EWKB, Z/M ignorados), ya en coordenadas de tesela.
/// Lo que no es superficie (puntos, líneas) no se pinta en F2·1.
fn poligonos(b: &[u8], f: &dyn Fn(f64, f64) -> (f64, f64)) -> Option<Vec<Poligono>> {
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
    fn poligono(
        b: &[u8],
        i: &mut usize,
        le: bool,
        extra: usize,
        f: &dyn Fn(f64, f64) -> (f64, f64),
    ) -> Option<Poligono> {
        let n = u32_(b, i, le)?;
        let mut anillos = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let k = u32_(b, i, le)?;
            let mut l = Vec::with_capacity(k as usize);
            for _ in 0..k {
                let x = f64_(b, i, le)?;
                let y = f64_(b, i, le)?;
                *i += 8 * extra;
                l.push(f(x, y));
            }
            anillos.push(l);
        }
        (!anillos.is_empty()).then_some(anillos)
    }
    let le = *b.first()? == 1;
    let mut i = 1usize;
    let (t, extra, srid) = dims(u32_(b, &mut i, le)?);
    if srid {
        i += 4;
    }
    let mut out = vec![];
    match t {
        3 => out.push(poligono(b, &mut i, le, extra, f)?),
        6 => {
            let n = u32_(b, &mut i, le)?;
            for _ in 0..n {
                let le2 = *b.get(i)? == 1;
                i += 1;
                let (_, extra2, srid2) = dims(u32_(b, &mut i, le2)?);
                if srid2 {
                    i += 4;
                }
                out.push(poligono(b, &mut i, le2, extra2, f)?);
            }
        }
        _ => return None,
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
        for r in &e.anillos {
            varint((1 << 3) | 1, &mut geom); // MoveTo 1
            varint(zz(r[0].0 - cx), &mut geom);
            varint(zz(r[0].1 - cy), &mut geom);
            (cx, cy) = r[0];
            varint((((r.len() - 1) as u64) << 3) | 2, &mut geom); // LineTo n−1
            for q in &r[1..] {
                varint(zz(q.0 - cx), &mut geom);
                varint(zz(q.1 - cy), &mut geom);
                (cx, cy) = *q;
            }
            varint((1 << 3) | 7, &mut geom); // ClosePath
        }
        if let Some(id) = e.id {
            clave(1, 0, &mut feat);
            varint(id, &mut feat);
        }
        clave(3, 0, &mut feat);
        varint(3, &mut feat); // POLYGON
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
