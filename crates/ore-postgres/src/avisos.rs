//! **Los avisos del almacenamiento** (0058 P4·5): sustituye a `avisos`, el
//! receptor de juguete de P2.
//!
//! El `storage_controller` avisa al plano de control (`--control-plane-url`)
//! cada vez que un tenant pasa a otro pageserver —uno cae y lo mueve, se drena un
//! nodo, se rebalancea— y también al crearlo:
//!
//! ```text
//!   PUT /avisos/notify-attach       {"tenant_id", "shards": [{"node_id", "shard_number"}], …}
//!   PUT /avisos/notify-safekeepers  (no se usa: los safekeepers no los gestiona el controller)
//!   Authorization: Bearer <JWT firmado con la del almacenamiento, scope `infra`>
//! ```
//!
//! A cada endpoint vivo de ese tenant se le manda su especificación con el
//! pageserver nuevo por `compute_ctl /configure`, sin reiniciarlo. Si alguno no
//! la aplica, **503**: el controller lo apunta como pendiente y lo reintenta, que
//! es su contrato. Sin esto, un cómputo seguiría pidiendo páginas a un
//! pageserver que ya no tiene su tenant.

use crate::almacen::Almacen;
use crate::base::mal;
use crate::computos::Computos;
use crate::llaves::Publica;
use crate::reconciliador::datos_de;
use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};
use postgres::Client;
use std::sync::Arc;

pub struct Avisos {
    pub almacen: Arc<dyn Almacen>,
    pub computos: Arc<dyn Computos>,
    /// La pública del almacenamiento: los avisos vienen firmados con su privada.
    pub infra: Publica,
}

impl Avisos {
    pub fn atender(&self, c: &mut Client, p: &Peticion, resto: &[&str]) -> Respuesta {
        if p.metodo != "PUT" && p.metodo != "POST" {
            return Respuesta::error(405, "los avisos son PUT");
        }
        let token = p.cabeceras.get("authorization").and_then(|a| {
            a.strip_prefix("Bearer ")
                .or_else(|| a.strip_prefix("bearer "))
        });
        let Some(token) = token else {
            return Respuesta::error(401, "los avisos traen el token del almacenamiento");
        };
        match self.infra.verificar(token) {
            Ok(cuerpo) if cuerpo.get("scope").and_then(|(_, v)| v.as_str()) == Some("infra") => {}
            Ok(_) => return Respuesta::error(401, "el token no es de scope `infra`"),
            Err(e) => return Respuesta::error(401, format!("el token no vale: {e}")),
        }
        match resto {
            ["notify-safekeepers"] => Respuesta::ok(Json::obj([])),
            ["notify-attach"] => self.attach(c, &p.cuerpo),
            _ => Respuesta::error(404, "no hay ningún aviso así"),
        }
    }

    fn attach(&self, c: &mut Client, cuerpo: &str) -> Respuesta {
        let Ok(n) = ore_core::parse::parse(cuerpo) else {
            return Respuesta::error(400, "el aviso no analiza");
        };
        let Some(tenant) = n
            .get("tenant_id")
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
        else {
            return Respuesta::error(400, "el aviso no dice `tenant_id`");
        };
        let filas = match c.query(
            &format!(
            "select e.vm, e.ip_pod, e.tipo = 'lectura', r.timeline, e.organizacion, e.proyecto, e.rama,
                    {}
               from plano.endpoint e
               join plano.proyecto p on p.organizacion = e.organizacion and p.id = e.proyecto
               join plano.rama r on r.organizacion = e.organizacion and r.proyecto = e.proyecto and r.id = e.rama
              where p.tenant = $1 and e.deseado = 'vivo' and e.observado = 'listo' and e.ip_pod is not null",
                crate::especificacion::CU_DE_LAS_CONEXIONES
            ),
            &[&tenant],
        ) {
            Ok(f) => f,
            Err(e) => return Respuesta::error(503, mal(e)),
        };
        if filas.is_empty() {
            eprintln!("aviso · tenant {tenant}: ningún cómputo vivo que reconfigurar");
            return Respuesta::ok(Json::obj([("reconfigurados", Json::Int(0))]));
        }
        let pageserver = match self.almacen.pageserver_de(&tenant) {
            Ok(p) => p,
            Err(e) => return Respuesta::error(503, e.motivo().to_string()),
        };
        let mut fallos = Vec::new();
        for f in &filas {
            let (vm, ip, lectura, timeline, org, proyecto, rama, cu): (
                String,
                String,
                bool,
                String,
                String,
                String,
                String,
                f64,
            ) = (
                f.get(0),
                f.get(1),
                f.get(2),
                f.get(3),
                f.get(4),
                f.get(5),
                f.get(6),
                f.get(7),
            );
            let r = datos_de(c, &org, &proyecto, &rama).and_then(|datos| {
                let cfg = self.computos.configuracion(
                    &vm,
                    &tenant,
                    &timeline,
                    &pageserver,
                    &proyecto,
                    lectura,
                    &datos,
                    cu,
                );
                self.computos.configurar(&vm, &ip, &cfg)
            });
            match r {
                Ok(()) => eprintln!("aviso · tenant {tenant}: {vm} → {pageserver}"),
                Err(e) => fallos.push(format!("{vm}: {}", e.motivo())),
            }
        }
        if fallos.is_empty() {
            Respuesta::ok(Json::obj([(
                "reconfigurados",
                Json::Int(filas.len() as i64),
            )]))
        } else {
            // El controller lo reintenta.
            Respuesta::error(503, fallos.join("; "))
        }
    }
}
