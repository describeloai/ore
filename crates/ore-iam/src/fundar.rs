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
    /// ⭐ El TÍTULO (035): cómo se llama de cara a la gente. `nombre` es el
    ///   identificador; esto es «Acme Corp S.L.». `None` ⇒ sin título.
    pub titulo: Option<&'a str>,
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
    /// El NOMBRE de su puerta —`ore-mesh.ore.paladio.io`—, al que la entrada
    /// de la organización tiene que resolver. La IP de detrás es carretera.
    /// Ver la `027` y la 0024-⑥.
    pub puerta: &'a str,
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

/// ⭐ Lo que la plataforma pone en una celda nueva y nadie elige al pedirla (0025
///   E6): el cluster compartido, su proveedor, su region y su puerta. Es lo que
///   `ORE_CELDA*` traen al servidor — la misma configuracion que `fundar` lee.
pub struct CeldaPlataforma {
    pub cluster: String,
    pub tier: String,
    pub proveedor: String,
    pub region: String,
    pub puerta: String,
}

impl CeldaPlataforma {
    pub fn como_celda(&self) -> Celda<'_> {
        Celda {
            nombre: &self.cluster,
            tier: &self.tier,
            proveedor: &self.proveedor,
            region: &self.region,
            puerta: &self.puerta,
        }
    }
}

/// ⭐⭐ PEDIR UNA CELDA (0025 E6): la segunda serverless de una organizacion.
///
/// La fila es la `spec`: nombre propio, cluster, tier y lo que de ellos se
/// deriva (`t-<nombre>/ontologia`, `<nombre>.ore.paladio.io`). Nace sin
/// `aprovisionada`: el reconciliador la levanta en su siguiente pasada y lo
/// dice el. Quien pide necesita `celda:crear` en la organizacion, que hoy solo
/// tiene `ORGADMIN`.
///
/// ⛔ Solo el tier `compartido` hoy. Un `dedicado` o `byoc` no es una fila: es
///   un cluster que alguien tiene que montar, y eso todavia no lo hace nadie.
pub fn crear_celda_en(
    tx: &mut Tx,
    emisor: &str,
    sujeto: &Identidad,
    org: &str,
    nombre: &str,
    tier: &str,
    plataforma: &CeldaPlataforma,
) -> Result<Json, String> {
    let org_id: String = tx
        .uno(
            "select id from iam.organizacion
              where id = $1 or nombre = $1
              order by (id = $1) desc limit 1",
            &[&org],
        )?
        // El mismo mensaje que «no puedes»: no confirmar que existe.
        .ok_or("no puedes hacer eso en esa organizacion")?
        .get(0);
    crate::potestad::exige(tx, emisor, &sujeto.persona, &org_id, "celda:crear")?;
    if !nombre_de_celda_valido(nombre) {
        return Err(format!(
            "`{nombre}` no sirve como nombre de celda: minuscula o digito, luego minusculas, \
             digitos o `-`, hasta 39. De aqui salen un namespace y un host."
        ));
    }
    if tier != "compartido" {
        return Err(format!(
            "el tier `{tier}` todavia no se pide desde aqui: hoy solo `compartido`. \
             Un cluster dedicado o propio se acuerda con una persona."
        ));
    }
    if tx
        .uno("select 1 from iam.celda where nombre = $1", &[&nombre])?
        .is_some()
    {
        return Err(format!("ya hay una celda que se llama `{nombre}`"));
    }
    let id = nuevo_id("cel");
    let arbol = format!("t-{nombre}/ontologia");
    let entrada = format!("{nombre}.ore.paladio.io");
    tx.ejecutar(
        "insert into iam.celda (id, organizacion, nombre, cluster, tier, proveedor, region, puerta, arbol, entrada)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        &[
            &id,
            &org_id,
            &nombre,
            &plataforma.cluster,
            &tier,
            &plataforma.proveedor,
            &plataforma.region,
            &plataforma.puerta,
            &arbol,
            &entrada,
        ],
    )?;
    tx.anotar(
        "celda:crear",
        &id,
        Json::obj([
            ("organizacion", Json::s(&org_id)),
            ("celda", Json::s(nombre)),
            ("tier", Json::s(tier)),
            ("cluster", Json::s(&plataforma.cluster)),
        ]),
    )?;
    Ok(Json::obj([
        ("celda", Json::s(id)),
        ("nombre", Json::s(nombre)),
        ("organizacion", Json::s(org_id)),
        ("arbol", Json::s(arbol)),
        ("entrada", Json::s(entrada)),
        ("estado", Json::s("activa")),
    ]))
}

/// ⭐ RETIRAR UNA CELDA (0025 E6): la fila pasa a `retirada` y el reconciliador
///   desmonta lo que habia levantado en su siguiente pasada. No se borra: la
///   fila es historia, y su nombre no se reusa.
///
/// ⛔ La celda de casa —la que se llama como la organizacion— NO se retira por
///   aqui: retirarla es retirar la organizacion, y eso es otro verbo con otro
///   peso.
pub fn retirar_celda_en(
    tx: &mut Tx,
    emisor: &str,
    sujeto: &Identidad,
    celda: &str,
) -> Result<(Json, bool), String> {
    let f = tx
        .uno(
            "select c.id, c.organizacion, o.nombre, c.estado
               from iam.celda c join iam.organizacion o on o.id = c.organizacion
              where c.nombre = $1",
            &[&celda],
        )?
        .ok_or("no puedes hacer eso en esa organizacion")?;
    let (id, org_id, org_nombre, estado): (String, String, String, String) =
        (f.get(0), f.get(1), f.get(2), f.get(3));
    crate::potestad::exige(tx, emisor, &sujeto.persona, &org_id, "celda:retirar")?;
    if celda == org_nombre {
        return Err(format!(
            "`{celda}` es la celda de casa de la organizacion: retirarla es retirar la \
             organizacion, y eso no se hace desde aqui"
        ));
    }
    if estado == "retirada" {
        return Ok((
            Json::obj([
                ("celda", Json::s(celda)),
                ("estado", Json::s("retirada")),
                ("ya", Json::Bool(true)),
            ]),
            false,
        ));
    }
    tx.ejecutar(
        "update iam.celda set estado = 'retirada' where id = $1",
        &[&id],
    )?;
    tx.anotar(
        "celda:retirar",
        &id,
        Json::obj([
            ("organizacion", Json::s(&org_id)),
            ("celda", Json::s(celda)),
        ]),
    )?;
    Ok((
        Json::obj([
            ("celda", Json::s(celda)),
            ("estado", Json::s("retirada")),
            ("ya", Json::Bool(false)),
        ]),
        true,
    ))
}

/// ⭐ EL PERFIL (035): título y logo. Quien lo cambia necesita `organizacion:editar`.
///   Se cambia lo que viene; lo que no viene se queda. Un logo vacío (`""`) lo quita.
pub fn editar_perfil_en(
    tx: &mut Tx,
    emisor: &str,
    sujeto: &Identidad,
    org: &str,
    titulo: Option<&str>,
    logo: Option<&str>,
) -> Result<Json, String> {
    let org_id: String = tx
        .uno(
            "select id from iam.organizacion
              where id = $1 or nombre = $1
              order by (id = $1) desc limit 1",
            &[&org],
        )?
        .ok_or("no puedes hacer eso en esa organizacion")?
        .get(0);
    crate::potestad::exige(tx, emisor, &sujeto.persona, &org_id, "organizacion:editar")?;
    if titulo.is_none() && logo.is_none() {
        return Err("no viene nada que cambiar: `titulo` y/o `logo`".into());
    }
    if let Some(t) = titulo {
        let t = t.trim();
        if t.is_empty() || t.chars().count() > 80 {
            return Err("el titulo va de 1 a 80 caracteres".into());
        }
        tx.ejecutar(
            "update iam.organizacion set titulo = $2 where id = $1",
            &[&org_id, &t],
        )?;
    }
    if let Some(l) = logo {
        if l.is_empty() {
            tx.ejecutar(
                "update iam.organizacion set logo = null where id = $1",
                &[&org_id],
            )?;
        } else {
            // La forma se comprueba aqui con una frase; el `check` de la 035 es la guarda.
            if !l.starts_with("data:image/") || !l.contains(";base64,") {
                return Err("el logo es una imagen embebida: `data:image/<tipo>;base64,…`".into());
            }
            if l.len() > 200_000 {
                return Err(
                    "el logo pesa mas de 200 KB: reducelo (un SVG o un PNG pequeño)".into(),
                );
            }
            tx.ejecutar(
                "update iam.organizacion set logo = $2 where id = $1",
                &[&org_id, &l],
            )?;
        }
    }
    tx.anotar(
        "organizacion:editar",
        &org_id,
        Json::obj([
            ("titulo", Json::Bool(titulo.is_some())),
            ("logo", Json::Bool(logo.is_some())),
        ]),
    )?;
    Ok(Json::obj([
        ("organizacion", Json::s(org_id)),
        ("titulo", Json::Bool(titulo.is_some())),
        ("logo", Json::Bool(logo.is_some())),
    ]))
}

/// El nombre de una celda: la etiqueta que la 029 exige (`celda_nombre_es_etiqueta`),
/// dicha con una frase antes de que la diga el `check`.
fn nombre_de_celda_valido(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 39
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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
/// ⛔ NO crea autoridad nueva. Registrar y conceder siguen siendo dos actos
///   —la `007`— y esto no concede nada que una persona tuviera que decidir.
///   Lo UNICO que hace ademas de registrar es extender al agente lo que el
///   custodio ya da a TODO agente de la organizacion al emitir un secreto:
///   `usar`. Un agente que llega despues de emitir recibe lo mismo que habria
///   recibido de estar antes, y nada mas. Ver el bloque de abajo.
///
/// ⭐ Es idempotente: registrar dos veces devuelve el mismo agente. Un guion que
///   converge tiene que poder llamarlo sin mirar si ya lo hizo. Y cada vez
///   hereda lo que le falte, asi que tambien sirve para ponerse al dia.
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
        tipo: None,
    };
    let mut tx = Tx::abrir(c, &operador)?;
    let (j, cambio) = registrar_agente_en(&mut tx, org, emisor, sub, nombre)?;
    if cambio {
        tx.confirmar()?;
    }
    // Si no cambio nada, `Tx` se deshace al soltarse: leer no es un acto.
    Ok(j)
}

/// ⭐ El nucleo, sobre una transaccion que trae quien llama (0025 E5): el Job
///   de operador y el verbo `POST /organizaciones/{org}/agentes` hacen
///   EXACTAMENTE lo mismo, y la huella dice quien fue — el operador, o el
///   aprovisionador con su `sub`. Devuelve si hubo algo que confirmar.
pub fn registrar_agente_en(
    tx: &mut Tx,
    org: &str,
    emisor: &str,
    sub: &str,
    nombre: Option<&str>,
) -> Result<(Json, bool), String> {
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

    // ¿Ya estaba? Entonces no se vuelve a escribir — pero SÍ hereda lo que le
    // falte (abajo): un agente registrado antes de que hubiera secretos, o antes
    // de que esta regla existiera, tiene que poder usarlos igual.
    let (id, ya) = match tx.uno(
        "select id from iam.agente where emisor = $1 and sub = $2 and organizacion = $3",
        &[&emisor, &sub, &org_id],
    )? {
        Some(f) => (f.get::<_, String>(0), true),
        None => {
            let id = nuevo_id("age");
            tx.ejecutar(
                "insert into iam.agente (id, emisor, sub, organizacion, nombre)
                 values ($1, $2, $3, $4, $5)",
                &[&id, &emisor, &sub, &org_id, &nombre],
            )?;
            (id, false)
        }
    };

    // ── ⭐⭐ Y HEREDA `usar` SOBRE LOS SECRETOS QUE LA ORGANIZACION TIENE ─────
    //
    // El custodio concede `usar` a los agentes de la organizacion AL EMITIR. Un
    // agente que llega DESPUES no esta en esa lista: los secretos ya emitidos
    // —las credenciales de cada fuente— no le alcanzan, y su primer Job de
    // catalogo moriria con un 403 del custodio que nadie entenderia.
    //
    // ⇒ Los secretos de la organizacion se VEN desde `iam.concesion`: todo
    //   secreto nace con una concesion `owner` a quien lo emitio, asi que
    //   «cada `recurso` `secreto/%` con una concesion viva en esta
    //   organizacion» es exactamente su lista de secretos. Se concede `usar`
    //   sobre cada uno que le falte, con `concedio` = quien lo emitio.
    //
    // ⛔ Y no «de los agentes que ya habia», que fue la primera version y CI
    //   la tumbo: el PRIMER agente de una organizacion con fuentes no tiene
    //   hermanos, y se quedaba con cero.
    //
    // ⚠️ Se lee `iam.concesion` y no `cofre.secreto`: este papel no alcanza el
    //   esquema `cofre` (la `020`), y no le hace falta.
    //
    // ⛔ Lo que esto NO arregla, y se dice: la concesion sigue nombrando a un
    //   agente y no a «los agentes de la organizacion». Es el patron de nombrar
    //   la instancia en vez de la clase, y esta copia es el precio de no haber
    //   cambiado el modelo de concesion aqui.
    let heredadas = tx.filas(
        "select distinct on (c.recurso) c.recurso, c.concedio
           from iam.concesion_viva c
          where c.organizacion = $1
            and c.recurso like 'secreto/%'
            and not exists (select 1 from iam.concesion_viva h
                             where h.sujeto = $2 and h.recurso = c.recurso and h.rol = 'usar')
          order by c.recurso, (c.rol = 'owner') desc, c.desde",
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

    if ya && heredadas.is_empty() {
        // ⛔ NO hay nada que confirmar: `Tx` se niega a hacerlo si nadie anoto,
        //   y aqui no ha cambiado nada. Leer no es un acto. Quien llama lo sabe
        //   por el `false`.
        return Ok((
            Json::obj([
                ("agente", Json::s(id)),
                ("organizacion", Json::s(org_id)),
                ("ya", Json::Bool(true)),
                ("secretos_heredados", Json::Int(0)),
            ]),
            false,
        ));
    }

    // ⛔ Y queda escrito. `Tx` se niega a confirmar si nadie anoto, y aqui esa
    //   regla vale doble: registrar un sujeto de maquina sin dejar rastro seria
    //   crear autoridad en silencio.
    tx.anotar(
        if ya {
            "agente:heredar"
        } else {
            "agente:registrar"
        },
        &id,
        Json::obj([
            ("organizacion", Json::s(&org_id)),
            ("emisor", Json::s(emisor)),
            ("sub", Json::s(sub)),
            ("heredadas", Json::Int(heredadas.len() as i64)),
        ]),
    )?;
    Ok((
        Json::obj([
            ("agente", Json::s(id)),
            ("organizacion", Json::s(org_id)),
            ("ya", Json::Bool(ya)),
            // Cuantos secretos puede usar desde ya que antes no podia. Un cero en
            // una organizacion con fuentes y sin haberlo corrido antes es la senal
            // de que algo no cuadra; un numero es la prueba de que el Job va a
            // poder pedir.
            ("secretos_heredados", Json::Int(heredadas.len() as i64)),
        ]),
        true,
    ))
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
        tipo: None,
    };
    let mut tx = Tx::abrir(c, &operador)?;
    let (j, cambio) = fundar_en(&mut tx, p)?;
    if cambio {
        tx.confirmar()?;
    }
    Ok(j)
}

/// ⭐ El nucleo de fundar, sobre una transaccion que trae quien llama (0025
///   E6): el mando de operador y `POST /organizaciones` hacen lo mismo, y la
///   huella dice quien — el operador, o la persona que pidio su cuenta.
///   Devuelve si hubo algo que confirmar («ya estaba» no lo es).
pub fn fundar_en(tx: &mut Tx, p: &Peticion) -> Result<(Json, bool), String> {
    // ── ¿ya estaba? ─────────────────────────────────────────────────────────
    // ⭐ El arbol y la entrada, DE LA CELDA (029, 0025-2): la primera celda de la
    //   organizacion, que se llama como ella. `kek` sigue siendo de la cuenta.
    //   Una organizacion fundada sin celda no tiene arbol que devolver, y se
    //   dice con un vacio y no con un invento.
    if let Some(f) = tx.uno(
        "select o.id, coalesce(c.arbol, ''), o.kek, coalesce(c.entrada, '')
           from iam.organizacion o
           left join iam.celda c on c.organizacion = o.id and c.nombre = o.nombre
          where o.nombre = $1",
        &[&p.organizacion],
    )? {
        let id: String = f.get(0);
        // ⭐ Y se devuelve su árbol. Un reintento que sólo dijera «ya existia»
        //   obligaría a ir a buscar a la base el dato por el que se llamó.
        let arbol: String = f.get(1);
        let kek: String = f.get(2);
        let entrada: String = f.get(3);
        return Ok((
            Json::obj([
                ("organizacion", Json::s(id)),
                ("arbol", Json::s(arbol)),
                ("kek", Json::s(kek)),
                ("entrada", Json::s(entrada)),
                ("nota", Json::s("ya existia: no se toco nada")),
            ]),
            false,
        ));
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

    // ⭐ La CUENTA: nombre, llave, quien la fundo. `arbol` y `entrada` ya no van
    //   aqui —son de la celda (029/030, 0025-2)— y la 031 borra las columnas.
    let titulo = p.titulo.map(str::trim).filter(|t| !t.is_empty());
    if titulo.is_some_and(|t| t.chars().count() > 80) {
        return Err("el titulo no cabe: hasta 80 caracteres".into());
    }
    tx.ejecutar(
        "insert into iam.organizacion (id, nombre, kek, creada_por, titulo)
         values ($1, $2, $3, $4, $5)",
        &[&org, &p.organizacion, &kek, &persona, &titulo],
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
            // ⛔ La puerta con el mismo alfabeto que la entrada, y dicho con una
            //   frase antes de que lo diga el `check` de la `027`.
            if !entrada_valida(c.puerta) {
                return Err(format!(
                    "`{}` no sirve como puerta de la celda. Es un HOST —`ore-mesh.ore.paladio.io`—: \
                     sin esquema, sin puerto, sin camino.",
                    c.puerta
                ));
            }
            let id = nuevo_id("cel");
            // ⭐ Desde la 029 la celda tiene nombre PROPIO y es el de la
            //   organizacion para la primera —`demo` → celda `demo`—: de el se
            //   deriva todo lo tecnico (`t-demo`, `demo.ore.paladio.io`). Lo que
            //   `--celda` trae es el CLUSTER (`ore-mesh`), y va a `cluster`.
            //   `arbol` y `entrada` se escriben AQUI y en la organizacion (R1
            //   de la 0025: las dos verdades, hasta que la 030 recorte una).
            tx.ejecutar(
                "insert into iam.celda (id, organizacion, nombre, cluster, tier, proveedor, region, puerta, arbol, entrada)
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                &[
                    &id,
                    &org,
                    &p.organizacion,
                    &c.nombre,
                    &c.tier,
                    &c.proveedor,
                    &c.region,
                    &c.puerta,
                    &arbol,
                    &entrada,
                ],
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
    Ok((
        Json::obj([
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
        ]),
        true,
    ))
}
