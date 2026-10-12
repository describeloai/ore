//! **Las contraseñas de los roles, guardadas cifradas** (ADR 0058, P8·1, D-P8a).
//!
//! La consola ejecuta SQL como el rol que se elige, así que el plano necesita su
//! contraseña. Se guarda cifrada con AES-256-GCM y una clave de 32 bytes que vive
//! en un Secret de `ore-pg` (`--clave-contrasenas`), nunca en la base.
//!
//!   v1:<nonce, 12 bytes en hex>:<cifrado + etiqueta, en hex>
//!
//! El dato asociado es `organización/proyecto/rol`: una contraseña cifrada copiada a
//! otro rol no se descifra. No lleva la rama: una rama hereda los roles de su padre
//! con la misma contraseña, y la fila se copia tal cual.
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};

pub struct Secreto {
    cifrador: Aes256Gcm,
}

impl Secreto {
    /// De los 32 bytes de la clave.
    pub fn de_bytes(clave: &[u8]) -> Result<Secreto, String> {
        if clave.len() != 32 {
            return Err(format!(
                "la clave de las contraseñas son 32 bytes, no {}",
                clave.len()
            ));
        }
        Ok(Secreto {
            cifrador: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(clave)),
        })
    }

    /// Del fichero del Secret: 32 bytes tal cual, o 64 cifras hexadecimales (con o sin
    /// salto de línea al final, que es como sale de `openssl rand -hex 32`).
    pub fn del_fichero(ruta: &std::path::Path) -> Result<Secreto, String> {
        let b = std::fs::read(ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
        let texto = String::from_utf8_lossy(&b);
        let t = texto.trim();
        if t.len() == 64 && t.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Secreto::de_bytes(&de_hex(t)?);
        }
        Secreto::de_bytes(&b)
    }

    pub fn cifrar(&self, contrasena: &str, asociado: &str) -> Result<String, String> {
        let nonce = crate::scram::azar(12)?;
        let cifrado = self
            .cifrador
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: contrasena.as_bytes(),
                    aad: asociado.as_bytes(),
                },
            )
            .map_err(|_| "no se pudo cifrar".to_string())?;
        Ok(format!("v1:{}:{}", hex(&nonce), hex(&cifrado)))
    }

    pub fn descifrar(&self, guardada: &str, asociado: &str) -> Result<String, String> {
        let mut partes = guardada.split(':');
        let (Some("v1"), Some(n), Some(c), None) =
            (partes.next(), partes.next(), partes.next(), partes.next())
        else {
            return Err("la contraseña guardada no tiene la forma `v1:…:…`".into());
        };
        let nonce = de_hex(n)?;
        if nonce.len() != 12 {
            return Err("la contraseña guardada no tiene un nonce de 12 bytes".into());
        }
        let claro = self
            .cifrador
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &de_hex(c)?,
                    aad: asociado.as_bytes(),
                },
            )
            .map_err(|_| {
                "la contraseña guardada no se descifra con esta clave y este rol".to_string()
            })?;
        String::from_utf8(claro).map_err(|_| "la contraseña descifrada no es UTF-8".into())
    }
}

/// El dato asociado de la contraseña de un rol.
pub fn asociado(organizacion: &str, proyecto: &str, rol: &str) -> String {
    format!("{organizacion}/{proyecto}/{rol}")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn de_hex(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("hexadecimal de longitud impar".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "no es hexadecimal".to_string()))
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn secreto(b: u8) -> Secreto {
        Secreto::de_bytes(&[b; 32]).unwrap()
    }

    #[test]
    fn ida_y_vuelta() {
        let s = secreto(7);
        let a = asociado("org_1", "ventas", "ana");
        let g = s.cifrar("contraseña-larga", &a).unwrap();
        assert!(g.starts_with("v1:"), "{g}");
        assert!(!g.contains("contraseña"), "{g}");
        assert_eq!(s.descifrar(&g, &a).unwrap(), "contraseña-larga");
        // Dos veces lo mismo no da lo mismo: el nonce es nuevo cada vez.
        assert_ne!(s.cifrar("x", &a).unwrap(), s.cifrar("x", &a).unwrap());
    }

    #[test]
    fn otro_rol_otra_clave_o_un_byte_cambiado_no_descifran() {
        let s = secreto(7);
        let g = s.cifrar("p", &asociado("org_1", "ventas", "ana")).unwrap();
        assert!(
            s.descifrar(&g, &asociado("org_1", "ventas", "bea"))
                .is_err()
        );
        assert!(
            s.descifrar(&g, &asociado("org_2", "ventas", "ana"))
                .is_err()
        );
        assert!(
            secreto(8)
                .descifrar(&g, &asociado("org_1", "ventas", "ana"))
                .is_err()
        );
        let mut tocada = g.clone();
        let ultimo = tocada.pop().unwrap();
        tocada.push(if ultimo == '0' { '1' } else { '0' });
        assert!(
            s.descifrar(&tocada, &asociado("org_1", "ventas", "ana"))
                .is_err()
        );
        assert!(s.descifrar("v2:00:00", "x").is_err());
    }

    #[test]
    fn la_clave_son_32_bytes_o_64_cifras_hex() {
        assert!(Secreto::de_bytes(&[0; 16]).is_err());
        let d = std::env::temp_dir().join(format!("ore-pg-clave-{}", std::process::id()));
        std::fs::write(&d, format!("{}\n", "ab".repeat(32))).unwrap();
        assert!(Secreto::del_fichero(&d).is_ok());
        std::fs::write(&d, [9u8; 32]).unwrap();
        assert!(Secreto::del_fichero(&d).is_ok());
        std::fs::write(&d, "corta").unwrap();
        assert!(Secreto::del_fichero(&d).is_err());
        let _ = std::fs::remove_file(&d);
    }
}
