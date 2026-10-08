//! **Contraseñas sin cofre** (0058 P4·4): se generan, se enseñan UNA vez y sólo
//! se guarda el verificador SCRAM-SHA-256 (RFC 5802 / 7677), el mismo que
//! guarda Postgres en `pg_authid`. Con él se puede comprobar una contraseña y
//! no se puede recuperar: es lo que el proxy de P5 pedirá al plano de control.
//!
//! ```text
//!   SCRAM-SHA-256$<iteraciones>:<sal>$<StoredKey>:<ServerKey>     (base64 con relleno)
//!   SaltedPassword = PBKDF2-HMAC-SHA256(contraseña, sal, iteraciones)
//!   StoredKey      = SHA256(HMAC(SaltedPassword, "Client Key"))
//!   ServerKey      = HMAC(SaltedPassword, "Server Key")
//! ```
//!
//! HMAC y PBKDF2 son veinte líneas sobre `sha2`, que ya está en el árbol; lo
//! difícil (la función de resumen) no se escribe aquí. Y no se fía de sí mismo:
//! el contrato crea un rol con este verificador y entra en Postgres con la
//! contraseña.

use sha2::{Digest, Sha256};

/// Las de Postgres por defecto (`scram_iterations`).
pub const ITERACIONES: u32 = 4096;

const BLOQUE: usize = 64;

pub fn hmac(clave: &[u8], mensaje: &[u8]) -> [u8; 32] {
    let mut k = [0u8; BLOQUE];
    if clave.len() > BLOQUE {
        k[..32].copy_from_slice(&Sha256::digest(clave));
    } else {
        k[..clave.len()].copy_from_slice(clave);
    }
    let (mut dentro, mut fuera) = (Sha256::new(), Sha256::new());
    dentro.update(k.map(|b| b ^ 0x36));
    dentro.update(mensaje);
    fuera.update(k.map(|b| b ^ 0x5c));
    fuera.update(dentro.finalize());
    fuera.finalize().into()
}

/// PBKDF2-HMAC-SHA256 de un solo bloque (32 bytes: lo que pide SCRAM).
pub fn pbkdf2(contrasena: &[u8], sal: &[u8], iteraciones: u32) -> [u8; 32] {
    let mut u = hmac(contrasena, &[sal, &1u32.to_be_bytes()].concat());
    let mut t = u;
    for _ in 1..iteraciones {
        u = hmac(contrasena, &u);
        for (a, b) in t.iter_mut().zip(u) {
            *a ^= b;
        }
    }
    t
}

/// El verificador de una contraseña con una sal dada.
pub fn verificador(contrasena: &str, sal: &[u8], iteraciones: u32) -> String {
    let salada = pbkdf2(contrasena.as_bytes(), sal, iteraciones);
    let stored = Sha256::digest(hmac(&salada, b"Client Key"));
    let server = hmac(&salada, b"Server Key");
    format!(
        "SCRAM-SHA-256${iteraciones}:{}${}:{}",
        b64(sal),
        b64(&stored),
        b64(&server)
    )
}

/// Bytes al azar del sistema (`/dev/urandom`, que existe también en `scratch`).
pub fn azar(n: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut b = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| format!("no hay azar del sistema: {e}"))?;
    Ok(b)
}

/// Una contraseña nueva (32 letras y cifras: ~190 bits) y su verificador.
pub fn nueva() -> Result<(String, String), String> {
    const ABC: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    // Rechazo para no sesgar: sólo bytes por debajo del mayor múltiplo del alfabeto.
    let tope = 256 - 256 % ABC.len();
    let mut c = String::with_capacity(32);
    while c.len() < 32 {
        for b in azar(64)? {
            if (b as usize) < tope && c.len() < 32 {
                c.push(ABC[b as usize % ABC.len()] as char);
            }
        }
    }
    let v = verificador(&c, &azar(16)?, ITERACIONES);
    Ok((c, v))
}

fn b64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for t in b.chunks(3) {
        let n = t
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, x)| n | (*x as u32) << (16 - 8 * i));
        for i in 0..4 {
            s.push(if i <= t.len() {
                A[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    s
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn hmac_como_la_rfc_4231() {
        // Caso 2: clave «Jefe».
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn pbkdf2_como_la_rfc_7914() {
        // PBKDF2-HMAC-SHA256 («passwd», «salt», 1): los primeros 32 bytes.
        assert_eq!(
            hex(&pbkdf2(b"passwd", b"salt", 1)),
            "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc"
        );
    }

    #[test]
    fn base64_con_relleno() {
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
    }

    #[test]
    fn el_verificador_tiene_la_forma_de_postgres() {
        let v = verificador("secreto", b"0123456789abcdef", 4096);
        assert!(
            v.starts_with("SCRAM-SHA-256$4096:MDEyMzQ1Njc4OWFiY2RlZg==$"),
            "{v}"
        );
        assert_eq!(v.matches('$').count(), 2);
    }
}
