//! **El handle de una persona** (la 048): lo que va detrás de `user:` en el
//! `owner` de lo que crea.
//!
//! ```text
//!   el token     `preferred_username` —el nombre de usuario que eligió al
//!                registrarse—, y si no lo trae, su correo
//!   la regla     `ore_core::pertenencia::handle_de`: minúsculas, dígitos y `-`
//!   la fila      `iam.persona.handle`, único entre todas las personas
//! ```
//!
//! ⭐ Se asigna **una vez** —la primera vez que `ore-iam` ve el token de la
//! persona ([`asegurar`]) — y no se toca más: es lo que queda escrito en los
//! `owner` del árbol, y un handle que cambiara dejaría documentos a nombre de
//! nadie. Si el que sale ya es de otra persona, `-2`, `-3`…
//!
//! ⚠️ Y no deja huella, como el nombre (`refrescar_nombre`): no es un acto de
//!   nadie, es el sujeto quedando dicho en la tabla.

use crate::base::Tx;
use ore_core::pertenencia::{HANDLE_MAXIMO, handle_de};
use ore_entrada::identidad::Identidad;

/// Cuántos sufijos se prueban antes de rendirse. Cien personas que eligieron el
/// mismo nombre ya no es un empate: es un fallo de quien deriva.
const SUFIJOS: u32 = 100;

/// El handle de quien trae el token: el que tiene, o uno nuevo si aún no tiene.
/// `None` si no es una persona de esta tabla (un agente, alguien sin fila).
pub fn asegurar(tx: &mut Tx, emisor: &str, s: &Identidad) -> Result<Option<String>, String> {
    let Some(f) = tx.uno(
        "select id, handle, correo from iam.persona where emisor = $1 and sub = $2",
        &[&emisor, &s.persona],
    )?
    else {
        return Ok(None);
    };
    let (id, handle, correo): (String, Option<String>, Option<String>) =
        (f.get(0), f.get(1), f.get(2));
    if let Some(h) = handle {
        return Ok(Some(h));
    }
    let base = derivar(
        s.usuario.as_deref(),
        s.correo.as_deref().or(correo.as_deref()),
        &id,
    );
    asignar(tx, &id, &base).map(Some)
}

/// El handle de una persona de `organizacion`, por su `sub`, para una celda que
/// no tiene su token: desde un puesto, quien llama es el agente y la persona es
/// la que lo abrió. Si aún no tiene, se le asigna con lo que haya en la fila (el
/// correo): es el caso de alguien que no ha entrado desde la 048.
///
/// `None` si no es una persona que pertenezca a `organizacion` —un agente no es
/// dueño de nada, y una celda no pregunta por gente de otra—.
pub fn de_la_persona(
    tx: &mut Tx,
    emisor: &str,
    sub: &str,
    organizacion: &str,
) -> Result<Option<String>, String> {
    let Some(f) = tx.uno(
        "select p.id, p.handle, p.correo
           from iam.persona p
           join iam.pertenencia pe on pe.persona = p.id
          where p.emisor = $1 and p.sub = $2 and pe.organizacion = $3",
        &[&emisor, &sub, &organizacion],
    )?
    else {
        return Ok(None);
    };
    let (id, handle, correo): (String, Option<String>, Option<String>) =
        (f.get(0), f.get(1), f.get(2));
    if let Some(h) = handle {
        return Ok(Some(h));
    }
    asignar(tx, &id, &derivar(None, correo.as_deref(), &id)).map(Some)
}

/// El handle que tocaría, antes de desempatar: el del usuario; si no da nada, el
/// del correo; y si tampoco, uno del id de la persona (opaco, pero suyo).
pub fn derivar(usuario: Option<&str>, correo: Option<&str>, id: &str) -> String {
    usuario
        .and_then(handle_de)
        .or_else(|| correo.and_then(handle_de))
        .or_else(|| handle_de(&format!("persona-{}", id.trim_start_matches("per_"))))
        .unwrap_or_else(|| "persona".to_string())
}

/// El candidato `n`-ésimo: `base`, `base-2`, `base-3`… sin pasar del techo.
pub fn candidato(base: &str, n: u32) -> String {
    if n <= 1 {
        return base.to_string();
    }
    let sufijo = format!("-{n}");
    let mut b = base.to_string();
    b.truncate(HANDLE_MAXIMO.saturating_sub(sufijo.len()));
    format!("{}{sufijo}", b.trim_end_matches('-'))
}

fn asignar(tx: &mut Tx, id: &str, base: &str) -> Result<String, String> {
    for n in 1..=SUFIJOS {
        let h = candidato(base, n);
        if tx
            .uno("select 1 from iam.persona where handle = $1", &[&h])?
            .is_some()
        {
            continue;
        }
        // `handle is null`: si otra petición se adelantó, gana la suya y se lee.
        tx.ejecutar(
            "update iam.persona set handle = $2 where id = $1 and handle is null",
            &[&id, &h],
        )?;
        let quedo: Option<String> = tx
            .uno("select handle from iam.persona where id = $1", &[&id])?
            .and_then(|f| f.get(0));
        return quedo.ok_or_else(|| format!("la persona `{id}` se quedó sin handle"));
    }
    Err(format!(
        "no hay handle libre para `{base}` en {SUFIJOS} intentos"
    ))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_core::pertenencia::es_handle;

    #[test]
    fn el_usuario_manda_y_luego_el_correo_y_luego_el_id() {
        assert_eq!(
            derivar(Some("Victor.G"), Some("v@x.io"), "per_1"),
            "victor-g"
        );
        assert_eq!(derivar(None, Some("ana.lopez@x.io"), "per_1"), "ana-lopez");
        assert_eq!(derivar(Some("@@"), Some("bea@x.io"), "per_1"), "bea");
        assert_eq!(derivar(None, None, "per_7f3a"), "persona-7f3a");
        assert_eq!(derivar(Some("日本"), None, "per_9"), "persona-9");
    }

    #[test]
    fn el_desempate_no_pasa_del_techo_ni_deja_guiones() {
        assert_eq!(candidato("ana", 1), "ana");
        assert_eq!(candidato("ana", 2), "ana-2");
        let largo = "a".repeat(HANDLE_MAXIMO);
        let c = candidato(&largo, 17);
        assert!(c.len() <= HANDLE_MAXIMO && c.ends_with("-17"), "{c}");
        let con_guion = format!("{}-b", "a".repeat(HANDLE_MAXIMO - 4));
        let c = candidato(&con_guion, 2);
        assert!(!c.contains("--") && es_handle(&format!("user:{c}")), "{c}");
    }
}
