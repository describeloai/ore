//! ⭐ **EL RECONCILIADOR** (0079 §19.2, E1a·5c·5c·4): lo que git confirmó y el catálogo no
//! llegó a aplicar, aplicado.
//!
//! Retirar, renombrar y fusionar hacen dos cosas sin una transacción común: el commit de git, que
//! lleva el plan de los punteros en su bloque `Punteros:`, y su aplicación en el catálogo
//! (`crate::definicion`, `crate::fusion_catalogo`). Si ore-serve cae entre las dos, o el catálogo
//! no contesta en ese momento, la respuesta dijo `punteros: pendiente` y nadie lo terminaba.
//!
//! Una **pasada**: los commits recientes de cada rama con bloque ([`crate::git::Forja::con_bloque`]),
//! del más viejo al más nuevo; los que el catálogo no tiene (`aplicado(sha)`), se aplican con su
//! sha. Aplicar dos veces el mismo sha no hace nada, así que una pasada a la vez que la petición que
//! aplica no hace daño. Una fusión cuyo cerrojo caducó se aplica como tardía: gana lo que `main`
//! escribió entretanto. Lo que no se puede aplicar ahora (un cerrojo vivo, el catálogo caído) se
//! reintenta en la siguiente.
//!
//! Corre al arrancar y cada [`CADENCIA`], sólo con el catálogo (`main.rs`).

use std::time::Duration;

use ore_catalogo::almacen::FalloAplicar;
use ore_core::punteros::Contexto;
use ore_core::punteros::grabadora::{Bloque, leer_bloque};

/// Cada cuánto se pasa.
pub(crate) const CADENCIA: Duration = Duration::from_secs(120);

/// Cuántos commits de cada rama se miran: los de una caída reciente.
pub(crate) const POR_RAMA: usize = 30;

/// Lo que hizo una pasada.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Informe {
    pub(crate) aplicados: Vec<String>,
    pub(crate) ya: usize,
    pub(crate) fallos: Vec<String>,
}

impl Informe {
    /// Una línea para el registro, o nada si no hubo nada que contar.
    pub(crate) fn linea(&self) -> Option<String> {
        if self.aplicados.is_empty() && self.fallos.is_empty() {
            return None;
        }
        Some(format!(
            "  reconciliador: {} aplicado(s) {:?}, {} ya estaban, {} fallo(s) {:?}",
            self.aplicados.len(),
            self.aplicados,
            self.ya,
            self.fallos.len(),
            self.fallos
        ))
    }
}

/// **Aplica un commit** con bloque si el catálogo no lo tiene. `Ok(true)`: lo aplicó;
/// `Ok(false)`: ya estaba.
pub(crate) fn aplicar_commit(sha: &str, autor: &str, mensaje: &str) -> Result<bool, String> {
    let bloque = match leer_bloque(mensaje) {
        None => return Ok(false),
        Some(b) => b.map_err(|e| e.to_string())?,
    };
    let ya = crate::lugar::en_el_catalogo(None, |c| c.aplicado(sha))
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if ya {
        return Ok(false);
    }
    let autor = format!("reconciliador:{autor}");
    match bloque {
        Bloque::Definicion {
            r,
            operacion,
            cambios,
        } => {
            let ctx = Contexto {
                autor,
                operacion,
                idempotencia: None,
            };
            crate::lugar::en_el_catalogo(Some(&r), |c| {
                c.aplicar_definicion(sha, &ctx, &cambios, None)
            })
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        }
        Bloque::Fusion { rama, plan } => {
            let plan = plan.jcs();
            crate::lugar::en_el_catalogo(Some(&rama), |c| {
                match c.aplicar_plan(sha, &autor, &plan, false) {
                    Err(FalloAplicar::Protocolo(_)) => c.aplicar_plan(sha, &autor, &plan, true),
                    r => r,
                }
            })
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("{e:?}"))?;
        }
    }
    Ok(true)
}

/// **Una pasada** sobre los commits recientes de la forja.
pub(crate) fn pasada(forja: &crate::git::Forja) -> Informe {
    let mut inf = Informe::default();
    let commits = match forja.con_bloque(POR_RAMA) {
        Ok(c) => c,
        Err(e) => {
            inf.fallos.push(format!("la forja: {e}"));
            return inf;
        }
    };
    for (sha, autor, mensaje) in commits {
        match aplicar_commit(&sha, &autor, &mensaje) {
            Ok(true) => inf.aplicados.push(sha[..sha.len().min(12)].to_string()),
            Ok(false) => inf.ya += 1,
            Err(e) => inf
                .fallos
                .push(format!("{}: {e}", &sha[..sha.len().min(12)])),
        }
    }
    inf
}

#[cfg(test)]
mod pruebas {}
