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
use sha2::{Digest, Sha256};

/// La versión de OOS bajo la que compila esta implementación.
///
/// Entra en el digest del bundle porque **el mismo fuente compilado bajo una
/// especificación distinta no es el mismo artefacto**: si `v1beta1` cambiara una
/// regla de normalización, los bytes canónicos cambiarían y un digest idéntico
/// estaría mintiendo sobre qué significa el paquete.
pub const OOS_VERSION: &str = "oos.dev/v1alpha1";

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

/// `sha256:<64 dígitos hexadecimales en minúscula>`, la convención de OCI
/// (§5.4). El prefijo no es decoración: dice qué algoritmo, y sin él un digest
/// no es verificable por quien no conoce esta implementación.
pub fn representar(d: [u8; 32]) -> String {
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// El digest de unos bytes cualesquiera, en la misma convención.
///
/// Existe porque el plano de contexto no es lo único que tiene identidad: el
/// artefacto de topología también, y su contenido no es un documento OOS sino un
/// fichero binario. Que las dos identidades salgan de **la misma función** no es
/// comodidad — dos convenciones de digest en el mismo sistema son dos formas de
/// escribir lo mismo, y la primera vez que alguien compare una con otra tendrá
/// razón en desconfiar.
pub fn de_bytes(b: &[u8]) -> String {
    representar(sha256(b))
}

/// `docDigest = SHA-256( bytes_canónicos_JCS(documento) )`
pub fn document(j: &Json) -> String {
    representar(sha256(j.jcs().as_bytes()))
}
