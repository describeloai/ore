//! `ore-teselas`: el motor de teselas de la celda (ADR 0072 C3, F2·1), un proceso vivo
//! detrás de `ore-serve`. Escucha `POST /v1/teselas` (la petición de C3 v1, JSON) y
//! contesta los bytes MVT (200, con `x-ore-cuenta`), vacía (204), inválida (422), fuera
//! de plazo o de tamaño (503), o sin sitio: `ORE_TESELAS_EN_VUELO` teselas a la vez (64), que
//! esperan a GCS sin ocupar CPU; los turnos de CPU los toma el motor (F2·3, D-F2-9). `GET
//! /salud` para la readiness.

use std::sync::Arc;

use ore_base::json::Json;
use ore_entrada::http::{Bytes, Peticion, Respuesta, Salida};
use ore_teselas::{Motor, PROTOCOLO, Turnos, peticion};

// El `malloc` de musl tiene un candado global: ver el `Cargo.toml`.
#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn atender(motor: &Motor, en_vuelo: &Turnos, p: &Peticion) -> Salida {
    match (p.metodo.as_str(), p.ruta.as_str()) {
        ("GET", "/salud") => Salida::Una(Respuesta::ok(Json::obj([(
            "protocolo",
            Json::Int(PROTOCOLO as i64),
        )]))),
        ("POST", "/v1/teselas") => {
            let mut pet = match peticion(&p.cuerpo) {
                Ok(pet) => pet,
                Err(f) => return Salida::Una(Respuesta::error(f.codigo(), f.motivo())),
            };
            // El sitio (F2·3): lo que espera cuenta en su plazo; sin sitio a tiempo, 503.
            let t = std::time::Instant::now();
            let Some(_sitio) = en_vuelo.entrar(std::time::Duration::from_millis(pet.plazo_ms))
            else {
                eprintln!(
                    "tesela {}/{}/{} · 503 · sin sitio en su plazo",
                    pet.z, pet.x, pet.y
                );
                return Salida::Una(Respuesta::error(
                    503,
                    "el motor está lleno: la tesela no tuvo turno en su plazo",
                ));
            };
            pet.plazo_ms = pet.plazo_ms.saturating_sub(t.elapsed().as_millis() as u64);
            match motor.tesela(&pet) {
                Ok((None, c)) => Salida::Bytes(Bytes {
                    codigo: 204,
                    cabeceras: vec![("x-ore-cuenta".into(), c.json().jcs())],
                    largo: Some(0),
                    lector: Box::new(std::io::empty()),
                    finales: None,
                }),
                Ok((Some(b), c)) => Salida::Bytes(Bytes {
                    codigo: 200,
                    cabeceras: vec![
                        (
                            "content-type".into(),
                            "application/vnd.mapbox-vector-tile".into(),
                        ),
                        ("etag".into(), format!("\"{:016x}\"", ore_teselas::fnv(&b))),
                        ("x-ore-cuenta".into(), c.json().jcs()),
                    ],
                    largo: Some(b.len() as u64),
                    lector: Box::new(std::io::Cursor::new(b)),
                    finales: None,
                }),
                Err(f) => {
                    eprintln!(
                        "tesela {}/{}/{} · {} · {}",
                        pet.z,
                        pet.x,
                        pet.y,
                        f.codigo(),
                        f.motivo()
                    );
                    Salida::Una(Respuesta::error(f.codigo(), f.motivo()))
                }
            }
        }
        _ => Salida::Una(Respuesta::error(404, "ruta desconocida")),
    }
}

fn main() -> std::process::ExitCode {
    let puerto = std::env::var("ORE_TESELAS_PUERTO").unwrap_or_else(|_| "8099".into());
    let escucha = match std::net::TcpListener::bind(format!("0.0.0.0:{puerto}")) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: no se pudo escuchar en {puerto}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let motor = Arc::new(Motor::nuevo());
    let en_vuelo = Arc::new(Turnos::nuevos(
        std::env::var("ORE_TESELAS_EN_VUELO")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64),
    ));
    eprintln!("ore-teselas · escucha en {puerto} · protocolo {PROTOCOLO}");
    match ore_entrada::http::servir_con_flujos(escucha, move |p| atender(&motor, &en_vuelo, p)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
