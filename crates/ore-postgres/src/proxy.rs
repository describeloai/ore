//! **Lo que el proxy le pregunta al plano de control** (0058 P5·1).
//!
//! El proxy de Neon (`--auth-backend control-plane --auth-endpoint
//! http://ore-postgres…:8100/proxy`) hace dos preguntas, las del commit fijado
//! (`proxy/src/control_plane/client/cplane_proxy_v1.rs`):
//!
//! ```text
//!   GET /proxy/get_endpoint_access_control?endpointish=ep-…&role=…
//!       → {"role_secret": "SCRAM-SHA-256$…", "project_id", "allowed_ips"?…}
//!   GET /proxy/wake_compute?endpointish=ep-…
//!       → {"address": "10.100.128.7:55433", "aux": {endpoint_id, project_id, branch_id, compute_id}}
//!   Authorization: Bearer <el token del proxy>
//! ```
//!
//! - **`endpointish`** es la primera etiqueta del SNI: el nombre de la VM
//!   (`ep-<20 hex>`), con `-pooler` si se entra por el pool.
//! - **`role_secret`** es el verificador SCRAM que se guardó al crear el rol
//!   (P4·4): con él el proxy hace el SCRAM con el cliente sin conocer nunca la
//!   contraseña.
//! - **`project_id` es el tenant**, no el id del proyecto: el proxy agrupa por
//!   él lo que guarda y lo que olvida ([`crate::olvidar`]), y el id del
//!   proyecto se repite entre organizaciones (el tenant no). `account_id` es la
//!   organización.
//! - **Los errores, con la forma de Neon** (`status.details.error_info.reason`):
//!   el proxy lee `*_NOT_FOUND` como «esa credencial no vale» y
//!   `RUNNING_OPERATIONS` como «reintenta».
//!
//! ⛔ El token del proxy es suyo (un Secret que sólo montan él y este proceso):
//!   ni una celda ni el controller entran aquí.

use crate::base::mal;
use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};
use postgres::Client;

/// El puerto de Postgres en el cómputo (la especificación, `especificacion.rs`).
const PUERTO: u16 = 55433;

pub struct Proxy {
    /// El token que trae el proxy, leído de su Secret al arrancar.
    pub token: String,
}

/// Un error como lo espera el proxy de Neon.
fn fallo(codigo: u16, razon: &str, mensaje: &str) -> Respuesta {
    let mut detalles = vec![("error_info", Json::obj([("reason", Json::s(razon))]))];
    if razon == "RUNNING_OPERATIONS" {
        detalles.push((
            "retry_info",
            Json::obj([("retry_delay_ms", Json::Int(1000))]),
        ));
    }
    Respuesta {
        codigo,
        cuerpo: Json::obj([
            ("error", Json::s(mensaje)),
            (
                "status",
                Json::obj([
                    (
                        "code",
                        Json::s(if codigo == 404 {
                            "NOT_FOUND"
                        } else {
                            "UNAVAILABLE"
                        }),
                    ),
                    ("message", Json::s(mensaje)),
                    ("details", Json::obj(detalles)),
                ]),
            ),
        ]),
    }
}

/// Comparar sin dar pistas por el tiempo.
fn iguales(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// `ep-…-pooler` → `ep-…`: el pool es otra puerta del mismo cómputo.
pub fn vm_de(endpointish: &str) -> &str {
    endpointish.strip_suffix("-pooler").unwrap_or(endpointish)
}

impl Proxy {
    pub fn atender(&self, c: &mut Client, p: &Peticion, resto: &[&str]) -> Respuesta {
        let token = p.cabeceras.get("authorization").and_then(|a| {
            a.strip_prefix("Bearer ")
                .or_else(|| a.strip_prefix("bearer "))
        });
        if !token.is_some_and(|t| iguales(t.trim().as_bytes(), self.token.as_bytes())) {
            return Respuesta::error(401, "esto es sólo para el proxy");
        }
        if p.metodo != "GET" {
            return Respuesta::error(405, "el proxy sólo pregunta");
        }
        let Some(endpoint) = p.consulta.get("endpointish") else {
            return fallo(404, "ENDPOINT_NOT_FOUND", "falta `endpointish`");
        };
        let vm = vm_de(endpoint);
        match resto {
            ["get_endpoint_access_control"] => {
                let Some(rol) = p.consulta.get("role") else {
                    return fallo(404, "RESOURCE_NOT_FOUND", "falta `role`");
                };
                self.acceso(c, vm, rol)
            }
            ["wake_compute"] => self.despertar(c, vm),
            _ => Respuesta::error(404, "el proxy no pregunta eso"),
        }
    }

    fn acceso(&self, c: &mut Client, vm: &str, rol: &str) -> Respuesta {
        match c.query_opt(
            "select r.verificador, p.tenant, e.organizacion
               from plano.endpoint e
               join plano.proyecto p on p.organizacion = e.organizacion and p.id = e.proyecto
               join plano.rol r on r.organizacion = e.organizacion and r.proyecto = e.proyecto
                               and r.rama = e.rama and r.nombre = $2 and r.deseado = 'vivo'
              where e.vm = $1 and e.deseado = 'vivo' and p.tenant is not null",
            &[&vm, &rol],
        ) {
            Ok(Some(f)) => Respuesta::ok(Json::obj([
                ("role_secret", Json::s(f.get::<_, String>(0))),
                ("project_id", Json::s(f.get::<_, String>(1))),
                ("account_id", Json::s(f.get::<_, String>(2))),
            ])),
            // Ni el endpoint ni el rol se distinguen: no se le dice a nadie qué existe.
            Ok(None) => fallo(404, "RESOURCE_NOT_FOUND", "no hay tal endpoint o tal rol"),
            Err(e) => fallo(503, "RUNNING_OPERATIONS", &mal(e)),
        }
    }

    fn despertar(&self, c: &mut Client, vm: &str) -> Respuesta {
        let fila = match c.query_opt(
            "select e.observado, e.ip_pod, e.direccion, p.tenant, e.rama
               from plano.endpoint e
               join plano.proyecto p on p.organizacion = e.organizacion and p.id = e.proyecto
              where e.vm = $1 and e.deseado = 'vivo' and p.tenant is not null",
            &[&vm],
        ) {
            Ok(f) => f,
            Err(e) => return fallo(503, "RUNNING_OPERATIONS", &mal(e)),
        };
        let Some(f) = fila else {
            return fallo(404, "ENDPOINT_NOT_FOUND", "no hay tal endpoint");
        };
        let (observado, direccion): (String, Option<String>) = (f.get(0), f.get(2));
        // P5: el cómputo ya está encendido o arrancando; despertarlo de verdad es P6.
        let Some(dir) = direccion.filter(|_| observado == "listo") else {
            return fallo(503, "RUNNING_OPERATIONS", "el cómputo está arrancando");
        };
        Respuesta::ok(Json::obj([
            ("address", Json::s(format!("{dir}:{PUERTO}"))),
            (
                "aux",
                Json::obj([
                    ("endpoint_id", Json::s(vm)),
                    ("project_id", Json::s(f.get::<_, String>(3))),
                    ("branch_id", Json::s(f.get::<_, String>(4))),
                    ("compute_id", Json::s(vm)),
                    ("cold_start_info", Json::s("warm")),
                ]),
            ),
        ]))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_pool_es_el_mismo_computo() {
        assert_eq!(
            vm_de("ep-0123456789abcdef0123-pooler"),
            "ep-0123456789abcdef0123"
        );
        assert_eq!(vm_de("ep-0123456789abcdef0123"), "ep-0123456789abcdef0123");
    }

    #[test]
    fn el_token_se_compara_entero() {
        assert!(iguales(b"abc", b"abc"));
        assert!(!iguales(b"abc", b"abd"));
        assert!(!iguales(b"abc", b"abcd"));
    }

    #[test]
    fn los_errores_tienen_la_forma_de_neon() {
        let r = fallo(404, "RESOURCE_NOT_FOUND", "x");
        assert_eq!(
            r.cuerpo.jcs(),
            r#"{"error":"x","status":{"code":"NOT_FOUND","details":{"error_info":{"reason":"RESOURCE_NOT_FOUND"}},"message":"x"}}"#
        );
        let r = fallo(503, "RUNNING_OPERATIONS", "y");
        assert!(
            r.cuerpo
                .jcs()
                .contains(r#""retry_info":{"retry_delay_ms":1000}"#)
        );
    }
}
