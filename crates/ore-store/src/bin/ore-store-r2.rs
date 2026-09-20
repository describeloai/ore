//! `ore-store-r2`: el ciclo sobre un S3 con clave estática (SigV4). Ver `lib.rs`.
fn main() -> std::process::ExitCode {
    match ore_store::r2::Cuenta::del_entorno() {
        Ok(c) => ore_store::ciclo::principal(std::sync::Arc::new(c)),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
