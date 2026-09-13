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
    /// Cómo se llama su puerta. `None` ⇒ se deriva de `organizacion`.
    ///
    /// ⛔ **No es el alta.** Es el host por el que se llega a este inquilino
    /// desde fuera —`demo.ore.paladio.io`—, y existe porque la consola tiene
    /// que poder contestar «¿a qué URL le pregunto por el árbol de `acme`?».
    ///
    /// ⚠️ El derivado sirve para casi todos y el que no, es el que paga: un
    /// cliente que quiere `ontologia.acme.com`. Y ahí la fila nombra un host
    /// que no es nuestro, así que el certificado pasa a depender de que
    /// **ellos** muevan un registro DNS. La `022` lo argumenta.
    pub entrada: Option<&'a str>,
    /// Dónde va a correr. `None` ⇒ se funda igual, SIN celda, y se dice.
    ///
    /// ⭐ Es camino, no identidad — por eso no se deriva de `organizacion` como
    /// el árbol o la llave: no hay nada en el nombre de un cliente que diga en
    /// qué clúster vive. Sale de la configuración de la plataforma
    /// (`ORE_CELDA*`), que es quien sabe dónde está corriendo esto.
    ///
    /// ⛔ Y sin ella NO se niega a fundar. Una organización sin celda es una
    /// fila que la consola no puede pintar como clúster — que es exactamente lo
    /// que la `025` comprueba al migrar—, pero negarse a fundar sería peor:
    /// dejaría al cliente sin organización por un ajuste de plataforma.
    pub celda: Option<Celda<'a>>,
}

/// Dónde corre el plano del árbol de una organización. Ver la `025` y la 0024.
pub struct Celda<'a> {
    pub nombre: &'a str,
    /// `compartido` · `dedicado` · `byoc`. Lo comprueba la base, no esto.
    pub tier: &'a str,
    pub proveedor: &'a str,
    pub region: &'a str,
}

/// Un nombre de dominio, y el alfabeto lo fija RFC 1123 y no nosotros.
///
/// ⛔ Sin esquema, sin puerto y sin camino: lo que se guarda es el HOST. Un
/// `https://` sería carretera metida dentro de la identidad.
fn entrada_valida(s: &str) -> bool {
    let etiqueta = |t: &str| {
        !t.is_empty()
            && t.len() <= 63
            && !t.starts_with('-')
            && !t.ends_with('-')
            && t.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    s.len() <= 253 && s.contains('.') && s.split('.').all(etiqueta)
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

/// ⭐⭐ REGISTRAR UN AGENTE — un sujeto que no es nadie, y uno POR INQUILINO.
///
/// La `024` puso la tabla; esto es la unica puerta de escritura. Lo que hace es
/// pequeño y lo que decide no lo es: crear un sujeto al que despues alguien
/// concede `usar` sobre un secreto.
///
/// ⛔ NO concede nada. Registrar y conceder son dos actos, y fundirlos haria que
///   dar de alta un Job le diera acceso — que es exactamente lo que la `007`
///   evita al separar el sujeto de la concesion.
///
/// ⭐ Es idempotente: registrar dos veces devuelve el mismo agente. Un guion que
///   converge tiene que poder llamarlo sin mirar si ya lo hizo.
pub fn registrar_agente(
    c: &mut Client,
    org: &str,
    emisor: &str,
    sub: &str,
    nombre: Option<&str>,
) -> Result<Json, String> {
    // Quien registra es el OPERADOR, igual que en `fundar`. Un agente no se
    // registra a si mismo: alguien decide que ese `sub` puede ser un sujeto.
    let operador = Identidad {
        persona: "operador".into(),
        agente: Some("ore-iam agente".into()),
        correo: None,
        nombre: None,
    };
    let mut tx = Tx::abrir(c, &operador)?;
    // El nombre o el id: la misma cortesia que el custodio, y por lo mismo —
    // quien llama escribe `demo`, no `org_b7b98fdd…`.
    let org_id: String = tx
        .uno(
            "select id from iam.organizacion
              where id = $1 or nombre = $1
              order by (id = $1) desc limit 1",
            &[&org],
        )?
        .ok_or_else(|| format!("`{org}` no es ninguna organizacion"))?
        .get(0);

    if let Some(f) = tx.uno(
        "select id from iam.agente where emisor = $1 and sub = $2 and organizacion = $3",
        &[&emisor, &sub, &org_id],
    )? {
        let id: String = f.get(0);
        // ⛔ NO se confirma: `Tx` se niega a hacerlo si nadie anoto, y aqui no
        //   hay nada que anotar porque no ha cambiado nada. Leer no es un acto.
        //   La transaccion se deshace al soltarse, que es lo correcto para una
        //   lectura.
        return Ok(Json::obj([
            ("agente", Json::s(id)),
            ("organizacion", Json::s(org_id)),
            ("ya", Json::Bool(true)),
        ]));
    }

    let id = nuevo_id("age");
    tx.ejecutar(
        "insert into iam.agente (id, emisor, sub, organizacion, nombre)
         values ($1, $2, $3, $4, $5)",
        &[&id, &emisor, &sub, &org_id, &nombre],
    )?;

    // ── ⭐⭐ Y HEREDA `usar` SOBRE LOS SECRETOS QUE YA HAY ──────────────────
    //
    // El custodio concede `usar` a TODOS los agentes de la organizacion en el
    // momento de emitir un secreto (`ore-cofre`, al emitir). Un agente que
    // llega DESPUES no esta en esa lista: los secretos ya emitidos —las
    // credenciales de cada fuente— no le alcanzan, y su primer Job de catalogo
    // moriria con un 403 del custodio que nadie entenderia.
    //
    // ⇒ Al registrarse, copia las concesiones `usar` vivas de sus hermanos —los
    //   agentes que la organizacion ya tenia— recurso a recurso. Es la misma
    //   regla que el custodio aplica al emitir, extendida hacia atras: un
    //   agente de la organizacion puede usar sus secretos, los que hay y los
    //   que vengan.
    //
    // ⚠️ Se COPIA de `iam.concesion` y no se lee `cofre.secreto`: este papel no
    //   alcanza el esquema `cofre` (la `020`), y no le hace falta — el recurso
    //   ya esta escrito en cada concesion. Y `concedio` se hereda tambien: la
    //   persona que emitio sigue siendo quien concedio.
    //
    // ⛔ Lo que esto NO arregla, y se dice: la concesion sigue nombrando a un
    //   agente y no a «los agentes de la organizacion». Es el patron de nombrar
    //   la instancia en vez de la clase, y esta copia es el precio de no haber
    //   cambiado el modelo de concesion aqui.
    let heredadas = tx.filas(
        "select distinct on (c.recurso) c.recurso, c.concedio
           from iam.concesion_viva c
           join iam.agente a on a.id = c.sujeto
          where a.organizacion = $1 and a.id <> $2
            and c.rol = 'usar' and c.recurso like 'secreto/%'
            and not exists (select 1 from iam.concesion_viva h
                             where h.sujeto = $2 and h.recurso = c.recurso and h.rol = 'usar')
          order by c.recurso, c.desde",
        &[&org_id, &id],
    )?;
    for h in &heredadas {
        let recurso: String = h.get(0);
        let concedio: String = h.get(1);
        tx.ejecutar(
            "insert into iam.concesion (id, sujeto, recurso, rol, concedio, organizacion)
             values ($1, $2, $3, 'usar', $4, $5)",
            &[&nuevo_id("con"), &id, &recurso, &concedio, &org_id],
        )?;
    }

    // ⛔ Y queda escrito. `Tx` se niega a confirmar si nadie anoto, y aqui esa
    //   regla vale doble: registrar un sujeto de maquina sin dejar rastro seria
    //   crear autoridad en silencio.
    tx.anotar(
        "agente:registrar",
        &id,
        Json::obj([
            ("organizacion", Json::s(&org_id)),
            ("emisor", Json::s(emisor)),
            ("sub", Json::s(sub)),
            ("heredadas", Json::Int(heredadas.len() as i64)),
        ]),
    )?;
    tx.confirmar()?;
    Ok(Json::obj([
        ("agente", Json::s(id)),
        ("organizacion", Json::s(org_id)),
        ("ya", Json::Bool(false)),
        // Cuantos secretos ya emitidos puede usar desde ya. Se dice: un cero
        // aqui en una organizacion con fuentes es la senal de que algo no
        // cuadra, y un numero es la prueba de que el Job va a poder pedir.
        ("secretos_heredados", Json::Int(heredadas.len() as i64)),
    ]))
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
        "select id, arbol, kek, entrada from iam.organizacion where nombre = $1",
        &[&p.organizacion],
    )? {
        let id: String = f.get(0);
        // ⭐ Y se devuelve su árbol. Un reintento que sólo dijera «ya existia»
        //   obligaría a ir a buscar a la base el dato por el que se llamó.
        let arbol: String = f.get(1);
        let kek: String = f.get(2);
        let entrada: String = f.get(3);
        return Ok(Json::obj([
            ("organizacion", Json::s(id)),
            ("arbol", Json::s(arbol)),
            ("kek", Json::s(kek)),
            ("entrada", Json::s(entrada)),
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
    // ⭐ Y la puerta, tercera de la misma figura. El derivado es el que la `b`
    //   de la E6 sirve sin coste por cliente: un `Gateway` compartido y una
    //   `HTTPRoute` por inquilino bajo `*.ore.paladio.io`.
    let entrada = match p.entrada {
        Some(e) => e.to_string(),
        None => format!("{}.ore.paladio.io", p.organizacion),
    };
    if !entrada_valida(&entrada) {
        return Err(format!(
            "`{entrada}` no sirve como entrada. Es un HOST —`demo.ore.paladio.io`—: \
             minusculas, digitos y `-`, al menos dos etiquetas.\n  \
             Sin esquema, sin puerto y sin camino: eso es carretera, y la carretera \
             no va en la fila.\n  \
             Si el nombre de la organizacion no encaja, dilo con `--entrada`."
        ));
    }
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
        "insert into iam.organizacion (id, nombre, arbol, kek, entrada, creada_por)
         values ($1, $2, $3, $4, $5, $6)",
        &[&org, &p.organizacion, &arbol, &kek, &entrada, &persona],
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

    // ── y la celda, en el mismo acto. Es la 0024 ④: el clúster es un hecho
    //    del plano de control, y el momento en que se decide es éste.
    let celda_id = match &p.celda {
        Some(c) => {
            let id = nuevo_id("cel");
            tx.ejecutar(
                "insert into iam.celda (id, organizacion, nombre, tier, proveedor, region)
                 values ($1, $2, $3, $4, $5, $6)",
                &[&id, &org, &c.nombre, &c.tier, &c.proveedor, &c.region],
            )?;
            Some(id)
        }
        None => None,
    };

    tx.anotar(
        "organizacion:fundar",
        &org,
        Json::obj([
            ("organizacion", Json::s(p.organizacion)),
            ("dueno", Json::s(&persona)),
            ("emisor", Json::s(p.emisor)),
            ("arbol", Json::s(&arbol)),
            ("kek", Json::s(&kek)),
            ("entrada", Json::s(&entrada)),
            (
                "celda",
                match &p.celda {
                    Some(c) => Json::s(format!("{}/{}/{}", c.tier, c.proveedor, c.nombre)),
                    None => Json::s("ninguna"),
                },
            ),
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
        ("entrada", Json::s(&entrada)),
        (
            "arbol_nota",
            Json::s(
                "declarado, no creado: el repositorio lo aprovisiona quien puede salir a la red",
            ),
        ),
        (
            "celda",
            match celda_id {
                Some(id) => Json::s(id),
                // ⚠️ Y se dice. Una organizacion sin celda no la pinta la consola.
                None => Json::s("ninguna: ni `--celda` ni `ORE_CELDA`"),
            },
        ),
    ]))
}
