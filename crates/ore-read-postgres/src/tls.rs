//! **Cómo se abre una sesión con el Postgres de un cliente** (ADR 0065, D-PG1).
//!
//! `tokio-postgres` sólo conoce `disable`, `prefer` y `require`, y lo que
//! verifica lo decide el conector TLS que se le pase. Medido en PG0: sin
//! `sslmode` valía `prefer`, y contra un servidor sin TLS la sesión iba **en
//! claro**; una CA privada (RDS, Cloud SQL, Supabase) no había forma de darla;
//! y `verify-full`, lo que trae la guía de RDS, era «invalid connection
//! string». Aquí se decide todo eso, antes de que la URL llegue al crate:
//!
//! | `sslmode` | qué hace |
//! |---|---|
//! | ninguno, `require`, `verify-full` | TLS, la cadena **y** el nombre del host |
//! | `verify-ca` | TLS y la cadena; el nombre no (Cloud SQL por IP) |
//! | `disable`, `allow`, `prefer` | **se niegan**: texto plano por internet |
//!
//! **La CA** va en la URL: `sslrootcert=<PEM, codificado en la URL>`, que vive
//! en el cofre con el resto de la credencial (no es secreta, pero así no hay
//! un fichero que montar en el Job). Con ella, **sólo** esa CA vale, como en
//! libpq; `sslrootcert=system`, el almacén del sistema, que es el defecto.
//!
//! Fuera de producción —el laboratorio, la CI— `ORE_PG_LABORATORIO=1` deja
//! pasar `disable` y `prefer` tal cual, y sin `sslmode` vale `prefer` (lo de antes):
//! las URLs de la CI no lo llevan. `require` y `verify-*` verifican igual.

use crate::texto;
use ore_driver::{Codigo, Fallo};
use postgres::config::SslMode;
use postgres_native_tls::MakeTlsConnector;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Lo que se verifica del certificado del servidor.
#[derive(Debug, Clone, PartialEq)]
pub enum Modo {
    /// La cadena y el nombre del host.
    Completo,
    /// La cadena; el nombre no.
    SoloCadena,
    /// Sin TLS o con `prefer`: sólo con `ORE_PG_LABORATORIO=1`.
    Laboratorio(SslMode),
}

/// La URL ya sin `sslmode` ni `sslrootcert`, lo que se verifica y la CA.
#[derive(Debug)]
pub struct Plan {
    pub url: String,
    pub modo: Modo,
    pub ca: Vec<String>,
}

fn laboratorio() -> bool {
    std::env::var("ORE_PG_LABORATORIO").as_deref() == Ok("1")
}

/// `%XX` y `+` de un valor de la URL.
fn descodificar(v: &str) -> String {
    let b = v.as_bytes();
    let mut s = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
                {
                    Ok(x) => {
                        s.push(x);
                        i += 3;
                        continue;
                    }
                    Err(_) => s.push(b'%'),
                }
            }
            b'+' => s.push(b' '),
            x => s.push(x),
        }
        i += 1;
    }
    String::from_utf8_lossy(&s).into_owned()
}

/// **Lee la URL**: separa `sslmode` y `sslrootcert`, decide y deja el resto
/// intacto. Admite la forma URL (`postgres://…?…`) y la de pares
/// (`host=… sslmode=…`).
pub fn planear(url: &str, laboratorio: bool) -> Result<Plan, Fallo> {
    let mut sslmode: Option<String> = None;
    let mut raiz: Option<String> = None;
    let limpia = if url.contains("://") {
        match url.split_once('?') {
            None => url.to_string(),
            Some((base, consulta)) => {
                let mut quedan = Vec::new();
                for par in consulta.split('&').filter(|p| !p.is_empty()) {
                    let (k, v) = par.split_once('=').unwrap_or((par, ""));
                    match k {
                        "sslmode" => sslmode = Some(descodificar(v)),
                        "sslrootcert" => raiz = Some(descodificar(v)),
                        _ => quedan.push(par),
                    }
                }
                if quedan.is_empty() {
                    base.to_string()
                } else {
                    format!("{base}?{}", quedan.join("&"))
                }
            }
        }
    } else {
        let mut quedan = Vec::new();
        for t in url.split_whitespace() {
            match t.split_once('=') {
                Some(("sslmode", v)) => sslmode = Some(v.trim_matches('\'').to_string()),
                Some(("sslrootcert", v)) => raiz = Some(v.trim_matches('\'').to_string()),
                _ => quedan.push(t),
            }
        }
        quedan.join(" ")
    };

    let modo = match sslmode.as_deref() {
        // En el laboratorio, sin `sslmode` es lo de siempre (`prefer`): las
        // URLs de la CI y de las pruebas de fuego no lo llevan.
        None if laboratorio => Modo::Laboratorio(SslMode::Prefer),
        None | Some("require") | Some("verify-full") => Modo::Completo,
        Some("verify-ca") => Modo::SoloCadena,
        Some(m @ ("disable" | "allow" | "prefer")) => {
            if laboratorio {
                Modo::Laboratorio(if m == "disable" {
                    SslMode::Disable
                } else {
                    SslMode::Prefer
                })
            } else {
                return Err(Fallo::new(
                    Codigo::Credencial,
                    format!(
                        "`sslmode={m}` no se admite: deja hablar en claro por internet, con la \
                         contraseña y los datos a la vista (ADR 0065, D-PG1). Quítalo —el defecto es \
                         verificar el certificado— o usa `verify-ca` si el certificado no lleva el \
                         nombre del host"
                    ),
                ));
            }
        }
        Some(otro) => {
            return Err(Fallo::new(
                Codigo::Credencial,
                format!(
                    "`sslmode={otro}` no es un modo de Postgres (require, verify-ca, verify-full)"
                ),
            ));
        }
    };

    let ca = match raiz.as_deref().map(str::trim) {
        None | Some("") | Some("system") => Vec::new(),
        Some(pem) if pem.contains("-----BEGIN CERTIFICATE-----") => {
            let certs: Vec<String> = pem
                .split_inclusive("-----END CERTIFICATE-----")
                .filter(|c| c.contains("-----BEGIN CERTIFICATE-----"))
                .map(|c| c.trim().to_string())
                .collect();
            certs
        }
        Some(otro) => {
            return Err(Fallo::new(
                Codigo::Credencial,
                format!(
                    "`sslrootcert` lleva el PEM de la CA dentro de la URL (o `system`), no un \
                     fichero ni un nombre: `{}` no es un certificado",
                    otro.chars().take(40).collect::<String>()
                ),
            ));
        }
    };
    if matches!(modo, Modo::Laboratorio(SslMode::Disable)) && !ca.is_empty() {
        return Err(Fallo::new(
            Codigo::Credencial,
            "`sslrootcert` con `sslmode=disable` no dice nada",
        ));
    }
    Ok(Plan {
        url: limpia,
        modo,
        ca,
    })
}

/// El conector de un plan.
pub fn conector(plan: &Plan) -> Result<MakeTlsConnector, Fallo> {
    let mut b = native_tls::TlsConnector::builder();
    if !plan.ca.is_empty() {
        // Como libpq: con `sslrootcert`, sólo esa CA.
        b.disable_built_in_roots(true);
        for pem in &plan.ca {
            let c = native_tls::Certificate::from_pem(pem.as_bytes()).map_err(|e| {
                Fallo::new(
                    Codigo::Credencial,
                    format!("el PEM de `sslrootcert` no se lee: {e}"),
                )
            })?;
            b.add_root_certificate(c);
        }
    }
    if plan.modo == Modo::SoloCadena {
        b.danger_accept_invalid_hostnames(true);
    }
    b.build()
        .map(MakeTlsConnector::new)
        .map_err(|e| Fallo::origen(format!("no se pudo preparar TLS: {e}")))
}

/// Los conectores de las sesiones abiertas, por URL: cancelar en el origen
/// tiene que hablar TLS con la misma CA (Neon enruta por SNI).
fn conectores() -> &'static Mutex<HashMap<String, MakeTlsConnector>> {
    static C: OnceLock<Mutex<HashMap<String, MakeTlsConnector>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// El conector con el que se abrió la sesión de `url`.
pub fn conector_de(url: &str) -> Option<MakeTlsConnector> {
    conectores().lock().ok()?.get(url).cloned()
}

/// **Lo que se le dice a quien no conecta por TLS**: la causa entera del
/// crate y, si se reconoce, qué hacer.
pub fn explicar(e: &postgres::Error) -> String {
    let causa = texto::causa(e);
    let c = causa.to_lowercase();
    let pista = if c.contains("unable to get local issuer")
        || c.contains("self-signed certificate")
        || c.contains("self signed certificate")
        || c.contains("certificate verify failed") && !c.contains("hostname")
        || c.contains("untrusted root")
    {
        " — el certificado del servidor lo firma una CA que no se conoce: da su CA en `sslrootcert` \
         (el PEM, dentro de la URL)"
    } else if c.contains("hostname mismatch")
        || c.contains("ip address mismatch")
        || c.contains("cn_no_match")
    {
        " — el certificado no lleva este nombre de host: conecta por el nombre que lleva, o usa \
         `sslmode=verify-ca` (sólo la cadena)"
    } else if c.contains("server does not support tls") {
        " — el servidor no habla TLS, y en claro no se conecta (D-PG1): activa `ssl = on` en el servidor"
    } else {
        ""
    };
    format!("{causa}{pista}")
}

/// **Abre una sesión** con `url`: TLS según D-PG1, 10 s para conectar, el
/// nombre de la aplicación y de sólo lectura.
pub fn abrir(url: &str) -> Result<postgres::Client, Fallo> {
    let plan = planear(url, laboratorio())?;
    let tls = conector(&plan)?;
    let mut config: postgres::Config = plan
        .url
        .parse()
        .map_err(|e| Fallo::new(Codigo::Conexion, format!("la URL no es de Postgres: {e}")))?;
    config.ssl_mode(match &plan.modo {
        Modo::Laboratorio(m) => *m,
        _ => SslMode::Require,
    });
    config.connect_timeout(Duration::from_secs(10));
    config.application_name("ore-read-postgres");
    let mut c = config.connect(tls.clone()).map_err(|e| {
        let mut f = crate::lectura::fallo_de(&e);
        if e.as_db_error().is_none() {
            f.mensaje = format!("no se pudo conectar: {}", explicar(&e));
        }
        f
    })?;
    c.batch_execute("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")
        .map_err(|e| crate::lectura::fallo_de(&e))?;
    if let Ok(mut m) = conectores().lock() {
        m.insert(url.to_string(), tls);
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";

    #[test]
    fn sin_sslmode_se_verifica_todo() {
        let p = planear("postgres://u:p@h/db", false).unwrap();
        assert_eq!(p.modo, Modo::Completo);
        assert_eq!(p.url, "postgres://u:p@h/db");
    }

    #[test]
    fn require_y_verify_full_son_lo_mismo_y_salen_de_la_url() {
        for m in ["require", "verify-full"] {
            let p = planear(
                &format!("postgres://u:p@h/db?sslmode={m}&channel_binding=require"),
                false,
            )
            .unwrap();
            assert_eq!(p.modo, Modo::Completo);
            assert_eq!(p.url, "postgres://u:p@h/db?channel_binding=require");
        }
    }

    #[test]
    fn verify_ca_no_mira_el_nombre() {
        assert_eq!(
            planear("postgres://h/db?sslmode=verify-ca", false)
                .unwrap()
                .modo,
            Modo::SoloCadena
        );
    }

    #[test]
    fn en_claro_se_niega_salvo_en_el_laboratorio() {
        assert_eq!(
            planear("postgres://h/db", true).unwrap().modo,
            Modo::Laboratorio(SslMode::Prefer)
        );
        assert_eq!(
            planear("postgres://h/db?sslmode=require", true)
                .unwrap()
                .modo,
            Modo::Completo
        );
        for m in ["disable", "allow", "prefer"] {
            let e = planear(&format!("postgres://h/db?sslmode={m}"), false).unwrap_err();
            assert!(e.mensaje.contains("no se admite"), "{}", e.mensaje);
        }
        assert_eq!(
            planear("postgres://h/db?sslmode=disable", true)
                .unwrap()
                .modo,
            Modo::Laboratorio(SslMode::Disable)
        );
        assert_eq!(
            planear("postgres://h/db?sslmode=prefer", true)
                .unwrap()
                .modo,
            Modo::Laboratorio(SslMode::Prefer)
        );
    }

    #[test]
    fn la_ca_va_en_la_url_codificada() {
        let cod = PEM
            .replace('\n', "%0A")
            .replace(' ', "%20")
            .replace('+', "%2B");
        let p = planear(
            &format!("postgres://h/db?sslrootcert={cod}&application_name=x"),
            false,
        )
        .unwrap();
        assert_eq!(p.ca, vec![PEM.to_string()]);
        assert_eq!(p.url, "postgres://h/db?application_name=x");
        let dos = format!("{PEM}\n{PEM}")
            .replace('\n', "%0A")
            .replace(' ', "%20");
        assert_eq!(
            planear(&format!("postgres://h/db?sslrootcert={dos}"), false)
                .unwrap()
                .ca
                .len(),
            2
        );
    }

    #[test]
    fn un_fichero_no_es_una_ca() {
        let e = planear("postgres://h/db?sslrootcert=/etc/ca.pem", false).unwrap_err();
        assert!(e.mensaje.contains("no un"), "{}", e.mensaje);
        assert!(
            planear("postgres://h/db?sslrootcert=system", false)
                .unwrap()
                .ca
                .is_empty()
        );
    }

    #[test]
    fn la_forma_de_pares_tambien() {
        let p = planear("host=h dbname=db sslmode=verify-ca", false).unwrap();
        assert_eq!(p.modo, Modo::SoloCadena);
        assert_eq!(p.url, "host=h dbname=db");
        assert!(planear("host=h sslmode=disable", false).is_err());
    }

    #[test]
    fn un_modo_inventado_se_dice() {
        assert!(
            planear("postgres://h/db?sslmode=verify", false)
                .unwrap_err()
                .mensaje
                .contains("no es un modo")
        );
    }
}
