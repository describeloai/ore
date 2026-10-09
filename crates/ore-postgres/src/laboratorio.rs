//! **El laboratorio local** (0058 P6·1): cómputos en Docker y un almacenamiento
//! de mentira. ⛔ **Desechable**: sólo con la *feature* `laboratorio`; el binario
//! de producción no lo lleva, y borrar este fichero no toca la lógica del
//! producto (dormir, despertar, el pool, la API).
//!
//! ```text
//!   ore-postgres --computos docker:172.29.51.9:2375 --red-computo p5lab_lab --imagen-computo p5lab-computo:2
//!       │  la API de Docker (un socat en la red del laboratorio, nunca fuera de ella)
//!       ▼
//!   contenedor `ep-…` = una VM: Postgres 17 + pgbouncer + el compute_ctl falso
//!   (pruebas-de-fuego/ore-postgres/lab/computo/), con el volumen `p5lab-almacen`
//! ```
//!
//! - **El mismo contrato que NeonVM**: `listo` y `configurar` son las mismas
//!   llamadas a `compute_ctl` ([`crate::computos::listo_por_http`],
//!   [`crate::computos::configurar_por_http`]) con el mismo token; la
//!   especificación es la misma ([`crate::especificacion::especificacion`]).
//! - **El almacenamiento es un volumen compartido**: el cómputo de mentira guarda
//!   sus datos en `/almacen/<tenant>/<timeline>`, como si fuera el pageserver, así
//!   que dormir (borrar el contenedor) y despertar conserva los datos.
//!   [`AlmacenDeMentira`] borra el directorio del tenant al borrar el proyecto.
//!
//! ⚠️ Lo que el laboratorio NO es: ni ramas con los datos de su padre (una rama
//!   nueva nace vacía), ni réplicas de lectura, ni PITR.

use crate::almacen::{Almacen, Fallo, Origen};
use crate::computos::{
    Computos, Estado, Vm, actividad_por_http, configurar_por_http, listo_por_http,
    terminar_por_http,
};
use crate::especificacion::{Computo, Datos, especificacion, iso};
use crate::llaves::Llave;
use ore_core::json::Json;
use ore_entrada::http::{Plazos, pedir_con};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// La versión de la API de Docker (Docker Engine 24+).
const API: &str = "/v1.43";

/// El volumen que hace de pageserver.
pub const VOLUMEN: &str = "p5lab-almacen";

/// La etiqueta de lo que crea el laboratorio (`lab.sh abajo` lo barre por ella).
pub const ETIQUETA: &str = "ore.dev/laboratorio";

pub struct Docker {
    /// `host:puerto` de la API de Docker.
    pub api: String,
    /// La red de Docker del laboratorio.
    pub red: String,
    /// La imagen del cómputo de mentira.
    pub imagen: String,
    propia: Llave,
}

impl Docker {
    pub fn nuevo(api: &str, red: &str, imagen: &str, propia: Llave) -> Docker {
        Docker {
            api: api.into(),
            red: red.into(),
            imagen: imagen.into(),
            propia,
        }
    }

    fn pedir(
        &self,
        metodo: &str,
        camino: &str,
        cuerpo: Option<&Json>,
    ) -> Result<(u16, String), Fallo> {
        pedir_con(
            metodo,
            &self.api,
            &format!("{API}{camino}"),
            &[],
            cuerpo,
            Plazos {
                conectar: Duration::from_secs(3),
                responder: Duration::from_secs(30),
            },
        )
        .map_err(|e| Fallo::Reintentar(format!("la API de Docker: {e}")))
    }
}

fn texto_de<'a>(n: &'a ore_core::parse::Node, camino: &[&str]) -> Option<&'a str> {
    let mut nodo = n;
    for k in camino {
        nodo = nodo.get(k)?.1;
    }
    nodo.as_str()
}

impl Computos for Docker {
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
        let ahora = iso(segundos());
        especificacion(
            &Computo {
                nombre: vm,
                tenant,
                timeline,
                pageserver,
                safekeepers: &[],
                grupo,
                ahora: &ahora,
                replica,
                datos,
                cu_conexiones,
            },
            &self.propia,
            None,
        )
    }

    fn configurar(&self, vm: &str, ip_pod: &str, configuracion: &Json) -> Result<(), Fallo> {
        configurar_por_http(&self.propia, vm, ip_pod, configuracion)
    }

    fn estado(&self, vm: &str) -> Result<Option<Estado>, Fallo> {
        let (c, r) = self.pedir("GET", &format!("/containers/{vm}/json"), None)?;
        match c {
            404 => return Ok(None),
            200 => {}
            _ => return Err(Fallo::Reintentar(format!("inspeccionar {vm}: {c}"))),
        }
        let n = ore_core::parse::parse(&r)
            .map_err(|_| Fallo::Reintentar(format!("la inspección de {vm} no analiza")))?;
        let fase = match texto_de(&n, &["State", "Status"]) {
            Some("running") => "Running",
            Some("exited") | Some("dead") => "Failed",
            _ => "Pending",
        };
        let ip = texto_de(&n, &["NetworkSettings", "Networks", &self.red, "IPAddress"])
            .filter(|ip| !ip.is_empty())
            .map(str::to_string);
        Ok(Some(Estado {
            fase: fase.into(),
            ip_pod: ip.clone(),
            ip_overlay: ip,
        }))
    }

    fn runner_vivo(&self, vm: &str) -> Result<bool, Fallo> {
        // `DELETE ?force=true` es síncrono: si ya no está, no queda nada con su nombre.
        Ok(self.estado(vm)?.is_some())
    }

    fn crear(&self, vm: &Vm, configuracion: &str) -> Result<(), Fallo> {
        let etiquetas: BTreeMap<String, Json> = [
            (ETIQUETA, "p5lab"),
            ("ore.dev/organizacion", vm.organizacion),
            ("ore.dev/proyecto", vm.proyecto),
            ("ore.dev/endpoint", vm.endpoint),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), Json::s(v)))
        .collect();
        let cuerpo = Json::obj([
            ("Image", Json::s(&self.imagen)),
            ("Hostname", Json::s(vm.nombre)),
            (
                "Env",
                Json::Arr(vec![
                    Json::s(format!("COMPUTE_ID={}", vm.nombre)),
                    Json::s(format!("CONFIG_JSON={configuracion}")),
                ]),
            ),
            ("Labels", Json::Obj(etiquetas)),
            (
                "HostConfig",
                Json::obj([
                    ("NetworkMode", Json::s(&self.red)),
                    (
                        "Mounts",
                        Json::Arr(vec![Json::obj([
                            ("Type", Json::s("volume")),
                            ("Source", Json::s(VOLUMEN)),
                            ("Target", Json::s("/almacen")),
                        ])]),
                    ),
                ]),
            ),
        ]);
        let (c, r) = self.pedir(
            "POST",
            &format!("/containers/create?name={}", vm.nombre),
            Some(&cuerpo),
        )?;
        // 409: ya existe (idempotente).
        if c != 201 && c != 409 {
            return Err(Fallo::Reintentar(format!(
                "crear {}: {c} {}",
                vm.nombre,
                r.chars().take(200).collect::<String>()
            )));
        }
        match self.pedir("POST", &format!("/containers/{}/start", vm.nombre), None)? {
            (204 | 304, _) => Ok(()),
            (c, r) => Err(Fallo::Reintentar(format!(
                "arrancar {}: {c} {}",
                vm.nombre,
                r.chars().take(200).collect::<String>()
            ))),
        }
    }

    fn listo(&self, vm: &str, ip_pod: &str) -> Result<bool, Fallo> {
        listo_por_http(&self.propia, vm, ip_pod)
    }

    fn actividad(&self, vm: &str, ip_pod: &str) -> Result<Option<String>, Fallo> {
        actividad_por_http(&self.propia, vm, ip_pod)
    }

    fn terminar(&self, vm: &str, ip_pod: &str) -> Result<Option<String>, Fallo> {
        terminar_por_http(&self.propia, vm, ip_pod)
    }

    fn borrar(&self, vm: &str) -> Result<(), Fallo> {
        match self.pedir("DELETE", &format!("/containers/{vm}?force=true"), None)? {
            (204 | 404, _) => Ok(()),
            (c, r) => Err(Fallo::Reintentar(format!(
                "borrar {vm}: {c} {}",
                r.chars().take(200).collect::<String>()
            ))),
        }
    }
}

/// El almacenamiento de mentira: todo existe; los datos, en el volumen.
pub struct AlmacenDeMentira {
    /// Donde está montado el volumen [`VOLUMEN`] en este proceso.
    pub raiz: PathBuf,
}

impl Almacen for AlmacenDeMentira {
    fn asegurar_tenant(&self, _: &str) -> Result<(), Fallo> {
        Ok(())
    }
    fn asegurar_timeline(&self, _: &str, _: &str, _: Option<Origen>) -> Result<(), Fallo> {
        Ok(())
    }
    fn borrar_tenant(&self, tenant: &str) -> Result<(), Fallo> {
        borrar_dir(self.raiz.join(tenant))
    }
    fn borrar_timeline(&self, tenant: &str, timeline: &str) -> Result<(), Fallo> {
        borrar_dir(self.raiz.join(tenant).join(timeline))
    }
    fn lsn_en_instante(&self, _: &str, _: &str, _: &str) -> Result<String, Fallo> {
        Err(Fallo::Definitivo(
            "el laboratorio no tiene historia: una rama en un instante es del clúster".into(),
        ))
    }
    fn pageserver_de(&self, _: &str) -> Result<String, Fallo> {
        Ok("host=almacen-de-mentira port=0".into())
    }
}

fn borrar_dir(d: PathBuf) -> Result<(), Fallo> {
    match std::fs::remove_dir_all(&d) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Fallo::Reintentar(format!("borrar {}: {e}", d.display()))),
    }
}

fn segundos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
