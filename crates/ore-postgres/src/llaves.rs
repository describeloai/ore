//! **Las llaves** (0058 P4·3·1): dos pares Ed25519 y lo que se firma con ellos.
//!
//! | llave | de quién | firma |
//! |---|---|---|
//! | la del **almacenamiento** (`almacen-jwt-privada`, P2·4) | del almacenamiento | el token de scope `tenant` que lleva cada cómputo para hablar con el pageserver y los safekeepers |
//! | la **propia** (`ore-postgres-llaves`) | de `ore-postgres` | los tokens con que se le habla a cada `compute_ctl` (`:3080`); su JWK va en la especificación |
//!
//! Las dos son PKCS#8 en PEM, como las escribe `openssl` o `cryptography`. Un
//! PKCS#8 de Ed25519 son 48 bytes: un prefijo fijo de 16 y la semilla de 32. Se
//! lee comprobando el prefijo entero —uno de otra curva no casa— y la aritmética
//! la pone `ed25519-compact`, que ya está en el árbol (ver el `Cargo.toml` del
//! espacio de trabajo).
//!
//! ⚠️ Medido en B.5: `compute_ctl` exige `compute_id` en su token, y el `kid`
//!   tiene que estar en el JWKS de la especificación.

use ore_core::json::Json;
use sha2::{Digest, Sha256};

/// El prefijo DER de un PKCS#8 de Ed25519 (RFC 8410): la secuencia, la versión 0,
/// el algoritmo 1.3.101.112 y la cadena de 34 bytes que envuelve la semilla.
const PKCS8_ED25519: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

pub struct Llave {
    par: ed25519_compact::KeyPair,
    /// Los 16 primeros hex del SHA-256 de la pública, como `especificacion.py`.
    pub kid: String,
}

impl Llave {
    /// De un PEM `PRIVATE KEY` (PKCS#8) de Ed25519.
    pub fn de_pem(pem: &str) -> Result<Llave, String> {
        let cuerpo: String = pem
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("-----") && !l.is_empty())
            .collect();
        let der = b64_decodificar(&cuerpo)?;
        if der.len() != 48 || der[..16] != PKCS8_ED25519 {
            return Err("no es una llave privada Ed25519 en PKCS#8".into());
        }
        let mut semilla = [0u8; 32];
        semilla.copy_from_slice(&der[16..]);
        let par = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(semilla));
        let kid = hex(&Sha256::digest(&par.pk[..])[..8]);
        Ok(Llave { par, kid })
    }

    pub fn del_fichero(ruta: &std::path::Path) -> Result<Llave, String> {
        let pem = std::fs::read_to_string(ruta)
            .map_err(|e| format!("la llave `{}`: {e}", ruta.display()))?;
        Llave::de_pem(&pem).map_err(|e| format!("la llave `{}`: {e}", ruta.display()))
    }

    /// Un JWT EdDSA con este cuerpo. Con `kid` en la cabecera si se pide.
    pub fn jwt(&self, cuerpo: &Json, con_kid: bool) -> String {
        let mut cabecera = vec![("alg", Json::s("EdDSA")), ("typ", Json::s("JWT"))];
        if con_kid {
            cabecera.push(("kid", Json::s(&self.kid)));
        }
        let firmado = format!(
            "{}.{}",
            b64url(Json::obj(cabecera).jcs().as_bytes()),
            b64url(cuerpo.jcs().as_bytes())
        );
        let firma = self.par.sk.sign(firmado.as_bytes(), None);
        format!("{firmado}.{}", b64url(&firma[..]))
    }

    /// La pública como JWK, para el JWKS de la especificación.
    pub fn jwk(&self) -> Json {
        Json::obj([
            ("use", Json::s("sig")),
            ("key_ops", Json::Arr(vec![Json::s("verify")])),
            ("alg", Json::s("EdDSA")),
            ("kid", Json::s(&self.kid)),
            ("kty", Json::s("OKP")),
            ("crv", Json::s("Ed25519")),
            ("x", Json::s(b64url(&self.par.pk[..]))),
        ])
    }
}

/// El prefijo DER de una pública Ed25519 (SubjectPublicKeyInfo, RFC 8410).
const SPKI_ED25519: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// Una llave pública Ed25519: para creerse los avisos del `storage_controller`
/// (P4·5), que vienen firmados con la del almacenamiento y `scope: infra`.
pub struct Publica(ed25519_compact::PublicKey);

impl Publica {
    pub fn de_pem(pem: &str) -> Result<Publica, String> {
        let cuerpo: String = pem
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("-----") && !l.is_empty())
            .collect();
        let der = b64_decodificar(&cuerpo)?;
        if der.len() != 44 || der[..12] != SPKI_ED25519 {
            return Err("no es una llave pública Ed25519".into());
        }
        ed25519_compact::PublicKey::from_slice(&der[12..])
            .map(Publica)
            .map_err(|e| format!("la llave pública: {e}"))
    }

    pub fn del_fichero(ruta: &std::path::Path) -> Result<Publica, String> {
        let pem = std::fs::read_to_string(ruta)
            .map_err(|e| format!("la llave `{}`: {e}", ruta.display()))?;
        Publica::de_pem(&pem)
    }

    /// Un JWT EdDSA firmado con la privada de esta pública: su cuerpo, o por qué no.
    pub fn verificar(&self, token: &str) -> Result<ore_core::parse::Node, String> {
        let partes: Vec<&str> = token.trim().split('.').collect();
        let [cabeza, cuerpo, firma] = partes.as_slice() else {
            return Err("no es un JWT".into());
        };
        let leer = |s: &str| b64_decodificar(&s.replace('-', "+").replace('_', "/"));
        let cabecera = String::from_utf8(leer(cabeza)?).map_err(|_| "cabecera no UTF-8")?;
        if !cabecera.contains("\"EdDSA\"") {
            return Err("el algoritmo no es EdDSA".into());
        }
        let firma = ed25519_compact::Signature::from_slice(&leer(firma)?)
            .map_err(|_| "la firma no tiene la forma de una Ed25519")?;
        self.0
            .verify(format!("{cabeza}.{cuerpo}"), &firma)
            .map_err(|_| "la firma no es de esta llave")?;
        let texto = String::from_utf8(leer(cuerpo)?).map_err(|_| "cuerpo no UTF-8")?;
        ore_core::parse::parse(&texto).map_err(|e| format!("el cuerpo no analiza: {e:?}"))
    }
}

/// El token de scope `tenant` que lleva un cómputo (firmado con la del almacenamiento).
pub fn token_de_tenant(almacen: &Llave, tenant: &str) -> String {
    almacen.jwt(
        &Json::obj([("scope", Json::s("tenant")), ("tenant_id", Json::s(tenant))]),
        false,
    )
}

/// Un token para hablarle al `compute_ctl` de un cómputo (firmado con la propia).
pub fn token_de_computo(propia: &Llave, computo: &str, vence: i64) -> String {
    propia.jwt(
        &Json::obj([("compute_id", Json::s(computo)), ("exp", Json::Int(vence))]),
        true,
    )
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

const ALFABETO: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url sin relleno (RFC 4648 §5), lo que lleva un JWT.
pub fn b64url(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for trozo in b.chunks(3) {
        let n = trozo
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, x)| n | (*x as u32) << (16 - 8 * i));
        for i in 0..=trozo.len() {
            s.push(ALFABETO[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    s
}

/// base64 normal (el de un PEM), con o sin relleno.
fn b64_decodificar(s: &str) -> Result<Vec<u8>, String> {
    let mut acumulado: u32 = 0;
    let mut bits = 0;
    let mut fuera = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => continue,
            _ => return Err(format!("carácter `{}` fuera de base64", c as char)),
        };
        acumulado = (acumulado << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            fuera.push((acumulado >> bits) as u8);
        }
    }
    Ok(fuera)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// La llave de ejemplo de RFC 8410 §10.3 (PKCS#8); su pública es la de §10.1.
    const PEM: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC
-----END PRIVATE KEY-----";

    #[test]
    fn base64url_como_la_rfc() {
        assert_eq!(b64url(b""), "");
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
        assert_eq!(b64url(b"foob"), "Zm9vYg");
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn la_llave_de_la_rfc_da_su_publica() {
        let l = Llave::de_pem(PEM).unwrap();
        // RFC 8410 §10.1: la pública que corresponde a esa privada.
        assert_eq!(
            hex(&l.par.pk[..]),
            "19bf44096984cdfe8541bac167dc3b96c85086aa30b6b6cb0c5c38ad703166e1"
        );
        assert_eq!(l.kid.len(), 16);
    }

    #[test]
    fn otra_cosa_no_es_una_llave() {
        assert!(
            Llave::de_pem("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----").is_err()
        );
        assert!(Llave::de_pem("").is_err());
    }

    #[test]
    fn el_jwt_verifica_con_su_publica() {
        let l = Llave::de_pem(PEM).unwrap();
        let t = token_de_computo(&l, "ep-1", 2_000_000_000);
        let partes: Vec<&str> = t.split('.').collect();
        assert_eq!(partes.len(), 3);
        let firma = ed25519_compact::Signature::from_slice(&b64url_a_bytes(partes[2])).unwrap();
        l.par
            .pk
            .verify(format!("{}.{}", partes[0], partes[1]), &firma)
            .unwrap();
        let cuerpo = String::from_utf8(b64url_a_bytes(partes[1])).unwrap();
        assert_eq!(cuerpo, r#"{"compute_id":"ep-1","exp":2000000000}"#);
        let cabecera = String::from_utf8(b64url_a_bytes(partes[0])).unwrap();
        assert!(cabecera.contains(&format!(r#""kid":"{}""#, l.kid)));
    }

    #[test]
    fn una_publica_cree_lo_que_firma_su_privada_y_nada_mas() {
        let l = Llave::de_pem(PEM).unwrap();
        // RFC 8410 §10.1: la pública de esa privada, en SPKI.
        let p = Publica::de_pem(
            "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=
-----END PUBLIC KEY-----",
        )
        .unwrap();
        let t = l.jwt(&Json::obj([("scope", Json::s("infra"))]), false);
        let cuerpo = p.verificar(&t).unwrap();
        assert_eq!(
            cuerpo.get("scope").and_then(|(_, v)| v.as_str()),
            Some("infra")
        );
        // Tocando un byte del cuerpo, ya no.
        let mut partes: Vec<String> = t.split('.').map(String::from).collect();
        partes[1] = b64url(br#"{"scope":"admin"}"#);
        assert!(p.verificar(&partes.join(".")).is_err());
        assert!(p.verificar("a.b").is_err());
    }

    fn b64url_a_bytes(s: &str) -> Vec<u8> {
        b64_decodificar(&s.replace('-', "+").replace('_', "/")).unwrap()
    }
}
