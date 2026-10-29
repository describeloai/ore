//! **La URL de CockroachDB** (ADR 0080, paso 1): la de PostgreSQL, porque es la que CockroachDB y sus guías ya dan.
//!
//! `cockroachdb://usuario[:clave]@host[:26257][/base]?sslmode=…&sslrootcert=…&timeoutMs=…&tramos=…` (también `postgresql://` y `postgres://`).
//!
//! **Una lista cerrada de parámetros.** El cliente de PostgreSQL entiende muchos parámetros en la query, y varios **cambian a dónde se conecta**:
//! medido, `cockroachdb://kit@192.0.2.1:1/kit?host=localhost&port=26259` conectaba a `localhost:26259`. La consola y `ore-serve` validan la
//! dirección de la URL (la celda no llega a redes privadas); un parámetro que la sustituyera se saltaría esa validación. Por eso aquí solo entran
//! `sslmode` y `sslrootcert` (que lee [`crate::tls`]), `timeoutMs` y `tramos`; **cualquier otro se rechaza**, también `options` (que no cambia el
//! servidor pero sí la sesión: si hace falta, por ejemplo `--cluster=` de CockroachDB Cloud, entrará aquí como excepción explícita).
//!
//! **Un solo servidor, sin puerto implícito de PostgreSQL.** La dirección lleva un host y un puerto; si no hay puerto es el **26257** de CockroachDB
//! (el cliente pondría el 5432); varios servidores separados por comas y un *socket* de Unix no se admiten.

use ore_driver::{Codigo, Fallo};

/// Una URL ya lista para el cliente de PostgreSQL.
#[derive(Debug, PartialEq, Eq)]
pub struct Destino {
    /// La URL con esquema `postgresql://`, con puerto y sin `timeoutMs` ni `tramos`.
    pub url: String,
    pub timeout_ms: Option<u64>,
    /// Cuántas conexiones leen a la vez una tabla grande (`tramos=N`); `1` lo apaga. `None`: [`TRAMOS`].
    pub tramos: Option<usize>,
}

/// Cuántas conexiones leen a la vez una tabla grande, si la URL no dice otra cosa.
pub const TRAMOS: usize = 4;
/// El techo de `tramos`: más conexiones que esto no suman y sí cuestan en el servidor.
pub const TRAMOS_MAX: usize = 16;
/// El puerto SQL de CockroachDB, el que se usa si la URL no dice otro.
pub const PUERTO: u16 = 26257;

const ESQUEMAS: [&str; 3] = ["cockroachdb://", "postgresql://", "postgres://"];

/// Lo que pasa tal cual al cliente (lo lee [`crate::tls`]); `timeoutMs` y `tramos` se quedan aquí.
const ADMITIDOS: [&str; 2] = ["sslmode", "sslrootcert"];

/// Un defecto de la URL: es de quien la escribió, y volver a intentarlo no lo arregla.
fn rechazo(mensaje: impl Into<String>) -> Fallo {
    Fallo::new(Codigo::Conexion, mensaje).reintentable(false)
}

/// La dirección de la URL (`usuario@host:puerto`), comprobada y con su puerto: un solo servidor.
fn servidor(autoridad: &str) -> Result<String, Fallo> {
    let (info, hostport) = match autoridad.rsplit_once('@') {
        Some((i, h)) => (Some(i), h),
        None => (None, autoridad),
    };
    if hostport.is_empty() {
        return Err(rechazo(
            "la URL de CockroachDB no nombra el servidor: `cockroachdb://usuario@host:26257/base`",
        ));
    }
    if hostport.contains(',') {
        return Err(rechazo(
            "la URL de CockroachDB nombra varios servidores (una coma en la dirección): se lee de uno solo",
        ));
    }
    let bajo = hostport.to_ascii_lowercase();
    if hostport.starts_with('/') || bajo.contains("%2f") || bajo.contains("%5c") {
        return Err(rechazo(
            "la dirección de la URL de CockroachDB es un nombre de servidor, no una ruta ni un socket de Unix",
        ));
    }
    let (host, puerto) = if hostport.starts_with('[') {
        // IPv6: `[::1]` o `[::1]:26257`.
        let fin = hostport
            .find(']')
            .ok_or_else(|| rechazo("la dirección IPv6 de la URL no cierra el corchete"))?;
        let resto = &hostport[fin + 1..];
        match resto.strip_prefix(':') {
            Some(p) => (&hostport[..=fin], Some(p)),
            None if resto.is_empty() => (&hostport[..=fin], None),
            None => return Err(rechazo("la dirección IPv6 de la URL no es válida")),
        }
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if host.is_empty() {
        return Err(rechazo(
            "la URL de CockroachDB no nombra el servidor: `cockroachdb://usuario@host:26257/base`",
        ));
    }
    let puerto = match puerto {
        None => PUERTO,
        Some(p) => p
            .parse::<u16>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| rechazo(format!("`{p}` no es un puerto (1 a 65535)")))?,
    };
    Ok(match info {
        Some(i) => format!("{i}@{host}:{puerto}"),
        None => format!("{host}:{puerto}"),
    })
}

pub fn analizar(url: &str) -> Result<Destino, Fallo> {
    let url = url.trim();
    let Some(esquema) = ESQUEMAS.iter().find(|e| url.starts_with(**e)) else {
        return Err(rechazo(
            "la URL de CockroachDB empieza por `cockroachdb://` (o `postgresql://`)",
        ));
    };
    let resto = &url[esquema.len()..];
    let (cuerpo, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let (autoridad, ruta) = match cuerpo.find('/') {
        Some(i) => (&cuerpo[..i], &cuerpo[i..]),
        None => (cuerpo, ""),
    };
    let autoridad = servidor(autoridad)?;
    let mut timeout_ms = None;
    let mut tramos = None;
    let mut quedan = Vec::new();
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (clave, v) = par.split_once('=').unwrap_or((par, ""));
        match clave {
            "timeoutMs" => {
                timeout_ms = Some(v.parse::<u64>().map_err(|_| {
                    rechazo(format!("`timeoutMs={v}` no es un número de milisegundos"))
                })?);
            }
            "tramos" => {
                let n = v
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=TRAMOS_MAX).contains(n))
                    .ok_or_else(|| {
                        rechazo(format!(
                            "`tramos={v}` no es un número de conexiones entre 1 y {TRAMOS_MAX}"
                        ))
                    })?;
                tramos = Some(n);
            }
            c if ADMITIDOS.contains(&c) => quedan.push(par),
            otro => {
                let servidor = if matches!(otro, "host" | "hostaddr" | "port") {
                    " — el servidor sale solo de la dirección de la URL (`usuario@host:puerto`)"
                } else {
                    ""
                };
                return Err(rechazo(format!(
                    "el parámetro `{otro}` no se admite en la URL de CockroachDB (se admiten `sslmode`, `sslrootcert`, `timeoutMs` y `tramos`){servidor}"
                )));
            }
        }
    }
    let mut url = format!("postgresql://{autoridad}{ruta}");
    if !quedan.is_empty() {
        url.push('?');
        url.push_str(&quedan.join("&"));
    }
    Ok(Destino {
        url,
        timeout_ms,
        tramos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url_de(u: &str) -> String {
        analizar(u).unwrap_or_else(|f| panic!("{u}: {f:?}")).url
    }

    fn rechazada(u: &str) -> Fallo {
        let f = analizar(u).expect_err(u);
        assert_eq!(f.codigo, Codigo::Conexion, "{u}");
        assert!(
            !f.reintentable,
            "{u}: un defecto de la URL no se arregla reintentando"
        );
        f
    }

    #[test]
    fn el_esquema_propio_se_vuelve_el_de_postgres() {
        let d = analizar("cockroachdb://kit@localhost:26257/kit").unwrap();
        assert_eq!(d.url, "postgresql://kit@localhost:26257/kit");
        assert_eq!(d.timeout_ms, None);
    }

    #[test]
    fn timeout_ms_sale_y_sslmode_y_la_ca_se_quedan() {
        let d = analizar(
            "cockroachdb://u:p@h:26257/db?sslmode=verify-full&timeoutMs=5000&sslrootcert=-----BEGIN%20CERTIFICATE-----%0AAB%0A-----END%20CERTIFICATE-----",
        )
        .unwrap();
        assert_eq!(
            d.url,
            "postgresql://u:p@h:26257/db?sslmode=verify-full&sslrootcert=-----BEGIN%20CERTIFICATE-----%0AAB%0A-----END%20CERTIFICATE-----"
        );
        assert_eq!(d.timeout_ms, Some(5000));
    }

    #[test]
    fn postgres_y_postgresql_valen() {
        assert!(analizar("postgresql://u@h/db").is_ok());
        assert!(analizar("postgres://u@h/db").is_ok());
    }

    #[test]
    fn otro_esquema_o_sin_servidor_se_niega() {
        for u in [
            "mysql://u@h/db",
            "http://h",
            "cockroachdb://",
            "cockroachdb:///db",
            "cockroachdb://@/db",
            "cockroachdb://kit@/db",
            "cockroachdb://kit@",
            "cockroachdb://:26257/db",
        ] {
            rechazada(u);
        }
    }

    #[test]
    fn sin_puerto_es_el_de_cockroachdb_y_no_el_de_postgres() {
        assert_eq!(
            url_de("cockroachdb://kit@localhost/kit"),
            "postgresql://kit@localhost:26257/kit"
        );
        assert_eq!(
            url_de("cockroachdb://kit@localhost"),
            "postgresql://kit@localhost:26257"
        );
        assert_eq!(
            url_de("cockroachdb://localhost/kit"),
            "postgresql://localhost:26257/kit"
        );
        assert_eq!(
            url_de("postgres://u:p%40q@db.ejemplo.com/x?sslmode=require"),
            "postgresql://u:p%40q@db.ejemplo.com:26257/x?sslmode=require"
        );
        // Con puerto, el suyo: también el 5432, si es lo que se escribió.
        assert_eq!(
            url_de("cockroachdb://kit@h:5432/kit"),
            "postgresql://kit@h:5432/kit"
        );
        assert_eq!(
            url_de("cockroachdb://kit@h:1/kit"),
            "postgresql://kit@h:1/kit"
        );
    }

    #[test]
    fn ipv6_con_y_sin_puerto() {
        assert_eq!(
            url_de("cockroachdb://kit@[::1]/kit"),
            "postgresql://kit@[::1]:26257/kit"
        );
        assert_eq!(
            url_de("cockroachdb://kit@[::1]:26260/kit"),
            "postgresql://kit@[::1]:26260/kit"
        );
        rechazada("cockroachdb://kit@[::1/kit");
        rechazada("cockroachdb://kit@[::1]x/kit");
    }

    #[test]
    fn un_puerto_que_no_lo_es_se_niega() {
        for p in ["", "0", "65536", "abc", "-1", "26257x"] {
            let f = rechazada(&format!("cockroachdb://kit@h:{p}/kit"));
            assert!(f.mensaje.contains("puerto"), "{p}: {}", f.mensaje);
        }
    }

    #[test]
    fn un_parametro_que_cambia_el_servidor_se_rechaza_y_dice_por_que() {
        // El caso medido: la URL decía 192.0.2.1:1 y el parámetro llevaba a localhost.
        let f = rechazada("cockroachdb://kit@192.0.2.1:1/kit?host=localhost&port=26259");
        assert!(
            f.mensaje.contains("`host`") && f.mensaje.contains("la dirección de la URL"),
            "{}",
            f.mensaje
        );
        for malo in [
            "host=otro",
            "hostaddr=127.0.0.1",
            "port=1",
            "service=x",
            "dbname=otra",
            "user=otro",
            "password=x",
            "passfile=/etc/x",
        ] {
            let f = rechazada(&format!("cockroachdb://kit@h:26257/kit?{malo}"));
            assert!(f.mensaje.contains("no se admite"), "{malo}: {}", f.mensaje);
        }
        // Delante, detrás o entre los admitidos: no se cuela.
        rechazada("cockroachdb://kit@h/kit?sslmode=verify-full&host=otro");
        rechazada("cockroachdb://kit@h/kit?host=otro&sslmode=verify-full");
        // Ni disfrazado: otra capitalización, la clave codificada, sin valor.
        rechazada("cockroachdb://kit@h/kit?HOST=otro");
        rechazada("cockroachdb://kit@h/kit?Host=otro");
        rechazada("cockroachdb://kit@h/kit?ho%73t=otro");
        rechazada("cockroachdb://kit@h/kit?host");
    }

    #[test]
    fn lo_que_cambia_la_sesion_o_el_destino_del_tls_tampoco_entra() {
        // `options` no cambia el servidor, pero sí la sesión: entra el día que haga falta (CockroachDB Cloud), explícitamente.
        for malo in [
            "options=--cluster%3Dx",
            "options=-c%20default_transaction_read_only%3Doff",
            "application_name=otro",
            "connect_timeout=1",
            "channel_binding=disable",
            "target_session_attrs=any",
            "load_balance_hosts=random",
            "sslcert=/x",
            "sslkey=/x",
            "gssencmode=disable",
        ] {
            let f = rechazada(&format!("cockroachdb://kit@h/kit?{malo}"));
            assert!(
                f.mensaje
                    .contains("se admiten `sslmode`, `sslrootcert`, `timeoutMs` y `tramos`"),
                "{malo}: {}",
                f.mensaje
            );
        }
    }

    #[test]
    fn un_solo_servidor_y_ningun_socket() {
        let f = rechazada("cockroachdb://kit@a:26257,b:26257/kit");
        assert!(f.mensaje.contains("varios servidores"), "{}", f.mensaje);
        rechazada("cockroachdb://kit@a,b/kit");
        for socket in ["%2Fvar%2Frun%2Fpostgresql", "%2fvar%2frun", "%5Cvar"] {
            let f = rechazada(&format!("cockroachdb://kit@{socket}/kit"));
            assert!(f.mensaje.contains("socket"), "{socket}: {}", f.mensaje);
        }
        // Con la barra sin codificar es una ruta, y la dirección queda vacía.
        let f = rechazada("cockroachdb://kit@/var/run/postgresql/kit");
        assert!(f.mensaje.contains("no nombra el servidor"), "{}", f.mensaje);
    }

    #[test]
    fn una_contrasena_con_arroba_o_dos_puntos_no_confunde_al_servidor() {
        // La clave codificada lleva `@` y `:`; el servidor es lo que va tras el último `@` sin codificar.
        assert_eq!(
            url_de("cockroachdb://kit:a%40b%3Ac@h.ejemplo.com/kit"),
            "postgresql://kit:a%40b%3Ac@h.ejemplo.com:26257/kit"
        );
    }

    #[test]
    fn tramos_sale_de_la_url_y_se_acota() {
        let d = analizar("cockroachdb://u@h:26257/db?tramos=8&timeoutMs=100").unwrap();
        assert_eq!(
            (d.tramos, d.url.as_str()),
            (Some(8), "postgresql://u@h:26257/db")
        );
        assert_eq!(analizar("cockroachdb://u@h/db").unwrap().tramos, None);
        for malo in ["0", "17", "muchos", "-1"] {
            let f = rechazada(&format!("cockroachdb://u@h/db?tramos={malo}"));
            assert!(f.mensaje.contains("tramos"), "{malo}: {}", f.mensaje);
        }
    }

    #[test]
    fn timeout_que_no_es_numero_se_niega() {
        let f = rechazada("cockroachdb://u@h/db?timeoutMs=mucho");
        assert!(f.mensaje.contains("timeoutMs"), "{}", f.mensaje);
    }
}
