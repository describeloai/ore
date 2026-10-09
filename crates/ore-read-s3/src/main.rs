//! `ore-read-s3` — el lector de un bucket de S3, **la familia de los objetos**
//! (ADR 0046 E4).
//!
//! Lo de todos los almacenes de objetos —el catálogo, las filas, las versiones
//! de una colección, la bajada y el bucle de verbos— vive en
//! `ore-read-objetos` (ADR 0061 O0·2); aquí queda lo de S3: leer su URL (con
//! el rol canjeado por `ore-sts`), decir qué acción de IAM falta y sobre qué
//! ARN (`check`, `acceso.rs`), explorar sus carpetas, y clasificar lo que S3
//! contesta. El bucket es su [`ore_objetos::Origen`] por `ore_s3::origen::Cubo`.
//!
//! La URL lleva la credencial y va **siempre por stdin**, nunca por `argv`, y
//! este programa no la imprime: ni en un error (lo que dice S3 no la contiene)
//! ni en `explorar` (`fuente::publica`).

mod acceso;
mod fuente;

use ore_objetos::Origen;
use ore_read_objetos::driver::{self, Proveedor};

struct S3;

impl Proveedor for S3 {
    const NOMBRE: &'static str = "ore-read-s3";
    type Fuente = fuente::Fuente;

    fn leer(url: &str) -> Result<fuente::Fuente, String> {
        fuente::leer(url)
    }

    fn prefijo(f: &fuente::Fuente) -> &str {
        &f.prefijo
    }

    fn origen(f: &fuente::Fuente) -> Box<dyn Origen + Sync + '_> {
        Box::new(ore_s3::origen::Cubo(&f.bucket))
    }

    fn comprobar(f: &fuente::Fuente) -> String {
        acceso::comprobar(f)
    }

    fn explorar(f: &fuente::Fuente) -> Result<String, String> {
        acceso::explorar(f)
    }

    /// Lo que S3 dice: `403 AccessDenied`, `404 NoSuchKey`…
    fn fallo(m: String) -> ore_driver::Fallo {
        driver::clasificar(
            m,
            &[
                "403",
                "AccessDenied",
                "InvalidAccessKeyId",
                "SignatureDoesNotMatch",
                "ExpiredToken",
                "falta `secret_access_key`",
            ],
            &["NoSuchBucket", "NoSuchKey", "404"],
        )
    }

    fn contadores() -> (usize, usize) {
        ore_s3::contadores()
    }
}

fn main() -> std::process::ExitCode {
    driver::main::<S3>()
}
