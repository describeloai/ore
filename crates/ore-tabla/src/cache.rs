//! **Las tablas abiertas, entre consultas** (0081 §7.6, K3). Un `metadata_location` no cambia nunca, así que la tabla
//! que se abrió en él sirve para la consulta siguiente sin volver a leerla: ni sus metadatos ni, porque la
//! `ObjectCache` de iceberg-rust va dentro de la tabla, su manifest-list y sus manifests.
//!
//! - **Lo que se sigue pidiendo:** el puntero (la foto, o `loadTable`) en cada sentencia, que es lo que dice qué
//!   `metadata_location` toca, y la credencial de cada tabla, que se renueva aparte (§7.7). La caché de iceberg-rust
//!   no guarda las tablas con credencial prestada porque las ata; en ORE la credencial no va en la tabla.
//! - **Acotada** a [`CUPO`] tablas, la menos usada fuera. Cada una lleva su `ObjectCache` (32 MB como mucho). La cota
//!   de memoria común, con el pool del frente 7.

use iceberg::table::Table;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Cuántas tablas se guardan.
pub const CUPO: usize = 32;

/// Las tablas abiertas de un catálogo, por su `metadata_location`. Se comparten entre las consultas del motor.
#[derive(Clone, Default)]
pub struct Abiertas(Arc<Mutex<Lru>>);

#[derive(Default)]
struct Lru {
    orden: VecDeque<String>,
    tablas: HashMap<String, Table>,
}

impl std::fmt::Debug for Abiertas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Abiertas({})", self.cuantas())
    }
}

impl Abiertas {
    /// La tabla abierta en `metadata_location`, si está.
    pub fn toma(&self, metadata_location: &str) -> Option<Table> {
        let mut l = self.0.lock().unwrap();
        let t = l.tablas.get(metadata_location)?.clone();
        l.orden.retain(|m| m != metadata_location);
        l.orden.push_back(metadata_location.to_string());
        Some(t)
    }

    /// Guarda una tabla abierta en `metadata_location`. Sin ubicación (un esbozo) no se guarda.
    pub fn guarda(&self, metadata_location: &str, tabla: &Table) {
        if metadata_location.is_empty() {
            return;
        }
        let mut l = self.0.lock().unwrap();
        if l.tablas
            .insert(metadata_location.to_string(), tabla.clone())
            .is_none()
        {
            l.orden.push_back(metadata_location.to_string());
        }
        while l.orden.len() > CUPO {
            if let Some(m) = l.orden.pop_front() {
                l.tablas.remove(&m);
            }
        }
    }

    /// Cuántas hay.
    pub fn cuantas(&self) -> usize {
        self.0.lock().unwrap().tablas.len()
    }
}
