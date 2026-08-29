//! Digests — documento, paquete y bundle.
//!
//! Es la **garantía G1** convertida en tres funciones: el mismo commit produce
//! el mismo digest, siempre, en cualquier máquina.
//!
//! Todo lo anterior existe para que esto sea cierto. El parser conserva el
//! texto crudo para no perder dígitos; `OOS6003` prohíbe los decimales sin
//! comillas para que no haya coma flotante que serializar; N1–N8 quitan del
//! medio todo lo que dos autores pueden escribir distinto diciendo lo mismo; y
//! JCS fija los bytes. Aquí solo se aplica SHA-256 a un resultado que ya no
//! tiene grados de libertad.

use crate::json::Json;
use crate::link::Package;
use crate::normalize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// La versión de OOS bajo la que compila esta implementación.
///
/// Entra en el digest del bundle porque **el mismo fuente compilado bajo una
/// especificación distinta no es el mismo artefacto**: si `v1beta1` cambiara una
/// regla de normalización, los bytes canónicos cambiarían y un digest idéntico
/// estaría mintiendo sobre qué significa el paquete.
pub const OOS_VERSION: &str = "oos.dev/v1alpha1";

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

/// `sha256:<64 dígitos hexadecimales en minúscula>`, la convención de OCI
/// (§5.4). El prefijo no es decoración: dice qué algoritmo, y sin él un digest
/// no es verificable por quien no conoce esta implementación.
fn representar(d: [u8; 32]) -> String {
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// `docDigest = SHA-256( bytes_canónicos_JCS(documento) )`
pub fn document(j: &Json) -> String {
    representar(sha256(j.jcs().as_bytes()))
}

/// El digest de cada documento fuente, por identidad.
pub fn documents(pkg: &Package) -> BTreeMap<String, String> {
    normalize::package(pkg)
        .iter()
        .map(|(id, j)| (id.clone(), document(j)))
        .collect()
}

/// `pkgDigest = SHA-256( concatenación de ( docId || 0x00 || docDigest ) )`
///
/// El separador `0x00` no es ceremonia: sin él, `("ab", "c")` y `("a", "bc")`
/// concatenarían a lo mismo. Un byte que no puede aparecer en un identificador
/// ni en un hexadecimal es lo que hace la construcción inyectiva.
pub fn package(pkg: &Package) -> String {
    let mut h = Sha256::new();
    // `BTreeMap` ya los da ordenados por `docId`, que es exactamente el orden
    // que §5.2 exige — y por identidad, nunca por ruta.
    for (id, dig) in documents(pkg) {
        h.update(id.as_bytes());
        h.update([0x00]);
        h.update(dig.as_bytes());
    }
    representar(h.finalize().into())
}

/// El digest del lock, o el del vacío si el paquete no tiene lock.
///
/// «Sin dependencias resueltas» es un estado, no una ausencia de información: si
/// no se distinguiera del lock vacío, añadir el primer lock trivial cambiaría el
/// bundle sin que nada del significado hubiera cambiado.
fn lock(pkg: &Package) -> String {
    match pkg.docs.iter().find(|d| normalize::es_lock(d)) {
        Some(d) => document(&normalize::document(d)),
        None => representar(sha256(b"")),
    }
}

/// `bundleDigest = SHA-256( pkgDigest || versión_OOS || digest_del_lock )`
///
/// Las dependencias resueltas forman parte del significado: el mismo fuente con
/// otro lock **es otro artefacto**, porque las etiquetas y las políticas que
/// importa son otras.
pub fn bundle(pkg: &Package) -> String {
    let mut h = Sha256::new();
    h.update(package(pkg).as_bytes());
    h.update(OOS_VERSION.as_bytes());
    h.update(lock(pkg).as_bytes());
    representar(h.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectores de FIPS 180-4. No prueban `sha2` —está auditada— sino que este
    /// módulo la usa bien: `finalize().into()` en el orden correcto y sin
    /// truncar.
    #[test]
    fn los_vectores_del_estandar() {
        assert_eq!(
            representar(sha256(b"")),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            representar(sha256(b"abc")),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn el_separador_hace_la_construccion_inyectiva() {
        let mut a = Sha256::new();
        a.update(b"ab");
        a.update([0x00]);
        a.update(b"c");
        let mut b = Sha256::new();
        b.update(b"a");
        b.update([0x00]);
        b.update(b"bc");
        assert_ne!(a.finalize(), b.finalize());
    }
}
