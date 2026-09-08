//! `fundar` — la primera organización, y su dueño.
//!
//! # Por qué esto es un mando y no una ruta
//!
//! Todos los demás verbos los hace una persona con un token. **Éste no puede**:
//! la primera organización se crea cuando todavía no existe ninguna persona con
//! potestad, así que no habría con qué autenticar la petición.
//!
//! Es la misma figura que `ore init`: **fundar es un acto de operador**, con las
//! credenciales del clúster, y deja su huella igual que los demás.
//!
//! ⛔ Y de ahí una regla que hay que sostener: **la cara de línea de órdenes no
//! debe crecer.** El día que `ore-iam conceder` exista por `argv`, existirá una
//! forma de conceder que no pasa por la identidad de nadie.
//!
//! # Lo que hace, y por qué es una sola transacción
//!
//! Cuatro escrituras que **no tienen sentido por separado**:
//!
//! ```text
//!   iam.organizacion    la organización
//!   iam.persona         su dueño, atado a `(emisor, sub)`
//!   iam.pertenencia     con rol `ORGADMIN`
//!   iam.huella          y el rastro de haberlo hecho
//! ```
//!
//! Una organización sin dueño no la puede administrar nadie; un dueño sin
//! organización no es nada. O las cuatro, o ninguna.
//!
//! # Idempotente, y eso importa en un Job
//!
//! Un `Job` de Kubernetes se puede reintentar. Si `fundar` creara una segunda
//! organización al repetirse, un reintento silencioso duplicaría el inquilino.
//! Se apoya en las claves: `(emisor, sub)` es única en `persona` y el nombre lo
//! es en `organizacion`, así que la segunda vez **no cambia nada y lo dice**.

use crate::base::{Tx, nuevo_id};
use ore_core::json::Json;
use ore_entrada::identidad::Identidad;
use postgres::Client;

pub struct Peticion<'a> {
    pub organizacion: &'a str,
    pub emisor: &'a str,
    pub sub: &'a str,
    pub correo: Option<&'a str>,
}

pub fn fundar(c: &mut Client, p: &Peticion) -> Result<Json, String> {
    // Quien funda es el OPERADOR, no el dueño. Se distingue en la huella: la
    // organización la crea la plataforma; el dueño es a quien se le entrega.
    let operador = Identidad {
        persona: "operador".into(),
        agente: Some("ore-iam fundar".into()),
        // Un operador no tiene correo aqui: no es una persona de `iam`, es
        // quien opera el cluster. La huella lo dice con su nombre.
        correo: None,
        nombre: None,
    };
    let mut tx = Tx::abrir(c, &operador)?;

    // ── ¿ya estaba? ─────────────────────────────────────────────────────────
    if let Some(f) = tx.uno(
        "select id from iam.organizacion where nombre = $1",
        &[&p.organizacion],
    )? {
        let id: String = f.get(0);
        return Ok(Json::obj([
            ("organizacion", Json::s(id)),
            ("nota", Json::s("ya existia: no se toco nada")),
        ]));
    }

    let org = nuevo_id("org");
    let persona = nuevo_id("per");

    // ── la persona, primero: la organizacion la referencia ──────────────────
    //
    // `on conflict` sobre `(emisor, sub)`: si esa persona ya existe —porque
    // funda su segunda organizacion— se reusa. Un mismo humano con dos
    // identificadores seria dos personas para el sistema.
    let persona = match tx.uno(
        "insert into iam.persona (id, emisor, sub, correo) values ($1, $2, $3, $4)
         on conflict (emisor, sub) do nothing returning id",
        &[&persona, &p.emisor, &p.sub, &p.correo],
    )? {
        Some(f) => f.get::<_, String>(0),
        None => tx
            .uno(
                "select id from iam.persona where emisor = $1 and sub = $2",
                &[&p.emisor, &p.sub],
            )?
            .ok_or("la persona no se creo y tampoco estaba")?
            .get::<_, String>(0),
    };

    tx.ejecutar(
        "insert into iam.organizacion (id, nombre, creada_por) values ($1, $2, $3)",
        &[&org, &p.organizacion, &persona],
    )?;

    // ── y el rol. `ORGADMIN` es UNO por organizacion, y lo sostiene un indice
    //    unico parcial: si algun dia esto se llamara dos veces con dos personas
    //    distintas, la base lo niega en vez de dejar dos dueños.
    tx.ejecutar(
        "insert into iam.pertenencia (persona, organizacion) values ($1, $2)",
        &[&persona, &org],
    )?;
    // ⭐⭐ Y el cargo aparte, con `otorgo` NULL: en una organizacion recien
    //   fundada NO HAY NADIE DENTRO que pueda conceder. Es el unico caso
    //   legitimo de la columna, y es el arranque de su `76` §5.
    tx.ejecutar(
        "insert into iam.pertenencia_rol (persona, organizacion, rol, otorgo)
         values ($1, $2, 'ORGADMIN', null)
         on conflict do nothing",
        &[&persona, &org],
    )?;

    tx.anotar(
        "organizacion:fundar",
        &org,
        Json::obj([
            ("organizacion", Json::s(p.organizacion)),
            ("dueno", Json::s(&persona)),
            ("emisor", Json::s(p.emisor)),
        ]),
    )?;
    tx.confirmar()?;

    Ok(Json::obj([
        ("organizacion", Json::s(org)),
        ("nombre", Json::s(p.organizacion)),
        ("dueno", Json::s(persona)),
    ]))
}
