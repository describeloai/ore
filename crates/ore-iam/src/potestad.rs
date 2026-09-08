//! Quién puede qué — y **el rodeo que esto existe para cerrar**.
//!
//! # La guarda que no es obvia
//!
//! La escalera de roles de la organización es evidente: un administrador puede
//! más que un miembro. Lo que no es evidente es que hace falta una segunda
//! comprobación, y su motivo lo escribió la plataforma con la cicatriz al lado:
//!
//! > *«sin esa segunda potestad, un USERADMIN invitaría a un cómplice como
//! > ACCOUNTADMIN y tendría **por un rodeo** el poder que se le niega.»*
//!
//! ⇒ **Nadie puede otorgar un rol más alto que el suyo.** Sin eso, `invitar` es
//! una escalada de privilegio con forma de cortesía.
//!
//! # ⛔ Y esa guarda es SÓLO del plano de arriba
//!
//! Hasta la `011` también se aplicaba a `conceder`, comparando mi altura en la
//! pertenencia contra la del rol que doy sobre un recurso. Son dos escalas
//! distintas sobre la misma recta numérica: parecía funcionar porque los números
//! existían, no porque significaran lo mismo.
//!
//! Abajo **no hay altura**: `owner` no implica `lector`, así que no hay nada que
//! comparar. Lo que gobierna allí es ser owner del ámbito — y eso es una
//! travesía del árbol, no una resta.
//!
//! # Y el sujeto se busca por `(emisor, sub)`, nunca por `sub` a secas
//!
//! Un `sub` sin su emisor no identifica a nadie — es el mismo error que aceptar
//! un token sin mirar el `iss`. Dos emisores pueden traer el mismo `sub` y
//! serían dos personas distintas viendo la misma organización.

use crate::base::Tx;

/// El rol de alguien **en una organización**, con su altura.
pub struct Rol {
    pub nombre: String,
    pub ordinal: i16,
}

/// `None` si esa persona no pertenece a esa organización.
pub fn rol_de(tx: &mut Tx, emisor: &str, sub: &str, org: &str) -> Result<Option<Rol>, String> {
    let f = tx.uno(
        "select pe.rol, r.ordinal
           from iam.pertenencia pe
           join iam.persona p on p.id = pe.persona
           join iam.rol     r on r.nombre = pe.rol
          where p.emisor = $1 and p.sub = $2 and pe.organizacion = $3",
        &[&emisor, &sub, &org],
    )?;
    Ok(f.map(|f| Rol {
        nombre: f.get(0),
        ordinal: f.get(1),
    }))
}

pub fn ordinal_de(tx: &mut Tx, rol: &str) -> Result<i16, String> {
    tx.uno("select ordinal from iam.rol where nombre = $1", &[&rol])?
        .map(|f| f.get(0))
        .ok_or_else(|| format!("`{rol}` no es un rol de organizacion"))
}

/// ⛔ Sin ordinal, y por eso esto sólo comprueba que **existe**. Ver la `011`:
/// una altura aquí sería `owner ⇒ lector` escrito en una columna.
pub fn rol_de_recurso(tx: &mut Tx, rol: &str) -> Result<(), String> {
    tx.uno(
        "select 1 from iam.rol_de_recurso where nombre = $1",
        &[&rol],
    )?
    .map(|_| ())
    .ok_or_else(|| format!("`{rol}` no es un rol de recurso. Son `lector` y `owner`"))
}

/// Exige al menos `minimo`, y devuelve lo que tiene.
///
/// ⛔ «No perteneces» y «no llegas» dan **el mismo mensaje** a propósito: decir
/// «no eres administrador de esa organización» le confirma a quien pregunta que
/// esa organización existe.
pub fn exige(tx: &mut Tx, emisor: &str, sub: &str, org: &str, minimo: &str) -> Result<Rol, String> {
    let suelo = ordinal_de(tx, minimo)?;
    match rol_de(tx, emisor, sub, org)? {
        Some(r) if r.ordinal >= suelo => Ok(r),
        _ => Err("no puedes hacer eso en esa organizacion".into()),
    }
}

/// **La guarda del rodeo.** Otorgar por encima del propio rol se niega.
pub fn no_por_encima(mio: &Rol, doy: &str, ordinal_doy: i16) -> Result<(), String> {
    if ordinal_doy > mio.ordinal {
        return Err(format!(
            "no puedes otorgar `{doy}`: es mas alto que tu propio rol, `{}`. \
             Otorgar por encima de uno mismo es tener por un rodeo lo que no se \
             tiene de frente",
            mio.nombre
        ));
    }
    Ok(())
}
