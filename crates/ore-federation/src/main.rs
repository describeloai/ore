//! `ore-federation`: la pasarela del Federation Engine, un servicio por celda.
//!
//! ```text
//! ore-federation [--escucha 0.0.0.0:8099] [--conectores <dir>] [--tipos postgres,bigquery,s3,gcs]
//! ```
//!
//! `--conectores` es donde están los `ore-read-<tipo>` (por defecto, junto a
//! este binario: en la imagen `ore-drivers` viven juntos). Las cotas salen del
//! entorno (`ORE_FED_*`, ver [`ore_federation::cotas`]).

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;

use ore_federation::cotas::Cotas;
use ore_federation::fondo::Fondo;
use ore_federation::pasarela::Pasarela;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let valor = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let escucha = valor("--escucha").unwrap_or_else(|| "0.0.0.0:8099".into());
    // 0053 F9·4: en la imagen, los conectores viven fuera del `PATH`, en
    // `/opt/ore/conectores` (sólo la pasarela los lanza); si no está, junto al
    // binario, como en el CI y en una máquina.
    let conectores = valor("--conectores").map(PathBuf::from).unwrap_or_else(|| {
        let imagen = PathBuf::from("/opt/ore/conectores");
        if imagen.is_dir() {
            return imagen;
        }
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
    });
    let tipos: Vec<String> = valor("--tipos")
        .unwrap_or_else(|| "postgres,bigquery,s3,gcs".into())
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let cotas = match Cotas::del_entorno() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ore-federation: {e}");
            return ExitCode::from(2);
        }
    };
    eprintln!(
        "ore-federation: tope {} filas · {} B · {} ms; {} a la vez por origen, cola {}, espera {:?}, ociosa {:?}",
        cotas.presupuesto.filas,
        cotas.presupuesto.bytes,
        cotas.presupuesto.ms,
        cotas.concurrencia,
        cotas.cola,
        cotas.espera,
        cotas.ociosa
    );
    let fondo = Fondo::new(cotas, conectores);
    fondo.barrendero();
    let pasarela = Pasarela::new(fondo, &tipos);
    let l = match TcpListener::bind(&escucha) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ore-federation: no se escucha en {escucha}: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("ore-federation: escuchando en {escucha}");
    match ore_entrada::http::servir_con_flujos(l, move |p| pasarela.atender(p)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ore-federation: {e}");
            ExitCode::FAILURE
        }
    }
}
