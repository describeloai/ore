//! **El registro del proceso**: lo que Ballista y DataFusion escriben con el facade `log`, a stderr, una
//! línea por mensaje. Sin dependencias: el formato y el destino son del clúster (frente 9).

use log::{Level, Log, Metadata, Record};
use std::io::Write;

struct Stderr {
    nivel: Level,
}

impl Log for Stderr {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.nivel
    }

    fn log(&self, r: &Record) {
        if self.enabled(r.metadata()) {
            let _ = writeln!(
                std::io::stderr().lock(),
                "{} {} {}",
                r.level(),
                r.target(),
                r.args()
            );
        }
    }

    fn flush(&self) {}
}

/// Pone el registro a stderr con `nivel`. Sólo la primera vez cuenta: el facade admite un registro por
/// proceso.
pub fn iniciar(nivel: Level) {
    // `set_logger` y no `set_boxed_logger`, que pide la feature `std` de `log`: un registro por proceso, que vive
    // lo que el proceso.
    if log::set_logger(Box::leak(Box::new(Stderr { nivel }))).is_ok() {
        log::set_max_level(nivel.to_level_filter());
    }
}
