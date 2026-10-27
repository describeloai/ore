//! ORE 0072 F2·0 · El motor de teselas de laboratorio (desde `e2/teselas`, T1). Lo que mide:
//!
//!   M1 (`medir`)   polígonos de verdad (los edificios de F1·3) con la cadena de consenso: a la tesela
//!                  (4326 → 3857 → 0..4096), simplificar por debajo del píxel, recortar con margen, a la
//!                  rejilla entera, sin repetidos, fuera los anillos < 4 u², orientar; orden Hilbert + id;
//!                  MVT con propiedades. Poda por las estadísticas de `geom_bbox` (y, aparte, por las geo de
//!                  Parquet). Por tesela: ms por fase, KB (y gzip), elementos, descartes y validez.
//!   M2 (`servir`, `puerta`, `carga`)  el salto: el motor por HTTP, una puerta que abre una conexión
//!                  nueva por tesela como `ore-serve` (`preview.rs` `al_motor`: `connection: close`) y una
//!                  carga de N lectores contra cualquiera de los dos.
//!
//! Entorno: `F2_FICHEROS` (gs://… por comas), `F2_SIMPLIFICA` (rdp | vw | no), `F2_SEGUNDOS`.
//! Emite «F2M|…».

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arrow::array::{Array, BinaryArray, Int64Array, LargeBinaryArray};
use futures::TryStreamExt;
use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon, Rect, Simplify, SimplifyVwPreserve, Validation};
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt};
use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions};
use parquet::arrow::async_reader::{ParquetObjectReader, ParquetRecordBatchStreamBuilder};
use parquet::arrow::ProjectionMask;
use parquet::file::statistics::Statistics;
use rand::{Rng, SeedableRng};

const EXTENT: f64 = 4096.0;
const BUFFER: f64 = 64.0;
const R: f64 = 6378137.0;

fn di(que: &str, v: serde_json::Value) {
    println!("F2M|{que}|{v}");
}

struct Fichero {
    store: Arc<dyn ObjectStore>,
    path: Path,
    size: u64,
}

struct Motor {
    ficheros: Vec<Fichero>,
    pies: Mutex<HashMap<String, ArrowReaderMetadata>>,
    simplifica: String,
}

/// Lo que cuenta una tesela.
#[derive(Default, Debug, Clone)]
struct Medida {
    leer_ms: f64,
    geom_ms: f64,
    cod_ms: f64,
    bytes: usize,
    elementos: usize,
    filas: usize,
    fuera: usize,
    diminutos: usize,
    invalidos: usize,
    grupos_bbox: usize,
    grupos_geo: usize,
    grupos: usize,
}

/// La caja lon/lat (grados) de la tesela z/x/y: Mercator conserva el rectángulo.
fn caja_lonlat(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z as i32);
    let lat = |t: f64| (std::f64::consts::PI * (1.0 - 2.0 * t / n)).sinh().atan().to_degrees();
    (x as f64 / n * 360.0 - 180.0, lat(y as f64 + 1.0), (x as f64 + 1.0) / n * 360.0 - 180.0, lat(y as f64))
}

fn caja_3857(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z as i32);
    let ancho = 2.0 * std::f64::consts::PI * R / n;
    let x0 = -std::f64::consts::PI * R + x as f64 * ancho;
    let y1 = std::f64::consts::PI * R - y as f64 * ancho;
    (x0, y1 - ancho, x0 + ancho, y1)
}

fn stat_f64(s: Option<&Statistics>) -> Option<(f64, f64)> {
    match s? {
        Statistics::Double(d) => Some((*d.min_opt()?, *d.max_opt()?)),
        _ => None,
    }
}

impl Motor {
    async fn pie(&self, f: &Fichero) -> Result<ArrowReaderMetadata, String> {
        if let Some(m) = self.pies.lock().unwrap().get(f.path.as_ref()) {
            return Ok(m.clone());
        }
        let mut r = ParquetObjectReader::new(f.store.clone(), f.path.clone()).with_file_size(f.size);
        let m = ArrowReaderMetadata::load_async(&mut r, ArrowReaderOptions::new()).await.map_err(|e| e.to_string())?;
        self.pies.lock().unwrap().insert(f.path.to_string(), m.clone());
        Ok(m)
    }

    async fn tesela(&self, z: u32, x: u32, y: u32) -> Result<(Vec<u8>, Medida), String> {
        let mut m = Medida::default();
        let ll = caja_lonlat(z, x, y);
        let c = caja_3857(z, x, y);
        // el margen en grados (el buffer de la tesela), para no perder lo que entra por el borde
        let mx = (ll.2 - ll.0) * BUFFER / EXTENT;
        let my = (ll.3 - ll.1) * BUFFER / EXTENT;
        let cd = (ll.0 - mx, ll.1 - my, ll.2 + mx, ll.3 + my);
        let t0 = Instant::now();
        let mut geoms: Vec<(i64, Vec<u8>)> = Vec::new();
        for f in &self.ficheros {
            let meta = self.pie(f).await?;
            let esquema = meta.schema().clone();
            let ig = esquema.index_of("geom").map_err(|e| e.to_string())?;
            let ii = esquema.index_of("id").map_err(|e| e.to_string())?;
            let cols = meta.parquet_schema().columns();
            let hoja = |a: &str, b: Option<&str>| {
                cols.iter().position(|c| {
                    let p = c.path().parts();
                    p[0] == a && b.map_or(true, |b| p.len() > 1 && p[1] == b)
                })
            };
            let (hg, bx0, by0, bx1, by1) =
                (hoja("geom", None), hoja("geom_bbox", Some("xmin")), hoja("geom_bbox", Some("ymin")), hoja("geom_bbox", Some("xmax")), hoja("geom_bbox", Some("ymax")));
            let mut grupos = vec![];
            for (i, rg) in meta.metadata().row_groups().iter().enumerate() {
                m.grupos += 1;
                // por la cobertura: xmin ≤ caja.x1, xmax ≥ caja.x0, …
                let por_bbox = match (bx0, by0, bx1, by1) {
                    (Some(a), Some(b), Some(cc), Some(d)) => match (
                        stat_f64(rg.column(a).statistics()),
                        stat_f64(rg.column(b).statistics()),
                        stat_f64(rg.column(cc).statistics()),
                        stat_f64(rg.column(d).statistics()),
                    ) {
                        (Some(xa), Some(ya), Some(xb), Some(yb)) => !(xa.0 > cd.2 || ya.0 > cd.3 || xb.1 < cd.0 || yb.1 < cd.1),
                        _ => true,
                    },
                    _ => true,
                };
                let por_geo = match hg.and_then(|h| rg.column(h).geo_statistics().and_then(|g| g.bounding_box())) {
                    Some(b) => !(b.get_xmax() < cd.0 || b.get_xmin() > cd.2 || b.get_ymax() < cd.1 || b.get_ymin() > cd.3),
                    None => true,
                };
                m.grupos_geo += por_geo as usize;
                if por_bbox {
                    m.grupos_bbox += 1;
                    grupos.push(i);
                }
            }
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
                for i in 0..lote.num_rows() {
                    let b = if let Some(a) = g.as_any().downcast_ref::<LargeBinaryArray>() {
                        (!a.is_null(i)).then(|| a.value(i))
                    } else if let Some(a) = g.as_any().downcast_ref::<BinaryArray>() {
                        (!a.is_null(i)).then(|| a.value(i))
                    } else {
                        None
                    };
                    if let Some(b) = b {
                        m.filas += 1;
                        geoms.push((ids.value(i), b.to_vec()));
                    }
                }
            }
        }
        m.leer_ms = t0.elapsed().as_secs_f64() * 1000.0;

        // ── la cadena de geometría ──
        let t1 = Instant::now();
        let a_tesela = |lon: f64, lat: f64| -> Coord<f64> {
            let mxm = lon.to_radians() * R;
            let mym = R * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln();
            Coord { x: (mxm - c.0) / (c.2 - c.0) * EXTENT, y: (c.3 - mym) / (c.3 - c.1) * EXTENT }
        };
        let marco = Rect::new(Coord { x: -BUFFER, y: -BUFFER }, Coord { x: EXTENT + BUFFER, y: EXTENT + BUFFER }).to_polygon();
        let mut salida: Vec<(u64, i64, u64, Vec<Vec<(i64, i64)>>)> = Vec::new(); // (hilbert, id, área, anillos)
        for (id, b) in &geoms {
            let Some(mp) = wkb_poligonos(b, &a_tesela) else { continue };
            // fuera de la tesela (con margen) por la caja: no se recorta lo que no toca
            let toca = mp.0.iter().any(|p| p.exterior().0.iter().any(|q| q.x > -BUFFER && q.x < EXTENT + BUFFER && q.y > -BUFFER && q.y < EXTENT + BUFFER))
                || mp.0.iter().any(|p| cubre(p, EXTENT / 2.0));
            if !toca {
                m.fuera += 1;
                continue;
            }
            let simpl: MultiPolygon<f64> = match self.simplifica.as_str() {
                "no" => mp,
                "vw" => mp.simplify_vw_preserve(1.0),
                _ => mp.simplify(1.0),
            };
            let dentro = simpl.0.iter().all(|p| p.exterior().0.iter().all(|q| q.x >= -BUFFER && q.x <= EXTENT + BUFFER && q.y >= -BUFFER && q.y <= EXTENT + BUFFER));
            let recortada = if dentro { simpl } else { simpl.intersection(&MultiPolygon(vec![marco.clone()])) };
            // a la rejilla entera, sin repetidos, fuera lo diminuto, orientado (exterior > 0 con y hacia abajo)
            let mut anillos_mvt: Vec<Vec<(i64, i64)>> = Vec::new();
            let mut area_total = 0.0;
            let mut validos: Vec<Polygon<f64>> = vec![];
            for p in &recortada.0 {
                let Some(ext) = a_rejilla(p.exterior(), true) else {
                    m.diminutos += 1;
                    continue;
                };
                area_total += area(&ext).abs();
                let mut ints = vec![];
                for h in p.interiors() {
                    if let Some(r) = a_rejilla(h, false) {
                        ints.push(r);
                    }
                }
                validos.push(Polygon::new(
                    LineString::from(cerrado(&ext)),
                    ints.iter().map(|r| LineString::from(cerrado(r))).collect(),
                ));
                anillos_mvt.push(ext);
                anillos_mvt.extend(ints);
            }
            if anillos_mvt.is_empty() {
                continue;
            }
            if !MultiPolygon(validos).is_valid() {
                m.invalidos += 1;
            }
            let (cx, cy) = centro(&anillos_mvt[0]);
            salida.push((hilbert(cx, cy), *id, area_total.round() as u64, anillos_mvt));
        }
        salida.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        m.elementos = salida.len();
        m.geom_ms = t1.elapsed().as_secs_f64() * 1000.0;
        let t2 = Instant::now();
        let bytes = mvt_poligonos(&salida);
        m.cod_ms = t2.elapsed().as_secs_f64() * 1000.0;
        m.bytes = bytes.len();
        Ok((bytes, m))
    }
}

/// ¿Contiene el exterior el punto (c, c)? (la tesela entera dentro de un polígono grande)
fn cubre(p: &Polygon<f64>, c: f64) -> bool {
    let r = &p.exterior().0;
    let mut dentro = false;
    let mut j = r.len().saturating_sub(1);
    for i in 0..r.len() {
        let (a, b) = (r[i], r[j]);
        if (a.y > c) != (b.y > c) && c < (b.x - a.x) * (c - a.y) / (b.y - a.y) + a.x {
            dentro = !dentro;
        }
        j = i;
    }
    dentro
}

fn cerrado(r: &[(i64, i64)]) -> Vec<Coord<f64>> {
    let mut v: Vec<Coord<f64>> = r.iter().map(|&(x, y)| Coord { x: x as f64, y: y as f64 }).collect();
    v.push(v[0]);
    v
}

fn area(r: &[(i64, i64)]) -> f64 {
    let mut s = 0i128;
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        s += a.0 as i128 * b.1 as i128 - b.0 as i128 * a.1 as i128;
    }
    s as f64 / 2.0
}

/// El anillo en enteros, sin repetidos ni el cierre; `None` si queda en < 4 u² o < 3 vértices.
/// Exterior con área > 0 (horario con y hacia abajo), interior < 0 (spec 2.1).
fn a_rejilla(l: &LineString<f64>, exterior: bool) -> Option<Vec<(i64, i64)>> {
    let mut v: Vec<(i64, i64)> = Vec::with_capacity(l.0.len());
    for c in &l.0 {
        let p = (c.x.round() as i64, c.y.round() as i64);
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
    let celda = |v: i64| (((v as f64 + BUFFER) / (EXTENT + 2.0 * BUFFER) * (n - 1) as f64).round() as i64).clamp(0, n - 1);
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

/// Polígono o multipolígono WKB (2D, o Z/M ignorando lo demás), ya en coordenadas de tesela.
fn wkb_poligonos(b: &[u8], f: &dyn Fn(f64, f64) -> Coord<f64>) -> Option<MultiPolygon<f64>> {
    let mut i = 0usize;
    let mut out = vec![];
    fn u32_(b: &[u8], i: &mut usize, le: bool) -> Option<u32> {
        let s: [u8; 4] = b.get(*i..*i + 4)?.try_into().ok()?;
        *i += 4;
        Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }
    fn f64_(b: &[u8], i: &mut usize, le: bool) -> Option<f64> {
        let s: [u8; 8] = b.get(*i..*i + 8)?.try_into().ok()?;
        *i += 8;
        Some(if le { f64::from_le_bytes(s) } else { f64::from_be_bytes(s) })
    }
    fn dims(t: u32) -> (u32, usize) {
        // ISO (1000/2000/3000) y EWKB (bits altos)
        let ew_z = t & 0x8000_0000 != 0;
        let ew_m = t & 0x4000_0000 != 0;
        let t = t & 0x0fff_ffff;
        let (base, iso) = (t % 1000, t / 1000);
        let extra = match iso {
            1 | 2 => 1,
            3 => 2,
            _ => ew_z as usize + ew_m as usize,
        };
        (base, extra)
    }
    fn poligono(b: &[u8], i: &mut usize, le: bool, extra: usize, f: &dyn Fn(f64, f64) -> Coord<f64>) -> Option<Polygon<f64>> {
        let n = u32_(b, i, le)?;
        let mut anillos = vec![];
        for _ in 0..n {
            let k = u32_(b, i, le)?;
            let mut l = Vec::with_capacity(k as usize);
            for _ in 0..k {
                let x = f64_(b, i, le)?;
                let y = f64_(b, i, le)?;
                *i += 8 * extra;
                l.push(f(x, y));
            }
            anillos.push(LineString(l));
        }
        let mut it = anillos.into_iter();
        Some(Polygon::new(it.next()?, it.collect()))
    }
    let le = *b.first()? == 1;
    i += 1;
    let crudo = u32_(b, &mut i, le)?;
    let (t, extra) = dims(crudo);
    if crudo & 0x2000_0000 != 0 {
        i += 4;
    }
    match t {
        3 => out.push(poligono(b, &mut i, le, extra, f)?),
        6 => {
            let n = u32_(b, &mut i, le)?;
            for _ in 0..n {
                let le2 = *b.get(i)? == 1;
                i += 1;
                let (_, extra2) = dims(u32_(b, &mut i, le2)?);
                out.push(poligono(b, &mut i, le2, extra2, f)?);
            }
        }
        _ => return None,
    }
    Some(MultiPolygon(out))
}

// ── MVT a mano (protobuf, spec 2.1): capa «edificios», id, propiedad `a` (área en u²), POLYGON ──
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
fn mvt_poligonos(f: &[(u64, i64, u64, Vec<Vec<(i64, i64)>>)]) -> Vec<u8> {
    let mut valores: Vec<u64> = Vec::new();
    let mut indice: HashMap<u64, u64> = HashMap::new();
    let mut capa = Vec::with_capacity(f.len() * 64 + 64);
    clave(15, 0, &mut capa);
    varint(2, &mut capa);
    clave(1, 2, &mut capa);
    varint(9, &mut capa);
    capa.extend_from_slice(b"edificios");
    let mut feat = Vec::new();
    let mut geom = Vec::new();
    for (_, id, a, anillos) in f {
        feat.clear();
        geom.clear();
        let (mut cx, mut cy) = (0i64, 0i64);
        for r in anillos {
            varint((1 << 3 | 1) as u64, &mut geom); // MoveTo 1
            varint(zz(r[0].0 - cx), &mut geom);
            varint(zz(r[0].1 - cy), &mut geom);
            (cx, cy) = r[0];
            varint((((r.len() - 1) as u64) << 3) | 2, &mut geom); // LineTo n-1
            for p in &r[1..] {
                varint(zz(p.0 - cx), &mut geom);
                varint(zz(p.1 - cy), &mut geom);
                (cx, cy) = *p;
            }
            varint((1 << 3) | 7, &mut geom); // ClosePath
        }
        let vi = *indice.entry(*a).or_insert_with(|| {
            valores.push(*a);
            (valores.len() - 1) as u64
        });
        clave(1, 0, &mut feat);
        varint(*id as u64, &mut feat);
        clave(2, 2, &mut feat);
        let mut tags = vec![];
        varint(0, &mut tags);
        varint(vi, &mut tags);
        varint(tags.len() as u64, &mut feat);
        feat.extend_from_slice(&tags);
        clave(3, 0, &mut feat);
        varint(3, &mut feat); // POLYGON
        clave(4, 2, &mut feat);
        varint(geom.len() as u64, &mut feat);
        feat.extend_from_slice(&geom);
        clave(2, 2, &mut capa);
        varint(feat.len() as u64, &mut capa);
        capa.extend_from_slice(&feat);
    }
    clave(3, 2, &mut capa);
    varint(1, &mut capa);
    capa.push(b'a');
    for v in &valores {
        let mut val = vec![];
        clave(5, 0, &mut val); // uint_value
        varint(*v, &mut val);
        clave(4, 2, &mut capa);
        varint(val.len() as u64, &mut capa);
        capa.extend_from_slice(&val);
    }
    clave(5, 0, &mut capa);
    varint(EXTENT as u64, &mut capa);
    let mut t = Vec::with_capacity(capa.len() + 8);
    clave(3, 2, &mut t);
    varint(capa.len() as u64, &mut t);
    t.extend_from_slice(&capa);
    t
}

fn gzip(b: &[u8]) -> usize {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
    e.write_all(b).unwrap();
    e.finish().unwrap().len()
}

fn tesela_de(lon: f64, lat: f64, z: u32) -> (u32, u32) {
    let n = 2f64.powi(z as i32);
    ((((lon + 180.0) / 360.0) * n) as u32, ((1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n) as u32)
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

async fn motor() -> Arc<Motor> {
    let rutas: Vec<String> = std::env::var("F2_FICHEROS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect();
    let mut almacenes: HashMap<String, Arc<dyn ObjectStore>> = HashMap::new();
    let mut ficheros = Vec::new();
    for r in &rutas {
        let (cubo, obj) = r.trim_start_matches("gs://").split_once('/').expect("gs://cubo/objeto");
        let store = almacenes
            .entry(cubo.to_string())
            .or_insert_with(|| Arc::new(GoogleCloudStorageBuilder::from_env().with_bucket_name(cubo).build().expect("gcs")))
            .clone();
        let path = Path::from(obj);
        let size = store.head(&path).await.expect("head").size;
        ficheros.push(Fichero { store, path, size });
    }
    Arc::new(Motor { ficheros, pies: Mutex::new(HashMap::new()), simplifica: std::env::var("F2_SIMPLIFICA").unwrap_or_else(|_| "rdp".into()) })
}

const MADRID: (f64, f64) = (-3.7038, 40.4168);

async fn medir() {
    let t = Instant::now();
    let m = motor().await;
    di("inicio", serde_json::json!({"ficheros": m.ficheros.len(), "simplifica": m.simplifica, "arranque_ms": t.elapsed().as_millis(), "rss_mb": rss_mb()}));
    // Madrid de z8 a z16 (dos veces: fría y con los pies en caché), y la tesela guardada para el contraste
    let _ = std::fs::create_dir_all("/tmp/teselas");
    for z in 8u32..=16 {
        let (x, y) = tesela_de(MADRID.0, MADRID.1, z);
        let mut fria = 0.0;
        for k in 0..2 {
            let t0 = Instant::now();
            match m.tesela(z, x, y).await {
                Ok((b, md)) => {
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    if k == 0 {
                        fria = ms;
                        continue;
                    }
                    let _ = std::fs::write(format!("/tmp/teselas/{z}_{x}_{y}.mvt"), &b);
                    di("tesela", serde_json::json!({"z": z, "x": x, "y": y, "frio_ms": fria.round(), "ms": (ms * 10.0).round() / 10.0,
                        "leer_ms": md.leer_ms.round(), "geom_ms": md.geom_ms.round(), "cod_ms": (md.cod_ms * 10.0).round() / 10.0,
                        "kb": (md.bytes as f64 / 102.4).round() / 10.0, "kb_gzip": (gzip(&b) as f64 / 102.4).round() / 10.0,
                        "elementos": md.elementos, "filas_leidas": md.filas, "fuera": md.fuera, "diminutos": md.diminutos, "invalidos": md.invalidos,
                        "grupos_bbox": md.grupos_bbox, "grupos_geo": md.grupos_geo, "grupos": md.grupos, "rss_mb": rss_mb().round()}));
                }
                Err(e) => {
                    di("tesela", serde_json::json!({"z": z, "error": e}));
                    break;
                }
            }
        }
    }
    // concurrencia en z12–z16 al azar sobre la zona de los datos
    let segundos: u64 = std::env::var("F2_SEGUNDOS").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    for lectores in [1usize, 8, 32] {
        let fin = Instant::now() + std::time::Duration::from_secs(segundos);
        let mut tareas = vec![];
        for k in 0..lectores {
            let m = m.clone();
            tareas.push(tokio::spawn(async move {
                let mut rnd = rand::rngs::StdRng::seed_from_u64(k as u64);
                let mut t = vec![];
                while Instant::now() < fin {
                    let z = rnd.gen_range(12..=16);
                    let (x, y) = tesela_de(MADRID.0 + rnd.gen_range(-0.6..0.6), MADRID.1 + rnd.gen_range(-0.4..0.4), z);
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
        di("concurrencia", serde_json::json!({"lectores": lectores, "teselas": n, "por_s": (n as f64 / segundos as f64 * 10.0).round() / 10.0,
            "p50_ms": percentil(&mut todos, 0.5).round(), "p95_ms": percentil(&mut todos, 0.95).round(), "rss_mb": rss_mb().round()}));
    }
}

// ── M2 · el salto ──────────────────────────────────────────────────────────────────────────────────
async fn servir(puerto: u16) {
    let m = motor().await;
    let l = tokio::net::TcpListener::bind(("0.0.0.0", puerto)).await.unwrap();
    di("servir", serde_json::json!({"puerto": puerto}));
    loop {
        let (mut s, _) = l.accept().await.unwrap();
        let m = m.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 2048];
            let mut n = 0;
            while !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf[n..]).await {
                    Ok(0) | Err(_) => return,
                    Ok(k) => n += k,
                }
            }
            let linea = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string();
            // `…/t/{z}/{x}/{y}[.mvt][?firma=…]`: el prefijo de la ruta y la firma no cuentan aquí (M3)
            let ruta = linea.split_whitespace().nth(1).unwrap_or("");
            let ruta = ruta.split('?').next().unwrap_or("");
            let ruta = ruta.rsplit_once("/t/").map(|(_, r)| r).unwrap_or("");
            let p: Vec<u32> = ruta.trim_end_matches(".mvt").split('/').filter_map(|v| v.parse().ok()).collect();
            let (cod, cuerpo, ms) = if p.len() == 3 {
                let t0 = Instant::now();
                match m.tesela(p[0], p[1], p[2]).await {
                    Ok((b, _)) => ("200 OK", b, t0.elapsed().as_secs_f64() * 1000.0),
                    Err(e) => ("500 Internal Server Error", e.into_bytes(), 0.0),
                }
            } else {
                ("404 Not Found", vec![], 0.0)
            };
            // la huella del contenido como ETag (FNV-1a: determinista, sin dependencias)
            let mut h: u64 = 0xcbf29ce484222325;
            for b in &cuerpo {
                h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
            }
            let cab = format!(
                "HTTP/1.1 {cod}\r\ncontent-type: application/vnd.mapbox-vector-tile\r\ncontent-length: {}\r\n\
                 cache-control: public, max-age=31536000, immutable\r\netag: \"{h:016x}\"\r\n\
                 access-control-allow-origin: *\r\nserver-timing: motor;dur={ms:.1}\r\nconnection: close\r\n\r\n",
                cuerpo.len()
            );
            let _ = s.write_all(cab.as_bytes()).await;
            let _ = s.write_all(&cuerpo).await;
        });
    }
}

/// La puerta: un hilo por conexión y una conexión nueva al motor por tesela (`al_motor`).
fn puerta(puerto: u16, motor: String) {
    let l = std::net::TcpListener::bind(("0.0.0.0", puerto)).unwrap();
    di("puerta", serde_json::json!({"puerto": puerto, "motor": motor}));
    for s in l.incoming() {
        let Ok(mut s) = s else { continue };
        let motor = motor.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 2048];
            let mut n = 0;
            while !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf[n..]) {
                    Ok(0) | Err(_) => return,
                    Ok(k) => n += k,
                }
            }
            let Ok(mut m) = std::net::TcpStream::connect(&motor) else { return };
            m.set_nodelay(true).ok();
            let linea = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string();
            let ruta = linea.split_whitespace().nth(1).unwrap_or("/");
            let _ = m.write_all(format!("GET {ruta} HTTP/1.1\r\nhost: motor\r\nconnection: close\r\n\r\n").as_bytes());
            let mut r = Vec::new();
            let _ = m.read_to_end(&mut r);
            let _ = s.write_all(&r);
        });
    }
}

/// La carga: N lectores, una conexión por tesela, teselas z12–z16 al azar sobre Madrid.
fn carga(destino: String, lectores: usize, segundos: u64) {
    let fin = Instant::now() + std::time::Duration::from_secs(segundos);
    let hilos: Vec<_> = (0..lectores)
        .map(|k| {
            let destino = destino.clone();
            std::thread::spawn(move || {
                let mut rnd = rand::rngs::StdRng::seed_from_u64(1000 + k as u64);
                let mut t = vec![];
                let mut errores = 0;
                while Instant::now() < fin {
                    let z = rnd.gen_range(12..=16);
                    let (x, y) = tesela_de(MADRID.0 + rnd.gen_range(-0.6..0.6), MADRID.1 + rnd.gen_range(-0.4..0.4), z);
                    let t0 = Instant::now();
                    let ok = (|| -> std::io::Result<bool> {
                        let mut s = std::net::TcpStream::connect(&destino)?;
                        s.set_nodelay(true).ok();
                        s.write_all(format!("GET /t/{z}/{x}/{y} HTTP/1.1\r\nhost: m\r\nconnection: close\r\n\r\n").as_bytes())?;
                        let mut r = Vec::new();
                        s.read_to_end(&mut r)?;
                        Ok(r.starts_with(b"HTTP/1.1 200"))
                    })()
                    .unwrap_or(false);
                    if ok {
                        t.push(t0.elapsed().as_secs_f64() * 1000.0);
                    } else {
                        errores += 1;
                    }
                }
                (t, errores)
            })
        })
        .collect();
    let mut todos = vec![];
    let mut errores = 0;
    for h in hilos {
        let (t, e) = h.join().unwrap();
        todos.extend(t);
        errores += e;
    }
    let n = todos.len();
    di("carga", serde_json::json!({"destino": destino, "lectores": lectores, "teselas": n, "errores": errores,
        "por_s": (n as f64 / segundos as f64 * 10.0).round() / 10.0, "p50_ms": (percentil(&mut todos, 0.5) * 10.0).round() / 10.0,
        "p95_ms": (percentil(&mut todos, 0.95) * 10.0).round() / 10.0}));
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let modo = a.get(1).map(String::as_str).unwrap_or("medir");
    let rt = || tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    match modo {
        "medir" => rt().block_on(medir()),
        "servir" => rt().block_on(servir(a[2].parse().unwrap())),
        "puerta" => puerta(a[2].parse().unwrap(), a[3].clone()),
        "carga" => carga(a[2].clone(), a[3].parse().unwrap(), a[4].parse().unwrap()),
        _ => eprintln!("modo: medir | servir <puerto> | puerta <puerto> <motor> | carga <destino> <lectores> <s>"),
    }
}
