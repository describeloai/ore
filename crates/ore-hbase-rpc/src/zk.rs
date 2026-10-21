//! **ZooKeeper, solo lo que hace falta**: abrir una sesion, leer un nodo (`getData`) y cerrar. Sin *watches*:
//! la ubicacion de `hbase:meta` se vuelve a leer cuando falla, no se escucha. El protocolo es el de *jute*
//! (enteros en red, cadenas con longitud de 4 bytes) y cada trama lleva su longitud delante.

use crate::error::{Error, Resultado};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const GET_DATA: i32 = 4;
const CLOSE_SESSION: i32 = -11;
const NO_NODE: i32 = -101;

/// Una direccion `host:puerto`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Direccion {
    pub host: String,
    pub puerto: u16,
}

impl Direccion {
    /// `host` o `host:puerto` (2181 por defecto, el de ZooKeeper).
    pub fn analizar(s: &str) -> Resultado<Direccion> {
        let s = s.trim();
        if s.is_empty() {
            return Err(Error::Peticion("una direccion de ZooKeeper vacia".into()));
        }
        match s.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => Ok(Direccion {
                host: h.to_string(),
                puerto: p.parse().map_err(|_| {
                    Error::Peticion(format!("el puerto `{p}` de `{s}` no es un numero"))
                })?,
            }),
            _ => Ok(Direccion {
                host: s.to_string(),
                puerto: 2181,
            }),
        }
    }
}

/// La lista `a:2181,b:2181,c`.
pub fn quorum(s: &str) -> Resultado<Vec<Direccion>> {
    let v = s
        .split(',')
        .map(Direccion::analizar)
        .collect::<Resultado<Vec<_>>>()?;
    if v.is_empty() {
        return Err(Error::Peticion(
            "un quorum de ZooKeeper sin direcciones".into(),
        ));
    }
    Ok(v)
}

fn trama(cuerpo: &[u8]) -> Vec<u8> {
    let mut m = (cuerpo.len() as u32).to_be_bytes().to_vec();
    m.extend_from_slice(cuerpo);
    m
}

fn leer_n(s: &mut TcpStream, n: usize) -> Resultado<Vec<u8>> {
    let mut b = vec![0u8; n];
    s.read_exact(&mut b)?;
    Ok(b)
}

fn leer_trama(s: &mut TcpStream) -> Resultado<Vec<u8>> {
    let l = leer_n(s, 4)?;
    let n = u32::from_be_bytes([l[0], l[1], l[2], l[3]]) as usize;
    // Una trama de ZooKeeper no pasa de 1 MB por defecto (`jute.maxbuffer`); una mayor es basura.
    if n > 64 << 20 {
        return Err(Error::Protocolo(format!(
            "una trama de ZooKeeper de {n} bytes"
        )));
    }
    leer_n(s, n)
}

/// El `ConnectRequest`: version 0, ultimo zxid 0, tiempo de sesion, sesion 0, contrasena de 16 ceros, no solo
/// lectura.
pub fn peticion_de_conexion(sesion_ms: i32) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&0i32.to_be_bytes());
    c.extend_from_slice(&0i64.to_be_bytes());
    c.extend_from_slice(&sesion_ms.to_be_bytes());
    c.extend_from_slice(&0i64.to_be_bytes());
    c.extend_from_slice(&16i32.to_be_bytes());
    c.extend_from_slice(&[0u8; 16]);
    c.push(0);
    trama(&c)
}

/// La peticion `getData` de `ruta` (sin *watch*) con el `xid` dado.
pub fn peticion_get_data(xid: i32, ruta: &str) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&xid.to_be_bytes());
    c.extend_from_slice(&GET_DATA.to_be_bytes());
    c.extend_from_slice(&(ruta.len() as i32).to_be_bytes());
    c.extend_from_slice(ruta.as_bytes());
    c.push(0);
    trama(&c)
}

/// La respuesta a `getData`: los datos del nodo, o el error de ZooKeeper.
pub fn respuesta_get_data(cuerpo: &[u8], ruta: &str) -> Resultado<Vec<u8>> {
    // xid (4) + zxid (8) + error (4), y si va bien: datos con longitud (4) y el `Stat` (68)
    if cuerpo.len() < 16 {
        return Err(Error::Protocolo(
            "una respuesta de getData demasiado corta".into(),
        ));
    }
    let err = i32::from_be_bytes([cuerpo[12], cuerpo[13], cuerpo[14], cuerpo[15]]);
    if err == NO_NODE {
        return Err(Error::Zk(format!(
            "el nodo `{ruta}` no existe (es otro `znode=`, o HBase no ha arrancado)"
        )));
    }
    if err != 0 {
        return Err(Error::Zk(format!(
            "getData de `{ruta}` fallo con el codigo {err}"
        )));
    }
    if cuerpo.len() < 20 {
        return Err(Error::Protocolo("getData sin longitud de datos".into()));
    }
    let n = i32::from_be_bytes([cuerpo[16], cuerpo[17], cuerpo[18], cuerpo[19]]);
    if n < 0 {
        return Ok(Vec::new());
    }
    let n = n as usize;
    cuerpo
        .get(20..20 + n)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| Error::Protocolo("getData con menos datos de los que anuncia".into()))
}

fn conectar(d: &Direccion, tiempo: Duration) -> Resultado<TcpStream> {
    let mut ultimo = None;
    for a in (d.host.as_str(), d.puerto).to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, tiempo) {
            Ok(s) => {
                s.set_read_timeout(Some(tiempo))?;
                s.set_write_timeout(Some(tiempo))?;
                s.set_nodelay(true)?;
                return Ok(s);
            }
            Err(e) => ultimo = Some(e),
        }
    }
    Err(ultimo.map_or_else(|| Error::Zk(format!("`{}` no resuelve", d.host)), Error::Io))
}

/// Lee los datos del nodo `ruta` probando cada direccion del quorum por orden. Abre una sesion, lee, la cierra.
pub fn leer_nodo(quorum: &[Direccion], ruta: &str, tiempo: Duration) -> Resultado<Vec<u8>> {
    let mut ultimo = None;
    for d in quorum {
        match leer_nodo_de(d, ruta, tiempo) {
            Ok(v) => return Ok(v),
            // Un nodo que no existe lo dice cualquier miembro: probar otro no lo arregla.
            Err(e @ Error::Zk(_)) => return Err(e),
            Err(e) => ultimo = Some(e),
        }
    }
    Err(ultimo.unwrap_or_else(|| Error::Zk("sin direcciones".into())))
}

fn leer_nodo_de(d: &Direccion, ruta: &str, tiempo: Duration) -> Resultado<Vec<u8>> {
    let mut s = conectar(d, tiempo)?;
    s.write_all(&peticion_de_conexion(
        tiempo.as_millis().clamp(4000, 60_000) as i32,
    ))?;
    let r = leer_trama(&mut s)?;
    // version (4) + tiempo negociado (4) + sesion (8): un tiempo <= 0 es que el servidor no abrio la sesion
    if r.len() < 16 || i32::from_be_bytes([r[4], r[5], r[6], r[7]]) <= 0 {
        return Err(Error::Zk(format!(
            "{}:{} no abrio la sesion",
            d.host, d.puerto
        )));
    }
    s.write_all(&peticion_get_data(1, ruta))?;
    let datos = respuesta_get_data(&leer_trama(&mut s)?, ruta);
    // Cierre ordenado, sin esperar la respuesta: la sesion caduca sola si esto falla.
    let mut cierre = 2i32.to_be_bytes().to_vec();
    cierre.extend_from_slice(&CLOSE_SESSION.to_be_bytes());
    let _ = s.write_all(&trama(&cierre));
    datos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_quorum_admite_lista_y_puerto_por_defecto() {
        let q = quorum("zk1:2181, zk2 ,zk3:2182").unwrap();
        assert_eq!(q.len(), 3);
        assert_eq!(
            q[1],
            Direccion {
                host: "zk2".into(),
                puerto: 2181
            }
        );
        assert_eq!(q[2].puerto, 2182);
        assert!(quorum("").is_err());
        assert!(quorum("zk:no").is_err());
    }

    #[test]
    fn la_peticion_get_data_lleva_xid_tipo_ruta_y_sin_watch() {
        let t = peticion_get_data(1, "/hbase/x");
        assert_eq!(&t[0..4], &(4 + 4 + 4 + 8 + 1u32).to_be_bytes());
        assert_eq!(&t[4..8], &1i32.to_be_bytes());
        assert_eq!(&t[8..12], &4i32.to_be_bytes());
        assert_eq!(&t[12..16], &8i32.to_be_bytes());
        assert_eq!(&t[16..24], b"/hbase/x");
        assert_eq!(t[24], 0);
    }

    #[test]
    fn una_respuesta_de_nodo_inexistente_dice_cual() {
        let mut c = vec![0u8; 12];
        c.extend_from_slice(&NO_NODE.to_be_bytes());
        let e = respuesta_get_data(&c, "/hbase/meta-region-server").unwrap_err();
        assert!(e.to_string().contains("meta-region-server"), "{e}");
    }

    #[test]
    fn una_respuesta_corta_no_entra_en_panico() {
        assert!(respuesta_get_data(&[0; 5], "/x").is_err());
        let mut c = vec![0u8; 16];
        c.extend_from_slice(&1000i32.to_be_bytes());
        assert!(respuesta_get_data(&c, "/x").is_err());
    }
}
