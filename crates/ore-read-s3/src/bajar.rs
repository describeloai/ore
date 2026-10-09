//! **Los bytes de una colección mantenida** (0046 E8·2): cada ítem que la
//! transacción no encontró en el lago, bajado del bucket **fijado a su
//! versión** y entregado al almacén en tramas (`ore_driver::tramas`).
//!
//! - **Fijado a la versión** que el manifiesto dice, no a lo que hay ahora: un
//!   ítem sobrescrito o borrado después de listarse se sigue copiando mientras
//!   su versión exista (E7: una versión vieja se lee por su `VersionId`).
//! - **Cotejado mientras baja**: el CRC64NVME de los bytes que llegan contra la
//!   huella del manifiesto. Lo que no cuadra no entra; un ítem pequeño ni
//!   siquiera se entrega, uno grande se cierra con error y el almacén lo tira.
//! - **En paralelo**: lo pequeño (menos de 8 MiB, casi todo PDF y foto) se baja
//!   entero en cada hilo y se entrega de una vez; lo grande se entrega según
//!   llega, con la salida para él solo, y la memoria no crece con el fichero.
//!
//! Entra `{url, items: [{clave, version, huella, tamano, tipo}], hilos?}`; sale
//! el flujo de tramas por stdout, y las cuentas por stderr.

use crate::origen::Origen;
use ore_driver::tramas;
use std::io::{Read, Write};
use std::sync::Mutex;

/// Por debajo, entero en memoria y de una vez.
pub const EN_MEMORIA: u64 = 8 << 20;
const HILOS: usize = 32;

#[derive(Debug, Clone)]
pub struct Pedido {
    pub clave: String,
    pub version: String,
    pub huella: String,
    pub tamano: u64,
    pub tipo: String,
}

pub fn pedidos(peticion: &str) -> Result<(Vec<Pedido>, usize), String> {
    let n: serde_json::Value =
        serde_json::from_str(peticion).map_err(|e| format!("la petición no es JSON: {e}"))?;
    let s = |i: &serde_json::Value, k: &str| {
        i.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    let items = n
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or("a `bajar` le faltan los `items`")?
        .iter()
        .map(|i| Pedido {
            clave: s(i, "clave"),
            version: s(i, "version"),
            huella: s(i, "huella"),
            tamano: i
                .get("tamano")
                .and_then(|t| t.as_u64().or_else(|| t.as_str()?.parse().ok()))
                .unwrap_or(0),
            tipo: s(i, "tipo"),
        })
        .collect();
    let hilos = n
        .get("hilos")
        .and_then(|h| h.as_u64())
        .map(|h| h as usize)
        .unwrap_or(HILOS)
        .clamp(1, 128);
    Ok((items, hilos))
}

/// Lo que se cuenta al final.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cuentas {
    pub entregados: usize,
    pub fallidos: usize,
    pub bytes: u64,
}

/// Qué dice la huella del manifiesto que debe dar el CRC64NVME, si lo dice.
fn esperado(huella: &str) -> Option<&str> {
    huella.strip_prefix("crc64nvme:")
}

fn tipo_de(p: &Pedido, a: &ore_s3::Abierto) -> String {
    if !p.tipo.is_empty() {
        return p.tipo.clone();
    }
    match a.tipo.as_deref() {
        Some(t) if !t.contains("octet-stream") => t.to_string(),
        _ => String::new(),
    }
}

/// **Un ítem**: abrirlo, bajarlo cotejado y entregar su trama.
pub fn uno<W: Write>(o: &dyn Origen, p: &Pedido, salida: &Mutex<W>) -> Result<u64, String> {
    let fallar = |m: String| -> Result<u64, String> {
        let mut w = salida
            .lock()
            .map_err(|_| "la salida quedó envenenada".to_string())?;
        tramas::escribir_fallo(&mut *w, &p.clave, &p.version, &m)
            .and_then(|_| w.flush())
            .map_err(|e| format!("no se pudo escribir el flujo: {e}"))?;
        Err(m)
    };
    let (mut r, a) = match o.abrir_version(&p.clave, &p.version) {
        Ok(x) => x,
        Err(m) => return fallar(m),
    };
    if let Some(t) = a.tamano.filter(|t| *t != p.tamano) {
        return fallar(format!(
            "`{}` (versión {}) mide {t} bytes y el manifiesto dice {}",
            p.clave, p.version, p.tamano
        ));
    }
    if let (Some(e), Some(s3)) = (
        esperado(&p.huella),
        a.huella
            .as_deref()
            .and_then(|h| h.strip_prefix("crc64nvme:")),
    ) && e != s3
    {
        return fallar(format!(
            "`{}` (versión {}): S3 da la huella {s3} y el manifiesto {e}",
            p.clave, p.version
        ));
    }
    let cab = tramas::Cabecera {
        clave: p.clave.clone(),
        version: p.version.clone(),
        huella: p.huella.clone(),
        tipo: tipo_de(p, &a),
        tamano: p.tamano,
    };
    let mut crc = ore_s3::huella::Crc64Nvme::default();
    let cotejo = |crc: &ore_s3::huella::Crc64Nvme| -> Result<(), String> {
        match esperado(&p.huella) {
            Some(e) if e != crc.base64() => Err(format!(
                "los bytes de `{}` (versión {}) dan la huella {} y el manifiesto dice {e}: no se copian",
                p.clave,
                p.version,
                crc.base64()
            )),
            _ => Ok(()),
        }
    };

    if p.tamano < EN_MEMORIA {
        // Pequeño: entero aquí, cotejado, y se entrega de una vez.
        let mut b = Vec::with_capacity(p.tamano as usize);
        if let Err(e) = (&mut r).take(p.tamano + 1).read_to_end(&mut b) {
            return fallar(format!("`{}` se cortó al bajar: {e}", p.clave));
        }
        if b.len() as u64 != p.tamano {
            return fallar(format!(
                "`{}` dio {} bytes y el manifiesto dice {}",
                p.clave,
                b.len(),
                p.tamano
            ));
        }
        crc.sumar(&b);
        if let Err(m) = cotejo(&crc) {
            return fallar(m);
        }
        let mut w = salida
            .lock()
            .map_err(|_| "la salida quedó envenenada".to_string())?;
        tramas::escribir_cabecera(&mut *w, &cab)
            .and_then(|_| w.write_all(&b))
            .and_then(|_| tramas::escribir_cierre(&mut *w, Ok(())))
            .and_then(|_| w.flush())
            .map_err(|e| format!("no se pudo escribir el flujo: {e}"))?;
        return Ok(p.tamano);
    }

    // Grande: según llega, con la salida para él solo. Si se corta, se
    // completa con ceros y se cierra con error: el flujo sigue legible.
    let mut w = salida
        .lock()
        .map_err(|_| "la salida quedó envenenada".to_string())?;
    let io = |e: std::io::Error| format!("no se pudo escribir el flujo: {e}");
    tramas::escribir_cabecera(&mut *w, &cab).map_err(io)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut resto = p.tamano;
    let mut corte: Option<String> = None;
    while resto > 0 {
        let hasta = resto.min(buf.len() as u64) as usize;
        match r.read(&mut buf[..hasta]) {
            Ok(0) => {
                corte = Some(format!(
                    "`{}` acabó a los {} de {} bytes",
                    p.clave,
                    p.tamano - resto,
                    p.tamano
                ));
                break;
            }
            Ok(k) => {
                crc.sumar(&buf[..k]);
                w.write_all(&buf[..k]).map_err(io)?;
                resto -= k as u64;
            }
            Err(e) => {
                corte = Some(format!("`{}` se cortó al bajar: {e}", p.clave));
                break;
            }
        }
    }
    let cierre = match corte {
        Some(m) => {
            let ceros = vec![0u8; 1 << 16];
            while resto > 0 {
                let k = resto.min(ceros.len() as u64) as usize;
                w.write_all(&ceros[..k]).map_err(io)?;
                resto -= k as u64;
            }
            Err(m)
        }
        None => cotejo(&crc),
    };
    tramas::escribir_cierre(&mut *w, cierre.as_ref().map(|_| ()).map_err(String::as_str))
        .and_then(|_| w.flush())
        .map_err(io)?;
    cierre.map(|_| p.tamano)
}

/// Todos, uno tras otro. Lo que usan las pruebas (el bucket en memoria no se
/// comparte entre hilos).
#[cfg(test)]
pub fn todos<W: Write>(o: &dyn Origen, pedidos: &[Pedido], salida: &Mutex<W>) -> Cuentas {
    let mut c = Cuentas::default();
    for p in pedidos {
        match uno(o, p, salida) {
            Ok(b) => {
                c.entregados += 1;
                c.bytes += b;
            }
            Err(_) => c.fallidos += 1,
        }
    }
    c
}

/// Todos, en `hilos` a la vez, contra un origen de verdad.
pub fn en_paralelo<W: Write + Send>(
    b: &(dyn Origen + Sync),
    pedidos: &[Pedido],
    hilos: usize,
    salida: &Mutex<W>,
) -> Cuentas {
    let siguiente = std::sync::atomic::AtomicUsize::new(0);
    let total = Mutex::new(Cuentas::default());
    std::thread::scope(|s| {
        for _ in 0..hilos.min(pedidos.len()).max(1) {
            s.spawn(|| {
                loop {
                    let i = siguiente.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(p) = pedidos.get(i) else { return };
                    let r = uno(b, p, salida);
                    let mut t = total.lock().unwrap();
                    match r {
                        Ok(n) => {
                            t.entregados += 1;
                            t.bytes += n;
                        }
                        Err(_) => t.fallidos += 1,
                    }
                }
            });
        }
    });
    total.into_inner().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origen::EnMemoria;
    use ore_driver::tramas::Trama;

    fn pedido(clave: &str, version: &str, cuerpo: &[u8]) -> Pedido {
        Pedido {
            clave: clave.into(),
            version: version.into(),
            huella: ore_s3::huella::de(cuerpo),
            tamano: cuerpo.len() as u64,
            tipo: "application/pdf".into(),
        }
    }

    /// Lo que un almacén leería del flujo: (clave, bytes, cierre) o el fallo.
    fn leer(flujo: &[u8]) -> Vec<(String, Result<Vec<u8>, String>)> {
        let mut r = std::io::BufReader::new(flujo);
        let mut out = Vec::new();
        while let Some(t) = tramas::leer_trama(&mut r).unwrap() {
            match t {
                Trama::Fallo { clave, motivo, .. } => out.push((clave, Err(motivo))),
                Trama::Bytes(c) => {
                    let mut b = Vec::new();
                    (&mut r).take(c.tamano).read_to_end(&mut b).unwrap();
                    let cierre = tramas::leer_cierre(&mut r).unwrap();
                    out.push((c.clave, cierre.map(|_| b)));
                }
            }
        }
        out
    }

    /// **El experimento de E7**: `a.pdf` en su versión vieja (sobrescrita) y
    /// en la nueva, `b.pdf` borrado (su versión sigue): todo se baja fijado
    /// a su versión y cotejado.
    #[test]
    fn se_baja_cada_version_y_se_coteja() {
        let mut o = EnMemoria::default();
        for (c, v, b) in [
            ("r/a.pdf", "a1", "FACTURA"),
            ("r/a.pdf", "a2", "RECIBO"),
            ("r/b.pdf", "b1", "RECIBO"),
        ] {
            o.por_version
                .insert((c.into(), v.into()), b.as_bytes().to_vec());
        }
        let ps = vec![
            pedido("r/a.pdf", "a1", b"FACTURA"),
            pedido("r/a.pdf", "a2", b"RECIBO"),
            pedido("r/b.pdf", "b1", b"RECIBO"),
        ];
        let salida = Mutex::new(Vec::new());
        let c = todos(&o, &ps, &salida);
        assert_eq!((c.entregados, c.fallidos, c.bytes), (3, 0, 19));
        let r = leer(&salida.into_inner().unwrap());
        assert_eq!(
            r[0],
            ("r/a.pdf".into(), Ok(b"FACTURA".to_vec())),
            "la vieja, por su versión"
        );
        assert_eq!(r[2].1, Ok(b"RECIBO".to_vec()));
    }

    /// Lo que no cuadra no entra: una huella que no es la de los bytes, una
    /// versión que ya no existe, un tamaño que no es el del manifiesto. Los
    /// demás pasan.
    #[test]
    fn lo_que_no_cuadra_no_se_entrega() {
        let o = EnMemoria::con(&[
            ("r/a.pdf", b"RECIBO".to_vec()),
            ("r/c.jpg", b"FOTO".to_vec()),
        ]);
        let mut malo = pedido("r/a.pdf", "null", b"RECIBO");
        malo.huella = ore_s3::huella::de(b"OTRA COSA");
        let mut corto = pedido("r/c.jpg", "null", b"FOTO");
        corto.tamano = 3;
        let ps = vec![
            malo,
            pedido("r/borrada.pdf", "zz", b"X"),
            corto,
            pedido("r/c.jpg", "null", b"FOTO"),
        ];
        let salida = Mutex::new(Vec::new());
        let c = todos(&o, &ps, &salida);
        assert_eq!((c.entregados, c.fallidos), (1, 3));
        let r = leer(&salida.into_inner().unwrap());
        assert!(
            matches!(&r[0].1, Err(m) if m.contains("huella")),
            "{:?}",
            r[0]
        );
        assert!(matches!(&r[1].1, Err(m) if m.contains("404")), "{:?}", r[1]);
        assert!(
            matches!(&r[2].1, Err(m) if m.contains("mide 4")),
            "{:?}",
            r[2]
        );
        assert_eq!(r[3].1, Ok(b"FOTO".to_vec()));
    }

    /// Lo grande pasa según llega, y se coteja igual: con la huella mala, el
    /// cierre es de error y el almacén lo tira.
    #[test]
    fn lo_grande_pasa_en_flujo_y_se_coteja_al_final() {
        let grande: Vec<u8> = (0..(EN_MEMORIA + 5)).map(|i| (i % 251) as u8).collect();
        let o = EnMemoria::con(&[("r/v.mp4", grande.clone())]);
        let bueno = pedido("r/v.mp4", "null", &grande);
        let mut malo = bueno.clone();
        malo.huella = ore_s3::huella::de(b"no");
        let salida = Mutex::new(Vec::new());
        let c = todos(&o, &[bueno, malo], &salida);
        assert_eq!((c.entregados, c.fallidos), (1, 1));
        let r = leer(&salida.into_inner().unwrap());
        assert_eq!(r[0].1.as_ref().map(|b| b.len()), Ok(grande.len()));
        assert!(
            matches!(&r[1].1, Err(m) if m.contains("huella")),
            "{:?}",
            r[1].1.as_ref().err()
        );
    }
}
