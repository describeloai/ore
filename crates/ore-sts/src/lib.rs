//! **La federación con AWS** (0046 E9b): la celda no guarda una clave del
//! cliente; el cliente crea en SU cuenta un rol que confía en la cuenta de
//! Google de la celda, y esto canjea el token de identidad de esa cuenta por una
//! credencial temporal del rol (`AssumeRoleWithWebIdentity`).
//!
//! # La coordenada
//!
//! La misma URL de siempre, con `role_arn` en lugar de la clave:
//!
//! ```text
//!   s3://<bucket>[/<prefijo>]?region=eu-north-1&role_arn=arn:aws:iam::<cuenta>:role/<rol>
//! ```
//!
//! [`resolver`] la devuelve con `access_key_id`, `secret_access_key` y
//! `session_token` en lugar de `role_arn` —la forma que `ore-sigv4` ya lee—, y
//! con cuándo caduca. Sin `role_arn`, la devuelve tal cual.
//!
//! # Lo medido (2026-09-30, desde pods de victor y demo)
//!
//! - El token de Google (`/identity?audience=sts.amazonaws.com`): 0,2 s. Lleva
//!   `iss` = `https://accounts.google.com`, y `sub` = `azp` = el ID único de la
//!   cuenta de servicio. AWS reconoce ese emisor sin dar de alta un proveedor.
//! - STS: 0,3 s y una hora de credencial. La cuenta de OTRA celda: `403
//!   AccessDenied`. Otra audiencia: `400`.
//! - ⛔ `AssumeRoleWithWebIdentity` **no admite `ExternalId`**. Lo que aquí
//!   hace de *external ID* es que la confianza del rol nombra el `sub` de las
//!   cuentas de UNA celda: otro cliente de la plataforma corre con otras.
//!
//! # Lo que esto no hace
//!
//! No lee un bucket (no depende de `ore-s3`): canjea un token. Y no renueva:
//! quien trabaje más de una hora con la credencial la vuelve a pedir.

use ore_core::json::Json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// La audiencia del token de Google que STS acepta (`accounts.google.com:oaud`
/// en la confianza del rol).
pub const AUDIENCIA: &str = "sts.amazonaws.com";

/// El STS global: medido que vale para un bucket de `eu-north-1`.
pub const STS: &str = "https://sts.amazonaws.com/";

/// Cuánto se pide: la hora que un rol da por defecto (`MaxSessionDuration`).
pub const DURACION: u64 = 3600;

/// ⚠️ Sólo para probar: otro STS (el S3 de mentira de las pruebas de fuego).
const STS_DE_PRUEBA: &str = "ORE_STS_URL";

/// Una credencial temporal. `caduca_ms` es prudente: se cuenta desde ANTES de
/// pedirla, así que nunca es más tarde que la de verdad.
pub struct Temporal {
    pub clave: String,
    pub secreto: String,
    pub token: String,
    pub caduca_ms: u64,
}

impl std::fmt::Debug for Temporal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Temporal")
            .field(
                "clave",
                &format!("{}****", self.clave.get(..4).unwrap_or("")),
            )
            .field("caduca_ms", &self.caduca_ms)
            .finish_non_exhaustive()
    }
}

pub fn ahora_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `%XX` → el byte; `+` es `+` (un secreto de AWS lo lleva).
fn descodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Todo lo que no es «no reservado» (RFC 3986), en `%XX`: un secreto lleva `+`
/// y `/`, y un token de sesión `=`.
fn codificar(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// El `role_arn` de la URL, si lo lleva.
pub fn rol_de(url: &str) -> Option<String> {
    let (_, consulta) = url.trim().split_once('?')?;
    consulta
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| descodificar(k) == "role_arn")
        .map(|(_, v)| descodificar(v))
        .filter(|v| !v.is_empty())
}

/// ⛔ Un ARN de rol de IAM y nada más: es lo que va en la petición a STS.
fn admisible(rol: &str) -> Result<(), String> {
    let (cuenta, nombre) = rol
        .strip_prefix("arn:aws:iam::")
        .and_then(|r| r.split_once(":role/"))
        .ok_or_else(|| {
            format!("`{rol}` no es el ARN de un rol: `arn:aws:iam::<cuenta>:role/<nombre>`")
        })?;
    if cuenta.len() != 12 || !cuenta.bytes().all(|b| b.is_ascii_digit()) || nombre.is_empty() {
        return Err(format!(
            "`{rol}` no es el ARN de un rol: `arn:aws:iam::<cuenta de 12 cifras>:role/<nombre>`"
        ));
    }
    Ok(())
}

/// La URL con la credencial temporal en lugar del rol. Lo demás (la región, un
/// `endpoint`) se queda como estaba.
pub fn reescribir(url: &str, t: &Temporal) -> String {
    let url = url.trim();
    let (base, consulta) = url.split_once('?').unwrap_or((url, ""));
    let mut ps: Vec<String> = consulta
        .split('&')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let k = descodificar(p.split_once('=').map_or(*p, |(k, _)| k));
            ![
                "role_arn",
                "access_key_id",
                "secret_access_key",
                "session_token",
            ]
            .contains(&k.as_str())
        })
        .map(String::from)
        .collect();
    ps.push(format!("access_key_id={}", codificar(&t.clave)));
    ps.push(format!("secret_access_key={}", codificar(&t.secreto)));
    ps.push(format!("session_token={}", codificar(&t.token)));
    format!("{base}?{}", ps.join("&"))
}

/// El contenido de la primera `<k>…</k>` de una respuesta de STS.
fn etiqueta(xml: &str, k: &str) -> Option<String> {
    let a = format!("<{k}>");
    let i = xml.find(&a)? + a.len();
    let j = xml[i..].find(&format!("</{k}>"))? + i;
    Some(
        xml[i..j]
            .trim()
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&"),
    )
}

/// Lo que STS devuelve → la credencial.
pub fn de_respuesta(xml: &str, caduca_ms: u64) -> Result<Temporal, String> {
    let campo = |k| {
        etiqueta(xml, k)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("STS no devolvió `{k}`"))
    };
    Ok(Temporal {
        clave: campo("AccessKeyId")?,
        secreto: campo("SecretAccessKey")?,
        token: campo("SessionToken")?,
        caduca_ms,
    })
}

/// El `sub` de un JWT (sin verificarlo: sólo para decir en un error qué cuenta
/// tiene que nombrar la confianza).
fn sub_de(jwt: &str) -> Option<String> {
    let carga = jwt.split('.').nth(1)?;
    let bytes = ore_gcp::de_base64(carga).ok()?;
    let n = ore_core::parse::parse(&String::from_utf8(bytes).ok()?).ok()?;
    n.get("sub").and_then(|(_, v)| v.as_str().map(String::from))
}

/// Un error de STS, dicho de forma que se pueda arreglar en la consola de AWS.
fn de_error(codigo: u16, xml: &str, web: &str, rol: &str) -> String {
    let c = etiqueta(xml, "Code").unwrap_or_else(|| "?".into());
    let m = etiqueta(xml, "Message").unwrap_or_else(|| xml.trim().chars().take(200).collect());
    let pista = match (c.as_str(), sub_de(web)) {
        ("AccessDenied", Some(sub)) => format!(
            ". La confianza de `{rol}` tiene que nombrar a esta celda: \
             `accounts.google.com:aud` y `:sub` = `{sub}`, y `:oaud` = `{AUDIENCIA}`"
        ),
        _ => String::new(),
    };
    format!("AWS no dejó asumir el rol ({codigo} {c}): {m}{pista}")
}

/// `AssumeRoleWithWebIdentity` con el token `web`. `sesion` sale en CloudTrail
/// del cliente (`assumed-role/<rol>/<sesion>`): quién de la celda la pidió.
pub fn asumir(
    endpoint: &str,
    rol: &str,
    sesion: &str,
    web: &str,
    segundos: u64,
) -> Result<Temporal, String> {
    admisible(rol)?;
    let antes = ahora_ms();
    let segundos_txt = segundos.to_string();
    let r = ore_gcp::cliente()?
        .post(endpoint)
        .timeout(Duration::from_secs(20))
        .send_form(&[
            ("Action", "AssumeRoleWithWebIdentity"),
            ("Version", "2011-06-15"),
            ("RoleArn", rol),
            ("RoleSessionName", sesion),
            ("WebIdentityToken", web),
            ("DurationSeconds", &segundos_txt),
        ]);
    match r {
        Ok(ok) => {
            let xml = ok
                .into_string()
                .map_err(|e| format!("la respuesta de STS no se pudo leer: {e}"))?;
            de_respuesta(&xml, antes + segundos * 1000)
        }
        Err(ureq::Error::Status(c, r)) => {
            Err(de_error(c, &r.into_string().unwrap_or_default(), web, rol))
        }
        Err(e) => Err(format!("no se pudo hablar con STS ({endpoint}): {e}")),
    }
}

/// ⭐ La URL de una fuente, lista para `ore-sigv4`: si lleva `role_arn`, con la
/// credencial temporal del rol y cuándo caduca; si no, tal cual y `None`.
pub fn resolver(url: &str, sesion: &str) -> Result<(String, Option<u64>), String> {
    let Some(rol) = rol_de(url) else {
        return Ok((url.to_string(), None));
    };
    admisible(&rol)?;
    let web = ore_gcp::identidad(AUDIENCIA)?;
    let endpoint = std::env::var(STS_DE_PRUEBA)
        .ok()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| STS.to_string());
    let t = asumir(&endpoint, &rol, sesion, &web, DURACION)?;
    Ok((reescribir(url, &t), Some(t.caduca_ms)))
}

/// Lo que imprime `ore-asumir-rol`: `{url, caduca_ms}` (`caduca_ms` sólo si hubo rol).
pub fn salida(url: &str, caduca_ms: Option<u64>) -> Json {
    let mut o = vec![("url", Json::s(url))];
    if let Some(c) = caduca_ms {
        o.push(("caduca_ms", Json::Int(c as i64)));
    }
    Json::obj(o)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn t() -> Temporal {
        Temporal {
            clave: "ASIAEJEMPLO".into(),
            secreto: "a/b+c".into(),
            token: "tok=en/+".into(),
            caduca_ms: 7,
        }
    }

    #[test]
    fn el_rol_sale_de_la_url() {
        let u = "s3://cubo/Nueva%20carpeta?region=eu-north-1&role_arn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2Flector";
        assert_eq!(
            rol_de(u).as_deref(),
            Some("arn:aws:iam::123456789012:role/lector")
        );
        assert_eq!(rol_de("s3://cubo?region=x&access_key_id=a"), None);
        assert_eq!(rol_de("s3://cubo"), None);
    }

    #[test]
    fn reescrita_lleva_la_credencial_y_no_el_rol() {
        let u = "s3://cubo/p?region=eu-north-1&role_arn=arn:aws:iam::123456789012:role/lector";
        let r = reescribir(u, &t());
        assert!(r.starts_with("s3://cubo/p?region=eu-north-1&"), "{r}");
        assert!(!r.contains("role_arn"), "{r}");
        // `+`, `/` y `=` van escapados: `ore-sigv4` lee `+` como `+`, no como espacio.
        assert!(r.contains("secret_access_key=a%2Fb%2Bc"), "{r}");
        assert!(r.contains("session_token=tok%3Den%2F%2B"), "{r}");
        assert_eq!(descodificar("a%2Fb%2Bc"), "a/b+c");
    }

    #[test]
    fn solo_un_arn_de_rol() {
        assert!(admisible("arn:aws:iam::123456789012:role/lector").is_ok());
        assert!(admisible("arn:aws:iam::123456789012:role/ruta/lector").is_ok());
        for malo in [
            "",
            "arn:aws:iam::123456789012:user/alguien",
            "arn:aws:iam::12345:role/x",
            "arn:aws:iam::123456789012:role/",
            "https://evil/?x",
        ] {
            assert!(admisible(malo).is_err(), "{malo}");
        }
    }

    #[test]
    fn la_respuesta_de_sts() {
        // La forma de `AssumeRoleWithWebIdentityResponse` (recortada).
        let xml = "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>\
            <Credentials><AccessKeyId>ASIAX</AccessKeyId><SecretAccessKey>s/k+1</SecretAccessKey>\
            <SessionToken>T0k==</SessionToken><Expiration>2026-09-30T13:45:24Z</Expiration></Credentials>\
            </AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>";
        let c = de_respuesta(xml, 99).unwrap();
        assert_eq!(
            (
                c.clave.as_str(),
                c.secreto.as_str(),
                c.token.as_str(),
                c.caduca_ms
            ),
            ("ASIAX", "s/k+1", "T0k==", 99)
        );
        assert!(de_respuesta("<Error/>", 0).is_err());
        // Y el Debug no enseña la credencial.
        let d = format!("{c:?}");
        assert!(
            !d.contains("s/k+1") && !d.contains("T0k") && !d.contains("ASIAX"),
            "{d}"
        );
    }

    #[test]
    fn el_error_dice_que_cuenta_nombrar() {
        // Un JWT cuya carga es {"sub":"1234"} (sin firma: sólo se lee).
        let web = format!("x.{}.y", "eyJzdWIiOiIxMjM0In0");
        let xml = "<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code>\
            <Message>Not authorized to perform sts:AssumeRoleWithWebIdentity</Message></Error></ErrorResponse>";
        let e = de_error(403, xml, &web, "arn:aws:iam::123456789012:role/lector");
        assert!(
            e.contains("AccessDenied") && e.contains("`1234`") && e.contains(AUDIENCIA),
            "{e}"
        );
    }

    #[test]
    fn sin_rol_la_url_pasa_tal_cual() {
        let u = "s3://cubo?region=x&access_key_id=a&secret_access_key=b";
        assert_eq!(resolver(u, "prueba").unwrap(), (u.to_string(), None));
    }

    #[test]
    fn contra_un_sts_de_mentira() {
        use std::io::{BufRead, BufReader, Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut n = 0usize;
            loop {
                let mut linea = String::new();
                r.read_line(&mut linea).unwrap();
                if let Some(v) = linea.to_ascii_lowercase().strip_prefix("content-length:") {
                    n = v.trim().parse().unwrap();
                }
                if linea == "\r\n" {
                    break;
                }
            }
            let mut cuerpo = vec![0; n];
            r.read_exact(&mut cuerpo).unwrap();
            let cuerpo = String::from_utf8(cuerpo).unwrap();
            let xml = "<R><Credentials><AccessKeyId>ASIAM</AccessKeyId><SecretAccessKey>S</SecretAccessKey>\
                <SessionToken>T</SessionToken></Credentials></R>";
            write!(
                s,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{xml}",
                xml.len()
            )
            .unwrap();
            cuerpo
        });
        let antes = ahora_ms();
        let c = asumir(
            &url,
            "arn:aws:iam::123456789012:role/lector",
            "ore-prueba",
            "jwt",
            3600,
        )
        .unwrap();
        let pedido = h.join().unwrap();
        assert!(
            pedido.contains("Action=AssumeRoleWithWebIdentity"),
            "{pedido}"
        );
        assert!(pedido.contains("RoleSessionName=ore-prueba"), "{pedido}");
        assert!(pedido.contains("WebIdentityToken=jwt"), "{pedido}");
        assert_eq!(c.clave, "ASIAM");
        assert!(c.caduca_ms >= antes + 3_600_000 && c.caduca_ms <= ahora_ms() + 3_600_000);
    }
}
