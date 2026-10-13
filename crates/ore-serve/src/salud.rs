//! **La salud de un activo, servida** — ADR 0063 S2, OOS v1alpha30.
//!
//! El cálculo es de `ore_core::salud` y no tiene reloj; la hora la pone quien
//! sirve, aquí, en el momento de contestar. No se cachea: el estado de una
//! promesa de frescura cambia con el reloj aunque el árbol no cambie.

/// Segundos de época de ahora.
pub(crate) fn ahora() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
