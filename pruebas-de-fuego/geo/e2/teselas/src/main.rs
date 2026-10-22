//! ORE 0072 E2·0 T1 · Un motor de teselas MVT de puntos, en Rust, sobre los ficheros que escribe el
//! escritor de ORE (Hilbert, grupos de 100 000 filas, bbox por grupo en la estadística geo de Parquet).
//! Laboratorio: lo que mide es si leer sólo los grupos que tocan la tesela, reproyectar y codificar en
//! Rust gana a DuckDB (`previo_a.py`) en tiempo por tesela, memoria y concurrencia.
//!
//! Entorno: `E2R_FICHEROS` (gs://… separados por comas), `E2R_CRS` (25830 | 3857), `E2R_SEGUNDOS`.
//! Emite «E2R|…».

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arrow::array::{Array, BinaryArray, Int64Array, LargeBinaryArray};
use futures::TryStreamExt;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt};
use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions};
use parquet::arrow::async_reader::{ParquetObjectReader, ParquetRecordBatchStreamBuilder};
use parquet::arrow::ProjectionMask;
use proj4rs::proj::Proj;
use rand::{Rng, SeedableRng};

const EXTENT: f64 = 4096.0;
const BUFFER: f64 = 64.0;
const R: f64 = 6378137.0;

fn di(que: &str, v: serde_json::Value) {
    println!("E2R|{que}|{v}");
}

struct Fichero {
    store: Arc<dyn ObjectStore>,
    path: Path,
    size: u64,
}

struct Motor {
    ficheros: Vec<Fichero>,
    /// La caché de pies de fichero (lo que un servicio guarda): ruta → metadatos.
    pies: Mutex<HashMap<String, ArrowReaderMetadata>>,
    utm: Option<(Proj, Proj)>,
}

/// La caja 3857 de la tesela z/x/y.
fn caja_3857(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z as i32);
    let ancho = 2.0 * std::f64::consts::PI * R / n;
    let x0 = -std::f64::consts::PI * R + x as f64 * ancho;
    let y1 = std::f64::consts::PI * R - y as f64 * ancho;
    (x0, y1 - ancho, x0 + ancho, y1)
}

fn merc_a_lonlat(mx: f64, my: f64) -> (f64, f64) {
    (mx / R, (2.0 * (my / R).exp().atan()) - std::f64::consts::FRAC_PI_2) // radianes
}

fn lonlat_a_merc(lon: f64, lat: f64) -> (f64, f64) {
    (lon * R, R * (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln())
}

impl Motor {
    /// La caja de la tesela en el CRS de los datos (esquinas y medios de lado reproyectados).
    fn caja_datos(&self, c: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
        let Some((ll, utm)) = &self.utm else { return c };
        let (mut a, mut b, mut cc, mut d) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for fx in [0.0, 0.5, 1.0] {
            for fy in [0.0, 0.5, 1.0] {
                let (lon, lat) = merc_a_lonlat(c.0 + (c.2 - c.0) * fx, c.1 + (c.3 - c.1) * fy);
                let mut p = (lon, lat, 0.0);
                if proj4rs::transform::transform(ll, utm, &mut p).is_ok() {
                    a = a.min(p.0);
                    b = b.min(p.1);
                    cc = cc.max(p.0);
                    d = d.max(p.1);
                }
            }
        }
        (a, b, cc, d)
    }

    fn a_3857(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        match &self.utm {
            None => Some((x, y)),
            Some((ll, utm)) => {
                let mut p = (x, y, 0.0);
                proj4rs::transform::transform(utm, ll, &mut p).ok()?;
                Some(lonlat_a_merc(p.0, p.1))
            }
        }
    }

    async fn pie(&self, f: &Fichero) -> Result<ArrowReaderMetadata, String> {
        if let Some(m) = self.pies.lock().unwrap().get(f.path.as_ref()) {
            return Ok(m.clone());
        }
        let mut r = ParquetObjectReader::new(f.store.clone(), f.path.clone()).with_file_size(f.size);
        let m = ArrowReaderMetadata::load_async(&mut r, ArrowReaderOptions::new())
            .await
            .map_err(|e| e.to_string())?;
        self.pies.lock().unwrap().insert(f.path.to_string(), m.clone());
        Ok(m)
    }

    /// Una tesela: (bytes del MVT, puntos, grupos leídos, grupos totales).
    async fn tesela(&self, z: u32, x: u32, y: u32) -> Result<(Vec<u8>, usize, usize, usize), String> {
        let c = caja_3857(z, x, y);
        let cd = self.caja_datos(c);
        let mut feats: Vec<(i64, i64, i64)> = Vec::new();
        let (mut leidos, mut total) = (0, 0);
        for f in &self.ficheros {
            let meta = self.pie(f).await?;
            let esquema = meta.schema().clone();
            let ig = esquema.index_of("geom").map_err(|e| e.to_string())?;
            let ii = esquema.index_of("id").map_err(|e| e.to_string())?;
            // la columna hoja de `geom` en el esquema de Parquet
            let hoja = meta
                .parquet_schema()
                .columns()
                .iter()
                .position(|c| c.path().parts()[0] == "geom")
                .ok_or("sin columna geom")?;
            let grupos: Vec<usize> = meta
                .metadata()
                .row_groups()
                .iter()
                .enumerate()
                .filter(|(_, rg)| {
                    match rg.column(hoja).geo_statistics().and_then(|g| g.bounding_box()) {
                        Some(b) => !(b.get_xmax() < cd.0 || b.get_xmin() > cd.2 || b.get_ymax() < cd.1 || b.get_ymin() > cd.3),
                        None => true,
                    }
                })
                .map(|(i, _)| i)
                .collect();
            total += meta.metadata().num_row_groups();
            leidos += grupos.len();
            if grupos.is_empty() {
                continue;
            }
            let r = ParquetObjectReader::new(f.store.clone(), f.path.clone()).with_file_size(f.size);
            let mask = ProjectionMask::roots(meta.parquet_schema(), [ig, ii]);
            let mut flujo = ParquetRecordBatchStreamBuilder::new_with_metadata(r, meta.clone())
                .with_row_groups(grupos)
                .with_projection(mask)
                .with_batch_size(65536)
                .build()
                .map_err(|e| e.to_string())?;
            while let Some(lote) = flujo.try_next().await.map_err(|e| e.to_string())? {
                let g = lote.column_by_name("geom").ok_or("geom")?;
                let ids = lote.column_by_name("id").ok_or("id")?.as_any().downcast_ref::<Int64Array>().ok_or("id no es Int64")?;
                let wkb_de = |i: usize| -> Option<&[u8]> {
                    if let Some(a) = g.as_any().downcast_ref::<BinaryArray>() {
                        (!a.is_null(i)).then(|| a.value(i))
                    } else if let Some(a) = g.as_any().downcast_ref::<LargeBinaryArray>() {
                        (!a.is_null(i)).then(|| a.value(i))
                    } else {
                        None
                    }
                };
                for i in 0..lote.num_rows() {
                    let Some((px, py)) = wkb_de(i).and_then(punto) else { continue };
                    if px < cd.0 || px > cd.2 || py < cd.1 || py > cd.3 {
                        continue;
                    }
                    let Some((mx, my)) = self.a_3857(px, py) else { continue };
                    let tx = (mx - c.0) / (c.2 - c.0) * EXTENT;
                    let ty = (c.3 - my) / (c.3 - c.1) * EXTENT;
                    if tx < -BUFFER || tx > EXTENT + BUFFER || ty < -BUFFER || ty > EXTENT + BUFFER {
                        continue;
                    }
                    feats.push((ids.value(i), tx.round() as i64, ty.round() as i64));
                }
            }
        }
        let n = feats.len();
        Ok((mvt_puntos(&feats), n, leidos, total))
    }
}

/// Un punto WKB 2D (ISO o EWKB sin SRID), en cualquier orden de bytes.
fn punto(b: &[u8]) -> Option<(f64, f64)> {
    if b.len() < 21 {
        return None;
    }
    let le = b[0] == 1;
    let rd32 = |s: &[u8]| if le { u32::from_le_bytes(s.try_into().unwrap()) } else { u32::from_be_bytes(s.try_into().unwrap()) };
    let rd64 = |s: &[u8]| if le { f64::from_le_bytes(s.try_into().unwrap()) } else { f64::from_be_bytes(s.try_into().unwrap()) };
    let t = rd32(&b[1..5]);
    if t & 0xffff != 1 && t % 1000 != 1 {
        return None;
    }
    Some((rd64(&b[5..13]), rd64(&b[13..21])))
}

// ── MVT a mano (protobuf): una capa «capa», un punto por feature con su id ──
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
fn mvt_puntos(f: &[(i64, i64, i64)]) -> Vec<u8> {
    let mut capa = Vec::with_capacity(f.len() * 12 + 32);
    clave(15, 0, &mut capa);
    varint(2, &mut capa);
    clave(1, 2, &mut capa);
    varint(4, &mut capa);
    capa.extend_from_slice(b"capa");
    let mut feat = Vec::with_capacity(24);
    let mut geom = Vec::with_capacity(12);
    for &(id, x, y) in f {
        feat.clear();
        geom.clear();
        varint(9, &mut geom); // MoveTo, 1
        varint(zz(x), &mut geom);
        varint(zz(y), &mut geom);
        clave(1, 0, &mut feat);
        varint(id as u64, &mut feat);
        clave(3, 0, &mut feat);
        varint(1, &mut feat); // POINT
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

fn tesela_de(lon: f64, lat: f64, z: u32) -> (u32, u32) {
    let n = 2f64.powi(z as i32);
    let x = ((lon + 180.0) / 360.0 * n) as u32;
    let y = ((1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n) as u32;
    (x, y)
}

fn percentil(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() {
        return 0.0;
    }
    v[((v.len() as f64 * p).ceil() as usize).clamp(1, v.len()) - 1]
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l.split_whitespace().nth(1).unwrap_or("0").parse::<f64>().unwrap_or(0.0)))
        .unwrap_or(0.0)
        / 1024.0
}

#[tokio::main]
async fn main() {
    let t_arranque = Instant::now();
    let rutas: Vec<String> = std::env::var("E2R_FICHEROS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect();
    let crs = std::env::var("E2R_CRS").unwrap_or_else(|_| "25830".into());
    let segundos: u64 = std::env::var("E2R_SEGUNDOS").ok().and_then(|s| s.parse().ok()).unwrap_or(40);
    let mut almacenes: HashMap<String, Arc<dyn ObjectStore>> = HashMap::new();
    let mut ficheros = Vec::new();
    for r in &rutas {
        let sin = r.trim_start_matches("gs://");
        let (cubo, obj) = sin.split_once('/').expect("gs://cubo/objeto");
        let store = almacenes
            .entry(cubo.to_string())
            .or_insert_with(|| Arc::new(GoogleCloudStorageBuilder::from_env().with_bucket_name(cubo).build().expect("gcs")))
            .clone();
        let path = Path::from(obj);
        let size = store.head(&path).await.expect("head").size;
        ficheros.push(Fichero { store, path, size });
    }
    let utm = (crs == "25830").then(|| {
        (
            Proj::from_proj_string("+proj=longlat +ellps=GRS80 +no_defs").unwrap(),
            Proj::from_proj_string("+proj=utm +zone=30 +ellps=GRS80 +units=m +no_defs").unwrap(),
        )
    });
    let motor = Arc::new(Motor { ficheros, pies: Mutex::new(HashMap::new()), utm });
    di("inicio", serde_json::json!({"ficheros": rutas.len(), "crs": crs, "arranque_ms": t_arranque.elapsed().as_millis(), "rss_mb": rss_mb()}));

    let madrid = (-3.7038, 40.4168);
    for z in [4u32, 6, 8, 10, 12, 14] {
        let (x, y) = tesela_de(madrid.0, madrid.1, z);
        let mut medida = vec![];
        for _ in 0..2 {
            let t0 = Instant::now();
            match motor.tesela(z, x, y).await {
                Ok((b, n, l, t)) => medida.push((t0.elapsed().as_secs_f64() * 1000.0, b.len(), n, l, t)),
                Err(e) => {
                    di("tesela", serde_json::json!({"z": z, "error": e}));
                    break;
                }
            }
        }
        if medida.len() == 2 {
            di("tesela", serde_json::json!({"z": z, "frio_ms": (medida[0].0 * 10.0).round() / 10.0, "ms": (medida[1].0 * 10.0).round() / 10.0,
                "filas": medida[1].2, "kb": (medida[1].1 as f64 / 102.4).round() / 10.0, "grupos": medida[1].3, "de": medida[1].4, "rss_mb": rss_mb()}));
        }
    }

    for lectores in [1usize, 8, 32, 64] {
        let fin = Instant::now() + std::time::Duration::from_secs(segundos);
        let mut tareas = vec![];
        for k in 0..lectores {
            let m = motor.clone();
            tareas.push(tokio::spawn(async move {
                let mut rnd = rand::rngs::StdRng::seed_from_u64(k as u64);
                let mut t = vec![];
                while Instant::now() < fin {
                    let z = rnd.gen_range(10..=14);
                    let lon = madrid.0 + rnd.gen_range(-0.6..0.6);
                    let lat = madrid.1 + rnd.gen_range(-0.4..0.4);
                    let (x, y) = tesela_de(lon, lat, z);
                    let t0 = Instant::now();
                    if m.tesela(z, x, y).await.is_ok() {
                        t.push(t0.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                t
            }));
        }
        let mut todos = vec![];
        for t in tareas {
            todos.extend(t.await.unwrap());
        }
        let n = todos.len();
        let p50 = percentil(&mut todos, 0.5);
        let p95 = percentil(&mut todos, 0.95);
        di("concurrencia", serde_json::json!({"lectores": lectores, "teselas": n, "por_s": (n as f64 / segundos as f64 * 10.0).round() / 10.0,
            "p50_ms": p50.round(), "p95_ms": p95.round(), "rss_mb": rss_mb()}));
    }
    di("fin", serde_json::json!({"rss_mb": rss_mb()}));
}
