//! `ore-postgres` — **el plano de control de ORE Serverless Postgres** (ADR 0058, P4).
//!
//! Lo que tú decides arriba, el producto lo hace correr abajo. Una celda —el
//! workspace del cliente— pide proyectos, ramas, endpoints, roles y bases; esto
//! lo recuerda (lo deseado y lo observado) y, desde P4·2, lo hace existir en el
//! almacenamiento y en NeonVM.
//!
//! ```text
//!   ore-serve (la celda) ──WI de la celda──► ore-postgres ──► storage_controller, safekeepers (P4·2)
//!                                            org = la de la celda       NeonVM en ore-pg-computo (P4·3)
//!                                            (POST /access/v1/celda de ore-iam)
//! ```
//!
//! ⭐ Integrado en la identidad y la propiedad, aparte en el runtime (decidido 8):
//!   la identidad y la organización las dice el plano de control común
//!   (`ore-iam`); el cómputo y el almacenamiento son del producto. Si `ore-iam`
//!   cae se para la gestión, no los datos.
//!
//! Lo que hay: el contrato ([`api`]), la base y sus migraciones ([`base`]), la
//! verificación de la celda ([`celda`]) y, desde P4·2, el almacenamiento
//! ([`almacen`]) y el reconciliador que lo hace existir ([`reconciliador`]): un
//! proyecto es un tenant y su `main`, un timeline.

pub mod almacen;
pub mod api;
pub mod base;
pub mod celda;
pub mod reconciliador;

use ore_entrada::http;
use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Mutex;

const USO: &str = "\
ore-postgres — el plano de control de ORE Serverless Postgres

  ore-postgres servir [--bind DIRECCION] [--iam DESTINO]
                      [--controlador DESTINO] [--safekeepers D1,D2,D3] [--llaves-almacen DIR]

  La base sale de `ORE_POSTGRES_URL`; sin valor por defecto. Las migraciones se
  aplican al arrancar. `--iam` es dónde preguntar de qué organización es una celda
  (por defecto `ore-iam.identidad.svc.cluster.local.:8090`).

  El reconciliador (P4·2) habla con el storage_controller y los safekeepers de
  `ore-pg` con los tokens de `--llaves-almacen` (`admin`, `safekeeperdata`; por
  defecto /llaves/almacen). Sin ellos no arranca, se dice, y las operaciones
  esperan en curso.
";

fn valor(args: &[String], que: &str) -> Option<String> {
    args.iter()
        .position(|a| a == que)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

pub fn arrancar(args: Vec<String>) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("servir") => servir(&args),
        Some("-h" | "--help") => {
            print!("{USO}");
            ExitCode::SUCCESS
        }
        Some(otro) => {
            eprintln!("✗ mando desconocido: `{otro}`\n\n{USO}");
            ExitCode::from(64)
        }
        None => {
            print!("{USO}");
            ExitCode::from(64)
        }
    }
}

fn servir(args: &[String]) -> ExitCode {
    let url = match std::env::var("ORE_POSTGRES_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("✗ falta `ORE_POSTGRES_URL`: sin ella no hay dónde recordar nada,");
            eprintln!("  y un valor por defecto sería apuntar a una base que nadie eligió.");
            return ExitCode::from(64);
        }
    };
    let bind = valor(args, "--bind").unwrap_or_else(|| "127.0.0.1:8100".into());
    let iam = valor(args, "--iam").unwrap_or_else(|| celda::DESTINO.into());
    let controlador = valor(args, "--controlador")
        .unwrap_or_else(|| "storage-controller.ore-pg.svc.cluster.local.:1234".into());
    let safekeepers: Vec<String> = valor(args, "--safekeepers")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_else(|| {
            (0..3)
                .map(|i| format!("safekeeper-{i}.ore-pg.svc.cluster.local.:7676"))
                .collect()
        });
    let llaves = valor(args, "--llaves-almacen").unwrap_or_else(|| "/llaves/almacen".into());

    let mut base = match base::conectar(&url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(69);
        }
    };
    match base::migrar(&mut base) {
        Ok(a) if a.is_empty() => eprintln!("  base         al día"),
        Ok(a) => eprintln!("  base         migrada: {}", a.join(", ")),
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(70);
        }
    }
    let escucha = match TcpListener::bind(&bind) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("✗ no se pudo escuchar en `{bind}`: {e}");
            return ExitCode::from(73);
        }
    };
    eprintln!("ore-postgres · {bind}");
    eprintln!("  celdas       la organización de cada una, de ore-iam en {iam}");
    match almacen::Neon::nuevo(
        &controlador,
        safekeepers.clone(),
        std::path::Path::new(&llaves),
    ) {
        Ok(neon) => {
            eprintln!(
                "  almacén      controller {controlador} · safekeepers {}",
                safekeepers.join(", ")
            );
            reconciliador::arrancar(url.clone(), Box::new(neon));
        }
        Err(e) => {
            eprintln!("  ⚠ SIN RECONCILIADOR: {e}");
            eprintln!("    las operaciones quedan en curso hasta que se monten los tokens");
        }
    }
    let servidor = api::Servidor {
        base: Mutex::new(base),
        celdas: Box::new(celda::PorOreIam::nuevo(&iam)),
        url: Some(url),
    };
    match http::servir(escucha, move |p| servidor.atender(p)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ el servidor terminó: {e}");
            ExitCode::from(70)
        }
    }
}
