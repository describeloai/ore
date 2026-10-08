//! **El almacenamiento** (0058 P4·2): lo que `ore-postgres` le pide al
//! `storage_controller` y a los safekeepers. Es lo que hacía `tenant.sh` a mano.
//!
//! ```text
//!   tenant     POST   /v1/tenant                         {new_tenant_id}          controller, token `admin`
//!   timeline   POST   /v1/tenant/{t}/timeline            {new_timeline_id, …}     controller, token `admin`
//!   borrar     DELETE /v1/tenant/{t}  hasta 404                                   controller, token `admin`
//!              DELETE /v1/tenant/{t}  en CADA safekeeper (su WAL, local y GCS)    token `safekeeperdata`
//!   rama       DELETE /v1/tenant/{t}/timeline/{tl}  hasta 404, y en cada safekeeper
//!   instante   GET    /v1/tenant/{t}/timeline/{tl}/get_lsn_by_timestamp?timestamp=…  (el controller lo
//!              pasa al pageserver que lleva el tenant)
//!   dónde      GET    /control/v1/tenant/{t} (`node_attached`) y /control/v1/node: el pageserver del
//!              tenant, para la especificación de su cómputo (P4·3·1)
//! ```
//!
//! ⭐ Todo es **idempotente** visto desde aquí: «asegurar» un tenant que ya
//!   existe es éxito, y borrar uno que ya no está, también. El reconciliador
//!   repite sin miedo.
//!
//! ⚠️ Lo que responde el controller cuando algo ya existe, o cuando el tenant
//!   aún no está `Active` (409 «Timed out waiting … for tenant active state»,
//!   medido en P2·1), se distingue aquí: lo primero es éxito, lo segundo,
//!   reintentar.

use ore_core::json::Json;
use ore_entrada::http::{Plazos, pedir_con};
use std::path::Path;
use std::time::Duration;

/// Por qué no salió.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallo {
    /// Pasajero: el tenant aún no está activo, el controller no contesta… Se
    /// reintenta con espera.
    Reintentar(String),
    /// No va a salir repitiendo: una petición mal formada, un token que no vale.
    Definitivo(String),
    /// Va bien y lleva su tiempo (una VM arrancando, un runner yéndose): se
    /// vuelve a mirar en 2 s SIN gastar intentos, hasta un plazo total.
    Esperar(String),
}

impl Fallo {
    pub fn motivo(&self) -> &str {
        match self {
            Fallo::Reintentar(m) | Fallo::Definitivo(m) | Fallo::Esperar(m) => m,
        }
    }
}

/// De dónde sale una rama nueva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origen<'a> {
    pub timeline: &'a str,
    pub lsn: Option<&'a str>,
}

/// Lo que el reconciliador necesita del almacenamiento.
pub trait Almacen: Send + Sync {
    fn asegurar_tenant(&self, tenant: &str) -> Result<(), Fallo>;
    fn asegurar_timeline(
        &self,
        tenant: &str,
        timeline: &str,
        origen: Option<Origen>,
    ) -> Result<(), Fallo>;
    /// Hasta que el controller diga que no está, y después su WAL en cada
    /// safekeeper. `Ok` sólo cuando no queda nada.
    fn borrar_tenant(&self, tenant: &str) -> Result<(), Fallo>;
    /// Lo mismo para un timeline: el controller y su WAL en cada safekeeper.
    fn borrar_timeline(&self, tenant: &str, timeline: &str) -> Result<(), Fallo>;
    /// El LSN de un timeline en un instante (RFC 3339, UTC). Un instante anterior
    /// al timeline, o sin nada que encaje, es definitivo: repetir no lo arregla.
    fn lsn_en_instante(
        &self,
        tenant: &str,
        timeline: &str,
        instante: &str,
    ) -> Result<String, Fallo>;
    /// `host=… port=…` del pageserver que lleva el tenant. Un tenant partido en
    /// varios shards no se sirve todavía: es definitivo.
    fn pageserver_de(&self, tenant: &str) -> Result<String, Fallo>;
}

/// El de verdad: Neon, en `ore-pg`.
pub struct Neon {
    pub controlador: String,
    pub safekeepers: Vec<String>,
    admin: String,
    safekeeperdata: String,
}

/// Cuánto se espera a una respuesta. Crear un timeline espera al tenant
/// (el controller lo hace ~5 s antes de devolver 409), así que no es poco.
const PLAZO: Duration = Duration::from_secs(30);

impl Neon {
    /// Los tokens, de un directorio con un fichero por token (`admin`,
    /// `safekeeperdata`): el Secret montado. No se imprimen nunca.
    pub fn nuevo(
        controlador: &str,
        safekeepers: Vec<String>,
        llaves: &Path,
    ) -> Result<Neon, String> {
        let leer = |n: &str| {
            std::fs::read_to_string(llaves.join(n))
                .map(|t| t.trim().to_string())
                .map_err(|e| format!("el token `{n}` en `{}`: {e}", llaves.display()))
        };
        Ok(Neon {
            controlador: controlador.to_string(),
            safekeepers,
            admin: leer("admin")?,
            safekeeperdata: leer("safekeeperdata")?,
        })
    }

    fn pedir(
        &self,
        destino: &str,
        token: &str,
        metodo: &str,
        camino: &str,
        cuerpo: Option<&Json>,
    ) -> Result<(u16, String), Fallo> {
        let autorizacion = format!("Bearer {token}");
        pedir_con(
            metodo,
            destino,
            camino,
            &[("Authorization", &autorizacion)],
            cuerpo,
            Plazos {
                conectar: Duration::from_secs(3),
                responder: PLAZO,
            },
        )
        .map_err(|e| Fallo::Reintentar(format!("{destino}: {e}")))
    }
}

impl Almacen for Neon {
    fn asegurar_tenant(&self, tenant: &str) -> Result<(), Fallo> {
        let cuerpo = Json::obj([("new_tenant_id", Json::s(tenant))]);
        let (c, r) = self.pedir(
            &self.controlador,
            &self.admin,
            "POST",
            "/v1/tenant",
            Some(&cuerpo),
        )?;
        clasificar("crear el tenant", c, &r)
    }

    fn asegurar_timeline(
        &self,
        tenant: &str,
        timeline: &str,
        origen: Option<Origen>,
    ) -> Result<(), Fallo> {
        let mut campos = vec![
            ("new_timeline_id", Json::s(timeline)),
            ("pg_version", Json::Int(17)),
        ];
        if let Some(o) = origen {
            campos.push(("ancestor_timeline_id", Json::s(o.timeline)));
            if let Some(lsn) = o.lsn {
                campos.push(("ancestor_start_lsn", Json::s(lsn)));
            }
        }
        let (c, r) = self.pedir(
            &self.controlador,
            &self.admin,
            "POST",
            &format!("/v1/tenant/{tenant}/timeline"),
            Some(&Json::obj(campos)),
        )?;
        clasificar("crear el timeline", c, &r)
    }

    fn borrar_tenant(&self, tenant: &str) -> Result<(), Fallo> {
        let camino = format!("/v1/tenant/{tenant}");
        let (c, r) = self.pedir(&self.controlador, &self.admin, "DELETE", &camino, None)?;
        match c {
            404 => {}
            200..=299 => {
                return Err(Fallo::Reintentar(format!(
                    "el controller aún está borrando el tenant ({c})"
                )));
            }
            _ => return Err(clasificar("borrar el tenant", c, &r).unwrap_err()),
        }
        // El WAL: el controller no lo toca (P2·6). En cada safekeeper, local y GCS.
        self.borrar_en_los_safekeepers(&camino)
    }

    fn borrar_timeline(&self, tenant: &str, timeline: &str) -> Result<(), Fallo> {
        let camino = format!("/v1/tenant/{tenant}/timeline/{timeline}");
        let (c, r) = self.pedir(&self.controlador, &self.admin, "DELETE", &camino, None)?;
        match c {
            404 => {}
            // ⚠️ Medido (P4·2·3, en vivo): a un timeline que YA NO EXISTE el
            //   controller le contesta 200 con `null`, no 404 como a un tenant.
            //   Quien dice si se fue es el GET.
            200..=299 => {
                let (g, _) = self.pedir(&self.controlador, &self.admin, "GET", &camino, None)?;
                if g != 404 {
                    return Err(Fallo::Reintentar(format!(
                        "el controller aún está borrando el timeline ({c}, y el GET da {g})"
                    )));
                }
            }
            _ => return Err(clasificar("borrar el timeline", c, &r).unwrap_err()),
        }
        self.borrar_en_los_safekeepers(&camino)
    }

    fn lsn_en_instante(
        &self,
        tenant: &str,
        timeline: &str,
        instante: &str,
    ) -> Result<String, Fallo> {
        let camino = format!(
            "/v1/tenant/{tenant}/timeline/{timeline}/get_lsn_by_timestamp?timestamp={}",
            instante.replace(':', "%3A")
        );
        let (c, r) = self.pedir(&self.controlador, &self.admin, "GET", &camino, None)?;
        clasificar("el LSN del instante", c, &r)?;
        lsn_de_la_respuesta(&r)
    }

    fn pageserver_de(&self, tenant: &str) -> Result<String, Fallo> {
        let (c, r) = self.pedir(
            &self.controlador,
            &self.admin,
            "GET",
            &format!("/control/v1/tenant/{tenant}"),
            None,
        )?;
        clasificar("dónde está el tenant", c, &r)?;
        let nodo = nodo_del_tenant(&r)?;
        let (c, r) = self.pedir(
            &self.controlador,
            &self.admin,
            "GET",
            "/control/v1/node",
            None,
        )?;
        clasificar("los pageservers", c, &r)?;
        direccion_del_nodo(&r, &nodo)
    }
}

impl Neon {
    /// `host:5454` de cada safekeeper (la especificación los quiere por su
    /// puerto de Postgres, no por el HTTP con que se les borra).
    pub fn safekeepers_pg(&self) -> Vec<String> {
        self.safekeepers
            .iter()
            .map(|s| {
                let host = s.rsplit_once(':').map(|(h, _)| h).unwrap_or(s);
                format!("{}:5454", host.trim_end_matches('.'))
            })
            .collect()
    }

    fn borrar_en_los_safekeepers(&self, camino: &str) -> Result<(), Fallo> {
        for sk in &self.safekeepers {
            let (c, r) = self.pedir(sk, &self.safekeeperdata, "DELETE", camino, None)?;
            if !(200..=299).contains(&c) && c != 404 {
                return Err(Fallo::Reintentar(format!(
                    "el safekeeper {sk} no borró `{camino}` ({c}): {}",
                    recorte(&r)
                )));
            }
        }
        Ok(())
    }
}

/// De `GET /control/v1/tenant/{t}`: el nodo de su único shard.
pub fn nodo_del_tenant(cuerpo: &str) -> Result<String, Fallo> {
    let n = ore_core::parse::parse(cuerpo)
        .map_err(|_| Fallo::Reintentar(format!("respuesta que no analiza: {}", recorte(cuerpo))))?;
    let shards: Vec<_> = n
        .get("shards")
        .map(|(_, s)| s.items().to_vec())
        .unwrap_or_default();
    match shards.as_slice() {
        [uno] => uno
            .get("node_attached")
            .and_then(|(_, v)| v.as_str())
            .filter(|v| *v != "null" && !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| Fallo::Reintentar("el tenant aún no está en ningún pageserver".into())),
        [] => Err(Fallo::Reintentar(
            "el controller no da shards del tenant".into(),
        )),
        varios => Err(Fallo::Definitivo(format!(
            "el tenant tiene {} shards: un cómputo sobre varios aún no se sirve",
            varios.len()
        ))),
    }
}

/// De `GET /control/v1/node`: `host=… port=…` del nodo con ese id.
pub fn direccion_del_nodo(cuerpo: &str, nodo: &str) -> Result<String, Fallo> {
    let n = ore_core::parse::parse(cuerpo)
        .map_err(|_| Fallo::Reintentar(format!("respuesta que no analiza: {}", recorte(cuerpo))))?;
    let texto = |x: &ore_core::parse::Node, k: &str| {
        x.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
    };
    n.items()
        .iter()
        .find(|x| texto(x, "id").as_deref() == Some(nodo))
        .and_then(|x| {
            Some(format!(
                "host={} port={}",
                texto(x, "listen_pg_addr")?,
                texto(x, "listen_pg_port")?
            ))
        })
        .ok_or_else(|| Fallo::Reintentar(format!("el controller no conoce el nodo {nodo}")))
}

/// `{"lsn": "0/16B5A50", "kind": "present" | "future" | "past" | "nomatch"}`.
///
/// - `present`: el último registro de antes del instante.
/// - `future`: el instante es posterior a todo lo escrito; vale su último LSN
///   (la rama sale de la punta de entonces, que es la de ahora).
/// - `past` y `nomatch`: no hay datos tan atrás. Definitivo.
pub fn lsn_de_la_respuesta(cuerpo: &str) -> Result<String, Fallo> {
    let n = ore_core::parse::parse(cuerpo)
        .map_err(|_| Fallo::Reintentar(format!("respuesta que no analiza: {}", recorte(cuerpo))))?;
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).unwrap_or_default();
    match (campo("kind"), campo("lsn")) {
        ("present" | "future", lsn) if !lsn.is_empty() => Ok(lsn.to_string()),
        ("past" | "nomatch", _) => Err(Fallo::Definitivo(
            "no hay datos de la rama padre en ese instante: es anterior a ella o a lo que se guarda"
                .into(),
        )),
        _ => Err(Fallo::Reintentar(format!(
            "respuesta inesperada: {}",
            recorte(cuerpo)
        ))),
    }
}

/// De un código y un cuerpo del controller a éxito, reintentar o rendirse.
pub fn clasificar(que: &str, codigo: u16, cuerpo: &str) -> Result<(), Fallo> {
    let m = || format!("{que}: {codigo} {}", recorte(cuerpo));
    let dice = |s: &str| cuerpo.to_ascii_lowercase().contains(s);
    match codigo {
        200..=299 => Ok(()),
        // Ya estaba: es lo que se quería.
        409 if dice("already exists") || dice("exists") && !dice("active") => Ok(()),
        // El tenant aún no está activo, o el controller está ocupado: luego.
        409 | 429 | 500..=599 => Err(Fallo::Reintentar(m())),
        404 if dice("tenant") => Err(Fallo::Reintentar(m())),
        _ => Err(Fallo::Definitivo(m())),
    }
}

fn recorte(s: &str) -> String {
    s.chars().take(300).collect::<String>().replace('\n', " ")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn lo_que_dice_el_controller() {
        assert_eq!(clasificar("x", 201, ""), Ok(()));
        assert_eq!(
            clasificar("x", 409, r#"{"msg":"Timeline already exists"}"#),
            Ok(())
        );
        assert!(matches!(
            clasificar(
                "x",
                409,
                r#"{"msg":"Timed out waiting 5s for tenant active state"}"#
            ),
            Err(Fallo::Reintentar(_))
        ));
        assert!(matches!(
            clasificar("x", 503, ""),
            Err(Fallo::Reintentar(_))
        ));
        assert!(matches!(
            clasificar("x", 400, "bad"),
            Err(Fallo::Definitivo(_))
        ));
        assert!(matches!(
            clasificar("x", 401, ""),
            Err(Fallo::Definitivo(_))
        ));
    }

    #[test]
    fn el_pageserver_del_tenant() {
        let t = r#"{"tenant_id":"t","shards":[{"tenant_shard_id":"t","node_attached":1,"node_secondary":[]}]}"#;
        assert_eq!(nodo_del_tenant(t), Ok("1".into()));
        let nodos = r#"[{"id":1,"listen_pg_addr":"pageserver-0.ore-pg.svc.cluster.local","listen_pg_port":6400},
                        {"id":2,"listen_pg_addr":"pageserver-1","listen_pg_port":6400}]"#;
        assert_eq!(
            direccion_del_nodo(nodos, "1"),
            Ok("host=pageserver-0.ore-pg.svc.cluster.local port=6400".into())
        );
        assert!(matches!(
            direccion_del_nodo(nodos, "3"),
            Err(Fallo::Reintentar(_))
        ));
        assert!(matches!(
            nodo_del_tenant(r#"{"shards":[{"node_attached":1},{"node_attached":2}]}"#),
            Err(Fallo::Definitivo(_))
        ));
        assert!(matches!(
            nodo_del_tenant(r#"{"shards":[{"node_attached":null}]}"#),
            Err(Fallo::Reintentar(_))
        ));
    }

    #[test]
    fn el_lsn_de_un_instante() {
        assert_eq!(
            lsn_de_la_respuesta(r#"{"lsn":"0/16B5A50","kind":"present"}"#),
            Ok("0/16B5A50".into())
        );
        assert_eq!(
            lsn_de_la_respuesta(r#"{"lsn":"0/2000000","kind":"future"}"#),
            Ok("0/2000000".into())
        );
        assert!(matches!(
            lsn_de_la_respuesta(r#"{"lsn":"0/0","kind":"past"}"#),
            Err(Fallo::Definitivo(_))
        ));
        assert!(matches!(
            lsn_de_la_respuesta(r#"{"lsn":"0/0","kind":"nomatch"}"#),
            Err(Fallo::Definitivo(_))
        ));
    }
}
