//! `invitar`, `admitir`, `conceder` y `revocar`.
//!
//! Los cuatro comparten forma, y no por casualidad: **el hecho y su huella en
//! la misma transacción**, y una guarda antes de tocar nada.
//!
//! ```text
//!   1  ¿quién eres?          ya lo dijo el token
//!   2  ¿puedes?              `potestad::exige`
//!   3  ¿puedes ESO?          `potestad::no_por_encima` — el rodeo
//!   4  el hecho
//!   5  la huella             y sin ella no se confirma
//! ```

use crate::base::{Tx, nuevo_id};
use crate::potestad;
use ore_core::json::Json;
use ore_entrada::identidad::Identidad;
use sha2::{Digest, Sha256};

/// El resumen de un vale. Se guarda esto; el vale nunca.
pub fn resumen(vale: &str) -> String {
    let mut h = Sha256::new();
    h.update(vale.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn quien(s: &Identidad) -> &str {
    &s.persona
}

// ── invitar ─────────────────────────────────────────────────────────────────

/// Emite una invitación y **devuelve el vale una sola vez**.
///
/// El vale no se puede volver a leer: de él sólo queda su resumen. Si se
/// pierde, se revoca y se emite otra — que es lo correcto, porque un vale que
/// se puede recuperar es un vale que alguien más puede recuperar.
pub fn invitar(
    tx: &mut Tx,
    sujeto: &Identidad,
    emisor: &str,
    org: &str,
    correo: &str,
    rol: &str,
    dias: i64,
) -> Result<Json, String> {
    let mio = potestad::exige(tx, emisor, quien(sujeto), org, "administrador")?;
    let ord = potestad::ordinal_de(tx, rol)?;
    potestad::no_por_encima(&mio, rol, ord)?;

    // ⛔⛔ Y a `dueno` NO se invita. Lo descubrio la prueba: la guarda del
    //   rodeo deja pasar «un dueño invita a otro dueño» —no es por encima de
    //   el— y `005-la-pertenencia.sql` tiene un indice unico parcial que dice
    //   que el dueño es UNO. ⇒ se habria emitido un vale que **nadie puede
    //   canjear**, y el fallo aparecería una semana despues, en la cara de
    //   quien lo intenta.
    //
    // ⭐ Y el arreglo no es relajar el indice: es que cambiar de dueño no es
    //   invitar. `002-el-papel.sql` ya lo dice — «dueno: ademas TRASPASA» —, y
    //   traspasar es un verbo que todavia no existe. Mejor negarlo con su
    //   motivo que emitir algo que no sirve.
    if rol == "dueno" {
        return Err(concat!(
            "no se invita a `dueno`: una organizacion tiene UNO, y cambiarlo ",
            "es traspasarla, no invitar. Ese verbo todavia no existe"
        )
        .into());
    }

    // El correo es una CITA, no una identidad: se pliega. El `sub` NO se
    // pliega nunca, y esa distincion es de `021` y vale entera.
    let correo = correo.trim().to_lowercase();
    if !correo.contains('@') {
        return Err("eso no parece un correo".into());
    }

    let id = nuevo_id("inv");
    let vale = nuevo_id("vale");
    let res = resumen(&vale);

    let quien_id = persona_id(tx, emisor, quien(sujeto))?;
    tx.ejecutar(
        "insert into iam.invitacion
           (id, organizacion, correo, rol, invito, caduca_en, vale_resumen)
         values ($1, $2, $3, $4, $5, now() + ($6 || ' days')::interval, $7)",
        &[&id, &org, &correo, &rol, &quien_id, &dias.to_string(), &res],
    )?;
    tx.anotar(
        "invitacion:emitir",
        &id,
        Json::obj([
            ("organizacion", Json::s(org)),
            ("correo", Json::s(&correo)),
            ("rol", Json::s(rol)),
        ]),
    )?;

    Ok(Json::obj([
        ("invitacion", Json::s(id)),
        ("rol", Json::s(rol)),
        // ⚠️ La UNICA vez que este valor existe fuera de un correo.
        ("vale", Json::s(vale)),
        (
            "aviso",
            Json::s("el vale no se puede volver a leer: solo se guardo su resumen"),
        ),
    ]))
}

// ── admitir ─────────────────────────────────────────────────────────────────

/// Redime un vale. Crea la persona si no existía y la hace miembro.
///
/// ⛔ El correo del token tiene que ser el de la invitación. Una invitación a
/// `a@x` redimida por `b@y` es un traspaso que nadie autorizó — y el sistema no
/// tendría forma de contar quién lo hizo.
pub fn admitir(tx: &mut Tx, sujeto: &Identidad, emisor: &str, vale: &str) -> Result<Json, String> {
    let res = resumen(vale);
    let f = tx
        .uno(
            "select id, organizacion, correo, rol,
                    (revocada_en is not null) as revocada,
                    (redimida_en is not null) as redimida,
                    (caduca_en < now())       as caducada
               from iam.invitacion where vale_resumen = $1",
            &[&res],
        )?
        // El mismo mensaje que una caducada: decir «ese vale no existe» le
        // confirma a quien prueba vales cuáles sí.
        .ok_or("ese vale no sirve")?;

    let (id, org, correo, rol): (String, String, String, String) =
        (f.get(0), f.get(1), f.get(2), f.get(3));
    if f.get::<_, bool>(4) || f.get::<_, bool>(5) || f.get::<_, bool>(6) {
        return Err("ese vale no sirve".into());
    }

    match sujeto.correo.as_deref() {
        Some(c) if c.trim().to_lowercase() == correo => {}
        _ => {
            return Err(
                "ese vale es para otra persona: el correo del token no es el de la invitacion"
                    .into(),
            );
        }
    }

    let persona = crear_o_hallar(tx, emisor, quien(sujeto), Some(&correo))?;

    tx.ejecutar(
        "insert into iam.pertenencia (persona, organizacion, rol) values ($1, $2, $3)
         on conflict (persona, organizacion) do nothing",
        &[&persona, &org, &rol],
    )?;
    tx.ejecutar(
        "update iam.invitacion set redimida_en = now(), redimio = $2 where id = $1",
        &[&id, &persona],
    )?;
    tx.anotar(
        "invitacion:redimir",
        &id,
        Json::obj([
            ("organizacion", Json::s(&org)),
            ("persona", Json::s(&persona)),
            ("rol", Json::s(&rol)),
        ]),
    )?;

    Ok(Json::obj([
        ("organizacion", Json::s(org)),
        ("persona", Json::s(persona)),
        ("rol", Json::s(rol)),
    ]))
}

// ── conceder ────────────────────────────────────────────────────────────────

/// Concede un rol **de recurso** sobre un recurso.
///
/// ⛔⛔ Y aquí va escrito lo que quien implemente el decisor no puede olvidar:
/// **la concesión puede NEGAR, no puede conceder por encima del conducto.** Una
/// fila de esta tabla que ensanchara lo que el retículo de la ontología cerró
/// convertiría el gobierno del flujo en una sugerencia.
///
/// # ⛔ Y por qué `owner` se niega hoy
///
/// La guarda de verdad es de `modelo/puerta/dueno.mjs`: *«para nombrar dueño de
/// un ámbito hay que ser dueño de ese ámbito — y por el cierre reflexivo, serlo
/// de cualquiera que lo contenga sirve»*. Eso es **una travesía del árbol**, y
/// nuestro árbol es `paquete → vista` y vive en la forja: `ore-iam` todavía no
/// habla con él.
///
/// Mientras no exista esa travesía, la única guarda posible sería «eres
/// administrador», que es más ancha que la correcta — un administrador podría
/// nombrar owner de cualquier cosa. ⇒ se niega. **Omitir es cerrar, no abrir**,
/// y es la misma respuesta que `invitar` le da a `dueno`.
#[allow(clippy::too_many_arguments)]
pub fn conceder(
    tx: &mut Tx,
    sujeto: &Identidad,
    emisor: &str,
    org: &str,
    a_quien: &str,
    recurso: &str,
    rol: &str,
) -> Result<Json, String> {
    potestad::exige(tx, emisor, quien(sujeto), org, "administrador")?;
    // ⛔ Contra la tabla del plano de ABAJO, que no tiene ordinal: `owner` no
    //   implica `lector`, asi que no hay altura que comparar. Ver la `011`.
    potestad::rol_de_recurso(tx, rol)?;
    if rol == "owner" {
        return Err(
            "todavia no se concede `owner`: nombrarlo exige ser owner de ese \
                    ambito o de uno que lo contenga, y esa travesia del arbol no \
                    existe aun. Omitir es cerrar, no abrir"
                .into(),
        );
    }

    let id = nuevo_id("con");
    let quien_id = persona_id(tx, emisor, quien(sujeto))?;
    tx.ejecutar(
        "insert into iam.concesion (id, sujeto, recurso, rol, concedio, organizacion)
         values ($1, $2, $3, $4, $5, $6)",
        &[&id, &a_quien, &recurso, &rol, &quien_id, &org],
    )?;
    tx.anotar(
        "concesion:conceder",
        &id,
        Json::obj([
            ("organizacion", Json::s(org)),
            ("sujeto", Json::s(a_quien)),
            ("recurso", Json::s(recurso)),
            ("rol", Json::s(rol)),
        ]),
    )?;
    Ok(Json::obj([("concesion", Json::s(id))]))
}

// ── revocar ─────────────────────────────────────────────────────────────────

/// ⛔ Un solo mensaje para «no existe» y «no es de tu organizacion». Dos
/// mensajes distintos son un directorio de lo ajeno, consultable id a id.
const AJENA: &str = "no hay tal concesion en una organizacion tuya";

/// Revoca una concesión. **No la borra.**
///
/// La fila se queda con `revocada_en` y `revoco`, y la vista `concesion_viva`
/// deja de verla. La tabla guarda también lo que VALIÓ, que es lo único que
/// hace auditable una revocación: sin eso, «nunca tuvo permiso» y «se lo
/// quitamos» son indistinguibles.
pub fn revocar(tx: &mut Tx, sujeto: &Identidad, emisor: &str, id: &str) -> Result<Json, String> {
    let f = tx
        .uno(
            "select organizacion, (revocada_en is not null) from iam.concesion where id = $1",
            &[&id],
        )?
        // ⛔ El MISMO mensaje que «no es de tu organizacion», abajo. Ver `0021`.
        .ok_or(AJENA)?;
    let org: String = f.get(0);

    // ⛔⛔ EL ORDEN, y es la primera consecuencia de `0021`.
    //
    //   Antes esto distinguia «no hay tal concesion» de «ya estaba revocada»
    //   ANTES de exigir nada. Con una organizacion por persona era inofensivo.
    //   Con varias es una SONDA ENTRE INQUILINOS: un administrador de A
    //   averigua, id a id, que concesiones existen en B y cuales siguen vivas.
    //
    //   Su propia frase estaba en `exige` y no la habiamos extendido al orden:
    //   decir «no eres administrador de esa organizacion» le confirma a quien
    //   pregunta que esa organizacion existe.
    potestad::exige(tx, emisor, quien(sujeto), &org, "administrador").map_err(|_| AJENA)?;

    // Y esto SOLO despues de saber que es suya: a partir de aqui, contar la
    // verdad no le dice a nadie nada que no pudiera ver de todas formas.
    if f.get::<_, bool>(1) {
        return Err("esa concesion ya estaba revocada".into());
    }

    let quien_id = persona_id(tx, emisor, quien(sujeto))?;
    tx.ejecutar(
        "update iam.concesion set revocada_en = now(), revoco = $2 where id = $1",
        &[&id, &quien_id],
    )?;
    tx.anotar(
        "concesion:revocar",
        id,
        Json::obj([("organizacion", Json::s(&org))]),
    )?;
    Ok(Json::obj([
        ("concesion", Json::s(id)),
        ("revocada", Json::Bool(true)),
    ]))
}

// ── el sujeto, en la tabla ──────────────────────────────────────────────────

fn persona_id(tx: &mut Tx, emisor: &str, sub: &str) -> Result<String, String> {
    tx.uno(
        "select id from iam.persona where emisor = $1 and sub = $2",
        &[&emisor, &sub],
    )?
    .map(|f| f.get(0))
    .ok_or_else(|| "quien pide no es una persona conocida aqui".to_string())
}

fn crear_o_hallar(
    tx: &mut Tx,
    emisor: &str,
    sub: &str,
    correo: Option<&str>,
) -> Result<String, String> {
    if let Some(f) = tx.uno(
        "select id from iam.persona where emisor = $1 and sub = $2",
        &[&emisor, &sub],
    )? {
        return Ok(f.get(0));
    }
    let id = nuevo_id("per");
    tx.ejecutar(
        "insert into iam.persona (id, emisor, sub, correo) values ($1, $2, $3, $4)",
        &[&id, &emisor, &sub, &correo],
    )?;
    Ok(id)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_resumen_es_sha256_en_hexadecimal() {
        let r = resumen("hola");
        assert_eq!(r.len(), 64);
        assert!(r.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(resumen("hola"), r);
        assert_ne!(resumen("hola "), r);
    }

    /// La guarda del rodeo, que es la razon de que `potestad` exista.
    #[test]
    fn nadie_otorga_por_encima_de_si_mismo() {
        let admin = potestad::Rol {
            nombre: "administrador".into(),
            ordinal: 3,
        };
        assert!(potestad::no_por_encima(&admin, "miembro", 2).is_ok());
        assert!(potestad::no_por_encima(&admin, "administrador", 3).is_ok());
        assert!(potestad::no_por_encima(&admin, "dueno", 4).is_err());
    }
}
