//! `ore-teselas`: el motor de teselas de la celda (ADR 0072 C3, F2·1), un proceso vivo
//! detrás de `ore-serve`. Escucha `POST /v1/teselas` (la petición de C3 v1, JSON) y
//! contesta los bytes MVT (200, con `x-ore-cuenta`), vacía (204), inválida (422), fuera
//! de plazo o de tamaño (503). `GET /salud` para la readiness.

use std::sync::Arc;

use ore_core::json::Json;
use ore_entrada::http::{Bytes, Peticion, Respuesta, Salida};
use ore_teselas::{Motor, PROTOCOLO, peticion};

fn atender(motor: &Motor, p: &Peticion) -> Salida {
    match (p.metodo.as_str(), p.ruta.as_str()) {
        ("GET", "/salud") => Salida::Una(Respuesta::ok(Json::obj([(
            "protocolo",
            Json::Int(PROTOCOLO as i64),
        )]))),
        ("POST", "/v1/teselas") => {
            let pet = match peticion(&p.cuerpo) {
                Ok(pet) => pet,
                Err(f) => return Salida::Una(Respuesta::error(f.codigo(), f.motivo())),
            };
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
    eprintln!("ore-teselas · escucha en {puerto} · protocolo {PROTOCOLO}");
    match ore_entrada::http::servir_con_flujos(escucha, move |p| atender(&motor, p)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
