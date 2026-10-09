//! **El token de Entra de una app del cliente** (ADR 0061 D-O3, D-O5): la
//! cuenta de Google de la celda pide su token de identidad (`ore-gcp`,
//! audiencia `api://AzureADTokenExchange`) y Entra se lo cambia por uno de la
//! app del cliente, que tiene una *federated identity credential* para ella: el
//! *client credentials* con aserción federada. Sin secretos.
//!
//! El mismo canje sirve para Storage (`ore-azure`) y para Graph (`ore-graph`):
//! sólo cambia el ámbito. Un token fijo en la variable que diga quien lo usa
//! (`ORE_AZURE_TOKEN`, `ORE_GRAPH_TOKEN`) se salta el canje: el laboratorio.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const LOGIN: &str = "https://login.microsoftonline.com";
/// La audiencia del token de Google que Entra acepta en una credencial federada.
pub const AUDIENCIA: &str = "api://AzureADTokenExchange";
/// Cuánto se guarda un token de Entra (viven entre 60 y 90 minutos).
const VIDA_DEL_TOKEN: Duration = Duration::from_secs(45 * 60);

static CANJES: AtomicUsize = AtomicUsize::new(0);

/// Cuántas veces se le pidió un token a Entra en este proceso.
pub fn canjes() -> usize {
    CANJES.load(Ordering::Relaxed)
}

/// **Una app del tenant de un cliente, para un ámbito**, con su token guardado
/// mientras vale.
pub struct Entra {
    pub tenant: String,
    pub cliente: String,
    alcance: &'static str,
    variable: &'static str,
    agente: String,
    guardado: Mutex<Option<(String, Instant)>>,
}

impl Entra {
    /// `alcance`, como `https://graph.microsoft.com/.default`; `variable`, la
    /// del token fijo; `agente`, el `User-Agent`.
    pub fn nueva(
        tenant: &str,
        cliente: &str,
        alcance: &'static str,
        variable: &'static str,
        agente: &str,
    ) -> Entra {
        Entra {
            tenant: tenant.to_string(),
            cliente: cliente.to_string(),
            alcance,
            variable,
            agente: agente.to_string(),
            guardado: Mutex::new(None),
        }
    }

    /// **El token**: el fijo, el guardado, o uno nuevo de Entra.
    pub fn token(&self, http: &ureq::Agent) -> Result<String, String> {
        if let Ok(t) = std::env::var(self.variable)
            && !t.is_empty()
        {
            return Ok(t);
        }
        let mut v = self.guardado.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, caduca)) = v.as_ref()
            && Instant::now() < *caduca
        {
            return Ok(t.clone());
        }
        let id = ore_gcp::identidad(AUDIENCIA)?;
        let r = http
            .post(&format!("{LOGIN}/{}/oauth2/v2.0/token", self.tenant))
            .set("user-agent", &self.agente)
            .send_form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.cliente),
                ("scope", self.alcance),
                (
                    "client_assertion_type",
                    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                ),
                ("client_assertion", &id),
            ]);
        CANJES.fetch_add(1, Ordering::Relaxed);
        let texto = match r {
            Ok(x) => x
                .into_string()
                .map_err(|e| format!("el token de Entra no se pudo leer: {e}"))?,
            Err(ureq::Error::Status(c, x)) => {
                return Err(motivo(
                    &self.tenant,
                    &self.cliente,
                    c,
                    &x.into_string().unwrap_or_default(),
                ));
            }
            Err(e) => return Err(format!("Entra no contesta: {e}")),
        };
        let n = ore_core::parse::parse(&texto)
            .map_err(|e| format!("el token de Entra no analiza: {e:?}"))?;
        let t = n
            .get("access_token")
            .and_then(|(_, v)| v.as_str())
            .ok_or("Entra no devolvió `access_token`")?
            .to_string();
        *v = Some((t.clone(), Instant::now() + VIDA_DEL_TOKEN));
        Ok(t)
    }
}

/// Por qué Entra no canjeó, dicho para quien lo arregla: los `AADSTS` que se
/// ven al montar la federación, con su arreglo; la primera línea de lo demás.
pub fn motivo(tenant: &str, cliente: &str, estado: u16, cuerpo: &str) -> String {
    let n = ore_core::parse::parse(cuerpo).ok();
    let d = n
        .as_ref()
        .and_then(|n| n.get("error_description"))
        .and_then(|(_, v)| v.as_str().map(String::from))
        .unwrap_or_else(|| cuerpo.chars().take(200).collect());
    let d = d.lines().next().unwrap_or("").to_string();
    let que = if d.contains("AADSTS70021") {
        format!(
            "la app `{cliente}` no tiene una credencial federada para la cuenta de Google de esta \
             celda (issuer `https://accounts.google.com`, subject = su ID único, audiencia \
             `{AUDIENCIA}`), o aún se está propagando: espera unos minutos"
        )
    } else if d.contains("AADSTS700016") {
        format!("no hay una app `{cliente}` en el tenant `{tenant}`")
    } else if d.contains("AADSTS90002") || d.contains("AADSTS900023") {
        format!("no hay un tenant `{tenant}` en Entra")
    } else if d.contains("AADSTS65001") || d.contains("AADSTS7000215") {
        format!(
            "la app `{cliente}` no tiene el consentimiento de un administrador para lo que se pide"
        )
    } else {
        "Entra no canjea el token de la celda".to_string()
    };
    format!("{que} ({estado}: {d})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entra_se_explica() {
        let m = motivo(
            "t",
            "app",
            400,
            r#"{"error":"invalid_request","error_description":"AADSTS70021: No matching federated identity record found for presented assertion.\r\nTrace ID: x"}"#,
        );
        assert!(
            m.contains("credencial federada") && m.contains("espera unos minutos"),
            "{m}"
        );
        assert!(!m.contains("Trace ID"), "{m}");
        let m = motivo(
            "t",
            "app",
            400,
            r#"{"error_description":"AADSTS700016: x"}"#,
        );
        assert!(m.contains("no hay una app `app`"), "{m}");
    }
}
