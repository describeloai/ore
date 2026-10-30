//! **Cómo se abre una sesión con el CockroachDB de un cliente** (ADR 0080, paso 1; ADR 0065 D-PG1).
//!
//! ⚠️ **Copiado de `ore-read-postgres/src/tls.rs`** (`planear`, `conector` y `explicar`), sin el túnel SSH ni el IAM de Google. Es
//! código de seguridad duplicado: si se arregla uno hay que arreglar el otro. Se extrae a una crate común cuando se haga el túnel o
//! Cloud (ADR 0080 §7).
//!
//! `tokio-postgres` sólo conoce `disable`, `prefer` y `require`, y lo que verifica lo decide el conector TLS que se le pase. Aquí se
//! decide antes de que la URL llegue al crate:
//!
//! | `sslmode` | qué hace |
//! |---|---|
//! | ninguno, `require`, `verify-full` | TLS, la cadena **y** el nombre del host |
//! | `verify-ca` | TLS y la cadena; el nombre no |
//! | `disable`, `allow`, `prefer` | **se niegan**: texto plano por internet |
//!
//! **La CA** va en la URL: `sslrootcert=<PEM, codificado en la URL>`, que vive en el cofre con el resto de la credencial (así no hay un
//! fichero que montar en el Job). Con ella, **sólo** esa CA vale, como en libpq; `sslrootcert=system`, el almacén del sistema, es el
//! defecto.
//!
//! Fuera de producción —el laboratorio, la CI— `ORE_COCKROACH_LABORATORIO=1` deja pasar `disable` y `prefer` tal cual (los nodos
//! `--insecure`), y sin `sslmode` vale `prefer`. `require` y `verify-*` verifican igual.

use crate::url;
use ore_driver::{Codigo, Fallo};
use postgres::config::SslMode;
use postgres_native_tls::MakeTlsConnector;
use std::time::Duration;

/// Lo que se verifica del certificado del servidor.
#[derive(Debug, Clone, PartialEq)]
pub enum Modo {
    /// La cadena y el nombre del host.
    Completo,
    /// La cadena; el nombre no.
    SoloCadena,
    /// Sin TLS o con `prefer`: sólo con `ORE_COCKROACH_LABORATORIO=1`.
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
    std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1")
}

/// `%XX` de un valor de la URL. `+` se queda: un PEM en base64 lo lleva, y libpq tampoco lo lee como espacio.
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
            x => s.push(x),
        }
        i += 1;
    }
    String::from_utf8_lossy(&s).into_owned()
}

/// **Lee la URL**: separa `sslmode` y `sslrootcert`, decide y deja el resto intacto. Sólo la forma URL (`postgresql://…?…`): la
/// de pares (`host=… sslmode=…`) no se admite, porque [`url::analizar`] ya la habría rechazado.
pub fn planear(url: &str, laboratorio: bool) -> Result<Plan, Fallo> {
    let mut sslmode: Option<String> = None;
    let mut raiz: Option<String> = None;
    let limpia = match url.split_once('?') {
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
    };

    let modo = match sslmode.as_deref() {
        // En el laboratorio, sin `sslmode` es `prefer`: las URLs de la CI y de las pruebas de fuego no lo llevan.
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
                        "`sslmode={m}` no se admite: deja hablar en claro por internet, con la contraseña y los datos a la vista \
                         (ADR 0065, D-PG1). Quítalo —el defecto es verificar el certificado— o usa `verify-ca` si el certificado \
                         no lleva el nombre del host"
                    ),
                ));
            }
        }
        Some(otro) => {
            return Err(Fallo::new(
                Codigo::Credencial,
                format!(
                    "`sslmode={otro}` no es un modo de CockroachDB (require, verify-ca, verify-full)"
                ),
            ));
        }
    };

    let ca = match raiz.as_deref().map(str::trim) {
        None | Some("") | Some("system") => Vec::new(),
        Some(pem) if pem.contains("-----BEGIN CERTIFICATE-----") => pem
            .split_inclusive("-----END CERTIFICATE-----")
            .filter(|c| c.contains("-----BEGIN CERTIFICATE-----"))
            .map(|c| c.trim().to_string())
            .collect(),
        Some(otro) => {
            return Err(Fallo::new(
                Codigo::Credencial,
                format!(
                    "`sslrootcert` lleva el PEM de la CA dentro de la URL (o `system`), no un fichero ni un nombre: `{}` no es un \
                     certificado",
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

/// El error del crate envuelve el nuestro —«error deserializing column 1», «db error»— y lo que dice de verdad está en la causa.
/// Se sigue la cadena entera.
pub fn causa(e: &dyn std::error::Error) -> String {
    let mut partes = vec![e.to_string()];
    let mut actual = e.source();
    while let Some(c) = actual {
        partes.push(c.to_string());
        actual = c.source();
    }
    partes.join(": ")
}

/// **Lo que se le dice a quien no conecta por TLS**: la causa entera del crate y, si se reconoce, qué hacer.
pub fn explicar(e: &postgres::Error) -> String {
    let causa = causa(e);
    let c = causa.to_lowercase();
    let pista = if c.contains("unable to get local issuer")
        || c.contains("self-signed certificate")
        || c.contains("self signed certificate")
        || c.contains("certificate verify failed") && !c.contains("hostname")
        || c.contains("untrusted root")
    {
        " — el certificado del servidor lo firma una CA que no se conoce: da su CA en `sslrootcert` (el PEM, dentro de la URL)"
    } else if c.contains("hostname mismatch")
        || c.contains("ip address mismatch")
        || c.contains("cn_no_match")
    {
        " — el certificado no lleva este nombre de host: conecta por el nombre que lleva, o usa `sslmode=verify-ca` (sólo la cadena)"
    } else if c.contains("server does not support tls") {
        " — el servidor no habla TLS, y en claro no se conecta (D-PG1): arráncalo con certificados (`--certs-dir`)"
    } else if c.contains("tls handshake") {
        // El texto de la causa lo pone la biblioteca TLS del sistema, y `schannel` (Windows) lo escribe en el idioma del sistema:
        // no se puede reconocer el motivo, pero sí decir las dos cosas que se arreglan en la URL.
        " — falló el handshake TLS: si el servidor usa una CA propia, da su CA en `sslrootcert` (el PEM, dentro de la URL); si el \
         certificado no lleva el nombre de host al que conectas, conecta por el nombre que lleva o usa `sslmode=verify-ca` (sólo la cadena)"
    } else {
        ""
    };
    format!("{causa}{pista}")
}

/// Una sesión abierta: el cliente, el conector TLS con que se abrió (cancelar habla TLS con la misma CA) y el plazo de la URL.
pub struct Abierta {
    pub cliente: postgres::Client,
    pub tls: MakeTlsConnector,
    pub timeout_ms: Option<u64>,
    /// Las conexiones con que se lee una tabla grande (ver [`url::TRAMOS`]).
    pub tramos: usize,
}

/// **Abre una sesión** con `url`: TLS según D-PG1, 10 s para conectar (o `timeoutMs` si es menor), el nombre de la aplicación y de
/// sólo lectura.
pub fn abrir(url_entera: &str) -> Result<Abierta, Fallo> {
    let destino = url::analizar(url_entera)?;
    let plan = planear(&destino.url, laboratorio())?;
    let tls = conector(&plan)?;
    let mut config: postgres::Config = plan.url.parse().map_err(|e| {
        Fallo::new(
            Codigo::Conexion,
            format!("la URL no es de CockroachDB: {e}"),
        )
    })?;
    config.ssl_mode(match &plan.modo {
        Modo::Laboratorio(m) => *m,
        _ => SslMode::Require,
    });
    let plazo = destino
        .timeout_ms
        .map_or(10, |ms| ms.div_ceil(1000).clamp(1, 10));
    config.connect_timeout(Duration::from_secs(plazo));
    config.application_name("ore-read-cockroach");
    let mut cliente = config.connect(tls.clone()).map_err(|e| {
        let mut f = crate::lectura::fallo_de(&e);
        if e.as_db_error().is_none() {
            f.mensaje = format!("no se pudo conectar: {}", explicar(&e));
        }
        f
    })?;
    cliente
        .batch_execute("SET default_transaction_read_only = true")
        .map_err(|e| {
            crate::lectura::en(
                "SET default_transaction_read_only",
                crate::lectura::fallo_de(&e),
            )
        })?;
    // El texto canónico: los instantes en UTC, sin depender de la zona que traiga el servidor o el rol.
    cliente
        .batch_execute("SET TIME ZONE 'UTC'")
        .map_err(|e| crate::lectura::en("SET TIME ZONE", crate::lectura::fallo_de(&e)))?;
    Ok(Abierta {
        cliente,
        tls,
        timeout_ms: destino.timeout_ms,
        tramos: destino.tramos.unwrap_or(url::TRAMOS),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";

    #[test]
    fn sin_sslmode_se_verifica_todo() {
        let p = planear("postgresql://u:p@h/db", false).unwrap();
        assert_eq!(p.modo, Modo::Completo);
        assert_eq!(p.url, "postgresql://u:p@h/db");
    }

    #[test]
    fn require_y_verify_full_son_lo_mismo_y_salen_de_la_url() {
        for m in ["require", "verify-full"] {
            let p = planear(
                &format!("postgresql://u:p@h/db?sslmode={m}&options=x"),
                false,
            )
            .unwrap();
            assert_eq!(p.modo, Modo::Completo);
            assert_eq!(p.url, "postgresql://u:p@h/db?options=x");
        }
    }

    #[test]
    fn verify_ca_no_mira_el_nombre() {
        assert_eq!(
            planear("postgresql://h/db?sslmode=verify-ca", false)
                .unwrap()
                .modo,
            Modo::SoloCadena
        );
    }

    #[test]
    fn el_texto_plano_se_niega_fuera_del_laboratorio() {
        for m in ["disable", "allow", "prefer"] {
            let f = planear(&format!("postgresql://u:p@h/db?sslmode={m}"), false).unwrap_err();
            assert_eq!(f.codigo, Codigo::Credencial, "{m}");
            assert!(f.mensaje.contains("no se admite"), "{}", f.mensaje);
        }
    }

    #[test]
    fn el_laboratorio_deja_pasar_el_texto_plano() {
        let p = planear("postgresql://u@h/db?sslmode=disable", true).unwrap();
        assert_eq!(p.modo, Modo::Laboratorio(SslMode::Disable));
        let p = planear("postgresql://u@h/db", true).unwrap();
        assert_eq!(p.modo, Modo::Laboratorio(SslMode::Prefer));
    }

    #[test]
    fn un_modo_que_no_existe_se_niega() {
        let f = planear("postgresql://h/db?sslmode=casi", false).unwrap_err();
        assert!(f.mensaje.contains("casi"), "{}", f.mensaje);
    }

    #[test]
    fn la_ca_va_en_la_url_codificada() {
        let cod = PEM.replace('\n', "%0A");
        let p = planear(&format!("postgresql://h/db?sslrootcert={cod}"), false).unwrap();
        assert_eq!(p.ca, vec![PEM.to_string()]);
        assert_eq!(p.url, "postgresql://h/db");
    }

    #[test]
    fn la_ca_no_es_un_fichero() {
        let f = planear("postgresql://h/db?sslrootcert=/etc/ca.pem", false).unwrap_err();
        assert!(f.mensaje.contains("no es un certificado"), "{}", f.mensaje);
    }

    #[test]
    fn sslrootcert_system_es_el_defecto() {
        let p = planear("postgresql://h/db?sslrootcert=system", false).unwrap();
        assert!(p.ca.is_empty());
    }

    #[test]
    fn una_ca_con_disable_no_dice_nada() {
        let cod = PEM.replace('\n', "%0A");
        let f = planear(
            &format!("postgresql://h/db?sslmode=disable&sslrootcert={cod}"),
            true,
        )
        .unwrap_err();
        assert!(f.mensaje.contains("no dice nada"), "{}", f.mensaje);
    }
}
