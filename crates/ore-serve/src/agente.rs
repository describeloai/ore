//! **`ore-serve` como el agente de su celda** (0046 E9·3): lo que hace falta para
//! servir un ítem de una colección **virtual**, cuyos bytes están en el origen.
//!
//! Para firmar una URL del origen hace falta la credencial de la fuente, y esa
//! vive en el cofre, que la entrega a quien tenga `usar` sobre `fuente-<n>` y lo
//! anota. Quien la tiene en la celda es **el agente** —el cliente de Keycloak del
//! inquilino que el aprovisionador crea (⑦) y que los Jobs de la copia ya usan—.
//! Es el patrón de la conexión autorizadora de BigQuery (ObjectRef): el usuario
//! tiene derecho sobre el objeto; el acceso al almacén, la celda.
//!
//! - el cliente y su secreto, de dos ficheros que el contenedor de inicio trae
//!   del almacén a un tmpfs (`--agente-fichero /testigo/agente` →
//!   `/testigo/agente-cliente` y `-secreto`): nunca por el entorno ni por etcd;
//! - el token, del IdP **del clúster** por HTTP llano (`--idp`), el mismo camino
//!   que los Jobs: este proceso sigue sin cliente TLS;
//! - y guardado hasta 30 s antes de caducar: una galería no pide cien tokens.

use ore_core::json::Json;
use ore_entrada::http::{self, Plazos};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Agente {
    /// `host:puerto` del IdP dentro del clúster.
    idp: String,
    /// `/realms/<realm>`, del `--emisor`.
    realm: String,
    cliente: PathBuf,
    secreto: PathBuf,
    guardado: Mutex<Option<(String, Instant)>>,
}

/// Lo que se le quita a la vida de un token: pedir otro antes de que caduque.
const MARGEN: Duration = Duration::from_secs(30);

impl Agente {
    /// `prefijo` es `/testigo/agente`: se leen `<prefijo>-cliente` y `-secreto`.
    /// `emisor` es el `--emisor` (`https://…/realms/<realm>`): de él sale el realm.
    pub fn de(idp: &str, emisor: &str, prefijo: &str) -> Result<Agente, String> {
        let realm = emisor
            .find("/realms/")
            .map(|i| emisor[i..].trim_end_matches('/').to_string())
            .ok_or_else(|| format!("el emisor `{emisor}` no dice su realm (`…/realms/<realm>`)"))?;
        Ok(Agente {
            idp: idp.to_string(),
            realm,
            cliente: PathBuf::from(format!("{prefijo}-cliente")),
            secreto: PathBuf::from(format!("{prefijo}-secreto")),
            guardado: Mutex::new(None),
        })
    }

    /// Un token del agente que vale al menos [`MARGEN`].
    pub fn token(&self) -> Result<String, String> {
        if let Ok(g) = self.guardado.lock()
            && let Some((t, vale)) = g.as_ref()
            && Instant::now() < *vale
        {
            return Ok(t.clone());
        }
        let leer = |p: &PathBuf| {
            std::fs::read_to_string(p)
                .map(|s| s.trim().to_string())
                .map_err(|e| format!("no se pudo leer `{}`: {e}", p.display()))
        };
        let (cliente, secreto) = (leer(&self.cliente)?, leer(&self.secreto)?);
        let (codigo, cuerpo) = http::pedir_formulario(
            &self.idp,
            &format!("{}/protocol/openid-connect/token", self.realm),
            &[
                ("grant_type", "client_credentials"),
                ("client_id", &cliente),
                ("client_secret", &secreto),
            ],
            Plazos {
                conectar: Duration::from_secs(5),
                responder: Duration::from_secs(10),
            },
        )
        .map_err(|e| format!("el IdP no contesta al token del agente: {e}"))?;
        if codigo != 200 {
            return Err(format!("el IdP contestó {codigo} al token del agente"));
        }
        let n =
            ore_core::parse::parse(&cuerpo).map_err(|_| "el IdP no devolvió JSON".to_string())?;
        let token = n
            .get("access_token")
            .and_then(|(_, v)| v.as_str())
            .ok_or("el IdP no devolvió `access_token`")?
            .to_string();
        let vida = n
            .get("expires_in")
            .and_then(|(_, v)| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        let vale = Instant::now() + Duration::from_secs(vida).saturating_sub(MARGEN);
        if let Ok(mut g) = self.guardado.lock() {
            *g = Some((token.clone(), vale));
        }
        Ok(token)
    }
}

/// El valor de un secreto del cofre (`{valor}`), o `None`.
pub fn valor_de(cuerpo: &str) -> Option<String> {
    let n = ore_core::parse::parse(cuerpo).ok()?;
    match Json::de_node(n.get("valor")?.1) {
        Json::Str(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// ¿Puede `env` llevar la credencial de una fuente al proceso hijo? El nombre
/// sale del árbol, que escriben personas: sólo `<ALGO>_URL` —como las declara
/// `ore source add`— y nunca lo que cambia cómo arranca un proceso.
pub fn variable_admisible(env: &str) -> bool {
    env.len() <= 64
        && env.ends_with("_URL")
        && env.starts_with(|c: char| c.is_ascii_uppercase())
        && env
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !env.starts_with("LD_")
        && !env.starts_with("DYLD_")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_realm_sale_del_emisor() {
        let a = Agente::de(
            "idp:8080",
            "https://login.paladio.io/realms/rubix",
            "/t/agente",
        )
        .unwrap();
        assert_eq!(a.realm, "/realms/rubix");
        assert_eq!(a.cliente, PathBuf::from("/t/agente-cliente"));
        assert!(Agente::de("idp:8080", "https://login.paladio.io", "/t/a").is_err());
    }

    #[test]
    fn solo_una_url_entra_en_el_entorno_del_hijo() {
        for bien in ["ORE_S3_URL", "VENTAS_URL", "S3_STANDARD_URL"] {
            assert!(variable_admisible(bien), "{bien}");
        }
        for mal in [
            "LD_PRELOAD",
            "LD_X_URL",
            "PATH",
            "ore_s3_url",
            "X-URL",
            "_URL",
            "HOME_URL\n",
        ] {
            assert!(!variable_admisible(mal), "{mal}");
        }
    }

    #[test]
    fn el_valor_del_cofre() {
        assert_eq!(
            valor_de(r#"{"valor":"s3://b?x=1"}"#).as_deref(),
            Some("s3://b?x=1")
        );
        assert_eq!(valor_de(r#"{"valor":""}"#), None);
        assert_eq!(valor_de("no"), None);
    }
}
