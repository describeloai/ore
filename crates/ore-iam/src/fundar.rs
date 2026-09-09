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
//!   iam.organizacion    la organización, Y CÓMO SE LLAMA SU ÁRBOL
//!   iam.persona         su dueño, atado a `(emisor, sub)`
//!   iam.pertenencia     con rol `ORGADMIN`
//!   iam.huella          y el rastro de haberlo hecho
//! ```
//!
//! Una organización sin dueño no la puede administrar nadie; un dueño sin
//! organización no es nada. O las cuatro, o ninguna.
//!
//! # El árbol se DECLARA aquí, y no se crea aquí
//!
//! Es el mismo argumento que sostiene lo de arriba, un piso más allá: una
//! organización sin árbol no es un inquilino, es una fila. Pero **los dos actos
//! no pueden ser uno** — esto es una transacción y crear un repositorio en la
//! forja es un efecto externo; si la transacción se deshace después, el
//! repositorio queda, y si el repositorio falla después de confirmar, la fila
//! queda.
//!
//! ⇒ Se elige cuál es la verdad, y es **la fila**. Aquí se escribe el nombre
//! que le toca; quien pueda salir a la red converge hacia él. Quien llegue
//! antes se encuentra «tu árbol todavía no está aprovisionado», que dice qué
//! falta. Al revés quedarían repositorios huérfanos, y a esos no los ve nadie.
//!
//! ⛔ Y este binario no podría crearlo aunque quisiera: sale de `scratch` y
//! depende de `ore-core`, `ore-entrada`, `postgres` y `sha2`. Sin shell, sin
//! `git`, sin certificados, sin cliente HTTP. No es código que falte — es la
//! imagen, y es la misma que `ore-serve` tuvo que ceder para poder empujar.
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
    /// Cómo se llama su llave maestra. `None` ⇒ se deriva de `organizacion`.
    ///
    /// ⚠️ La `019` la puso `not null`, y este verbo no la escribía: fundar una
    /// organización nueva reventaba con una violación de `not null`. En el
    /// clúster no se vio —las dos que había se rellenaron en la migración— y lo
    /// cazó CI al fundar una de cero. Es exactamente el agujero que
    /// `las-migraciones.sh` existe para tapar, un piso más allá: **una migración
    /// puede sobrevivir a los datos que hay y romper lo que viene después**.
    pub kek: Option<&'a str>,
    /// Cómo se llama su árbol. `None` ⇒ se deriva de `organizacion`.
    ///
    /// ⭐ Derivar el valor por defecto es P2 —lo derivable no se pregunta—, y
    /// guardarlo aun así no lo contradice: es la misma figura que `ore source
    /// add`, que deriva el nombre de la variable y lo escribe en el manifiesto.
    /// Lo que no se puede es derivarlo CADA VEZ, porque `nombre` cambia y un
    /// repositorio no.
    pub arbol: Option<&'a str>,
}

/// `<propietario>/<repositorio>`, el mismo alfabeto que la `017` comprueba.
///
/// ⛔ Se comprueba aquí ADEMÁS de en la base para poder decirlo con una frase.
/// La guarda es la restricción; esto es la cortesía de no contestar con una
/// violación de `check` cuando lo que pasa es que el nombre lleva un espacio.
fn arbol_valido(s: &str) -> bool {
    let segmento = |t: &str, punto: bool| {
        !t.is_empty()
            && t.len() <= 63
            && t.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && t.chars().all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '_'
                    || c == '-'
                    || (punto && c == '.')
            })
    };
    match s.split_once('/') {
        Some((duenno, repo)) => segmento(duenno, false) && segmento(repo, true),
        None => false,
    }
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
        "select id, arbol, kek from iam.organizacion where nombre = $1",
        &[&p.organizacion],
    )? {
        let id: String = f.get(0);
        // ⭐ Y se devuelve su árbol. Un reintento que sólo dijera «ya existia»
        //   obligaría a ir a buscar a la base el dato por el que se llamó.
        let arbol: String = f.get(1);
        let kek: String = f.get(2);
        return Ok(Json::obj([
            ("organizacion", Json::s(id)),
            ("arbol", Json::s(arbol)),
            ("kek", Json::s(kek)),
            ("nota", Json::s("ya existia: no se toco nada")),
        ]));
    }

    // ── cómo se llama su árbol ──────────────────────────────────────────────
    let arbol = match p.arbol {
        Some(a) => a.to_string(),
        None => format!("t-{}/ontologia", p.organizacion),
    };
    // ⭐ La llave, con la misma forma y el mismo argumento: `<llavero>/<clave>`,
    //   el NOMBRE y no la carretera. El llavero por defecto es el nuestro; el
    //   día que un cliente traiga el suyo, `--kek` lo dice.
    let kek = match p.kek {
        Some(k) => k.to_string(),
        None => format!("ore/{}", p.organizacion),
    };
    if !arbol_valido(&kek) {
        return Err(format!(
            "`{kek}` no sirve como nombre de llave. Es `<llavero>/<clave>`, con el \
             mismo alfabeto que el arbol: acaba dentro del nombre de un recurso de la nube."
        ));
    }
    if !arbol_valido(&arbol) {
        return Err(format!(
            "`{arbol}` no sirve como nombre de arbol.\n  \
             Es `<propietario>/<repositorio>`: minuscula o digito, luego minusculas, \
             digitos, `_` o `-` (y `.` en el segundo).\n  \
             Acaba dentro de una URL de clon, asi que el alfabeto es cerrado.\n  \
             Si el nombre de la organizacion no encaja, dilo con `--arbol`."
        ));
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
        "insert into iam.organizacion (id, nombre, arbol, kek, creada_por)
         values ($1, $2, $3, $4, $5)",
        &[&org, &p.organizacion, &arbol, &kek, &persona],
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
            ("arbol", Json::s(&arbol)),
            ("kek", Json::s(&kek)),
        ]),
    )?;
    tx.confirmar()?;

    Ok(Json::obj([
        ("organizacion", Json::s(org)),
        ("nombre", Json::s(p.organizacion)),
        ("dueno", Json::s(persona)),
        // ⚠️ Y se dice que TODAVIA NO EXISTE. Devolver el nombre a secas se
        //   leeria como «hecho», y lo que se ha hecho es apuntarlo.
        ("arbol", Json::s(&arbol)),
        ("kek", Json::s(&kek)),
        (
            "arbol_nota",
            Json::s(
                "declarado, no creado: el repositorio lo aprovisiona quien puede salir a la red",
            ),
        ),
    ]))
}
