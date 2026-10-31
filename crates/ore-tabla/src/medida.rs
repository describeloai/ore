//! **Lo que cuesta planificar** (0081 §7.6, K1): cuántas peticiones a `/v1` y cuántas lecturas del almacén prestado
//! hace este proceso. Son cuentas de proceso, sin coste que importe, y `ore-runtime local --medir` las resta antes y
//! después de cada fase de una sentencia.

use std::sync::atomic::{AtomicU64, Ordering};

static V1: AtomicU64 = AtomicU64::new(0);
static LECTURAS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

/// Las cuentas, en un instante.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Medida {
    /// Peticiones a `/v1`.
    pub v1: u64,
    /// Lecturas del almacén prestado (objetos enteros o rangos).
    pub lecturas: u64,
    /// Bytes leídos del almacén prestado.
    pub bytes: u64,
}

impl Medida {
    /// Lo de ahora.
    pub fn ahora() -> Medida {
        Medida {
            v1: V1.load(Ordering::Relaxed),
            lecturas: LECTURAS.load(Ordering::Relaxed),
            bytes: BYTES.load(Ordering::Relaxed),
        }
    }

    /// Lo que pasó desde `antes`.
    pub fn desde(self, antes: Medida) -> Medida {
        Medida {
            v1: self.v1 - antes.v1,
            lecturas: self.lecturas - antes.lecturas,
            bytes: self.bytes - antes.bytes,
        }
    }
}

pub(crate) fn una_peticion() {
    V1.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn una_lectura(bytes: usize) {
    LECTURAS.fetch_add(1, Ordering::Relaxed);
    BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}
