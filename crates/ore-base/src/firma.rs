//! La firma, y la mitad de ella que vive aquí dentro.
//!
//! Un digest dice **qué** es un paquete. Una firma dice **de quién**, y es lo
//! único que un digest no puede contestar: un digest correcto lo puede producir
//! cualquiera que tenga los bytes, y quien los sustituyó los tiene.
//!
//! # Verificar dentro, firmar fuera
//!
//! No es una preferencia, sale de la doctrina que ya existía:
//!
//! | | Dónde | Por qué |
//! |---|---|---|
//! | **verificar** | aquí | es aritmética sobre bytes que ya tienes. No es red |
//! | **firmar** | delegado | exige una clave privada, y **una credencial nunca entra en el compilador** |
//!
//! Es la misma frontera que `source add` traza para un secreto y que
//! `ore-read-<tipo>` traza para una conexión. Verificar no necesita nada que no
//! esté ya en el árbol; firmar necesita justamente lo que este binario no debe
//! poder tocar.
//!
//! Y el veto de `dependencias.rs` no lo impide: veta **red y FFI** —`ring` está
//! vetada por traer ensamblador, no por ser cripto— y `sha2` ya está dentro
//! porque el digest la necesita. `ed25519-compact` es Rust puro, sin
//! aleatoriedad, sin reloj y sin sistema: entra una crate, y `CIERRE` lo dice.
//!
//! # Qué se firma, y por qué no son los bytes del fichero
//!
//! Se firma el **enunciado**: el paquete, su versión y su digest, en forma
//! canónica.
//!
//! ```text
//! {"digest":"sha256:…","package":"oos.dev/regulatory/gdpr","version":"0.2.0"}
//! ```
//!
//! Firmar el fichero habría atado la firma al contenedor, y el contenedor no
//! cambia la identidad: el mismo paquete como árbol y como `.oob` digiere igual,
//! así que una firma sobre el fichero valdría para una forma y no para la otra.
//! Y firmar el digest **a secas** habría producido una afirmación sin sujeto —
//! *«estos bytes existen»*—, replicable sobre cualquier coordenada. Con los tres
//! campos, la firma dice lo mismo que dice una entrada del lock, que es
//! exactamente lo que se quiere poder creer.

use crate::json::Json;

/// Ed25519 y nada más, por ahora.
///
/// El campo se escribe igualmente en cada firma: el día que haga falta un
/// segundo algoritmo, lo que no puede pasar es que las firmas viejas no digan
/// cuál son —y una firma sin algoritmo es una firma que hay que adivinar.
pub const ED25519: &str = "ed25519";

/// El enunciado que se firma: la coordenada y el digest, en forma canónica.
///
/// Es una función de tres cadenas y nada más, así que quien firma fuera puede
/// construirlo sin este código. Esa es la condición para que firmar sea
/// delegable de verdad y no un favor que hace este binario.
pub fn enunciado(paquete: &str, version: &str, digest: &str) -> String {
    Json::obj([
        ("digest", Json::s(digest)),
        ("package", Json::s(paquete)),
        ("version", Json::s(version)),
    ])
    .jcs()
}

/// Por qué una firma no vale. Cada variante es un fallo distinto y se cuentan
/// aparte porque no significan lo mismo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalida {
    /// El algoritmo no es uno que se sepa comprobar.
    Algoritmo,
    /// La clave pública no mide 32 bytes, o no es hexadecimal.
    Clave,
    /// La firma no mide 64 bytes, o no es hexadecimal.
    Forma,
    /// Mide bien y **no case**: el caso que importa.
    NoCasa,
}

impl Invalida {
    pub const fn como_texto(self) -> &'static str {
        match self {
            Invalida::Algoritmo => "el algoritmo no es `ed25519`",
            Invalida::Clave => "la clave pública no es 32 bytes en hexadecimal",
            Invalida::Forma => "la firma no es 64 bytes en hexadecimal",
            Invalida::NoCasa => "la firma no corresponde a esta clave",
        }
    }
}

/// Comprueba una firma contra un enunciado.
///
/// Las cuatro formas de fallar se distinguen a propósito. «No case» y «está mal
/// escrita» tienen la misma consecuencia —no se usa— y causas opuestas: una es
/// alguien sustituyendo un paquete y la otra es alguien tecleando un campo, y
/// darle el mismo mensaje a las dos manda a investigar en la dirección
/// equivocada la mitad de las veces.
pub fn verificar(
    algoritmo: &str,
    clave_publica: &str,
    firma: &str,
    enunciado: &str,
) -> Result<(), Invalida> {
    if algoritmo != ED25519 {
        return Err(Invalida::Algoritmo);
    }
    let pk = hex(clave_publica).ok_or(Invalida::Clave)?;
    let pk = ed25519_compact::PublicKey::from_slice(&pk).map_err(|_| Invalida::Clave)?;
    let sig = hex(firma).ok_or(Invalida::Forma)?;
    let sig = ed25519_compact::Signature::from_slice(&sig).map_err(|_| Invalida::Forma)?;
    pk.verify(enunciado.as_bytes(), &sig)
        .map_err(|_| Invalida::NoCasa)
}

/// Hexadecimal a bytes. En minúsculas y con longitud par, como lo escribe
/// `digest`: una sola forma de escribir cada valor, que es lo mismo que pide la
/// forma canónica de todo lo demás.
pub fn hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.is_empty() {
        return None;
    }
    let d = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    s.as_bytes()
        .chunks(2)
        .map(|p| Some(d(p[0])? << 4 | d(p[1])?))
        .collect()
}

/// Y la vuelta, para quien escribe una firma o una clave.
pub fn a_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_compact::{KeyPair, Seed};

    fn par() -> KeyPair {
        KeyPair::from_seed(Seed::new([7u8; 32]))
    }

    #[test]
    fn una_firma_del_enunciado_verifica() {
        let kp = par();
        let e = enunciado("oos.dev/regulatory/gdpr", "0.2.0", "sha256:abc");
        let firma = a_hex(kp.sk.sign(e.as_bytes(), None).as_ref());
        assert_eq!(
            verificar(ED25519, &a_hex(kp.pk.as_ref()), &firma, &e),
            Ok(())
        );
    }

    /// El punto entero de firmar el enunciado y no el digest a secas: una firma
    /// sobre `gdpr 0.2.0` no vale para `gdpr 0.3.0` aunque el digest coincida.
    #[test]
    fn una_firma_no_vale_para_otra_version_del_mismo_digest() {
        let kp = par();
        let firma = a_hex(
            kp.sk
                .sign(
                    enunciado("oos.dev/regulatory/gdpr", "0.2.0", "sha256:abc").as_bytes(),
                    None,
                )
                .as_ref(),
        );
        assert_eq!(
            verificar(
                ED25519,
                &a_hex(kp.pk.as_ref()),
                &firma,
                &enunciado("oos.dev/regulatory/gdpr", "0.3.0", "sha256:abc"),
            ),
            Err(Invalida::NoCasa)
        );
    }

    /// Ni para otro paquete, que es la otra mitad del sujeto.
    #[test]
    fn una_firma_no_vale_para_otro_paquete() {
        let kp = par();
        let firma = a_hex(
            kp.sk
                .sign(
                    enunciado("oos.dev/regulatory/gdpr", "0.2.0", "sha256:abc").as_bytes(),
                    None,
                )
                .as_ref(),
        );
        assert_eq!(
            verificar(
                ED25519,
                &a_hex(kp.pk.as_ref()),
                &firma,
                &enunciado("oos.dev/otro/paquete", "0.2.0", "sha256:abc"),
            ),
            Err(Invalida::NoCasa)
        );
    }

    /// Otra clave no verifica lo que no firmó, que es lo que hace que la firma
    /// diga **de quién** y no solo **qué**.
    #[test]
    fn otra_clave_no_verifica_la_firma() {
        let e = enunciado("p", "1.0.0", "sha256:abc");
        let firma = a_hex(par().sk.sign(e.as_bytes(), None).as_ref());
        let otra = KeyPair::from_seed(Seed::new([9u8; 32]));
        assert_eq!(
            verificar(ED25519, &a_hex(otra.pk.as_ref()), &firma, &e),
            Err(Invalida::NoCasa)
        );
    }

    /// Un campo mal escrito y una firma que no case fallan los dos, y no por lo
    /// mismo: distinguirlos es lo que decide hacia dónde mirar.
    #[test]
    fn un_campo_mal_escrito_no_se_confunde_con_una_firma_que_no_casa() {
        let e = enunciado("p", "1.0.0", "sha256:abc");
        let kp = par();
        let firma = a_hex(kp.sk.sign(e.as_bytes(), None).as_ref());
        let pk = a_hex(kp.pk.as_ref());
        assert_eq!(verificar("rsa", &pk, &firma, &e), Err(Invalida::Algoritmo));
        assert_eq!(verificar(ED25519, "zz", &firma, &e), Err(Invalida::Clave));
        assert_eq!(verificar(ED25519, &pk, "abcd", &e), Err(Invalida::Forma));
    }

    #[test]
    fn el_hexadecimal_va_y_vuelve() {
        assert_eq!(hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(a_hex(&[0, 255, 16]), "00ff10");
        assert_eq!(hex("0f0"), None, "longitud impar");
        assert_eq!(hex("0G"), None, "no es hexadecimal");
        assert_eq!(hex("0F"), None, "mayúsculas: una sola forma de escribirlo");
    }
}
