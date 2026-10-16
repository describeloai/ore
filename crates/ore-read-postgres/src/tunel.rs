//! **Un Postgres detrás de un bastión SSH** (ADR 0065 PG4, D-PG3).
//!
//! Es el estándar del sector para una base en una subred a la que no se llega
//! (Fivetran, Airbyte): el cliente pega la clave pública del proveedor en el
//! `authorized_keys` de un bastión que sí ve la base. Aquí la identidad es la de
//! SFTP (D-O4), la misma pieza (`ore_sftp::sesion`): la clave Ed25519 **de la
//! celda** (`ORE_SFTP_CLAVE`) y la huella del host **siempre fijada**, comparada
//! antes de autenticar.
//!
//! ```text
//! postgres://u:p@<host de la base>:5432/db?ssh=<usuario>@<bastión>[:<puerto>]&ssh_huella=SHA256:<…>[&ssh_legado=1]
//! ```
//!
//! **El reenvío vive dentro del proceso**: un `127.0.0.1:<efímero>` que sólo oye
//! este driver, y por cada conexión aceptada una sesión SSH con su canal
//! `direct-tcpip` hasta `<host de la base>:<puerto>`. La sesión de Postgres va a
//! esa dirección con `hostaddr`, y **conserva el nombre del host**: TLS va de
//! punta a punta por dentro del túnel y verifica el certificado de la base, no
//! el del bastión. Cancelar en el origen abre otra conexión por el mismo
//! reenvío.

use ore_driver::{Codigo, Fallo};
use ore_sftp::ssh2;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// El bastión que pide la URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Bastion {
    pub usuario: String,
    pub host: String,
    pub puerto: u16,
    pub huella: Option<String>,
    pub legado: bool,
}

/// `%XX`, sin tocar `+`: una huella en base64 lo lleva.
fn descodificar(v: &str) -> String {
    let b = v.as_bytes();
    let mut s = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(x) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            s.push(x);
            i += 3;
            continue;
        }
        s.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&s).into_owned()
}

/// **Separa de la URL lo del túnel** (`ssh`, `ssh_huella`, `ssh_legado`). La
/// URL que queda es la de la base, tal cual.
pub fn separar(url: &str) -> Result<(String, Option<Bastion>), Fallo> {
    let Some((base, consulta)) = url.split_once('?').filter(|_| url.contains("://")) else {
        if url.split_whitespace().any(|t| t.starts_with("ssh")) {
            return Err(Fallo::new(
                Codigo::Credencial,
                "el túnel SSH sólo se dice en la forma URL (`postgres://…?ssh=…`)",
            ));
        }
        return Ok((url.to_string(), None));
    };
    let (mut ssh, mut huella, mut legado) = (None, None, false);
    let mut quedan = Vec::new();
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        match k {
            "ssh" => ssh = Some(descodificar(v)),
            "ssh_huella" => huella = Some(descodificar(v)),
            "ssh_legado" => legado = v == "1",
            _ => quedan.push(par),
        }
    }
    let limpia = if quedan.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", quedan.join("&"))
    };
    let Some(ssh) = ssh else {
        if huella.is_some() || legado {
            return Err(Fallo::new(
                Codigo::Credencial,
                "`ssh_huella` o `ssh_legado` sin `ssh=<usuario>@<bastión>`",
            ));
        }
        return Ok((limpia, None));
    };
    let (usuario, resto) = ssh.split_once('@').ok_or_else(|| {
        Fallo::new(
            Codigo::Credencial,
            format!("`ssh={ssh}`: falta el usuario del bastión (`ssh=<usuario>@<bastión>`)"),
        )
    })?;
    let (host, puerto) = match resto.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse().map_err(|_| {
                Fallo::new(
                    Codigo::Credencial,
                    format!("`ssh={ssh}`: `{p}` no es un puerto"),
                )
            })?,
        ),
        None => (resto, 22),
    };
    if usuario.is_empty() || host.is_empty() {
        return Err(Fallo::new(
            Codigo::Credencial,
            format!("`ssh={ssh}` no es `<usuario>@<bastión>[:<puerto>]`"),
        ));
    }
    Ok((
        limpia,
        Some(Bastion {
            usuario: usuario.into(),
            host: host.into(),
            puerto,
            huella,
            legado,
        }),
    ))
}

/// **El destino del túnel** (`host`, `puerto`) y la URL **con el puerto del
/// reenvío**: el host se queda para TLS y para el nombre; el puerto pasa a ser
/// el local (con `hostaddr` = 127.0.0.1, la conexión va al reenvío).
pub fn destino(url: &str, local: u16) -> Result<(String, String, u16), Fallo> {
    let mal = |m: &str| Fallo::new(Codigo::Credencial, format!("la URL con túnel: {m}"));
    let (esquema, resto) = url.split_once("://").ok_or_else(|| mal("no es una URL"))?;
    let fin = resto.find(['/', '?']).unwrap_or(resto.len());
    let (autoridad, cola) = resto.split_at(fin);
    let (usuario, hostport) = match autoridad.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, autoridad),
    };
    if hostport.contains(',') {
        return Err(mal("un túnel lleva a un solo host"));
    }
    let (host, puerto) = match hostport
        .rsplit_once(':')
        .filter(|_| !hostport.ends_with(']'))
    {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| mal(&format!("`{p}` no es un puerto")))?,
        ),
        None => (hostport.to_string(), 5432),
    };
    if host.is_empty() {
        return Err(mal("falta el host de la base"));
    }
    let autoridad = match usuario {
        Some(u) => format!("{u}@{host}:{local}"),
        None => format!("{host}:{local}"),
    };
    Ok((format!("{esquema}://{autoridad}{cola}"), host, puerto))
}

/// Copia de un lado al otro hasta que alguno cierra. La sesión va sin bloquear
/// y se sondea: un canal de `ssh2` no se puede leer y escribir desde dos hilos.
fn bombear(mut tcp: TcpStream, sesion: ssh2::Session, mut canal: ssh2::Channel) {
    let _ = tcp.set_nonblocking(true);
    sesion.set_blocking(false);
    let mut buf = vec![0u8; 32 * 1024];
    let escribir_canal = |canal: &mut ssh2::Channel, mut datos: &[u8]| -> bool {
        while !datos.is_empty() {
            match canal.write(datos) {
                Ok(n) => datos = &datos[n..],
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(_) => return false,
            }
        }
        true
    };
    let escribir_tcp = |tcp: &mut TcpStream, mut datos: &[u8]| -> bool {
        while !datos.is_empty() {
            match tcp.write(datos) {
                Ok(n) => datos = &datos[n..],
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(_) => return false,
            }
        }
        true
    };
    loop {
        let mut algo = false;
        match tcp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if !escribir_canal(&mut canal, &buf[..n]) {
                    break;
                }
                algo = true;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
        match canal.read(&mut buf) {
            Ok(0) => {
                if canal.eof() {
                    break;
                }
            }
            Ok(n) => {
                if !escribir_tcp(&mut tcp, &buf[..n]) {
                    break;
                }
                algo = true;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
        if !algo {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let _ = canal.close();
}

/// Una sesión con el bastión y su canal hasta la base, o el error que dice qué
/// falló: la huella, la clave o que el bastión no llega a la base.
fn canal(b: &Bastion, host: &str, puerto: u16) -> Result<(ssh2::Session, ssh2::Channel), Fallo> {
    let f = ore_sftp::Fuente::bastion(&b.usuario, &b.host, b.puerto, b.huella.clone(), b.legado);
    let s = ore_sftp::sesion(&f).map_err(|e| {
        let codigo = match e.tipo {
            ore_sftp::Tipo::Conexion => Codigo::Conexion,
            _ => Codigo::Credencial,
        };
        Fallo::new(codigo, format!("el bastión SSH: {}", e.mensaje))
    })?;
    let c = s.channel_direct_tcpip(host, puerto, None).map_err(|e| {
        Fallo::new(
            Codigo::Conexion,
            format!(
                "el bastión `{}` no abre el camino hasta `{host}:{puerto}` ({e}): ¿llega a la base, \
                 y permite el reenvío (`AllowTcpForwarding`)?",
                b.host
            ),
        )
    })?;
    Ok((s, c))
}

/// **Abre el túnel**: comprueba ya el bastión y el camino (el error sale aquí,
/// con su causa) y deja un reenvío local. Devuelve su puerto.
pub fn abrir(b: &Bastion, host: &str, puerto: u16) -> Result<u16, Fallo> {
    let primero = canal(b, host, puerto)?;
    let escucha = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| Fallo::origen(format!("no se pudo abrir el reenvío local: {e}")))?;
    let local = escucha
        .local_addr()
        .map_err(|e| Fallo::origen(e.to_string()))?
        .port();
    let (b, host) = (b.clone(), host.to_string());
    std::thread::spawn(move || {
        let mut primero = Some(primero);
        for tcp in escucha.incoming().flatten() {
            let par = match primero.take() {
                Some(p) => Ok(p),
                None => canal(&b, &host, puerto),
            };
            match par {
                Ok((s, c)) => {
                    std::thread::spawn(move || bombear(tcp, s, c));
                }
                Err(f) => eprintln!("ore-read-postgres: aviso · túnel: {}", f.mensaje),
            }
        }
    });
    Ok(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_tunel_sale_de_la_url() {
        let (u, b) = separar(
            "postgres://u:p@db.interna:6543/x?sslmode=verify-ca&ssh=ore@bastion.cliente.com:2222&ssh_huella=SHA256%3Aab%2Bc/d",
        )
        .unwrap();
        assert_eq!(u, "postgres://u:p@db.interna:6543/x?sslmode=verify-ca");
        let b = b.unwrap();
        assert_eq!(
            (
                b.usuario.as_str(),
                b.host.as_str(),
                b.puerto,
                b.huella.as_deref(),
                b.legado
            ),
            (
                "ore",
                "bastion.cliente.com",
                2222,
                Some("SHA256:ab+c/d"),
                false
            )
        );
        // `+` sin codificar es `+`: una huella en base64 lo lleva.
        let (_, b) = separar("postgres://h/db?ssh=ore@b&ssh_huella=SHA256:a+b").unwrap();
        assert_eq!(b.unwrap().huella.as_deref(), Some("SHA256:a+b"));
        assert_eq!(
            separar("postgres://h/db?ssh=ore@b")
                .unwrap()
                .1
                .unwrap()
                .puerto,
            22
        );
    }

    #[test]
    fn sin_tunel_la_url_no_cambia() {
        assert_eq!(
            separar("postgres://u@h/db?sslmode=require").unwrap(),
            ("postgres://u@h/db?sslmode=require".into(), None)
        );
        assert_eq!(separar("host=h dbname=x").unwrap().1, None);
    }

    #[test]
    fn un_tunel_mal_dicho_se_dice() {
        for u in [
            "postgres://h/db?ssh=bastion",
            "postgres://h/db?ssh=ore@b:xx",
            "postgres://h/db?ssh_huella=SHA256:x",
            "host=h ssh=ore@b",
        ] {
            assert!(separar(u).is_err(), "{u}");
        }
    }

    #[test]
    fn el_destino_y_la_url_con_el_puerto_del_reenvio() {
        assert_eq!(
            destino("postgres://u:p%40x@db.interna:6543/x?a=1", 40001).unwrap(),
            (
                "postgres://u:p%40x@db.interna:40001/x?a=1".into(),
                "db.interna".into(),
                6543
            )
        );
        assert_eq!(destino("postgres://db/x", 1).unwrap().2, 5432);
        assert!(destino("postgres://a,b/x", 1).is_err());
    }
}
