//! **El contrato** (0058 P4·1): lo que una celda le pide a `ore-postgres`.
//!
//! ```text
//!   GET    /salud
//!   GET    /v1/postgres/proyectos                 los de la organización de la celda
//!   POST   /v1/postgres/proyectos                 {"id": "ventas", "dueno": "user:ana"}  → 202 + operación
//!   GET    /v1/postgres/proyectos/{p}
//!   DELETE /v1/postgres/proyectos/{p}             → 202 + operación
//!   GET    /v1/postgres/proyectos/{p}/acceso         quién entra: IPs, bloqueo público, límites (P5·6)
//!   POST   /v1/postgres/proyectos/{p}/acceso         {"ips_permitidas": [...], "bloquear_publico": false,
//!                                                     "limites": {"tcp": {"por_segundo", "rafaga"}, "ws", "http"}}
//!                                                     → 202; lo que no viene, se queda como está
//!   GET    /v1/postgres/proyectos/{p}/ramas
//!   POST   /v1/postgres/proyectos/{p}/ramas       {"id": "dev", "padre": "main", "lsn" | "instante"}  → 202
//!   GET    /v1/postgres/proyectos/{p}/ramas/{r}
//!   DELETE /v1/postgres/proyectos/{p}/ramas/{r}   → 202 (`main`, una rama con hijas o con endpoints, no: 409)
//!   GET    /v1/postgres/proyectos/{p}/ramas/{r}/endpoints
//!   POST   /v1/postgres/proyectos/{p}/ramas/{r}/endpoints   {"id", "tipo": "lectura-escritura" | "lectura",
//!                                                           "cu_min": "0.25", "cu_max": "1"}  → 202
//!   GET    /v1/postgres/proyectos/{p}/ramas/{r}/endpoints/{e}
//!   DELETE /v1/postgres/proyectos/{p}/ramas/{r}/endpoints/{e}   → 202
//!   POST   /v1/postgres/proyectos/{p}/ramas/{r}/endpoints/{e}/ajustes   {"cu_min", "cu_max", "dormir_tras"}
//!                                                           → 202 (P6·2); lo que no viene, se queda
//!   GET    /v1/postgres/proyectos/{p}/ramas/{r}/roles
//!   POST   /v1/postgres/proyectos/{p}/ramas/{r}/roles            {"nombre": "app"}  → 202 + la contraseña, UNA vez
//!   POST   /v1/postgres/proyectos/{p}/ramas/{r}/roles/{n}/contrasena               → 202 + una nueva; la vieja deja de valer
//!   DELETE /v1/postgres/proyectos/{p}/ramas/{r}/roles/{n}        → 202 (con bases suyas, 409)
//!   GET    /v1/postgres/proyectos/{p}/ramas/{r}/bases
//!   POST   /v1/postgres/proyectos/{p}/ramas/{r}/bases            {"nombre": "ventas", "dueno": "app"}  → 202
//!   DELETE /v1/postgres/proyectos/{p}/ramas/{r}/bases/{b}        → 202
//!   GET    /v1/postgres/operaciones/{op}
//! ```
//!
//! - **La celda en `Authorization`** (su token de Workload Identity, audiencia
//!   `ore-postgres`). La organización sale de ella ([`crate::celda`]); un `id` de
//!   otra organización es un 404, igual que uno que no existe.
//! - **Toda escritura es una operación** con id, que se sondea hasta `hecha`. Una
//!   en curso por proyecto: la segunda es un **409** (lo cierra la base,
//!   `una_en_curso_por_proyecto`). En P4·1 un proyecto está vacío y no hay nada
//!   que esperar, así que la operación nace hecha; desde P4·2 la termina el
//!   reconciliador.
//! - **Ids que pone quien crea**: `[a-z0-9-]`, de 1 a 63, sin guion en los
//!   extremos, e inmutables. Crear uno que ya existe es un 409: repetir la
//!   petición no crea dos.
//!
//! ⛔ Ni la persona ni sus potestades llegan aquí: eso lo resuelve `ore-serve`
//!   con `ore-acceso` antes de llamar (P4·6). `dueno` es lo que `ore-serve`
//!   preguntó a `quien`.

use crate::base::{choca, mal};
use crate::celda::{Celda, Celdas};
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::{Peticion, Respuesta};
use postgres::{Client, Row};
use std::sync::Mutex;

pub struct Servidor {
    pub base: Mutex<Client>,
    pub celdas: Box<dyn Celdas>,
    /// Con qué volver a conectar si la base se cae (reinicio de `storcon-db`).
    /// Sin ella, una conexión cerrada es un 503 hasta que se reinicie el pod.
    pub url: Option<String>,
    /// P4·5: los avisos del `storage_controller` (`/avisos/…`). Sin ellos, 404.
    pub avisos: Option<crate::avisos::Avisos>,
    /// P5·1: las preguntas del proxy (`/proxy/…`). Sin él, 404.
    pub proxy: Option<crate::proxy::Proxy>,
    /// P5·7: el dominio de la entrada pública (`europe-west1.pg.paladio.io`).
    /// Con él, cada endpoint dice su `host` y su `host_pool`; sin él, no los dice.
    pub dominio: Option<String>,
}

/// Lo que se elige de un proyecto, en el orden en que lo lee [`proyecto_json`].
const PROYECTO: &str = "id, celda, dueno,
    to_char(creado at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
    deseado, observado, generacion, tenant";

/// Y de una operación, para [`operacion_json`].
const OPERACION: &str = "id, tipo, proyecto, estado, error,
    to_char(creada at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
    to_char(terminada at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

/// Un id nuevo de operación: lo pone la base, que es quien sabe dar uno único.
const NUEVA_OPERACION: &str = "'op_' || replace(gen_random_uuid()::text, '-', '')";

/// Y uno de tenant o de timeline: 32 cifras hexadecimales, como los de Neon.
const NUEVO_HEX: &str = "replace(gen_random_uuid()::text, '-', '')";

/// El nombre de una VM en Kubernetes: único en todo `ore-pg-computo`.
const NUEVA_VM: &str = "'ep-' || substr(replace(gen_random_uuid()::text, '-', ''), 1, 20)";

impl Servidor {
    pub fn atender(&self, p: &Peticion) -> Respuesta {
        let seg = p.segmentos();
        if let ("GET", ["salud"]) = (p.metodo.as_str(), seg.as_slice()) {
            return Respuesta::ok(Json::obj([("ok", Json::Bool(true))]));
        }
        // P4·5: los avisos del almacenamiento, con SU token (no el de una celda).
        if let ["avisos", resto @ ..] = seg.as_slice() {
            let Some(avisos) = self.avisos.as_ref() else {
                return Respuesta::error(404, "los avisos no están montados");
            };
            let Ok(mut base) = self.base.lock() else {
                return Respuesta::error(500, "la conexión quedó envenenada");
            };
            if base.is_closed()
                && let Some(Ok(nueva)) = self.url.as_deref().map(crate::base::conectar)
            {
                *base = nueva;
            }
            return avisos.atender(&mut base, p, resto);
        }
        // P5·1: el proxy, con SU token. P6·4: con la base sin tomar: un despertar espera, y
        // mientras espera la suelta (la API entera la comparte).
        if let ["proxy", resto @ ..] = seg.as_slice() {
            let Some(proxy) = self.proxy.as_ref() else {
                return Respuesta::error(404, "el proxy no está montado");
            };
            return proxy.atender(&self.base, self.url.as_deref(), p, resto);
        }
        let ["v1", "postgres", resto @ ..] = seg.as_slice() else {
            return Respuesta::error(404, "no hay nada en ese camino");
        };
        // ① LA CELDA, antes de mirar nada más.
        let celda = match self
            .celdas
            .de(p.cabeceras.get("authorization").map(String::as_str))
        {
            Ok(c) => c,
            Err(e) => return Respuesta::error(e.codigo(), e.motivo()),
        };
        // ② lo que pide.
        let pedido = match ruta(&p.metodo, resto) {
            Ok(r) => r,
            Err(r) => return r,
        };
        let cuerpo = match &pedido {
            Pedido::CrearProyecto
            | Pedido::CrearRama(_)
            | Pedido::CrearEndpoint(..)
            | Pedido::CrearRol(..)
            | Pedido::CrearBase(..)
            | Pedido::CambiarAcceso(_)
            | Pedido::AjustarEndpoint(..) => match analizar(&p.cuerpo) {
                Ok(n) => Some(n),
                Err(m) => return Respuesta::error(400, m),
            },
            _ => None,
        };
        let Ok(mut base) = self.base.lock() else {
            return Respuesta::error(500, "la conexión quedó envenenada");
        };
        // Una conexión que se cayó (la base reinició) se reabre aquí, una vez por
        // petición; si no se puede, 503 y la próxima lo vuelve a intentar.
        if base.is_closed() {
            match self.url.as_deref().map(crate::base::conectar) {
                Some(Ok(nueva)) => *base = nueva,
                Some(Err(e)) => return Respuesta::error(503, format!("la base no contesta: {e}")),
                None => return Respuesta::error(503, "la conexión con la base está cerrada"),
            }
        }
        let hacer = |c: &mut Client| match &pedido {
            Pedido::Proyectos => proyectos(c, &celda),
            Pedido::Proyecto(id) => proyecto(c, &celda, id),
            Pedido::CrearProyecto => crear_proyecto(c, &celda, cuerpo.as_ref().expect("analizado")),
            Pedido::BorrarProyecto(id) => borrar_proyecto(c, &celda, id),
            Pedido::Acceso(id) => acceso(c, &celda, id),
            Pedido::CambiarAcceso(id) => {
                cambiar_acceso(c, &celda, id, cuerpo.as_ref().expect("analizado"))
            }
            Pedido::Operacion(id) => operacion(c, &celda, id),
            Pedido::Ramas(p) => ramas(c, &celda, p),
            Pedido::Rama(p, r) => rama(c, &celda, p, r),
            Pedido::CrearRama(p) => crear_rama(c, &celda, p, cuerpo.as_ref().expect("analizado")),
            Pedido::BorrarRama(p, r) => borrar_rama(c, &celda, p, r),
            Pedido::Endpoints(p, r) => endpoints(c, &celda, p, r, self.dominio.as_deref()),
            Pedido::Endpoint(p, r, e) => endpoint(c, &celda, p, r, e, self.dominio.as_deref()),
            Pedido::CrearEndpoint(p, r) => crear_endpoint(
                c,
                &celda,
                p,
                r,
                cuerpo.as_ref().expect("analizado"),
                self.dominio.as_deref(),
            ),
            Pedido::BorrarEndpoint(p, r, e) => borrar_endpoint(c, &celda, p, r, e),
            Pedido::AjustarEndpoint(p, r, e) => ajustar_endpoint(
                c,
                &celda,
                (p, r, e),
                cuerpo.as_ref().expect("analizado"),
                self.dominio.as_deref(),
            ),
            Pedido::Roles(p, r) => roles(c, &celda, p, r),
            Pedido::CrearRol(p, r) => {
                crear_rol(c, &celda, p, r, cuerpo.as_ref().expect("analizado"))
            }
            Pedido::Contrasena(p, r, n) => nueva_contrasena(c, &celda, p, r, n),
            Pedido::BorrarRol(p, r, n) => borrar_rol(c, &celda, p, r, n),
            Pedido::Bases(p, r) => bases(c, &celda, p, r),
            Pedido::CrearBase(p, r) => {
                crear_base(c, &celda, p, r, cuerpo.as_ref().expect("analizado"))
            }
            Pedido::BorrarBase(p, r, b) => borrar_base(c, &celda, p, r, b),
        };
        let mut r = hacer(&mut base);
        // La caída sólo se ve al usarla: si falló y la conexión resulta cerrada, se
        // reabre y se repite UNA vez. Es seguro: lo que no se confirmó no dejó nada.
        if r.is_err()
            && base.is_closed()
            && let Some(Ok(nueva)) = self.url.as_deref().map(crate::base::conectar)
        {
            *base = nueva;
            r = hacer(&mut base);
        }
        r.unwrap_or_else(|Fallo(codigo, m)| Respuesta::error(codigo, m))
    }
}

/// Lo que una ruta pide.
#[derive(Debug, PartialEq, Eq)]
pub enum Pedido<'a> {
    Proyectos,
    CrearProyecto,
    Proyecto(&'a str),
    BorrarProyecto(&'a str),
    Acceso(&'a str),
    CambiarAcceso(&'a str),
    Operacion(&'a str),
    Ramas(&'a str),
    CrearRama(&'a str),
    Rama(&'a str, &'a str),
    BorrarRama(&'a str, &'a str),
    Endpoints(&'a str, &'a str),
    CrearEndpoint(&'a str, &'a str),
    Endpoint(&'a str, &'a str, &'a str),
    BorrarEndpoint(&'a str, &'a str, &'a str),
    AjustarEndpoint(&'a str, &'a str, &'a str),
    Roles(&'a str, &'a str),
    CrearRol(&'a str, &'a str),
    Contrasena(&'a str, &'a str, &'a str),
    BorrarRol(&'a str, &'a str, &'a str),
    Bases(&'a str, &'a str),
    CrearBase(&'a str, &'a str),
    BorrarBase(&'a str, &'a str, &'a str),
}

/// De método y camino (sin `/v1/postgres`) a lo que se pide.
pub fn ruta<'a>(metodo: &str, resto: &[&'a str]) -> Result<Pedido<'a>, Respuesta> {
    let pedido = match (metodo, resto) {
        ("GET", ["proyectos"]) => Pedido::Proyectos,
        ("POST", ["proyectos"]) => Pedido::CrearProyecto,
        ("GET", ["proyectos", p]) => Pedido::Proyecto(p),
        ("DELETE", ["proyectos", p]) => Pedido::BorrarProyecto(p),
        ("GET", ["proyectos", p, "acceso"]) => Pedido::Acceso(p),
        ("POST", ["proyectos", p, "acceso"]) => Pedido::CambiarAcceso(p),
        ("GET", ["operaciones", o]) => Pedido::Operacion(o),
        ("GET", ["proyectos", p, "ramas"]) => Pedido::Ramas(p),
        ("POST", ["proyectos", p, "ramas"]) => Pedido::CrearRama(p),
        ("GET", ["proyectos", p, "ramas", r]) => Pedido::Rama(p, r),
        ("DELETE", ["proyectos", p, "ramas", r]) => Pedido::BorrarRama(p, r),
        ("GET", ["proyectos", p, "ramas", r, "endpoints"]) => Pedido::Endpoints(p, r),
        ("POST", ["proyectos", p, "ramas", r, "endpoints"]) => Pedido::CrearEndpoint(p, r),
        ("GET", ["proyectos", p, "ramas", r, "endpoints", e]) => Pedido::Endpoint(p, r, e),
        ("DELETE", ["proyectos", p, "ramas", r, "endpoints", e]) => Pedido::BorrarEndpoint(p, r, e),
        ("POST", ["proyectos", p, "ramas", r, "endpoints", e, "ajustes"]) => {
            Pedido::AjustarEndpoint(p, r, e)
        }
        ("GET", ["proyectos", p, "ramas", r, "roles"]) => Pedido::Roles(p, r),
        ("POST", ["proyectos", p, "ramas", r, "roles"]) => Pedido::CrearRol(p, r),
        ("POST", ["proyectos", p, "ramas", r, "roles", n, "contrasena"]) => {
            Pedido::Contrasena(p, r, n)
        }
        ("DELETE", ["proyectos", p, "ramas", r, "roles", n]) => Pedido::BorrarRol(p, r, n),
        ("GET", ["proyectos", p, "ramas", r, "bases"]) => Pedido::Bases(p, r),
        ("POST", ["proyectos", p, "ramas", r, "bases"]) => Pedido::CrearBase(p, r),
        ("DELETE", ["proyectos", p, "ramas", r, "bases", b]) => Pedido::BorrarBase(p, r, b),
        (
            _,
            ["proyectos"]
            | ["proyectos", _]
            | ["proyectos", _, "acceso"]
            | ["operaciones", _]
            | ["proyectos", _, "ramas"]
            | ["proyectos", _, "ramas", _]
            | ["proyectos", _, "ramas", _, "endpoints"]
            | ["proyectos", _, "ramas", _, "endpoints", _]
            | ["proyectos", _, "ramas", _, "endpoints", _, "ajustes"]
            | ["proyectos", _, "ramas", _, "roles" | "bases"]
            | ["proyectos", _, "ramas", _, "roles" | "bases", _]
            | ["proyectos", _, "ramas", _, "roles", _, "contrasena"],
        ) => {
            return Err(Respuesta::error(
                405,
                format!("`{metodo}` no se atiende aquí"),
            ));
        }
        _ => return Err(Respuesta::error(404, "no hay nada en ese camino")),
    };
    match &pedido {
        Pedido::Ramas(p)
        | Pedido::CrearRama(p)
        | Pedido::Rama(p, _)
        | Pedido::BorrarRama(p, _)
        | Pedido::Acceso(p)
        | Pedido::CambiarAcceso(p)
            if !id_valido(p) =>
        {
            Err(Respuesta::error(
                404,
                format!("no hay ningún proyecto `{p}`"),
            ))
        }
        Pedido::Endpoints(p, r)
        | Pedido::CrearEndpoint(p, r)
        | Pedido::Endpoint(p, r, _)
        | Pedido::BorrarEndpoint(p, r, _)
        | Pedido::AjustarEndpoint(p, r, _)
            if !id_valido(p) || !id_valido(r) =>
        {
            Err(Respuesta::error(
                404,
                format!("no hay ninguna rama `{r}` en `{p}`"),
            ))
        }
        Pedido::Roles(p, r)
        | Pedido::CrearRol(p, r)
        | Pedido::Contrasena(p, r, _)
        | Pedido::BorrarRol(p, r, _)
        | Pedido::Bases(p, r)
        | Pedido::CrearBase(p, r)
        | Pedido::BorrarBase(p, r, _)
            if !id_valido(p) || !id_valido(r) =>
        {
            Err(Respuesta::error(
                404,
                format!("no hay ninguna rama `{r}` en `{p}`"),
            ))
        }
        Pedido::Contrasena(_, _, n) | Pedido::BorrarRol(_, _, n) | Pedido::BorrarBase(_, _, n)
            if !nombre_valido(n) =>
        {
            Err(Respuesta::error(
                404,
                format!("no hay nada que se llame `{n}`"),
            ))
        }
        Pedido::Endpoint(_, _, e) | Pedido::BorrarEndpoint(_, _, e) if !id_valido(e) => Err(
            Respuesta::error(404, format!("no hay ningún endpoint `{e}`")),
        ),
        Pedido::Rama(_, r) | Pedido::BorrarRama(_, r) if !id_valido(r) => {
            Err(Respuesta::error(404, format!("no hay ninguna rama `{r}`")))
        }
        Pedido::Proyecto(p) | Pedido::BorrarProyecto(p) if !id_valido(p) => Err(Respuesta::error(
            404,
            format!("no hay ningún proyecto `{p}`"),
        )),
        Pedido::Operacion(o) if !o.starts_with("op_") || o.len() > 40 => Err(Respuesta::error(
            404,
            format!("no hay ninguna operación `{o}`"),
        )),
        _ => Ok(pedido),
    }
}

/// Un id de recurso: `[a-z0-9-]`, de 1 a 63, sin guion en los extremos (como un
/// nombre DNS, y como los de Lakebase).
pub fn id_valido(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 63
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ── los verbos ─────────────────────────────────────────────────────────────

struct Fallo(u16, String);

impl From<postgres::Error> for Fallo {
    fn from(e: postgres::Error) -> Fallo {
        Fallo(500, mal(e))
    }
}

fn proyectos(c: &mut Client, celda: &Celda) -> Result<Respuesta, Fallo> {
    let filas = c.query(
        &format!(
            "select {PROYECTO} from plano.proyecto
              where organizacion = $1 and deseado = 'vivo' order by id"
        ),
        &[&celda.organizacion],
    )?;
    Ok(Respuesta::ok(Json::obj([(
        "proyectos",
        Json::Arr(filas.iter().map(proyecto_json).collect()),
    )])))
}

fn proyecto(c: &mut Client, celda: &Celda, id: &str) -> Result<Respuesta, Fallo> {
    match c.query_opt(
        &format!("select {PROYECTO} from plano.proyecto where organizacion = $1 and id = $2"),
        &[&celda.organizacion, &id],
    )? {
        Some(f) => Ok(Respuesta::ok(proyecto_json(&f))),
        None => Err(no_hay_proyecto(id)),
    }
}

fn crear_proyecto(c: &mut Client, celda: &Celda, cuerpo: &Node) -> Result<Respuesta, Fallo> {
    let texto = |k: &str| cuerpo.get(k).and_then(|(_, v)| v.as_str());
    let Some(id) = texto("id") else {
        return Err(Fallo(400, "falta `id`: el nombre del proyecto".into()));
    };
    if !id_valido(id) {
        return Err(Fallo(
            400,
            format!(
                "`{id}` no vale como id: `[a-z0-9-]`, de 1 a 63, sin guion al principio ni al final"
            ),
        ));
    }
    let dueno = texto("dueno");
    if let Some(d) = dueno
        && !(d.starts_with("user:") && ore_core::pertenencia::es_handle(d))
    {
        return Err(Fallo(
            400,
            format!("`dueno` es `user:<handle>` (0052), no `{d}`"),
        ));
    }
    let mut tx = c.transaction()?;
    // P4·2: nace `nuevo`, con su tenant y su `main` ya nombrados (un reintento usa
    // los mismos ids), y el reconciliador los crea en el almacenamiento.
    match tx.query_one(
        &format!(
            "insert into plano.proyecto (organizacion, id, celda, dueno, tenant)
             values ($1, $2, $3, $4, {NUEVO_HEX}) returning {PROYECTO}"
        ),
        &[&celda.organizacion, &id, &celda.id, &dueno],
    ) {
        Ok(fila) => {
            tx.execute(
                &format!(
                    "insert into plano.rama (organizacion, proyecto, id, timeline)
                     values ($1, $2, 'main', {NUEVO_HEX})"
                ),
                &[&celda.organizacion, &id],
            )?;
            // P4·3·3: y su endpoint de escritura en main, como Lakebase.
            tx.execute(
                &format!(
                    "insert into plano.endpoint (organizacion, proyecto, rama, id, vm)
                     values ($1, $2, 'main', 'principal', {NUEVA_VM})"
                ),
                &[&celda.organizacion, &id],
            )?;
            // P4·4: y el rol de quien lo crea (su handle), dueño de una base con el
            // nombre del proyecto. Su contraseña sale en esta respuesta y en ninguna otra.
            let mut contrasena = None;
            if let Some(d) = dueno {
                let rol = d.trim_start_matches("user:");
                let (clave, verificador) = crate::scram::nueva().map_err(|e| Fallo(500, e))?;
                tx.execute(
                    "insert into plano.rol (organizacion, proyecto, rama, nombre, verificador)
                     values ($1, $2, 'main', $3, $4)",
                    &[&celda.organizacion, &id, &rol, &verificador],
                )?;
                tx.execute(
                    "insert into plano.base (organizacion, proyecto, rama, nombre, dueno)
                     values ($1, $2, 'main', $2, $3)",
                    &[&celda.organizacion, &id, &rol],
                )?;
                contrasena = Some((rol.to_string(), clave));
            }
            let op = tx.query_one(
                &format!(
                    "insert into plano.operacion (id, organizacion, proyecto, tipo, celda)
                     values ({NUEVA_OPERACION}, $1, $2, 'crear-proyecto', $3)
                     returning {OPERACION}"
                ),
                &[&celda.organizacion, &id, &celda.id],
            )?;
            tx.commit()?;
            let mut r = aceptada(&op, Some(("proyecto", proyecto_json(&fila))));
            if let (Some((rol, clave)), Json::Obj(o)) = (contrasena, &mut r.cuerpo) {
                o.insert(
                    "rol".into(),
                    Json::obj([("nombre", Json::s(rol)), ("contrasena", Json::s(clave))]),
                );
            }
            Ok(r)
        }
        Err(e) if choca(&e, "proyecto_pkey") => Err(Fallo(
            409,
            format!("ya hay un proyecto `{id}` en esta organización"),
        )),
        Err(e) => Err(e.into()),
    }
}

fn borrar_proyecto(c: &mut Client, celda: &Celda, id: &str) -> Result<Respuesta, Fallo> {
    let mut tx = c.transaction()?;
    let Some(_) = tx.query_opt(
        "select 1 from plano.proyecto where organizacion = $1 and id = $2 for update",
        &[&celda.organizacion, &id],
    )?
    else {
        return Err(no_hay_proyecto(id));
    };
    let op = match tx.query_one(
        &format!(
            "insert into plano.operacion (id, organizacion, proyecto, tipo, celda)
             values ({NUEVA_OPERACION}, $1, $2, 'borrar-proyecto', $3) returning id"
        ),
        &[&celda.organizacion, &id, &celda.id],
    ) {
        Ok(f) => f.get::<_, String>(0),
        Err(e) if choca(&e, "una_en_curso_por_proyecto") => {
            return Err(Fallo(
                409,
                format!("el proyecto `{id}` tiene otra operación en curso: espera a que termine"),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    // P4·2: se marca y lo borra el reconciliador (el tenant y su WAL en cada
    // safekeeper); la fila se va cuando ya no queda nada fuera.
    tx.execute(
        "update plano.proyecto
            set deseado = 'borrado', observado = 'borrando', generacion = generacion + 1
          where organizacion = $1 and id = $2",
        &[&celda.organizacion, &id],
    )?;
    let op = tx.query_one(
        &format!("select {OPERACION} from plano.operacion where id = $1"),
        &[&op],
    )?;
    tx.commit()?;
    Ok(aceptada(&op, None))
}

fn operacion(c: &mut Client, celda: &Celda, id: &str) -> Result<Respuesta, Fallo> {
    match c.query_opt(
        &format!("select {OPERACION} from plano.operacion where organizacion = $1 and id = $2"),
        &[&celda.organizacion, &id],
    )? {
        Some(f) => Ok(Respuesta::ok(operacion_json(&f))),
        None => Err(Fallo(404, format!("no hay ninguna operación `{id}`"))),
    }
}

// ── la forma de lo que sale ────────────────────────────────────────────────

// ── las ramas (P4·2·3) ─────────────────────────────────────────────────────

/// De una rama, en el orden en que lo lee [`rama_json`].
const RAMA: &str = "id, timeline, padre, lsn_origen,
    to_char(instante_origen at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'),
    deseado, observado,
    to_char(creada at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

/// El proyecto, vivo y de esta organización; si no, 404.
fn proyecto_vivo(
    c: &mut impl postgres::GenericClient,
    celda: &Celda,
    p: &str,
) -> Result<(), Fallo> {
    match c.query_opt(
        "select 1 from plano.proyecto where organizacion = $1 and id = $2 and deseado = 'vivo'",
        &[&celda.organizacion, &p],
    )? {
        Some(_) => Ok(()),
        None => Err(no_hay_proyecto(p)),
    }
}

fn ramas(c: &mut Client, celda: &Celda, p: &str) -> Result<Respuesta, Fallo> {
    proyecto_vivo(c, celda, p)?;
    let filas = c.query(
        &format!(
            "select {RAMA} from plano.rama
              where organizacion = $1 and proyecto = $2 and deseado = 'viva'
              order by padre nulls first, id"
        ),
        &[&celda.organizacion, &p],
    )?;
    Ok(Respuesta::ok(Json::obj([(
        "ramas",
        Json::Arr(filas.iter().map(rama_json).collect()),
    )])))
}

fn rama(c: &mut Client, celda: &Celda, p: &str, r: &str) -> Result<Respuesta, Fallo> {
    proyecto_vivo(c, celda, p)?;
    match c.query_opt(
        &format!(
            "select {RAMA} from plano.rama where organizacion = $1 and proyecto = $2 and id = $3"
        ),
        &[&celda.organizacion, &p, &r],
    )? {
        Some(f) => Ok(Respuesta::ok(rama_json(&f))),
        None => Err(no_hay_rama(r)),
    }
}

/// Un LSN como los escribe Postgres: `0/16B5A50`.
pub fn lsn_valido(l: &str) -> bool {
    let Some((a, b)) = l.split_once('/') else {
        return false;
    };
    [a, b]
        .iter()
        .all(|x| !x.is_empty() && x.len() <= 8 && x.chars().all(|c| c.is_ascii_hexdigit()))
}

fn crear_rama(c: &mut Client, celda: &Celda, p: &str, cuerpo: &Node) -> Result<Respuesta, Fallo> {
    let texto = |k: &str| cuerpo.get(k).and_then(|(_, v)| v.as_str());
    let Some(id) = texto("id") else {
        return Err(Fallo(400, "falta `id`: el nombre de la rama".into()));
    };
    if !id_valido(id) {
        return Err(Fallo(
            400,
            format!(
                "`{id}` no vale como id: `[a-z0-9-]`, de 1 a 63, sin guion al principio ni al final"
            ),
        ));
    }
    let padre = texto("padre").unwrap_or("main");
    let lsn = texto("lsn");
    let instante = texto("instante");
    if lsn.is_some() && instante.is_some() {
        return Err(Fallo(400, "o `lsn` o `instante`, no los dos".into()));
    }
    if let Some(l) = lsn
        && !lsn_valido(l)
    {
        return Err(Fallo(400, format!("`{l}` no es un LSN (como `0/16B5A50`)")));
    }
    let mut tx = c.transaction()?;
    proyecto_vivo(&mut tx, celda, p)?;
    let Some(f) = tx.query_opt(
        "select observado from plano.rama
          where organizacion = $1 and proyecto = $2 and id = $3 and deseado = 'viva'",
        &[&celda.organizacion, &p, &padre],
    )?
    else {
        return Err(Fallo(
            404,
            format!("no hay ninguna rama `{padre}` de la que salir"),
        ));
    };
    if f.get::<_, String>(0) != "lista" {
        return Err(Fallo(
            409,
            format!("la rama `{padre}` aún no está lista: no se puede salir de ella"),
        ));
    }
    // El instante lo interpreta la base: lo que no sea una fecha es un 400. Se
    // prueba en un punto de guardado, para que el error no estropee la transacción.
    let instante: Option<String> = match instante {
        None => None,
        Some(i) => {
            let mut sp = tx.transaction()?;
            match sp.query_one(
                "select to_char($1::text::timestamptz at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')",
                &[&i],
            ) {
                Ok(f) => {
                    sp.commit()?;
                    Some(f.get(0))
                }
                Err(_) => {
                    return Err(Fallo(
                        400,
                        format!("`{i}` no es un instante (como `2026-10-08T10:00:00Z`)"),
                    ));
                }
            }
        }
    };
    let fila = match tx.query_one(
        &format!(
            "insert into plano.rama (organizacion, proyecto, id, timeline, padre, lsn_origen, instante_origen)
             values ($1, $2, $3, {NUEVO_HEX}, $4, $5, $6::text::timestamptz) returning {RAMA}"
        ),
        &[&celda.organizacion, &p, &id, &padre, &lsn, &instante],
    ) {
        Ok(f) => f,
        Err(e) if choca(&e, "rama_pkey") => {
            return Err(Fallo(409, format!("ya hay una rama `{id}` en `{p}`")));
        }
        Err(e) => return Err(e.into()),
    };
    // P4·4: hereda los roles y las bases de su padre (ya están en sus datos).
    tx.execute(
        "insert into plano.rol (organizacion, proyecto, rama, nombre, verificador)
         select organizacion, proyecto, $3, nombre, verificador from plano.rol
          where organizacion = $1 and proyecto = $2 and rama = $4 and deseado = 'vivo'",
        &[&celda.organizacion, &p, &id, &padre],
    )?;
    tx.execute(
        "insert into plano.base (organizacion, proyecto, rama, nombre, dueno)
         select organizacion, proyecto, $3, nombre, dueno from plano.base
          where organizacion = $1 and proyecto = $2 and rama = $4 and deseado = 'vivo'",
        &[&celda.organizacion, &p, &id, &padre],
    )?;
    let op = nueva_operacion(&mut tx, celda, p, "crear-rama", Some(id))?;
    tx.commit()?;
    Ok(aceptada(&op, Some(("rama", rama_json(&fila)))))
}

fn borrar_rama(c: &mut Client, celda: &Celda, p: &str, r: &str) -> Result<Respuesta, Fallo> {
    let mut tx = c.transaction()?;
    proyecto_vivo(&mut tx, celda, p)?;
    let Some(f) = tx.query_opt(
        "select padre from plano.rama
          where organizacion = $1 and proyecto = $2 and id = $3 and deseado = 'viva' for update",
        &[&celda.organizacion, &p, &r],
    )?
    else {
        return Err(no_hay_rama(r));
    };
    if f.get::<_, Option<String>>(0).is_none() {
        return Err(Fallo(
            409,
            format!("`{r}` es la primera rama del proyecto: se va con el proyecto, no sola"),
        ));
    }
    let hijas: Vec<String> = tx
        .query(
            "select id from plano.rama
              where organizacion = $1 and proyecto = $2 and padre = $3 and deseado = 'viva'
              order by id",
            &[&celda.organizacion, &p, &r],
        )?
        .iter()
        .map(|f| f.get(0))
        .collect();
    let con_endpoints: Vec<String> = tx
        .query(
            "select id from plano.endpoint
              where organizacion = $1 and proyecto = $2 and rama = $3 order by id",
            &[&celda.organizacion, &p, &r],
        )?
        .iter()
        .map(|f| f.get(0))
        .collect();
    if !con_endpoints.is_empty() {
        return Err(Fallo(
            409,
            format!(
                "`{r}` tiene endpoints ({}): bórralos antes",
                con_endpoints.join(", ")
            ),
        ));
    }
    if !hijas.is_empty() {
        return Err(Fallo(
            409,
            format!(
                "`{r}` tiene ramas que salen de ella ({}): bórralas antes",
                hijas.join(", ")
            ),
        ));
    }
    let op = nueva_operacion(&mut tx, celda, p, "borrar-rama", Some(r))?;
    tx.execute(
        "update plano.rama set deseado = 'borrada', observado = 'borrando'
          where organizacion = $1 and proyecto = $2 and id = $3",
        &[&celda.organizacion, &p, &r],
    )?;
    tx.commit()?;
    Ok(aceptada(&op, None))
}

/// Una operación en curso sobre el proyecto; si ya hay otra, 409.
fn nueva_operacion(
    tx: &mut postgres::Transaction,
    celda: &Celda,
    p: &str,
    tipo: &str,
    rama: Option<&str>,
) -> Result<Row, Fallo> {
    nueva_operacion_de(tx, celda, p, tipo, rama, None)
}

fn nueva_operacion_de(
    tx: &mut postgres::Transaction,
    celda: &Celda,
    p: &str,
    tipo: &str,
    rama: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Row, Fallo> {
    match tx.query_one(
        &format!(
            "insert into plano.operacion (id, organizacion, proyecto, tipo, celda, rama, endpoint)
             values ({NUEVA_OPERACION}, $1, $2, $3, $4, $5, $6) returning {OPERACION}"
        ),
        &[&celda.organizacion, &p, &tipo, &celda.id, &rama, &endpoint],
    ) {
        Ok(f) => Ok(f),
        Err(e) if choca(&e, "una_en_curso_por_proyecto") => Err(Fallo(
            409,
            format!("el proyecto `{p}` tiene otra operación en curso: espera a que termine"),
        )),
        Err(e) => Err(e.into()),
    }
}

fn rama_json(f: &Row) -> Json {
    let mut v = vec![
        ("id", Json::s(f.get::<_, String>(0))),
        ("timeline", Json::s(f.get::<_, String>(1))),
        (
            "estado",
            Json::obj([
                ("deseado", Json::s(f.get::<_, String>(5))),
                ("observado", Json::s(f.get::<_, String>(6))),
            ]),
        ),
        ("creada", Json::s(f.get::<_, String>(7))),
    ];
    let mut origen = Vec::new();
    if let Some(p) = f.get::<_, Option<String>>(2) {
        origen.push(("rama", Json::s(p)));
    }
    if let Some(l) = f.get::<_, Option<String>>(3) {
        origen.push(("lsn", Json::s(l)));
    }
    if let Some(i) = f.get::<_, Option<String>>(4) {
        origen.push(("instante", Json::s(i)));
    }
    if !origen.is_empty() {
        v.push(("origen", Json::obj(origen)));
    }
    Json::obj(v)
}

// ── los endpoints (P4·3·3) ─────────────────────────────────────────────────

/// De un endpoint, en el orden en que lo lee [`endpoint_json`].
const ENDPOINT: &str = "id, rama, tipo, vm, cu_min, cu_max, deseado, observado, direccion,
    to_char(creado at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), dormir_tras,
    to_char(ultima_actividad at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
    to_char(dormido_en at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), computo";

/// Lo que hace falta para decir sus conexiones ([`endpoint_json`]), detrás de
/// [`ENDPOINT`], con la fila del endpoint como `e`.
fn conexiones_sql() -> String {
    format!(
        "{}, (select count(*) from plano.base b where b.organizacion = e.organizacion
              and b.proyecto = e.proyecto and b.rama = e.rama and b.deseado = 'vivo')",
        crate::especificacion::CU_DE_LAS_CONEXIONES
    )
}

/// La rama, viva y de este proyecto vivo de esta organización: si no, 404.
fn rama_viva(
    c: &mut impl postgres::GenericClient,
    celda: &Celda,
    p: &str,
    r: &str,
) -> Result<String, Fallo> {
    proyecto_vivo(c, celda, p)?;
    match c.query_opt(
        "select observado from plano.rama
          where organizacion = $1 and proyecto = $2 and id = $3 and deseado = 'viva'",
        &[&celda.organizacion, &p, &r],
    )? {
        Some(f) => Ok(f.get(0)),
        None => Err(no_hay_rama(r)),
    }
}

fn endpoints(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    dominio: Option<&str>,
) -> Result<Respuesta, Fallo> {
    rama_viva(c, celda, p, r)?;
    let filas = c.query(
        &format!(
            "select {ENDPOINT}, {} from plano.endpoint e
              where organizacion = $1 and proyecto = $2 and rama = $3 and deseado = 'vivo'
              order by id",
            conexiones_sql()
        ),
        &[&celda.organizacion, &p, &r],
    )?;
    Ok(Respuesta::ok(Json::obj([(
        "endpoints",
        Json::Arr(filas.iter().map(|f| endpoint_json(f, dominio)).collect()),
    )])))
}

fn endpoint(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    e: &str,
    dominio: Option<&str>,
) -> Result<Respuesta, Fallo> {
    rama_viva(c, celda, p, r)?;
    match c.query_opt(
        &format!(
            "select {ENDPOINT}, {} from plano.endpoint e
              where organizacion = $1 and proyecto = $2 and rama = $3 and id = $4",
            conexiones_sql()
        ),
        &[&celda.organizacion, &p, &r, &e],
    )? {
        Some(f) => Ok(Respuesta::ok(endpoint_json(&f, dominio))),
        None => Err(no_hay_endpoint(e)),
    }
}

/// Unas CU: un número entre 0.25 y 2.
fn cu(cuerpo: &Node, k: &str, por_defecto: f64) -> Result<f64, Fallo> {
    match cuerpo.get(k).and_then(|(_, v)| v.as_str()) {
        None => Ok(por_defecto),
        Some(v) => v
            .parse::<f64>()
            .ok()
            .filter(|x| (0.25..=2.0).contains(x))
            .ok_or_else(|| {
                Fallo(
                    400,
                    format!("`{k}` son unidades de cómputo, de 0.25 a 2, no `{v}`"),
                )
            }),
    }
}

/// `dormir_tras`, si viene: segundos sin actividad hasta dormir; 0 = nunca; si no, de 60 s a 7 días.
fn dormir_tras(cuerpo: &Node) -> Result<Option<i32>, Fallo> {
    match cuerpo.get("dormir_tras").and_then(|(_, v)| v.as_str()) {
        None => Ok(None),
        Some(v) => v
            .parse::<i32>()
            .ok()
            .filter(|s| *s == 0 || (60..=604_800).contains(s))
            .map(Some)
            .ok_or_else(|| {
                Fallo(
                    400,
                    format!("`dormir_tras` son segundos: 0 (nunca) o de 60 a 604800, no `{v}`"),
                )
            }),
    }
}

/// P6·2 · Los límites de un endpoint vivo: CU mínimas y máximas y el tiempo hasta dormir.
/// Lo que no viene se queda. Las CU (y con ellas `max_connections`) valen en el siguiente
/// arranque del cómputo; `dormir_tras`, en cuanto lo mira el reconciliador (P6·3).
fn ajustar_endpoint(
    c: &mut Client,
    celda: &Celda,
    (p, r, e): (&str, &str, &str),
    cuerpo: &Node,
    dominio: Option<&str>,
) -> Result<Respuesta, Fallo> {
    let dormir = dormir_tras(cuerpo)?;
    let viene = |k: &str| cuerpo.get(k).is_some();
    if !viene("cu_min") && !viene("cu_max") && dormir.is_none() {
        return Err(Fallo(
            400,
            "nada que cambiar: `cu_min`, `cu_max` o `dormir_tras`".into(),
        ));
    }
    let mut tx = c.transaction()?;
    rama_viva(&mut tx, celda, p, r)?;
    let Some(f) = tx.query_opt(
        "select cu_min, cu_max from plano.endpoint
          where organizacion = $1 and proyecto = $2 and rama = $3 and id = $4 and deseado = 'vivo'
          for update",
        &[&celda.organizacion, &p, &r, &e],
    )?
    else {
        return Err(Fallo(404, format!("no hay ningún endpoint `{e}` en `{r}`")));
    };
    let (cu_min, cu_max) = (
        cu(cuerpo, "cu_min", f.get(0))?,
        cu(cuerpo, "cu_max", f.get(1))?,
    );
    if cu_min > cu_max {
        return Err(Fallo(400, "`cu_min` no puede pasar de `cu_max`".into()));
    }
    let op = nueva_operacion_de(&mut tx, celda, p, "configurar-endpoint", Some(r), Some(e))?;
    let fila = tx.query_one(
        &format!(
            "update plano.endpoint e
                set cu_min = $5, cu_max = $6, dormir_tras = coalesce($7, dormir_tras)
              where organizacion = $1 and proyecto = $2 and rama = $3 and id = $4
          returning {ENDPOINT}, {}",
            conexiones_sql()
        ),
        &[&celda.organizacion, &p, &r, &e, &cu_min, &cu_max, &dormir],
    )?;
    tx.commit()?;
    Ok(aceptada(
        &op,
        Some(("endpoint", endpoint_json(&fila, dominio))),
    ))
}

fn crear_endpoint(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    cuerpo: &Node,
    dominio: Option<&str>,
) -> Result<Respuesta, Fallo> {
    let texto = |k: &str| cuerpo.get(k).and_then(|(_, v)| v.as_str());
    let Some(id) = texto("id") else {
        return Err(Fallo(400, "falta `id`: el nombre del endpoint".into()));
    };
    if !id_valido(id) {
        return Err(Fallo(
            400,
            format!(
                "`{id}` no vale como id: `[a-z0-9-]`, de 1 a 63, sin guion al principio ni al final"
            ),
        ));
    }
    let tipo = texto("tipo").unwrap_or("lectura-escritura");
    if tipo != "lectura-escritura" && tipo != "lectura" {
        return Err(Fallo(
            400,
            format!("`tipo` es `lectura-escritura` o `lectura`, no `{tipo}`"),
        ));
    }
    let (cu_min, cu_max) = (cu(cuerpo, "cu_min", 0.25)?, cu(cuerpo, "cu_max", 1.0)?);
    if cu_min > cu_max {
        return Err(Fallo(400, "`cu_min` no puede pasar de `cu_max`".into()));
    }
    let dormir = dormir_tras(cuerpo)?.unwrap_or(300);
    let mut tx = c.transaction()?;
    if rama_viva(&mut tx, celda, p, r)? != "lista" {
        return Err(Fallo(409, format!("la rama `{r}` aún no está lista")));
    }
    let fila = match tx.query_one(
        &format!(
            "insert into plano.endpoint (organizacion, proyecto, rama, id, tipo, vm, cu_min, cu_max, dormir_tras)
             values ($1, $2, $3, $4, $5, {NUEVA_VM}, $6, $7, $8) returning {ENDPOINT}"
        ),
        &[&celda.organizacion, &p, &r, &id, &tipo, &cu_min, &cu_max, &dormir],
    ) {
        Ok(f) => f,
        // ⭐ El cerco, capa 1: lo cierra la base, también con dos peticiones a la vez.
        Err(e) if choca(&e, "endpoint_escritura_por_rama") => {
            return Err(Fallo(
                409,
                format!("la rama `{r}` ya tiene un endpoint de escritura: sólo puede haber uno"),
            ));
        }
        Err(e) if choca(&e, "endpoint_pkey") => {
            return Err(Fallo(409, format!("ya hay un endpoint `{id}` en `{p}`")));
        }
        Err(e) => return Err(e.into()),
    };
    let op = nueva_operacion_de(&mut tx, celda, p, "crear-endpoint", Some(r), Some(id))?;
    tx.commit()?;
    Ok(aceptada(
        &op,
        Some(("endpoint", endpoint_json(&fila, dominio))),
    ))
}

fn borrar_endpoint(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    e: &str,
) -> Result<Respuesta, Fallo> {
    let mut tx = c.transaction()?;
    rama_viva(&mut tx, celda, p, r)?;
    let Some(_) = tx.query_opt(
        "select 1 from plano.endpoint
          where organizacion = $1 and proyecto = $2 and rama = $3 and id = $4 and deseado = 'vivo'
          for update",
        &[&celda.organizacion, &p, &r, &e],
    )?
    else {
        return Err(no_hay_endpoint(e));
    };
    let op = nueva_operacion_de(&mut tx, celda, p, "borrar-endpoint", Some(r), Some(e))?;
    tx.execute(
        "update plano.endpoint set deseado = 'borrado', observado = 'borrando', generacion = generacion + 1
          where organizacion = $1 and proyecto = $2 and id = $3",
        &[&celda.organizacion, &p, &e],
    )?;
    tx.commit()?;
    Ok(aceptada(&op, None))
}

fn endpoint_json(f: &Row, dominio: Option<&str>) -> Json {
    let mut v = vec![
        ("id", Json::s(f.get::<_, String>(0))),
        ("rama", Json::s(f.get::<_, String>(1))),
        ("tipo", Json::s(f.get::<_, String>(2))),
        ("vm", Json::s(f.get::<_, String>(3))),
        (
            "cu",
            Json::obj([
                ("min", Json::s(f.get::<_, f64>(4).to_string())),
                ("max", Json::s(f.get::<_, f64>(5).to_string())),
            ]),
        ),
        (
            "estado",
            Json::obj([
                ("deseado", Json::s(f.get::<_, String>(6))),
                ("observado", Json::s(f.get::<_, String>(7))),
            ]),
        ),
        ("creado", Json::s(f.get::<_, String>(9))),
        // P6·2: segundos sin actividad hasta dormir; 0 = nunca.
        ("dormir_tras", Json::Int(f.get::<_, i32>(10) as i64)),
    ];
    // P6·3: la última actividad que dijo su compute_ctl y, dormido, desde cuándo.
    if let Some(t) = f.get::<_, Option<String>>(11) {
        v.push(("ultima_actividad", Json::s(t)));
    }
    if let Some(t) = f.get::<_, Option<String>>(12) {
        v.push(("dormido_en", Json::s(t)));
    }
    // P6·5: el cómputo que le sirve ahora (`pool-…` si salió del pool); dormido, ninguno.
    if let Some(t) = f.get::<_, Option<String>>(13) {
        v.push(("computo", Json::s(t)));
    }
    if let Some(d) = f.get::<_, Option<String>>(8) {
        v.push(("direccion", Json::s(d)));
    }
    // P5·7: el nombre público, por el proxy (TLS, puerto 5432); `-pooler`, el pool (P5·5).
    if let Some(dom) = dominio {
        let vm: String = f.get(3);
        v.push(("host", Json::s(format!("{vm}.{dom}"))));
        v.push(("host_pool", Json::s(format!("{vm}-pooler.{dom}"))));
    }
    // P5·5: cuántas conexiones admite (directas) y el pool por base (-pooler).
    if f.len() > 15 {
        let maximas = crate::especificacion::conexiones(f.get(14));
        v.push((
            "conexiones",
            Json::obj([
                ("maximas", Json::Int(maximas)),
                (
                    "pool_por_base",
                    Json::Int(crate::especificacion::pool_por_base(
                        maximas,
                        f.get::<_, i64>(15) as usize,
                    )),
                ),
            ]),
        ));
    }
    Json::obj(v)
}

// ── roles y bases (P4·4) ───────────────────────────────────────────────────

/// Un nombre de rol o de base: `[a-z_][a-z0-9_-]`, hasta 63 (Postgres lo cita).
pub fn nombre_valido(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 63
        && n.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && !n.starts_with("pg_")
}

const RESERVADOS: &[&str] = &[
    "cloud_admin",
    "neon_superuser",
    "public",
    "postgres",
    "zenith_admin",
    "template0",
    "template1",
];

fn nombre_de(cuerpo: &Node, k: &str) -> Result<String, Fallo> {
    let Some(n) = cuerpo.get(k).and_then(|(_, v)| v.as_str()) else {
        return Err(Fallo(400, format!("falta `{k}`")));
    };
    if !nombre_valido(n) || RESERVADOS.contains(&n) {
        return Err(Fallo(
            400,
            format!("`{n}` no vale: `[a-z_][a-z0-9_-]`, hasta 63, sin `pg_` y no uno reservado"),
        ));
    }
    Ok(n.to_string())
}

/// Lo que cambia los roles o las bases de una rama: una operación que lo
/// aplica a su cómputo de escritura (si hay alguno en marcha).
fn configurar(
    tx: &mut postgres::Transaction,
    celda: &Celda,
    p: &str,
    r: &str,
) -> Result<Row, Fallo> {
    nueva_operacion_de(tx, celda, p, "configurar-rama", Some(r), None)
}

fn roles(c: &mut Client, celda: &Celda, p: &str, r: &str) -> Result<Respuesta, Fallo> {
    rama_viva(c, celda, p, r)?;
    let filas = c.query(
        "select nombre, to_char(creado at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
           from plano.rol
          where organizacion = $1 and proyecto = $2 and rama = $3 and deseado = 'vivo' order by nombre",
        &[&celda.organizacion, &p, &r],
    )?;
    // ⛔ Nunca el verificador: ni siquiera eso sale de aquí.
    Ok(Respuesta::ok(Json::obj([(
        "roles",
        Json::Arr(
            filas
                .iter()
                .map(|f| {
                    Json::obj([
                        ("nombre", Json::s(f.get::<_, String>(0))),
                        ("creado", Json::s(f.get::<_, String>(1))),
                    ])
                })
                .collect(),
        ),
    )])))
}

/// `202` con la contraseña: la única vez que se enseña.
fn con_contrasena(op: &Row, nombre: &str, clave: String) -> Respuesta {
    let mut r = aceptada(op, None);
    if let Json::Obj(o) = &mut r.cuerpo {
        o.insert(
            "rol".into(),
            Json::obj([("nombre", Json::s(nombre)), ("contrasena", Json::s(clave))]),
        );
    }
    r
}

fn crear_rol(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    cuerpo: &Node,
) -> Result<Respuesta, Fallo> {
    let nombre = nombre_de(cuerpo, "nombre")?;
    let (clave, verificador) = crate::scram::nueva().map_err(|e| Fallo(500, e))?;
    let mut tx = c.transaction()?;
    rama_viva(&mut tx, celda, p, r)?;
    if tx.execute(
        "insert into plano.rol (organizacion, proyecto, rama, nombre, verificador)
         values ($1, $2, $3, $4, $5)
         on conflict (organizacion, proyecto, rama, nombre) do update
           set verificador = excluded.verificador, deseado = 'vivo', creado = now()
           where plano.rol.deseado = 'borrado'",
        &[&celda.organizacion, &p, &r, &nombre, &verificador],
    )? == 0
    {
        return Err(Fallo(409, format!("ya hay un rol `{nombre}` en `{r}`")));
    }
    let op = configurar(&mut tx, celda, p, r)?;
    tx.commit()?;
    Ok(con_contrasena(&op, &nombre, clave))
}

fn nueva_contrasena(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    n: &str,
) -> Result<Respuesta, Fallo> {
    let (clave, verificador) = crate::scram::nueva().map_err(|e| Fallo(500, e))?;
    let mut tx = c.transaction()?;
    rama_viva(&mut tx, celda, p, r)?;
    if tx.execute(
        "update plano.rol set verificador = $5
          where organizacion = $1 and proyecto = $2 and rama = $3 and nombre = $4 and deseado = 'vivo'",
        &[&celda.organizacion, &p, &r, &n, &verificador],
    )? == 0
    {
        return Err(Fallo(404, format!("no hay ningún rol `{n}` en `{r}`")));
    }
    let op = configurar(&mut tx, celda, p, r)?;
    tx.commit()?;
    Ok(con_contrasena(&op, n, clave))
}

fn borrar_rol(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    n: &str,
) -> Result<Respuesta, Fallo> {
    let mut tx = c.transaction()?;
    rama_viva(&mut tx, celda, p, r)?;
    let suyas: Vec<String> = tx
        .query(
            "select nombre from plano.base
              where organizacion = $1 and proyecto = $2 and rama = $3 and dueno = $4 and deseado = 'vivo'
              order by nombre",
            &[&celda.organizacion, &p, &r, &n],
        )?
        .iter()
        .map(|f| f.get(0))
        .collect();
    if !suyas.is_empty() {
        return Err(Fallo(
            409,
            format!("`{n}` es dueño de {}: bórralas antes", suyas.join(", ")),
        ));
    }
    if tx.execute(
        "update plano.rol set deseado = 'borrado'
          where organizacion = $1 and proyecto = $2 and rama = $3 and nombre = $4 and deseado = 'vivo'",
        &[&celda.organizacion, &p, &r, &n],
    )? == 0
    {
        return Err(Fallo(404, format!("no hay ningún rol `{n}` en `{r}`")));
    }
    let op = configurar(&mut tx, celda, p, r)?;
    tx.commit()?;
    Ok(aceptada(&op, None))
}

fn bases(c: &mut Client, celda: &Celda, p: &str, r: &str) -> Result<Respuesta, Fallo> {
    rama_viva(c, celda, p, r)?;
    let filas = c.query(
        "select nombre, dueno from plano.base
          where organizacion = $1 and proyecto = $2 and rama = $3 and deseado = 'vivo' order by nombre",
        &[&celda.organizacion, &p, &r],
    )?;
    Ok(Respuesta::ok(Json::obj([(
        "bases",
        Json::Arr(
            filas
                .iter()
                .map(|f| {
                    Json::obj([
                        ("nombre", Json::s(f.get::<_, String>(0))),
                        ("dueno", Json::s(f.get::<_, String>(1))),
                    ])
                })
                .collect(),
        ),
    )])))
}

fn crear_base(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    cuerpo: &Node,
) -> Result<Respuesta, Fallo> {
    let nombre = nombre_de(cuerpo, "nombre")?;
    let dueno = nombre_de(cuerpo, "dueno")?;
    let mut tx = c.transaction()?;
    rama_viva(&mut tx, celda, p, r)?;
    if tx
        .query_opt(
            "select 1 from plano.rol
              where organizacion = $1 and proyecto = $2 and rama = $3 and nombre = $4 and deseado = 'vivo'",
            &[&celda.organizacion, &p, &r, &dueno],
        )?
        .is_none()
    {
        return Err(Fallo(404, format!("no hay ningún rol `{dueno}` en `{r}` que pueda ser su dueño")));
    }
    if tx.execute(
        "insert into plano.base (organizacion, proyecto, rama, nombre, dueno)
         values ($1, $2, $3, $4, $5)
         on conflict (organizacion, proyecto, rama, nombre) do update
           set dueno = excluded.dueno, deseado = 'vivo', creada = now()
           where plano.base.deseado = 'borrado'",
        &[&celda.organizacion, &p, &r, &nombre, &dueno],
    )? == 0
    {
        return Err(Fallo(409, format!("ya hay una base `{nombre}` en `{r}`")));
    }
    let op = configurar(&mut tx, celda, p, r)?;
    tx.commit()?;
    Ok(aceptada(&op, None))
}

fn borrar_base(
    c: &mut Client,
    celda: &Celda,
    p: &str,
    r: &str,
    b: &str,
) -> Result<Respuesta, Fallo> {
    let mut tx = c.transaction()?;
    rama_viva(&mut tx, celda, p, r)?;
    if tx.execute(
        "update plano.base set deseado = 'borrado'
          where organizacion = $1 and proyecto = $2 and rama = $3 and nombre = $4 and deseado = 'vivo'",
        &[&celda.organizacion, &p, &r, &b],
    )? == 0
    {
        return Err(Fallo(404, format!("no hay ninguna base `{b}` en `{r}`")));
    }
    let op = configurar(&mut tx, celda, p, r)?;
    tx.commit()?;
    Ok(aceptada(&op, None))
}

fn no_hay_endpoint(id: &str) -> Fallo {
    Fallo(404, format!("no hay ningún endpoint `{id}`"))
}

fn no_hay_rama(id: &str) -> Fallo {
    Fallo(404, format!("no hay ninguna rama `{id}`"))
}

// ── quién entra (P5·6) ─────────────────────────────────────────────────────

/// Los protocolos con límite propio, como los nombra el proxy.
pub const PROTOCOLOS: [&str; 3] = ["tcp", "ws", "http"];

/// Lo que se elige de un proyecto para [`acceso_json`].
const ACCESO: &str = "ips_permitidas, bloquear_publico,
    (limites->'tcp'->>'por_segundo')::bigint, (limites->'tcp'->>'rafaga')::bigint,
    (limites->'ws'->>'por_segundo')::bigint, (limites->'ws'->>'rafaga')::bigint,
    (limites->'http'->>'por_segundo')::bigint, (limites->'http'->>'rafaga')::bigint";

/// Una entrada de la lista de IPs, como la entiende el proxy de Neon
/// (`parse_ip_pattern`): una IP, `IP/prefijo` o `IP-IP`, v4 o v6. Lo que el
/// proxy no entiende lo convierte en «ninguna IP» y deja fuera a todos: aquí no
/// pasa.
pub fn patron_ip_valido(p: &str) -> bool {
    use std::net::IpAddr;
    if let Some((ip, prefijo)) = p.split_once('/') {
        let Ok(ip) = ip.parse::<IpAddr>() else {
            return false;
        };
        let tope = if ip.is_ipv4() { 32 } else { 128 };
        return prefijo.parse::<u8>().is_ok_and(|n| n <= tope)
            && !prefijo.starts_with('+')
            && prefijo.len() <= 3;
    }
    if let Some((a, b)) = p.split_once('-') {
        return match (a.parse::<IpAddr>(), b.parse::<IpAddr>()) {
            (Ok(a), Ok(b)) => a.is_ipv4() == b.is_ipv4() && a <= b,
            _ => false,
        };
    }
    p.parse::<IpAddr>().is_ok()
}

/// Los límites de un protocolo: por segundo y ráfaga, enteros con techo.
fn limite_de(protocolo: &str, n: &Node) -> Result<(i64, i64), Fallo> {
    let num = |k: &str, tope: i64| -> Result<i64, Fallo> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| (1..=tope).contains(v))
            .ok_or_else(|| {
                Fallo(
                    400,
                    format!("`limites.{protocolo}.{k}`: un entero de 1 a {tope}"),
                )
            })
    };
    Ok((num("por_segundo", 100_000)?, num("rafaga", 1_000_000)?))
}

fn acceso_json(f: &Row) -> Json {
    let par = |i: usize| {
        Json::obj([
            ("por_segundo", Json::Int(f.get::<_, i64>(i))),
            ("rafaga", Json::Int(f.get::<_, i64>(i + 1))),
        ])
    };
    Json::obj([
        (
            "ips_permitidas",
            Json::Arr(
                f.get::<_, Vec<String>>(0)
                    .into_iter()
                    .map(Json::s)
                    .collect(),
            ),
        ),
        ("bloquear_publico", Json::Bool(f.get(1))),
        (
            "limites",
            Json::obj([("tcp", par(2)), ("ws", par(4)), ("http", par(6))]),
        ),
    ])
}

fn acceso(c: &mut Client, celda: &Celda, id: &str) -> Result<Respuesta, Fallo> {
    match c.query_opt(
        &format!(
            "select {ACCESO} from plano.proyecto
              where organizacion = $1 and id = $2 and deseado = 'vivo'"
        ),
        &[&celda.organizacion, &id],
    )? {
        Some(f) => Ok(Respuesta::ok(acceso_json(&f))),
        None => Err(no_hay_proyecto(id)),
    }
}

/// Cambia lo que venga en el cuerpo; lo demás se queda. Es una operación: al
/// quedar hecha, el proxy olvida lo que tenía guardado y lo nuevo vale ya.
fn cambiar_acceso(
    c: &mut Client,
    celda: &Celda,
    id: &str,
    cuerpo: &Node,
) -> Result<Respuesta, Fallo> {
    let ips = match cuerpo.get("ips_permitidas") {
        None => None,
        Some((_, Node::Sequence { items, .. })) => {
            let mut v = Vec::new();
            for i in items {
                let p = i.as_str().map(str::trim).unwrap_or_default();
                if !patron_ip_valido(p) {
                    return Err(Fallo(
                        400,
                        format!(
                            "`{p}` no es una IP, una subred (`203.0.113.0/24`) ni un rango (`203.0.113.1-203.0.113.9`)"
                        ),
                    ));
                }
                v.push(p.to_string());
            }
            if v.len() > 100 {
                return Err(Fallo(
                    400,
                    "como mucho 100 entradas en `ips_permitidas`".into(),
                ));
            }
            Some(v)
        }
        Some(_) => return Err(Fallo(400, "`ips_permitidas` es una lista".into())),
    };
    let bloquear = match cuerpo.get("bloquear_publico").map(|(_, v)| v.as_str()) {
        None => None,
        Some(Some("true")) => Some(true),
        Some(Some("false")) => Some(false),
        Some(_) => return Err(Fallo(400, "`bloquear_publico` es true o false".into())),
    };
    let mut limites = Vec::new();
    if let Some((_, l)) = cuerpo.get("limites") {
        for (k, v) in l.entries() {
            let k = k.as_str().unwrap_or_default();
            let Some(protocolo) = PROTOCOLOS.iter().find(|p| **p == k) else {
                return Err(Fallo(400, format!("`limites.{k}`: sólo tcp, ws y http")));
            };
            let (por_segundo, rafaga) = limite_de(protocolo, v)?;
            limites.push((
                *protocolo,
                Json::obj([
                    ("por_segundo", Json::Int(por_segundo)),
                    ("rafaga", Json::Int(rafaga)),
                ]),
            ));
        }
    }
    if ips.is_none() && bloquear.is_none() && limites.is_empty() {
        return Err(Fallo(
            400,
            "nada que cambiar: `ips_permitidas`, `bloquear_publico` o `limites`".into(),
        ));
    }
    let limites = Json::obj(limites).jcs();
    let mut tx = c.transaction()?;
    let Some(_) = tx.query_opt(
        "select 1 from plano.proyecto
          where organizacion = $1 and id = $2 and deseado = 'vivo' for update",
        &[&celda.organizacion, &id],
    )?
    else {
        return Err(no_hay_proyecto(id));
    };
    let op = nueva_operacion(&mut tx, celda, id, "configurar-acceso", None)?;
    let f = tx.query_one(
        &format!(
            "update plano.proyecto
                set ips_permitidas = coalesce($3, ips_permitidas),
                    bloquear_publico = coalesce($4, bloquear_publico),
                    limites = limites || $5::text::jsonb
              where organizacion = $1 and id = $2
          returning {ACCESO}"
        ),
        &[&celda.organizacion, &id, &ips, &bloquear, &limites],
    )?;
    tx.commit()?;
    Ok(aceptada(&op, Some(("acceso", acceso_json(&f)))))
}

fn no_hay_proyecto(id: &str) -> Fallo {
    Fallo(404, format!("no hay ningún proyecto `{id}`"))
}

/// `202`: la operación, y lo que ya se sabe del recurso.
fn aceptada(op: &Row, recurso: Option<(&'static str, Json)>) -> Respuesta {
    let mut cuerpo = vec![("operacion", operacion_json(op))];
    if let Some(r) = recurso {
        cuerpo.push(r);
    }
    Respuesta {
        codigo: 202,
        cuerpo: Json::obj(cuerpo),
    }
}

fn proyecto_json(f: &Row) -> Json {
    let mut v = vec![
        ("id", Json::s(f.get::<_, String>(0))),
        ("celda", Json::s(f.get::<_, String>(1))),
        ("creado", Json::s(f.get::<_, String>(3))),
        (
            "estado",
            Json::obj([
                ("deseado", Json::s(f.get::<_, String>(4))),
                ("observado", Json::s(f.get::<_, String>(5))),
                ("generacion", Json::Int(f.get::<_, i64>(6))),
            ]),
        ),
    ];
    if let Some(d) = f.get::<_, Option<String>>(2) {
        v.push(("dueno", Json::s(d)));
    }
    if let Some(t) = f.get::<_, Option<String>>(7) {
        v.push(("tenant", Json::s(t)));
    }
    Json::obj(v)
}

fn operacion_json(f: &Row) -> Json {
    let estado: String = f.get(3);
    let mut v = vec![
        ("id", Json::s(f.get::<_, String>(0))),
        ("tipo", Json::s(f.get::<_, String>(1))),
        ("proyecto", Json::s(f.get::<_, String>(2))),
        ("hecha", Json::Bool(estado != "en-curso")),
        ("estado", Json::s(estado)),
        ("creada", Json::s(f.get::<_, String>(5))),
    ];
    if let Some(e) = f.get::<_, Option<String>>(4) {
        v.push(("error", Json::s(e)));
    }
    if let Some(t) = f.get::<_, Option<String>>(6) {
        v.push(("terminada", Json::s(t)));
    }
    Json::obj(v)
}

fn analizar(cuerpo: &str) -> Result<Node, String> {
    if cuerpo.trim().is_empty() {
        return Err("el cuerpo está vacío".into());
    }
    parse::parse(cuerpo).map_err(|e| format!("el cuerpo no analiza: {e:?}"))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_ids() {
        for bueno in ["a", "ventas", "ventas-2026", "0", &"x".repeat(63)] {
            assert!(id_valido(bueno), "{bueno}");
        }
        for malo in [
            "",
            "-a",
            "a-",
            "Ventas",
            "ven_tas",
            "ventas.x",
            &"x".repeat(64),
        ] {
            assert!(!id_valido(malo), "{malo}");
        }
    }

    #[test]
    fn las_ips_como_las_entiende_el_proxy() {
        for bueno in [
            "203.0.113.7",
            "203.0.113.0/24",
            "0.0.0.0/0",
            "203.0.113.1-203.0.113.9",
            "2001:db8::1",
            "2001:db8::/32",
            "2001:db8::1-2001:db8::ff",
        ] {
            assert!(patron_ip_valido(bueno), "{bueno}");
        }
        for malo in [
            "",
            "203.0.113",
            "203.0.113.0/33",
            "2001:db8::/129",
            "203.0.113.0/+8",
            "203.0.113.9-203.0.113.1",
            "203.0.113.1-2001:db8::1",
            "ejemplo.com",
            " 203.0.113.7",
        ] {
            assert!(!patron_ip_valido(malo), "{malo}");
        }
    }

    #[test]
    fn las_rutas() {
        assert_eq!(
            ruta("POST", &["proyectos", "ventas", "acceso"]).ok(),
            Some(Pedido::CambiarAcceso("ventas"))
        );
        assert_eq!(
            ruta(
                "POST",
                &[
                    "proyectos",
                    "ventas",
                    "ramas",
                    "main",
                    "endpoints",
                    "principal",
                    "ajustes"
                ]
            )
            .ok(),
            Some(Pedido::AjustarEndpoint("ventas", "main", "principal"))
        );
        assert_eq!(
            ruta("DELETE", &["proyectos", "ventas", "acceso"])
                .err()
                .map(|r| r.codigo),
            Some(405)
        );
        assert_eq!(ruta("GET", &["proyectos"]).ok(), Some(Pedido::Proyectos));
        assert_eq!(
            ruta("POST", &["proyectos"]).ok(),
            Some(Pedido::CrearProyecto)
        );
        assert_eq!(
            ruta("DELETE", &["proyectos", "ventas"]).ok(),
            Some(Pedido::BorrarProyecto("ventas"))
        );
        assert_eq!(
            ruta("GET", &["operaciones", "op_1"]).ok(),
            Some(Pedido::Operacion("op_1"))
        );
        assert_eq!(
            ruta("PUT", &["proyectos"]).err().map(|r| r.codigo),
            Some(405)
        );
        assert_eq!(ruta("GET", &["ramas"]).err().map(|r| r.codigo), Some(404));
        // Un id que no puede existir es un 404, no un 400: no se distingue de uno que no está.
        assert_eq!(
            ruta("GET", &["proyectos", "Ventas"])
                .err()
                .map(|r| r.codigo),
            Some(404)
        );
        assert_eq!(
            ruta("GET", &["operaciones", "x"]).err().map(|r| r.codigo),
            Some(404)
        );
    }
}
