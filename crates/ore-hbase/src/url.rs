//! **La URL de conexión**: `hbase://[usuario@]zk1:2181,zk2:2181[/espacio][?znode=/hbase&…]`.
//!
//! Tres oficios, los tres puros: [`analizar`] (lo que la URL dice, o por qué no se admite), [`Destino::opciones`]
//! (las opciones del cliente nativo) y [`publica`] (la URL sin usuario, para enseñarla).
//!
//! Opciones (ficha 0072, §7.1):
//!
//! - **`simple=acepto`** o **`auth=kerberos`**: una de las dos, siempre. La autenticacion SIMPLE de HBase **afirma**
//!   un usuario: no hay secreto, y lo que viaja va sin cifrar; no se conecta sin que la URL lo reconozca.
//! - `auth=kerberos` (SASL/GSSAPI, N6) con `principal=` y `keytab=` (la ruta de un fichero, no un secreto en la URL);
//!   sin ellos, la cache de tickets (`kinit`) o el keytab por defecto de la GSSAPI. `servicio=` (`hbase`). La proteccion
//!   (`authentication`, `integrity`, `privacy`) la dice el servidor. El binario necesita el feature `kerberos`.
//! - `znode=`: la raiz de HBase en ZooKeeper (`/hbase`).
//! - `timeoutMs=`: el tiempo de cada llamada al servidor.
//! - `paralelo=`: lectores a la vez sobre una tabla grande, uno por grupo de regiones (por defecto 4).
//! - `familia=`: la familia de columnas por defecto: un nombre de columna **sin `:`** es el cualificador de esa familia.
//!   Para tablas de una sola familia, donde `familia:cualificador` es ruido.
//! - `clave=`: como se llama a la clave de fila en las peticiones, ademas de `_clave` (`clave=id`).
//! - `rezago=`: milisegundos que el `testigo` del incremental deja sin mirar hacia atras desde el reloj (5000). Una
//!   escritura con marca de hace unos segundos no se pierde si el testigo todavia no la cubre.
//! - `traduccion=`: `anunciado:puerto=real:puerto,…`. El cliente se conecta al nombre que cada RegionServer
//!   anuncia en ZooKeeper; si el job no lo resuelve, esto lo lleva a donde si se llega (§7.3).
//!
//! No hay contraseña en la URL: SIMPLE no la tiene, y una URL con `usuario:clave@` se rechaza.

use ore_hbase_rpc::{Kerberos, Opciones};
use std::time::Duration;

pub const PUERTO_ZK: u16 = 2181;
pub const ZNODE: &str = "/hbase";
pub const TIEMPO_MS: u64 = 30_000;
/// Lectores a la vez por defecto: cuatro, como en Phoenix (§3.1 de la ficha: con una conexion por region, mas no
/// ayuda a un nodo; el reparto por regiones se mide en el laboratorio).
pub const PARALELO: usize = 4;
/// El rezago del `testigo` por defecto: una escritura confirma en milisegundos, pero un cliente con el reloj
/// atrasado o una escritura lenta pone marcas de hace unos segundos. Mas que eso se pide a mano.
pub const REZAGO_MS: u64 = 5_000;
const MAX_REZAGO_MS: u64 = 3_600_000;
const MAX_PARALELO: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destino {
    /// El quorum de ZooKeeper, normalizado: `a:2181,b:2181`.
    pub zk: String,
    pub znode: String,
    /// El espacio de nombres al que se limita el catalogo. Vacio = todos los de usuario.
    pub espacio: Option<String>,
    /// El usuario que el cliente afirma (SIMPLE).
    pub usuario: String,
    pub timeout_ms: Option<u64>,
    pub paralelo: usize,
    pub traduccion: String,
    /// La familia por defecto (`familia=`).
    pub familia: Option<String>,
    /// El otro nombre de la clave de fila (`clave=`).
    pub clave: Option<String>,
    /// Kerberos en vez de SIMPLE (`auth=kerberos`).
    pub kerberos: Option<Kerberos>,
    /// Milisegundos que el `testigo` deja sin mirar hacia atras desde el reloj (`rezago=`).
    pub rezago_ms: u64,
}

fn decodificar(s: &str) -> Result<String, String> {
    let b = s.as_bytes();
    let mut sal = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or("un `%` sin dos cifras hexadecimales detras")?;
            sal.push(h);
            i += 3;
        } else {
            sal.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(sal).map_err(|_| "un valor que no es UTF-8".to_string())
}

fn host_valido(h: &str) -> bool {
    !h.is_empty()
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
}

/// Lo que la URL dice, o por qué no se admite (sin repetir nada que parezca una credencial).
pub fn analizar(url: &str) -> Result<Destino, String> {
    let resto = url.strip_prefix("hbase://").ok_or(
        "la URL de HBase es `hbase://[usuario@]zk1:2181[,zk2:2181][/espacio]?simple=acepto`",
    )?;
    if resto.contains('#') {
        return Err("la URL de HBase no lleva `#`".into());
    }
    let (antes, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let (autoridad, ruta) = match antes.find('/') {
        Some(i) => antes.split_at(i),
        None => (antes, ""),
    };
    let (usuario, hosts) = match autoridad.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, autoridad),
    };
    let usuario_dado = usuario.is_some_and(|u| !u.is_empty());
    if usuario.is_some_and(|u| u.contains(':')) {
        return Err("HBase se autentica con SIMPLE, que no tiene contrasena: la URL no lleva `usuario:clave@`".into());
    }
    let usuario = match usuario {
        Some(u) if !u.is_empty() => decodificar(u)?,
        Some(_) => return Err("un usuario vacio antes de `@`".into()),
        None => "ore".to_string(),
    };
    if hosts.is_empty() {
        return Err("falta el quorum de ZooKeeper (`hbase://zk1:2181`)".into());
    }
    let mut zk = Vec::new();
    for h in hosts.split(',') {
        if !host_valido(h) {
            return Err(format!(
                "`{h}` no es una direccion de ZooKeeper (`host` o `host:puerto`)"
            ));
        }
        let (host, puerto) = match h.rsplit_once(':') {
            Some((a, p)) if !a.is_empty() => {
                let p: u16 = p
                    .parse()
                    .map_err(|_| format!("el puerto `{p}` de `{h}` no es un numero"))?;
                (a.to_string(), p)
            }
            _ => (h.to_string(), PUERTO_ZK),
        };
        zk.push(format!("{host}:{puerto}"));
    }
    let espacio = match ruta.strip_prefix('/').filter(|r| !r.is_empty()) {
        None => None,
        Some(e) if e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => Some(e.to_string()),
        Some(e) => {
            return Err(format!(
                "`{e}` no es un espacio de nombres de HBase (letras, cifras y `_`)"
            ));
        }
    };

    let mut d = Destino {
        zk: zk.join(","),
        znode: ZNODE.to_string(),
        espacio,
        usuario,
        timeout_ms: None,
        paralelo: PARALELO,
        traduccion: String::new(),
        familia: None,
        clave: None,
        kerberos: None,
        rezago_ms: REZAGO_MS,
    };
    let (mut simple, mut auth_kerberos) = (false, false);
    let (mut principal, mut keytab, mut servicio) = (None, None, None);
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        let v = decodificar(v)?;
        match k {
            "simple" if v == "acepto" => simple = true,
            "simple" => return Err("`simple=` solo admite `acepto`".into()),
            "auth" if v == "kerberos" => auth_kerberos = true,
            "auth" => return Err("`auth=` solo admite `kerberos`".into()),
            "principal" if !v.is_empty() => principal = Some(v),
            "keytab" if !v.is_empty() => keytab = Some(v),
            "servicio" if !v.is_empty() && !v.contains(['/', '@']) => servicio = Some(v),
            "principal" | "keytab" | "servicio" => {
                return Err(format!(
                    "`{k}=` no puede ir vacio ni (`servicio`) llevar `/` o `@`"
                ));
            }
            "znode" if v.starts_with('/') && v.len() > 1 => {
                d.znode = v.trim_end_matches('/').to_string()
            }
            "znode" => return Err("`znode=` es una ruta de ZooKeeper (`/hbase`)".into()),
            "timeoutMs" => {
                let ms: u64 = v
                    .parse()
                    .map_err(|_| "`timeoutMs=` no es un numero".to_string())?;
                if !(1..=3_600_000).contains(&ms) {
                    return Err("`timeoutMs=` va de 1 a 3600000".into());
                }
                d.timeout_ms = Some(ms);
            }
            "paralelo" => {
                let n: usize = v
                    .parse()
                    .map_err(|_| "`paralelo=` no es un numero".to_string())?;
                if !(1..=MAX_PARALELO).contains(&n) {
                    return Err(format!("`paralelo=` va de 1 a {MAX_PARALELO}"));
                }
                d.paralelo = n;
            }
            "familia" if !v.is_empty() && !v.contains(':') => d.familia = Some(v),
            "familia" => return Err("`familia=` es el nombre de una familia, sin `:`".into()),
            "clave" if !v.is_empty() && v != "_clave" => d.clave = Some(v),
            "clave" => {
                return Err(
                    "`clave=` es otro nombre para la clave de fila (distinto de `_clave`)".into(),
                );
            }
            "rezago" => {
                d.rezago_ms = v
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n <= MAX_REZAGO_MS)
                    .ok_or_else(|| {
                        format!("`rezago={v}`: son milisegundos de rezago, de 0 a {MAX_REZAGO_MS}")
                    })?;
            }
            "traduccion" => {
                // comprobada al construir las opciones; aqui solo se guarda
                d.traduccion = v;
            }
            otra => {
                return Err(format!(
                    "la opcion `{otra}` no se conoce (simple, auth, principal, keytab, servicio, znode, timeoutMs, paralelo, familia, clave, rezago, traduccion)"
                ));
            }
        }
    }
    if auth_kerberos {
        if simple {
            return Err("`simple=acepto` y `auth=kerberos` son lo contrario: una sola".into());
        }
        if usuario_dado {
            return Err(
                "con Kerberos el usuario es el principal: pon `principal=` y no `usuario@` en la URL".into(),
            );
        }
        d.kerberos = Some(Kerberos {
            principal,
            keytab,
            servicio: servicio.unwrap_or_else(|| "hbase".into()),
        });
    } else if principal.is_some() || keytab.is_some() || servicio.is_some() {
        return Err("`principal=`, `keytab=` y `servicio=` son de `auth=kerberos`".into());
    } else if !simple {
        return Err(
            "HBase se conecta con autenticacion SIMPLE: afirma quien eres, sin secreto, y lo que viaja va sin cifrar. \
             Si la red es de confianza, anade `simple=acepto` a la URL; si no, usa `auth=kerberos`"
                .into(),
        );
    }
    d.opciones(None)?;
    Ok(d)
}

impl Destino {
    /// Las opciones del cliente nativo. `timeout_ms` (el de la peticion) manda sobre el de la URL.
    pub fn opciones(&self, timeout_ms: Option<u64>) -> Result<Opciones, String> {
        let o = Opciones {
            tiempo: Duration::from_millis(timeout_ms.or(self.timeout_ms).unwrap_or(TIEMPO_MS)),
            usuario: self.usuario.clone(),
            kerberos: self.kerberos.clone(),
            ..Opciones::default()
        };
        o.con_traduccion(&self.traduccion)
            .map_err(|e| e.to_string())
    }
}

/// La URL sin el usuario: se puede enseñar.
pub fn publica(url: &str) -> String {
    match url.strip_prefix("hbase://") {
        Some(r) => {
            let (autoridad, resto) = r.split_at(r.find(['/', '?']).unwrap_or(r.len()));
            let hosts = autoridad.rsplit_once('@').map_or(autoridad, |(_, h)| h);
            format!("hbase://{hosts}{resto}")
        }
        None => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_url_completa_se_entiende() {
        let d = analizar(
            "hbase://lector@zk1:2181,zk2/ventas?simple=acepto&znode=/hbase-prod&timeoutMs=5000&paralelo=8&traduccion=rs1:16020%3D10.0.0.5:6020",
        )
        .unwrap();
        assert_eq!(d.zk, "zk1:2181,zk2:2181");
        assert_eq!(d.usuario, "lector");
        assert_eq!(d.espacio.as_deref(), Some("ventas"));
        assert_eq!(d.znode, "/hbase-prod");
        assert_eq!((d.timeout_ms, d.paralelo), (Some(5000), 8));
        assert_eq!(d.traduccion, "rs1:16020=10.0.0.5:6020");
    }

    #[test]
    fn familia_y_clave_son_nombres_cortos_para_tablas_de_una_familia() {
        let d = analizar("hbase://zk?simple=acepto&familia=f&clave=id").unwrap();
        assert_eq!(
            (d.familia.as_deref(), d.clave.as_deref()),
            (Some("f"), Some("id"))
        );
        for mala in [
            "hbase://zk?simple=acepto&familia=f:g",
            "hbase://zk?simple=acepto&familia=",
            "hbase://zk?simple=acepto&clave=_clave",
        ] {
            assert!(analizar(mala).is_err(), "{mala}");
        }
    }

    #[test]
    fn los_valores_por_defecto_son_los_de_la_ficha() {
        let d = analizar("hbase://zk?simple=acepto").unwrap();
        assert_eq!(d.zk, "zk:2181");
        assert_eq!((d.usuario.as_str(), d.znode.as_str()), ("ore", "/hbase"));
        assert_eq!((d.espacio, d.timeout_ms, d.paralelo), (None, None, 4));
        assert_eq!(d.rezago_ms, 5_000);
    }

    #[test]
    fn sin_reconocer_que_simple_no_autentica_no_se_conecta() {
        let e = analizar("hbase://zk:2181").unwrap_err();
        assert!(
            e.contains("simple=acepto") && e.contains("sin cifrar"),
            "{e}"
        );
        assert!(analizar("hbase://zk:2181?simple=si").is_err());
    }

    #[test]
    fn una_contrasena_en_la_url_se_rechaza_sin_repetirla() {
        let e = analizar("hbase://lector:secreto@zk:2181?simple=acepto").unwrap_err();
        assert!(e.contains("contrasena") && !e.contains("secreto"), "{e}");
    }

    #[test]
    fn lo_que_no_es_una_url_de_hbase_se_dice() {
        for mala in [
            "phoenix://h",
            "hbase://",
            "hbase://zk:no?simple=acepto",
            "hbase://zk/a-b?simple=acepto",
            "hbase://zk?simple=acepto&paralelo=0",
            "hbase://zk?simple=acepto&paralelo=17",
            "hbase://zk?simple=acepto&timeoutMs=0",
            "hbase://zk?simple=acepto&znode=hbase",
            "hbase://zk?simple=acepto&rarisima=1",
            "hbase://zk?simple=acepto&rezago=-1",
            "hbase://zk?simple=acepto&rezago=3600001",
            "hbase://zk?simple=acepto&traduccion=sin-igual",
            "hbase://zk?simple=acepto#x",
            "hbase://zk /x?simple=acepto",
        ] {
            assert!(analizar(mala).is_err(), "{mala}");
        }
    }

    #[test]
    fn publica_quita_el_usuario_y_deja_el_resto() {
        assert_eq!(
            publica("hbase://lector@zk1:2181/ventas?simple=acepto"),
            "hbase://zk1:2181/ventas?simple=acepto"
        );
        assert_eq!(
            publica("hbase://zk1:2181?simple=acepto"),
            "hbase://zk1:2181?simple=acepto"
        );
    }

    #[test]
    fn kerberos_se_pide_con_auth_y_no_necesita_simple() {
        let d = analizar(
            "hbase://zk1:2181/ventas?auth=kerberos&principal=lector%40ORE.TEST&keytab=/etc/krb5/lector.keytab",
        )
        .unwrap();
        let k = d.kerberos.clone().unwrap();
        assert_eq!(k.principal.as_deref(), Some("lector@ORE.TEST"));
        assert_eq!(k.keytab.as_deref(), Some("/etc/krb5/lector.keytab"));
        assert_eq!(k.servicio, "hbase");
        assert_eq!(d.opciones(None).unwrap().kerberos, Some(k));
        // sin principal ni keytab: la cache de tickets
        let d = analizar("hbase://zk?auth=kerberos&servicio=hbase2").unwrap();
        let k = d.kerberos.unwrap();
        assert_eq!(
            (k.principal, k.keytab, k.servicio.as_str()),
            (None, None, "hbase2")
        );
        // SIMPLE no lleva Kerberos
        assert!(
            analizar("hbase://zk?simple=acepto")
                .unwrap()
                .kerberos
                .is_none()
        );
    }

    #[test]
    fn kerberos_y_simple_no_se_mezclan_ni_se_adivinan() {
        for mala in [
            "hbase://zk?auth=kerberos&simple=acepto",
            "hbase://lector@zk?auth=kerberos",
            "hbase://zk?auth=ntlm",
            "hbase://zk?simple=acepto&keytab=/k",
            "hbase://zk?principal=a%40B",
            "hbase://zk?auth=kerberos&keytab=",
            "hbase://zk?auth=kerberos&servicio=hbase%2Fx",
        ] {
            assert!(analizar(mala).is_err(), "{mala}");
        }
        let e = analizar("hbase://zk").unwrap_err();
        assert!(e.contains("auth=kerberos"), "{e}");
    }

    #[test]
    fn la_traduccion_llega_a_las_opciones_del_cliente() {
        let d =
            analizar("hbase://zk?simple=acepto&traduccion=rs1:16020%3D127.0.0.1:26020").unwrap();
        let o = d.opciones(Some(1234)).unwrap();
        assert_eq!(o.tiempo, Duration::from_millis(1234));
        assert_eq!(o.traduccion.len(), 1);
    }
}
