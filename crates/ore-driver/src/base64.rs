//! **Base64 estándar con relleno** (RFC 4648 §4): cómo viaja un binario (`bytea`, `RAW`, `BLOB`, `varbinary`) como
//! texto en un lote o en una petición.
//!
//! Estaba escrita **cinco veces** —en `ore-phoenix`, `ore-cassandra`, `ore-mongodb`, `ore-hbase` y, al empezar,
//! `ore-oracle`—, byte a byte igual. Una copia que diverge en un borde (el relleno, un byte suelto) cambia lo que un
//! cliente lee sin que ninguna prueba de su conector lo vea; aquí hay una, con los vectores de la RFC.

const ALFABETO: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Los bytes, en base64 estándar con relleno.
pub fn codificar(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for t in bytes.chunks(3) {
        let n = (u32::from(t[0]) << 16)
            | (u32::from(*t.get(1).unwrap_or(&0)) << 8)
            | u32::from(*t.get(2).unwrap_or(&0));
        s.push(ALFABETO[(n >> 18) as usize & 63] as char);
        s.push(ALFABETO[(n >> 12) as usize & 63] as char);
        s.push(if t.len() > 1 {
            ALFABETO[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if t.len() > 2 {
            ALFABETO[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// Lo contrario de [`codificar`]; `None` si no es base64 estándar con relleno.
pub fn descodificar(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let valor = |c: u8| ALFABETO.iter().position(|a| *a == c).map(|p| p as u32);
    let mut sal = Vec::with_capacity(b.len() / 4 * 3);
    for (i, t) in b.chunks(4).enumerate() {
        let ultimo = (i + 1) * 4 == b.len();
        let relleno = t.iter().rev().take_while(|c| **c == b'=').count();
        if relleno > 2 || (relleno > 0 && !ultimo) {
            return None;
        }
        let mut n = 0u32;
        for (j, c) in t.iter().enumerate() {
            n = (n << 6) | if j >= 4 - relleno { 0 } else { valor(*c)? };
        }
        sal.push((n >> 16) as u8);
        if relleno < 2 {
            sal.push((n >> 8) as u8);
        }
        if relleno < 1 {
            sal.push(n as u8);
        }
    }
    Some(sal)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Los vectores de la RFC 4648 §10 y los bordes del relleno.
    #[test]
    fn los_vectores_de_la_rfc_4648() {
        for (plano, cod) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(codificar(plano.as_bytes()), cod, "{plano:?}");
            assert_eq!(descodificar(cod).unwrap(), plano.as_bytes(), "{cod:?}");
        }
        assert_eq!(codificar(&[0, 255, 16, 171]), "AP8Qqw==");
        assert_eq!(descodificar("AAAAKg==").unwrap(), [0, 0, 0, 42]);
    }

    #[test]
    fn lo_que_no_es_base64_estandar_se_rechaza() {
        for malo in ["Zg=", "Zg", "Z===", "====", "Zm9v=Zg=", "Zm9*", "Zm 9"] {
            assert!(descodificar(malo).is_none(), "{malo:?}");
        }
    }
}
