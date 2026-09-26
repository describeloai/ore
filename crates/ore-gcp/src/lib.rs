//! **El token de la cuenta que corre, y cuándo deja de valer.**
//!
//! Un proceso en GCP no tiene clave (la política de la organización prohíbe
//! crearlas, medido el 2026-09-16): tiene un token de acceso de corta vida que
//! le da el metadata server con la identidad del pod (Workload Identity). Ese
//! token **caduca** —el metadata server dice cuándo en `expires_in`— y hasta
//! A1 se guardaba para siempre (`ore-store-gcs`): una copia de más de una hora
//! fallaba a medias con un 401.
//!
//! # Las dos fuentes, y lo que cada una promete
//!
//! | fuente | dónde | renueva |
//! |---|---|---|
//! | el metadata server | en GCP (GKE con Workload Identity, una VM) | **sí**: se pide otro [`MARGEN`] antes de que caduque |
//! | `ORE_GCP_TOKEN` (o `ORE_GCS_TOKEN`, el nombre de antes) | en local, `$(gcloud auth print-access-token)` | **no**: vale lo que le quede |
//!
//! La segunda no es una hora: `gcloud` devuelve el token que tiene en caché con
//! la vida que le quede (34 min medidos el 2026-09-26). Por eso [`Credencial::renovable`]
//! existe: quien vaya a trabajar mucho rato puede decirlo antes de empezar.
//!
//! # Lo que este módulo no hace
//!
//! No acota el token: eso es de quien sabe a qué recurso lo presta
//! (`ore-store-gcs` con STS). Y acotar **no sirve para BigQuery**, que ignora la
//! Credential Access Boundary (medido el 2026-09-26): la frontera de un driver de
//! BigQuery es el IAM de su cuenta de servicio. Tampoco reintenta: un metadata
//! server que no contesta es un error que hay que ver.
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// El metadata server de GCP. Solo contesta dentro de GCP, por HTTP y sin TLS:
/// es una dirección local del nodo.
pub const METADATA: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

/// Cuánto antes de caducar se pide otro. Cinco minutos cubren la petición más
/// larga que se hace con un token ya obtenido (una página, una subida) sin
/// gastar más de una petición al metadata server por hora.
pub const MARGEN: Duration = Duration::from_secs(300);

/// Las variables de entorno de un token fijo, en orden. `ORE_GCS_TOKEN` es el
/// nombre que tenía cuando solo lo usaba `ore-store-gcs`, y el SDK del puesto lo
/// sigue poniendo.
pub const VARIABLES: &[&str] = &["ORE_GCP_TOKEN", "ORE_GCS_TOKEN"];

enum Fuente {
    Fijo(String),
    Metadata { url: String, margen: Duration },
}

struct Vigente {
    token: String,
    caduca: Instant,
}

/// El token de la cuenta que corre. Se comparte entre hilos: pedirlo es barato
/// cuando está vigente y serializado cuando hay que renovarlo.
pub struct Credencial {
    fuente: Fuente,
    vigente: Mutex<Option<Vigente>>,
}

impl Credencial {
    /// La del entorno: un token fijo si alguna de [`VARIABLES`] trae uno; si no,
    /// el metadata server. No llama a nadie todavía: el primer token se pide
    /// con la primera petición.
    pub fn del_entorno() -> Credencial {
        match VARIABLES
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|t| !t.is_empty()))
        {
            Some(t) => Credencial::fija(t),
            None => Credencial::del_metadata(METADATA, MARGEN),
        }
    }

    pub fn fija(token: impl Into<String>) -> Credencial {
        Credencial {
            fuente: Fuente::Fijo(token.into()),
            vigente: Mutex::new(None),
        }
    }

    /// Contra otro metadata server y con otro margen: para las pruebas, que no
    /// pueden esperar una hora a que algo caduque.
    pub fn del_metadata(url: impl Into<String>, margen: Duration) -> Credencial {
        Credencial {
            fuente: Fuente::Metadata {
                url: url.into(),
                margen,
            },
            vigente: Mutex::new(None),
        }
    }

    /// Si esta credencial sabe pedir otro token cuando el suyo caduque.
    pub fn renovable(&self) -> bool {
        matches!(self.fuente, Fuente::Metadata { .. })
    }

    /// Un token que vale al menos el margen. Pide uno nuevo al metadata server
    /// si no hay o si caduca antes.
    pub fn token(&self) -> Result<String, String> {
        let (url, margen) = match &self.fuente {
            Fuente::Fijo(t) => return Ok(t.clone()),
            Fuente::Metadata { url, margen } => (url, *margen),
        };
        let mut vigente = self.vigente.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = vigente.as_ref()
            && Instant::now() + margen < v.caduca
        {
            return Ok(v.token.clone());
        }
        let (token, vida) = pedir(url)?;
        *vigente = Some(Vigente {
            token: token.clone(),
            caduca: Instant::now() + vida,
        });
        Ok(token)
    }
}

/// Un token nuevo y cuánto vale, del metadata server.
fn pedir(url: &str) -> Result<(String, Duration), String> {
    let cuerpo = ureq::get(url)
        .set("metadata-flavor", "Google")
        .timeout(Duration::from_secs(5))
        .call()
        .map_err(|e| {
            format!(
                "sin `ORE_GCP_TOKEN` y el metadata server no contesta ({e}): en GCP hace falta \
                 Workload Identity; en local, `ORE_GCP_TOKEN=$(gcloud auth print-access-token)`"
            )
        })?
        .into_string()
        .map_err(|e| format!("el token del metadata server no se pudo leer: {e}"))?;
    let n = ore_core::parse::parse(&cuerpo)
        .map_err(|_| "el metadata server no devolvió JSON".to_string())?;
    let token = n
        .get("access_token")
        .and_then(|(_, v)| v.as_str().map(String::from))
        .ok_or("el metadata server no devolvió `access_token`")?;
    // Sin `expires_in` no se sabe cuánto vale: se trata como caducado, y la
    // próxima petición pide otro. Es más barato que adivinar una hora.
    let vida = n
        .get("expires_in")
        .and_then(|(_, v)| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .map_or(Duration::ZERO, Duration::from_secs);
    Ok((token, vida))
}

/// Un cliente HTTPS con el TLS de la plataforma.
pub fn cliente() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new()
        .map_err(|e| format!("no se pudo abrir el TLS de la plataforma: {e}"))?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Un metadata server de mentira: cada petición recibe un token nuevo
    /// (`t1`, `t2`…) que vale `vida` segundos, y cuenta cuántas llegaron. Exige
    /// la cabecera `Metadata-Flavor: Google`, como el de verdad.
    fn metadata(vida: &'static str) -> (String, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/token", l.local_addr().unwrap());
        let n = Arc::new(AtomicUsize::new(0));
        let cuenta = n.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut sabor = false;
                for linea in BufReader::new(&s).lines() {
                    let linea = linea.unwrap();
                    if linea.eq_ignore_ascii_case("metadata-flavor: google") {
                        sabor = true;
                    }
                    if linea.is_empty() {
                        break;
                    }
                }
                let i = cuenta.fetch_add(1, Ordering::SeqCst) + 1;
                let (estado, cuerpo) = if sabor {
                    (
                        "200 OK",
                        format!(
                            "{{\"access_token\":\"t{i}\",\"expires_in\":{vida},\"token_type\":\"Bearer\"}}"
                        ),
                    )
                } else {
                    ("403 Forbidden", "sin Metadata-Flavor".to_string())
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {estado}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
                    cuerpo.len()
                );
            }
        });
        (url, n)
    }

    #[test]
    fn un_token_vigente_no_se_vuelve_a_pedir() {
        let (url, n) = metadata("3599");
        let c = Credencial::del_metadata(url, MARGEN);
        assert_eq!(c.token().unwrap(), "t1");
        assert_eq!(c.token().unwrap(), "t1");
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(c.renovable());
    }

    /// El fallo de antes de A1: un token guardado para siempre. Aquí caduca
    /// dentro del margen, así que cada petición trae uno nuevo.
    #[test]
    fn el_que_caduca_dentro_del_margen_se_renueva() {
        let (url, n) = metadata("200");
        let c = Credencial::del_metadata(url, MARGEN);
        assert_eq!(c.token().unwrap(), "t1");
        assert_eq!(c.token().unwrap(), "t2");
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn caduca_de_verdad_con_el_reloj() {
        let (url, n) = metadata("1");
        let c = Credencial::del_metadata(url, Duration::ZERO);
        assert_eq!(c.token().unwrap(), "t1");
        assert_eq!(c.token().unwrap(), "t1", "todavía vale");
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(c.token().unwrap(), "t2", "caducó y se pidió otro");
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn sin_expires_in_no_se_guarda() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/token", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                for linea in BufReader::new(&s).lines() {
                    if linea.unwrap().is_empty() {
                        break;
                    }
                }
                let cuerpo = "{\"access_token\":\"sin-vida\"}";
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
                    cuerpo.len()
                );
            }
        });
        let c = Credencial::del_metadata(url, Duration::ZERO);
        assert_eq!(c.token().unwrap(), "sin-vida");
        assert_eq!(c.token().unwrap(), "sin-vida");
    }

    #[test]
    fn el_fijo_no_llama_a_nadie_y_lo_dice() {
        let c = Credencial::fija("de-gcloud");
        assert_eq!(c.token().unwrap(), "de-gcloud");
        assert!(!c.renovable());
    }

    #[test]
    fn sin_metadata_server_el_error_dice_que_hacer() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/token", l.local_addr().unwrap());
        drop(l);
        let e = Credencial::del_metadata(url, MARGEN).token().unwrap_err();
        assert!(e.contains("ORE_GCP_TOKEN"), "{e}");
        assert!(e.contains("Workload Identity"), "{e}");
    }

    /// Un metadata server que no recibe la cabecera contesta 403; el de mentira
    /// también, así que esto prueba que se manda.
    #[test]
    fn manda_metadata_flavor() {
        let (url, _) = metadata("3599");
        assert!(Credencial::del_metadata(url, MARGEN).token().is_ok());
    }
}
