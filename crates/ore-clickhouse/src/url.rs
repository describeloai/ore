//! **La URL de conexión**: `clickhouse://usuario[:clave]@host[:8123][/BASE]?tls=true&timeoutMs=…`.
//!
//! Tres oficios, los tres puros: [`analizar`] (lo que la URL dice, o por qué no se admite), [`validar`] y [`publica`]
//! (la URL sin usuario ni contraseña, para enseñarla).
//!
//! Opciones:
//!
//! - `tls=true|false`: `https://`. El puerto por defecto es 8123, o 8443 con `tls=true` (el de ClickHouse Cloud).
//!   **Una contraseña por HTTP en claro no se admite**: HTTP *Basic* iría legible por la red. Un usuario **sin**
//!   contraseña sí (el laboratorio, o un usuario de sólo lectura que el servidor limita por red).
//! - `timeoutMs=`: el tiempo límite de cada consulta (`max_execution_time`).
//!
//! Lo que apaga la verificación de TLS no se ofrece, y una CA propia por fichero tampoco: el conector corre en un Job
//! sin tus ficheros (la misma regla que Phoenix).

use ore_driver::url::{booleano, decodificar, host_y_puerto};

pub const PUERTO: u16 = 8123;
pub const PUERTO_TLS: u16 = 8443;

#[derive(Clone, PartialEq, Eq)]
pub struct Destino {
    pub usuario: Option<String>,
    pub clave: Option<String>,
    pub host: String,
    pub puerto: u16,
    /// La base de datos a la que se limita el catálogo y desde la que se resuelven los nombres.
    pub base: Option<String>,
    pub tls: bool,
    pub timeout_ms: Option<u64>,
}

// La contraseña no sale nunca por `{:?}`.
impl std::fmt::Debug for Destino {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Destino")
            .field("usuario", &self.usuario)
            .field("clave", &self.clave.as_ref().map(|_| "***"))
            .field("host", &self.host)
            .field("puerto", &self.puerto)
            .field("base", &self.base)
            .field("tls", &self.tls)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl Destino {
    /// `http://host:8123/`, sin la credencial.
    pub fn direccion(&self) -> String {
        let esquema = if self.tls { "https" } else { "http" };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{esquema}://{host}:{}/", self.puerto)
    }
}

pub fn analizar(url: &str) -> Result<Destino, String> {
    let resto = url
        .strip_prefix("clickhouse://")
        .ok_or("la URL de ClickHouse es `clickhouse://usuario[:clave]@host[:8123][/BASE]`")?;
    if resto.contains('#') {
        return Err("la URL de ClickHouse no lleva `#`".into());
    }
    let (antes, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let (autoridad, ruta) = match antes.find('/') {
        Some(i) => antes.split_at(i),
        None => (antes, ""),
    };
    let (credencial, hosts) = match autoridad.rsplit_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, autoridad),
    };
    let (usuario, clave) = match credencial {
        Some(c) => match c.split_once(':') {
            Some((u, p)) => (
                Some(decodificar(u)?),
                Some(decodificar(p)?).filter(|p| !p.is_empty()),
            ),
            None => (Some(decodificar(c)?), None),
        },
        None => (None, None),
    };
    if usuario.as_deref() == Some("") {
        return Err("la credencial es `usuario[:clave]`: falta el usuario".into());
    }
    if hosts.contains(',') {
        return Err("la URL de ClickHouse nombra un solo servidor (no una lista de hosts)".into());
    }
    if hosts.is_empty() {
        return Err("la URL de ClickHouse no dice a qué servidor conectar".into());
    }
    let base = ruta
        .strip_prefix('/')
        .filter(|k| !k.is_empty())
        .map(decodificar)
        .transpose()?;

    let mut tls = false;
    let mut timeout_ms = None;
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (clave_opcion, valor) = par.split_once('=').unwrap_or((par, ""));
        match clave_opcion.to_ascii_lowercase().as_str() {
            "tls" | "ssl" | "secure" => tls = booleano(clave_opcion, valor)?,
            "timeoutms" => {
                timeout_ms = Some(valor.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(
                    || format!("`{clave_opcion}={valor}`: son milisegundos, un entero mayor que 0"),
                )?)
            }
            "tlscafile" | "cafile" | "sslrootcert" => {
                return Err(format!(
                    "`{clave_opcion}`: una CA propia va por fichero y el conector corre en un Job sin tus ficheros; todavía no se admite"
                ));
            }
            // Lo que apaga la verificación de TLS no se ofrece. Dicho en falso no apaga nada.
            "tlsinsecure" | "tlsallowinvalidcertificates" | "tlsallowinvalidhostnames" => {
                if !matches!(valor, "false" | "0") {
                    return Err(format!(
                        "`{clave_opcion}` apaga la verificación de TLS y no se admite: el servidor tiene que presentar un certificado que se verifique"
                    ));
                }
            }
            "verify" | "sslverify" => {
                if matches!(valor, "false" | "0") {
                    return Err(format!(
                        "`{clave_opcion}={valor}` apaga la verificación de TLS y no se admite"
                    ));
                }
            }
            otra => return Err(format!("`{otra}` no es una opción de la URL de ClickHouse")),
        }
    }
    let (host, puerto) = host_y_puerto(hosts, if tls { PUERTO_TLS } else { PUERTO })?;
    // HTTP *Basic* va en una cabecera legible: sin TLS, la contraseña cruzaría la red tal cual.
    if clave.is_some() && !tls {
        return Err(
            "la contraseña iría por HTTP en claro: con contraseña, ClickHouse exige `tls=true`"
                .into(),
        );
    }
    Ok(Destino {
        usuario,
        clave,
        host,
        puerto,
        base,
        tls,
        timeout_ms,
    })
}

/// `Ok` si la URL se puede usar; si no, por qué.
pub fn validar(url: &str) -> Result<(), String> {
    analizar(url).map(|_| ())
}

/// La URL sin `usuario:contraseña@`, para enseñarla (ver [`ore_driver::url::publica`]).
pub fn publica(url: &str) -> String {
    ore_driver::url::publica(url, "ClickHouse")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn una_url_minima_es_el_laboratorio() {
        let d = analizar("clickhouse://127.0.0.1").unwrap();
        assert_eq!(
            (d.host.as_str(), d.puerto, d.tls),
            ("127.0.0.1", 8123, false)
        );
        assert_eq!(d.direccion(), "http://127.0.0.1:8123/");
        assert!(d.usuario.is_none() && d.clave.is_none() && d.base.is_none());
    }

    #[test]
    fn un_usuario_sin_clave_se_admite_sin_tls() {
        let d = analizar("clickhouse://kit@localhost:8123/kit").unwrap();
        assert_eq!(d.usuario.as_deref(), Some("kit"));
        assert!(d.clave.is_none());
        assert_eq!(d.base.as_deref(), Some("kit"));
        // `usuario:` con la clave vacía es lo mismo que sin clave.
        assert!(
            analizar("clickhouse://kit:@localhost")
                .unwrap()
                .clave
                .is_none()
        );
    }

    #[test]
    fn con_tls_el_puerto_es_8443_y_la_clave_se_admite() {
        let d = analizar(
            "clickhouse://ana:p%40ss%3Aw@x.clickhouse.cloud/VENTAS?tls=true&timeoutMs=5000",
        )
        .unwrap();
        assert_eq!(
            (d.host.as_str(), d.puerto, d.tls),
            ("x.clickhouse.cloud", 8443, true)
        );
        assert_eq!(d.clave.as_deref(), Some("p@ss:w"));
        assert_eq!(d.timeout_ms, Some(5000));
        assert_eq!(d.direccion(), "https://x.clickhouse.cloud:8443/");
        // Un puerto explícito manda.
        assert_eq!(
            analizar("clickhouse://u@h:9443?tls=true").unwrap().puerto,
            9443
        );
    }

    #[test]
    fn la_contrasena_por_http_en_claro_no_se_admite() {
        let e = analizar("clickhouse://ana:secreta@localhost").unwrap_err();
        assert!(e.contains("HTTP en claro") && !e.contains("secreta"), "{e}");
    }

    #[test]
    fn lo_que_apaga_tls_no_se_ofrece() {
        for u in [
            "clickhouse://u@h?tls=true&tlsInsecure=true",
            "clickhouse://u@h?tls=true&verify=false",
            "clickhouse://u@h?tls=true&tlsCAFile=/x.pem",
        ] {
            assert!(analizar(u).is_err(), "{u}");
        }
        assert!(analizar("clickhouse://u@h?tls=true&tlsInsecure=false").is_ok());
    }

    #[test]
    fn lo_que_no_es_clickhouse_se_dice() {
        assert!(
            analizar("postgres://u@h")
                .unwrap_err()
                .contains("clickhouse://")
        );
        assert!(analizar("clickhouse://").is_err());
        assert!(analizar("clickhouse://a,b").is_err());
        assert!(
            analizar("clickhouse://u@h?nada=1")
                .unwrap_err()
                .contains("nada")
        );
        assert!(analizar("clickhouse://u@h?timeoutMs=0").is_err());
    }

    #[test]
    fn el_debug_no_enseña_la_clave() {
        let d = analizar("clickhouse://ana:secreta@h?tls=true").unwrap();
        assert!(!format!("{d:?}").contains("secreta"));
    }
}
