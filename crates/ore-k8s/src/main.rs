//! `ore-redimensionar` — redimensiona en caliente el pod de una sesión
//! (Elastic Sessions S2a), por la entrada estándar.
//!
//! ```text
//!   echo '{"job":"puesto-ana-1a2b3c4d","contenedor":"puesto",
//!          "memoria_pedida_mb":8192,"memoria_tope_mb":8192,"cpu_tope_m":4000}' | ore-redimensionar
//!   → {"estado":"hecho","pod":"puesto-ana-1a2b3c4d-x7k2p","memoria_tope_mb":8192,"cpu_tope_m":4000,"ms":2310}
//! ```
//!
//! Lo lanza `ore-serve`, que no habla TLS (`ore-cli/tests/dependencias.rs`). Habla
//! con la API del clúster y con nadie más, con la cuenta de servicio del pod
//! (`/var/run/secrets/kubernetes.io/serviceaccount`): `get`/`list` de `pods` y
//! `patch` de `pods/resize` en su namespace, y nada más (`malla/40-ore-serve.yaml`).
//!
//! `estado`: `hecho` (el cgroup ya tiene el tope nuevo), `infeasible` o `deferred`
//! (el nodo no tiene sitio: el pod sigue como estaba), `en_curso` (no se vio a
//! tiempo). Salidas: 0 con el JSON; 64 lo pedido no vale; 65 no hay pod; 69 la API
//! no contestó bien. El porqué, en una línea por stderr.
use std::io::Read;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const CUENTA: &str = "/var/run/secrets/kubernetes.io/serviceaccount";

struct Api {
    base: String,
    testigo: String,
    agente: ureq::Agent,
}

impl Api {
    /// La API desde dentro del clúster. `ORE_K8S_API`, `ORE_K8S_TESTIGO` y
    /// `ORE_K8S_CA` la cambian (para probarlo fuera).
    fn del_cluster() -> Result<Api, String> {
        let base = match std::env::var("ORE_K8S_API") {
            Ok(b) => b,
            Err(_) => {
                let h = std::env::var("KUBERNETES_SERVICE_HOST")
                    .map_err(|_| "no estoy en un pod: falta KUBERNETES_SERVICE_HOST".to_string())?;
                let p = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
                format!("https://{h}:{p}")
            }
        };
        let ruta =
            |var: &str, f: &str| std::env::var(var).unwrap_or_else(|_| format!("{CUENTA}/{f}"));
        let testigo = std::fs::read_to_string(ruta("ORE_K8S_TESTIGO", "token"))
            .map_err(|e| format!("sin el testigo de la cuenta de servicio: {e}"))?
            .trim()
            .to_string();
        let ca = std::fs::read(ruta("ORE_K8S_CA", "ca.crt"))
            .map_err(|e| format!("sin la CA del clúster: {e}"))?;
        let ca = native_tls::Certificate::from_pem(&ca)
            .map_err(|e| format!("la CA del clúster no se lee: {e}"))?;
        let tls = native_tls::TlsConnector::builder()
            .add_root_certificate(ca)
            .build()
            .map_err(|e| format!("no se pudo abrir el TLS: {e}"))?;
        let agente = ureq::AgentBuilder::new()
            .tls_connector(std::sync::Arc::new(tls))
            .redirects(0)
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build();
        Ok(Api {
            base,
            testigo,
            agente,
        })
    }

    fn pide(&self, metodo: &str, ruta: &str) -> ureq::Request {
        self.agente
            .request(metodo, &format!("{}{ruta}", self.base))
            .set("authorization", &format!("Bearer {}", self.testigo))
            .set("accept", "application/json")
    }

    fn leer(r: Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
        match r {
            Ok(r) => r
                .into_string()
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .ok_or_else(|| "la API no devolvió JSON".to_string()),
            Err(ureq::Error::Status(c, r)) => {
                let cuerpo: Value = r
                    .into_string()
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or(Value::Null);
                let m = cuerpo
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Err(format!("la API dijo {c}: {m}"))
            }
            Err(e) => Err(format!("la API no contestó: {e}")),
        }
    }

    fn get(&self, ruta: &str) -> Result<Value, String> {
        Self::leer(self.pide("GET", ruta).call())
    }
}

fn main() -> ExitCode {
    let mut entrada = String::new();
    if std::io::stdin().read_to_string(&mut entrada).is_err() || entrada.trim().is_empty() {
        eprintln!("✗ la petición va por la entrada estándar (JSON)");
        return ExitCode::from(64);
    }
    // El namespace es el del propio pod, no el que diga la petición.
    let ns = std::env::var("ORE_K8S_NAMESPACE")
        .or_else(|_| std::fs::read_to_string(format!("{CUENTA}/namespace")))
        .unwrap_or_default();
    let p = match ore_k8s::peticion(&entrada, &ns) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(64);
        }
    };
    match redimensionar(&p) {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err((codigo, e)) => {
            eprintln!("✗ {e}");
            ExitCode::from(codigo)
        }
    }
}

fn redimensionar(p: &ore_k8s::Peticion) -> Result<Value, (u8, String)> {
    let t0 = Instant::now();
    let api = Api::del_cluster().map_err(|e| (69, e))?;
    let ns = &p.namespace;
    let lista = api
        .get(&format!(
            "/api/v1/namespaces/{ns}/pods?labelSelector=job-name%3D{}",
            p.job
        ))
        .map_err(|e| (69, e))?;
    let pod =
        ore_k8s::pod_de_la_lista(&lista).ok_or((65, format!("el Job `{}` no tiene pod", p.job)))?;
    let ruta = format!("/api/v1/namespaces/{ns}/pods/{pod}");
    Api::leer(
        api.pide("PATCH", &format!("{ruta}/resize"))
            .set("content-type", "application/strategic-merge-patch+json")
            .send_string(&ore_k8s::parche(p).to_string()),
    )
    .map_err(|e| (69, e))?;
    // Lo que el pod dice, cada 250 ms, hasta que diga lo nuevo o por qué no.
    let limite = t0 + Duration::from_secs(p.espera_s);
    let (estado, visto) = loop {
        let v = api.get(&ruta).map_err(|e| (69, e))?;
        let e = ore_k8s::estado(&v, p);
        if e != ore_k8s::Estado::EnCurso || Instant::now() >= limite {
            break (e, v);
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let (mem, cpu) = ore_k8s::recursos_reales(&visto, &p.contenedor).unwrap_or((0, 0));
    let mut r = json!({
        "estado": estado.nombre(),
        "pod": pod,
        "memoria_tope_mb": mem,
        "cpu_tope_m": cpu,
        "ms": t0.elapsed().as_millis() as u64,
    });
    if let ore_k8s::Estado::Pendiente { mensaje, .. } = &estado {
        r["mensaje"] = json!(mensaje);
    }
    Ok(r)
}
