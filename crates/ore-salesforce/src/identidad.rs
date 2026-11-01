//! **La identidad: JWT bearer** (ADR 0083 §1.3, medido en X0 F1 y F2).
//!
//! ```text
//!   aserto = base64url({"alg":"RS256"}) . base64url({iss, sub, aud, exp}) . firma
//!   POST <host>/services/oauth2/token
//!        grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer & assertion=<aserto>
//!   ──▶ { access_token, instance_url, token_type }        (sin `expires_in`)
//! ```
//!
//! - `iss` es la *Consumer Key* de la External Client App; `sub`, el usuario
//!   preautorizado; `aud`, el sitio al que se canjea (el My Domain: medido).
//! - La app necesita **los ámbitos `api` y `refresh_token`** aunque no se emita
//!   ningún refresh token (F1): sin el segundo, `invalid_request`.
//! - El `access_token` **no dice cuándo caduca**. Se guarda lo que Airbyte
//!   guarda (30 minutos) y se renueva antes, o al primer `401`.
//! - La clave privada no se guarda ni se imprime: llega por entorno ([`clave_del_entorno`]).

use ore_driver::base64;
use rsa::RsaPrivateKey;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use sha2::Sha256;

pub const ENV_CLAVE: &str = "ORE_SALESFORCE_CLAVE";
pub const ENV_CLAVE_FICHERO: &str = "ORE_SALESFORCE_CLAVE_FICHERO";

/// Cuánto se confía en un `access_token` sin que lo diga el servidor.
pub const VIDA_DEL_TOKEN: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Lo que dura el aserto: el canje es inmediato, y Salesforce admite hasta 3 minutos.
const VIDA_DEL_ASERTO: u64 = 180;

/// El PEM de la clave: el entorno, o el fichero que el entorno nombra.
pub fn clave_del_entorno() -> Result<String, String> {
    if let Ok(k) = std::env::var(ENV_CLAVE)
        && !k.trim().is_empty()
    {
        return Ok(k);
    }
    match std::env::var(ENV_CLAVE_FICHERO) {
        Ok(ruta) if !ruta.is_empty() => std::fs::read_to_string(&ruta).map_err(|e| {
            format!("no se pudo leer la clave de `{ENV_CLAVE_FICHERO}` ({ruta}): {e}")
        }),
        _ => Err(format!(
            "no hay clave privada: se espera `clave=<PEM>` en la URL, o el PEM en `{ENV_CLAVE}`, o su ruta en `{ENV_CLAVE_FICHERO}`"
        )),
    }
}

/// PKCS#8 (`BEGIN PRIVATE KEY`, lo que da `openssl req -nodes`) o PKCS#1.
pub fn clave(pem: &str) -> Result<RsaPrivateKey, String> {
    let r = if pem.contains("BEGIN RSA PRIVATE KEY") {
        RsaPrivateKey::from_pkcs1_pem(pem).map_err(|e| e.to_string())
    } else {
        RsaPrivateKey::from_pkcs8_pem(pem).map_err(|e| e.to_string())
    };
    // El motivo se dice sin el PEM: un error de parseo no debe llevar la clave.
    r.map_err(|e| format!("la clave privada no es un PEM de RSA que se pueda leer ({e})"))
}

fn b64url(bytes: &[u8]) -> String {
    base64::codificar(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// El aserto firmado. `ahora` en segundos desde la época: se pasa para que
/// la prueba no dependa del reloj.
pub fn aserto(
    cliente: &str,
    usuario: &str,
    audiencia: &str,
    ahora: u64,
    k: &RsaPrivateKey,
) -> String {
    let cabeza = b64url(br#"{"alg":"RS256"}"#);
    let cuerpo = b64url(
        serde_json::json!({
            "iss": cliente,
            "sub": usuario,
            "aud": audiencia,
            "exp": ahora + VIDA_DEL_ASERTO,
        })
        .to_string()
        .as_bytes(),
    );
    let firmado = format!("{cabeza}.{cuerpo}");
    let firma = SigningKey::<Sha256>::new(k.clone()).sign(firmado.as_bytes());
    format!("{firmado}.{}", b64url(&firma.to_bytes()))
}

pub fn ahora() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Lo que Salesforce contesta al canje, dicho con **el arreglo** de lo que ya
/// medimos que falla (F1): el mensaje de la API habla de «connected app» y de
/// ámbitos, y quien lo lee tiene que saber qué tocar.
pub fn explicar_canje(estado: u16, cuerpo: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(cuerpo).unwrap_or_default();
    let error = v["error"].as_str().unwrap_or("");
    let detalle = v["error_description"].as_str().unwrap_or(cuerpo.trim());
    let arreglo = if detalle.contains("refresh_token scope is required") {
        " · arreglo: la app necesita también el ámbito `refresh_token` (aunque no se use) y estar preautorizada para el perfil del usuario"
    } else if detalle.contains("user hasn't approved") || detalle.contains("not approved") {
        " · arreglo: en la app, `Usuarios permitidos` = «aprobados por el administrador están preautorizados», con el perfil o conjunto de permisos del usuario"
    } else if error == "invalid_client_id" || detalle.contains("client identifier invalid") {
        " · arreglo: la Consumer Key no es de esta org, o la app aún no se ha propagado (pueden pasar unos minutos)"
    } else if detalle.contains("invalid assertion") || detalle.contains("signature") {
        " · arreglo: el certificado subido a la app no es el de esta clave privada"
    } else if detalle.contains("audience") {
        " · arreglo: `aud` tiene que ser el dominio de la org (el My Domain), no `login.salesforce.com`"
    } else {
        ""
    };
    format!("el canje del JWT contestó {estado} `{error}`: {detalle}{arreglo}")
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use rsa::BigUint;
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::signature::Verifier;

    /// **Una llave fija, escrita aquí** (la misma de `ore-entrada`, 1024 bits).
    /// No se genera, y ⛔ es **pública**: está en un repositorio y no vale para
    /// nada más que para esta prueba.
    const N: &str = "98108802788390257451427544537569571344186445898290802556306202303563898746153694624427727358963513671154354914513738651625770608698713459103692963416659567693829394560290454767975140775489577446428399646018299491037316206274830556376268979724700474004190230381644330590836431547957441542425447960647896094871";
    const D: &str = "58486241606109815346595400118075370902635461720864906313568320457114881061285666040279031389708798321838495691825034032384259760307155288367215166817606418492512751565372832090822858982082061256407746822196621146620704044182961301819536667603341097088576106781350305670874531525425729267744540582701760709001";
    const P: &str = "12922613922745463790944134391245983402900101885290450417130464591514354462035060913154692844874292403894705167594547541490375570416296977031527206228087109";
    const Q: &str = "7592024599272917751470408163358661974578034658831077878519008285342471477204994309097838424350386387366925565400131136857459422116519343986932596361202219";

    fn llave() -> RsaPrivateKey {
        let e = |s: &str| s.parse::<BigUint>().unwrap();
        RsaPrivateKey::from_components(e(N), BigUint::from(65537u32), e(D), vec![e(P), e(Q)])
            .unwrap()
    }

    fn partes(a: &str) -> (String, serde_json::Value, Vec<u8>) {
        let mut p = a.split('.');
        let (c, u, f) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
        assert!(p.next().is_none(), "un aserto son tres partes");
        let de = |s: &str| {
            let std: String = s.replace('-', "+").replace('_', "/");
            base64::descodificar(&format!("{std}{}", "=".repeat((4 - std.len() % 4) % 4))).unwrap()
        };
        (
            format!("{c}.{u}"),
            serde_json::from_slice(&de(u)).unwrap(),
            de(f),
        )
    }

    #[test]
    fn el_aserto_dice_quien_es_y_se_verifica_con_la_clave_publica() {
        let k = llave();
        let a = aserto(
            "3MVG9abc",
            "victor@x.com",
            "https://org.my.salesforce.com",
            1_000,
            &k,
        );
        let (firmado, cuerpo, firma) = partes(&a);
        assert_eq!(cuerpo["iss"], "3MVG9abc");
        assert_eq!(cuerpo["sub"], "victor@x.com");
        assert_eq!(cuerpo["aud"], "https://org.my.salesforce.com");
        assert_eq!(cuerpo["exp"], 1_000 + 180);
        VerifyingKey::<Sha256>::new(k.to_public_key())
            .verify(
                firmado.as_bytes(),
                &Signature::try_from(firma.as_slice()).unwrap(),
            )
            .expect("la firma es de esta clave");
        // Base64 de URL, sin relleno ni `+` ni `/`: lo que pide un JWT.
        assert!(!a.contains(['=', '+', '/']));
    }

    #[test]
    fn la_cabecera_es_rs256() {
        let a = aserto("c", "u", "a", 0, &llave());
        assert!(a.starts_with("eyJhbGciOiJSUzI1NiJ9."), "{a}");
    }

    #[test]
    fn el_pem_pkcs8_se_lee_y_firma_igual() {
        let k = llave();
        let pem = k.to_pkcs8_pem(LineEnding::LF).unwrap();
        let leida = clave(&pem).expect("el PEM de openssl");
        // RSA PKCS#1 v1.5 es determinista: misma clave, mismo aserto.
        assert_eq!(
            aserto("c", "u", "a", 7, &k),
            aserto("c", "u", "a", 7, &leida)
        );
    }

    #[test]
    fn un_pem_malo_no_lleva_la_clave_en_el_error() {
        let e = clave("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----").unwrap_err();
        assert!(e.contains("no es un PEM"), "{e}");
        assert!(!e.contains("AAAA"), "{e}");
    }

    /// Lo que Salesforce contestó de verdad en X0 (F1), con su arreglo.
    #[test]
    fn el_error_medido_en_x0_dice_que_falta_el_ambito() {
        let m = explicar_canje(
            400,
            r#"{"error":"invalid_request","error_description":"refresh_token scope is required and the connected app should be installed and preauthorized."}"#,
        );
        assert!(m.contains("400 `invalid_request`"), "{m}");
        assert!(m.contains("ámbito `refresh_token`"), "{m}");
    }

    #[test]
    fn un_error_que_no_conocemos_se_cita_tal_cual() {
        let m = explicar_canje(503, "upstream down");
        assert!(m.contains("upstream down") && !m.contains("arreglo"), "{m}");
    }
}
