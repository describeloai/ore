//! **La URL de conexión**: `phoenix://usuario:clave@host:8765/ESQUEMA?tls=true&tenant=…`, con su
//! credencial dentro (va al cofre como `fuente-<n>`, como la de Cassandra).
//!
//! Tres oficios, los tres puros: [`analizar`] (lo que la URL dice, o por qué no se admite), [`validar`] y
//! [`publica`] (la URL sin usuario ni contraseña, para enseñarla).
//!
//! Opciones (ficha 0071 §4):
//!
//! - `tls=true|false`: `https://` al PQS. **Una contraseña por HTTP en claro no se admite** (HTTP *Basic*
//!   iría legible por la red).
//! - `tenant=`: el inquilino de una tabla multi-tenant (`TenantId` de `OpenConnection`, §3.5).
//! - `timeoutMs=`: el tiempo límite **del servidor** para cada consulta (`phoenix.query.timeoutMs`, §3.8).
//! - `paralelo=`: conexiones que leen a la vez una tabla grande, partida por rangos de su clave (por defecto 4;
//!   `paralelo=1` la lee con una sola).
//! - `rezago=`: milisegundos que el `testigo` deja sin mirar hacia atrás desde la hora del servidor (por defecto
//!   5 000; `0` lo apaga). Las filas que se están escribiendo con la marca de ahora todavía no son de nadie
//!   (§3.16).
//! - `frame=`: filas por frame (por defecto 20 000; §3.7 mide que ≥ 10 000 es lo que rinde).
//!
//! Lo que apaga la verificación de TLS no se ofrece, y una CA propia por fichero tampoco: el conector
//! corre en un Job sin tus ficheros.

use ore_driver::url::{booleano, decodificar, host_y_puerto};

pub const PUERTO: u16 = 8765;
pub const FILAS_POR_FRAME: i32 = 20_000;
const MAX_FILAS_POR_FRAME: i32 = 500_000;
/// Conexiones que leen a la vez una tabla grande, por defecto. Medido contra el laboratorio (10⁶ filas): 1 → 22,9 s,
/// 2 → 9,5 s, 4 → 6,2 s, 8 → 7,4 s: con un solo nodo y un PQS de 1 GB, 4 es el punto.
pub const PARALELO: usize = 4;
const MAX_PARALELO: usize = 16;
/// El rezago del `testigo` por defecto: una escritura de HBase confirma en milisegundos, pero un cliente con el
/// reloj adelantado o una transacción lenta pone marcas de hace unos segundos. Más que eso se pide a mano.
pub const REZAGO_MS: u64 = 5_000;
const MAX_REZAGO_MS: u64 = 3_600_000;

#[derive(Clone, PartialEq, Eq)]
pub struct Destino {
    pub usuario: Option<String>,
    pub clave: Option<String>,
    pub host: String,
    pub puerto: u16,
    /// El esquema de Phoenix al que se limita el catálogo (M3). Vacío = todos.
    pub esquema: Option<String>,
    pub tls: bool,
    pub tenant: Option<String>,
    pub timeout_ms: Option<u64>,
    pub filas_por_frame: i32,
    /// Cuántas conexiones leen a la vez una tabla grande (M5, ficha §3.13). 1 = una sola.
    pub paralelo: usize,
    /// Cuánto se queda atrás el `testigo` de la hora del servidor (M7, ficha §3.16).
    pub rezago_ms: u64,
}

impl std::fmt::Debug for Destino {
    /// La contraseña no sale nunca, ni en un `{:?}` perdido.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Destino")
            .field("usuario", &self.usuario)
            .field("clave", &self.clave.as_ref().map(|_| "***"))
            .field("host", &self.host)
            .field("puerto", &self.puerto)
            .field("esquema", &self.esquema)
            .field("tls", &self.tls)
            .field("tenant", &self.tenant)
            .field("timeout_ms", &self.timeout_ms)
            .field("filas_por_frame", &self.filas_por_frame)
            .field("paralelo", &self.paralelo)
            .field("rezago_ms", &self.rezago_ms)
            .finish()
    }
}

impl Destino {
    /// La dirección HTTP del PQS, sin credencial.
    pub fn direccion(&self) -> String {
        let esquema = if self.tls { "https" } else { "http" };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{esquema}://{host}:{}/", self.puerto)
    }

    /// Las opciones del cliente Avatica. `timeout_ms` manda sobre el de la URL (el de la petición).
    pub fn opciones(&self, timeout_ms: Option<u64>) -> ore_avatica::Opciones {
        let mut o = ore_avatica::Opciones::nuevas(self.direccion());
        if let (Some(u), Some(c)) = (&self.usuario, &self.clave) {
            o.credencial = Some((u.clone(), c.clone()));
        }
        if let Some(t) = &self.tenant {
            o.propiedades.insert("TenantId".into(), t.clone());
        }
        if let Some(ms) = timeout_ms.or(self.timeout_ms) {
            o.propiedades
                .insert("phoenix.query.timeoutMs".into(), ms.to_string());
            // El cliente espera algo más que el servidor: que sea el servidor quien diga «tiempo».
            o.tiempo_peticion = std::time::Duration::from_millis(ms.saturating_add(30_000));
        }
        o.filas_por_frame = self.filas_por_frame;
        o
    }
}

/// Lo que la URL dice, o por qué no se admite (sin repetir la credencial).
pub fn analizar(url: &str) -> Result<Destino, String> {
    let resto = url
        .strip_prefix("phoenix://")
        .ok_or("la URL de Phoenix es `phoenix://usuario:clave@host[:8765][/ESQUEMA]`")?;
    if resto.contains('#') {
        return Err("la URL de Phoenix no lleva `#`".into());
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
            Some((u, p)) => (Some(decodificar(u)?), Some(decodificar(p)?)),
            None => (Some(decodificar(c)?), None),
        },
        None => (None, None),
    };
    if usuario.is_some() != clave.is_some() {
        return Err("la credencial es `usuario:clave`: falta una de las dos partes".into());
    }
    if hosts.contains(',') {
        return Err(
            "la URL de Phoenix nombra un solo Phoenix Query Server (no una lista de hosts)".into(),
        );
    }
    if hosts.is_empty() {
        return Err("la URL de Phoenix no dice a qué servidor conectar".into());
    }
    let (host, puerto) = host_y_puerto(hosts, PUERTO)?;
    let esquema = ruta
        .strip_prefix('/')
        .filter(|k| !k.is_empty())
        .map(decodificar)
        .transpose()?;

    let mut d = Destino {
        usuario,
        clave,
        host,
        puerto,
        esquema,
        tls: false,
        tenant: None,
        timeout_ms: None,
        filas_por_frame: FILAS_POR_FRAME,
        paralelo: PARALELO,
        rezago_ms: REZAGO_MS,
    };
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (clave, valor) = par.split_once('=').unwrap_or((par, ""));
        match clave.to_ascii_lowercase().as_str() {
            "tls" | "ssl" => d.tls = booleano(clave, valor)?,
            "tenant" | "tenantid" => d.tenant = Some(decodificar(valor)?).filter(|t| !t.is_empty()),
            "timeoutms" => {
                d.timeout_ms = Some(valor.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(
                    || format!("`{clave}={valor}`: son milisegundos, un entero mayor que 0"),
                )?)
            }
            "frame" | "filas" => {
                d.filas_por_frame = valor
                    .parse::<i32>()
                    .ok()
                    .filter(|n| (1..=MAX_FILAS_POR_FRAME).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "`{clave}={valor}`: son filas por frame, de 1 a {MAX_FILAS_POR_FRAME}"
                        )
                    })?
            }
            "paralelo" | "parallel" => {
                d.paralelo = valor
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=MAX_PARALELO).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "`{clave}={valor}`: son conexiones en paralelo, de 1 (una sola) a {MAX_PARALELO}"
                        )
                    })?
            }
            "rezago" | "lag" => {
                d.rezago_ms = valor
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n <= MAX_REZAGO_MS)
                    .ok_or_else(|| {
                        format!(
                            "`{clave}={valor}`: son milisegundos de rezago, de 0 a {MAX_REZAGO_MS}"
                        )
                    })?
            }
            "tlscafile" | "cafile" | "sslrootcert" => {
                return Err(format!(
                    "`{clave}`: una CA propia va por fichero y el conector corre en un Job sin tus ficheros; todavía no se admite"
                ));
            }
            // Lo que apaga la verificación de TLS no se ofrece: se apaga el día que alguien tiene
            // prisa. Dicho en falso no apaga nada.
            "tlsinsecure" | "tlsallowinvalidcertificates" | "tlsallowinvalidhostnames" => {
                if !matches!(valor, "false" | "0") {
                    return Err(format!(
                        "`{clave}` apaga la verificación de TLS y no se admite: el servidor tiene que presentar un certificado que se verifique"
                    ));
                }
            }
            "verify" | "sslverify" => {
                if matches!(valor, "false" | "0") {
                    return Err(format!(
                        "`{clave}={valor}` apaga la verificación de TLS y no se admite"
                    ));
                }
            }
            otra => return Err(format!("`{otra}` no es una opción de la URL de Phoenix")),
        }
    }
    // HTTP *Basic* va en una cabecera legible: sin TLS, la contraseña cruzaría la red tal cual.
    if d.clave.is_some() && !d.tls {
        return Err(
            "la contraseña iría por HTTP en claro: la autenticación de Phoenix Query Server exige `tls=true`".into(),
        );
    }
    Ok(d)
}

/// `Ok` si la URL se puede usar; si no, por qué.
pub fn validar(url: &str) -> Result<(), String> {
    analizar(url).map(|_| ())
}

/// La URL sin `usuario:contraseña@`, para enseñarla (ver [`ore_driver::url::publica`]).
pub fn publica(url: &str) -> String {
    ore_driver::url::publica(url, "Phoenix")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn una_url_minima_es_el_laboratorio() {
        let d = analizar("phoenix://127.0.0.1").unwrap();
        assert_eq!(
            (d.host.as_str(), d.puerto, d.tls),
            ("127.0.0.1", 8765, false)
        );
        assert_eq!(d.direccion(), "http://127.0.0.1:8765/");
        assert_eq!(d.filas_por_frame, 20_000);
        assert!(d.usuario.is_none() && d.esquema.is_none() && d.tenant.is_none());
    }

    #[test]
    fn la_url_entera() {
        let d = analizar("phoenix://ana:p%40ss%3Aw@pqs.ejemplo.com:9000/VENTAS?tls=true&tenant=t1&timeoutMs=5000&frame=1000").unwrap();
        assert_eq!(d.usuario.as_deref(), Some("ana"));
        assert_eq!(d.clave.as_deref(), Some("p@ss:w"));
        assert_eq!((d.host.as_str(), d.puerto), ("pqs.ejemplo.com", 9000));
        assert_eq!(d.esquema.as_deref(), Some("VENTAS"));
        assert!(d.tls);
        assert_eq!(d.tenant.as_deref(), Some("t1"));
        assert_eq!((d.timeout_ms, d.filas_por_frame), (Some(5000), 1000));
        assert_eq!(d.direccion(), "https://pqs.ejemplo.com:9000/");
    }

    #[test]
    fn las_opciones_llevan_el_inquilino_y_el_tiempo_limite() {
        let d = analizar("phoenix://h?tenant=t1&timeoutMs=5000").unwrap();
        let o = d.opciones(None);
        assert_eq!(
            o.propiedades.get("TenantId").map(String::as_str),
            Some("t1")
        );
        assert_eq!(
            o.propiedades
                .get("phoenix.query.timeoutMs")
                .map(String::as_str),
            Some("5000")
        );
        // El de la petición manda sobre el de la URL.
        let o = d.opciones(Some(300));
        assert_eq!(
            o.propiedades
                .get("phoenix.query.timeoutMs")
                .map(String::as_str),
            Some("300")
        );
        // Y sin ninguno no se pone: el del servidor es el suyo.
        let o = analizar("phoenix://h").unwrap().opciones(None);
        assert!(!o.propiedades.contains_key("phoenix.query.timeoutMs"));
    }

    #[test]
    fn una_contrasena_por_http_en_claro_no_se_admite() {
        let e = analizar("phoenix://u:secreta-9@h").unwrap_err();
        assert!(e.contains("tls=true") && !e.contains("secreta-9"), "{e}");
        assert!(analizar("phoenix://u:secreta-9@h?tls=true").is_ok());
        // Sin contraseña no hay nada que proteger (el laboratorio).
        assert!(analizar("phoenix://h").is_ok());
        // Usuario o contraseña a medias.
        assert!(analizar("phoenix://u@h?tls=true").is_err());
    }

    #[test]
    fn lo_que_apaga_el_tls_no_se_ofrece() {
        for malo in [
            "tlsInsecure=true",
            "verify=false",
            "sslverify=0",
            "tlsAllowInvalidCertificates=true",
            "tlsCAFile=/x.pem",
        ] {
            let e = analizar(&format!("phoenix://u:clave-1234@h?tls=true&{malo}")).unwrap_err();
            assert!(!e.contains("clave-1234"), "{e}");
        }
        // Dicho en falso no apaga nada.
        assert!(analizar("phoenix://h?tls=true&tlsInsecure=false&verify=true").is_ok());
    }

    #[test]
    fn lo_mal_formado_se_dice() {
        for malo in [
            "cassandra://h",
            "phoenix://",
            "phoenix://h1,h2",
            "phoenix://h:0",
            "phoenix://h:abc",
            "phoenix://h?tls=quizas",
            "phoenix://h?timeoutMs=0",
            "phoenix://h?frame=0",
            "phoenix://h?frame=9999999",
            "phoenix://h?tiempo=1",
            "phoenix://h#x",
            "phoenix://h/%zz",
        ] {
            assert!(analizar(malo).is_err(), "`{malo}` debería fallar");
        }
    }

    #[test]
    fn ipv6_entre_corchetes() {
        let d = analizar("phoenix://[::1]:9000").unwrap();
        assert_eq!((d.host.as_str(), d.puerto), ("::1", 9000));
        assert_eq!(d.direccion(), "http://[::1]:9000/");
    }

    /// Lo que la hoja de la consola (`rubix-platform`, `PhoenixConnection.tsx`) compone y este parser tiene que
    /// entender. Si una de las dos cambia, esta prueba es la que lo dice: el contrato entre repositorios.
    #[test]
    fn las_direcciones_que_compone_la_hoja_de_la_consola_se_entienden() {
        // Contraseña con `@`, `:` y `/` codificados, esquema y TLS.
        let d =
            analizar("phoenix://ore_lector:p%40ss%3Aw%2Frd@pqs.empresa.com:8765/VENTAS?tls=true")
                .unwrap();
        assert_eq!(d.usuario.as_deref(), Some("ore_lector"));
        assert_eq!(d.clave.as_deref(), Some("p@ss:w/rd"));
        assert_eq!((d.host.as_str(), d.puerto), ("pqs.empresa.com", 8765));
        assert_eq!(d.esquema.as_deref(), Some("VENTAS"));
        assert!(d.tls);
        // Sin esquema, la ruta es `/` y la consulta sigue.
        let d = analizar("phoenix://u:c@pqs.empresa.com:8765/?tls=true").unwrap();
        assert_eq!(d.esquema, None);
        assert!(d.tls);
        // Sin puerto: el de siempre.
        let d = analizar("phoenix://u:c@pqs.empresa.com/VENTAS?tls=true").unwrap();
        assert_eq!(d.puerto, PUERTO);
        // Con las propiedades avanzadas; el inquilino lleva un espacio codificado.
        let d = analizar(
            "phoenix://u:c@pqs.empresa.com:8765/VENTAS?tls=true&tenant=acme%20corp&timeoutMs=30000&paralelo=8&rezago=0",
        )
        .unwrap();
        assert_eq!(d.tenant.as_deref(), Some("acme corp"));
        assert_eq!(d.timeout_ms, Some(30_000));
        assert_eq!((d.paralelo, d.rezago_ms), (8, 0));
        // Anónimo y sin TLS: lo único que se admite sin TLS.
        let d = analizar("phoenix://pqs.empresa.com:8765/VENTAS").unwrap();
        assert_eq!((d.usuario, d.clave, d.tls), (None, None, false));
        // Y lo que la hoja no compone porque este parser lo rechaza: una contraseña por HTTP en claro.
        assert!(analizar("phoenix://u:c@pqs.empresa.com:8765/VENTAS").is_err());
    }

    #[test]
    fn la_publica_no_lleva_credencial_y_el_debug_tampoco() {
        let url = "phoenix://ana:secreta-1234@h:8765/V?tls=true&password=otra-5678";
        let p = publica(url);
        assert_eq!(p, "phoenix://h:8765/V?tls=true&password=***");
        let d = analizar("phoenix://ana:secreta-1234@h?tls=true").unwrap();
        assert!(!format!("{d:?}").contains("secreta-1234"));
        assert_eq!(publica("lo que sea"), "<url de Phoenix no reconocida>");
    }
}
