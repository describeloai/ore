//! El token del realm, verificado — y **la llave no se va a buscar**.
//!
//! # La decisión que da forma a este fichero
//!
//! Lo normal en un servidor OIDC es bajarse el JWKS del emisor al arrancar y
//! refrescarlo cada tanto. Aquí **no**: el juego de llaves llega como un
//! fichero, y este proceso lo lee.
//!
//! Es la misma figura que el resto del árbol, tres veces ya:
//!
//! | | quién habla con el mundo |
//! |---|---|
//! | leer un origen | `ore-read-<tipo>`, no `ore` |
//! | subir un artefacto | `ore-store-<tipo>`, no `ore` |
//! | **traer el JWKS** | **un Job con la imagen que tiene TLS, no el servidor** |
//!
//! Y compra tres cosas concretas:
//!
//! - **el plano de control no necesita una pila TLS de salida.** Su
//!   `NetworkPolicy` sigue abriendo sólo la forja y el DNS; si un día alguien
//!   metiera aquí una llamada al emisor, no llegaría;
//! - **el arranque no depende del IdP.** Un emisor caído no impide servir; sólo
//!   impide que lleguen tokens nuevos, que es otra cosa y se ve distinta;
//! - **la rotación es un despliegue**, no un temporizador que a veces falla en
//!   silencio.
//!
//! ⚠️ Y el precio, dicho: **si nadie refresca el fichero, una rotación de
//! llaves del realm deja fuera a todo el mundo.** El fichero no se refresca
//! solo, y quien lo refresca es `malla/50-jwks.yaml`. Escribirlo aquí y no
//! tenerlo allí sería una promesa a medias.
//!
//! # Lo que se comprueba, y en qué orden
//!
//! ```text
//!   1  la forma          tres partes, y la cabecera dice `alg` y `kid`
//!   2  el algoritmo      RS256, y **contra una lista**: `none` no es un alg
//!   3  la llave          la del `kid`, del juego que se cargó
//!   4  la FIRMA          antes de creerse un solo campo del cuerpo
//!   5  el emisor         `iss` exacto
//!   6  la audiencia      `aud` contiene la nuestra
//!   7  el reloj          `exp` y `nbf`, con holgura
//! ```
//!
//! **La firma va antes que los campos**, y no es una preferencia de orden: leer
//! `iss` de un token sin verificar es leerle un dato a quien lo escribió.

use crate::identidad::{Identidad, SinIdentidad};
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use sha2::Sha256;
use std::collections::BTreeMap;

/// Los algoritmos que se aceptan. **Una lista, no una exclusión.**
///
/// Rechazar `none` a mano es la forma de que mañana entre otro que tampoco
/// vale. Aquí lo que no está, no pasa — la misma regla que `mando::HERMETICOS`.
const ALGORITMOS: &[&str] = &["RS256"];

/// Cuánto se le perdona al reloj. Dos máquinas nunca marcan lo mismo, y un
/// token rechazado por dos segundos es un fallo que nadie sabe reproducir.
const HOLGURA: i64 = 60;

/// El juego de llaves, ya en forma utilizable.
pub struct Llaves(BTreeMap<String, RsaPublicKey>);

impl Llaves {
    /// Lee un JWKS. Se queda **sólo** con las RSA de firma que se pueden usar.
    ///
    /// Una llave que no se entiende se salta en silencio y no es dejadez: un
    /// realm publica también llaves de cifrado y algoritmos que no usamos, y
    /// fallar por una de ellas dejaría el servidor sin arrancar por una llave
    /// que nadie iba a mirar.
    pub fn leer(jwks: &str) -> Result<Llaves, String> {
        let arbol =
            ore_core::parse::parse(jwks).map_err(|e| format!("el JWKS no analiza: {e:?}"))?;
        let Some((_, lista)) = arbol.get("keys") else {
            return Err("el JWKS no tiene `keys`".into());
        };
        let mut llaves = BTreeMap::new();
        for k in lista.items() {
            let campo = |n: &str| k.get(n).and_then(|(_, v)| v.as_str()).unwrap_or_default();
            if campo("kty") != "RSA" {
                continue;
            }
            // `use` ausente significa «vale para las dos cosas» (RFC 7517 §4.2).
            if !matches!(campo("use"), "sig" | "") {
                continue;
            }
            let (Ok(n), Ok(e)) = (b64url(campo("n")), b64url(campo("e"))) else {
                continue;
            };
            let kid = campo("kid").to_string();
            if let Ok(llave) =
                RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e))
            {
                llaves.insert(kid, llave);
            }
        }
        if llaves.is_empty() {
            return Err("el JWKS no trae ninguna llave RSA de firma utilizable".into());
        }
        Ok(Llaves(llaves))
    }

    pub fn cuantas(&self) -> usize {
        self.0.len()
    }
}

/// Lo que hay que saber del emisor para creerse un token.
pub struct Emisor {
    /// El `iss` que tiene que decir, exacto.
    pub iss: String,
    /// La audiencia que tiene que contener. **Es nuestra**, y por eso un token
    /// emitido para otro servicio no vale aquí aunque venga del mismo realm.
    pub aud: String,
    pub llaves: Llaves,
}

impl Emisor {
    /// De un `Authorization: Bearer …` a un sujeto, o al motivo de que no.
    pub fn verificar(&self, cabecera: &str, ahora: i64) -> Result<Identidad, SinIdentidad> {
        let token = cabecera
            .strip_prefix("Bearer ")
            .or_else(|| cabecera.strip_prefix("bearer "))
            .ok_or(SinIdentidad::Ausente)?
            .trim();
        if token.is_empty() {
            return Err(SinIdentidad::Ausente);
        }

        let mal = |m: &str| SinIdentidad::Invalida(m.to_string());

        // 1 · la forma
        let partes: Vec<&str> = token.split('.').collect();
        if partes.len() != 3 {
            return Err(mal("el token no tiene tres partes"));
        }
        let cabeza = json(partes[0]).map_err(|_| mal("la cabecera del token no analiza"))?;
        let alg = cadena(&cabeza, "alg").unwrap_or_default();
        let kid = cadena(&cabeza, "kid").unwrap_or_default();

        // 2 · el algoritmo, contra la lista
        if !ALGORITMOS.contains(&alg.as_str()) {
            return Err(mal(&format!(
                "algoritmo `{alg}`: sólo se aceptan {}",
                ALGORITMOS.join(", ")
            )));
        }

        // 3 · la llave
        let llave = self.llaves.0.get(&kid).ok_or_else(|| {
            mal(&format!(
                "no hay llave `{kid}` en el juego cargado. Si el realm rotó, el \
                 fichero de llaves está viejo — se refresca fuera de este proceso"
            ))
        })?;

        // 4 · LA FIRMA, antes de creerse un solo campo del cuerpo
        let firmado = format!("{}.{}", partes[0], partes[1]);
        let firma = b64url(partes[2]).map_err(|_| mal("la firma no es base64url"))?;
        let firma = Signature::try_from(firma.as_slice())
            .map_err(|_| mal("la firma no tiene el tamaño de la llave"))?;
        VerifyingKey::<Sha256>::new(llave.clone())
            .verify(firmado.as_bytes(), &firma)
            .map_err(|_| mal("la firma no es válida"))?;

        // Y sólo ahora, el cuerpo.
        let cuerpo = json(partes[1]).map_err(|_| mal("el cuerpo del token no analiza"))?;

        // 5 · el emisor
        match cadena(&cuerpo, "iss") {
            Some(i) if i == self.iss => {}
            Some(i) => return Err(mal(&format!("el token lo emitió `{i}`, no `{}`", self.iss))),
            None => return Err(mal("el token no dice quién lo emitió")),
        }

        // 6 · la audiencia. `aud` es una cadena o una lista: las dos formas son
        // legales (RFC 7519 §4.1.3) y aceptar sólo una deja fuera medio mundo.
        if !audiencias(&cuerpo).iter().any(|a| a == &self.aud) {
            return Err(mal(&format!(
                "el token no es para `{}`: su audiencia es {:?}",
                self.aud,
                audiencias(&cuerpo)
            )));
        }

        // 7 · el reloj
        if let Some(exp) = entero(&cuerpo, "exp") {
            if ahora > exp + HOLGURA {
                return Err(mal("el token caducó"));
            }
        } else {
            return Err(mal("el token no caduca nunca, y eso no se acepta"));
        }
        if entero(&cuerpo, "nbf").is_some_and(|nbf| ahora + HOLGURA < nbf) {
            return Err(mal("el token todavía no vale"));
        }

        let persona = cadena(&cuerpo, "sub").ok_or_else(|| mal("el token no dice de quién es"))?;

        // Y la delegación, si la trae. RFC 8693 §4.1: `act.sub` es **quién
        // actúa** por la persona. Es exactamente la forma que `Identidad` ya
        // tenía tallada antes de que hubiera un token que la trajera.
        let agente = cuerpo
            .get("act")
            .and_then(|(_, a)| a.get("sub"))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string);

        Ok(Identidad {
            persona,
            agente,
            correo: cadena(&cuerpo, "email"),
            // `name` es el claim estandar de OIDC para el nombre completo.
            nombre: cadena(&cuerpo, "name"),
        })
    }
}

// ── Lectura ─────────────────────────────────────────────────────────────────

use ore_core::parse::Node;

fn json(b64: &str) -> Result<Node, String> {
    let bytes = b64url(b64)?;
    let texto = String::from_utf8(bytes).map_err(|_| "no es UTF-8".to_string())?;
    ore_core::parse::parse(&texto).map_err(|e| format!("{e:?}"))
}

fn cadena(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
}

fn entero(n: &Node, k: &str) -> Option<i64> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .and_then(|s| s.parse().ok())
}

fn audiencias(n: &Node) -> Vec<String> {
    match n.get("aud") {
        None => Vec::new(),
        Some((_, v)) => match v.as_str() {
            Some(s) => vec![s.to_string()],
            None => v
                .items()
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect(),
        },
    }
}

/// base64url sin relleno, escrito a mano.
///
/// Treinta líneas y ninguna dependencia. No es criptografía: es una tabla, y
/// equivocarse en ella da bytes distintos y una firma que no verifica — un
/// fallo ruidoso, que es lo contrario del que hace peligroso escribir RSA a
/// mano.
fn b64url(s: &str) -> Result<Vec<u8>, String> {
    let mut acumulado: u32 = 0;
    let mut bits = 0;
    let mut fuera = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            // El relleno se tolera porque hay emisores que lo ponen; cualquier
            // otro carácter NO, y eso incluye el `+` y el `/` del base64 normal:
            // un token con ellos no es base64url y aceptarlo sería aceptar dos
            // codificaciones para el mismo campo.
            b'=' => continue,
            _ => return Err(format!("carácter `{}` fuera de base64url", c as char)),
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
    use rsa::RsaPrivateKey;
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::traits::PublicKeyParts;

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut s = String::new();
        for trozo in bytes.chunks(3) {
            let b = [
                trozo[0],
                *trozo.get(1).unwrap_or(&0),
                *trozo.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            let cuantos = trozo.len() + 1;
            for i in 0..cuantos {
                s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        s
    }

    /// **Una llave fija, escrita aquí.** No se genera.
    ///
    /// Dos motivos y los dos importan: generar una RSA por prueba tarda, y una
    /// prueba que depende de un generador aleatorio falla un día de cada mil
    /// sin que nadie sepa por qué. Ésta es de 1024 bits porque lo que se prueba
    /// es la CADENA de verificación, no la resistencia de RSA.
    ///
    /// ⛔ Y por si a alguien se le ocurre: **esta clave es pública**, está en
    /// un repositorio y no vale para nada más que para esto.
    const N: &str = "98108802788390257451427544537569571344186445898290802556306202303563898746153694624427727358963513671154354914513738651625770608698713459103692963416659567693829394560290454767975140775489577446428399646018299491037316206274830556376268979724700474004190230381644330590836431547957441542425447960647896094871";
    const D: &str = "58486241606109815346595400118075370902635461720864906313568320457114881061285666040279031389708798321838495691825034032384259760307155288367215166817606418492512751565372832090822858982082061256407746822196621146620704044182961301819536667603341097088576106781350305670874531525425729267744540582701760709001";
    const P: &str = "12922613922745463790944134391245983402900101885290450417130464591514354462035060913154692844874292403894705167594547541490375570416296977031527206228087109";
    const Q: &str = "7592024599272917751470408163358661974578034658831077878519008285342471477204994309097838424350386387366925565400131136857459422116519343986932596361202219";

    fn entera(s: &str) -> BigUint {
        s.parse().unwrap()
    }

    fn banco() -> (RsaPrivateKey, String) {
        let k = RsaPrivateKey::from_components(
            entera(N),
            BigUint::from(65537u32),
            entera(D),
            vec![entera(P), entera(Q)],
        )
        .unwrap();
        let pubk = k.to_public_key();
        let jwks = format!(
            r#"{{"keys":[{{"kty":"RSA","use":"sig","kid":"k1","alg":"RS256","n":"{}","e":"{}"}}]}}"#,
            b64(&pubk.n().to_bytes_be()),
            b64(&pubk.e().to_bytes_be())
        );
        (k, jwks)
    }

    fn token(k: &RsaPrivateKey, cabeza: &str, cuerpo: &str) -> String {
        let firmado = format!("{}.{}", b64(cabeza.as_bytes()), b64(cuerpo.as_bytes()));
        let firma = SigningKey::<Sha256>::new(k.clone()).sign(firmado.as_bytes());
        format!("{firmado}.{}", b64(&firma.to_bytes()))
    }

    fn emisor(jwks: &str) -> Emisor {
        Emisor {
            iss: "https://login.paladio.io/realms/rubix".into(),
            aud: "ore-serve".into(),
            llaves: Llaves::leer(jwks).unwrap(),
        }
    }

    const CABEZA: &str = r#"{"alg":"RS256","typ":"JWT","kid":"k1"}"#;

    fn cuerpo(extra: &str) -> String {
        format!(
            r#"{{"iss":"https://login.paladio.io/realms/rubix","aud":"ore-serve","sub":"persona:ana","exp":2000000000{extra}}}"#
        )
    }

    #[test]
    fn un_token_bueno_da_el_sujeto() {
        let (k, jwks) = banco();
        let t = token(&k, CABEZA, &cuerpo(""));
        let id = emisor(&jwks)
            .verificar(&format!("Bearer {t}"), 1_700_000_000)
            .unwrap();
        assert_eq!(id.persona, "persona:ana");
        assert_eq!(id.agente, None);
    }

    /// RFC 8693: `act.sub` es quién actúa por la persona.
    #[test]
    fn la_delegacion_sale_de_act() {
        let (k, jwks) = banco();
        let t = token(&k, CABEZA, &cuerpo(r#","act":{"sub":"agente:job-7"}"#));
        let id = emisor(&jwks)
            .verificar(&format!("Bearer {t}"), 1_700_000_000)
            .unwrap();
        assert_eq!(id.agente.as_deref(), Some("agente:job-7"));
    }

    /// **El que más importa.** `alg: none` es la vulnerabilidad clásica de JWT,
    /// y aquí no hay que acordarse de excluirla: la lista es de permitidos.
    #[test]
    fn alg_none_no_pasa() {
        let (k, jwks) = banco();
        let t = token(&k, r#"{"alg":"none","kid":"k1"}"#, &cuerpo(""));
        assert!(matches!(
            emisor(&jwks).verificar(&format!("Bearer {t}"), 1_700_000_000),
            Err(SinIdentidad::Invalida(_))
        ));
    }

    /// **El ataque de verdad**: una firma válida, pegada a otro cuerpo.
    ///
    /// Es el caso que separa «verificar» de «leer». El token está bien firmado
    /// —por nuestra llave, con nuestro `kid`— y todos los campos son
    /// plausibles; lo único que no cuadra es que el cuerpo no es el que se
    /// firmó. Un servidor que leyera `sub` antes de comprobar la firma diría
    /// que quien pide es `persona:jefa`.
    #[test]
    fn una_firma_valida_sobre_otro_cuerpo_no_pasa() {
        let (k, jwks) = banco();
        let bueno = token(&k, CABEZA, &cuerpo(""));
        let partes: Vec<&str> = bueno.split('.').collect();

        let otro = r#"{"iss":"https://login.paladio.io/realms/rubix","aud":"ore-serve","sub":"persona:jefa","exp":2000000000}"#;
        let falso = format!("{}.{}.{}", partes[0], b64(otro.as_bytes()), partes[2]);

        match emisor(&jwks).verificar(&format!("Bearer {falso}"), 1_700_000_000) {
            Err(SinIdentidad::Invalida(m)) => {
                assert!(m.contains("firma"), "el motivo no es la firma: {m}")
            }
            otro => panic!("un cuerpo cambiado paso la puerta: {otro:?}"),
        }
    }

    /// Un token del MISMO realm para OTRO servicio. Sin esto, cualquier cliente
    /// del realm entraría aquí con su propio token.
    #[test]
    fn un_token_para_otra_audiencia_no_pasa() {
        let (k, jwks) = banco();
        let c = r#"{"iss":"https://login.paladio.io/realms/rubix","aud":"rubix-api","sub":"persona:ana","exp":2000000000}"#;
        let t = token(&k, CABEZA, c);
        assert!(matches!(
            emisor(&jwks).verificar(&format!("Bearer {t}"), 1_700_000_000),
            Err(SinIdentidad::Invalida(_))
        ));
    }

    #[test]
    fn el_emisor_tiene_que_ser_el_nuestro() {
        let (k, jwks) = banco();
        let c = r#"{"iss":"https://otro/realms/x","aud":"ore-serve","sub":"a","exp":2000000000}"#;
        let t = token(&k, CABEZA, c);
        assert!(matches!(
            emisor(&jwks).verificar(&format!("Bearer {t}"), 1_700_000_000),
            Err(SinIdentidad::Invalida(_))
        ));
    }

    #[test]
    fn un_token_caducado_no_pasa() {
        let (k, jwks) = banco();
        let t = token(&k, CABEZA, &cuerpo(""));
        assert!(matches!(
            emisor(&jwks).verificar(&format!("Bearer {t}"), 2_000_000_999),
            Err(SinIdentidad::Invalida(_))
        ));
    }

    /// Un token sin `exp` es un token para siempre.
    #[test]
    fn un_token_sin_caducidad_no_pasa() {
        let (k, jwks) = banco();
        let c = r#"{"iss":"https://login.paladio.io/realms/rubix","aud":"ore-serve","sub":"a"}"#;
        let t = token(&k, CABEZA, c);
        assert!(matches!(
            emisor(&jwks).verificar(&format!("Bearer {t}"), 1_700_000_000),
            Err(SinIdentidad::Invalida(_))
        ));
    }

    /// Sin cabecera no es lo mismo que con una mala: la primera es un `401`
    /// escueto y la segunda tiene que decir por qué.
    #[test]
    fn ausente_e_invalida_siguen_sin_ser_lo_mismo() {
        let (_, jwks) = banco();
        let e = emisor(&jwks);
        assert_eq!(e.verificar("", 0), Err(SinIdentidad::Ausente));
        assert_eq!(e.verificar("Bearer ", 0), Err(SinIdentidad::Ausente));
        assert!(matches!(
            e.verificar("Bearer no-es-un-token", 0),
            Err(SinIdentidad::Invalida(_))
        ));
    }

    /// El aviso que hace falta cuando el realm rota y el fichero no.
    #[test]
    fn una_llave_desconocida_lo_dice_con_su_nombre() {
        let (k, jwks) = banco();
        let t = token(&k, r#"{"alg":"RS256","kid":"otra"}"#, &cuerpo(""));
        match emisor(&jwks).verificar(&format!("Bearer {t}"), 1_700_000_000) {
            Err(SinIdentidad::Invalida(m)) => {
                assert!(m.contains("otra"), "no nombra el `kid`: {m}");
                assert!(
                    m.contains("rotó") || m.contains("viejo"),
                    "no explica la causa: {m}"
                );
            }
            otro => panic!("se esperaba invalida, salio {otro:?}"),
        }
    }

    #[test]
    fn el_aud_puede_ser_una_lista() {
        let (k, jwks) = banco();
        let c = r#"{"iss":"https://login.paladio.io/realms/rubix","aud":["rubix-api","ore-serve"],"sub":"a","exp":2000000000}"#;
        let t = token(&k, CABEZA, c);
        assert!(
            emisor(&jwks)
                .verificar(&format!("Bearer {t}"), 1_700_000_000)
                .is_ok()
        );
    }

    #[test]
    fn un_jwks_sin_llaves_utiles_se_niega() {
        assert!(Llaves::leer(r#"{"keys":[]}"#).is_err());
        assert!(Llaves::leer(r#"{"keys":[{"kty":"oct","k":"x"}]}"#).is_err());
        assert!(Llaves::leer("no es json").is_err());
    }

    #[test]
    fn base64url_no_acepta_el_alfabeto_del_otro() {
        assert!(b64url("a+b").is_err());
        assert!(b64url("a/b").is_err());
        assert_eq!(b64url("SGVsbG8").unwrap(), b"Hello");
        assert_eq!(b64url("SGVsbG8=").unwrap(), b"Hello");
    }
}
