//! **El TLS del cliente** (ficha 0070, §TLS real): qué certificado se acepta y por qué, y cómo se
//! cuenta cuando no se acepta.
//!
//! # Qué se verifica
//!
//! El driver (`scylla`) verifica el certificado del servidor contra la **dirección IP del nodo**
//! (`X509_VERIFY_PARAM_set1_ip`) y **nunca contra un nombre**. Medido: un certificado válido solo para
//! `cassandra.mi-empresa.com` se rechaza aunque la URL diga exactamente eso. Con certificados «de
//! nombre» —los normales—, o con un balanceador, NAT o Kubernetes por delante, no se podía conectar.
//!
//! Aquí el certificado se acepta si:
//!
//! 1. **la cadena** llega a una CA de confianza (el almacén del sistema, más `SSL_CERT_FILE` y
//!    `SSL_CERT_DIR`): no cambia, y **cualquier** fallo de cadena —otra CA, caducado, incompleto,
//!    revocado— es un error;
//! 2. y el certificado final lleva **la IP del nodo** (lo que hace el driver) **o el nombre de alguno de
//!    los hosts de la URL**.
//!
//! Lo segundo se hace con un `set_verify_callback` que perdona **un solo error**,
//! `X509_V_ERR_IP_ADDRESS_MISMATCH`, **en el certificado final** (profundidad 0), y solo si el certificado
//! lleva el nombre de un host de la URL ([`casa_nombre`]). Nada más.
//!
//! # Lo que no hay
//!
//! Ni certificado de cliente (mTLS), ni una CA por fichero (el driver corre en un Job sin tus
//! ficheros), ni SNI de un proxy (Astra, con su *bundle*), ni forma de apagar la verificación. Y el
//! mínimo es **TLS 1.2**, explícito: no depende de la política de OpenSSL del sistema.

use openssl::ssl::{SslContext, SslContextBuilder, SslMethod, SslVerifyMode, SslVersion};
use openssl::x509::{X509, X509StoreContextRef};
use ore_driver::{Codigo, Fallo};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

/// CA en las que, **además de las del sistema**, confía este proceso. Vacío en el producto: no hay forma
/// de pasar una CA (ni por URL ni por fichero). Existe para que las pruebas confíen en la suya sin tocar el
/// entorno del proceso (que OpenSSL, en Windows, no ve si cambia en caliente).
static CONFIANZA_EXTRA: Mutex<Vec<X509>> = Mutex::new(Vec::new());

/// **Solo para pruebas.** Añade una CA a las de confianza de este proceso.
#[doc(hidden)]
pub fn confiar_ademas_en(ca: X509) {
    if let Ok(mut v) = CONFIANZA_EXTRA.lock() {
        v.push(ca);
    }
}

/// `X509_V_ERR_IP_ADDRESS_MISMATCH`: el certificado final no lleva la IP con la que se conectó.
const IP_NO_CASA: i32 = 64;

/// Por qué falló la última verificación: la comparten el contexto y el *callback* que la escribe.
type Razon = Arc<Mutex<Option<String>>>;

/// El contexto TLS y **por qué** falló la última verificación (para contarlo).
pub struct Contexto {
    pub ctx: SslContext,
    razon: Razon,
}

impl Contexto {
    /// La razón por la que OpenSSL rechazó el certificado, si lo rechazó.
    pub fn razon(&self) -> Option<String> {
        self.razon.lock().ok().and_then(|r| r.clone())
    }
}

/// El contexto TLS para estos hosts (los de la URL).
pub fn contexto(hosts: &[String]) -> Result<Contexto, Fallo> {
    let (b, razon) = constructor(hosts)?;
    Ok(Contexto {
        ctx: b.build(),
        razon,
    })
}

/// El contexto **sin construir** y su razón: lo que [`contexto`] monta, a la vista de las pruebas.
fn constructor(hosts: &[String]) -> Result<(SslContextBuilder, Razon), Fallo> {
    let torcido = |e: openssl::error::ErrorStack| Fallo::origen(format!("TLS: {e}"));
    let mut b = SslContextBuilder::new(SslMethod::tls()).map_err(torcido)?;
    b.set_default_verify_paths().map_err(torcido)?;
    if let Ok(extra) = CONFIANZA_EXTRA.lock() {
        for ca in extra.iter() {
            b.cert_store_mut().add_cert(ca.clone()).map_err(torcido)?;
        }
    }
    b.set_min_proto_version(Some(SslVersion::TLS1_2))
        .map_err(torcido)?;

    // Los nombres de la URL: una IP no es un nombre, y de eso ya se ocupa el driver.
    let nombres: Arc<Vec<String>> = Arc::new(
        hosts
            .iter()
            .map(|h| h.trim_matches(['[', ']']).to_string())
            .filter(|h| h.parse::<IpAddr>().is_err())
            .collect(),
    );
    let razon: Razon = Arc::new(Mutex::new(None));
    let guardar = razon.clone();
    b.set_verify_callback(
        SslVerifyMode::PEER,
        move |previo: bool, almacen: &mut X509StoreContextRef| {
            if previo {
                return true;
            }
            let error = almacen.error();
            // Lo único que se perdona: la IP que no está, en el certificado final, si lleva el NOMBRE de la URL.
            if error.as_raw() == IP_NO_CASA
                && almacen.error_depth() == 0
                && let Some(cert) = almacen.current_cert()
                && cert_lleva_nombre(cert, &nombres)
            {
                return true;
            }
            // Se recuerda la PRIMERA razón: es la causa, lo demás son consecuencias.
            if let Ok(mut r) = guardar.lock()
                && r.is_none()
            {
                *r = Some(format!("{error}, a profundidad {}", almacen.error_depth()));
            }
            false
        },
    );
    Ok((b, razon))
}

/// Si el certificado lleva, como nombre DNS, el de alguno de los hosts.
fn cert_lleva_nombre(cert: &openssl::x509::X509Ref, hosts: &[String]) -> bool {
    let Some(alt) = cert.subject_alt_names() else {
        return false; // sin SAN no hay nombres: el CN no vale (RFC 6125, y lo exige el navegador desde 2017)
    };
    alt.iter()
        .filter_map(|n| n.dnsname())
        .any(|patron| hosts.iter().any(|h| casa_nombre(patron, h)))
}

/// Si un nombre del certificado (`patron`, que puede ser un comodín) casa con `host`.
///
/// Las reglas son las de RFC 6125 y las justas:
///
/// - sin distinguir mayúsculas, y con o sin el punto final;
/// - un comodín es **solo la primera etiqueta entera** (`*.mi-empresa.com`), **casa con una sola
///   etiqueta** (`a.mi-empresa.com`, no `a.b.mi-empresa.com` ni `mi-empresa.com`) y necesita **dos
///   etiquetas más** detrás (`*.com` no vale);
/// - un comodín parcial (`f*.mi-empresa.com`) no casa con nada;
/// - un nombre con un carácter de control, o vacío, no casa.
pub fn casa_nombre(patron: &str, host: &str) -> bool {
    let norm = |s: &str| s.strip_suffix('.').unwrap_or(s).to_ascii_lowercase();
    let (patron, host) = (norm(patron), norm(host));
    if patron.is_empty()
        || host.is_empty()
        || patron.chars().chain(host.chars()).any(char::is_control)
    {
        return false;
    }
    match patron.strip_prefix("*.") {
        None => !patron.contains('*') && patron == host,
        Some(resto) => {
            if resto.contains('*')
                || resto.split('.').count() < 2
                || resto.split('.').any(str::is_empty)
            {
                return false;
            }
            match host.split_once('.') {
                Some((etiqueta, cola)) => !etiqueta.is_empty() && cola == resto,
                None => false,
            }
        }
    }
}

/// El error del driver, dicho para quien lo lee: por qué no se aceptó el certificado, y que **no se
/// envió la credencial** (el *handshake* muere antes de la autenticación).
///
/// `None` si el error no es de TLS.
pub fn explicar(mensaje: &str, razon: Option<&str>) -> Option<Fallo> {
    let porque = |m: String| Some(Fallo::new(Codigo::Conexion, m).reintentable(false));
    if mensaje.contains("certificate verify failed") {
        let razon = razon.unwrap_or("OpenSSL no dio la razón");
        return porque(format!(
            "el certificado del servidor no se pudo verificar ({razon}): no se continuó y no se envió la credencial. \
             Tiene que ser de una CA de confianza (la del sistema, o la de SSL_CERT_FILE) y llevar la IP del nodo \
             o el nombre de la dirección"
        ));
    }
    if mensaje.contains("wrong version number")
        || mensaje.contains("packet length too long")
        || mensaje.contains("unknown protocol")
    {
        return porque(
            "el servidor no habla TLS en ese puerto (¿`UseSSL` debería ser `FALSE`?): no se envió la credencial".into(),
        );
    }
    if mensaje.contains("unsupported protocol") || mensaje.contains("protocol version") {
        return porque(
            "el servidor solo ofrece una versión de TLS anterior a 1.2, que no se admite: no se envió la credencial".into(),
        );
    }
    None
}

/// Una pista para el cliente que NO pidió TLS y recibe una conexión cortada nada más abrirla: es lo que hace un
/// Cassandra que exige TLS (`optional: false`) cuando le llega un cliente sin él.
pub fn pista_sin_tls(mensaje: &str) -> Option<&'static str> {
    (mensaje.contains("Request: OPTIONS") && mensaje.contains("Connection broken"))
        .then_some(" ¿El servidor exige TLS? Prueba con `tls=true` (UseSSL = TRUE).")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_servidor_que_corta_a_un_cliente_sin_tls_dice_que_quiza_exige_tls() {
        let crudo = "Control connection pool error: Failed to perform a connection setup request. \
                     Request: OPTIONS, reason: Connection broken, reason: Failed to deserialize frame";
        assert!(pista_sin_tls(crudo).unwrap().contains("tls=true"));
        // Una contraseña mala o un servidor caído no son eso.
        assert!(pista_sin_tls("Authentication failed - bad credentials").is_none());
        assert!(pista_sin_tls("Connection refused").is_none());
    }

    #[test]
    fn el_minimo_es_tls_1_2_y_no_lo_que_diga_el_sistema() {
        // En una máquina cuya política de OpenSSL ya rechaza TLS 1.1 esto no se nota en un servidor de
        // prueba, y en otra, con `MinProtocol = TLSv1`, es lo único que lo impide: se mira la propiedad.
        let (mut b, _) = constructor(&["cassandra.mi-empresa.com".to_string()]).unwrap();
        assert_eq!(b.min_proto_version(), Some(SslVersion::TLS1_2));
    }

    #[test]
    fn un_nombre_exacto_casa_sin_mirar_mayusculas_ni_el_punto_final() {
        assert!(casa_nombre(
            "cassandra.mi-empresa.com",
            "cassandra.mi-empresa.com"
        ));
        assert!(casa_nombre(
            "Cassandra.Mi-Empresa.COM",
            "cassandra.mi-empresa.com"
        ));
        assert!(casa_nombre(
            "cassandra.mi-empresa.com.",
            "cassandra.mi-empresa.com"
        ));
        assert!(casa_nombre(
            "cassandra.mi-empresa.com",
            "CASSANDRA.mi-empresa.com."
        ));
        assert!(!casa_nombre(
            "cassandra.mi-empresa.com",
            "cassandra2.mi-empresa.com"
        ));
        assert!(!casa_nombre("mi-empresa.com", "cassandra.mi-empresa.com"));
        assert!(!casa_nombre("cassandra.mi-empresa.com", "mi-empresa.com"));
    }

    #[test]
    fn un_comodin_es_la_primera_etiqueta_entera_y_una_sola() {
        assert!(casa_nombre("*.mi-empresa.com", "cassandra.mi-empresa.com"));
        assert!(casa_nombre("*.MI-empresa.com", "Cassandra.mi-empresa.com"));
        assert!(
            !casa_nombre("*.mi-empresa.com", "a.b.mi-empresa.com"),
            "una sola etiqueta"
        );
        assert!(
            !casa_nombre("*.mi-empresa.com", "mi-empresa.com"),
            "no casa con la raíz"
        );
        assert!(
            !casa_nombre("*.mi-empresa.com", ".mi-empresa.com"),
            "la etiqueta no puede estar vacía"
        );
        assert!(!casa_nombre("*.mi-empresa.com", "cassandra.otra.com"));
    }

    #[test]
    fn lo_que_no_es_un_comodin_de_verdad_no_casa() {
        // Un comodín parcial, en medio, doble, o sin dos etiquetas detrás: nada.
        for malo in [
            "f*.mi-empresa.com",
            "*a.mi-empresa.com",
            "cassandra.*.com",
            "**.mi-empresa.com",
            "*.*.com",
            "*.com",
            "*",
            "*.",
        ] {
            assert!(!casa_nombre(malo, "foo.mi-empresa.com"), "`{malo}`");
            assert!(!casa_nombre(malo, "foo.com"), "`{malo}`");
            assert!(!casa_nombre(malo, "foo"), "`{malo}`");
        }
        // Y un asterisco literal en el host no es un comodín.
        assert!(!casa_nombre("a*.mi-empresa.com", "a*.mi-empresa.com"));
    }

    #[test]
    fn un_nombre_con_caracteres_de_control_o_vacio_no_casa() {
        // El ataque clásico: un certificado para `mi-empresa.com\0.atacante.com`.
        assert!(!casa_nombre(
            "mi-empresa.com\0.atacante.com",
            "mi-empresa.com"
        ));
        assert!(!casa_nombre("mi-empresa.com\0", "mi-empresa.com"));
        assert!(!casa_nombre("mi-empresa.com", "mi-empresa.com\0"));
        assert!(!casa_nombre("", ""));
        assert!(!casa_nombre("a", ""));
        assert!(!casa_nombre("", "a"));
        assert!(!casa_nombre(".", "."));
    }

    #[test]
    fn los_errores_de_tls_se_explican_y_no_se_reintentan() {
        let f = explicar(
            "error:0A000086:SSL routines:tls_post_process_server_certificate:certificate verify failed:ssl\\\\statem:2092:",
            Some("unable to get local issuer certificate, a profundidad 1"),
        )
        .expect("es de TLS");
        assert_eq!(f.codigo, Codigo::Conexion);
        assert!(
            !f.reintentable,
            "un certificado malo no mejora a los tres segundos"
        );
        assert!(
            f.mensaje.contains("unable to get local issuer certificate"),
            "{}",
            f.mensaje
        );
        assert!(
            f.mensaje.contains("no se envió la credencial"),
            "{}",
            f.mensaje
        );
        assert!(f.mensaje.contains("SSL_CERT_FILE"), "dice dónde se arregla");

        let f = explicar(
            "error:0A00010B:SSL routines:ssl3_get_record:wrong version number",
            None,
        )
        .unwrap();
        assert!(
            f.mensaje.contains("no habla TLS") && f.mensaje.contains("UseSSL"),
            "{}",
            f.mensaje
        );
        let f = explicar("error:0A000102:SSL routines:unsupported protocol", None).unwrap();
        assert!(f.mensaje.contains("anterior a 1.2"), "{}", f.mensaje);
        // Lo que no es de TLS no se toca.
        assert!(explicar("Authentication failed - bad credentials", None).is_none());
        assert!(explicar("Connection refused", None).is_none());
    }
}
