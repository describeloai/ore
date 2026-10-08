//! **El API de Kubernetes** (0058 P4·3·2): `ore-postgres` crea, lee y borra las
//! VMs de los cómputos y sus ConfigMaps en `ore-pg-computo`, directamente.
//!
//! Es la decisión de P4·3: una VM es estado de ejecución que cambia a menudo
//! (en P6 se duerme y se despierta en segundos); pasarla por una cola de git y
//! Flux sería ruido y latencia. Lo que acota a `ore-postgres` no es el camino,
//! es su **cuenta**: un `Role` en `ore-pg-computo` y en ningún otro sitio
//! (`malla/86-…yaml`). Probado: en otro namespace, 403.
//!
//! - **TLS sólo hacia aquí, y sólo con la CA del clúster** (la que el kubelet
//!   monta con el token): las raíces del sistema, fuera. Un certificado que no
//!   firme esa CA no es el API server, aunque lo firme una CA pública.
//! - **El token se relee en cada petición**: el kubelet lo rota (proyectado,
//!   1 h), y uno guardado caducaría en silencio.
//! - **Escribir es `apply` del lado del servidor** (`fieldManager=ore-postgres`):
//!   idempotente, que es lo que un reconciliador necesita. Repetir no cambia nada.

use ore_core::json::Json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Dónde monta el kubelet la identidad del pod.
const CUENTA: &str = "/var/run/secrets/kubernetes.io/serviceaccount";

/// El nombre que está en el certificado del API server (no la IP del Service).
const API: &str = "https://kubernetes.default.svc";

pub struct Kube {
    agente: ureq::Agent,
    token: PathBuf,
    api: String,
}

impl Kube {
    /// Con la cuenta del pod.
    pub fn del_pod() -> Result<Kube, String> {
        Kube::con(API, &PathBuf::from(CUENTA))
    }

    pub fn con(api: &str, cuenta: &std::path::Path) -> Result<Kube, String> {
        let ca = std::fs::read(cuenta.join("ca.crt")).map_err(|e| {
            format!(
                "la CA del clúster en `{}`: {e} (¿la cuenta no monta su token?)",
                cuenta.display()
            )
        })?;
        let ca = native_tls::Certificate::from_pem(&ca)
            .map_err(|e| format!("la CA del clúster no es un PEM: {e}"))?;
        let tls = native_tls::TlsConnector::builder()
            .disable_built_in_roots(true)
            .add_root_certificate(ca)
            .build()
            .map_err(|e| format!("el TLS hacia el API server: {e}"))?;
        Ok(Kube {
            agente: ureq::AgentBuilder::new()
                .tls_connector(Arc::new(tls))
                .timeout_connect(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .build(),
            token: cuenta.join("token"),
            api: api.trim_end_matches('/').to_string(),
        })
    }

    /// Una petición: el código y el cuerpo. Un 4xx/5xx NO es un `Err`: lo decide
    /// quien llama (un 404 al borrar es éxito). `Err` es no haber llegado.
    pub fn pedir(
        &self,
        metodo: &str,
        camino: &str,
        tipo: Option<&str>,
        cuerpo: Option<&str>,
    ) -> Result<(u16, String), String> {
        let token = std::fs::read_to_string(&self.token)
            .map_err(|e| format!("el token de la cuenta: {e}"))?;
        let mut r = self
            .agente
            .request(metodo, &format!("{}{camino}", self.api))
            .set("Authorization", &format!("Bearer {}", token.trim()))
            .set("Accept", "application/json");
        if let Some(t) = tipo {
            r = r.set("Content-Type", t);
        }
        let respuesta = match cuerpo {
            Some(c) => r.send_string(c),
            None => r.call(),
        };
        match respuesta {
            Ok(x) => {
                let codigo = x.status();
                Ok((codigo, x.into_string().unwrap_or_default()))
            }
            Err(ureq::Error::Status(codigo, x)) => {
                Ok((codigo, x.into_string().unwrap_or_default()))
            }
            Err(e) => Err(format!("el API server: {e}")),
        }
    }

    /// `apply` del lado del servidor: crea o deja como se dice. El objeto lleva su
    /// `apiVersion`, `kind`, `metadata.name` y `metadata.namespace`.
    pub fn aplicar(&self, camino: &str, objeto: &Json) -> Result<(), String> {
        let (c, r) = self.pedir(
            "PATCH",
            &format!("{camino}?fieldManager=ore-postgres&force=true"),
            Some("application/apply-patch+yaml"),
            Some(&objeto.jcs()),
        )?;
        if (200..300).contains(&c) {
            Ok(())
        } else {
            Err(format!("aplicar `{camino}`: {c} {}", recorte(&r)))
        }
    }

    /// Leer: `None` si no está.
    pub fn leer(&self, camino: &str) -> Result<Option<String>, String> {
        match self.pedir("GET", camino, None, None)? {
            (200, r) => Ok(Some(r)),
            (404, _) => Ok(None),
            (c, r) => Err(format!("leer `{camino}`: {c} {}", recorte(&r))),
        }
    }

    /// Borrar: lo que ya no está también es éxito.
    pub fn borrar(&self, camino: &str) -> Result<(), String> {
        match self.pedir("DELETE", camino, None, None)? {
            (200..=299, _) | (404, _) => Ok(()),
            (c, r) => Err(format!("borrar `{camino}`: {c} {}", recorte(&r))),
        }
    }
}

/// El camino de un ConfigMap.
pub fn configmap(ns: &str, nombre: &str) -> String {
    format!("/api/v1/namespaces/{ns}/configmaps/{nombre}")
}

/// El camino de una VM de NeonVM.
pub fn vm(ns: &str, nombre: &str) -> String {
    format!("/apis/vm.neon.tech/v1/namespaces/{ns}/virtualmachines/{nombre}")
}

/// Un ConfigMap con un fichero dentro.
pub fn configmap_con(ns: &str, nombre: &str, fichero: &str, contenido: &str) -> Json {
    Json::obj([
        ("apiVersion", Json::s("v1")),
        ("kind", Json::s("ConfigMap")),
        (
            "metadata",
            Json::obj([
                ("name", Json::s(nombre)),
                ("namespace", Json::s(ns)),
                (
                    "labels",
                    Json::obj([("app.kubernetes.io/managed-by", Json::s("ore-postgres"))]),
                ),
            ]),
        ),
        (
            "data",
            Json::Obj([(fichero.to_string(), Json::s(contenido))].into()),
        ),
    ])
}

fn recorte(s: &str) -> String {
    s.chars().take(300).collect::<String>().replace('\n', " ")
}
