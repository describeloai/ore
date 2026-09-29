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
//! otra colección— cada blob deja un objeto diminuto
//! `ore/v2/blobs/huellas/<sha256(huella + tamaño)>` con su sha256 dentro. Se
//! escribe **después** del blob, así que nunca apunta a uno que no llegó. La
//! spec lo respalda: *«dos ítems con la misma huella son el mismo contenido»*
//! (`v1alpha16/02` §5).
//!
//! # Los verbos
//!
//! | verbo | entrada | salida |
//! |---|---|---|
//! | `blobs` | la petición en una línea (`hilos`, `temporal`) y después el flujo de tramas de un lector (`ore_driver::tramas`) | una línea por ítem (`blob`, `subido`, o `error`) y el resumen |
//! | `blobs-hay` | `{huellas: [[huella, tamaño], …]}` | `{hay: [[huella, tamaño, sha256], …]}`: los que ya están, blob incluido |
//! | `blob-leer` | `{blob, archivo, rango?}` | los bytes en `archivo`, y su sha256 |

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

/// Dónde vive el sha256 de lo que el origen llama `huella` con ese tamaño.
pub fn clave_de_huella(huella: &str, tamano: u64) -> String {
    format!(
        "{RAIZ}/huellas/{}",
        ore_s3::hex(&ore_s3::sha256(format!("{huella}\t{tamano}").as_bytes()))
    )
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
                &clave_de_huella(&l.cab.huella, l.cab.tamano),
                l.sha.as_bytes(),
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
/// que el lago ya tiene, con su blob. El índice dice el sha256 y un `HEAD`
/// comprueba que el blob sigue (la recogida pudo llevárselo).
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
                    let r = cuenta
                        .leer(&clave_de_huella(h, *t))
                        .and_then(|sha| match sha {
                            Some(sha) if sha.len() == 64 => {
                                Ok(cuenta.existe(&clave_de(&sha))?.then_some(sha))
                            }
                            _ => Ok(None),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Un almacén en memoria que coteja como GCS: un blob cuyo crc32c no es
    /// el de sus bytes no llega a existir.
    #[derive(Default)]
    struct Memoria {
        objetos: Mutex<BTreeMap<String, (Vec<u8>, String)>>,
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
        assert_eq!(
            o[&clave_de_huella("h:d.pdf", 6)].0,
            sha.as_bytes(),
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
}
