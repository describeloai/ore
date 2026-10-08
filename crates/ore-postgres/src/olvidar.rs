//! **Lo que el proxy tiene que olvidar** (0058 P5·1).
//!
//! El proxy de Neon guarda lo que le contestó [`crate::proxy`] —el verificador
//! de cada rol, cuatro minutos (`--project-info-cache`, `ttl=4m`)—. Medido en
//! el laboratorio: sin avisarle, tras un *Reset password* la contraseña nueva
//! no entra hasta que caduca la caché (231 s y 302 s).
//!
//! Neon lo resuelve avisando al proxy por Redis, y aquí se hace igual: el
//! proxy se suscribe a `neondb-proxy-ws-updates` (`--redis-plain`) y esto
//! publica allí lo que cambió, con el formato del commit fijado
//! (`proxy/src/redis/notifications.rs`):
//!
//! ```text
//!   PUBLISH neondb-proxy-ws-updates
//!     {"topic":"/project_settings_update","data":"{\"project_id\":\"<tenant>\"}"}
//! ```
//!
//! ⭐ **Se avisa cuando el cambio ya está en el cómputo**, no cuando se pide:
//!   cuando el reconciliador da por `hecha` una operación del proyecto
//!   (`configurar-rama` aplica roles y contraseñas con `/configure`). Si se
//!   avisara antes, el proxy volvería a leer el verificador nuevo y el cómputo
//!   aún tendría el viejo. El proxy, además, repite el olvido a los 20 s.
//!
//! ⭐ **Por proyecto entero, con el tenant**: es lo que [`crate::proxy`]
//!   contesta como `project_id`, único entre organizaciones (el id del
//!   proyecto no lo es). Olvidar de más sólo cuesta una pregunta.
//!
//! ⛔ Sin Redis no se para nada: lo que no se avise caduca solo en 4 minutos.
//!   Si Redis no contesta, la marca no avanza y se reintenta en la vuelta
//!   siguiente.

use crate::base::{conectar, mal};
use ore_core::json::Json;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// El canal al que se suscribe el proxy de Neon.
pub const CANAL: &str = "neondb-proxy-ws-updates";

/// Cada cuánto se mira qué se ha hecho.
const CADA: Duration = Duration::from_secs(1);

/// El mensaje que hace olvidar un proyecto entero.
pub fn mensaje(tenant: &str) -> String {
    let datos = Json::obj([("project_id", Json::s(tenant))]).jcs();
    Json::obj([
        ("topic", Json::s("/project_settings_update")),
        ("data", Json::s(datos)),
    ])
    .jcs()
}

/// Una orden en el protocolo de Redis (RESP): un array de cadenas.
pub fn orden(partes: &[&str]) -> Vec<u8> {
    let mut o = format!("*{}\r\n", partes.len()).into_bytes();
    for p in partes {
        o.extend(format!("${}\r\n", p.len()).into_bytes());
        o.extend(p.as_bytes());
        o.extend(b"\r\n");
    }
    o
}

/// `PUBLISH`: cuántos suscriptores lo recibieron.
pub fn publicar(destino: &str, canal: &str, mensaje: &str) -> Result<i64, String> {
    let plazo = Duration::from_secs(2);
    let dir = destino
        .to_socket_addrs()
        .map_err(|e| format!("redis `{destino}`: {e}"))?
        .next()
        .ok_or_else(|| format!("redis `{destino}`: sin dirección"))?;
    let mut s = TcpStream::connect_timeout(&dir, plazo).map_err(|e| format!("redis: {e}"))?;
    s.set_read_timeout(Some(plazo)).ok();
    s.set_write_timeout(Some(plazo)).ok();
    s.write_all(&orden(&["PUBLISH", canal, mensaje]))
        .map_err(|e| format!("redis: {e}"))?;
    let mut linea = String::new();
    BufReader::new(s)
        .read_line(&mut linea)
        .map_err(|e| format!("redis: {e}"))?;
    match linea.trim_end().split_at_checked(1) {
        Some((":", n)) => n.parse().map_err(|_| format!("redis contestó `{linea}`")),
        _ => Err(format!("redis contestó `{}`", linea.trim_end())),
    }
}

/// En un hilo, para siempre: lo hecho desde que arrancó, al proxy.
pub fn arrancar(url: String, redis: String) {
    std::thread::spawn(move || {
        let mut base: Option<postgres::Client> = None;
        let mut marca: Option<String> = None;
        let mut fallando = false;
        loop {
            if base.as_ref().is_none_or(|c| c.is_closed()) {
                base = conectar(&url).ok();
            }
            let r = match base.as_mut() {
                Some(c) => vuelta(c, &redis, &mut marca),
                None => Err("sin su base".to_string()),
            };
            match r {
                Ok(n) if fallando || n > 0 => {
                    if fallando {
                        eprintln!("olvidar · redis vuelve a contestar");
                    }
                    fallando = false;
                }
                Ok(_) => {}
                Err(e) if !fallando => {
                    eprintln!("olvidar · {e} (se reintenta; la caché del proxy caduca sola)");
                    fallando = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(CADA);
        }
    });
}

/// Una vuelta: los tenants con operaciones hechas después de la marca.
fn vuelta(
    c: &mut postgres::Client,
    redis: &str,
    marca: &mut Option<String>,
) -> Result<usize, String> {
    let desde = match marca {
        Some(m) => m.clone(),
        None => {
            let ahora: String = c.query_one("select now()::text", &[]).map_err(mal)?.get(0);
            *marca = Some(ahora.clone());
            ahora
        }
    };
    let filas = c
        .query(
            "select max(o.terminada)::text, p.tenant
               from plano.operacion o
               join plano.proyecto p on p.organizacion = o.organizacion and p.id = o.proyecto
              where o.estado = 'hecha' and o.terminada > $1::text::timestamptz
                and p.tenant is not null
              group by p.tenant",
            &[&desde],
        )
        .map_err(mal)?;
    let mut nueva = desde;
    for f in &filas {
        let (hasta, tenant): (String, String) = (f.get(0), f.get(1));
        publicar(redis, CANAL, &mensaje(&tenant))?;
        if hasta > nueva {
            nueva = hasta;
        }
    }
    *marca = Some(nueva);
    Ok(filas.len())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_mensaje_tiene_la_forma_de_neon() {
        assert_eq!(
            mensaje("0123456789abcdef0123456789abcdef"),
            r#"{"data":"{\"project_id\":\"0123456789abcdef0123456789abcdef\"}","topic":"/project_settings_update"}"#
        );
    }

    #[test]
    fn publicar_habla_resp_y_lee_cuantos_lo_oyeron() {
        use std::io::Read;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dir = l.local_addr().unwrap().to_string();
        let oido = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut b = vec![0u8; 256];
            let n = s.read(&mut b).unwrap();
            s.write_all(b":2\r\n").unwrap();
            b.truncate(n);
            b
        });
        assert_eq!(publicar(&dir, "c", "m"), Ok(2));
        assert_eq!(oido.join().unwrap(), orden(&["PUBLISH", "c", "m"]));
        // Un error de Redis es un error, no un «nadie lo oyó».
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dir = l.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let _ = s.read(&mut [0u8; 256]);
            s.write_all(b"-NOAUTH Authentication required.\r\n")
                .unwrap();
        });
        assert!(publicar(&dir, "c", "m").unwrap_err().contains("NOAUTH"));
    }

    #[test]
    fn la_orden_en_resp() {
        assert_eq!(
            orden(&["PUBLISH", "c", "hola"]),
            b"*3\r\n$7\r\nPUBLISH\r\n$1\r\nc\r\n$4\r\nhola\r\n"
        );
    }
}
