//! `ore-store-gcs`: el ciclo sobre Google Cloud Storage con el token de la cuenta
//! que corre. Ver `lib.rs` y `gcs.rs`.
fn main() -> std::process::ExitCode {
    match ore_store::gcs::Cuenta::del_entorno() {
        Ok(c) => ore_store::ciclo::principal(std::sync::Arc::new(c)),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
