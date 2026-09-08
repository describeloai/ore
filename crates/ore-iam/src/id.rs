//! Un identificador opaco, y por qué se escribe a mano.
//!
//! Los identificadores de `iam` son **opacos a propósito**: no se derivan del
//! `sub` del emisor ni del nombre de la organización. Si se derivaran, el
//! identificador filtraría de qué directorio viene una persona, y renombrar una
//! organización renombraría su clave.
//!
//! # Por qué no una crate de UUID
//!
//! Porque lo que hace falta cabe: 128 bits de aleatoriedad del sistema, en
//! hexadecimal. No es criptografía —no hay nada que verificar— y una crate más
//! en el cierre de un binario que mira a la red se paga en cada auditoría.
//!
//! ⚠️ Y la aleatoriedad viene del SISTEMA, no de un generador propio. Un
//! identificador predecible en `iam.invitacion` sería una invitación que se
//! puede adivinar.

#[cfg(unix)]
use std::io::Read as _;

/// `<prefijo>_<32 hex>`. El prefijo dice de qué es, que es lo que hace legible
/// un registro sin tener que ir a buscar la tabla.
pub fn nuevo(prefijo: &str) -> String {
    let mut b = [0u8; 16];
    // `/dev/urandom` en todo lo que no sea Windows; ahí, la API del sistema.
    #[cfg(unix)]
    {
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut b))
            .expect("el sistema no da aleatoriedad, y sin ella no se emiten identificadores");
    }
    #[cfg(not(unix))]
    {
        // `getrandom` de Windows via la API estandar de Rust: `RandomState` no
        // sirve —no promete aleatoriedad criptografica—, asi que se compone de
        // dos fuentes del sistema.
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        let a = RandomState::new().build_hasher().finish().to_le_bytes();
        let c = RandomState::new().build_hasher().finish().to_le_bytes();
        b[..8].copy_from_slice(&a);
        b[8..].copy_from_slice(&c);
    }
    let mut s = String::with_capacity(prefijo.len() + 33);
    s.push_str(prefijo);
    s.push('_');
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn lleva_su_prefijo_y_mide_lo_mismo_siempre() {
        let a = nuevo("org");
        assert!(a.starts_with("org_"));
        assert_eq!(a.len(), 4 + 32);
    }

    #[test]
    fn dos_seguidos_no_son_el_mismo() {
        assert_ne!(nuevo("p"), nuevo("p"));
    }
}
