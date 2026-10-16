//! **Entrar sin contraseña: el IAM de Google** (ADR 0065 PG5, D-PG2).
//!
//! Cloud SQL y AlloyDB aceptan, en vez de una contraseña, el *access token*
//! OAuth de una cuenta de Google: el servidor lo valida contra IAM. La celda ya
//! tiene la suya (`ore-driver-<celda>`, por Workload Identity), la misma con la
//! que lee BigQuery y GCS: así no queda ningún secreto del cliente en el cofre.
//!
//! ```text
//! postgres://<cuenta sin .gserviceaccount.com>@<host>/<base>?iam=gcp[&suplanta=<cuenta del cliente>]
//! ```
//!
//! - El usuario es la cuenta como la nombra Cloud SQL: `ore-driver-demo@proyecto.iam`
//!   (en la URL, la `@` va como `%40`).
//! - Con `suplanta`, el token es el de una cuenta del cliente, que deja a la de
//!   la celda suplantarla (`roles/iam.serviceAccountTokenCreator`): permisos por
//!   conexión, como en GCS (D-O2). El usuario es entonces esa cuenta.
//! - **El token se pide en cada conexión** y no se guarda: vale una hora para
//!   conectar, y la sesión sigue después.
//! - **Siempre con TLS** (D-PG1 lo da).

use ore_driver::{Codigo, Fallo};
use std::sync::OnceLock;

/// Lo que pide la URL.
#[derive(Debug, Clone, PartialEq)]
pub enum Iam {
    /// La cuenta de la celda.
    Celda,
    /// Una cuenta del cliente, suplantada.
    Suplanta(String),
}

/// Lo que basta para entrar en Cloud SQL y AlloyDB.
const ALCANCES: &[&str] = &[
    "https://www.googleapis.com/auth/sqlservice.login",
    "https://www.googleapis.com/auth/cloud-platform",
];

/// **Separa `iam` y `suplanta` de la URL.** Sin `iam=gcp`, `suplanta` es un
/// error: no se adivina.
pub fn separar(url: &str) -> Result<(String, Option<Iam>), Fallo> {
    let Some((base, consulta)) = url.split_once('?').filter(|_| url.contains("://")) else {
        return Ok((url.to_string(), None));
    };
    let (mut iam, mut suplanta) = (None, None);
    let mut quedan = Vec::new();
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        match k {
            "iam" => iam = Some(v.to_string()),
            "suplanta" => suplanta = Some(v.replace("%40", "@")),
            _ => quedan.push(par),
        }
    }
    let limpia = if quedan.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", quedan.join("&"))
    };
    let mal = |m: String| Err(Fallo::new(Codigo::Credencial, m));
    match (iam.as_deref(), suplanta) {
        (None, None) => Ok((limpia, None)),
        (None, Some(_)) => mal("`suplanta` sin `iam=gcp`".into()),
        (Some("gcp"), None) => Ok((limpia, Some(Iam::Celda))),
        (Some("gcp"), Some(c)) if c.contains('@') => Ok((limpia, Some(Iam::Suplanta(c)))),
        (Some("gcp"), Some(c)) => mal(format!("`suplanta={c}` no es una cuenta de servicio")),
        (Some(otro), _) => mal(format!(
            "`iam={otro}`: por ahora sólo `gcp` (Cloud SQL y AlloyDB); AWS y Azure son PG6"
        )),
    }
}

/// La credencial de la celda, una por proceso (el token se renueva solo).
fn celda() -> &'static ore_gcp::Credencial {
    static C: OnceLock<ore_gcp::Credencial> = OnceLock::new();
    C.get_or_init(ore_gcp::Credencial::del_entorno)
}

/// **El token que va de contraseña.**
pub fn token(iam: &Iam) -> Result<String, Fallo> {
    let r = match iam {
        Iam::Celda => celda().token(),
        Iam::Suplanta(cuenta) => ore_gcp::suplantar(celda(), cuenta, ALCANCES),
    };
    r.map_err(|e| {
        Fallo::new(
            Codigo::Credencial,
            format!("el token de Google para entrar: {e}"),
        )
    })
}

/// Lo que se dice cuando Postgres no acepta el token: casi siempre es una de
/// tres cosas, y las tres se arreglan en el lado del cliente.
pub fn pista() -> &'static str {
    " — con `iam=gcp`: ¿la instancia tiene `cloudsql.iam_authentication=on` (o \
     `alloydb.iam_authentication`), el usuario IAM existe con el nombre de la URL (la cuenta sin \
     `.gserviceaccount.com`) y la cuenta tiene `roles/cloudsql.instanceUser`?"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iam_sale_de_la_url() {
        assert_eq!(
            separar("postgres://ore-driver-demo%40p.iam@h/db?iam=gcp&sslmode=verify-ca").unwrap(),
            (
                "postgres://ore-driver-demo%40p.iam@h/db?sslmode=verify-ca".into(),
                Some(Iam::Celda)
            )
        );
        assert_eq!(
            separar("postgres://u@h/db?iam=gcp&suplanta=lector%40cliente.iam.gserviceaccount.com")
                .unwrap()
                .1,
            Some(Iam::Suplanta(
                "lector@cliente.iam.gserviceaccount.com".into()
            ))
        );
        assert_eq!(separar("postgres://u:p@h/db").unwrap().1, None);
    }

    #[test]
    fn lo_que_no_es_se_dice() {
        for u in [
            "postgres://u@h/db?suplanta=a%40b",
            "postgres://u@h/db?iam=aws",
            "postgres://u@h/db?iam=gcp&suplanta=nadie",
        ] {
            assert!(separar(u).is_err(), "{u}");
        }
    }
}
