//! **La URL de conexión**: `oracle://usuario:clave@host:1521/SERVICIO?tls=true&esquema=VENTAS`, con su
//! credencial dentro (va al cofre como `fuente-<n>`, como la de Cassandra y Phoenix).
//!
//! Tres oficios, los tres puros: [`analizar`] (lo que la URL dice, o por qué no se admite), [`validar`] y
//! [`publica`] (la URL sin usuario ni contraseña, para enseñarla).
//!
//! La ruta es el **nombre de servicio** (`FREEPDB1`, `XEPDB1`, `ORCLPDB1`), el que Oracle llama
//! `SERVICE_NAME`. Un SID de los de antes no es un nombre de servicio y el Easy Connect no lo expresa.
//!
//! Opciones:
//!
//! - `tls=true|false`: `tcps://` al listener. Oracle cifra el *login* (O5LOGON) aunque no haya TLS, pero los
//!   datos van en claro: es la diferencia, y por eso `false` es una decisión que se toma a sabiendas.
//! - `esquema=`: el esquema al que se limita el catálogo (el dueño de las tablas, en mayúsculas). Sin él, todos
//!   los que el usuario puede leer, menos los que Oracle mantiene (`ORACLE_MAINTAINED = 'Y'`).
//! - `timeoutMs=`: el tiempo límite de cada llamada al servidor (`call_timeout`). Al vencer, la conexión se
//!   cierra: el driver no sabe recuperarse de un corte a mitad de una respuesta.
//! - `fetch=`: filas por viaje al servidor (`fetch_array_size`; por defecto 5 000, el óptimo medido).
//!
//! Lo que apaga la verificación de TLS no se ofrece, y una CA o una wallet por fichero tampoco: el conector
//! corre en un Job sin tus ficheros.

pub const PUERTO: u16 = 1521;
/// Filas por viaje. Medido con 10⁷ filas (2026-10-25): 100 → 116 k filas/s, 1 000 → 266 k, **5 000 → 302 k**,
/// 20 000 → 276 k (y el triple de memoria).
pub const FILAS_POR_VIAJE: u32 = 5_000;
const MAX_FILAS_POR_VIAJE: u32 = 100_000;

#[derive(Clone, PartialEq, Eq)]
pub struct Destino {
    pub usuario: Option<String>,
    pub clave: Option<String>,
    pub host: String,
    pub puerto: u16,
    /// El nombre de servicio.
    pub servicio: String,
    /// El esquema al que se limita el catálogo. `None` = todos los visibles que no mantiene Oracle.
    pub esquema: Option<String>,
    pub tls: bool,
    pub timeout_ms: Option<u64>,
    pub filas_por_viaje: u32,
}

impl std::fmt::Debug for Destino {
    /// La contraseña no sale nunca, ni en un `{:?}` perdido.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Destino")
            .field("usuario", &self.usuario)
            .field("clave", &self.clave.as_ref().map(|_| "***"))
            .field("host", &self.host)
            .field("puerto", &self.puerto)
            .field("servicio", &self.servicio)
            .field("esquema", &self.esquema)
            .field("tls", &self.tls)
            .field("timeout_ms", &self.timeout_ms)
            .field("filas_por_viaje", &self.filas_por_viaje)
            .finish()
    }
}

impl Destino {
    /// El Easy Connect de Oracle, sin credencial: `host:puerto/servicio` o `tcps://host:puerto/servicio`.
    pub fn cadena(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let base = format!("{host}:{}/{}", self.puerto, self.servicio);
        if self.tls {
            format!("tcps://{base}")
        } else {
            base
        }
    }
}

/// Lo que la URL dice, o por qué no se admite (sin repetir la credencial).
pub fn analizar(url: &str) -> Result<Destino, String> {
    let resto = url
        .strip_prefix("oracle://")
        .ok_or("la URL de Oracle es `oracle://usuario:clave@host[:1521]/SERVICIO`")?;
    if resto.contains('#') {
        return Err("la URL de Oracle no lleva `#`".into());
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
        return Err("la URL de Oracle nombra un solo listener (no una lista de hosts)".into());
    }
    if hosts.is_empty() {
        return Err("la URL de Oracle no dice a qué servidor conectar".into());
    }
    let (host, puerto) = host_y_puerto(hosts)?;
    let servicio = ruta
        .strip_prefix('/')
        .filter(|k| !k.is_empty())
        .map(decodificar)
        .transpose()?
        .ok_or("falta el nombre de servicio: `oracle://usuario:clave@host:1521/SERVICIO`")?;
    if servicio.contains('/') || servicio.contains(' ') {
        return Err(format!(
            "`{servicio}` no es un nombre de servicio (sin barras ni espacios)"
        ));
    }

    let mut d = Destino {
        usuario,
        clave,
        host,
        puerto,
        servicio,
        esquema: None,
        tls: false,
        timeout_ms: None,
        filas_por_viaje: FILAS_POR_VIAJE,
    };
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (clave, valor) = par.split_once('=').unwrap_or((par, ""));
        match clave.to_ascii_lowercase().as_str() {
            "tls" | "ssl" => d.tls = booleano(clave, valor)?,
            "esquema" | "schema" => {
                d.esquema = Some(decodificar(valor)?).filter(|e| !e.is_empty());
            }
            "timeoutms" => {
                d.timeout_ms = Some(valor.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(
                    || format!("`{clave}={valor}`: son milisegundos, un entero mayor que 0"),
                )?)
            }
            "fetch" | "filas" => {
                d.filas_por_viaje = valor
                    .parse::<u32>()
                    .ok()
                    .filter(|n| (1..=MAX_FILAS_POR_VIAJE).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "`{clave}={valor}`: son filas por viaje, de 1 a {MAX_FILAS_POR_VIAJE}"
                        )
                    })?
            }
            "wallet" | "wallet_location" | "tlscafile" | "cafile" | "sslrootcert" => {
                return Err(format!(
                    "`{clave}`: una wallet o una CA propia van por fichero y el conector corre en un Job sin tus ficheros; todavía no se admite"
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
            "verify" | "sslverify" | "ssl_server_dn_match" => {
                if matches!(valor, "false" | "0" | "no") {
                    return Err(format!(
                        "`{clave}={valor}` apaga la verificación de TLS y no se admite"
                    ));
                }
            }
            otra => return Err(format!("`{otra}` no es una opción de la URL de Oracle")),
        }
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
        return "<url de Oracle no reconocida>".into();
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
    // Una dirección IPv6 va entre corchetes: `[::1]:1521`.
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
    fn una_url_completa() {
        let d = analizar(
            "oracle://ore:p%40ss@db.ejemplo.com:1522/FREEPDB1?tls=true&esquema=VENTAS&timeoutMs=30000&fetch=2000",
        )
        .unwrap();
        assert_eq!(d.usuario.as_deref(), Some("ore"));
        assert_eq!(d.clave.as_deref(), Some("p@ss"));
        assert_eq!((d.host.as_str(), d.puerto), ("db.ejemplo.com", 1522));
        assert_eq!(d.servicio, "FREEPDB1");
        assert_eq!(d.esquema.as_deref(), Some("VENTAS"));
        assert!(d.tls);
        assert_eq!(d.timeout_ms, Some(30_000));
        assert_eq!(d.filas_por_viaje, 2000);
        assert_eq!(d.cadena(), "tcps://db.ejemplo.com:1522/FREEPDB1");
    }

    #[test]
    fn los_valores_por_defecto() {
        let d = analizar("oracle://ore:ore@localhost/XEPDB1").unwrap();
        assert_eq!(d.puerto, 1521);
        assert!(!d.tls && d.esquema.is_none() && d.timeout_ms.is_none());
        assert_eq!(d.filas_por_viaje, FILAS_POR_VIAJE);
        assert_eq!(d.cadena(), "localhost:1521/XEPDB1");
        // Sin credencial también vale: la pone quien abre.
        let d = analizar("oracle://[::1]:1521/S").unwrap();
        assert_eq!(d.cadena(), "[::1]:1521/S");
        assert!(d.usuario.is_none());
    }

    #[test]
    fn lo_que_no_se_admite_dice_por_que() {
        for (url, trozo) in [
            ("postgres://x/y", "oracle://"),
            ("oracle://u:p@h", "nombre de servicio"),
            ("oracle://u:p@h/", "nombre de servicio"),
            ("oracle://u@h/S", "usuario:clave"),
            ("oracle://u:p@a,b/S", "un solo listener"),
            ("oracle://u:p@h/S?tls=quizas", "true"),
            ("oracle://u:p@h/S?fetch=0", "filas por viaje"),
            ("oracle://u:p@h/S?timeoutMs=-1", "milisegundos"),
            ("oracle://u:p@h/S?wallet=/x", "Job sin tus ficheros"),
            ("oracle://u:p@h/S?tlsInsecure=true", "no se admite"),
            ("oracle://u:p@h/S?verify=false", "no se admite"),
            ("oracle://u:p@h/S?cosa=1", "no es una opción"),
            ("oracle://u:p@h:0/S", "puerto"),
            ("oracle://u:p%zz@h/S", "mal codificado"),
        ] {
            let e = analizar(url).unwrap_err();
            assert!(e.contains(trozo), "{url}: {e}");
        }
        // Dicho en falso, apagar la verificación no apaga nada.
        assert!(analizar("oracle://u:p@h/S?tlsInsecure=false&verify=true").is_ok());
    }

    #[test]
    fn la_contrasena_no_sale() {
        let d = analizar("oracle://ore:secreto@h/S").unwrap();
        assert!(!format!("{d:?}").contains("secreto"));
        let p = publica("oracle://ore:secreto@h:1521/S?tls=true&password=otro");
        assert_eq!(p, "oracle://h:1521/S?tls=true&password=***");
        assert!(!p.contains("secreto"));
        assert_eq!(publica("sin esquema"), "<url de Oracle no reconocida>");
    }
}
