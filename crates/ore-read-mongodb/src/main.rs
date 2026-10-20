//! `ore-read-mongodb`: los verbos v2 del esqueleto (`capacidades`, `leer`,
//! `servir`) y los de MongoDB (`catalogo`, `check`, `explorar`, `testigo`).

use ore_mongodb::{lectura::Mongo, verbos};

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<Mongo>(verbos::otros)
}
