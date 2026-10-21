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
//! - `frame=`: filas por frame (por defecto 20 000; §3.7 mide que ≥ 10 000 es lo que rinde).
//!
//! Lo que apaga la verificación de TLS no se ofrece, y una CA propia por fichero tampoco: el conector
//! corre en un Job sin tus ficheros.

pub const PUERTO: u16 = 8765;
pub const FILAS_POR_FRAME: i32 = 20_000;
const MAX_FILAS_POR_FRAME: i32 = 500_000;
/// Conexiones que leen a la vez una tabla grande, por defecto. Medido contra el laboratorio (10⁶ filas): 1 → 22,9 s,
/// 2 → 9,5 s, 4 → 6,2 s, 8 → 7,4 s: con un solo nodo y un PQS de 1 GB, 4 es el punto.
pub const PARALELO: usize = 4;
const MAX_PARALELO: usize = 16;

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
    let (host, puerto) = host_y_puerto(hosts)?;
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

/// La URL sin `usuario:contraseña@`. Una URL que no se reconoce se devuelve tapada entera.
pub fn publica(url: &str) -> String {
    let Some((esquema, resto)) = url.split_once("://") else {
        return "<url de Phoenix no reconocida>".into();
    };
    let (antes, consulta) = match resto.split_once('?') {
        Some((a, c)) => (a, Some(c)),
        None => (resto, None),
    };
    let (autoridad, ruta) = match antes.find('/') {
        Some(i) => antes.split_at(i),
        None => (antes, ""),
    };
    let hosts = autoridad.rsplit_once('@').map_or(autoridad, |(_, h)| h);
    let mut s = format!("{esquema}://{hosts}{ruta}");
    if let Some(c) = consulta {
        let visibles: Vec<String> = c
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, _) = p.split_once('=').unwrap_or((p, ""));
                let kl = k.to_ascii_lowercase();
                if kl.contains("password") || kl.contains("secret") || kl.contains("token") {
                    format!("{k}=***")
                } else {
                    p.to_string()
                }
            })
            .collect();
        if !visibles.is_empty() {
            s.push('?');
            s.push_str(&visibles.join("&"));
        }
    }
    s
}

fn booleano(clave: &str, valor: &str) -> Result<bool, String> {
    match valor {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        otro => Err(format!("`{clave}={otro}`: es `true` o `false`")),
    }
}

fn host_y_puerto(h: &str) -> Result<(String, u16), String> {
    // Una dirección IPv6 va entre corchetes: `[::1]:8765`.
    if let Some(r) = h.strip_prefix('[') {
        let (dir, resto) = r
            .split_once(']')
            .ok_or("una dirección IPv6 sin `]`".to_string())?;
        let p = match resto.strip_prefix(':') {
            Some(p) => puerto(p)?,
            None if resto.is_empty() => PUERTO,
            None => return Err(format!("`{h}` no es un `host:puerto`")),
        };
        return Ok((dir.to_string(), p));
    }
    match h.rsplit_once(':') {
        Some((host, p)) if !host.is_empty() => Ok((host.to_string(), puerto(p)?)),
        Some(_) => Err(format!("`{h}` no es un `host:puerto`")),
        None => Ok((h.to_string(), PUERTO)),
    }
}

fn puerto(p: &str) -> Result<u16, String> {
    p.parse::<u16>()
        .ok()
        .filter(|n| *n != 0)
        .ok_or_else(|| format!("`{p}` no es un puerto"))
}

/// `%40` es una arroba que no parte la URL: la contraseña viene codificada.
fn decodificar(s: &str) -> Result<String, String> {
    let b = s.as_bytes();
    let mut fuera = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or("un `%` mal codificado en la URL")?;
            fuera.push(hex);
            i += 3;
        } else {
            fuera.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(fuera).map_err(|_| "la URL no es UTF-8 una vez descodificada".to_string())
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
