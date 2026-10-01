//! **El permiso de leer UN ítem** (ADR 0049 B3·2).
//!
//! `ore-serve` decide quién puede —autentica, comprueba la concesión, lee la
//! colección en la rama— y pide aquí un permiso para un ítem
//! (`POST /indice/abrir`). Lo que vuelve es un identificador opaco, aleatorio y
//! corto de vida; con él, el puesto lee los bytes de `ore-medios` directamente
//! (`GET /contenido?permiso=…`), y `ore-serve` no pasa bytes.
//!
//! ⭐ **El permiso no lleva nada dentro**: es una clave a lo que se guarda aquí
//!   —el ítem resuelto, fijado a su versión, y la credencial temporal de su
//!   origen—. La credencial no sale de los servicios de la celda y no hace
//!   falta un secreto compartido entre `ore-serve` y `ore-medios` para firmar
//!   nada.
//!
//! ⚠️ **Es un portador**: quien lo tenga lee ESE ítem, en ESA versión, mientras
//!   viva (5 minutos por defecto; todos los rangos que quiera, que es lo que un
//!   lector con `seek` necesita). No vale para otro ítem ni para listar.
//!
//! ⚠️ **En memoria**: un reinicio los olvida (el SDK pide otro), y con más de
//!   una réplica haría falta afinidad. Hoy hay una; dicho en 0049.

use crate::contenido::Pieza;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// La vida de un permiso, en segundos: por defecto, y sus límites.
pub const VIDA_POR_DEFECTO: u64 = 300;
pub const VIDA_MINIMA: u64 = 30;
pub const VIDA_MAXIMA: u64 = 3600;

/// Cuántos permisos vivos a la vez, como mucho. Uno por ítem que se abre;
/// pasado esto, `media/limite` (429) en vez de crecer sin techo.
pub const MAXIMOS: usize = 100_000;

pub struct Permiso {
    pub pieza: Pieza,
    caduca: Instant,
}

#[derive(Default)]
pub struct Permisos {
    vivos: Mutex<HashMap<String, Arc<Permiso>>>,
}

impl Permisos {
    /// Guarda la pieza y da su permiso, que vive `segundos` (acotados).
    /// `None`: no caben más.
    pub fn emitir(&self, pieza: Pieza, segundos: u64) -> Option<(String, u64)> {
        let segundos = segundos.clamp(VIDA_MINIMA, VIDA_MAXIMA);
        let ahora = Instant::now();
        let mut m = self.vivos.lock().unwrap();
        if m.len() >= MAXIMOS {
            m.retain(|_, p| p.caduca > ahora);
            if m.len() >= MAXIMOS {
                return None;
            }
        }
        let id = aleatorio();
        m.insert(
            id.clone(),
            Arc::new(Permiso {
                pieza,
                caduca: ahora + Duration::from_secs(segundos),
            }),
        );
        Some((id, segundos))
    }

    /// El permiso, si existe y vive. Uno caducado se olvida aquí.
    pub fn de(&self, id: &str) -> Option<Arc<Permiso>> {
        let mut m = self.vivos.lock().unwrap();
        match m.get(id) {
            Some(p) if p.caduca > Instant::now() => Some(p.clone()),
            Some(_) => {
                m.remove(id);
                None
            }
            None => None,
        }
    }

    pub fn cuantos(&self) -> usize {
        self.vivos.lock().unwrap().len()
    }
}

/// 64 hexadecimales de aleatoriedad del sistema (dos UUID v4: 244 bits). Cabe
/// en lo que la cadena de consulta admite (`ore-entrada`: `[A-Za-z0-9_-]`,
/// hasta 64).
fn aleatorio() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::contenido::Origen;

    fn pieza() -> Pieza {
        Pieza {
            coleccion: "legal.archivo.contratos".into(),
            camino: "a.pdf".into(),
            version: "v1".into(),
            origen: Origen::Lago {
                sha256: "aa".into(),
            },
            tamano: None,
            sha256: None,
            tipo: None,
        }
    }

    #[test]
    fn un_permiso_es_opaco_unico_y_da_su_pieza() {
        let p = Permisos::default();
        let (a, vida) = p.emitir(pieza(), 99_999).unwrap();
        let (b, _) = p.emitir(pieza(), 1).unwrap();
        assert_eq!(vida, VIDA_MAXIMA);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_eq!(p.de(&a).unwrap().pieza.camino, "a.pdf");
        assert!(p.de("otro").is_none());
    }

    #[test]
    fn uno_caducado_no_vale_y_se_olvida() {
        let p = Permisos::default();
        let (a, _) = p.emitir(pieza(), 30).unwrap();
        if let Some(x) = p.vivos.lock().unwrap().get_mut(&a) {
            *x = Arc::new(Permiso {
                pieza: pieza(),
                caduca: Instant::now() - Duration::from_secs(1),
            });
        }
        assert!(p.de(&a).is_none());
        assert_eq!(p.cuantos(), 0);
    }
}
