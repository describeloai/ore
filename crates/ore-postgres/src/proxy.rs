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
//!       → {"address": "10.100.128.7:5432", "aux": {endpoint_id, project_id, branch_id, compute_id}}
//!   Authorization: Bearer <el token del proxy>
//! ```
//!
//! - **`endpointish`** es la primera etiqueta del SNI: el nombre de la VM
//!   (`ep-<20 hex>`), con `-pooler` si se entra por el pool (P5·5): entonces
//!   la dirección es la del pgbouncer de la VM (6432, modo `transaction`, el
//!   de la imagen de cómputo de Neon), no la de Postgres (5432). El secreto es
//!   el mismo: pgbouncer hace el SCRAM con las claves que le pasa el proxy.
//! - **`role_secret`** es el verificador SCRAM que se guardó al crear el rol
//!   (P4·4): con él el proxy hace el SCRAM con el cliente sin conocer nunca la
//!   contraseña.
//! - **Quién entra** (P5·6), por proyecto: `allowed_ips` (vacía = todas),
//!   `block_public_connections` y `rate_limits.connection_attempts` por
//!   protocolo, la cubeta que corta la fuerza bruta.
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
use std::sync::Mutex;
use std::time::Duration;

/// El puerto de Postgres en el cómputo (la especificación, `especificacion.rs`):
/// el de Neon en sus VMs, al que apunta el pgbouncer de la imagen.
pub const PUERTO: u16 = 5432;

/// El pgbouncer de la imagen de cómputo (`compute/etc/pgbouncer.ini`).
pub const PUERTO_POOL: u16 = 6432;

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

/// Una cubeta del limitador del proxy (`LeakyBucketSetting`).
fn cubeta(rps: i64, rafaga: i64) -> Json {
    Json::obj([("rps", Json::Int(rps)), ("burst", Json::Int(rafaga))])
}

/// Comparar sin dar pistas por el tiempo.
fn iguales(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// `ep-…-pooler` → (`ep-…`, el puerto del pool): el pool es otra puerta del mismo cómputo.
pub fn vm_de(endpointish: &str) -> (&str, u16) {
    match endpointish.strip_suffix("-pooler") {
        Some(vm) => (vm, PUERTO_POOL),
        None => (endpointish, PUERTO),
    }
}

/// P6·4 · Lo más que `wake_compute` espera a que un cómputo esté listo. El proxy no tiene
/// tope (P6·0); el cliente, el suyo. Una VM en frío tarda ~35 s (P3): de sobra.
pub const PLAZO_DESPERTAR: Duration = Duration::from_secs(120);

/// Cada cuánto mira, mientras espera.
const MIRAR_CADA: Duration = Duration::from_millis(200);

/// Lo que ve `wake_compute` en una mirada.
enum Paso {
    /// Contestar ya (la dirección, o que no existe).
    Contestar(Respuesta),
    /// Aún no: arrancando, despertando o durmiéndose.
    Esperar,
}

impl Proxy {
    pub fn atender(
        &self,
        base: &Mutex<Client>,
        url: Option<&str>,
        p: &Peticion,
        resto: &[&str],
    ) -> Respuesta {
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
        let (vm, puerto) = vm_de(endpoint);
        match resto {
            ["get_endpoint_access_control"] => {
                let Some(rol) = p.consulta.get("role") else {
                    return fallo(404, "RESOURCE_NOT_FOUND", "falta `role`");
                };
                con_base(base, url, |c| self.acceso(c, vm, rol)).unwrap_or_else(|r| r)
            }
            ["wake_compute"] => self.despertar(base, url, vm, puerto),
            _ => Respuesta::error(404, "el proxy no pregunta eso"),
        }
    }

    fn acceso(&self, c: &mut Client, vm: &str, rol: &str) -> Respuesta {
        match c.query_opt(
            "select r.verificador, p.tenant, e.organizacion, p.ips_permitidas, p.bloquear_publico,
                    (p.limites->'tcp'->>'por_segundo')::bigint, (p.limites->'tcp'->>'rafaga')::bigint,
                    (p.limites->'ws'->>'por_segundo')::bigint, (p.limites->'ws'->>'rafaga')::bigint,
                    (p.limites->'http'->>'por_segundo')::bigint, (p.limites->'http'->>'rafaga')::bigint
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
                // P5·6: quién entra. Vacía = todas; las entradas las validó la API.
                (
                    "allowed_ips",
                    Json::Arr(
                        f.get::<_, Vec<String>>(3)
                            .into_iter()
                            .map(Json::s)
                            .collect(),
                    ),
                ),
                ("block_public_connections", Json::Bool(f.get(4))),
                (
                    "rate_limits",
                    Json::obj([(
                        "connection_attempts",
                        Json::obj([
                            ("tcp", cubeta(f.get(5), f.get(6))),
                            ("ws", cubeta(f.get(7), f.get(8))),
                            ("http", cubeta(f.get(9), f.get(10))),
                        ]),
                    )]),
                ),
            ])),
            // Ni el endpoint ni el rol se distinguen: no se le dice a nadie qué existe.
            Ok(None) => fallo(404, "RESOURCE_NOT_FOUND", "no hay tal endpoint o tal rol"),
            Err(e) => fallo(503, "RUNNING_OPERATIONS", &mal(e)),
        }
    }

    /// P6·4 · **Despertar**: listo, su dirección; dormido, una operación `despertar-endpoint`
    /// (el índice de una por proyecto hace que mil conexiones a la vez sean UN despertar) y
    /// esperar; arrancando o durmiéndose, esperar. El cliente no ve más que la espera; pasado
    /// [`PLAZO_DESPERTAR`], «reintenta».
    fn despertar(
        &self,
        base: &Mutex<Client>,
        url: Option<&str>,
        vm: &str,
        puerto: u16,
    ) -> Respuesta {
        let desde = std::time::Instant::now();
        loop {
            match con_base(base, url, |c| mirar(c, vm, puerto)) {
                Ok(Paso::Contestar(r)) | Err(r) => return r,
                Ok(Paso::Esperar) if desde.elapsed() >= PLAZO_DESPERTAR => {
                    return fallo(503, "RUNNING_OPERATIONS", "el cómputo aún no está listo");
                }
                Ok(Paso::Esperar) => std::thread::sleep(MIRAR_CADA),
            }
        }
    }
}

/// Una mirada: lo que hay, y si está dormido, pedir que despierte.
fn mirar(c: &mut Client, vm: &str, puerto: u16) -> Paso {
    let fila = match c.query_opt(
        "select e.observado, e.direccion, p.tenant, e.rama, e.organizacion, e.proyecto, e.id
           from plano.endpoint e
           join plano.proyecto p on p.organizacion = e.organizacion and p.id = e.proyecto
          where e.vm = $1 and e.deseado = 'vivo' and p.tenant is not null",
        &[&vm],
    ) {
        Ok(f) => f,
        Err(e) => return Paso::Contestar(fallo(503, "RUNNING_OPERATIONS", &mal(e))),
    };
    let Some(f) = fila else {
        return Paso::Contestar(fallo(404, "ENDPOINT_NOT_FOUND", "no hay tal endpoint"));
    };
    let (observado, direccion): (String, Option<String>) = (f.get(0), f.get(1));
    match (observado.as_str(), direccion) {
        ("listo", Some(dir)) => Paso::Contestar(Respuesta::ok(Json::obj([
            ("address", Json::s(format!("{dir}:{puerto}"))),
            (
                "aux",
                Json::obj([
                    ("endpoint_id", Json::s(vm)),
                    ("project_id", Json::s(f.get::<_, String>(2))),
                    ("branch_id", Json::s(f.get::<_, String>(3))),
                    ("compute_id", Json::s(vm)),
                    ("cold_start_info", Json::s("warm")),
                ]),
            ),
        ]))),
        ("dormido", _) => {
            let (org, p, rama, id): (String, String, String, String) =
                (f.get(4), f.get(5), f.get(3), f.get(6));
            match c.execute(
                "insert into plano.operacion (id, organizacion, proyecto, tipo, celda, rama, endpoint)
                 values ('op_' || replace(gen_random_uuid()::text, '-', ''), $1, $2,
                         'despertar-endpoint', 'proxy', $3, $4)",
                &[&org, &p, &rama, &id],
            ) {
                // Pedido, o ya hay una en curso (otro despertar, u otra cosa que acabará antes).
                Ok(_) => Paso::Esperar,
                Err(e) if crate::base::choca(&e, "una_en_curso_por_proyecto") => Paso::Esperar,
                Err(e) => Paso::Contestar(fallo(503, "RUNNING_OPERATIONS", &mal(e))),
            }
        }
        _ => Paso::Esperar,
    }
}

/// La base, tomada sólo para esto (y reabierta si se cayó).
fn con_base<T>(
    base: &Mutex<Client>,
    url: Option<&str>,
    f: impl FnOnce(&mut Client) -> T,
) -> Result<T, Respuesta> {
    let Ok(mut c) = base.lock() else {
        return Err(Respuesta::error(500, "la conexión quedó envenenada"));
    };
    if c.is_closed()
        && let Some(Ok(nueva)) = url.map(crate::base::conectar)
    {
        *c = nueva;
    }
    Ok(f(&mut c))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_pool_es_el_mismo_computo() {
        assert_eq!(
            vm_de("ep-0123456789abcdef0123-pooler"),
            ("ep-0123456789abcdef0123", 6432)
        );
        assert_eq!(
            vm_de("ep-0123456789abcdef0123"),
            ("ep-0123456789abcdef0123", 5432)
        );
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
