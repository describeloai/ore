//! **`SIGTERM` cancela el job en BigQuery** (`docs/federation.md` §1.5).
//!
//! Matar el proceso no para un job: BigQuery lo sigue ejecutando —y
//! facturando— hasta terminar. El manejador sólo levanta una bandera; un hilo
//! la vigila, manda `jobs.cancel` por el job en curso (`rest::EN_CURSO`) y
//! termina con 143. Una lectura por la Storage Read no tiene job: sus sesiones
//! caducan solas.

#[cfg(unix)]
pub fn al_terminar() {
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
    std::thread::spawn(|| {
        loop {
            if RECIBIDA.load(Ordering::SeqCst) {
                if let Ok(h) = crate::rest::Http::del_entorno() {
                    crate::rest::cancelar(&h);
                }
                std::process::exit(143);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
}

#[cfg(not(unix))]
pub fn al_terminar() {}
