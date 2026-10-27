//! **Las piezas de una URL de conexión que no son de ningún origen**: descodificar un `%40`, partir `host:puerto`
//! (con IPv6 entre corchetes), leer un `true`/`false` y enseñar la URL sin su credencial.
//!
//! Cada origen escribe su `url.rs` —el esquema, sus opciones y lo que no admite—, pero estas cuatro cosas las
//! escribía **cada uno por su cuenta**, y eran la misma: `puerto` y `booleano` idénticas byte a byte en Phoenix,
//! Cassandra y Oracle, `host_y_puerto` y `publica` iguales salvo el puerto por defecto y el nombre. Lo que es
//! específico (qué opciones tiene `oracle://`) sigue en su crate; lo que no, está aquí una vez.

/// `%40` es una arroba que no parte la URL: la contraseña viene codificada.
pub fn decodificar(s: &str) -> Result<String, String> {
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

/// Un puerto de red (1–65535).
pub fn puerto(p: &str) -> Result<u16, String> {
    p.parse::<u16>()
        .ok()
        .filter(|n| *n != 0)
        .ok_or_else(|| format!("`{p}` no es un puerto"))
}

/// `true`/`1` o `false`/`0`, con el nombre de la opción en el error.
pub fn booleano(clave: &str, valor: &str) -> Result<bool, String> {
    match valor {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        otro => Err(format!("`{clave}={otro}`: es `true` o `false`")),
    }
}

/// `host`, `host:puerto` o `[::1]:puerto`; sin puerto, el `por_defecto` del origen.
pub fn host_y_puerto(h: &str, por_defecto: u16) -> Result<(String, u16), String> {
    // Una dirección IPv6 va entre corchetes: `[::1]:1521`.
    if let Some(r) = h.strip_prefix('[') {
        let (dir, resto) = r
            .split_once(']')
            .ok_or("una dirección IPv6 sin `]`".to_string())?;
        let p = match resto.strip_prefix(':') {
            Some(p) => puerto(p)?,
            None if resto.is_empty() => por_defecto,
            None => return Err(format!("`{h}` no es un `host:puerto`")),
        };
        return Ok((dir.to_string(), p));
    }
    match h.rsplit_once(':') {
        Some((host, p)) if !host.is_empty() => Ok((host.to_string(), puerto(p)?)),
        Some(_) => Err(format!("`{h}` no es un `host:puerto`")),
        None => Ok((h.to_string(), por_defecto)),
    }
}

/// La URL sin `usuario:contraseña@`, para enseñarla. Una opción que parece un secreto (`password`, `secret`,
/// `token`) sale como `***`. Una URL que no se reconoce se devuelve tapada entera, con el `nombre` del origen.
pub fn publica(url: &str, nombre: &str) -> String {
    let Some((esquema, resto)) = url.split_once("://") else {
        return format!("<url de {nombre} no reconocida>");
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

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn un_porcentaje_no_parte_la_url() {
        assert_eq!(decodificar("p%40ss%2Fw").unwrap(), "p@ss/w");
        assert_eq!(decodificar("ñandú").unwrap(), "ñandú");
        assert_eq!(decodificar("%C3%B1").unwrap(), "ñ");
        for malo in ["%", "%4", "%zz", "%C3"] {
            assert!(decodificar(malo).is_err(), "{malo}");
        }
    }

    #[test]
    fn host_con_y_sin_puerto_e_ipv6() {
        assert_eq!(host_y_puerto("db", 1521).unwrap(), ("db".into(), 1521));
        assert_eq!(host_y_puerto("db:1522", 1521).unwrap(), ("db".into(), 1522));
        assert_eq!(host_y_puerto("[::1]", 9042).unwrap(), ("::1".into(), 9042));
        assert_eq!(
            host_y_puerto("[fe80::1]:8765", 1).unwrap(),
            ("fe80::1".into(), 8765)
        );
        for malo in ["db:0", "db:99999", "db:x", ":1521", "[::1", "[::1]x"] {
            assert!(host_y_puerto(malo, 1521).is_err(), "{malo}");
        }
    }

    #[test]
    fn el_booleano_dice_la_opcion() {
        assert_eq!(booleano("tls", "true"), Ok(true));
        assert_eq!(booleano("tls", "0"), Ok(false));
        assert!(
            booleano("tls", "quizas")
                .unwrap_err()
                .contains("tls=quizas")
        );
    }

    #[test]
    fn la_url_publica_no_lleva_credencial_ni_secretos() {
        assert_eq!(
            publica(
                "oracle://ore:secreto@h:1521/S?tls=true&password=otro",
                "Oracle"
            ),
            "oracle://h:1521/S?tls=true&password=***"
        );
        assert_eq!(publica("h:1/x", "Oracle"), "<url de Oracle no reconocida>");
        assert!(!publica("phoenix://u:clave@h/E", "Phoenix").contains("clave"));
    }
}
