//! **El contrato** (0058 P4·1): lo que una celda le pide a `ore-postgres`.
//!
//! ```text
//!   GET    /salud
//!   GET    /v1/postgres/proyectos                 los de la organización de la celda
//!   POST   /v1/postgres/proyectos                 {"id": "ventas", "dueno": "user:ana"}  → 202 + operación
//!   GET    /v1/postgres/proyectos/{p}
//!   DELETE /v1/postgres/proyectos/{p}             → 202 + operación
//!   GET    /v1/postgres/proyectos/{p}/ramas
//!   POST   /v1/postgres/proyectos/{p}/ramas       {"id": "dev", "padre": "main", "lsn" | "instante"}  → 202
//!   GET    /v1/postgres/proyectos/{p}/ramas/{r}
//!   DELETE /v1/postgres/proyectos/{p}/ramas/{r}   → 202 (`main` y una rama con hijas, no: 409)
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

impl Servidor {
    pub fn atender(&self, p: &Peticion) -> Respuesta {
        let seg = p.segmentos();
        if let ("GET", ["salud"]) = (p.metodo.as_str(), seg.as_slice()) {
            return Respuesta::ok(Json::obj([("ok", Json::Bool(true))]));
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
            Pedido::CrearProyecto | Pedido::CrearRama(_) => match analizar(&p.cuerpo) {
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
            Pedido::Operacion(id) => operacion(c, &celda, id),
            Pedido::Ramas(p) => ramas(c, &celda, p),
            Pedido::Rama(p, r) => rama(c, &celda, p, r),
            Pedido::CrearRama(p) => crear_rama(c, &celda, p, cuerpo.as_ref().expect("analizado")),
            Pedido::BorrarRama(p, r) => borrar_rama(c, &celda, p, r),
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
    Operacion(&'a str),
    Ramas(&'a str),
    CrearRama(&'a str),
    Rama(&'a str, &'a str),
    BorrarRama(&'a str, &'a str),
}

/// De método y camino (sin `/v1/postgres`) a lo que se pide.
pub fn ruta<'a>(metodo: &str, resto: &[&'a str]) -> Result<Pedido<'a>, Respuesta> {
    let pedido = match (metodo, resto) {
        ("GET", ["proyectos"]) => Pedido::Proyectos,
        ("POST", ["proyectos"]) => Pedido::CrearProyecto,
        ("GET", ["proyectos", p]) => Pedido::Proyecto(p),
        ("DELETE", ["proyectos", p]) => Pedido::BorrarProyecto(p),
        ("GET", ["operaciones", o]) => Pedido::Operacion(o),
        ("GET", ["proyectos", p, "ramas"]) => Pedido::Ramas(p),
        ("POST", ["proyectos", p, "ramas"]) => Pedido::CrearRama(p),
        ("GET", ["proyectos", p, "ramas", r]) => Pedido::Rama(p, r),
        ("DELETE", ["proyectos", p, "ramas", r]) => Pedido::BorrarRama(p, r),
        (
            _,
            ["proyectos"]
            | ["proyectos", _]
            | ["operaciones", _]
            | ["proyectos", _, "ramas"]
            | ["proyectos", _, "ramas", _],
        ) => {
            return Err(Respuesta::error(
                405,
                format!("`{metodo}` no se atiende aquí"),
            ));
        }
        _ => return Err(Respuesta::error(404, "no hay nada en ese camino")),
    };
    match &pedido {
        Pedido::Ramas(p) | Pedido::CrearRama(p) | Pedido::Rama(p, _) | Pedido::BorrarRama(p, _)
            if !id_valido(p) =>
        {
            Err(Respuesta::error(
                404,
                format!("no hay ningún proyecto `{p}`"),
            ))
        }
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
            let op = tx.query_one(
                &format!(
                    "insert into plano.operacion (id, organizacion, proyecto, tipo, celda)
                     values ({NUEVA_OPERACION}, $1, $2, 'crear-proyecto', $3)
                     returning {OPERACION}"
                ),
                &[&celda.organizacion, &id, &celda.id],
            )?;
            tx.commit()?;
            Ok(aceptada(&op, Some(("proyecto", proyecto_json(&fila)))))
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
    match tx.query_one(
        &format!(
            "insert into plano.operacion (id, organizacion, proyecto, tipo, celda, rama)
             values ({NUEVA_OPERACION}, $1, $2, $3, $4, $5) returning {OPERACION}"
        ),
        &[&celda.organizacion, &p, &tipo, &celda.id, &rama],
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

fn no_hay_rama(id: &str) -> Fallo {
    Fallo(404, format!("no hay ninguna rama `{id}`"))
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
    fn las_rutas() {
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
