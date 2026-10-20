//! **La URL de conexión**: `mongodb://…` o `mongodb+srv://…`, con su credencial
//! dentro (va al cofre como `fuente-<n>`, no se canjea nada: ficha 0069 §4.6).
//!
//! Dos oficios, los dos puros:
//!
//! - [`validar`]: lo que **no** se admite. Una verificación de TLS que se puede
//!   apagar por URL se apaga el día que alguien tiene prisa, así que no se
//!   ofrece (`tlsInsecure`, `tlsAllowInvalidCertificates`,
//!   `tlsAllowInvalidHostnames`, y sus nombres viejos `ssl…`). Lo medido en X0:
//!   con la CA propia y el nombre en el certificado se conecta; sin la CA, o con
//!   otro nombre, la verificación falla.
//! - [`publica`]: la misma URL **sin el usuario ni la contraseña**, para
//!   mostrarla (`check`, `explorar`, los errores).

/// Opciones que apagan la verificación de TLS.
const INSEGURAS: &[&str] = &[
    "tlsinsecure",
    "tlsallowinvalidcertificates",
    "tlsallowinvalidhostnames",
    "sslallowinvalidcertificates",
    "sslallowinvalidhostnames",
];

/// `Ok` si la URL se puede usar; si no, por qué (sin repetir la credencial).
pub fn validar(url: &str) -> Result<(), String> {
    let resto = url
        .strip_prefix("mongodb+srv://")
        .or_else(|| url.strip_prefix("mongodb://"))
        .ok_or("la URL de MongoDB es `mongodb://…` o `mongodb+srv://…`")?;
    if resto.contains('#') {
        return Err("la URL de MongoDB no lleva `#`".into());
    }
    let (autoridad, consulta) = partir(resto);
    if hosts_de(autoridad).is_empty() {
        return Err("la URL de MongoDB no dice a qué servidor conectar".into());
    }
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (clave, valor) = par.split_once('=').unwrap_or((par, ""));
        let k = clave.to_ascii_lowercase();
        if INSEGURAS.contains(&k.as_str()) && !matches!(valor, "false" | "0") {
            return Err(format!(
                "`{clave}` apaga la verificación de TLS y no se admite: el servidor tiene que presentar un certificado que se verifique (Atlas lo hace; una CA propia, por fichero, todavía no se admite en un Job)"
            ));
        }
    }
    Ok(())
}

/// La URL sin `usuario:contraseña@`. Una URL que no se reconoce se devuelve
/// tapada entera: es mejor no enseñar que enseñar de más.
pub fn publica(url: &str) -> String {
    let Some((esquema, resto)) = url.split_once("://") else {
        return "<url de MongoDB no reconocida>".into();
    };
    let (autoridad_y_ruta, consulta) = match resto.split_once('?') {
        Some((a, c)) => (a, Some(c)),
        None => (resto, None),
    };
    let (autoridad, ruta) = match autoridad_y_ruta.find('/') {
        Some(i) => autoridad_y_ruta.split_at(i),
        None => (autoridad_y_ruta, ""),
    };
    let hosts = autoridad.rsplit_once('@').map_or(autoridad, |(_, h)| h);
    let mut s = format!("{esquema}://{hosts}{ruta}");
    if let Some(c) = consulta {
        // Las opciones no llevan secretos salvo `authMechanismProperties` (un
        // token de OIDC) y `tlsCertificateKeyFilePassword`: esas se tapan.
        let visibles: Vec<String> = c
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, _) = p.split_once('=').unwrap_or((p, ""));
                let kl = k.to_ascii_lowercase();
                if kl.contains("password") || kl.contains("properties") || kl.contains("secret") {
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

/// El nombre de la base que la URL trae en su ruta, si lo trae.
pub fn base_de(url: &str) -> Option<String> {
    let resto = url.split_once("://")?.1;
    let (sin_consulta, _) = partir(resto);
    let (_, ruta) = sin_consulta.split_once('/')?;
    (!ruta.is_empty()).then(|| ruta.to_string())
}

/// `(lo anterior a la ruta y a la consulta, la consulta)`. La ruta se queda en
/// el primero para [`base_de`].
fn partir(resto: &str) -> (&str, &str) {
    let (antes, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    (antes, consulta)
}

/// Los `host[:puerto]` de la autoridad, sin credencial ni ruta.
fn hosts_de(antes_de_consulta: &str) -> Vec<&str> {
    let autoridad = antes_de_consulta
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit_once('@')
        .map_or_else(
            || antes_de_consulta.split('/').next().unwrap_or(""),
            |(_, h)| h,
        );
    autoridad.split(',').filter(|h| !h.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admite_las_dos_formas() {
        assert_eq!(validar("mongodb+srv://u:p@cluster0.x.mongodb.net/"), Ok(()));
        assert_eq!(
            validar(
                "mongodb://u:p@a:27017,b:27017/app?replicaSet=rs0&tls=true&tlsCAFile=/c/ca.pem"
            ),
            Ok(())
        );
    }

    #[test]
    fn no_admite_apagar_la_verificacion_de_tls() {
        for o in [
            "tlsInsecure=true",
            "tlsAllowInvalidCertificates=true",
            "tlsAllowInvalidHostnames=1",
            "sslAllowInvalidCertificates=true",
        ] {
            let e = validar(&format!("mongodb://u:p@a/?tls=true&{o}")).unwrap_err();
            assert!(e.contains("no se admite"), "{o}: {e}");
        }
        // Dicho en falso no apaga nada.
        assert_eq!(validar("mongodb://u:p@a/?tlsInsecure=false"), Ok(()));
    }

    #[test]
    fn rechaza_lo_que_no_es_mongodb() {
        assert!(validar("postgres://u:p@a/db").is_err());
        assert!(validar("mongodb://").is_err());
        assert!(validar("mongodb://u:p@/db").is_err());
    }

    /// La credencial no sale en lo que se enseña, ni la contraseña de una
    /// clave de cliente.
    #[test]
    fn la_url_publica_no_lleva_la_credencial() {
        let u = "mongodb+srv://victor:s3creta@cluster0.x.mongodb.net/ore_prueba?tls=true&tlsCertificateKeyFilePassword=otra&appName=ore";
        let p = publica(u);
        assert_eq!(
            p,
            "mongodb+srv://cluster0.x.mongodb.net/ore_prueba?tls=true&tlsCertificateKeyFilePassword=***&appName=ore"
        );
        assert!(
            !p.contains("s3creta") && !p.contains("victor") && !p.contains("otra"),
            "{p}"
        );
        assert_eq!(publica("lo que sea"), "<url de MongoDB no reconocida>");
    }

    #[test]
    fn la_base_sale_de_la_ruta() {
        assert_eq!(
            base_de("mongodb+srv://u:p@h/ore_prueba?x=1").as_deref(),
            Some("ore_prueba")
        );
        assert_eq!(base_de("mongodb://u:p@h/").as_deref(), None);
        assert_eq!(base_de("mongodb://u:p@h").as_deref(), None);
    }
}
