//! Quién puede qué — y **el rodeo que esto existe para cerrar**.
//!
//! # La guarda que no es obvia
//!
//! La escalera de papeles es evidente: un administrador puede más que un
//! miembro. Lo que no es evidente es que hace falta una segunda comprobación, y
//! su motivo lo escribió la plataforma con la cicatriz al lado:
//!
//! > *«sin esa segunda potestad, un USERADMIN invitaría a un cómplice como
//! > ACCOUNTADMIN y tendría **por un rodeo** el poder que se le niega.»*
//!
//! ⇒ **Nadie puede otorgar un papel más alto que el suyo.** Sin eso, `invitar`
//! es una escalada de privilegio con forma de cortesía, y `conceder` es la
//! misma escalada por otra puerta.
//!
//! # Y el sujeto se busca por `(emisor, sub)`, nunca por `sub` a secas
//!
//! Un `sub` sin su emisor no identifica a nadie — es el mismo error que aceptar
//! un token sin mirar el `iss`. Dos emisores pueden traer el mismo `sub` y
//! serían dos personas distintas viendo la misma organización.

use crate::base::Tx;

/// El papel de alguien en una organización, con su altura.
pub struct Papel {
    pub nombre: String,
    pub ordinal: i16,
}

/// `None` si esa persona no pertenece a esa organización.
pub fn papel_de(tx: &mut Tx, emisor: &str, sub: &str, org: &str) -> Result<Option<Papel>, String> {
    let f = tx.uno(
        "select pe.papel, pa.ordinal
           from iam.pertenencia pe
           join iam.persona  p on p.id = pe.persona
           join iam.papel    pa on pa.nombre = pe.papel
          where p.emisor = $1 and p.sub = $2 and pe.organizacion = $3",
        &[&emisor, &sub, &org],
    )?;
    Ok(f.map(|f| Papel {
        nombre: f.get(0),
        ordinal: f.get(1),
    }))
}

pub fn ordinal_de(tx: &mut Tx, papel: &str) -> Result<i16, String> {
    tx.uno("select ordinal from iam.papel where nombre = $1", &[&papel])?
        .map(|f| f.get(0))
        .ok_or_else(|| format!("`{papel}` no es un papel de esta plataforma"))
}

/// Exige al menos `minimo`, y devuelve lo que tiene.
///
/// ⛔ «No perteneces» y «no llegas» dan **el mismo mensaje** a propósito: decir
/// «no eres administrador de esa organización» le confirma a quien pregunta que
/// esa organización existe.
pub fn exige(
    tx: &mut Tx,
    emisor: &str,
    sub: &str,
    org: &str,
    minimo: &str,
) -> Result<Papel, String> {
    let suelo = ordinal_de(tx, minimo)?;
    match papel_de(tx, emisor, sub, org)? {
        Some(p) if p.ordinal >= suelo => Ok(p),
        _ => Err("no puedes hacer eso en esa organizacion".into()),
    }
}

/// **La guarda del rodeo.** Otorgar por encima del propio papel se niega.
pub fn no_por_encima(mio: &Papel, doy: &str, ordinal_doy: i16) -> Result<(), String> {
    if ordinal_doy > mio.ordinal {
        return Err(format!(
            "no puedes otorgar `{doy}`: es mas alto que tu propio papel, `{}`. \
             Otorgar por encima de uno mismo es tener por un rodeo lo que no se \
             tiene de frente",
            mio.nombre
        ));
    }
    Ok(())
}
