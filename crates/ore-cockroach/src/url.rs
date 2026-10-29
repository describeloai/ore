//! **La URL de CockroachDB** (ADR 0080, paso 1): la de PostgreSQL, porque es la que CockroachDB y sus guías ya dan.
//!
//! `cockroachdb://usuario[:clave]@host[:26257][/base]?sslmode=…&sslrootcert=…&timeoutMs=…` (también `postgresql://` y `postgres://`).
//! Aquí sólo se quita lo que no es del cliente de PostgreSQL —el esquema propio y `timeoutMs`—; `sslmode` y `sslrootcert` los lee
//! [`crate::tls`], y el resto (`options`, `application_name`…) llega tal cual al cliente.

use ore_driver::{Codigo, Fallo};

/// Una URL ya lista para el cliente de PostgreSQL.
#[derive(Debug, PartialEq, Eq)]
pub struct Destino {
    /// La URL con esquema `postgresql://` y sin `timeoutMs`.
    pub url: String,
    pub timeout_ms: Option<u64>,
}

const ESQUEMAS: [&str; 3] = ["cockroachdb://", "postgresql://", "postgres://"];

pub fn analizar(url: &str) -> Result<Destino, Fallo> {
    let url = url.trim();
    let Some(esquema) = ESQUEMAS.iter().find(|e| url.starts_with(**e)) else {
        return Err(Fallo::new(
            Codigo::Conexion,
            "la URL de CockroachDB empieza por `cockroachdb://` (o `postgresql://`)",
        ));
    };
    let resto = &url[esquema.len()..];
    let (cuerpo, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    if cuerpo
        .split('/')
        .next()
        .is_none_or(|h| h.is_empty() || h == "@")
    {
        return Err(Fallo::new(
            Codigo::Conexion,
            "la URL de CockroachDB no nombra el servidor: `cockroachdb://usuario@host:26257/base`",
        ));
    }
    let mut timeout_ms = None;
    let mut quedan = Vec::new();
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        match par.split_once('=') {
            Some(("timeoutMs", v)) => {
                timeout_ms = Some(v.parse::<u64>().map_err(|_| {
                    Fallo::new(
                        Codigo::Conexion,
                        format!("`timeoutMs={v}` no es un número de milisegundos"),
                    )
                })?);
            }
            _ => quedan.push(par),
        }
    }
    let mut url = format!("postgresql://{cuerpo}");
    if !quedan.is_empty() {
        url.push('?');
        url.push_str(&quedan.join("&"));
    }
    Ok(Destino { url, timeout_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_esquema_propio_se_vuelve_el_de_postgres() {
        let d = analizar("cockroachdb://kit@localhost:26257/kit").unwrap();
        assert_eq!(d.url, "postgresql://kit@localhost:26257/kit");
        assert_eq!(d.timeout_ms, None);
    }

    #[test]
    fn timeout_ms_sale_y_el_resto_se_queda() {
        let d = analizar(
            "cockroachdb://u:p@h/db?sslmode=verify-full&timeoutMs=5000&options=--cluster%3Dx",
        )
        .unwrap();
        assert_eq!(
            d.url,
            "postgresql://u:p@h/db?sslmode=verify-full&options=--cluster%3Dx"
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
        ] {
            let f = analizar(u).unwrap_err();
            assert_eq!(f.codigo, Codigo::Conexion, "{u}");
        }
    }

    #[test]
    fn timeout_que_no_es_numero_se_niega() {
        let f = analizar("cockroachdb://u@h/db?timeoutMs=mucho").unwrap_err();
        assert!(f.mensaje.contains("timeoutMs"), "{}", f.mensaje);
    }
}
