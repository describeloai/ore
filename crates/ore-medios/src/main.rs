//! `ore-medios`: escucha en `ORE_MEDIOS_PUERTO` (8097) y sirve el índice y las
//! URLs de las colecciones de la celda (ver `lib.rs`). El lago es el de
//! `ORE_STORE` (`gcs` o `r2`), como en `ore-store`.

use ore_medios::indice::Indices;
use ore_medios::servicio::Servicio;
use ore_store::almacen::Almacen;
use ore_store::lago::Lago;
use std::sync::Arc;

/// Filas de índice vivas, como mucho, entre todas las colecciones (B2·0: un
/// millón de ítems es del orden de medio GB en Arrow; aquí se guarda menos).
const FILAS_POR_DEFECTO: usize = 2_000_000;

fn main() -> std::process::ExitCode {
    let cuenta: Arc<dyn Almacen> = match std::env::var("ORE_STORE").as_deref() {
        Ok("r2") => match ore_store::r2::Cuenta::del_entorno() {
            Ok(c) => Arc::new(c),
            Err(e) => return fallo(&e),
        },
        _ => match ore_store::gcs::Cuenta::del_entorno() {
            Ok(c) => Arc::new(c),
            Err(e) => return fallo(&e),
        },
    };
    let puerto = std::env::var("ORE_MEDIOS_PUERTO").unwrap_or_else(|_| "8097".into());
    let filas = std::env::var("ORE_MEDIOS_FILAS")
        .ok()
        .and_then(|f| f.parse().ok())
        .unwrap_or(FILAS_POR_DEFECTO);
    let servicio = Arc::new(Servicio {
        listados: Box::new(Lago::nuevo(cuenta.clone())),
        cuenta,
        indices: Indices::nuevo(filas),
        vistos: Arc::default(),
    });
    let escucha = match std::net::TcpListener::bind(format!("0.0.0.0:{puerto}")) {
        Ok(e) => e,
        Err(e) => return fallo(&format!("no se pudo escuchar en {puerto}: {e}")),
    };
    eprintln!("ore-medios · escucha en {puerto} · hasta {filas} filas de índice");
    match ore_entrada::http::servir(escucha, move |p| servicio.atender(p)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => fallo(&e.to_string()),
    }
}

fn fallo(e: &str) -> std::process::ExitCode {
    eprintln!("error: {e}");
    std::process::ExitCode::FAILURE
}
