//! `ore-kit`: los 14 casos contra un conector.
//!
//! ```text
//! ore-kit --conector <binario> --banco postgres|s3|bigquery [--casos 1,2,5]
//!         [--informe informe.json] [--exige 1,2,3|todos]
//!         [--pasarela <ore-federation>]
//! ```
//!
//! Con `--pasarela`, en vez de los 14 casos del conector corren los 8 de la
//! pasarela (ADR 0053 F3·2): la pasarela de verdad, con ese conector detrás.
//!
//! Sin `--exige` es la **línea de base**: dice cómo está y sale con 0. Con él,
//! sale con 1 si alguno de esos casos falla —es lo que el CI pide a un conector
//! que ya dijo pasarlos—.
//!
//! El banco se configura por el entorno: `PG_URL` (un administrador del
//! Postgres de pruebas) o `S3_KIT_ENDPOINT`, `S3_KIT_CLAVE`, `S3_KIT_SECRETO`
//! (y `S3_KIT_DE_MENTIRA=1` si es el S3 de `pruebas-de-fuego/de-mentira.py`,
//! que no comprueba firmas), o `BQ_KIT_PROYECTO` (y `BQ_KIT_DATASET`,
//! `BQ_KIT_UBICACION`) con la credencial de `ore-gcp`; con
//! `ORE_BQ_CINTA=<dir> ORE_BQ_CINTA_MODO=reproducir`, el conector y el banco
//! reproducen una cinta grabada y no hablan con BigQuery.
//! Ninguna URL sale en el informe.

use ore_conector_kit::bancos::{
    Banco, azure::Azure, bigquery::BigQuery, gcs::Gcs, postgres::Postgres, s3::S3,
};
use ore_conector_kit::casos::{self, Estado};
use ore_conector_kit::conector::Conector;
use ore_core::json::Json;
use std::path::PathBuf;
use std::process::ExitCode;

fn lista(s: &str, hasta: u8) -> Result<Vec<u8>, String> {
    if s == "todos" {
        return Ok((1..=hasta).collect());
    }
    s.split(',')
        .map(|n| {
            n.trim()
                .parse::<u8>()
                .ok()
                .filter(|n| (1..=hasta).contains(n))
                .ok_or_else(|| format!("`{n}` no es un caso (1–{hasta})"))
        })
        .collect()
}

fn entorno(k: &str) -> Result<String, String> {
    std::env::var(k)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("falta `{k}` en el entorno"))
}

fn intentar() -> Result<bool, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let valor = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let conector = valor("--conector").ok_or("falta `--conector <binario>`")?;
    let familia = valor("--banco").ok_or("falta `--banco postgres|s3|gcs|azure|bigquery`")?;
    let pasarela = valor("--pasarela");
    let hasta = if pasarela.is_some() { 8 } else { 14 };
    let solo = valor("--casos")
        .map(|s| lista(&s, hasta))
        .transpose()?
        .unwrap_or_default();
    let exige = valor("--exige")
        .map(|s| lista(&s, hasta))
        .transpose()?
        .unwrap_or_default();

    let mut banco: Box<dyn Banco> = match familia.as_str() {
        "postgres" => Box::new(Postgres::new(&entorno("PG_URL")?)?),
        "s3" => Box::new(S3::new(
            &entorno("S3_KIT_ENDPOINT")?,
            &entorno("S3_KIT_CLAVE")?,
            &entorno("S3_KIT_SECRETO")?,
            std::env::var("S3_KIT_DE_MENTIRA").is_err(),
        )),
        "gcs" => Box::new(Gcs::new(&entorno("GCS_KIT_ENDPOINT")?)),
        "azure" => Box::new(Azure::new(
            &entorno("AZURE_KIT_ENDPOINT")?,
            &entorno("ORE_AZURE_TOKEN")?,
        )),
        "bigquery" => Box::new(BigQuery::new(
            &entorno("BQ_KIT_PROYECTO")?,
            &std::env::var("BQ_KIT_DATASET").unwrap_or_else(|_| "ore_kit".into()),
            &std::env::var("BQ_KIT_UBICACION").unwrap_or_else(|_| "EU".into()),
            std::env::var("ORE_BQ_CINTA_MODO").map_or(true, |m| m != "reproducir"),
        )),
        otro => {
            return Err(format!(
                "`{otro}` no es un banco: postgres, s3, gcs, azure o bigquery"
            ));
        }
    };
    eprintln!("ore-kit: cargando la semilla en {familia}…");
    banco.cargar()?;
    let c = Conector::new(PathBuf::from(&conector));
    eprintln!("ore-kit: {conector}");
    let resultados = match &pasarela {
        Some(p) => {
            eprintln!("ore-kit: la pasarela {p}");
            ore_conector_kit::pasarela::correr(
                &PathBuf::from(p),
                &c,
                &PathBuf::from(&conector),
                banco.as_mut(),
                &solo,
            )?
        }
        None => casos::correr(&c, banco.as_mut(), &solo),
    };

    println!("| caso | | estado | detalle |\n|---|---|---|---|");
    for r in &resultados {
        println!(
            "| {} | {} | **{}** | {} |",
            r.caso,
            r.nombre,
            r.estado.as_str(),
            r.detalle.replace('|', "\\|")
        );
    }
    let cuenta = |e: Estado| resultados.iter().filter(|r| r.estado == e).count();
    println!(
        "\n{} pasan, {} fallan, {} no aplican",
        cuenta(Estado::Pasa),
        cuenta(Estado::Falla),
        cuenta(Estado::NoAplica)
    );

    if let Some(ruta) = valor("--informe") {
        let informe = Json::obj([
            (
                "conector",
                Json::s(
                    PathBuf::from(&conector)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
            ),
            ("banco", Json::s(familia.as_str())),
            (
                "casos",
                Json::Arr(
                    resultados
                        .iter()
                        .map(|r| {
                            Json::obj([
                                ("caso", Json::Int(i64::from(r.caso))),
                                ("nombre", Json::s(r.nombre)),
                                ("estado", Json::s(r.estado.as_str())),
                                ("detalle", Json::s(r.detalle.as_str())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]);
        std::fs::write(&ruta, informe.jcs() + "\n")
            .map_err(|e| format!("no se escribe `{ruta}`: {e}"))?;
    }

    let fallan: Vec<u8> = resultados
        .iter()
        .filter(|r| exige.contains(&r.caso) && r.estado == Estado::Falla)
        .map(|r| r.caso)
        .collect();
    if !fallan.is_empty() {
        eprintln!("ore-kit: se exigían y fallan: {fallan:?}");
    }
    Ok(fallan.is_empty())
}

fn main() -> ExitCode {
    match intentar() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("ore-kit: {e}");
            ExitCode::from(2)
        }
    }
}
