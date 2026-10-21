//! **Cancelar una lectura**, tambien cuando esta esperando al servidor.
//!
//! Es una bandera compartida. Quien espera una respuesta no se bloquea sin mas: lee en rodajas de
//! [`crate::Conexion::RODAJA`] y mira la bandera entre una y otra, asi que cancelar tarda como mucho una
//! rodaja (100 ms) en hacer efecto, en cualquier sistema. (Se probo cerrar el *socket* desde otro hilo para
//! despertar la lectura: en Linux funciona y en Windows no, y una cancelacion que depende de la plataforma no
//! es una cancelacion.)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Se comparte entre quien lee y quien cancela (otro hilo, una senal, un cierre de la entrada).
#[derive(Clone, Default, Debug)]
pub struct Cancelacion(Arc<AtomicBool>);

impl Cancelacion {
    pub fn nueva() -> Cancelacion {
        Cancelacion::default()
    }

    /// Pide parar.
    pub fn cancelar(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn cancelada(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelar_se_ve_desde_todos_los_clones() {
        let c = Cancelacion::nueva();
        let otra = c.clone();
        assert!(!c.cancelada() && !otra.cancelada());
        otra.cancelar();
        assert!(c.cancelada());
    }
}
