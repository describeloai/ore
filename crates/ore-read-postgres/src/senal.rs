//! **`SIGTERM` cancela en el origen** (`docs/federation.md` §1.5).
//!
//! Matar el proceso no basta: Postgres no se entera de que el cliente se fue
//! hasta que intenta escribirle, y una consulta que aún no devuelve filas —un
//! `pg_sleep`, una agregación larga— sigue ocupando su sesión. El kit lo midió
//! en F2·1: tras el `SIGTERM`, la consulta seguía viva 3 s después.
//!
//! El manejador sólo levanta una bandera (es lo único que se puede hacer dentro
//! de él sin riesgo); un hilo la vigila, cancela por el `CancelToken` de la
//! sesión en curso y termina con 143, el código de quien muere por `SIGTERM`.

use crate::lectura::Token;

#[cfg(unix)]
pub fn al_terminar(token: Token) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RECIBIDA: AtomicBool = AtomicBool::new(false);
    extern "C" fn manejar(_: libc::c_int) {
        RECIBIDA.store(true, Ordering::SeqCst);
    }
    // SAFETY: `manejar` sólo escribe un atómico, que es seguro dentro de un
    // manejador de señal.
    unsafe {
        libc::signal(libc::SIGTERM, manejar as *const () as libc::sighandler_t);
    }
    std::thread::spawn(move || {
        loop {
            if RECIBIDA.load(Ordering::SeqCst) {
                crate::lectura::cancelar(&token);
                std::process::exit(143);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
}

/// Fuera de Unix no hay `SIGTERM`: quien para el proceso lo mata, y la
/// cancelación es la de `servir` (`{"cancelar": id}` o cerrar la entrada).
#[cfg(not(unix))]
pub fn al_terminar(_: Token) {}
