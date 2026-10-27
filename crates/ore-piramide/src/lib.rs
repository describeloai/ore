//! **`ore-piramide`** (ADR 0072 F2·3·2·2, D-F2-16): la pirámide de una capa, de z0 al suelo − 1, en un
//! PMTiles que sirve `ore-teselas` (F2·3·2·1). Lee la instantánea del lago como el motor (`ore-store`, los
//! trozos por rango, el mismo WKB) y, según lo medido en la prueba de concepto:
//!
//! - **los puntos se aclaran**: van tal cual a tippecanoe, que elige cuáles se ven en cada zoom
//!   (`--drop-densest-as-needed`: anidado, un punto que entra no sale);
//! - **los polígonos y las líneas se agregan**: una rejilla de 2⁵ × 2⁵ celdas por tesela en WebMercator. La
//!   más fina se llena una vez, en memoria (acotada por las celdas, no por las filas: nada a disco) y las
//!   demás salen de ella (`cx >> s`), así que todos los zooms cuentan lo mismo. Cada celda lleva `n`, `a` y
//!   la `cobertura` por m², que no depende del zoom: área construida / área de la celda, o km de línea por
//!   km².
//!
//! Los elementos van a tippecanoe **por la entrada estándar** (GeoJSON por líneas): sin fichero intermedio.
//! El PMTiles y su manifiesto se suben al destino con `ore-store` (el blob, cotejado). Todo acotado: tiempo
//! de tippecanoe, celdas en memoria (D-F2-17).

use std::collections::HashMap;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ore_base::json::Json;
use ore_store::almacen::{Almacen, Blob, Cuerpo};
use ore_store::lago::Lago;
use ore_teselas::Geometria;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use sha2::Digest;

/// La versión de lo que se construye: entra en la huella del destino (D-F2-15).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Celdas por lado de tesela en la rejilla (2⁵ = 32).
pub const CELDAS: u32 = 5;
const LAT_MAX: f64 = 85.051_128_779_806_6;
const R_X: f64 = 111_320.0;
const R_Y: f64 = 110_540.0;

pub mod indice;

// ── La petición ─────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    /// Aclarar (puntos).
    Puntos,
    /// Agregar en la rejilla (polígonos y líneas).
    Agregado,
}

/// Dónde se lee o se escribe y con qué: un token dado, o la identidad del pod (`None`).
#[derive(Debug, Clone)]
pub struct Lugar {
    pub ubicacion: String,
    pub token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Peticion {
    pub metadata_location: String,
    pub credencial: Option<String>,
    pub geometria: String,
    pub id: Option<String>,
    pub crs: String,
    /// El nombre de la capa en el MVT (el motor lo renombra al servir: aquí basta uno estable).
    pub capa: String,
    /// El primer zoom del crudo: la pirámide llega a `suelo − 1`. `None`: lo decide el tipo
    /// ([`suelo_por_tipo`]), que sólo se sabe al leer.
    pub suelo: Option<u32>,
    /// `None`: lo decide la primera geometría que no sea un punto.
    pub tipo: Option<Tipo>,
    pub destino: Lugar,
    pub segundos: u64,
    pub max_celdas: usize,
    pub trabajo: PathBuf,
    pub tippecanoe: String,
}

pub fn peticion(texto: &str) -> Result<Peticion, String> {
    let n = ore_base::parse::parse(texto).map_err(|e| format!("la petición no es JSON: {e:?}"))?;
    let en = |o: &str, k: &str| {
        n.get(o)
            .and_then(|(_, v)| v.get(k))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
    };
    let s = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let falta = |k: &str| format!("falta `{k}`");
    let num = |o: Option<String>, d: u64, k: &str| -> Result<u64, String> {
        o.map_or(Ok(d), |v| {
            v.parse().map_err(|_| format!("`{k}` no es un entero"))
        })
    };
    let crs = en("fuente", "crs").unwrap_or_else(|| "EPSG:4326".into());
    if !matches!(crs.as_str(), "EPSG:4326" | "OGC:CRS84" | "EPSG:3857") {
        return Err(format!("CRS `{crs}`: la pirámide lee 4326 y 3857"));
    }
    let tipo = match s("tipo").as_deref() {
        None | Some("auto") => None,
        Some("puntos") => Some(Tipo::Puntos),
        Some("agregado") => Some(Tipo::Agregado),
        Some(o) => return Err(format!("`tipo` {o}: auto, puntos o agregado")),
    };
    let suelo = match s("suelo") {
        None => None,
        Some(v) => {
            let z: u32 = v
                .parse()
                .map_err(|_| "`suelo` no es un entero".to_string())?;
            if !(1..=20).contains(&z) {
                return Err(format!("`suelo` {z}: entre 1 y 20"));
            }
            Some(z)
        }
    };
    Ok(Peticion {
        metadata_location: en("fuente", "metadata_location")
            .ok_or_else(|| falta("fuente.metadata_location"))?,
        credencial: n
            .get("fuente")
            .and_then(|(_, f)| f.get("credencial"))
            .and_then(|(_, c)| c.get("gcs.oauth2.token"))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string),
        geometria: en("fuente", "geometria").unwrap_or_else(|| "geom".into()),
        id: en("fuente", "id").filter(|v| !v.is_empty()),
        crs,
        capa: s("capa").unwrap_or_else(|| "capa".into()),
        suelo,
        tipo,
        destino: Lugar {
            ubicacion: en("destino", "ubicacion").ok_or_else(|| falta("destino.ubicacion"))?,
            token: n
                .get("destino")
                .and_then(|(_, f)| f.get("credencial"))
                .and_then(|(_, c)| c.get("gcs.oauth2.token"))
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string),
        },
        segundos: num(en("limites", "segundos"), 1800, "limites.segundos")?,
        max_celdas: num(en("limites", "celdas"), 20_000_000, "limites.celdas")? as usize,
        trabajo: PathBuf::from(s("trabajo").unwrap_or_else(|| "/tmp".into())),
        tippecanoe: std::env::var("ORE_TIPPECANOE").unwrap_or_else(|_| "tippecanoe".into()),
    })
}

/// `gs://cubo/clave` → (`cubo`, `clave`).
pub fn partir(ubicacion: &str) -> Result<(&str, &str), String> {
    let r = ubicacion
        .strip_prefix("gs://")
        .ok_or_else(|| format!("`{ubicacion}`: la pirámide lee y escribe en gs://"))?;
    r.split_once('/')
        .filter(|(c, k)| !c.is_empty() && !k.is_empty())
        .ok_or_else(|| format!("`{ubicacion}`: falta el objeto"))
}

pub fn cuenta_de(cubo: &str, token: Option<&str>) -> Result<Arc<dyn Almacen>, String> {
    Ok(Arc::new(match token {
        Some(t) => ore_store::gcs::Cuenta::con_token(cubo, t)?,
        None => ore_store::gcs::Cuenta::de_la_identidad(cubo)?,
    }))
}

// ── La rejilla ──────────────────────────────────────────────────────────────────────────────────

/// La celda de (lon, lat) en el nivel `l` de WebMercator (2^l celdas por lado).
pub fn celda(lon: f64, lat: f64, l: u32) -> (u32, u32) {
    let n = (1u64 << l) as f64;
    let lat = lat.clamp(-LAT_MAX, LAT_MAX);
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let y = ((1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor();
    (x.clamp(0.0, n - 1.0) as u32, y.clamp(0.0, n - 1.0) as u32)
}

/// La caja (lon0, lat0, lon1, lat1) de una celda.
pub fn caja(cx: u32, cy: u32, l: u32) -> (f64, f64, f64, f64) {
    let n = (1u64 << l) as f64;
    let lat = |y: f64| {
        (std::f64::consts::PI * (1.0 - 2.0 * y / n))
            .sinh()
            .atan()
            .to_degrees()
    };
    (
        cx as f64 / n * 360.0 - 180.0,
        lat(cy as f64 + 1.0),
        (cx as f64 + 1.0) / n * 360.0 - 180.0,
        lat(cy as f64),
    )
}

/// Área (m²) y centroide de un polígono en lon/lat: el exterior menos los agujeros, en el plano
/// local de su latitud (a la escala de un edificio, lo mismo que el esferoide: medido < 1 %).
pub fn area_y_centro(p: &[Vec<(f64, f64)>]) -> Option<(f64, (f64, f64))> {
    let ext = p.first().filter(|r| !r.is_empty())?;
    let lat0 = ext.iter().map(|q| q.1).sum::<f64>() / ext.len() as f64;
    let kx = R_X * lat0.to_radians().cos();
    // respecto al primer vértice: con lon/lat enteras, los productos de la fórmula del cordón se
    // comen los decimales de un edificio
    let o = ext[0];
    let anillo = |r: &[(f64, f64)]| {
        let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
        for w in r.windows(2) {
            let ((x0, y0), (x1, y1)) = ((w[0].0 - o.0, w[0].1 - o.1), (w[1].0 - o.0, w[1].1 - o.1));
            let c = x0 * y1 - x1 * y0;
            a += c;
            cx += (x0 + x1) * c;
            cy += (y0 + y1) * c;
        }
        (a / 2.0, cx, cy)
    };
    let (a_ext, cx, cy) = anillo(ext);
    let centro = if a_ext.abs() > 1e-18 {
        (o.0 + cx / (6.0 * a_ext), o.1 + cy / (6.0 * a_ext))
    } else {
        let n = ext.len() as f64;
        (ext.iter().map(|q| q.0).sum::<f64>() / n, lat0)
    };
    let huecos: f64 = p[1..].iter().map(|r| anillo(r).0.abs()).sum();
    Some(((a_ext.abs() - huecos).max(0.0) * kx * R_Y, centro))
}

/// Las celdas de un nivel: por (x, y), `n` y `a`.
pub type Celdas = HashMap<(u32, u32), (u64, f64)>;

/// La rejilla fina: por celda, `n` y `a` (m² de polígono o m de línea).
pub struct Rejilla {
    pub nivel: u32,
    pub celdas: Celdas,
    pub max_celdas: usize,
}

impl Rejilla {
    pub fn nueva(hasta: u32, max_celdas: usize) -> Rejilla {
        Rejilla {
            nivel: hasta + CELDAS,
            celdas: HashMap::new(),
            max_celdas,
        }
    }

    fn sumar(&mut self, c: (u32, u32), n: u64, a: f64) -> Result<(), String> {
        let e = self.celdas.entry(c).or_insert((0, 0.0));
        e.0 += n;
        e.1 += a;
        if self.celdas.len() > self.max_celdas {
            return Err(format!(
                "la rejilla pasa de {} celdas: sube el suelo o el tope",
                self.max_celdas
            ));
        }
        Ok(())
    }

    /// Una geometría: un polígono por su centroide y su área; una línea muestreada a un lado de
    /// celda (los metros de cada tramo, a su celda; `n` cuenta una vez por línea y celda); un punto,
    /// `n`.
    pub fn agregar(&mut self, g: &Geometria) -> Result<(), String> {
        match g {
            Geometria::Puntos(ps) => {
                for &(x, y) in ps {
                    self.sumar(celda(x, y, self.nivel), 1, 0.0)?;
                }
            }
            Geometria::Poligonos(pp) => {
                for p in pp {
                    if let Some((a, (x, y))) = area_y_centro(p) {
                        self.sumar(celda(x, y, self.nivel), 1, a)?;
                    }
                }
            }
            Geometria::Lineas(ls) => {
                let paso = 360.0 / (1u64 << self.nivel) as f64;
                for l in ls {
                    let mut vistas: HashMap<(u32, u32), f64> = HashMap::new();
                    for w in l.windows(2) {
                        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                        let (dx, dy) = (x1 - x0, y1 - y0);
                        let k = ((dx * dx + dy * dy).sqrt() / paso).ceil().max(1.0) as u64;
                        let m = ((dx * R_X * y0.to_radians().cos()).powi(2) + (dy * R_Y).powi(2))
                            .sqrt();
                        for j in 0..k {
                            let t = (j as f64 + 0.5) / k as f64;
                            *vistas
                                .entry(celda(x0 + dx * t, y0 + dy * t, self.nivel))
                                .or_insert(0.0) += m / k as f64;
                        }
                    }
                    for (c, m) in vistas {
                        self.sumar(c, 1, m)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Las celdas de cada zoom (0..=`nivel` − CELDAS), sumadas desde la fina.
    pub fn por_zoom(&self) -> Vec<(u32, Celdas)> {
        let hasta = self.nivel - CELDAS;
        (0..=hasta)
            .map(|z| {
                let d = hasta - z;
                let mut m: HashMap<(u32, u32), (u64, f64)> = HashMap::new();
                for (&(x, y), &(n, a)) in &self.celdas {
                    let e = m.entry((x >> d, y >> d)).or_insert((0, 0.0));
                    e.0 += n;
                    e.1 += a;
                }
                (z, m)
            })
            .collect()
    }
}

/// Una celda como GeoJSON (sólo en su zoom), con la cobertura por m² de celda: área / área para los
/// polígonos (tope 1), km de línea por km² para las líneas.
pub fn celda_geojson(z: u32, (cx, cy): (u32, u32), (n, a): (u64, f64), lineas: bool) -> String {
    let l = z + CELDAS;
    let (x0, y0, x1, y1) = caja(cx, cy, l);
    let lado = 40_075_016.7 * ((y0 + y1) / 2.0).to_radians().cos() / (1u64 << l) as f64;
    let cob = a / (lado * lado);
    let cob = if lineas { cob * 1000.0 } else { cob.min(1.0) };
    format!(
        r#"{{"type":"Feature","tippecanoe":{{"minzoom":{z},"maxzoom":{z}}},"properties":{{"n":{n},"a":{},"cobertura":{:.4}}},"geometry":{{"type":"Polygon","coordinates":[[[{x0:.7},{y0:.7}],[{x1:.7},{y0:.7}],[{x1:.7},{y1:.7}],[{x0:.7},{y1:.7}],[{x0:.7},{y0:.7}]]]}}}}"#,
        a.round() as i64,
        cob
    )
}

/// Un punto como GeoJSON, con su id.
pub fn punto_geojson(id: Option<u64>, (x, y): (f64, f64)) -> String {
    match id {
        Some(i) => format!(
            r#"{{"type":"Feature","properties":{{"id":{i}}},"geometry":{{"type":"Point","coordinates":[{x:.7},{y:.7}]}}}}"#
        ),
        None => format!(
            r#"{{"type":"Feature","properties":{{}},"geometry":{{"type":"Point","coordinates":[{x:.7},{y:.7}]}}}}"#
        ),
    }
}

fn a_lonlat(crs: &str) -> fn(f64, f64) -> (f64, f64) {
    if crs == "EPSG:3857" {
        |x, y| {
            const R: f64 = 6_378_137.0;
            (
                (x / R).to_degrees(),
                (2.0 * (y / R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees(),
            )
        }
    } else {
        |x, y| (x, y)
    }
}

// ── Leer ────────────────────────────────────────────────────────────────────────────────────────

/// Lo leído: filas, geometrías por tipo y las que no se entienden.
#[derive(Default, Debug, Clone)]
pub struct Leido {
    pub ficheros: usize,
    pub filas: u64,
    pub puntos: u64,
    pub lineas: u64,
    pub poligonos: u64,
    pub descartadas: u64,
}

/// Recorre la instantánea fila a fila (por grupos, con sólo `geom` e `id`) y le da cada geometría a
/// `f`, en lon/lat. Lo que no se entiende (curvas, WKB roto) se cuenta y se salta.
pub fn recorrer(
    cuenta: &Arc<dyn Almacen>,
    p: &Peticion,
    mut f: impl FnMut(Option<u64>, &Geometria) -> Result<(), String>,
) -> Result<Leido, String> {
    let lago = Lago::nuevo(cuenta.clone());
    let tabla = lago.abrir(&p.metadata_location, "piramide")?;
    let (_, vivos) = ore_store::muestra::ficheros_vivos(&lago, &tabla)?;
    let tr = a_lonlat(&p.crs);
    let mut l = Leido {
        ficheros: vivos.len(),
        ..Leido::default()
    };
    for v in vivos {
        let t = ore_teselas::lector::Trozos {
            cuenta: cuenta.clone(),
            clave: v.clave.clone(),
            largo: v.bytes,
            trozos: Arc::default(),
            aparte: Arc::new((AtomicU64::new(0), AtomicU64::new(0))),
        };
        let b = ParquetRecordBatchReaderBuilder::try_new(t)
            .map_err(|e| format!("`{}`: {e}", v.clave))?;
        let esquema = b.schema().clone();
        let mut raices = vec![
            esquema
                .index_of(&p.geometria)
                .map_err(|_| format!("no hay columna `{}`", p.geometria))?,
        ];
        if let Some(id) = &p.id {
            raices.push(
                esquema
                    .index_of(id)
                    .map_err(|_| format!("no hay columna `{id}`"))?,
            );
        }
        let mascara = ProjectionMask::roots(b.parquet_schema(), raices);
        let lector = b
            .with_projection(mascara)
            .with_batch_size(65_536)
            .build()
            .map_err(|e| e.to_string())?;
        for lote in lector {
            let lote = lote.map_err(|e| e.to_string())?;
            let mut filas = vec![];
            ore_teselas::recoger(&lote, &p.geometria, p.id.as_deref(), &mut filas)
                .map_err(|e| e.motivo().to_string())?;
            for (id, wkb) in filas {
                l.filas += 1;
                let Some(gs) = ore_teselas::geometrias(&wkb, &tr) else {
                    l.descartadas += 1;
                    continue;
                };
                for g in &gs {
                    match g {
                        Geometria::Puntos(_) => l.puntos += 1,
                        Geometria::Lineas(_) => l.lineas += 1,
                        Geometria::Poligonos(_) => l.poligonos += 1,
                    }
                    f(id, g)?;
                }
            }
        }
    }
    Ok(l)
}

// ── tippecanoe ──────────────────────────────────────────────────────────────────────────────────

/// tippecanoe en marcha, alimentado por su entrada estándar, con su error guardado (la cola) y un
/// tope de tiempo: al pasarlo se mata y la construcción falla.
pub struct Tippecanoe {
    hijo: Arc<Mutex<Child>>,
    entrada: BufWriter<std::process::ChildStdin>,
    error: std::thread::JoinHandle<String>,
    vigia: std::thread::JoinHandle<bool>,
    hecho: Arc<std::sync::atomic::AtomicBool>,
    pub orden: Vec<String>,
}

impl Tippecanoe {
    pub fn arrancar(
        p: &Peticion,
        salida: &Path,
        tipo: Tipo,
        suelo: u32,
    ) -> Result<Tippecanoe, String> {
        let hasta = suelo - 1;
        let base = format!("-B{}", suelo + 3);
        let mut orden: Vec<String> = [
            "-q",
            "-f",
            "-o",
            &salida.to_string_lossy(),
            "-Z0",
            &format!("-z{hasta}"),
            "-l",
            &p.capa,
        ]
        .map(String::from)
        .to_vec();
        orden.extend(
            match tipo {
                // el zoom base (donde estarían todos) tres por encima del suelo, como en la prueba de
                // concepto (base 16): cada zoom lleva 1/2,5 del de arriba y ninguna tesela llega al tope,
                // así que tippecanoe no aclara tesela a tesela y queda anidado. Con el base en el máximo
                // de la pirámide (z12) o en el suelo (z13), z9–z12 llegaban al tope y hasta el 55 % de los
                // puntos desaparecían al acercarse (banco de victor, 2026-10-26). El salto que queda en
                // el suelo (el crudo los trae todos) lo cierra el suelo calculado (F2·3·3)
                Tipo::Puntos => vec![
                    base.as_str(),
                    "--drop-densest-as-needed",
                    "--maximum-tile-bytes=500000",
                ],
                // las celdas ya son lo que se ve: ni se aclaran ni se simplifican
                Tipo::Agregado => vec![
                    "--no-tile-size-limit",
                    "--no-feature-limit",
                    "--no-tiny-polygon-reduction",
                    "--no-line-simplification",
                ],
            }
            .into_iter()
            .map(String::from),
        );
        let mut hijo = Command::new(&p.tippecanoe)
            .args(&orden)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("tippecanoe (`{}`) no arranca: {e}", p.tippecanoe))?;
        let entrada = BufWriter::with_capacity(1 << 20, hijo.stdin.take().unwrap());
        let mut err = hijo.stderr.take().unwrap();
        let error = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s.chars()
                .rev()
                .take(2000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        });
        let hijo = Arc::new(Mutex::new(hijo));
        let hecho = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (h, listo, tope) = (hijo.clone(), hecho.clone(), Duration::from_secs(p.segundos));
        let vigia = std::thread::spawn(move || {
            let t0 = Instant::now();
            while !listo.load(std::sync::atomic::Ordering::Relaxed) {
                if t0.elapsed() > tope {
                    let _ = h.lock().unwrap().kill();
                    return true;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            false
        });
        Ok(Tippecanoe {
            hijo,
            entrada,
            error,
            vigia,
            hecho,
            orden,
        })
    }

    pub fn escribir(&mut self, linea: &str) -> Result<(), String> {
        self.entrada
            .write_all(linea.as_bytes())
            .and_then(|_| self.entrada.write_all(b"\n"))
            .map_err(|e| format!("tippecanoe dejó de leer: {e}"))
    }

    /// Cierra la entrada y espera: `Ok` si acabó bien.
    pub fn acabar(self) -> Result<(), String> {
        let Tippecanoe {
            hijo,
            entrada,
            error,
            vigia,
            hecho,
            ..
        } = self;
        let cerrar = entrada.into_inner().map_err(|e| e.to_string()).map(drop);
        let estado = loop {
            if let Some(s) = hijo.lock().unwrap().try_wait().map_err(|e| e.to_string())? {
                break s;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        hecho.store(true, std::sync::atomic::Ordering::Relaxed);
        let paso_el_tope = vigia.join().unwrap_or(false);
        let error = error.join().unwrap_or_default();
        if paso_el_tope {
            return Err("tippecanoe pasó de su tope de tiempo".into());
        }
        if !estado.success() {
            return Err(format!("tippecanoe acabó con {estado}: {error}"));
        }
        cerrar.map_err(|e| format!("tippecanoe dejó de leer: {e} · {error}"))
    }
}

/// El suelo por defecto de cada tipo (lo medido en F2·0 y F2·3): las teselas del crudo caben en el
/// presupuesto desde z13 para puntos y polígonos, y desde z14 para las líneas (en z13 pasaban de 7 MB).
pub fn suelo_por_tipo(g: &Geometria) -> u32 {
    match g {
        Geometria::Lineas(_) => 14,
        _ => 13,
    }
}

// ── Construir ───────────────────────────────────────────────────────────────────────────────────

/// Lo que dice una construcción (y el manifiesto que va junto al PMTiles).
#[derive(Debug, Clone)]
pub struct Hecho {
    pub tipo: Tipo,
    /// El suelo con el que se construyó: el pedido, o el del tipo.
    pub suelo: u32,
    pub leido: Leido,
    pub celdas: usize,
    pub bytes: u64,
    pub leer_s: f64,
    pub tippecanoe_s: f64,
    pub orden: String,
}

/// **Construye** el PMTiles en `trabajo` (sin subirlo). `di` cuenta por dónde va.
pub fn construir(
    cuenta: &Arc<dyn Almacen>,
    p: &Peticion,
    salida: &Path,
    di: &mut dyn FnMut(&str, Json),
) -> Result<Hecho, String> {
    let t0 = Instant::now();
    // el tipo y el suelo: los pedidos, o los de la primera geometría
    let (tipo, suelo) = match (p.tipo, p.suelo) {
        (Some(t), Some(z)) => (t, z),
        _ => {
            let mut primera = None;
            let _ = recorrer(cuenta, p, |_, g| {
                primera = Some((
                    match g {
                        Geometria::Puntos(_) => Tipo::Puntos,
                        _ => Tipo::Agregado,
                    },
                    suelo_por_tipo(g),
                ));
                Err("visto".into())
            });
            let (t, z) =
                primera.ok_or("la instantánea no tiene geometrías que se puedan pintar")?;
            (p.tipo.unwrap_or(t), p.suelo.unwrap_or(z))
        }
    };
    let mut tip = Tippecanoe::arrancar(p, salida, tipo, suelo)?;
    let orden = tip.orden.join(" ");
    let (leido, celdas, leer_s) = match tipo {
        Tipo::Puntos => {
            let mut otros = 0u64;
            let l = recorrer(cuenta, p, |id, g| {
                match g {
                    Geometria::Puntos(ps) => {
                        for q in ps {
                            tip.escribir(&punto_geojson(id, *q))?;
                        }
                    }
                    _ => otros += 1,
                }
                Ok(())
            })?;
            if otros > 0 {
                di("aviso", Json::obj([("no_puntos", Json::Int(otros as i64))]));
            }
            (l, 0, t0.elapsed().as_secs_f64())
        }
        Tipo::Agregado => {
            let mut r = Rejilla::nueva(suelo - 1, p.max_celdas);
            let l = recorrer(cuenta, p, |_, g| r.agregar(g))?;
            let leer_s = t0.elapsed().as_secs_f64();
            let lineas = l.lineas > l.poligonos;
            let fina = r.celdas.len();
            for (z, m) in r.por_zoom() {
                for (c, v) in m {
                    tip.escribir(&celda_geojson(z, c, v, lineas))?;
                }
            }
            (l, fina, leer_s)
        }
    };
    di(
        "leido",
        Json::obj([
            ("filas", Json::Int(leido.filas as i64)),
            ("puntos", Json::Int(leido.puntos as i64)),
            ("lineas", Json::Int(leido.lineas as i64)),
            ("poligonos", Json::Int(leido.poligonos as i64)),
            ("descartadas", Json::Int(leido.descartadas as i64)),
            ("celdas", Json::Int(celdas as i64)),
            ("s", Json::Crudo(format!("{leer_s:.1}"))),
        ]),
    );
    let t1 = Instant::now();
    tip.acabar()?;
    let bytes = std::fs::metadata(salida)
        .map_err(|e| format!("tippecanoe no dejó `{}`: {e}", salida.display()))?
        .len();
    Ok(Hecho {
        tipo,
        suelo,
        leido,
        celdas,
        bytes,
        leer_s,
        tippecanoe_s: t1.elapsed().as_secs_f64(),
        orden,
    })
}

/// El manifiesto (D-F2-15): lo que el descriptor y la salud leen sin abrir el PMTiles.
pub fn manifiesto(p: &Peticion, h: &Hecho, version_tippecanoe: &str, subir_s: f64) -> Json {
    let n = |v: u64| Json::Int(v as i64);
    Json::obj([
        ("version", Json::Int(1)),
        ("ore_piramide", Json::s(VERSION)),
        ("tippecanoe", Json::s(version_tippecanoe)),
        ("instantanea", Json::s(p.metadata_location.clone())),
        ("capa", Json::s(p.capa.clone())),
        (
            "tipo",
            Json::s(match h.tipo {
                Tipo::Puntos => "puntos",
                Tipo::Agregado => "agregado",
            }),
        ),
        ("suelo", n(h.suelo as u64)),
        ("minzoom", n(0)),
        ("maxzoom", n(h.suelo as u64 - 1)),
        ("filas", n(h.leido.filas)),
        ("descartadas", n(h.leido.descartadas)),
        ("celdas", n(h.celdas as u64)),
        ("bytes", n(h.bytes)),
        ("leer_s", Json::Crudo(format!("{:.1}", h.leer_s))),
        (
            "tippecanoe_s",
            Json::Crudo(format!("{:.1}", h.tippecanoe_s)),
        ),
        ("subir_s", Json::Crudo(format!("{subir_s:.1}"))),
        ("orden", Json::s(h.orden.clone())),
    ])
}

/// Sube el PMTiles (en flujo, cotejado) y su manifiesto (`<clave>.json`). `Ok(false)` si ya
/// estaba: el nombre lleva la instantánea y la huella, así que es el mismo.
pub fn subir(destino: &Arc<dyn Almacen>, clave: &str, fichero: &Path) -> Result<bool, String> {
    subir_como(destino, clave, fichero, "application/vnd.pmtiles")
}

/// Sube un fichero local en flujo, cotejado (sha256 + crc32c), con su tipo. `Ok(false)`: ya estaba.
pub fn subir_como(
    destino: &Arc<dyn Almacen>,
    clave: &str,
    fichero: &Path,
    tipo: &str,
) -> Result<bool, String> {
    let mut sha = sha2::Sha256::new();
    let mut crc = ore_store::blobs::Crc32c::default();
    let mut f = std::fs::File::open(fichero).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    let mut tamano = 0u64;
    loop {
        let k = f.read(&mut buf).map_err(|e| e.to_string())?;
        if k == 0 {
            break;
        }
        sha.update(&buf[..k]);
        crc.sumar(&buf[..k]);
        tamano += k as u64;
    }
    destino.poner_blob(&Blob {
        clave: clave.to_string(),
        tipo: tipo.into(),
        tamano,
        sha256: sha.finalize().into(),
        crc32c: crc.valor(),
        cuerpo: Cuerpo::Fichero(fichero.to_path_buf()),
    })
}

// ── La retención (F2·3·2·6) ───────────────────────────────────────────────────────────────────────

/// Lo que cuelga de una pirámide: ella, su manifiesto y su fallo.
const DE_UNA_PIRAMIDE: [&str; 3] = [".pmtiles", ".pmtiles.json", ".pmtiles.fallo.json"];

/// El número de la instantánea de una pirámide (`…/00012-<uuid>-r1.pmtiles` → 12): el de su
/// `metadata.json`, que Iceberg sube de uno en uno.
fn secuencia(base: &str) -> Option<u64> {
    let nombre = base.rsplit('/').next()?;
    nombre.split('-').next()?.parse().ok()
}

/// **Retira las pirámides viejas** de la tabla de `clave` (la recién subida): se quedan ella, las
/// de instantáneas MÁS NUEVAS (las de otra rama, o una que ganó la carrera) y la anterior a ella
/// (la que se servía hasta ahora, por si alguien la tiene abierta); las demás se borran, con su
/// manifiesto y su fallo. Devuelve las bases borradas. Sin número en el nombre, no se toca nada.
pub fn retirar_viejas(destino: &Arc<dyn Almacen>, clave: &str) -> Result<Vec<String>, String> {
    let Some(nueva) = clave.strip_suffix(".pmtiles") else {
        return Ok(vec![]);
    };
    let (Some(n), Some((dir, _))) = (secuencia(nueva), nueva.rsplit_once('/')) else {
        return Ok(vec![]);
    };
    let mut bases: Vec<(u64, String)> = destino
        .listar(&format!("{dir}/"))?
        .into_iter()
        .filter_map(|k| {
            let b = DE_UNA_PIRAMIDE
                .iter()
                .find_map(|s| k.strip_suffix(s))?
                .to_string();
            Some((secuencia(&b)?, b))
        })
        .collect();
    bases.sort();
    bases.dedup();
    let anterior = bases
        .iter()
        .filter(|(s, b)| *s < n && b != nueva)
        .map(|(s, _)| *s)
        .max();
    let mut borradas = vec![];
    for (s, b) in bases {
        if b == nueva || s > n || Some(s) == anterior {
            continue;
        }
        for suf in DE_UNA_PIRAMIDE {
            destino.borrar(&format!("{b}{suf}"))?;
        }
        borradas.push(b);
    }
    Ok(borradas)
}

/// La versión de tippecanoe (para el manifiesto).
pub fn version_tippecanoe(bin: &str) -> String {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| {
            String::from_utf8_lossy(if o.stdout.is_empty() {
                &o.stderr
            } else {
                &o.stdout
            })
            .trim()
            .to_string()
        })
        .unwrap_or_else(|e| format!("? ({e})"))
}

#[cfg(test)]
mod pruebas;
