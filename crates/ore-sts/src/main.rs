//! `ore-asumir-rol` — la URL de una fuente de S3 por la entrada estándar; si
//! lleva `role_arn`, la devuelve con la credencial temporal del rol (0046 E9b).
//!
//! ```text
//!   echo 's3://cubo?region=eu-north-1&role_arn=arn:aws:iam::…:role/…' | ore-asumir-rol --sesion ore-serve
//!   → {"caduca_ms":…,"url":"s3://cubo?region=eu-north-1&access_key_id=…&secret_access_key=…&session_token=…"}
//! ```
//!
//! La URL va y vuelve por stdin/stdout, nunca por `argv`: lleva una credencial.
//! Lo usa `ore-serve`, que no habla TLS, para firmar los ítems de una colección
//! virtual cuya fuente es un rol.
use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sesion = args
        .iter()
        .position(|a| a == "--sesion")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "ore".into());
    if sesion.is_empty()
        || sesion.len() > 64
        || !sesion
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"=,.@_-".contains(&b))
    {
        eprintln!("✗ `--sesion` es un nombre de sesión de STS (`[A-Za-z0-9=,.@_-]`, hasta 64)");
        return ExitCode::from(64);
    }
    let mut url = String::new();
    if std::io::stdin().read_to_string(&mut url).is_err() || url.trim().is_empty() {
        eprintln!("✗ la URL de la fuente va por la entrada estándar");
        return ExitCode::from(64);
    }
    match ore_sts::resolver(url.trim(), &sesion) {
        Ok((u, caduca)) => {
            println!("{}", ore_sts::salida(&u, caduca).jcs());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::from(69)
        }
    }
}
