//! Quién puede qué — y **el rodeo que esto existe para cerrar**.
//!
//! # La guarda, y por qué dejó de ser una resta
//!
//! Su motivo lo escribió la plataforma con la cicatriz al lado:
//!
//! > *«sin esa segunda potestad, un USERADMIN invitaría a un cómplice como
//! > ACCOUNTADMIN y tendría **por un rodeo** el poder que se le niega.»*
//!
//! Hasta la `014` eso se comprobaba con un ordinal: los roles eran una escalera
//! y otorgar por encima era un `>`. Un ordinal sólo sabe decir «más» y «menos»,
//! y hay una separación que importa y no es de altura — *¿es el que corta más o
//! menos que el que da de alta?*.
//!
//! ⇒ **Puedes otorgar un rol si sus potestades están CONTENIDAS en las tuyas.**
//! Igual de comprobable, más expresivo, y admite un rol que corta sin nombrar.
//!
//! # Y pertenecer no es un rol
//!
//! Quien pertenece tiene el estado por defecto —`iam.por_defecto`— aunque su
//! `rol` sea `null`. Es la frase de su `76` §2: *«pertenecer ya da lectura.
//! Leer no es un rol»*, y esconder quién manda sería seguridad por oscuridad.
//!
//! # El sujeto se busca por `(emisor, sub)`, nunca por `sub` a secas
//!
//! Un `sub` sin su emisor no identifica a nadie — es el mismo error que aceptar
//! un token sin mirar el `iss`.

use crate::base::Tx;
use std::collections::BTreeSet;

/// Lo que alguien puede hacer **en una organización**.
pub type Potestades = BTreeSet<String>;

/// Las suyas, ya con el estado por defecto dentro.
///
/// ⭐ Vacío significa **no pertenece**. No hay forma de pertenecer sin
/// potestades: `iam.por_defecto` nunca está vacía.
pub fn potestades_de(
    tx: &mut Tx,
    emisor: &str,
    sub: &str,
    org: &str,
) -> Result<Potestades, String> {
    // ⭐ Desde la `016` la union vive en `iam.potestades_de_persona`: el estado
    //   por defecto por pertenecer, mas lo que añada CADA cargo. Que sean
    //   varios no cambia nada aqui — la union de conjuntos no necesita orden.
    let filas = tx.filas(
        "select pp.potestad
           from iam.potestades_de_persona pp
           join iam.persona p on p.id = pp.persona
          where p.emisor = $1 and p.sub = $2 and pp.organizacion = $3",
        &[&emisor, &sub, &org],
    )?;
    Ok(filas.iter().map(|f| f.get::<_, String>(0)).collect())
}

/// Las que da un rol, para poder compararlas con las de quien lo otorga.
pub fn potestades_del_rol(tx: &mut Tx, rol: &str) -> Result<Potestades, String> {
    let filas = tx.filas(
        "select potestad from iam.potestades_de_rol where rol = $1",
        &[&rol],
    )?;
    let ps: Potestades = filas.iter().map(|f| f.get::<_, String>(0)).collect();
    if ps.is_empty() {
        // ⛔ Un rol sin potestades no existe o esta vacio, y las dos cosas se
        //   niegan igual: otorgar algo que no da nada es una promesa falsa.
        return Err(format!("`{rol}` no es un rol que se pueda otorgar"));
    }
    Ok(ps)
}

/// ⛔⛔ EL PLANO DE ABAJO, y no se mezcla con lo de arriba.
///
/// `iam.rol_de_recurso` —`lector` y `owner`— gobierna el ARBOL, no la
/// organizacion, y **no tiene ordinal**: `owner` no implica `lector`, asi que
/// aqui no hay nada que contener ni que comparar. Solo se comprueba que exista.
/// Ver la `011`.
pub fn rol_de_recurso(tx: &mut Tx, rol: &str) -> Result<(), String> {
    tx.uno(
        "select 1 from iam.rol_de_recurso where nombre = $1",
        &[&rol],
    )?
    .map(|_| ())
    .ok_or_else(|| format!("`{rol}` no es un rol de recurso. Son `lector` y `owner`"))
}

/// Exige una potestad, y devuelve todas las que tiene quien pregunta.
///
/// ⛔ «No perteneces» y «no puedes» dan **el mismo mensaje** a propósito: decir
/// «no tienes esa potestad en esa organización» le confirma a quien pregunta
/// que esa organización existe.
pub fn exige(
    tx: &mut Tx,
    emisor: &str,
    sub: &str,
    org: &str,
    potestad: &str,
) -> Result<Potestades, String> {
    let mias = potestades_de(tx, emisor, sub, org)?;
    if mias.contains(potestad) {
        Ok(mias)
    } else {
        Err("no puedes hacer eso en esa organizacion".into())
    }
}

/// **La guarda del rodeo**, ahora por contención.
///
/// ⚠️ Y no basta con llamarla: otorgar un rol exige ADEMÁS la potestad
/// `rol:conceder`. Son dos preguntas distintas —*¿puedes otorgar?* y *¿puedes
/// otorgar ESO?*— y su `021` lo dice entero: **conceder aplazado sigue siendo
/// conceder**.
pub fn contenidas_en(doy: &Potestades, mias: &Potestades, rol: &str) -> Result<(), String> {
    let de_mas: Vec<&str> = doy.difference(mias).map(String::as_str).collect();
    if de_mas.is_empty() {
        return Ok(());
    }
    Err(format!(
        "no puedes otorgar `{rol}`: da potestades que tu no tienes ({}). \
         Otorgar por encima de uno mismo es tener por un rodeo lo que no se \
         tiene de frente",
        de_mas.join(", ")
    ))
}
