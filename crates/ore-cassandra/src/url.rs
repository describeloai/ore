//! **La URL de conexión**: `cassandra://usuario:clave@host1:9042,host2/keyspace?…`,
//! con su credencial dentro (va al cofre como `fuente-<n>`, ficha 0070 §4.6).
//!
//! Tres oficios, los tres puros:
//!
//! - [`analizar`]: lo que la URL dice ([`Destino`]) o por qué no se admite.
//! - [`validar`]: lo mismo, sin el destino. Una verificación de TLS que se puede
//!   apagar por URL se apaga el día que alguien tiene prisa, así que no se ofrece.
//! - [`publica`]: la URL **sin usuario ni contraseña**, para enseñarla.
//!
//! Opciones: `tls=true|false`, `consistency=` (por defecto `LOCAL_QUORUM`),
//! `dc=` (el centro de datos local) y `contacto=solo`: **hablar solo con el punto
//! de contacto**. Sin esa opción el driver descubre los nodos y usa SU dirección,
//! que tras NAT, un balanceador, Docker o Kubernetes no se alcanza (medido en X0:
//! conecta y luego `ConnectTimeout`).

/// Las consistencias que se admiten para leer. `ANY` y las de escritura no
/// existen aquí; `SERIAL` tampoco: un origen se lee.
const CONSISTENCIAS: &[&str] = &[
    "ONE",
    "TWO",
    "THREE",
    "QUORUM",
    "ALL",
    "LOCAL_ONE",
    "LOCAL_QUORUM",
    "EACH_QUORUM",
];

pub const PUERTO: u16 = 9042;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destino {
    pub usuario: Option<String>,
    pub clave: Option<String>,
    /// `(host, puerto)`: los puntos de contacto.
    pub hosts: Vec<(String, u16)>,
    pub keyspace: Option<String>,
    pub tls: bool,
    pub consistencia: String,
    pub dc_local: Option<String>,
    /// Hablar solo con los puntos de contacto, sin las direcciones que el
    /// clúster anuncia.
    pub solo_contacto: bool,
}

/// Lo que la URL dice, o por qué no se admite (sin repetir la credencial).
pub fn analizar(url: &str) -> Result<Destino, String> {
    let resto = url
        .strip_prefix("cassandra://")
        .ok_or("la URL de Cassandra es `cassandra://usuario:clave@host[:puerto]/keyspace`")?;
    if resto.contains('#') {
        return Err("la URL de Cassandra no lleva `#`".into());
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
    let mut destinos = Vec::new();
    for h in hosts.split(',').filter(|h| !h.is_empty()) {
        destinos.push(host_y_puerto(h)?);
    }
    if destinos.is_empty() {
        return Err("la URL de Cassandra no dice a qué servidor conectar".into());
    }
    let keyspace = ruta
        .strip_prefix('/')
        .filter(|k| !k.is_empty())
        .map(decodificar)
        .transpose()?;

    let mut d = Destino {
        usuario,
        clave,
        hosts: destinos,
        keyspace,
        tls: false,
        consistencia: "LOCAL_QUORUM".into(),
        dc_local: None,
        solo_contacto: false,
    };
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (clave, valor) = par.split_once('=').unwrap_or((par, ""));
        let k = clave.to_ascii_lowercase();
        match k.as_str() {
            "tls" | "ssl" => d.tls = booleano(clave, valor)?,
            "consistency" => {
                let c = valor.to_ascii_uppercase();
                if !CONSISTENCIAS.contains(&c.as_str()) {
                    return Err(format!(
                        "`consistency={valor}` no es una consistencia de lectura (admite {})",
                        CONSISTENCIAS.join(", ")
                    ));
                }
                d.consistencia = c;
            }
            "dc" | "localdc" => d.dc_local = Some(decodificar(valor)?),
            "contacto" => match valor {
                "solo" => d.solo_contacto = true,
                "todos" => d.solo_contacto = false,
                otro => return Err(format!("`contacto={otro}`: es `solo` o `todos`")),
            },
            "tlscafile" | "cafile" | "sslrootcert" => {
                return Err(format!(
                    "`{clave}`: una CA propia va por fichero y el driver corre en un Job sin tus ficheros; todavía no se admite"
                ));
            }
            // Lo que apaga la verificación de TLS no se ofrece: se apaga el día que
            // alguien tiene prisa. Dicho en falso no apaga nada.
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
            otra => return Err(format!("`{otra}` no es una opción de la URL de Cassandra")),
        }
    }
    Ok(d)
}

/// `Ok` si la URL se puede usar; si no, por qué.
pub fn validar(url: &str) -> Result<(), String> {
    analizar(url).map(|_| ())
}

/// La URL sin `usuario:contraseña@`. Una URL que no se reconoce se devuelve
/// tapada entera: es mejor no enseñar que enseñar de más.
pub fn publica(url: &str) -> String {
    let Some((esquema, resto)) = url.split_once("://") else {
        return "<url de Cassandra no reconocida>".into();
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
    // Una dirección IPv6 va entre corchetes: `[::1]:9042`.
    if let Some(r) = h.strip_prefix('[') {
        let (dir, resto) = r
            .split_once(']')
            .ok_or("una dirección IPv6 sin `]`".to_string())?;
        let puerto = match resto.strip_prefix(':') {
            Some(p) => puerto(p)?,
            None if resto.is_empty() => PUERTO,
            None => return Err(format!("`{h}` no es un `host:puerto`")),
        };
        return Ok((dir.to_string(), puerto));
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
    String::from_utf8(fuera).map_err(|_| "la URL no es UTF-8 tras decodificar".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lo_que_la_url_dice() {
        let d = analizar(
            "cassandra://ore:p%40ss%3Aw@a:9043,b/ventas?tls=true&consistency=local_one&dc=dc1&contacto=solo",
        )
        .unwrap();
        assert_eq!(d.usuario.as_deref(), Some("ore"));
        assert_eq!(d.clave.as_deref(), Some("p@ss:w"));
        assert_eq!(d.hosts, [("a".into(), 9043), ("b".into(), PUERTO)]);
        assert_eq!(d.keyspace.as_deref(), Some("ventas"));
        assert!(d.tls && d.solo_contacto);
        assert_eq!(d.consistencia, "LOCAL_ONE");
        assert_eq!(d.dc_local.as_deref(), Some("dc1"));
    }

    #[test]
    fn los_valores_por_defecto_son_los_seguros() {
        let d = analizar("cassandra://h").unwrap();
        assert_eq!(d.consistencia, "LOCAL_QUORUM");
        assert!(!d.solo_contacto && !d.tls);
        assert_eq!((d.usuario, d.keyspace), (None, None));
        assert_eq!(d.hosts, [("h".into(), PUERTO)]);
        let v6 = analizar("cassandra://[::1]:9044/k").unwrap();
        assert_eq!(v6.hosts, [("::1".into(), 9044)]);
    }

    #[test]
    fn no_admite_apagar_la_verificacion_de_tls() {
        for o in [
            "tlsInsecure=true",
            "tlsAllowInvalidCertificates=true",
            "tlsAllowInvalidHostnames=1",
            "verify=false",
            "sslVerify=0",
        ] {
            let e = analizar(&format!("cassandra://u:p@h/?tls=true&{o}")).unwrap_err();
            assert!(e.contains("no se admite"), "{o}: {e}");
        }
        assert!(analizar("cassandra://h/?tlsCAFile=/c/ca.pem").is_err());
    }

    #[test]
    fn rechaza_lo_que_no_es_cassandra_o_no_se_entiende() {
        for u in [
            "postgres://u:p@a/db",
            "cassandra://",
            "cassandra://u:p@/k",
            "cassandra://h:0",
            "cassandra://h:abc",
            "cassandra://h/?consistency=ANY",
            "cassandra://h/?consistency=SERIAL",
            "cassandra://h/?contacto=algunos",
            "cassandra://h/?rarisima=1",
            "cassandra://u:100%@h",
        ] {
            assert!(analizar(u).is_err(), "`{u}` debería fallar");
        }
    }

    /// La credencial no sale en lo que se enseña, ni un token.
    #[test]
    fn la_url_publica_no_lleva_la_credencial() {
        let u = "cassandra://victor:s3creta@a:9042,b/ore_prueba?tls=true&token=abc&contacto=solo";
        let p = publica(u);
        assert_eq!(
            p,
            "cassandra://a:9042,b/ore_prueba?tls=true&token=***&contacto=solo"
        );
        assert!(
            !p.contains("s3creta") && !p.contains("victor") && !p.contains("abc"),
            "{p}"
        );
        assert_eq!(publica("lo que sea"), "<url de Cassandra no reconocida>");
    }
}
