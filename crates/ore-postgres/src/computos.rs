//! **Los cómputos** (0058 P4·3·3): las VMs de NeonVM de los endpoints, en
//! `ore-pg-computo`. Lo que hacía `vm.sh` con `vm.yaml`, desde el reconciliador
//! y por el API de Kubernetes ([`crate::kube`]).
//!
//! Lo medido en P3 está dentro:
//! - **`podResources` = el mínimo de la VM**: sin reservas, el autoescalado de
//!   GKE no sube un nodo para ella (P3·2).
//! - **Una VM no nace mientras quede un runner con su nombre**: al borrarla, el
//!   runner viejo sigue unos segundos con la misma IP de la overlay y contestaba
//!   por la nueva (P3·5). Es también la capa 2 del cerco.
//! - **Listo es `compute_ctl` diciendo `running`**, con un token firmado con la
//!   llave propia (P4·3·1), no que el pod exista.

use crate::almacen::Fallo;
use crate::especificacion::{Computo, Datos, especificacion, iso};
use crate::kube::{self, Kube};
use crate::llaves::{Llave, token_de_computo};
use ore_core::json::Json;
use ore_entrada::http::{Plazos, pedir_con};
use std::time::Duration;

/// Lo que define una VM, además de su especificación.
pub struct Vm<'a> {
    pub nombre: &'a str,
    pub organizacion: &'a str,
    pub proyecto: &'a str,
    pub endpoint: &'a str,
    pub cu_min: f64,
    pub cu_max: f64,
    pub generacion: i64,
}

/// Lo que se ve de una VM.
#[derive(Debug, Clone, PartialEq)]
pub struct Estado {
    pub fase: String,
    pub ip_pod: Option<String>,
    pub ip_overlay: Option<String>,
}

/// Sin Kubernetes (la cuenta no monta su token): todo lo que lo necesita espera.
pub struct SinKube(pub String);

impl Computos for SinKube {
    fn configuracion(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
        _: &Datos,
        _: f64,
    ) -> Json {
        Json::obj([])
    }
    fn configurar(&self, _: &str, _: &str, _: &Json) -> Result<(), Fallo> {
        Err(Fallo::Reintentar(format!("sin Kubernetes: {}", self.0)))
    }
    fn estado(&self, _: &str) -> Result<Option<Estado>, Fallo> {
        Err(Fallo::Reintentar(format!("sin Kubernetes: {}", self.0)))
    }
    fn runner_vivo(&self, _: &str) -> Result<bool, Fallo> {
        Err(Fallo::Reintentar(format!("sin Kubernetes: {}", self.0)))
    }
    fn crear(&self, _: &Vm, _: &str) -> Result<(), Fallo> {
        Err(Fallo::Reintentar(format!("sin Kubernetes: {}", self.0)))
    }
    fn listo(&self, _: &str, _: &str) -> Result<bool, Fallo> {
        Ok(false)
    }
    fn borrar(&self, _: &str) -> Result<(), Fallo> {
        Err(Fallo::Reintentar(format!("sin Kubernetes: {}", self.0)))
    }
}

/// Lo que se le pide al cómputo. El de verdad es [`Neonvm`].
pub trait Computos: Send + Sync {
    /// El `config.json` de un cómputo (la especificación firmada).
    #[allow(clippy::too_many_arguments)]
    fn configuracion(
        &self,
        vm: &str,
        tenant: &str,
        timeline: &str,
        pageserver: &str,
        grupo: &str,
        replica: bool,
        datos: &Datos,
        cu_conexiones: f64,
    ) -> Json;
    /// P4·4: aplica una especificación nueva a un cómputo en marcha
    /// (`compute_ctl /configure`): roles, bases, lo borrado. Sin reiniciarlo.
    fn configurar(&self, vm: &str, ip_pod: &str, configuracion: &Json) -> Result<(), Fallo>;
    /// `None` si la VM no existe.
    fn estado(&self, vm: &str) -> Result<Option<Estado>, Fallo>;
    /// ¿Queda algún pod runner con ese nombre de VM?
    fn runner_vivo(&self, vm: &str) -> Result<bool, Fallo>;
    /// Su ConfigMap y su VM (idempotente).
    fn crear(&self, vm: &Vm, configuracion: &str) -> Result<(), Fallo>;
    /// ¿Dice su `compute_ctl` que está `running`?
    fn listo(&self, vm: &str, ip_pod: &str) -> Result<bool, Fallo>;
    /// La VM y su ConfigMap (idempotente: lo que no está, ya está borrado).
    fn borrar(&self, vm: &str) -> Result<(), Fallo>;
}

pub struct Neonvm {
    pub ns: String,
    pub imagen: String,
    pub pool: String,
    pub safekeepers: Vec<String>,
    kube: Kube,
    propia: Llave,
    almacen: Llave,
}

impl Neonvm {
    pub fn nuevo(
        ns: &str,
        imagen: &str,
        pool: &str,
        safekeepers: Vec<String>,
        kube: Kube,
        propia: Llave,
        almacen: Llave,
    ) -> Neonvm {
        Neonvm {
            ns: ns.into(),
            imagen: imagen.into(),
            pool: pool.into(),
            safekeepers,
            kube,
            propia,
            almacen,
        }
    }
}

fn k8s(e: String) -> Fallo {
    Fallo::Reintentar(e)
}

impl Computos for Neonvm {
    fn configuracion(
        &self,
        vm: &str,
        tenant: &str,
        timeline: &str,
        pageserver: &str,
        grupo: &str,
        replica: bool,
        datos: &Datos,
        cu_conexiones: f64,
    ) -> Json {
        let ahora = ahora_iso();
        especificacion(
            &Computo {
                nombre: vm,
                tenant,
                timeline,
                pageserver,
                safekeepers: &self.safekeepers,
                grupo,
                ahora: &ahora,
                replica,
                datos,
                cu_conexiones,
            },
            &self.propia,
            Some(&self.almacen),
        )
    }

    fn configurar(&self, vm: &str, ip_pod: &str, configuracion: &Json) -> Result<(), Fallo> {
        // Entera: `/configure` pide `{spec, compute_ctl_config}`, la misma forma que
        // el config.json del arranque (medido en vivo: sólo con `spec`, 422
        // «missing field `compute_ctl_config`»).
        let Json::Obj(todo) = configuracion else {
            return Err(Fallo::Definitivo("la configuración no es un objeto".into()));
        };
        if todo.get("spec").is_none() || todo.get("compute_ctl_config").is_none() {
            return Err(Fallo::Definitivo(
                "la configuración no trae `spec` y `compute_ctl_config`".into(),
            ));
        }
        let token = format!(
            "Bearer {}",
            token_de_computo(&self.propia, vm, ahora() + 300)
        );
        match pedir_con(
            "POST",
            &format!("{ip_pod}:3080"),
            "/configure",
            &[("Authorization", &token)],
            Some(configuracion),
            Plazos {
                conectar: Duration::from_secs(3),
                // compute_ctl contesta cuando lo ha aplicado (crear roles y bases).
                responder: Duration::from_secs(60),
            },
        ) {
            Ok((200, _)) => Ok(()),
            Ok((c, r)) => Err(Fallo::Reintentar(format!(
                "compute_ctl de {vm} no aplicó la configuración: {c} {}",
                r.chars().take(300).collect::<String>()
            ))),
            Err(e) => Err(Fallo::Reintentar(format!("compute_ctl de {vm}: {e}"))),
        }
    }

    fn estado(&self, vm: &str) -> Result<Option<Estado>, Fallo> {
        let Some(r) = self.kube.leer(&kube::vm(&self.ns, vm)).map_err(k8s)? else {
            return Ok(None);
        };
        Ok(Some(estado_de(&r)))
    }

    fn runner_vivo(&self, vm: &str) -> Result<bool, Fallo> {
        let (c, r) = self
            .kube
            .pedir(
                "GET",
                &format!(
                    "/api/v1/namespaces/{}/pods?labelSelector=vm.neon.tech%2Fname%3D{vm}",
                    self.ns
                ),
                None,
                None,
            )
            .map_err(k8s)?;
        if c != 200 {
            return Err(Fallo::Reintentar(format!("listar los runners: {c}")));
        }
        let n = ore_core::parse::parse(&r)
            .map_err(|_| Fallo::Reintentar("la lista de pods no analiza".into()))?;
        Ok(n.get("items").is_some_and(|(_, i)| !i.items().is_empty()))
    }

    fn crear(&self, vm: &Vm, configuracion: &str) -> Result<(), Fallo> {
        let nombre_cm = format!("{}-config", vm.nombre);
        self.kube
            .aplicar(
                &kube::configmap(&self.ns, &nombre_cm),
                &kube::configmap_con(&self.ns, &nombre_cm, "config.json", configuracion),
            )
            .map_err(k8s)?;
        self.kube
            .aplicar(
                &kube::vm(&self.ns, vm.nombre),
                &manifiesto(vm, &self.ns, &self.imagen, &self.pool),
            )
            .map_err(k8s)
    }

    fn listo(&self, vm: &str, ip_pod: &str) -> Result<bool, Fallo> {
        let token = format!(
            "Bearer {}",
            token_de_computo(&self.propia, vm, ahora() + 300)
        );
        match pedir_con(
            "GET",
            &format!("{ip_pod}:3080"),
            "/status",
            &[("Authorization", &token)],
            None,
            Plazos {
                conectar: Duration::from_secs(2),
                responder: Duration::from_secs(5),
            },
        ) {
            Ok((200, cuerpo)) => Ok(cuerpo.contains("\"running\"")),
            // Arrancando: aún no escucha, o aún no contesta bien.
            Ok(_) | Err(_) => Ok(false),
        }
    }

    fn borrar(&self, vm: &str) -> Result<(), Fallo> {
        self.kube.borrar(&kube::vm(&self.ns, vm)).map_err(k8s)?;
        self.kube
            .borrar(&kube::configmap(&self.ns, &format!("{vm}-config")))
            .map_err(k8s)
    }
}

/// De la VM que devuelve el API a lo que importa de ella.
pub fn estado_de(cuerpo: &str) -> Estado {
    let n = ore_core::parse::parse(cuerpo).ok();
    let de = |k: &str| {
        n.as_ref()
            .and_then(|n| n.get("status"))
            .and_then(|(_, s)| s.get(k))
            .and_then(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Estado {
        fase: de("phase").unwrap_or_else(|| "Pending".into()),
        ip_pod: de("podIP"),
        ip_overlay: de("extraNetIP"),
    }
}

/// `250m` de 0.25.
fn milis(cu: f64) -> String {
    format!("{}m", (cu * 1000.0).round() as i64)
}

/// Un número tal cual en el JSON. ⚠️ Medido en vivo (P4·3·3): el API server
/// valida `guest.cpus` contra el esquema de NeonVM y un `"250m"` es un 500
/// («expected numeric»); con `vm.yaml` funcionaba porque YAML ya lo escribía como
/// número. `Json` no modela decimales a propósito: va crudo.
fn numero(x: f64) -> Json {
    Json::Crudo(x.to_string())
}

/// Ranuras de memoria de 1 GiB para unas CU (1 CU = 4 GiB), al menos una.
fn ranuras(cu: f64) -> i64 {
    ((cu * 4.0).ceil() as i64).max(1)
}

/// La VM de NeonVM: lo de `pruebas-de-fuego/ore-postgres/vm.yaml`, con los límites del endpoint.
pub fn manifiesto(vm: &Vm, ns: &str, imagen: &str, pool: &str) -> Json {
    let (rmin, rmax) = (ranuras(vm.cu_min), ranuras(vm.cu_max));
    let limites = format!(
        r#"{{"min":{{"cpu":{},"mem":"{rmin}Gi"}},"max":{{"cpu":{},"mem":"{rmax}Gi"}}}}"#,
        vm.cu_min, vm.cu_max
    );
    let puerto = |p: i64| Json::obj([("port", Json::Int(p))]);
    Json::obj([
        ("apiVersion", Json::s("vm.neon.tech/v1")),
        ("kind", Json::s("VirtualMachine")),
        (
            "metadata",
            Json::obj([
                ("name", Json::s(vm.nombre)),
                ("namespace", Json::s(ns)),
                (
                    "labels",
                    Json::obj([
                        ("autoscaling.neon.tech/enabled", Json::s("true")),
                        ("ore.dev/rol", Json::s("postgres")),
                        ("ore.dev/organizacion", Json::s(vm.organizacion)),
                        ("ore.dev/proyecto", Json::s(vm.proyecto)),
                        ("ore.dev/endpoint", Json::s(vm.endpoint)),
                        ("app.kubernetes.io/managed-by", Json::s("ore-postgres")),
                    ]),
                ),
                (
                    "annotations",
                    Json::obj([
                        ("autoscaling.neon.tech/bounds", Json::s(limites)),
                        ("ore.dev/generacion", Json::s(vm.generacion.to_string())),
                    ]),
                ),
            ]),
        ),
        (
            "spec",
            Json::obj([
                ("schedulerName", Json::s("autoscale-scheduler")),
                ("nodeSelector", Json::obj([("ore.dev/pool", Json::s(pool))])),
                (
                    "tolerations",
                    Json::Arr(vec![Json::obj([
                        ("key", Json::s("ore.dev/neon")),
                        ("operator", Json::s("Equal")),
                        ("value", Json::s("true")),
                        ("effect", Json::s("NoSchedule")),
                    ])]),
                ),
                ("restartPolicy", Json::s("Never")),
                (
                    "podResources",
                    Json::obj([(
                        "requests",
                        Json::obj([
                            ("cpu", Json::s(milis(vm.cu_min))),
                            ("memory", Json::s(format!("{rmin}Gi"))),
                        ]),
                    )]),
                ),
                (
                    "guest",
                    Json::obj([
                        (
                            "cpus",
                            Json::obj([
                                ("min", numero(vm.cu_min)),
                                ("use", numero(vm.cu_min)),
                                ("max", numero(vm.cu_max)),
                            ]),
                        ),
                        ("memorySlotSize", Json::s("1Gi")),
                        (
                            "memorySlots",
                            Json::obj([
                                ("min", Json::Int(rmin)),
                                ("use", Json::Int(rmin)),
                                ("max", Json::Int(rmax)),
                            ]),
                        ),
                        (
                            "rootDisk",
                            Json::obj([("image", Json::s(imagen)), ("size", Json::s("8Gi"))]),
                        ),
                        (
                            "command",
                            Json::Arr(vec![Json::s("/usr/local/bin/compute_ctl")]),
                        ),
                        (
                            "args",
                            Json::Arr(
                                [
                                    "--pgdata",
                                    "/var/db/postgres/compute",
                                    "-C",
                                    "postgresql://cloud_admin@127.0.0.1:5432/postgres",
                                    "-b",
                                    "/usr/local/bin/postgres",
                                    "--compute-id",
                                    vm.nombre,
                                    "--config",
                                    "/var/db/postgres/configs/config.json",
                                    "--filecache-connstr",
                                    "host=127.0.0.1 port=5432 dbname=postgres user=cloud_admin sslmode=disable application_name=vm-monitor",
                                ]
                                .into_iter()
                                .map(Json::s)
                                .collect(),
                            ),
                        ),
                        (
                            "ports",
                            Json::Arr(vec![
                                puerto(5432),
                                puerto(6432),
                                puerto(3080),
                                puerto(10301),
                                puerto(9100),
                            ]),
                        ),
                        (
                            "env",
                            Json::Arr(vec![Json::obj([
                                ("name", Json::s("AUTOSCALING")),
                                ("value", Json::s("true")),
                            ])]),
                        ),
                    ]),
                ),
                (
                    "disks",
                    Json::Arr(vec![Json::obj([
                        ("name", Json::s("cfg")),
                        ("mountPath", Json::s("/var/db/postgres/configs")),
                        (
                            "configMap",
                            Json::obj([("name", Json::s(format!("{}-config", vm.nombre)))]),
                        ),
                    ])]),
                ),
                ("extraNetwork", Json::obj([("enable", Json::Bool(true))])),
            ]),
        ),
    ])
}

fn ahora() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// La hora de ahora en RFC 3339, para la especificación.
pub fn ahora_iso() -> String {
    iso(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_vm_lleva_sus_limites_y_su_reserva() {
        let m = manifiesto(
            &Vm {
                nombre: "ep-0123456789abcdef0123",
                organizacion: "org_1",
                proyecto: "ventas",
                endpoint: "principal",
                cu_min: 0.25,
                cu_max: 1.0,
                generacion: 3,
            },
            "ore-pg-computo",
            "img:1",
            "neon",
        )
        .jcs();
        for esperado in [
            r#""podResources":{"requests":{"cpu":"250m","memory":"1Gi"}}"#,
            r#""cpus":{"max":1,"min":0.25,"use":0.25}"#,
            r#""memorySlots":{"max":4,"min":1,"use":1}"#,
            r#""ore.dev/rol":"postgres""#,
            r#""ore.dev/generacion":"3""#,
            r#""name":"ep-0123456789abcdef0123-config""#,
            r#""--compute-id","ep-0123456789abcdef0123""#,
        ] {
            assert!(m.contains(esperado), "falta {esperado} en {m}");
        }
    }

    #[test]
    fn el_estado_de_una_vm() {
        let e = estado_de(
            r#"{"status":{"phase":"Running","podIP":"10.1.2.3","extraNetIP":"10.100.128.4"}}"#,
        );
        assert_eq!(e.fase, "Running");
        assert_eq!(e.ip_pod.as_deref(), Some("10.1.2.3"));
        assert_eq!(e.ip_overlay.as_deref(), Some("10.100.128.4"));
        assert_eq!(estado_de(r#"{"status":{}}"#).fase, "Pending");
    }
}
