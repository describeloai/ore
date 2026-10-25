//! **El lector por oleadas** (ADR 0072 F2·3, D-F2-8 a D-F2-10). Medido en el banco de
//! victor (2026-10-25): cada tesela esperaba ~40 GET a GCS **en serie** (~55 ms cada uno)
//! porque el lector de Parquet pedía cada trozo de columna y cada página de uno en uno. Aquí
//! la tesela calcula antes qué páginas necesita (con el índice de páginas, en memoria) y las
//! trae **juntas, en paralelo**, con los huecos pequeños fundidos: una oleada por fase.
//!
//! - **La caché de páginas** (D-F2-10): las páginas de un fichero inmutable, por contenido
//!   (fichero + rango), en RAM con tope; la que se usó hace más se va primero.
//! - **Un rango en camino no se pide dos veces** (D-F2-9): las teselas de una vista comparten
//!   sus grupos; la que llega segunda espera a la primera.
//! - **El tope de GET a la vez** del proceso: la espera de GCS no ocupa turno de CPU, pero
//!   tampoco abre conexiones sin fin.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bytes::{Buf, Bytes};
use ore_store::almacen::Almacen;
use parquet::errors::ParquetError;
use parquet::file::reader::{ChunkReader, Length};

use crate::{Cuenta, Fallo, Turnos};

/// Dos rangos a menos de esto se piden como uno (lo que cuesta un GET frente a 64 KB de más).
const HUECO: u64 = 64 << 10;
/// Ni un GET de más de esto: lo grande se reparte para ir en paralelo.
const MAXIMO: u64 = 8 << 20;
/// La cola que se trae para leer un pie que no está en caché.
pub const COLA: u64 = 64 << 10;

/// Un trozo de fichero: su clave en el almacén y su rango de bytes.
type Clave = (String, u64, u64);

/// Lo que espera quien llega a un rango que otra tesela ya está trayendo.
struct Vuelo {
    hecho: Mutex<Option<Result<Bytes, String>>>,
    cv: Condvar,
}

struct Guardadas {
    m: HashMap<Clave, (Bytes, u64)>,
    total: u64,
    reloj: u64,
}

pub struct Paginas {
    guardadas: Mutex<Guardadas>,
    vuelos: Mutex<HashMap<Clave, Arc<Vuelo>>>,
    gets: Turnos,
    tope: u64,
}

impl Paginas {
    pub fn nuevas(tope: u64, gets: usize) -> Paginas {
        Paginas {
            guardadas: Mutex::new(Guardadas {
                m: HashMap::new(),
                total: 0,
                reloj: 0,
            }),
            vuelos: Mutex::new(HashMap::new()),
            gets: Turnos::nuevos(gets),
            tope,
        }
    }

    /// `ORE_TESELAS_CACHE_MB` (1024: la mitad de los 2 GiB del pod) y `ORE_TESELAS_GETS` (32).
    pub fn de_la_maquina() -> Paginas {
        let n = |k: &str, d: u64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        Paginas::nuevas(
            n("ORE_TESELAS_CACHE_MB", 1024) << 20,
            n("ORE_TESELAS_GETS", 32) as usize,
        )
    }

    /// Los bytes guardados ahora (las pruebas).
    pub fn guardado(&self) -> u64 {
        self.guardadas.lock().unwrap().total
    }

    fn sacar(&self, k: &Clave) -> Option<Bytes> {
        let mut g = self.guardadas.lock().unwrap();
        g.reloj += 1;
        let reloj = g.reloj;
        g.m.get_mut(k).map(|(b, t)| {
            *t = reloj;
            b.clone()
        })
    }

    fn guardar(&self, k: Clave, b: Bytes) {
        let mut g = self.guardadas.lock().unwrap();
        g.reloj += 1;
        let (reloj, largo) = (g.reloj, b.len() as u64);
        if largo > self.tope {
            return;
        }
        if let Some((viejo, _)) = g.m.insert(k, (b, reloj)) {
            g.total -= viejo.len() as u64;
        }
        g.total += largo;
        if g.total > self.tope {
            // fuera lo usado hace más, hasta el 90 % del tope
            let mut por_edad: Vec<(u64, Clave, u64)> =
                g.m.iter()
                    .map(|(k, (b, t))| (*t, k.clone(), b.len() as u64))
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

    /// **Una oleada**: trae los rangos pedidos de cada fichero —los que no están en caché ni
    /// en camino, fundidos y en paralelo— y devuelve, por fichero, sus trozos por inicio.
    pub fn traer(
        &self,
        cuenta: &Arc<dyn Almacen>,
        pedidas: Vec<(String, Range<u64>)>,
        fin: Instant,
        cu: &mut Cuenta,
    ) -> Result<HashMap<String, BTreeMap<u64, Bytes>>, Fallo> {
        let mut claves: Vec<Clave> = pedidas
            .into_iter()
            .filter(|(_, r)| r.end > r.start)
            .map(|(f, r)| (f, r.start, r.end))
            .collect();
        claves.sort();
        claves.dedup();
        let mut out: HashMap<String, BTreeMap<u64, Bytes>> = HashMap::new();
        let (mut mias, mut ajenas) = (vec![], vec![]);
        for k in claves {
            if let Some(b) = self.sacar(&k) {
                cu.en_cache += 1;
                out.entry(k.0.clone()).or_default().insert(k.1, b);
                continue;
            }
            let mut v = self.vuelos.lock().unwrap();
            match v.get(&k) {
                Some(vuelo) => ajenas.push((k, vuelo.clone())),
                None => {
                    v.insert(
                        k.clone(),
                        Arc::new(Vuelo {
                            hecho: Mutex::new(None),
                            cv: Condvar::new(),
                        }),
                    );
                    mias.push(k);
                }
            }
        }

        // las mías, fundidas: (fichero, desde, hasta, las claves que cubre)
        let mut lotes: Vec<(String, u64, u64, Vec<Clave>)> = vec![];
        for k in mias {
            match lotes.last_mut() {
                Some(l) if l.0 == k.0 && k.1 <= l.2 + HUECO && k.2 - l.1 <= MAXIMO => {
                    l.2 = l.2.max(k.2);
                    l.3.push(k);
                }
                _ => lotes.push((k.0.clone(), k.1, k.2, vec![k])),
            }
        }
        if !lotes.is_empty() {
            cu.oleadas += 1;
            cu.peticiones += lotes.len();
        }
        let traidos: Vec<Result<Vec<u8>, String>> = std::thread::scope(|s| {
            let hilos: Vec<_> = lotes
                .iter()
                .map(|(f, a, z, _)| {
                    s.spawn(move || {
                        let espera = fin.saturating_duration_since(Instant::now());
                        let Some(_get) = self.gets.entrar(espera) else {
                            return Err("la tesela pasó su plazo esperando a GCS".to_string());
                        };
                        let b = cuenta
                            .leer_rango(f, Some((*a, z - 1)))?
                            .ok_or_else(|| format!("`{f}` ya no está en el almacén"))?;
                        if b.len() as u64 != z - a {
                            return Err(format!(
                                "`{f}`: se pidieron {} bytes desde {a} y llegaron {}",
                                z - a,
                                b.len()
                            ));
                        }
                        Ok(b)
                    })
                })
                .collect();
            hilos
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|_| Err("el GET se cayó".into())))
                .collect()
        });
        let mut fallo = None;
        for ((f, a, _, ks), r) in lotes.into_iter().zip(traidos) {
            let r = r.map(Bytes::from);
            if let Ok(b) = &r {
                cu.bytes += b.len();
            }
            for k in ks {
                let trozo = r
                    .as_ref()
                    .map(|b| b.slice((k.1 - a) as usize..(k.2 - a) as usize))
                    .map_err(|e| e.clone());
                if let Ok(t) = &trozo {
                    self.guardar(k.clone(), t.clone());
                    out.entry(f.clone()).or_default().insert(k.1, t.clone());
                }
                if let Err(e) = &trozo {
                    fallo.get_or_insert_with(|| e.clone());
                }
                if let Some(v) = self.vuelos.lock().unwrap().remove(&k) {
                    *v.hecho.lock().unwrap() = Some(trozo);
                    v.cv.notify_all();
                }
            }
        }
        if let Some(e) = fallo {
            return Err(if e.contains("plazo") {
                Fallo::Plazo(e)
            } else {
                Fallo::Interno(e)
            });
        }

        // las que trae otra tesela
        cu.compartidas += ajenas.len();
        for (k, v) in ajenas {
            let mut h = v.hecho.lock().unwrap();
            while h.is_none() {
                let ahora = Instant::now();
                if ahora >= fin {
                    return Err(Fallo::Plazo(
                        "la tesela pasó su plazo esperando a GCS".into(),
                    ));
                }
                h = v.cv.wait_timeout(h, fin - ahora).unwrap().0;
            }
            match h.as_ref().unwrap() {
                Ok(b) => {
                    out.entry(k.0.clone()).or_default().insert(k.1, b.clone());
                }
                Err(e) => return Err(Fallo::Interno(e.clone())),
            }
        }
        Ok(out)
    }
}

/// **Un fichero para el lector de Parquet sobre lo ya traído**: lo que no está (lo que la
/// cuenta previa no vio) se pide aparte y cuenta en `peticiones`, para que el banco lo vea.
pub struct Trozos {
    pub cuenta: Arc<dyn Almacen>,
    pub clave: String,
    pub largo: u64,
    pub trozos: Arc<BTreeMap<u64, Bytes>>,
    /// Las peticiones y los bytes de lo que hubo que pedir aparte.
    pub aparte: Arc<(AtomicU64, AtomicU64)>,
}

impl Trozos {
    fn de_lo_traido(&self, start: u64, n: u64) -> Option<Bytes> {
        let (a, b) = self.trozos.range(..=start).next_back()?;
        let z = a + b.len() as u64;
        (start + n <= z).then(|| b.slice((start - a) as usize..(start - a + n) as usize))
    }

    fn pedir(&self, start: u64, n: u64) -> parquet::errors::Result<Bytes> {
        let b = self
            .cuenta
            .leer_rango(&self.clave, Some((start, start + n - 1)))
            .map_err(ParquetError::General)?
            .ok_or_else(|| {
                ParquetError::General(format!("`{}` ya no está en el almacén", self.clave))
            })?;
        if b.len() as u64 != n {
            return Err(ParquetError::General(format!(
                "`{}`: se pidieron {n} bytes desde {start} y llegaron {}",
                self.clave,
                b.len()
            )));
        }
        self.aparte.0.fetch_add(1, Ordering::Relaxed);
        self.aparte.1.fetch_add(n, Ordering::Relaxed);
        Ok(Bytes::from(b))
    }
}

impl Length for Trozos {
    fn len(&self) -> u64 {
        self.largo
    }
}

impl ChunkReader for Trozos {
    type T = bytes::buf::Reader<Bytes>;

    /// Sin índice de offsets, el lector lee de corrido desde `start`: hasta el final del
    /// trozo traído que lo contiene (el trozo de columna entero).
    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        if let Some((a, b)) = self.trozos.range(..=start).next_back() {
            let z = a + b.len() as u64;
            if start < z {
                return Ok(b.slice((start - a) as usize..).reader());
            }
        }
        let n = self.largo.saturating_sub(start).min(MAXIMO);
        Ok(self.pedir(start, n)?.reader())
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        match self.de_lo_traido(start, length as u64) {
            Some(b) => Ok(b),
            None => self.pedir(start, length as u64),
        }
    }
}

/// Lo que se pidió aparte, sumado a la cuenta de la tesela.
pub fn sumar_aparte(aparte: &(AtomicU64, AtomicU64), cu: &mut Cuenta) {
    cu.peticiones += aparte.0.load(Ordering::Relaxed) as usize;
    cu.bytes += aparte.1.load(Ordering::Relaxed) as usize;
}

/// Cuánto queda hasta `fin`.
pub fn queda(fin: Instant) -> Duration {
    fin.saturating_duration_since(Instant::now())
}
