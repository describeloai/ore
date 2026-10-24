//! Elastic Sessions S2a (docs/elastic-sessions.md §3) · **Redimensionar en caliente
//! el pod de una sesión**: lo que `ore-redimensionar` sabe de la API de Kubernetes,
//! sin red. La red está en `main.rs`; aquí, lo que se puede probar:
//!
//! - las cantidades de Kubernetes (`8Gi`, `8192Mi`, `500m`, `2`);
//! - el parche de `pods/resize` (strategic merge, sólo memoria y CPU: el
//!   subrecurso no deja tocar nada más);
//! - qué pod es el de un Job, y en qué quedó un redimensionado.
//!
//! Medido en S0/R0 (victor, k8s 1.35): crecer de 4 a 16 GiB tarda 2–4 s sin
//! reiniciar; un nodo sin sitio deja `PodResizePending` con `Infeasible`, y el
//! pod sigue igual.
use serde_json::{Value, json};

/// Lo que se pide: el pod del Job `job` en `namespace`, su contenedor, y los
/// recursos que tiene que quedar teniendo.
#[derive(Debug, Clone, PartialEq)]
pub struct Peticion {
    pub namespace: String,
    pub job: String,
    pub contenedor: String,
    /// `requests.memory`: lo que el nodo le guarda.
    pub memoria_pedida_mb: u64,
    /// `limits.memory`: su cgroup (`memory.max`).
    pub memoria_tope_mb: u64,
    /// `limits.cpu`, en milinúcleos. `requests.cpu` no se toca: la clase de QoS
    /// del pod no puede cambiar al redimensionar, y con la CPU pedida por debajo
    /// del tope sigue siendo `Burstable`.
    pub cpu_tope_m: u64,
    /// Cuánto esperar a que el `status` diga lo nuevo.
    pub espera_s: u64,
}

/// La petición, de la entrada estándar (JSON), en `namespace`: el del propio
/// pod (lo monta su cuenta de servicio). La petición no lo elige: este binario
/// sólo toca pods de su celda.
pub fn peticion(texto: &str, namespace: &str) -> Result<Peticion, String> {
    let v: Value =
        serde_json::from_str(texto).map_err(|e| format!("la petición no es JSON: {e}"))?;
    let s = |k: &str| -> Result<String, String> {
        let x = v
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if x.is_empty() || x.contains('/') || x.contains('?') || x.contains('&') {
            return Err(format!("falta `{k}`, o no es un nombre de Kubernetes"));
        }
        Ok(x)
    };
    let n = |k: &str| -> Result<u64, String> {
        v.get(k)
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("falta `{k}` (un entero positivo)"))
    };
    let p = Peticion {
        namespace: namespace.trim().to_string(),
        job: s("job")?,
        contenedor: s("contenedor")?,
        memoria_pedida_mb: n("memoria_pedida_mb")?,
        memoria_tope_mb: n("memoria_tope_mb")?,
        cpu_tope_m: n("cpu_tope_m")?,
        espera_s: v
            .get("espera_s")
            .and_then(Value::as_u64)
            .unwrap_or(15)
            .min(60),
    };
    if p.namespace.is_empty() {
        return Err("no sé en qué namespace estoy".into());
    }
    if p.memoria_pedida_mb > p.memoria_tope_mb {
        return Err("la memoria pedida no puede pasar del tope".into());
    }
    Ok(p)
}

/// Una cantidad de memoria de Kubernetes, en MiB (redondeando hacia abajo).
pub fn memoria_mb(q: &str) -> Option<u64> {
    let q = q.trim();
    let (num, mult): (&str, f64) = [
        ("Ki", 1024.0),
        ("Mi", 1024f64.powi(2)),
        ("Gi", 1024f64.powi(3)),
        ("Ti", 1024f64.powi(4)),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
    ]
    .iter()
    .find_map(|(suf, m)| q.strip_suffix(suf).map(|n| (n, *m)))
    .unwrap_or((q, 1.0));
    let n: f64 = num.parse().ok()?;
    Some((n * mult / 1024f64.powi(2)) as u64)
}

/// Una cantidad de CPU de Kubernetes, en milinúcleos.
pub fn cpu_m(q: &str) -> Option<u64> {
    let q = q.trim();
    if let Some(m) = q.strip_suffix('m') {
        return m.parse().ok();
    }
    let n: f64 = q.parse().ok()?;
    Some((n * 1000.0).round() as u64)
}

/// El cuerpo del `PATCH …/pods/{pod}/resize` (`application/strategic-merge-patch+json`):
/// el contenedor por su nombre, y sólo memoria y CPU. Con `merge` a secas se
/// pierde el resto de la lista (medido en R0: «Forbidden» por quitar
/// `ephemeral-storage`).
pub fn parche(p: &Peticion) -> Value {
    json!({"spec": {"containers": [{
        "name": p.contenedor,
        "resources": {
            "requests": {"memory": format!("{}Mi", p.memoria_pedida_mb)},
            "limits": {"memory": format!("{}Mi", p.memoria_tope_mb), "cpu": format!("{}m", p.cpu_tope_m)},
        },
    }]}})
}

/// El pod del Job, de la lista `GET pods?labelSelector=job-name=<job>`: el que
/// corre (un Job de sesión tiene uno; si hubiera dos, el que no ha terminado).
pub fn pod_de_la_lista(lista: &Value) -> Option<String> {
    let items = lista.get("items")?.as_array()?;
    let fase = |p: &Value| {
        p.pointer("/status/phase")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    items
        .iter()
        .find(|p| fase(p) == "Running")
        .or_else(|| {
            items
                .iter()
                .find(|p| !matches!(fase(p).as_str(), "Succeeded" | "Failed"))
        })
        .and_then(|p| p.pointer("/metadata/name")?.as_str().map(str::to_string))
}

/// En qué quedó el redimensionado, según el pod.
#[derive(Debug, Clone, PartialEq)]
pub enum Estado {
    /// El `status` del contenedor ya dice el tope nuevo: el cgroup cambió.
    Hecho,
    /// `PodResizePending`: `Infeasible` (no cabe en el nodo, ni cabrá) o
    /// `Deferred` (ahora no cabe; el kubelet lo reintenta). Con su mensaje.
    Pendiente { razon: String, mensaje: String },
    /// Todavía no: aceptado y en curso, o sin nada que decir aún.
    EnCurso,
}

impl Estado {
    pub fn nombre(&self) -> String {
        match self {
            Estado::Hecho => "hecho".into(),
            Estado::Pendiente { razon, .. } => razon.to_lowercase(),
            Estado::EnCurso => "en_curso".into(),
        }
    }
}

/// Lo que el contenedor tiene AHORA (`status.containerStatuses[].resources`):
/// `(memoria_tope_mb, cpu_tope_m)`.
pub fn recursos_reales(pod: &Value, contenedor: &str) -> Option<(u64, u64)> {
    let c = pod
        .pointer("/status/containerStatuses")?
        .as_array()?
        .iter()
        .find(|c| c.get("name").and_then(Value::as_str) == Some(contenedor))?;
    let mem = memoria_mb(c.pointer("/resources/limits/memory")?.as_str()?)?;
    let cpu = c
        .pointer("/resources/limits/cpu")
        .and_then(Value::as_str)
        .and_then(cpu_m)
        .unwrap_or(0);
    Some((mem, cpu))
}

/// En qué quedó, mirando el pod: el tope real, o la condición que lo frena.
pub fn estado(pod: &Value, p: &Peticion) -> Estado {
    if recursos_reales(pod, &p.contenedor).is_some_and(|(m, _)| m == p.memoria_tope_mb) {
        return Estado::Hecho;
    }
    let condiciones = pod.pointer("/status/conditions").and_then(Value::as_array);
    if let Some(c) = condiciones.into_iter().flatten().find(|c| {
        c.get("type").and_then(Value::as_str) == Some("PodResizePending")
            && c.get("status").and_then(Value::as_str) == Some("True")
    }) {
        let t = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        return Estado::Pendiente {
            razon: t("reason"),
            mensaje: t("message"),
        };
    }
    Estado::EnCurso
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn pide() -> Peticion {
        peticion(
            r#"{"job":"puesto-ana-1a2b3c4d","contenedor":"puesto",
                "memoria_pedida_mb":8192,"memoria_tope_mb":8192,"cpu_tope_m":4000}"#,
            "t-victor",
        )
        .unwrap()
    }

    #[test]
    fn las_cantidades_de_kubernetes() {
        assert_eq!(memoria_mb("8Gi"), Some(8192));
        assert_eq!(memoria_mb("8192Mi"), Some(8192));
        assert_eq!(memoria_mb("768Mi"), Some(768));
        assert_eq!(memoria_mb("4294967296"), Some(4096));
        assert_eq!(memoria_mb("1G"), Some(953));
        assert_eq!(memoria_mb("nada"), None);
        assert_eq!(cpu_m("2"), Some(2000));
        assert_eq!(cpu_m("500m"), Some(500));
        assert_eq!(cpu_m("1.5"), Some(1500));
    }

    #[test]
    fn la_peticion_se_comprueba() {
        let p = pide();
        assert_eq!(p.espera_s, 15);
        assert_eq!(p.namespace, "t-victor");
        assert!(peticion(r#"{"job":"j"}"#, "t-victor").is_err());
        // El namespace es el del pod: lo que diga la petición no cuenta.
        let otro = peticion(
            r#"{"namespace":"t-demo","job":"j","contenedor":"puesto",
                "memoria_pedida_mb":1,"memoria_tope_mb":1,"cpu_tope_m":1}"#,
            "t-victor",
        )
        .unwrap();
        assert_eq!(otro.namespace, "t-victor");
        assert!(peticion(
            r#"{"job":"j","contenedor":"puesto","memoria_pedida_mb":1,"memoria_tope_mb":1,"cpu_tope_m":1}"#,
            ""
        )
        .is_err());
        // Un nombre con `/` o `?` iría a otra ruta de la API.
        assert!(
            peticion(
                r#"{"job":"x?labelSelector=y","contenedor":"puesto",
                "memoria_pedida_mb":1,"memoria_tope_mb":1,"cpu_tope_m":1}"#,
                "t-victor"
            )
            .is_err()
        );
        assert!(
            peticion(
                r#"{"job":"j","contenedor":"puesto",
                "memoria_pedida_mb":9000,"memoria_tope_mb":8192,"cpu_tope_m":1}"#,
                "t-victor"
            )
            .is_err()
        );
    }

    #[test]
    fn el_parche_es_solo_memoria_y_cpu_del_contenedor() {
        let v = parche(&pide());
        assert_eq!(
            v,
            json!({"spec": {"containers": [{"name": "puesto", "resources": {
                "requests": {"memory": "8192Mi"},
                "limits": {"memory": "8192Mi", "cpu": "4000m"}}}]}})
        );
    }

    #[test]
    fn el_pod_del_job_es_el_que_corre() {
        let l = json!({"items": [
            {"metadata": {"name": "viejo"}, "status": {"phase": "Failed"}},
            {"metadata": {"name": "bueno"}, "status": {"phase": "Running"}}]});
        assert_eq!(pod_de_la_lista(&l), Some("bueno".into()));
        let l = json!({"items": [{"metadata": {"name": "nace"}, "status": {"phase": "Pending"}}]});
        assert_eq!(pod_de_la_lista(&l), Some("nace".into()));
        assert_eq!(pod_de_la_lista(&json!({"items": []})), None);
    }

    #[test]
    fn en_que_quedo() {
        let p = pide();
        let con = |mem: &str, cond: Value| {
            json!({"status": {"conditions": cond, "containerStatuses": [
                {"name": "init", "resources": {"limits": {"memory": "8Gi"}}},
                {"name": "puesto", "resources": {"limits": {"memory": mem, "cpu": "4"}}}]}})
        };
        assert_eq!(estado(&con("8Gi", json!([])), &p), Estado::Hecho);
        assert_eq!(
            recursos_reales(&con("8Gi", json!([])), "puesto"),
            Some((8192, 4000))
        );
        assert_eq!(estado(&con("4Gi", json!([])), &p), Estado::EnCurso);
        let e = estado(
            &con(
                "4Gi",
                json!([{"type": "PodResizePending", "status": "True", "reason": "Infeasible",
                                "message": "Node didn't have enough capacity: memory"}]),
            ),
            &p,
        );
        assert_eq!(e.nombre(), "infeasible");
        assert!(matches!(e, Estado::Pendiente { ref mensaje, .. } if mensaje.contains("capacity")));
        let e = estado(
            &con(
                "4Gi",
                json!([{"type": "PodResizePending", "status": "True", "reason": "Deferred"}]),
            ),
            &p,
        );
        assert_eq!(e.nombre(), "deferred");
    }
}
