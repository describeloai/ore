//! **ORE Serverless Postgres desde la celda** (0058 P4·6): `/v1/postgres/…`.
//!
//! `ore-serve` no guarda nada de Postgres: decide si quien pide puede, y se lo
//! pasa a `ore-postgres` (el plano de control, en `ore-pg`) con el token de la
//! celda —Workload Identity, audiencia `ore-postgres`—, que le dice de qué
//! organización es. Lo que contesta, tal cual.
//!
//! Quién puede qué (las potestades de la 051 de `iam`; las cuatro, por defecto,
//! a todo miembro —el estándar: Databricks y Neon dejan crear a todos—; la
//! organización las quita a quien quiera con sus roles):
//!
//! ```text
//!   GET  …                                   postgres:ver
//!   POST /proyectos                          postgres:crear   (su dueño, el de `quien`)
//!   …/roles…  (crear, contraseña, borrar)    postgres:usar
//!   lo demás dentro de un proyecto           su dueño, o postgres:gestionar
//! ```
//!
//! ⛔ **Desde un puesto, nada** (ni leer): un producto no sabe del otro. Lo que
//!   corre en un puesto es de Code Repositories; Postgres se gestiona con el
//!   token de la persona.

use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::Identidad;
use std::sync::OnceLock;
use std::time::Duration;

/// Dónde escucha `ore-postgres` (`host:puerto`). Sin él, 503.
pub const ENTORNO: &str = "ORE_POSTGRES_DIRECCION";
/// Un token fijo en un fichero, en lugar del de Workload Identity (pruebas).
pub const TESTIGO: &str = "ORE_POSTGRES_TESTIGO";
/// La audiencia del token de la celda que acepta `ore-postgres`.
const AUDIENCIA: &str = "ore-postgres";

/// Lo que pide, a efectos de quién puede.
#[derive(Debug, PartialEq)]
pub(crate) enum Que<'a> {
    Ver,
    Crear,
    Usar,
    /// Dentro de un proyecto: su dueño, o quien tenga `postgres:gestionar`.
    Gestionar(&'a str),
}

impl Que<'_> {
    fn potestad(&self) -> &'static str {
        match self {
            Que::Ver => "postgres:ver",
            Que::Crear => "postgres:crear",
            Que::Usar => "postgres:usar",
            Que::Gestionar(_) => "postgres:gestionar",
        }
    }
}

/// De método y camino (sin `/v1/postgres`) a lo que se pide. `None`: no es
/// una ruta de `ore-postgres` (404, sin preguntar a nadie).
pub(crate) fn que_pide<'a>(metodo: &str, resto: &[&'a str]) -> Option<Que<'a>> {
    match (metodo, resto) {
        ("GET", ["proyectos", ..] | ["operaciones", _]) => Some(Que::Ver),
        ("POST", ["proyectos"]) => Some(Que::Crear),
        ("POST" | "DELETE", ["proyectos", _, "ramas", _, "roles", ..]) => Some(Que::Usar),
        ("POST" | "DELETE", ["proyectos", p, ..]) => Some(Que::Gestionar(p)),
        _ => None,
    }
}

fn credencial() -> &'static dyn ore_acceso::Credencial {
    static C: OnceLock<Box<dyn ore_acceso::Credencial>> = OnceLock::new();
    C.get_or_init(
        || match std::env::var(TESTIGO).ok().filter(|f| !f.is_empty()) {
            Some(f) => Box::new(ore_acceso::Fija(
                std::fs::read_to_string(&f)
                    .map(|t| t.trim().to_string())
                    .unwrap_or_default(),
            )),
            None => Box::new(ore_acceso::Metadatos::nuevo(AUDIENCIA)),
        },
    )
    .as_ref()
}

fn no_contesta(m: impl std::fmt::Display) -> Respuesta {
    Respuesta {
        codigo: 503,
        cuerpo: Json::obj([
            ("error", Json::s(format!("ore-postgres: {m}"))),
            ("reintentar", Json::Bool(true)),
        ]),
    }
}

/// Una petición a `ore-postgres` con el token de la celda: su código y su cuerpo.
fn pedir(metodo: &str, camino: &str, cuerpo: Option<&Json>) -> Result<(u16, String), Respuesta> {
    let Some(direccion) = std::env::var(ENTORNO).ok().filter(|d| !d.is_empty()) else {
        return Err(no_contesta(format!(
            "esta celda no sabe dónde está (`{ENTORNO}`)"
        )));
    };
    let token = credencial()
        .token()
        .map_err(|e| no_contesta(format!("sin el token de la celda: {e}")))?;
    let autorizacion = format!("Bearer {token}");
    ore_entrada::http::pedir_con(
        metodo,
        &direccion,
        camino,
        &[("Authorization", &autorizacion)],
        cuerpo,
        ore_entrada::http::Plazos {
            conectar: Duration::from_secs(3),
            responder: Duration::from_secs(30),
        },
    )
    .map_err(no_contesta)
}

/// Lo que contestó, tal cual (si no es JSON, envuelto).
fn tal_cual(codigo: u16, texto: String) -> Respuesta {
    let t = texto.trim_start();
    let cuerpo = if (t.starts_with('{') || t.starts_with('[')) && ore_core::parse::parse(t).is_ok()
    {
        Json::Crudo(texto)
    } else {
        Json::obj([("error", Json::s(texto))])
    };
    Respuesta { codigo, cuerpo }
}

/// El `dueno` de un proyecto, del cuerpo de `GET /proyectos/{p}`.
fn dueno_del_proyecto(texto: &str) -> Option<String> {
    ore_core::parse::parse(texto)
        .ok()?
        .get("dueno")
        .and_then(|(_, v)| v.as_str())
        .map(str::to_string)
}

impl Servidor {
    pub(crate) fn postgres(&self, p: &Peticion, sujeto: &Identidad, resto: &[&str]) -> Respuesta {
        if crate::puestos::es_agente(sujeto) {
            return Respuesta::error(
                403,
                "Postgres no se gestiona desde un puesto: con el token de la persona",
            );
        }
        let Some(que) = que_pide(&p.metodo, resto) else {
            return Respuesta::error(
                404,
                format!("no hay `{} /v1/postgres/{}`", p.metodo, resto.join("/")),
            );
        };
        let camino = format!("/v1/postgres/{}", resto.join("/"));
        let ruta = format!("{} {camino}", p.metodo);
        let mut cuerpo = None;
        let decision = match &que {
            Que::Ver | Que::Usar => match self.exigir(sujeto, que.potestad(), &ruta) {
                Ok(d) => d,
                Err(r) => return r,
            },
            Que::Crear => {
                let d = match self.exigir(sujeto, que.potestad(), &ruta) {
                    Ok(d) => d,
                    Err(r) => return r,
                };
                // El dueño lo pone ORE, el de `quien`; el que traiga el cuerpo, no.
                let dueno = match self.dueno_de_quien_crea(sujeto) {
                    Ok(d) => d,
                    Err(r) => return r,
                };
                let Ok(n) = ore_core::parse::parse(&p.cuerpo) else {
                    return Respuesta::error(400, "el cuerpo no es JSON");
                };
                let Json::Obj(mut m) = Json::de_node_fiel(&n) else {
                    return Respuesta::error(400, "el cuerpo es un objeto: {\"id\": …}");
                };
                m.insert("dueno".into(), Json::s(dueno));
                cuerpo = Some(Json::Obj(m));
                d
            }
            Que::Gestionar(proyecto) => {
                let dueno = match self.dueno_de_quien_crea(sujeto) {
                    Ok(d) => d,
                    Err(r) => return r,
                };
                let (c, t) = match pedir("GET", &format!("/v1/postgres/proyectos/{proyecto}"), None)
                {
                    Ok(x) => x,
                    Err(r) => return r,
                };
                if c != 200 {
                    return tal_cual(c, t);
                }
                if dueno_del_proyecto(&t).as_deref() == Some(dueno.as_str()) {
                    None
                } else {
                    match self.exigir(sujeto, que.potestad(), &ruta) {
                        Ok(d) => d,
                        Err(r) => return r,
                    }
                }
            }
        };
        if cuerpo.is_none() && p.metodo != "GET" && !p.cuerpo.trim().is_empty() {
            match ore_core::parse::parse(&p.cuerpo) {
                Ok(n) => cuerpo = Some(Json::de_node_fiel(&n)),
                Err(_) => return Respuesta::error(400, "el cuerpo no es JSON"),
            }
        }
        let (codigo, texto) = match pedir(&p.metodo, &camino, cuerpo.as_ref()) {
            Ok(x) => x,
            Err(r) => return r,
        };
        if p.metodo != "GET" && (200..300).contains(&codigo) {
            self.contar(crate::acceso::evento(
                que.potestad(),
                &format!("postgres{}", camino.trim_start_matches("/v1/postgres")),
                "hecho",
                decision,
                None,
            ));
        }
        tal_cual(codigo, texto)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn cada_ruta_pide_su_potestad() {
        let q = |m: &str, c: &str| {
            let v: Vec<&str> = c.split('/').collect();
            que_pide(m, &v).map(|q| format!("{q:?}"))
        };
        assert_eq!(q("GET", "proyectos").as_deref(), Some("Ver"));
        assert_eq!(
            q("GET", "proyectos/ventas/ramas/main/roles").as_deref(),
            Some("Ver")
        );
        assert_eq!(q("GET", "operaciones/op-1").as_deref(), Some("Ver"));
        assert_eq!(q("POST", "proyectos").as_deref(), Some("Crear"));
        assert_eq!(
            q("POST", "proyectos/ventas/ramas/main/roles").as_deref(),
            Some("Usar")
        );
        assert_eq!(
            q("POST", "proyectos/ventas/ramas/main/roles/app/contrasena").as_deref(),
            Some("Usar")
        );
        assert_eq!(
            q("DELETE", "proyectos/ventas/ramas/main/roles/app").as_deref(),
            Some("Usar")
        );
        assert_eq!(
            q("DELETE", "proyectos/ventas").as_deref(),
            Some("Gestionar(\"ventas\")")
        );
        assert_eq!(
            q("POST", "proyectos/ventas/ramas").as_deref(),
            Some("Gestionar(\"ventas\")")
        );
        assert_eq!(
            q("POST", "proyectos/ventas/ramas/dev/endpoints").as_deref(),
            Some("Gestionar(\"ventas\")")
        );
        assert_eq!(
            q("DELETE", "proyectos/ventas/ramas/dev/bases/x").as_deref(),
            Some("Gestionar(\"ventas\")")
        );
        // Lo que no existe no pregunta a nadie.
        assert_eq!(q("PUT", "proyectos/ventas"), None);
        assert_eq!(q("POST", "operaciones/op-1"), None);
        assert_eq!(q("GET", "avisos/notify-attach"), None);
    }

    #[test]
    fn el_dueno_sale_de_la_ficha_del_proyecto() {
        assert_eq!(
            dueno_del_proyecto(r#"{"id":"ventas","dueno":"user:ana","tenant":"x"}"#).as_deref(),
            Some("user:ana")
        );
        assert_eq!(dueno_del_proyecto(r#"{"id":"ventas"}"#), None);
        assert_eq!(dueno_del_proyecto("no es json"), None);
    }

    #[test]
    fn lo_que_contesta_va_tal_cual() {
        let r = tal_cual(202, r#"{"operacion":{"id":"op-1"}}"#.into());
        assert_eq!(r.codigo, 202);
        assert_eq!(r.cuerpo.jcs(), r#"{"operacion":{"id":"op-1"}}"#);
        let r = tal_cual(502, "Bad Gateway".into());
        assert_eq!(r.cuerpo.jcs(), r#"{"error":"Bad Gateway"}"#);
    }
}
