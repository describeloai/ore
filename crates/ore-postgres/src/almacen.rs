//! **El almacenamiento** (0058 P4·2): lo que `ore-postgres` le pide al
//! `storage_controller` y a los safekeepers. Es lo que hacía `tenant.sh` a mano.
//!
//! ```text
//!   tenant     POST   /v1/tenant                         {new_tenant_id}          controller, token `admin`
//!   timeline   POST   /v1/tenant/{t}/timeline            {new_timeline_id, …}     controller, token `admin`
//!   borrar     DELETE /v1/tenant/{t}  hasta 404                                   controller, token `admin`
//!              DELETE /v1/tenant/{t}  en CADA safekeeper (su WAL, local y GCS)    token `safekeeperdata`
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
}

impl Fallo {
    pub fn motivo(&self) -> &str {
        match self {
            Fallo::Reintentar(m) | Fallo::Definitivo(m) => m,
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
        for sk in &self.safekeepers {
            let (c, r) = self.pedir(sk, &self.safekeeperdata, "DELETE", &camino, None)?;
            if !(200..=299).contains(&c) && c != 404 {
                return Err(Fallo::Reintentar(format!(
                    "el safekeeper {sk} no borró el tenant ({c}): {}",
                    recorte(&r)
                )));
            }
        }
        Ok(())
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
}
