//! **El almacén de blobs de un inquilino** (0046 E8·2): los bytes de sus
//! colecciones, uno por contenido.
//!
//! Un blob vive en `ore/v2/blobs/sha256/<hex>`: su nombre **es** su contenido,
//! así que es inmutable y dos colecciones —o dos bases— que guardan el mismo
//! fichero lo guardan una vez. El alcance es el inquilino y no más: su lago
//! está cifrado con su clave (medido en E8·2 A1, `keyRings/ore/cryptoKeys/<i>`).
//!
//! Lo medido que decide la forma (E8·2 A y B, GCS real):
//!
//! - **el cotejo va por delante**: la subida `media` de GCS ignora el hash que
//!   se le manda; la multiparte y la reanudable, con el `crc32c` en los
//!   metadatos, dan 400 y el objeto no llega a existir. R2 coteja el
//!   `ChecksumSHA256` igual;
//! - **subir lo que ya estaba cuesta subirlo entero** (el 412 llega al final):
//!   quien llama pregunta antes, por la huella del origen (`blobs-hay`);
//! - **lo pequeño en memoria, lo grande a disco**: por debajo de 8 MiB (casi
//!   todo PDF y foto), una petición; por encima, un temporal y la subida
//!   reanudable en trozos, con la memoria constante;
//! - **en paralelo**: 64 subidas con conexión viva, 589 blobs de 20 KiB/s en el
//!   clúster; hashear (sha256 1,5 GB/s con SHA-NI) no es el cuello.
//!
//! # El índice de huellas
//!
//! Un ítem llega del origen con **su** huella (el CRC64NVME de S3), que se
//! conoce sin bajarlo; el sha256 sólo se sabe bajándolo. Para no bajar lo que
//! el lago ya tiene —un reintento tras un Job cortado, el mismo contrato en
//! otra colección— cada blob deja un objeto vacío
//! `ore/v2/blobs/huellas/<sha256(huella + tamaño)>/<sha256 del blob>`: el
//! nombre lo dice todo, así que preguntar es listar un prefijo y recogerlo
//! es listar, sin leer ninguno. Se escribe **después** del blob, así que
//! nunca apunta a uno que no llegó. La spec lo respalda: *«dos ítems con la
//! misma huella son el mismo contenido»* (`v1alpha16/02` §5).
//!
//! # La recogida, y la carrera con un Job (E8·3b)
//!
//! Un blob se recoge cuando **ninguna fila de ningún manifiesto del inquilino
//! lo nombra y nadie lo ha tocado en la gracia** (2 h por defecto: más que el
//! plazo de un Job). Un Job que reutiliza un blob que no subió él —por el
//! índice, o de una fila retirada— lo **toca** antes de sellar (`tocar`: un
//! metadato en GCS, una copia sobre sí mismo en S3), así que la recogida no se
//! lo lleva entre que el Job lo encuentra y lo nombra. Detrás, el borrado suave
//! de GCS (7 días) deja deshacer un error.
//!
//! # Los verbos
//!
//! | verbo | entrada | salida |
//! |---|---|---|
//! | `blobs` | la petición en una línea (`hilos`, `temporal`) y después el flujo de tramas de un lector (`ore_driver::tramas`) | una línea por ítem (`blob`, `subido`, o `error`) y el resumen |
//! | `blobs-hay` | `{huellas: [[huella, tamaño], …]}` | `{hay: [[huella, tamaño, sha256], …]}`: los que ya están, blob incluido |
//! | `blobs-tocar` | `{blobs: [sha256, …]}` | cuáles se tocaron y cuáles faltan (E8·3b) |
//! | `blobs-recoger` | `{vivos: [sha256, …], gracia_ms?, seco?}` | lo recogido: blobs que nadie nombra ni tocó en la gracia, y su índice (E8·3b) |
//! | `blobs-cotejar` | `{blobs: [[sha256, tamaño], …], muestra?}` | cuáles están y miden lo suyo, y cuáles —de una muestra— siguen siendo su contenido |
//! | `blob-leer` | `{blob, archivo, rango?}` | los bytes en `archivo`, y su sha256 |
//! | `blob-firmar` | `{firmas: [{blob, tipo?, disposicion?}, …], segundos?}` | `{segundos, caduca_ms, firmadas: [{blob, url}, …]}`: una URL de lectura por blob, con el tipo y la disposición firmados (0046 E9·2) |

use crate::almacen::{Almacen, Blob, Cuerpo};
use ore_core::json::Json;
use ore_driver::tramas::{self, Trama};
use sha2::Digest;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, mpsc};

pub const RAIZ: &str = "ore/v2/blobs";
/// Por debajo, en memoria y en una petición; por encima, a disco.
pub const EN_MEMORIA: u64 = 8 << 20;
const HILOS: usize = 32;
const TIPO_POR_DEFECTO: &str = "application/octet-stream";

pub fn clave_de(sha256: &str) -> String {
    format!("{RAIZ}/sha256/{sha256}")
}

/// El prefijo del índice de lo que el origen llama `huella` con ese tamaño:
/// debajo, un objeto vacío por blob, con su sha256 por nombre.
pub fn prefijo_de_huella(huella: &str, tamano: u64) -> String {
    format!(
        "{RAIZ}/huellas/{}/",
        ore_s3::hex(&ore_s3::sha256(format!("{huella}\t{tamano}").as_bytes()))
    )
}

/// ISO-8601 de un almacén (`2026-09-29T16:27:15.682Z`) a milisegundos.
pub fn ms_de_iso(s: &str) -> Option<i64> {
    let (fecha, hora) = s.trim().trim_end_matches('Z').split_once('T')?;
    let mut f = fecha.splitn(3, '-').map(|x| x.parse::<i64>().ok());
    let (y, m, d) = (f.next()??, f.next()??, f.next()??);
    let (hms, frac) = hora.split_once('.').unwrap_or((hora, "0"));
    let mut h = hms.splitn(3, ':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (h.next()??, h.next()??, h.next()??);
    let ms: i64 = format!("{frac:0<3}")[..3].parse().ok()?;
    // días desde 1970 (el algoritmo civil de Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let dias = era * 146_097 + doe - 719_468;
    Some(((dias * 24 + hh) * 60 + mm) * 60_000 + ss * 1000 + ms)
}

const TABLA: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0x82F6_3B78 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// CRC-32C (Castagnoli) en flujo: el que GCS coteja.
pub struct Crc32c(u32);

impl Default for Crc32c {
    fn default() -> Self {
        Crc32c(!0)
    }
}

impl Crc32c {
    pub fn sumar(&mut self, datos: &[u8]) {
        let mut c = self.0;
        for &b in datos {
            c = TABLA[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }
    pub fn valor(&self) -> u32 {
        !self.0
    }
}

/// Lo que se sabe de un ítem que llegó con sus bytes.
struct Llegado {
    cab: tramas::Cabecera,
    blob: Blob,
    sha: String,
}

/// Lee `tamano` bytes del flujo hashéandolos, a memoria o a un temporal.
fn recibir(
    r: &mut dyn BufRead,
    cab: &tramas::Cabecera,
    temporal: &std::path::Path,
) -> Result<(Cuerpo, [u8; 32], u32), String> {
    let mut sha = sha2::Sha256::new();
    let mut crc = Crc32c::default();
    let mut resto = cab.tamano;
    let mut buf = vec![0u8; 1 << 16];
    let mut memoria = Vec::new();
    let mut fichero: Option<(std::path::PathBuf, std::io::BufWriter<std::fs::File>)> = None;
    if cab.tamano >= EN_MEMORIA {
        let p = temporal.join(format!("ore-blob-{}", uuid::Uuid::new_v4()));
        let f = std::fs::File::create(&p)
            .map_err(|e| format!("no se pudo crear el temporal `{}`: {e}", p.display()))?;
        fichero = Some((p, std::io::BufWriter::with_capacity(1 << 20, f)));
    } else {
        memoria.reserve(cab.tamano as usize);
    }
    while resto > 0 {
        let hasta = resto.min(buf.len() as u64) as usize;
        let k = r
            .read(&mut buf[..hasta])
            .map_err(|e| format!("el flujo de `{}` se cortó: {e}", cab.clave))?;
        if k == 0 {
            if let Some((p, _)) = fichero {
                let _ = std::fs::remove_file(p);
            }
            return Err(format!(
                "el flujo acabó a mitad de `{}`: faltan {resto} de {} bytes",
                cab.clave, cab.tamano
            ));
        }
        sha.update(&buf[..k]);
        crc.sumar(&buf[..k]);
        match &mut fichero {
            Some((_, w)) => w
                .write_all(&buf[..k])
                .map_err(|e| format!("el temporal de `{}` no se pudo escribir: {e}", cab.clave))?,
            None => memoria.extend_from_slice(&buf[..k]),
        }
        resto -= k as u64;
    }
    let cuerpo = match fichero {
        Some((p, mut w)) => {
            w.flush()
                .map_err(|e| format!("el temporal de `{}` no se pudo escribir: {e}", cab.clave))?;
            Cuerpo::Fichero(p)
        }
        None => Cuerpo::Memoria(memoria),
    };
    Ok((cuerpo, sha.finalize().into(), crc.valor()))
}

fn fallo(clave: &str, version: &str, motivo: &str) -> Json {
    Json::obj([
        ("clave", Json::s(clave)),
        ("version", Json::s(version)),
        ("error", Json::s(motivo)),
    ])
}

/// Sube un ítem llegado y, si subió o ya estaba, deja su huella en el índice.
fn subir_uno(cuenta: &dyn Almacen, l: Llegado) -> (Json, Option<bool>, u64) {
    let r = cuenta.poner_blob(&l.blob).and_then(|subido| {
        if !l.cab.huella.is_empty() {
            cuenta.subir(
                &format!(
                    "{}{}",
                    prefijo_de_huella(&l.cab.huella, l.cab.tamano),
                    l.sha
                ),
                b"",
            )?;
        }
        Ok(subido)
    });
    if let Cuerpo::Fichero(p) = &l.blob.cuerpo {
        let _ = std::fs::remove_file(p);
    }
    match r {
        Ok(subido) => (
            Json::obj([
                ("clave", Json::s(&l.cab.clave)),
                ("version", Json::s(&l.cab.version)),
                ("huella", Json::s(&l.cab.huella)),
                ("blob", Json::s(&l.sha)),
                ("tamano", Json::Int(l.cab.tamano as i64)),
                ("tipo", Json::s(&l.blob.tipo)),
                ("subido", Json::Bool(subido)),
            ]),
            Some(subido),
            if subido { l.cab.tamano } else { 0 },
        ),
        Err(e) => (fallo(&l.cab.clave, &l.cab.version, &e), None, 0),
    }
}

/// **`blobs`**: el flujo de tramas de un lector, al almacén. Una línea por
/// ítem en cuanto se sabe, y el resumen al final.
pub fn poner(
    cuenta: Arc<dyn Almacen>,
    n: &ore_core::parse::Node,
    mut lector: impl BufRead,
) -> Result<String, String> {
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let hilos = campo("hilos")
        .and_then(|h| h.parse::<usize>().ok())
        .unwrap_or(HILOS)
        .clamp(1, 128);
    let temporal = campo("temporal")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    let (tx_trabajo, rx_trabajo) = mpsc::sync_channel::<Llegado>(hilos);
    let rx_trabajo = Arc::new(Mutex::new(rx_trabajo));
    let (tx_linea, rx_linea) = mpsc::channel::<(Json, Option<bool>, u64)>();

    // Quien escribe: una línea por ítem, y las cuentas.
    let escritor = std::thread::spawn(move || {
        let salida = std::io::stdout();
        let mut s = salida.lock();
        let (mut items, mut subidos, mut ya, mut errores, mut bytes) =
            (0i64, 0i64, 0i64, 0i64, 0u64);
        for (j, r, b) in rx_linea {
            items += 1;
            match r {
                Some(true) => subidos += 1,
                Some(false) => ya += 1,
                None => errores += 1,
            }
            bytes += b;
            let _ = writeln!(s, "{}", j.jcs());
        }
        let _ = s.flush();
        (items, subidos, ya, errores, bytes)
    });
    let obreros: Vec<_> = (0..hilos)
        .map(|_| {
            let rx = rx_trabajo.clone();
            let tx = tx_linea.clone();
            let c = cuenta.clone();
            std::thread::spawn(move || {
                loop {
                    let Ok(l) = rx
                        .lock()
                        .map_err(|_| ())
                        .and_then(|r| r.recv().map_err(|_| ()))
                    else {
                        return;
                    };
                    let _ = tx.send(subir_uno(c.as_ref(), l));
                }
            })
        })
        .collect();

    let mut roto: Option<String> = None;
    loop {
        let t = match tramas::leer_trama(&mut lector) {
            Ok(Some(t)) => t,
            Ok(None) => break,
            Err(e) => {
                roto = Some(e);
                break;
            }
        };
        match t {
            Trama::Fallo {
                clave,
                version,
                motivo,
            } => {
                let _ = tx_linea.send((fallo(&clave, &version, &motivo), None, 0));
            }
            Trama::Bytes(cab) => {
                let (cuerpo, sha256, crc32c) = match recibir(&mut lector, &cab, &temporal) {
                    Ok(x) => x,
                    Err(e) => {
                        roto = Some(e);
                        break;
                    }
                };
                match tramas::leer_cierre(&mut lector) {
                    Err(e) => {
                        roto = Some(e);
                        break;
                    }
                    // El lector no da los bytes por buenos: no se guardan.
                    Ok(Err(motivo)) => {
                        if let Cuerpo::Fichero(p) = &cuerpo {
                            let _ = std::fs::remove_file(p);
                        }
                        let _ = tx_linea.send((fallo(&cab.clave, &cab.version, &motivo), None, 0));
                    }
                    Ok(Ok(())) => {
                        let sha = ore_s3::hex(&sha256);
                        let tipo = if cab.tipo.is_empty() {
                            TIPO_POR_DEFECTO.to_string()
                        } else {
                            cab.tipo.clone()
                        };
                        let blob = Blob {
                            clave: clave_de(&sha),
                            tipo,
                            tamano: cab.tamano,
                            sha256,
                            crc32c,
                            cuerpo,
                        };
                        if tx_trabajo.send(Llegado { cab, blob, sha }).is_err() {
                            roto = Some("los que suben se fueron".into());
                            break;
                        }
                    }
                }
            }
        }
    }
    drop(tx_trabajo);
    for o in obreros {
        let _ = o.join();
    }
    drop(tx_linea);
    let (items, subidos, ya, errores, bytes) = escritor
        .join()
        .map_err(|_| "el que escribe las líneas se cayó".to_string())?;
    if let Some(e) = roto {
        return Err(format!(
            "{e} · {items} ítems contestados antes de cortarse ({subidos} subidos)"
        ));
    }
    Ok(Json::obj([(
        "blobs",
        Json::obj([
            ("items", Json::Int(items)),
            ("subidos", Json::Int(subidos)),
            ("ya_estaban", Json::Int(ya)),
            ("errores", Json::Int(errores)),
            ("bytes_subidos", Json::Int(bytes as i64)),
        ]),
    )])
    .jcs())
}

/// **`blobs-hay`**: de lo que el origen llama con su huella y su tamaño, lo
/// que el lago ya tiene, con su blob. El índice dice el sha256, y el blob se
/// **toca**: si sigue, la recogida no se lo lleva mientras el Job lo nombra.
pub fn hay(cuenta: Arc<dyn Almacen>, n: &ore_core::parse::Node) -> Result<String, String> {
    let pares: Vec<(String, u64)> = n
        .get("huellas")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| match x.items() {
            [h, t] => Some((h.as_str()?.to_string(), t.as_str()?.parse().ok()?)),
            _ => None,
        })
        .collect();
    let hilos = n
        .get("hilos")
        .and_then(|(_, v)| v.as_str()?.parse::<usize>().ok())
        .unwrap_or(HILOS)
        .clamp(1, 128);
    let encontrados: Mutex<Vec<(usize, Json)>> = Mutex::new(Vec::new());
    let error: Mutex<Option<String>> = Mutex::new(None);
    let siguiente = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..hilos.min(pares.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = siguiente.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((h, t)) = pares.get(i) else { return };
                    let pre = prefijo_de_huella(h, *t);
                    let r = cuenta.listar(&pre).and_then(|ks| {
                        for k in ks {
                            let sha = &k[pre.len()..];
                            if sha.len() == 64 && cuenta.tocar(&clave_de(sha))? {
                                return Ok(Some(sha.to_string()));
                            }
                        }
                        Ok(None)
                    });
                    match r {
                        Ok(Some(sha)) => encontrados.lock().unwrap().push((
                            i,
                            Json::Arr(vec![Json::s(h), Json::Int(*t as i64), Json::s(sha)]),
                        )),
                        Ok(None) => {}
                        Err(e) => {
                            error.lock().unwrap().get_or_insert(e);
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }
    let mut e = encontrados.into_inner().unwrap();
    e.sort_by_key(|(i, _)| *i);
    Ok(Json::obj([("hay", Json::Arr(e.into_iter().map(|(_, j)| j).collect()))]).jcs())
}

/// **`blobs-tocar`**: marca como vistos los blobs que un Job reutiliza de
/// filas que no son actuales (E8·3b). `{blobs: [sha256, …]}` →
/// `{tocados, faltan: […]}`.
pub fn tocar(cuenta: Arc<dyn Almacen>, n: &ore_core::parse::Node) -> Result<String, String> {
    let shas: Vec<String> = n
        .get("blobs")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let faltan: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let error: Mutex<Option<String>> = Mutex::new(None);
    let siguiente = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..HILOS.min(shas.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = siguiente.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(sha) = shas.get(i) else { return };
                    match cuenta.tocar(&clave_de(sha)) {
                        Ok(true) => {}
                        Ok(false) => faltan.lock().unwrap().push(sha.clone()),
                        Err(e) => {
                            error.lock().unwrap().get_or_insert(e);
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }
    let mut faltan = faltan.into_inner().unwrap();
    faltan.sort();
    Ok(Json::obj([
        ("tocados", Json::Int((shas.len() - faltan.len()) as i64)),
        ("faltan", Json::Arr(faltan.iter().map(Json::s).collect())),
    ])
    .jcs())
}

/// Dos horas: más que el plazo de un Job de la copia (una hora).
pub const GRACIA_MS: i64 = 2 * 3600 * 1000;

/// **`blobs-recoger`** (E8·3b): los blobs que ninguna fila nombra y nadie tocó
/// en la gracia se van, y con ellos las entradas del índice que ya no
/// apuntan a nada. `{vivos: [sha256, …], gracia_ms?, seco?, ahora_ms?}`: los
/// vivos son **todos** los que nombran los manifiestos del inquilino —quien
/// llama responde de que la lista esté entera—.
pub fn recoger(cuenta: Arc<dyn Almacen>, n: &ore_core::parse::Node) -> Result<String, String> {
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let vivos: std::collections::BTreeSet<String> = n
        .get("vivos")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let gracia = campo("gracia_ms")
        .and_then(|g| g.parse::<i64>().ok())
        .unwrap_or(GRACIA_MS);
    let seco = campo("seco").as_deref() == Some("true");
    let ahora = campo("ahora_ms")
        .and_then(|a| a.parse::<i64>().ok())
        .unwrap_or_else(crate::lago::ahora_ms);
    let pre = format!("{RAIZ}/sha256/");
    let hay = cuenta.listar_con_fecha(&pre)?;
    let (mut en_gracia, mut bytes) = (0i64, 0u64);
    let mut quedan: std::collections::BTreeSet<String> = Default::default();
    let mut muertos: Vec<String> = Vec::new();
    for o in &hay {
        let sha = o.clave[pre.len()..].to_string();
        if vivos.contains(&sha) {
            quedan.insert(sha);
        } else if o.tocado_ms.saturating_add(gracia) > ahora {
            en_gracia += 1;
            quedan.insert(sha);
        } else {
            bytes += o.tamano;
            muertos.push(o.clave.clone());
        }
    }
    let borrar = |claves: &[String]| -> Result<(), String> {
        if seco {
            return Ok(());
        }
        let error: Mutex<Option<String>> = Mutex::new(None);
        let siguiente = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..HILOS.min(claves.len().max(1)) {
                s.spawn(|| {
                    loop {
                        let i = siguiente.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(k) = claves.get(i) else { return };
                        if let Err(e) = cuenta.borrar(k) {
                            error.lock().unwrap().get_or_insert(e);
                        }
                    }
                });
            }
        });
        error.into_inner().unwrap().map_or(Ok(()), Err)
    };
    borrar(&muertos)?;
    // El índice: lo que apunta a un blob que ya no está.
    let hpre = format!("{RAIZ}/huellas/");
    let huellas: Vec<String> = cuenta
        .listar(&hpre)?
        .into_iter()
        .filter(|k| {
            let sha = k.rsplit('/').next().unwrap_or("");
            !quedan.contains(sha)
        })
        .collect();
    borrar(&huellas)?;
    Ok(Json::obj([
        ("blobs", Json::Int(hay.len() as i64)),
        ("vivos", Json::Int(vivos.len() as i64)),
        ("en_gracia", Json::Int(en_gracia)),
        ("recogidos", Json::Int(muertos.len() as i64)),
        ("bytes", Json::Int(bytes as i64)),
        ("huellas_recogidas", Json::Int(huellas.len() as i64)),
        ("seco", Json::Bool(seco)),
    ])
    .jcs())
}

/// **`blobs-cotejar`** (E8·2d): que lo que un manifiesto dice esté en el lago.
/// Cada blob, por su tamaño (un `HEAD`: ~600/s en paralelo, medido); y una
/// muestra —los primeros `muestra` en el orden de su sha256, que es un orden
/// al azar y el mismo cada vez— se baja entera y se vuelve a hashear.
/// Entra `{blobs: [[sha256, tamaño], …], muestra?, hilos?}`; sale
/// `{cotejados, bien, releidos, rotos: [{blob, motivo}]}`.
pub fn cotejar(cuenta: Arc<dyn Almacen>, n: &ore_core::parse::Node) -> Result<String, String> {
    let mut blobs: Vec<(String, u64)> = n
        .get("blobs")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| match x.items() {
            [b, t] => Some((b.as_str()?.to_string(), t.as_str()?.parse().ok()?)),
            _ => None,
        })
        .collect();
    blobs.sort();
    blobs.dedup();
    let num = |k: &str, d: usize| {
        n.get(k)
            .and_then(|(_, v)| v.as_str()?.parse::<usize>().ok())
            .unwrap_or(d)
    };
    let muestra = num("muestra", 0);
    let hilos = num("hilos", HILOS).clamp(1, 128);
    let rotos: Mutex<Vec<(usize, Json)>> = Mutex::new(Vec::new());
    let releidos = std::sync::atomic::AtomicUsize::new(0);
    let siguiente = std::sync::atomic::AtomicUsize::new(0);
    let roto = |i: usize, b: &str, m: String| {
        rotos
            .lock()
            .unwrap()
            .push((i, Json::obj([("blob", Json::s(b)), ("motivo", Json::s(m))])));
    };
    std::thread::scope(|s| {
        for _ in 0..hilos.min(blobs.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = siguiente.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((b, t)) = blobs.get(i) else { return };
                    match cuenta.tamano(&clave_de(b)) {
                        Err(e) => roto(i, b, e),
                        Ok(None) => roto(i, b, "no está en el lago".into()),
                        Ok(Some(x)) if x != *t => {
                            roto(i, b, format!("mide {x} bytes y el manifiesto dice {t}"))
                        }
                        Ok(Some(_)) if i < muestra => {
                            releidos.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            match cuenta.leer_rango(&clave_de(b), None) {
                                Ok(Some(bytes)) => {
                                    let h = ore_s3::hex(&ore_s3::sha256(&bytes));
                                    if h != *b {
                                        roto(i, b, format!("sus bytes dan el sha256 {h}"));
                                    }
                                }
                                Ok(None) => roto(i, b, "no está en el lago".into()),
                                Err(e) => roto(i, b, e),
                            }
                        }
                        Ok(Some(_)) => {}
                    }
                }
            });
        }
    });
    let mut rotos = rotos.into_inner().unwrap();
    rotos.sort_by_key(|(i, _)| *i);
    Ok(Json::obj([
        ("cotejados", Json::Int(blobs.len() as i64)),
        ("bien", Json::Int((blobs.len() - rotos.len()) as i64)),
        ("releidos", Json::Int(releidos.into_inner() as i64)),
        (
            "rotos",
            Json::Arr(rotos.into_iter().map(|(_, j)| j).collect()),
        ),
    ])
    .jcs())
}

/// **`blob-leer`**: un blob (o un rango) a un fichero, con el sha256 de lo
/// leído. Es lo que un cotejo necesita para decir que el blob es el suyo.
pub fn leer(cuenta: Arc<dyn Almacen>, n: &ore_core::parse::Node) -> Result<String, String> {
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let sha = campo("blob").ok_or("a `blob-leer` le falta `blob`: su sha256")?;
    let archivo = campo("archivo").ok_or("a `blob-leer` le falta `archivo`: dónde dejarlo")?;
    let rango = n.get("rango").and_then(|(_, r)| match r.items() {
        [a, z] => Some((a.as_str()?.parse().ok()?, z.as_str()?.parse().ok()?)),
        _ => None,
    });
    let b = cuenta
        .leer_rango(&clave_de(&sha), rango)?
        .ok_or_else(|| format!("el blob `{sha}` no está en el lago"))?;
    std::fs::write(&archivo, &b).map_err(|e| format!("no se pudo escribir `{archivo}`: {e}"))?;
    Ok(Json::obj([
        ("bytes", Json::Int(b.len() as i64)),
        ("sha256", Json::s(ore_s3::hex(&ore_s3::sha256(&b)))),
    ])
    .jcs())
}

/// Lo que vive una URL firmada: 5 minutos por defecto; ni menos de 30 s ni
/// más de una hora (una URL es un portador: quien la tenga, lee).
pub const VIDA_POR_DEFECTO: u64 = 300;
const VIDA_MINIMA: u64 = 30;
const VIDA_MAXIMA: u64 = 3600;

/// ¿Es `s` un sha256 en hex? Lo único que se firma son blobs.
fn es_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// **`blob-firmar`** (0046 E9·2): una URL de lectura por blob, con el tipo y la
/// disposición **dentro de la firma**. `{firmas: [{blob, tipo?, disposicion?}],
/// segundos?}` → `{segundos, caduca_ms, firmadas: [{blob, url}]}`. Sólo blobs
/// (`ore/v2/blobs/sha256/<hex>`): nada de este verbo abre otra clave del lago.
/// Quien llama decide **quién** puede; aquí sólo se firma. En paralelo: en GCS
/// cada una es un `signBlob` (58 ms, medido; 32 a la vez, 0,19 s).
pub fn firmar(cuenta: Arc<dyn Almacen>, n: &ore_core::parse::Node) -> Result<String, String> {
    let segundos = n
        .get("segundos")
        .and_then(|(_, v)| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(VIDA_POR_DEFECTO)
        .clamp(VIDA_MINIMA, VIDA_MAXIMA);
    let mut pedidas: Vec<(String, Vec<(&'static str, String)>)> = Vec::new();
    for f in n.get("firmas").map(|(_, v)| v.items()).unwrap_or(&[]) {
        let c = |k: &str| {
            f.get(k)
                .and_then(|(_, v)| v.as_str())
                .filter(|s| !s.is_empty())
        };
        let sha = c("blob").ok_or("a una firma le falta `blob`: su sha256")?;
        if !es_sha256(sha) {
            return Err(format!("`{sha}` no es un sha256: sólo se firman blobs"));
        }
        let mut r = Vec::new();
        if let Some(t) = c("tipo") {
            r.push(("response-content-type", t.to_string()));
        }
        if let Some(d) = c("disposicion") {
            r.push(("response-content-disposition", d.to_string()));
        }
        pedidas.push((sha.to_string(), r));
    }
    let caduca_ms = crate::lago::ahora_ms() + (segundos as i64) * 1000;
    let hechas: Mutex<Vec<Option<Result<String, String>>>> = Mutex::new(vec![None; pedidas.len()]);
    let siguiente = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..HILOS.min(pedidas.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = siguiente.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((sha, r)) = pedidas.get(i) else {
                        return;
                    };
                    let r: Vec<(&str, &str)> = r.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    let u = cuenta.firmar_lectura(&clave_de(sha), segundos, &r);
                    hechas.lock().unwrap()[i] = Some(u);
                }
            });
        }
    });
    let mut firmadas = Vec::new();
    for ((sha, _), u) in pedidas.iter().zip(hechas.into_inner().unwrap()) {
        let url = u.unwrap_or_else(|| Err("sin firmar".into()))?;
        firmadas.push(Json::obj([("blob", Json::s(sha)), ("url", Json::s(url))]));
    }
    Ok(Json::obj([
        ("segundos", Json::Int(segundos as i64)),
        ("caduca_ms", Json::Int(caduca_ms)),
        ("firmadas", Json::Arr(firmadas)),
    ])
    .jcs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Un almacén en memoria que coteja como GCS: un blob cuyo crc32c no es
    /// el de sus bytes no llega a existir.
    #[derive(Default)]
    struct Memoria {
        objetos: Mutex<BTreeMap<String, (Vec<u8>, String)>>,
        /// Cuándo se tocó cada uno; sin entrada, hace mucho.
        tocados: Mutex<BTreeMap<String, i64>>,
    }

    impl Almacen for Memoria {
        fn base(&self) -> String {
            "mem://x".into()
        }
        fn leer(&self, clave: &str) -> Result<Option<String>, String> {
            Ok(self
                .objetos
                .lock()
                .unwrap()
                .get(clave)
                .map(|(b, _)| String::from_utf8_lossy(b).to_string()))
        }
        fn existe(&self, clave: &str) -> Result<bool, String> {
            Ok(self.objetos.lock().unwrap().contains_key(clave))
        }
        fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
            let mut o = self.objetos.lock().unwrap();
            if o.contains_key(clave) {
                return Ok(false);
            }
            o.insert(clave.into(), (cuerpo.to_vec(), String::new()));
            Ok(true)
        }
        fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
            Ok(self
                .objetos
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(prefijo))
                .cloned()
                .collect())
        }
        fn borrar(&self, clave: &str) -> Result<(), String> {
            self.objetos.lock().unwrap().remove(clave);
            Ok(())
        }
        fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self
                .objetos
                .lock()
                .unwrap()
                .get(clave)
                .map(|(b, _)| b.clone()))
        }
        fn listar_con_fecha(&self, prefijo: &str) -> Result<Vec<crate::almacen::Listado>, String> {
            let t = self.tocados.lock().unwrap();
            Ok(self
                .objetos
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, _)| k.starts_with(prefijo))
                .map(|(k, (b, _))| crate::almacen::Listado {
                    clave: k.clone(),
                    tamano: b.len() as u64,
                    tocado_ms: t.get(k).copied().unwrap_or(0),
                })
                .collect())
        }
        fn tocar(&self, clave: &str) -> Result<bool, String> {
            if !self.existe(clave)? {
                return Ok(false);
            }
            self.tocados
                .lock()
                .unwrap()
                .insert(clave.into(), crate::lago::ahora_ms());
            Ok(true)
        }
        fn firmar_lectura(
            &self,
            clave: &str,
            segundos: u64,
            r: &[(&str, &str)],
        ) -> Result<String, String> {
            let extra: String = r.iter().map(|(k, v)| format!("&{k}={v}")).collect();
            Ok(format!("mem://{clave}?s={segundos}{extra}"))
        }
        fn poner_blob(&self, b: &Blob) -> Result<bool, String> {
            let bytes = b.cuerpo.bytes()?;
            let mut c = Crc32c::default();
            c.sumar(&bytes);
            if c.valor() != b.crc32c {
                return Err("400: el crc32c no cuadra".into());
            }
            let mut o = self.objetos.lock().unwrap();
            if o.contains_key(&b.clave) {
                return Ok(false);
            }
            o.insert(b.clave.clone(), (bytes, b.tipo.clone()));
            Ok(true)
        }
    }

    fn trama(w: &mut Vec<u8>, clave: &str, cuerpo: &[u8], cierre: Result<(), &str>) {
        tramas::escribir_cabecera(
            w,
            &tramas::Cabecera {
                clave: clave.into(),
                version: "v1".into(),
                huella: format!("h:{clave}"),
                tipo: "application/pdf".into(),
                tamano: cuerpo.len() as u64,
            },
        )
        .unwrap();
        w.extend_from_slice(cuerpo);
        tramas::escribir_cierre(w, cierre).unwrap();
    }

    fn parse(s: &str) -> ore_core::parse::Node {
        ore_core::parse::parse(s).unwrap()
    }

    #[test]
    fn el_crc32c_es_el_de_castagnoli_en_trozos() {
        let mut c = Crc32c::default();
        c.sumar(b"1234");
        c.sumar(b"56789");
        assert_eq!(c.valor(), 0xe306_9283);
    }

    /// **El flujo de un lector, al lago**: dos ítems con el mismo contenido
    /// son un blob (el segundo, ya estaba); uno que el lector no dio por
    /// bueno no se guarda; uno grande pasa por disco; y todos dejan su huella.
    #[test]
    fn el_flujo_de_un_lector_queda_por_contenido() {
        let m = Arc::new(Memoria::default());
        let grande = vec![7u8; EN_MEMORIA as usize + 10];
        let mut w = Vec::new();
        trama(&mut w, "a.pdf", b"RECIBO", Ok(()));
        trama(&mut w, "d.pdf", b"RECIBO", Ok(()));
        trama(&mut w, "roto.pdf", b"XXXX", Err("la huella no cuadra"));
        tramas::escribir_fallo(&mut w, "b.pdf", "v0", "404 NoSuchVersion").unwrap();
        trama(&mut w, "grande.bin", &grande, Ok(()));
        let r = poner(
            m.clone(),
            &parse(r#"{"hilos":"1"}"#),
            std::io::BufReader::new(&w[..]),
        )
        .unwrap();
        let r = parse(&r);
        let (_, b) = r.get("blobs").unwrap();
        let c = |k: &str| b.get(k).unwrap().1.as_str().unwrap().to_string();
        assert_eq!(
            (c("items"), c("subidos"), c("ya_estaban"), c("errores")),
            ("5".into(), "2".into(), "1".into(), "2".into())
        );
        let sha = ore_s3::hex(&ore_s3::sha256(b"RECIBO"));
        let o = m.objetos.lock().unwrap();
        assert_eq!(o[&clave_de(&sha)].0, b"RECIBO");
        assert_eq!(o[&clave_de(&sha)].1, "application/pdf");
        assert!(
            !o.keys().any(|k| o[k].0 == b"XXXX"),
            "lo que el lector no dio por bueno no se guarda"
        );
        assert!(
            o.contains_key(&format!("{}{sha}", prefijo_de_huella("h:d.pdf", 6))),
            "la huella de cada ítem apunta a su blob"
        );
        let g = ore_s3::hex(&ore_s3::sha256(&grande));
        assert_eq!(o[&clave_de(&g)].0.len(), grande.len());
    }

    /// Un flujo cortado a mitad de un ítem no deja ese ítem, y lo dice.
    #[test]
    fn un_flujo_cortado_se_dice() {
        let m = Arc::new(Memoria::default());
        let mut w = Vec::new();
        trama(&mut w, "a.pdf", b"RECIBO", Ok(()));
        tramas::escribir_cabecera(
            &mut w,
            &tramas::Cabecera {
                clave: "b.pdf".into(),
                version: "v".into(),
                huella: String::new(),
                tipo: String::new(),
                tamano: 100,
            },
        )
        .unwrap();
        w.extend_from_slice(b"poco");
        let e = poner(m.clone(), &parse("{}"), std::io::BufReader::new(&w[..])).unwrap_err();
        assert!(e.contains("faltan 96 de 100"), "{e}");
        assert_eq!(
            m.objetos.lock().unwrap().len(),
            2,
            "a.pdf y su huella, y nada de b"
        );
    }

    /// **La recogida**: lo que un manifiesto nombra se queda; lo que un Job
    /// tocó (lo reutiliza y aún no lo nombra) espera su gracia; lo demás se
    /// va, con su entrada del índice. En seco no se toca nada.
    #[test]
    fn la_recogida_respeta_lo_vivo_y_lo_tocado() {
        let m = Arc::new(Memoria::default());
        let mut w = Vec::new();
        for (c, b) in [
            ("a.pdf", "VIVO"),
            ("b.pdf", "TOCADO"),
            ("c.pdf", "HUERFANO"),
        ] {
            trama(&mut w, c, b.as_bytes(), Ok(()));
        }
        poner(m.clone(), &parse("{}"), std::io::BufReader::new(&w[..])).unwrap();
        let sha = |b: &str| ore_s3::hex(&ore_s3::sha256(b.as_bytes()));
        // Un Job encuentra b.pdf por su huella: `hay` lo toca.
        hay(m.clone(), &parse(r#"{"huellas":[["h:b.pdf","6"]]}"#)).unwrap();
        let pedir = |seco: bool| {
            recoger(
                m.clone(),
                &parse(&format!(
                    r#"{{"vivos":["{}"],"gracia_ms":"3600000","seco":"{seco}"}}"#,
                    sha("VIVO")
                )),
            )
            .unwrap()
        };
        let r = parse(&pedir(true));
        let v =
            |n: &ore_core::parse::Node, k: &str| n.get(k).unwrap().1.as_str().unwrap().to_string();
        assert_eq!(
            (
                v(&r, "recogidos"),
                v(&r, "en_gracia"),
                v(&r, "huellas_recogidas")
            ),
            ("1".into(), "1".into(), "1".into())
        );
        assert_eq!(
            m.objetos.lock().unwrap().len(),
            6,
            "en seco no se borra nada"
        );
        pedir(false);
        let o = m.objetos.lock().unwrap();
        assert!(o.contains_key(&clave_de(&sha("VIVO"))));
        assert!(o.contains_key(&clave_de(&sha("TOCADO"))), "en su gracia");
        assert!(!o.contains_key(&clave_de(&sha("HUERFANO"))));
        assert!(
            !o.keys().any(|k| k.ends_with(&sha("HUERFANO"))),
            "ni su entrada del índice: {o:?}",
            o = o.keys().collect::<Vec<_>>()
        );
        assert_eq!(o.len(), 4);
    }

    #[test]
    fn la_fecha_de_un_almacen() {
        assert_eq!(ms_de_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            ms_de_iso("2026-09-29T16:27:15.682Z"),
            Some(1_790_699_235_682)
        );
        assert_eq!(ms_de_iso("2000-02-29T00:00:01.5Z"), Some(951_782_401_500));
        assert_eq!(ms_de_iso("ayer"), None);
    }

    /// `blobs-cotejar`: lo que está y mide lo que dice, bien; lo que falta, lo
    /// que mide otra cosa y —en la muestra— lo que no es su contenido, roto.
    #[test]
    fn el_cotejo_dice_lo_roto() {
        let m = Arc::new(Memoria::default());
        let mut w = Vec::new();
        trama(&mut w, "a.pdf", b"RECIBO", Ok(()));
        trama(&mut w, "c.jpg", b"FOTO", Ok(()));
        poner(m.clone(), &parse("{}"), std::io::BufReader::new(&w[..])).unwrap();
        let (a, c) = (
            ore_s3::hex(&ore_s3::sha256(b"RECIBO")),
            ore_s3::hex(&ore_s3::sha256(b"FOTO")),
        );
        let pedir = |muestra: &str, extra: &str| {
            let r = cotejar(
                m.clone(),
                &parse(&format!(
                    r#"{{"muestra":"{muestra}","blobs":[["{a}","6"],["{c}","4"]{extra}]}}"#
                )),
            )
            .unwrap();
            parse(&r)
        };
        let v =
            |n: &ore_core::parse::Node, k: &str| n.get(k).unwrap().1.as_str().unwrap().to_string();
        let r = pedir("9", "");
        assert_eq!((v(&r, "bien"), v(&r, "releidos")), ("2".into(), "2".into()));
        // uno que falta y uno que mide otra cosa
        let r = pedir("0", &format!(r#",["{}","1"],["{a}","7"]"#, "0".repeat(64)));
        assert_eq!(v(&r, "bien"), "2");
        assert_eq!(r.get("rotos").unwrap().1.items().len(), 2);
        // uno cuyo contenido ya no es el suyo: sólo lo ve la relectura
        m.objetos
            .lock()
            .unwrap()
            .insert(clave_de(&c), (b"FOTX".to_vec(), String::new()));
        assert_eq!(v(&pedir("0", ""), "bien"), "2");
        let r = pedir("9", "");
        assert_eq!(v(&r, "bien"), "1");
        assert!(format!("{:?}", r.get("rotos")).contains("sus bytes dan"));
    }

    /// `blobs-hay`: lo que el índice conoce y cuyo blob sigue; lo que el
    /// índice conoce pero se recogió, no.
    #[test]
    fn lo_que_ya_hay_por_su_huella() {
        let m = Arc::new(Memoria::default());
        let mut w = Vec::new();
        trama(&mut w, "a.pdf", b"RECIBO", Ok(()));
        trama(&mut w, "c.jpg", b"FOTO", Ok(()));
        poner(m.clone(), &parse("{}"), std::io::BufReader::new(&w[..])).unwrap();
        m.borrar(&clave_de(&ore_s3::hex(&ore_s3::sha256(b"FOTO"))))
            .unwrap();
        let r = hay(
            m.clone(),
            &parse(
                r#"{"huellas":[["h:a.pdf","6"],["h:c.jpg","4"],["h:nada","1"],["h:a.pdf","7"]]}"#,
            ),
        )
        .unwrap();
        let sha = ore_s3::hex(&ore_s3::sha256(b"RECIBO"));
        assert_eq!(r, format!(r#"{{"hay":[["h:a.pdf",6,"{sha}"]]}}"#));
    }

    /// `blob-firmar`: una URL por blob, en orden, con el tipo y la disposición
    /// que se piden y la vida acotada; lo que no es un sha256 no se firma.
    #[test]
    fn firmar_da_una_url_por_blob_y_solo_de_blobs() {
        let m = Arc::new(Memoria::default());
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let r = firmar(
            m.clone(),
            &parse(&format!(
                r#"{{"segundos":"99999","firmas":[{{"blob":"{a}","tipo":"application/pdf","disposicion":"inline"}},{{"blob":"{b}"}}]}}"#
            )),
        )
        .unwrap();
        let r = parse(&r);
        assert_eq!(r.get("segundos").unwrap().1.as_str(), Some("3600"));
        let f = r.get("firmadas").unwrap().1.items();
        let url = |i: usize| f[i].get("url").unwrap().1.as_str().unwrap().to_string();
        assert_eq!(
            url(0),
            format!(
                "mem://ore/v2/blobs/sha256/{a}?s=3600&response-content-type=application/pdf&response-content-disposition=inline"
            )
        );
        assert_eq!(url(1), format!("mem://ore/v2/blobs/sha256/{b}?s=3600"));
        for malo in ["ore/v2/indice", "../x", &"A".repeat(64)] {
            let e = firmar(
                m.clone(),
                &parse(&format!(r#"{{"firmas":[{{"blob":"{malo}"}}]}}"#)),
            )
            .unwrap_err();
            assert!(e.contains("sólo se firman blobs"), "{e}");
        }
    }
}
